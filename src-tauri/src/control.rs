use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::OpenOptions,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use tauri::State;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    time::timeout,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_RESPONSE_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct ClientError {
    pub kind: String,
    pub message: String,
}

impl ClientError {
    fn new(kind: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            message: message.into(),
        }
    }

    fn io(operation: &str, error: std::io::Error) -> Self {
        let kind = match error.kind() {
            ErrorKind::PermissionDenied => "permission_denied",
            ErrorKind::NotFound | ErrorKind::ConnectionRefused => "not_running",
            _ => "unavailable",
        };
        Self::new(kind, format!("{operation}: {error}"))
    }
}

#[derive(Clone)]
pub struct ControlClient {
    socket_path: PathBuf,
    timeout: Duration,
}

impl ControlClient {
    pub fn new(socket_path: PathBuf) -> Self {
        Self {
            socket_path,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    #[cfg(test)]
    fn with_timeout(socket_path: PathBuf, timeout: Duration) -> Self {
        Self {
            socket_path,
            timeout,
        }
    }

    pub fn socket_path(&self) -> &std::path::Path {
        &self.socket_path
    }

    pub async fn query(&self, command: &str) -> Result<Value, ClientError> {
        self.request(json!({ "command": command })).await
    }

    pub async fn query_with_params(
        &self,
        command: &str,
        params: Value,
    ) -> Result<Value, ClientError> {
        self.request(json!({ "command": command, "params": params }))
            .await
    }

    async fn request(&self, request: Value) -> Result<Value, ClientError> {
        let mut bytes = serde_json::to_vec(&request)
            .map_err(|error| ClientError::new("protocol", error.to_string()))?;
        bytes.push(b'\n');

        let stream = timeout(self.timeout, UnixStream::connect(&self.socket_path))
            .await
            .map_err(|_| ClientError::new("timeout", "connection timed out"))?
            .map_err(|error| ClientError::io("connect", error))?;
        let (reader, mut writer) = stream.into_split();

        timeout(self.timeout, writer.write_all(&bytes))
            .await
            .map_err(|_| ClientError::new("timeout", "write timed out"))?
            .map_err(|error| ClientError::io("write", error))?;
        writer
            .shutdown()
            .await
            .map_err(|error| ClientError::io("shutdown", error))?;

        let mut line = String::new();
        let mut reader = BufReader::new(reader).take(MAX_RESPONSE_BYTES + 1);
        let read = timeout(self.timeout, reader.read_line(&mut line))
            .await
            .map_err(|_| ClientError::new("timeout", "response timed out"))?
            .map_err(|error| ClientError::io("read", error))?;
        if read == 0 {
            return Err(ClientError::new(
                "protocol",
                "daemon closed the socket without a response",
            ));
        }
        if read as u64 > MAX_RESPONSE_BYTES {
            return Err(ClientError::new(
                "protocol",
                "daemon response exceeded 256 KiB",
            ));
        }

        let response: Value = serde_json::from_str(line.trim_end())
            .map_err(|error| ClientError::new("protocol", format!("invalid response: {error}")))?;
        match response.get("status").and_then(Value::as_str) {
            Some("ok") => Ok(response.get("data").cloned().unwrap_or(Value::Null)),
            Some("error") => Err(ClientError::new(
                "daemon",
                response
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("daemon rejected the request"),
            )),
            _ => Err(ClientError::new(
                "protocol",
                "response did not contain a recognized status",
            )),
        }
    }
}

pub fn resolve_socket_path(explicit: Option<PathBuf>) -> PathBuf {
    if let Some(path) = explicit {
        return path;
    }
    if let Some(path) = std::env::var_os("FIPS_MONITOR_SOCKET") {
        return PathBuf::from(path);
    }
    let candidates = [
        PathBuf::from("/var/run/fips/control.sock"),
        PathBuf::from("/run/fips/control.sock"),
        PathBuf::from("/tmp/fips-control.sock"),
    ];
    candidates
        .iter()
        .find(|candidate| candidate.exists())
        .cloned()
        .unwrap_or_else(|| candidates[0].clone())
}

fn client(state: &State<'_, crate::AppState>) -> ControlClient {
    ControlClient::new(state.socket_path.lock().unwrap().clone())
}

#[tauri::command]
pub async fn get_snapshot(
    state: State<'_, crate::AppState>,
) -> Result<crate::MonitorSnapshot, ClientError> {
    Ok(state.last_snapshot.lock().unwrap().clone())
}

#[tauri::command]
pub async fn get_peers(state: State<'_, crate::AppState>) -> Result<Value, ClientError> {
    client(&state).query("show_peers").await
}

#[tauri::command]
pub async fn get_transports(state: State<'_, crate::AppState>) -> Result<Value, ClientError> {
    client(&state).query("show_transports").await
}

#[tauri::command]
pub async fn get_acl(state: State<'_, crate::AppState>) -> Result<Value, ClientError> {
    client(&state).query("show_acl").await
}

#[tauri::command]
pub async fn add_acl_entry(
    state: State<'_, crate::AppState>,
    list: String,
    entry: String,
) -> Result<Value, ClientError> {
    update_acl_file(&client(&state), &list, &entry, false).await
}

#[tauri::command]
pub async fn remove_acl_entry(
    state: State<'_, crate::AppState>,
    list: String,
    entry: String,
) -> Result<Value, ClientError> {
    update_acl_file(&client(&state), &list, &entry, true).await
}

async fn update_acl_file(
    client: &ControlClient,
    list: &str,
    entry: &str,
    remove: bool,
) -> Result<Value, ClientError> {
    let list = match list {
        "allow" | "deny" => list,
        _ => {
            return Err(ClientError::new(
                "invalid_acl_list",
                "ACL list must be either allow or deny",
            ));
        }
    };
    let entry = normalize_acl_entry(entry)?;
    let snapshot = client.query("show_acl").await?;
    let key = if list == "allow" {
        "allow_file"
    } else {
        "deny_file"
    };
    let path = snapshot
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| ClientError::new("protocol", format!("show_acl did not return {key}")))?;
    let path = validate_acl_path(path, list)?;
    let changed = write_acl_entry(&path, &entry, remove)?;
    Ok(json!({
        "changed": changed,
        "entry": entry,
        "list": list,
        "path": path.display().to_string(),
    }))
}

fn normalize_acl_entry(entry: &str) -> Result<String, ClientError> {
    let entry = entry.trim();
    if entry.is_empty() {
        return Err(ClientError::new(
            "invalid_acl_entry",
            "ACL entry cannot be empty",
        ));
    }
    if entry.len() > 256
        || entry.starts_with('#')
        || entry
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
        || entry.chars().any(|character| {
            !character.is_ascii_alphanumeric() && !matches!(character, '.' | '_' | '-')
        })
    {
        return Err(ClientError::new(
            "invalid_acl_entry",
            "ACL entries may contain only letters, numbers, dots, underscores, and hyphens",
        ));
    }
    if entry.eq_ignore_ascii_case("all") {
        Ok("ALL".into())
    } else {
        Ok(entry.into())
    }
}

fn validate_acl_path(path: &str, list: &str) -> Result<PathBuf, ClientError> {
    let path = PathBuf::from(path);
    let expected_name = if list == "allow" {
        "peers.allow"
    } else {
        "peers.deny"
    };
    let standard_parent = [Path::new("/etc/fips"), Path::new("/usr/local/etc/fips")];
    let is_standard_path = path.is_absolute()
        && path.file_name().and_then(|name| name.to_str()) == Some(expected_name)
        && path
            .parent()
            .is_some_and(|parent| standard_parent.contains(&parent));
    if !is_standard_path {
        return Err(ClientError::new(
            "invalid_acl_path",
            "FIPS returned an ACL path outside its standard system configuration directory",
        ));
    }
    Ok(path)
}

fn write_acl_entry(path: &Path, entry: &str, remove: bool) -> Result<bool, ClientError> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == ErrorKind::NotFound && !remove => String::new(),
        Err(error) => return Err(acl_file_error("read", path, error)),
    };
    let trailing_newline = contents.ends_with('\n');
    let mut lines: Vec<String> = contents.lines().map(ToOwned::to_owned).collect();
    let was_present = lines
        .iter()
        .any(|line| line.trim().eq_ignore_ascii_case(entry));
    let changed = if remove {
        lines.retain(|line| !line.trim().eq_ignore_ascii_case(entry));
        was_present
    } else if was_present {
        false
    } else {
        lines.push(entry.to_owned());
        true
    };
    if !changed {
        return Ok(false);
    }

    let mut output = lines.join("\n");
    if !output.is_empty() && (trailing_newline || !remove) {
        output.push('\n');
    }
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
        .map_err(|error| acl_file_error("open", path, error))?;
    file.write_all(output.as_bytes())
        .map_err(|error| acl_file_error("write", path, error))?;
    file.sync_all()
        .map_err(|error| acl_file_error("sync", path, error))?;
    Ok(true)
}

fn acl_file_error(operation: &str, path: &Path, error: std::io::Error) -> ClientError {
    let kind = if error.kind() == ErrorKind::PermissionDenied {
        "permission_denied"
    } else {
        "acl_file"
    };
    ClientError::new(
        kind,
        format!("Could not {operation} {}: {error}", path.display()),
    )
}

#[tauri::command]
pub async fn connect_peer(
    state: State<'_, crate::AppState>,
    npub: String,
    address: String,
    transport: String,
) -> Result<Value, ClientError> {
    client(&state)
        .query_with_params(
            "connect",
            json!({ "npub": npub, "address": address, "transport": transport }),
        )
        .await
}

#[tauri::command]
pub async fn disconnect_peer(
    state: State<'_, crate::AppState>,
    npub: String,
) -> Result<Value, ClientError> {
    client(&state)
        .query_with_params("disconnect", json!({ "npub": npub }))
        .await
}

#[tauri::command]
pub async fn get_config(state: State<'_, crate::AppState>) -> Result<Value, ClientError> {
    client(&state)
        .query("show_config")
        .await
        .map_err(upgrade_error)
}

#[tauri::command]
pub async fn validate_config(
    state: State<'_, crate::AppState>,
    expected_revision: String,
    yaml: String,
) -> Result<Value, ClientError> {
    client(&state)
        .query_with_params(
            "validate_config",
            json!({ "expected_revision": expected_revision, "yaml": yaml }),
        )
        .await
        .map_err(upgrade_error)
}

#[tauri::command]
pub async fn apply_config(
    state: State<'_, crate::AppState>,
    expected_revision: String,
    yaml: String,
) -> Result<Value, ClientError> {
    client(&state)
        .query_with_params(
            "apply_config",
            json!({ "expected_revision": expected_revision, "yaml": yaml }),
        )
        .await
        .map_err(upgrade_error)
}

#[tauri::command]
pub async fn get_apply_status(state: State<'_, crate::AppState>) -> Result<Value, ClientError> {
    client(&state)
        .query("show_config_apply")
        .await
        .map_err(upgrade_error)
}

#[tauri::command]
pub async fn reset_config(
    state: State<'_, crate::AppState>,
    expected_revision: String,
) -> Result<Value, ClientError> {
    client(&state)
        .query_with_params(
            "reset_managed_config",
            json!({ "expected_revision": expected_revision }),
        )
        .await
        .map_err(upgrade_error)
}

#[tauri::command]
pub async fn set_socket_path(
    state: State<'_, crate::AppState>,
    socket_path: String,
) -> Result<String, ClientError> {
    let path = PathBuf::from(socket_path.trim());
    if !path.is_absolute() {
        return Err(ClientError::new(
            "invalid_path",
            "the development socket path must be absolute",
        ));
    }
    *state.socket_path.lock().unwrap() = path.clone();
    state.refresh.notify_one();
    Ok(path.display().to_string())
}

#[tauri::command]
pub fn refresh_now(state: State<'_, crate::AppState>) {
    state.refresh.notify_one();
}

fn upgrade_error(mut error: ClientError) -> ClientError {
    if error.kind == "daemon" && error.message.starts_with("unknown command:") {
        error.kind = "upgrade_required".into();
        error.message = "This FIPS daemon can be monitored, but it must be upgraded before its configuration can be changed.".into();
    }
    error
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, time::SystemTime};
    use tokio::net::UnixListener;

    fn socket_path(label: &str) -> PathBuf {
        PathBuf::from(format!(
            "/tmp/fm-{}-{label}-{}.sock",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ))
    }

    #[tokio::test]
    async fn parses_tolerant_success_response() {
        let path = socket_path("success");
        let listener = UnixListener::bind(&path).unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = String::new();
            BufReader::new(&mut socket)
                .read_line(&mut request)
                .await
                .unwrap();
            assert!(request.contains("show_status"));
            socket
                .write_all(b"{\"status\":\"ok\",\"data\":{\"state\":\"Running\",\"future\":42}}\n")
                .await
                .unwrap();
        });
        let value = ControlClient::new(path.clone())
            .query("show_status")
            .await
            .unwrap();
        assert_eq!(value["future"], 42);
        server.await.unwrap();
        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn categorizes_permission_and_daemon_errors() {
        let missing = socket_path("missing");
        let error = ControlClient::new(missing)
            .query("show_status")
            .await
            .unwrap_err();
        assert_eq!(error.kind, "not_running");

        let path = socket_path("daemon-error");
        let listener = UnixListener::bind(&path).unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = String::new();
            BufReader::new(&mut socket)
                .read_line(&mut request)
                .await
                .unwrap();
            socket
                .write_all(b"{\"status\":\"error\",\"message\":\"nope\"}\n")
                .await
                .unwrap();
        });
        let error = ControlClient::new(path.clone())
            .query("show_status")
            .await
            .unwrap_err();
        assert_eq!(error.kind, "daemon");
        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn times_out_a_silent_daemon() {
        let path = socket_path("timeout");
        let listener = UnixListener::bind(&path).unwrap();
        tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_millis(200)).await;
        });
        let error = ControlClient::with_timeout(path.clone(), Duration::from_millis(20))
            .query("show_status")
            .await
            .unwrap_err();
        assert_eq!(error.kind, "timeout");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn validates_acl_entries_and_paths() {
        assert_eq!(normalize_acl_entry(" all ").unwrap(), "ALL");
        assert_eq!(normalize_acl_entry("core-vm").unwrap(), "core-vm");
        assert_eq!(normalize_acl_entry("npub1example").unwrap(), "npub1example");
        assert_eq!(
            normalize_acl_entry("bad entry").unwrap_err().kind,
            "invalid_acl_entry"
        );
        assert_eq!(
            validate_acl_path("/usr/local/etc/fips/peers.allow", "allow").unwrap(),
            PathBuf::from("/usr/local/etc/fips/peers.allow")
        );
        assert_eq!(
            validate_acl_path("/tmp/peers.allow", "allow")
                .unwrap_err()
                .kind,
            "invalid_acl_path"
        );
    }

    #[test]
    fn updates_acl_entries_without_removing_comments() {
        let path = socket_path("acl-file");
        fs::write(&path, "# managed by operator\nnpub1existing\n").unwrap();
        assert!(write_acl_entry(&path, "npub1added", false).unwrap());
        assert!(!write_acl_entry(&path, "npub1added", false).unwrap());
        assert!(write_acl_entry(&path, "npub1existing", true).unwrap());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "# managed by operator\nnpub1added\n"
        );
        let _ = fs::remove_file(path);
    }
}
