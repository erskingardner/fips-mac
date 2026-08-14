use fips::Config;
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};
use serde_yaml::{Mapping, Value as YamlValue};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

pub const REDACTED_SENTINEL: &str = "<redacted:preserve>";
pub const MAX_CONFIG_BYTES: usize = 128 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyActivation {
    None,
    Restart,
}

#[derive(Debug, Clone)]
pub struct ValidatedConfig {
    pub hydrated_yaml: String,
    pub redacted_yaml: String,
    pub diff: Vec<JsonValue>,
    pub activation: ApplyActivation,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigSnapshot {
    pub source: &'static str,
    pub base_path: String,
    pub managed_path: String,
    pub revision: String,
    pub yaml: String,
    pub guided: JsonValue,
    pub secrets: JsonValue,
    pub last_apply: Option<JsonValue>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigApplyResult {
    pub apply_id: String,
    pub revision: String,
    pub activation: ApplyActivation,
    pub diff: Vec<JsonValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApplyJournal {
    apply_id: String,
    state: String,
    previous_revision: String,
    candidate_revision: String,
    error: Option<String>,
    updated_at_ms: u128,
}

#[derive(Debug)]
struct ActiveDocument {
    raw: String,
    config: Config,
    revision: String,
}

#[derive(Debug, Clone)]
pub struct ConfigManager {
    config_path: PathBuf,
    original_path: PathBuf,
    last_good_path: PathBuf,
    journal_path: PathBuf,
}

impl ConfigManager {
    pub fn new(config_path: PathBuf, state_dir: PathBuf) -> Self {
        Self {
            config_path,
            original_path: state_dir.join("fips.original.yaml"),
            last_good_path: state_dir.join("fips.last-good.yaml"),
            journal_path: state_dir.join("fips-config-state.json"),
        }
    }

    pub fn bootstrap(&self, legacy_managed_path: Option<&Path>) -> Result<(), String> {
        if self.original_path.exists() {
            return require_secure_regular_file(&self.original_path);
        }
        let active = read_secure_regular_file(&self.config_path)?;
        atomic_write(&self.original_path, active.as_bytes())?;
        if let Some(legacy_path) = legacy_managed_path
            && legacy_path.exists()
        {
            let legacy = read_secure_regular_file(legacy_path)?;
            validate_document(&legacy)?;
            atomic_write(&self.config_path, legacy.as_bytes())?;
        }
        Ok(())
    }

    pub fn snapshot(&self) -> Result<ConfigSnapshot, String> {
        let active = self.active_document()?;
        let mut yaml_value: YamlValue = serde_yaml::from_str(&active.raw)
            .map_err(|error| format!("failed to parse active configuration: {error}"))?;
        normalize_editable_null_sections(&mut yaml_value);
        let secrets = secret_metadata(&yaml_value);
        redact_secrets(&mut yaml_value);
        let yaml = serde_yaml::to_string(&yaml_value)
            .map_err(|error| format!("failed to render redacted configuration: {error}"))?;
        Ok(ConfigSnapshot {
            source: "managed",
            base_path: self.original_path.display().to_string(),
            managed_path: self.config_path.display().to_string(),
            revision: active.revision,
            yaml,
            guided: sanitized_json(&active.config)?,
            secrets,
            last_apply: self.last_apply(),
        })
    }

    pub fn validate(
        &self,
        expected_revision: &str,
        proposed_yaml: &str,
    ) -> Result<ValidatedConfig, String> {
        if proposed_yaml.len() > MAX_CONFIG_BYTES {
            return Err(format!(
                "configuration is too large ({} bytes; maximum {MAX_CONFIG_BYTES})",
                proposed_yaml.len()
            ));
        }
        let active = self.active_document()?;
        if active.revision != expected_revision {
            return Err(format!(
                "configuration changed since it was loaded (expected revision {expected_revision}, current revision {})",
                active.revision
            ));
        }

        let mut proposed_value: YamlValue = serde_yaml::from_str(proposed_yaml)
            .map_err(|error| format!("invalid YAML: {error}"))?;
        let current_value: YamlValue = serde_yaml::from_str(&active.raw)
            .map_err(|error| format!("failed to parse active configuration: {error}"))?;
        hydrate_secrets(&mut proposed_value, &current_value)?;
        let hydrated_yaml = serde_yaml::to_string(&proposed_value)
            .map_err(|error| format!("failed to render proposed configuration: {error}"))?;
        let config = validate_document(&hydrated_yaml)?;

        if !config.node.control.enabled
            || config.node.control.socket_path != "/var/run/fips/control.sock"
        {
            return Err(
                "node.control.enabled must remain true and node.control.socket_path must remain /var/run/fips/control.sock when this app manages the node"
                    .to_string(),
            );
        }

        let current_json = sanitized_json(&active.config)?;
        let proposed_json = sanitized_json(&config)?;
        let mut diff = Vec::new();
        collect_diff("", &current_json, &proposed_json, &mut diff);
        let activation = if current_json == proposed_json {
            ApplyActivation::None
        } else {
            ApplyActivation::Restart
        };
        let mut redacted_value = proposed_value;
        redact_secrets(&mut redacted_value);
        let redacted_yaml = serde_yaml::to_string(&redacted_value)
            .map_err(|error| format!("failed to render redacted configuration: {error}"))?;

        Ok(ValidatedConfig {
            hydrated_yaml,
            redacted_yaml,
            diff,
            activation,
        })
    }

    pub fn apply(
        &self,
        expected_revision: &str,
        proposed_yaml: &str,
    ) -> Result<(ConfigApplyResult, ValidatedConfig), String> {
        let validated = self.validate(expected_revision, proposed_yaml)?;
        let active = self.active_document()?;
        atomic_write(&self.last_good_path, active.raw.as_bytes())?;
        let candidate_revision = revision(validated.hydrated_yaml.as_bytes());
        let apply_id = new_apply_id();
        self.write_journal(&ApplyJournal {
            apply_id: apply_id.clone(),
            state: "pending".to_string(),
            previous_revision: active.revision,
            candidate_revision: candidate_revision.clone(),
            error: None,
            updated_at_ms: now_ms(),
        })?;
        if let Err(error) = atomic_write(&self.config_path, validated.hydrated_yaml.as_bytes()) {
            let _ = self.rollback_pending(&error);
            return Err(error);
        }
        Ok((
            ConfigApplyResult {
                apply_id,
                revision: candidate_revision,
                activation: validated.activation,
                diff: validated.diff.clone(),
            },
            validated,
        ))
    }

    pub fn reset(&self, expected_revision: &str) -> Result<ConfigApplyResult, String> {
        let active = self.active_document()?;
        if active.revision != expected_revision {
            return Err("configuration changed since it was loaded".to_string());
        }
        let original = read_secure_regular_file(&self.original_path)?;
        let original_config = validate_document(&original)?;
        let current_json = sanitized_json(&active.config)?;
        let original_json = sanitized_json(&original_config)?;
        let mut diff = Vec::new();
        collect_diff("", &current_json, &original_json, &mut diff);
        let activation = if current_json == original_json {
            ApplyActivation::None
        } else {
            ApplyActivation::Restart
        };
        atomic_write(&self.last_good_path, active.raw.as_bytes())?;
        let apply_id = new_apply_id();
        let candidate_revision = revision(original.as_bytes());
        self.write_journal(&ApplyJournal {
            apply_id: apply_id.clone(),
            state: "pending".to_string(),
            previous_revision: active.revision,
            candidate_revision: candidate_revision.clone(),
            error: None,
            updated_at_ms: now_ms(),
        })?;
        if let Err(error) = atomic_write(&self.config_path, original.as_bytes()) {
            let _ = self.rollback_pending(&error);
            return Err(error);
        }
        Ok(ConfigApplyResult {
            apply_id,
            revision: candidate_revision,
            activation,
            diff,
        })
    }

    pub fn mark_applied(&self) -> Result<(), String> {
        self.update_pending("applied", None)
    }

    pub fn mark_failed(&self, error: impl Into<String>) -> Result<(), String> {
        self.update_pending("failed", Some(error.into()))
    }

    pub fn rollback_pending(&self, error: impl Into<String>) -> Result<bool, String> {
        let error = error.into();
        let Some(mut journal) = self.read_journal()? else {
            return Ok(false);
        };
        if journal.state != "pending" {
            return Ok(false);
        }
        let previous = read_secure_regular_file(&self.last_good_path)?;
        if let Err(restore_error) = atomic_write(&self.config_path, previous.as_bytes()) {
            let message =
                format!("{error}; last-known-good restoration also failed: {restore_error}");
            journal.state = "failed".to_string();
            journal.error = Some(message.clone());
            journal.updated_at_ms = now_ms();
            let _ = self.write_journal(&journal);
            return Err(message);
        }
        journal.state = "rolled_back".to_string();
        journal.error = Some(error);
        journal.updated_at_ms = now_ms();
        self.write_journal(&journal)?;
        Ok(true)
    }

    pub fn last_apply(&self) -> Option<JsonValue> {
        self.read_journal()
            .ok()
            .flatten()
            .and_then(|journal| serde_json::to_value(journal).ok())
    }

    pub fn redact_error_message(&self, message: &str) -> String {
        let Ok(raw) = fs::read_to_string(&self.config_path) else {
            return message.to_string();
        };
        redact_message_secrets(message, &raw)
    }

    fn active_document(&self) -> Result<ActiveDocument, String> {
        let raw = read_secure_regular_file(&self.config_path)?;
        let config = validate_document(&raw)?;
        let revision = revision(raw.as_bytes());
        Ok(ActiveDocument {
            raw,
            config,
            revision,
        })
    }

    fn read_journal(&self) -> Result<Option<ApplyJournal>, String> {
        if !self.journal_path.exists() {
            return Ok(None);
        }
        let raw = read_secure_regular_file(&self.journal_path)?;
        serde_json::from_str(&raw)
            .map(Some)
            .map_err(|error| format!("failed to parse configuration apply journal: {error}"))
    }

    fn write_journal(&self, journal: &ApplyJournal) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(journal)
            .map_err(|error| format!("failed to serialize apply journal: {error}"))?;
        atomic_write(&self.journal_path, &bytes)
    }

    fn update_pending(&self, state: &str, error: Option<String>) -> Result<(), String> {
        let Some(mut journal) = self.read_journal()? else {
            return Ok(());
        };
        if journal.state == "pending" || state == "failed" {
            journal.state = state.to_string();
            journal.error = error;
            journal.updated_at_ms = now_ms();
            self.write_journal(&journal)?;
        }
        Ok(())
    }
}

fn validate_document(raw: &str) -> Result<Config, String> {
    let config: Config = serde_yaml::from_str(raw)
        .map_err(|error| format!("invalid FIPS configuration: {error}"))?;
    config.validate().map_err(|error| error.to_string())?;
    if config.node.identity.nsec.is_some() {
        config
            .create_identity()
            .map_err(|error| format!("invalid node identity: {error}"))?;
    }
    Ok(config)
}

fn require_secure_regular_file(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
        return Err(format!(
            "{} must be owned by the service account with mode 0600",
            path.display()
        ));
    }
    Ok(())
}

fn read_secure_regular_file(path: &Path) -> Result<String, String> {
    require_secure_regular_file(path)?;
    fs::read_to_string(path).map_err(|error| format!("failed to read {}: {error}", path.display()))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent", path.display()))?;
    let parent_metadata = fs::symlink_metadata(parent)
        .map_err(|error| format!("failed to inspect {}: {error}", parent.display()))?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err(format!("{} is not a real directory", parent.display()));
    }
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return Err(format!(
            "refusing to replace unsafe path {}",
            path.display()
        ));
    }
    let temporary = parent.join(format!(
        ".fips-config-{}-{}.tmp",
        std::process::id(),
        now_ms()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|error| format!("failed to create {}: {error}", temporary.display()))?;
    let result = (|| -> Result<(), String> {
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("failed to persist {}: {error}", temporary.display()))?;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("failed to secure {}: {error}", temporary.display()))?;
        fs::rename(&temporary, path)
            .map_err(|error| format!("failed to replace {}: {error}", path.display()))?;
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("failed to sync {}: {error}", parent.display()))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn revision(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn new_apply_id() -> String {
    static APPLY_COUNTER: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}-{}",
        now_ms(),
        std::process::id(),
        APPLY_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

fn key(name: &str) -> YamlValue {
    YamlValue::String(name.to_string())
}

fn mapping_at_mut<'a>(root: &'a mut YamlValue, path: &[&str]) -> Option<&'a mut Mapping> {
    let mut current = root;
    for component in path {
        current = current.as_mapping_mut()?.get_mut(key(component))?;
    }
    current.as_mapping_mut()
}

fn mapping_at<'a>(root: &'a YamlValue, path: &[&str]) -> Option<&'a Mapping> {
    let mut current = root;
    for component in path {
        current = current.as_mapping()?.get(key(component))?;
    }
    current.as_mapping()
}

fn value_at_mut<'a>(root: &'a mut YamlValue, path: &[&str]) -> Option<&'a mut YamlValue> {
    let mut current = root;
    for component in path {
        current = current.as_mapping_mut()?.get_mut(key(component))?;
    }
    Some(current)
}

fn normalize_editable_null_sections(root: &mut YamlValue) {
    // The shipped operator file keeps these optional sections present with
    // commented examples beneath them. serde_yaml accepts that source when it
    // deserializes directly into FIPS Config, but projecting through Value and
    // serializing turns the empty sections into explicit `null`, which Config
    // then rejects when the draft is submitted unchanged. Keep the editable
    // projection round-trippable without adding any effective settings.
    for path in [["node", "identity"], ["node", "rendezvous"]] {
        if let Some(value) = value_at_mut(root, &path)
            && value.is_null()
        {
            *value = YamlValue::Mapping(Mapping::new());
        }
    }
}

fn redact_secrets(root: &mut YamlValue) {
    if let Some(identity) = mapping_at_mut(root, &["node", "identity"])
        && identity.contains_key(key("nsec"))
    {
        identity.insert(
            key("nsec"),
            YamlValue::String(REDACTED_SENTINEL.to_string()),
        );
    }
    if let Some(tor) = mapping_at_mut(root, &["transports", "tor"]) {
        redact_control_auth(tor);
    }
}

fn redact_control_auth(mapping: &mut Mapping) {
    if let Some(value) = mapping.get_mut(key("control_auth"))
        && value
            .as_str()
            .is_some_and(|value| value.starts_with("password:"))
    {
        *value = YamlValue::String(REDACTED_SENTINEL.to_string());
    }
    for value in mapping.values_mut() {
        if let Some(child) = value.as_mapping_mut() {
            redact_control_auth(child);
        }
    }
}

fn secret_metadata(root: &YamlValue) -> JsonValue {
    let identity_nsec =
        mapping_at(root, &["node", "identity"]).is_some_and(|map| map.contains_key(key("nsec")));
    let mut tor_passwords = Vec::new();
    if let Some(tor) = mapping_at(root, &["transports", "tor"]) {
        collect_tor_password_paths("tor", tor, &mut tor_passwords);
    }
    json!({
        "identity_nsec_configured": identity_nsec,
        "tor_password_paths": tor_passwords,
    })
}

fn collect_tor_password_paths(prefix: &str, mapping: &Mapping, output: &mut Vec<String>) {
    if mapping
        .get(key("control_auth"))
        .and_then(YamlValue::as_str)
        .is_some_and(|value| value.starts_with("password:"))
    {
        output.push(format!("transports.{prefix}.control_auth"));
    }
    for (name, value) in mapping {
        if let (Some(name), Some(child)) = (name.as_str(), value.as_mapping()) {
            collect_tor_password_paths(&format!("{prefix}.{name}"), child, output);
        }
    }
}

fn hydrate_secrets(proposed: &mut YamlValue, current: &YamlValue) -> Result<(), String> {
    hydrate_one(proposed, current, &["node", "identity"], "nsec")?;
    let current_tor = mapping_at(current, &["transports", "tor"]);
    if let Some(proposed_tor) = mapping_at_mut(proposed, &["transports", "tor"]) {
        hydrate_control_auth(proposed_tor, current_tor, "transports.tor")?;
    }
    Ok(())
}

fn hydrate_one(
    proposed: &mut YamlValue,
    current: &YamlValue,
    parent_path: &[&str],
    field: &str,
) -> Result<(), String> {
    let replacement = mapping_at(current, parent_path)
        .and_then(|mapping| mapping.get(key(field)))
        .cloned();
    if let Some(parent) = mapping_at_mut(proposed, parent_path)
        && parent.get(key(field)).and_then(YamlValue::as_str) == Some(REDACTED_SENTINEL)
    {
        let value = replacement.ok_or_else(|| {
            format!(
                "redacted preserve marker at {}.{field} has no source value",
                parent_path.join(".")
            )
        })?;
        parent.insert(key(field), value);
    }
    Ok(())
}

fn hydrate_control_auth(
    proposed: &mut Mapping,
    current: Option<&Mapping>,
    path: &str,
) -> Result<(), String> {
    if proposed
        .get(key("control_auth"))
        .and_then(YamlValue::as_str)
        == Some(REDACTED_SENTINEL)
    {
        let source = current
            .and_then(|mapping| mapping.get(key("control_auth")))
            .cloned()
            .ok_or_else(|| {
                format!("redacted preserve marker at {path}.control_auth has no source value")
            })?;
        proposed.insert(key("control_auth"), source);
    }
    let child_names: Vec<String> = proposed
        .keys()
        .filter_map(YamlValue::as_str)
        .filter(|name| *name != "control_auth")
        .map(str::to_string)
        .collect();
    for name in child_names {
        if let Some(proposed_child) = proposed
            .get_mut(key(&name))
            .and_then(YamlValue::as_mapping_mut)
        {
            let current_child = current
                .and_then(|mapping| mapping.get(key(&name)))
                .and_then(YamlValue::as_mapping);
            hydrate_control_auth(proposed_child, current_child, &format!("{path}.{name}"))?;
        }
    }
    Ok(())
}

fn sanitized_json(config: &Config) -> Result<JsonValue, String> {
    let mut value = serde_json::to_value(config)
        .map_err(|error| format!("failed to project configuration: {error}"))?;
    if let Some(identity) = value
        .pointer_mut("/node/identity")
        .and_then(JsonValue::as_object_mut)
        && identity.contains_key("nsec")
    {
        identity.insert(
            "nsec".to_string(),
            JsonValue::String(REDACTED_SENTINEL.to_string()),
        );
    }
    redact_json_passwords(&mut value);
    Ok(value)
}

fn redact_json_passwords(value: &mut JsonValue) {
    match value {
        JsonValue::Object(map) => {
            if map
                .get("control_auth")
                .and_then(JsonValue::as_str)
                .is_some_and(|value| value.starts_with("password:"))
            {
                map.insert(
                    "control_auth".to_string(),
                    JsonValue::String(REDACTED_SENTINEL.to_string()),
                );
            }
            for value in map.values_mut() {
                redact_json_passwords(value);
            }
        }
        JsonValue::Array(values) => {
            for value in values {
                redact_json_passwords(value);
            }
        }
        _ => {}
    }
}

fn collect_diff(path: &str, before: &JsonValue, after: &JsonValue, output: &mut Vec<JsonValue>) {
    if before == after {
        return;
    }
    match (before, after) {
        (JsonValue::Object(before), JsonValue::Object(after)) => {
            let mut keys: Vec<&String> = before.keys().chain(after.keys()).collect();
            keys.sort();
            keys.dedup();
            for key in keys {
                let child_path = format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"));
                collect_diff(
                    &child_path,
                    before.get(key).unwrap_or(&JsonValue::Null),
                    after.get(key).unwrap_or(&JsonValue::Null),
                    output,
                );
            }
        }
        _ => output.push(json!({
            "path": if path.is_empty() { "/" } else { path },
            "before": before,
            "after": after,
        })),
    }
}

pub fn validation_error_path(message: &str) -> &'static str {
    if message.contains("node.control") {
        "/node/control"
    } else if message.contains("identity") || message.contains("nsec") {
        "/node/identity"
    } else if message.contains("transport") {
        "/transports"
    } else if message.contains("peer") {
        "/peers"
    } else if message.contains("tun") {
        "/tun"
    } else if message.contains("dns") {
        "/dns"
    } else {
        "/"
    }
}

pub fn redact_message_secrets(message: &str, yaml: &str) -> String {
    let Ok(value) = serde_yaml::from_str::<YamlValue>(yaml) else {
        return message.to_string();
    };
    let mut secrets = Vec::new();
    if let Some(secret) = mapping_at(&value, &["node", "identity"])
        .and_then(|identity| identity.get(key("nsec")))
        .and_then(YamlValue::as_str)
        && secret != REDACTED_SENTINEL
    {
        secrets.push(secret.to_string());
    }
    collect_secret_values(&value, &mut secrets);
    secrets
        .into_iter()
        .fold(message.to_string(), |redacted, secret| {
            redacted.replace(&secret, "<redacted>")
        })
}

fn collect_secret_values(value: &YamlValue, output: &mut Vec<String>) {
    match value {
        YamlValue::Mapping(mapping) => {
            for (name, value) in mapping {
                if name.as_str() == Some("control_auth")
                    && let Some(secret) = value.as_str()
                    && secret.starts_with("password:")
                {
                    output.push(secret.to_string());
                }
                collect_secret_values(value, output);
            }
        }
        YamlValue::Sequence(values) => {
            for value in values {
                collect_secret_values(value, output);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const CONFIG: &str = "node:\n  identity:\n    nsec: 0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20\n    persistent: true\n  control:\n    enabled: true\n    socket_path: /var/run/fips/control.sock\ntun:\n  enabled: false\ndns:\n  enabled: false\npeers: []\n";

    fn manager() -> (TempDir, ConfigManager) {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("fips.yaml");
        fs::write(&path, CONFIG).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let manager = ConfigManager::new(path, directory.path().join("state"));
        fs::create_dir(directory.path().join("state")).unwrap();
        manager.bootstrap(None).unwrap();
        (directory, manager)
    }

    #[test]
    fn snapshot_redacts_identity_and_preserves_it_during_validation() {
        let (_directory, manager) = manager();
        let snapshot = manager.snapshot().unwrap();
        assert!(!snapshot.yaml.contains("010203"));
        assert!(snapshot.yaml.contains(REDACTED_SENTINEL));
        let validated = manager
            .validate(&snapshot.revision, &snapshot.yaml)
            .unwrap();
        assert!(validated.hydrated_yaml.contains("010203"));
    }

    #[test]
    fn snapshot_keeps_comment_only_node_sections_round_trippable() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("fips.yaml");
        let config = "node:\n  identity:\n    # Optional identity settings.\n  rendezvous:\n    # Optional rendezvous settings.\n  control:\n    enabled: true\n    socket_path: /var/run/fips/control.sock\ntun:\n  enabled: false\ndns:\n  enabled: false\npeers: []\n";
        fs::write(&path, config).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let manager = ConfigManager::new(path, directory.path().join("state"));
        fs::create_dir(directory.path().join("state")).unwrap();
        manager.bootstrap(None).unwrap();

        let snapshot = manager.snapshot().unwrap();
        assert!(snapshot.yaml.contains("identity: {}"));
        assert!(snapshot.yaml.contains("rendezvous: {}"));
        manager
            .validate(&snapshot.revision, &snapshot.yaml)
            .unwrap();
    }

    #[test]
    fn redacts_new_secret_values_from_validation_errors() {
        let yaml = "node:\n  identity:\n    nsec: do-not-return-this\ntransports:\n  tor:\n    control_auth: password:also-secret\n";
        let message = "invalid nsec do-not-return-this and password:also-secret";
        let redacted = redact_message_secrets(message, yaml);
        assert_eq!(redacted, "invalid nsec <redacted> and <redacted>");
    }

    #[test]
    fn rejects_stale_edits_and_control_channel_changes() {
        let (_directory, manager) = manager();
        let snapshot = manager.snapshot().unwrap();
        assert!(manager.validate("stale", &snapshot.yaml).is_err());
        let disabled = snapshot.yaml.replace("enabled: true", "enabled: false");
        assert!(manager.validate(&snapshot.revision, &disabled).is_err());
    }

    #[test]
    fn formatting_only_apply_does_not_restart() {
        let (_directory, manager) = manager();
        let snapshot = manager.snapshot().unwrap();
        let changed = format!("# comment\n{}", snapshot.yaml);
        let (result, _) = manager.apply(&snapshot.revision, &changed).unwrap();
        assert_eq!(result.activation, ApplyActivation::None);
        manager.mark_applied().unwrap();
        assert_eq!(
            manager.last_apply().unwrap()["state"],
            JsonValue::String("applied".into())
        );
    }

    #[test]
    fn semantic_changes_restart_and_can_roll_back() {
        let (_directory, manager) = manager();
        let snapshot = manager.snapshot().unwrap();
        let changed = snapshot
            .yaml
            .replace("persistent: true", "persistent: false");
        let (result, _) = manager.apply(&snapshot.revision, &changed).unwrap();
        assert_eq!(result.activation, ApplyActivation::Restart);
        assert!(manager.rollback_pending("startup failed").unwrap());
        assert!(
            fs::read_to_string(&manager.config_path)
                .unwrap()
                .contains("persistent: true")
        );
    }

    #[test]
    fn reset_restores_initial_configuration() {
        let (_directory, manager) = manager();
        let snapshot = manager.snapshot().unwrap();
        let changed = snapshot
            .yaml
            .replace("persistent: true", "persistent: false");
        manager.apply(&snapshot.revision, &changed).unwrap();
        manager.mark_applied().unwrap();
        let changed_snapshot = manager.snapshot().unwrap();
        let result = manager.reset(&changed_snapshot.revision).unwrap();
        assert_eq!(result.activation, ApplyActivation::Restart);
        assert!(
            fs::read_to_string(&manager.config_path)
                .unwrap()
                .contains("persistent: true")
        );
    }

    #[test]
    fn rejects_oversized_documents_and_symlinks() {
        let (directory, manager) = manager();
        let snapshot = manager.snapshot().unwrap();
        assert!(
            manager
                .validate(&snapshot.revision, &"x".repeat(MAX_CONFIG_BYTES + 1))
                .is_err()
        );
        let unsafe_path = directory.path().join("unsafe.yaml");
        std::os::unix::fs::symlink(&manager.config_path, &unsafe_path).unwrap();
        let unsafe_manager = ConfigManager::new(unsafe_path, directory.path().join("state-unsafe"));
        assert!(unsafe_manager.snapshot().is_err());
    }
}
