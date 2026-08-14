use crate::{MonitorSnapshot, now_ms, service::ServiceStatus};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashSet, sync::Mutex};

const DEFAULT_SCENARIO: &str = "managed_running";
const PREVIEW_SOCKET: &str = "preview://fips/control.sock";
const PREVIEW_CONFIG: &str = r#"node:
  identity:
    persistent: true
  leaf_only: false
  log_level: info
  control:
    enabled: true
    socket_path: /var/run/fips/control.sock
  rendezvous:
    lan:
      enabled: true
    nostr:
      enabled: false

tun:
  enabled: true
  name: fips0
  mtu: 1280

dns:
  enabled: true
  port: 5354

transports:
  udp:
    bind_addr: "0.0.0.0:2121"
  tcp:
    bind_addr: "0.0.0.0:8443"

peers:
  - npub: npub1previewpeer00000000000000000000000000000000000000000000001
    alias: Studio gateway
    via_nostr: false
    connect_policy: auto_connect
    addresses:
      - transport: udp
        addr: "192.168.1.42:2121"
"#;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreviewScenario {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreviewStatus {
    pub available: bool,
    pub enabled: bool,
    pub scenario: String,
    pub scenarios: Vec<PreviewScenario>,
}

#[derive(Debug)]
struct PreviewModel {
    enabled: bool,
    scenario: String,
    running_override: Option<bool>,
    config_yaml: String,
    revision: u64,
    last_apply: Value,
    removed_peers: HashSet<String>,
    connected_peer: Option<Value>,
}

pub struct PreviewController {
    inner: Mutex<PreviewModel>,
}

impl PreviewController {
    pub fn new() -> Self {
        let (enabled, scenario) = if crate::DEVELOPMENT_TOOLS_INCLUDED {
            let requested =
                std::env::var("FIPS_MAC_PREVIEW").unwrap_or_else(|_| DEFAULT_SCENARIO.to_string());
            if matches!(requested.as_str(), "0" | "false" | "off" | "live") {
                (false, DEFAULT_SCENARIO.to_string())
            } else if scenario_exists(&requested) {
                (true, requested)
            } else {
                (true, DEFAULT_SCENARIO.to_string())
            }
        } else {
            (false, DEFAULT_SCENARIO.to_string())
        };

        Self::with_state(enabled, scenario)
    }

    fn with_state(enabled: bool, scenario: String) -> Self {
        Self {
            inner: Mutex::new(PreviewModel {
                enabled,
                scenario,
                running_override: None,
                config_yaml: PREVIEW_CONFIG.to_string(),
                revision: 1,
                last_apply: Value::Null,
                removed_peers: HashSet::new(),
                connected_peer: None,
            }),
        }
    }

    pub fn status(&self) -> PreviewStatus {
        let model = self.inner.lock().unwrap();
        PreviewStatus {
            available: crate::DEVELOPMENT_TOOLS_INCLUDED,
            enabled: crate::DEVELOPMENT_TOOLS_INCLUDED && model.enabled,
            scenario: model.scenario.clone(),
            scenarios: scenarios(),
        }
    }

    pub fn set(&self, enabled: bool, scenario: &str) -> Result<PreviewStatus, String> {
        if !crate::DEVELOPMENT_TOOLS_INCLUDED {
            return Err("Product Preview is available only in development builds.".into());
        }
        if !scenario_exists(scenario) {
            return Err(format!("Unknown Product Preview scenario: {scenario}"));
        }
        let mut model = self.inner.lock().unwrap();
        model.enabled = enabled;
        if model.scenario != scenario {
            model.scenario = scenario.to_string();
            model.running_override = None;
            model.removed_peers.clear();
            model.connected_peer = None;
        }
        drop(model);
        Ok(self.status())
    }

    pub fn snapshot(&self) -> Option<MonitorSnapshot> {
        let model = self.inner.lock().unwrap();
        if !crate::DEVELOPMENT_TOOLS_INCLUDED || !model.enabled {
            return None;
        }
        Some(snapshot_for(&model))
    }

    pub fn peers(&self) -> Option<Value> {
        let model = self.inner.lock().unwrap();
        if !crate::DEVELOPMENT_TOOLS_INCLUDED || !model.enabled {
            return None;
        }
        Some(json!({ "peers": peers_for(&model) }))
    }

    pub fn transports(&self) -> Option<Value> {
        if !self.status().enabled {
            return None;
        }
        Some(json!({ "transports": preview_transports() }))
    }

    pub fn mmp(&self) -> Option<Value> {
        let model = self.inner.lock().unwrap();
        if !crate::DEVELOPMENT_TOOLS_INCLUDED || !model.enabled {
            return None;
        }
        let losses = [0.001, 0.004, 0.0];
        let peers = peers_for(&model)
            .into_iter()
            .take(losses.len())
            .zip(losses)
            .filter_map(|(peer, loss)| {
                Some(json!({
                    "peer": peer.get("node_addr")?.as_str()?,
                    "display_name": peer.get("display_name")?.as_str()?,
                    "mode": "full",
                    "link_layer": {
                        "loss_rate": loss,
                        "smoothed_loss": loss,
                        "srtt_ms": 18.0,
                        "etx": 1.0 / (1.0 - loss)
                    }
                }))
            })
            .collect::<Vec<_>>();
        Some(json!({ "peers": peers, "sessions": [] }))
    }

    pub fn connect_peer(&self, npub: String, address: String, transport: String) -> Option<Value> {
        let mut model = self.inner.lock().unwrap();
        if !crate::DEVELOPMENT_TOOLS_INCLUDED || !model.enabled {
            return None;
        }
        model.removed_peers.remove(&npub);
        model.connected_peer = Some(json!({
            "npub": npub,
            "display_name": "Preview connection",
            "ipv6_addr": "fdc5:e354:55a4:8702:44d8:3302:1320:fc02",
            "connectivity": "Connected",
            "transport_type": transport,
            "transport_addr": address,
            "direction": "outbound",
            "tree_depth": 2
        }));
        Some(json!({ "preview": true }))
    }

    pub fn disconnect_peer(&self, npub: String) -> Option<Value> {
        let mut model = self.inner.lock().unwrap();
        if !crate::DEVELOPMENT_TOOLS_INCLUDED || !model.enabled {
            return None;
        }
        model.removed_peers.insert(npub.clone());
        if model
            .connected_peer
            .as_ref()
            .and_then(|peer| peer.get("npub"))
            .and_then(Value::as_str)
            == Some(npub.as_str())
        {
            model.connected_peer = None;
        }
        Some(json!({ "preview": true }))
    }

    pub fn service_status(&self) -> Option<ServiceStatus> {
        let model = self.inner.lock().unwrap();
        if !crate::DEVELOPMENT_TOOLS_INCLUDED || !model.enabled {
            return None;
        }
        Some(service_for(&model))
    }

    pub fn service_action(&self, command: &str) -> Option<ServiceStatus> {
        let mut model = self.inner.lock().unwrap();
        if !crate::DEVELOPMENT_TOOLS_INCLUDED || !model.enabled {
            return None;
        }
        match command {
            "start" => model.running_override = Some(true),
            "stop" => model.running_override = Some(false),
            "restart" => model.running_override = Some(true),
            "repair" => {
                model.scenario = DEFAULT_SCENARIO.into();
                model.running_override = Some(true);
            }
            "remove_keep_data" => {
                model.scenario = "fresh_install".into();
                model.running_override = None;
            }
            _ => return None,
        }
        Some(service_for(&model))
    }

    pub fn use_existing(&self) -> Option<ServiceStatus> {
        let mut model = self.inner.lock().unwrap();
        if !crate::DEVELOPMENT_TOOLS_INCLUDED || !model.enabled {
            return None;
        }
        model.scenario = "existing_ready".into();
        model.running_override = Some(true);
        Some(service_for(&model))
    }

    pub fn install(&self) -> Option<ServiceStatus> {
        let mut model = self.inner.lock().unwrap();
        if !crate::DEVELOPMENT_TOOLS_INCLUDED || !model.enabled {
            return None;
        }
        model.scenario = DEFAULT_SCENARIO.into();
        model.running_override = Some(true);
        Some(service_for(&model))
    }

    pub fn config(&self) -> Option<Result<Value, String>> {
        let model = self.inner.lock().unwrap();
        if !crate::DEVELOPMENT_TOOLS_INCLUDED || !model.enabled {
            return None;
        }
        if service_for(&model).ownership != "app_managed" {
            return Some(Err(
                "Move the existing FIPS installation into this app to edit its configuration."
                    .into(),
            ));
        }
        Some(Ok(config_snapshot(&model)))
    }

    pub fn validate_config(&self, expected_revision: &str, yaml: &str) -> Option<Value> {
        let model = self.inner.lock().unwrap();
        if !crate::DEVELOPMENT_TOOLS_INCLUDED || !model.enabled {
            return None;
        }
        let revision = revision(&model);
        if expected_revision != revision {
            return Some(json!({
                "valid": false,
                "errors": [{ "path": "/", "message": "The preview draft is stale. Reload it before reviewing changes." }],
                "diff": [],
                "warnings": [],
                "activation": null
            }));
        }
        if let Err(error) = serde_yaml::from_str::<serde_yaml::Value>(yaml) {
            return Some(json!({
                "valid": false,
                "errors": [{ "path": "/", "message": format!("Invalid YAML: {error}") }],
                "diff": [],
                "warnings": [],
                "activation": null
            }));
        }
        let changed = yaml != model.config_yaml;
        Some(json!({
            "valid": true,
            "errors": [],
            "yaml": yaml,
            "diff": if changed { json!([{
                "path": "/configuration",
                "before": "Current preview configuration",
                "after": "Edited preview configuration"
            }]) } else { json!([]) },
            "warnings": ["Product Preview validates this draft without writing to your Mac."],
            "activation": if changed { "restart" } else { "none" }
        }))
    }

    pub fn apply_config(
        &self,
        expected_revision: &str,
        yaml: String,
    ) -> Option<Result<Value, String>> {
        let mut model = self.inner.lock().unwrap();
        if !crate::DEVELOPMENT_TOOLS_INCLUDED || !model.enabled {
            return None;
        }
        if expected_revision != revision(&model) {
            return Some(Err(
                "The preview draft is stale. Reload it before applying.".into(),
            ));
        }
        let changed = yaml != model.config_yaml;
        model.config_yaml = yaml;
        model.revision += 1;
        let apply_id = format!("preview-apply-{}", model.revision);
        model.last_apply = json!({
            "apply_id": apply_id,
            "state": "applied",
            "updated_at_ms": now_ms()
        });
        Some(Ok(json!({
            "apply_id": apply_id,
            "revision": revision(&model),
            "activation": if changed { "restart" } else { "none" },
            "diff": if changed { json!([{
                "path": "/configuration",
                "before": "Current preview configuration",
                "after": "Edited preview configuration"
            }]) } else { json!([]) }
        })))
    }

    pub fn apply_status(&self) -> Option<Value> {
        let model = self.inner.lock().unwrap();
        if !crate::DEVELOPMENT_TOOLS_INCLUDED || !model.enabled {
            return None;
        }
        Some(model.last_apply.clone())
    }

    pub fn reset_config(&self, expected_revision: &str) -> Option<Result<Value, String>> {
        self.apply_config(expected_revision, PREVIEW_CONFIG.to_string())
    }
}

fn scenarios() -> Vec<PreviewScenario> {
    [
        ("managed_running", "Managed by this app · Running"),
        ("managed_stopped", "Managed by this app · Stopped"),
        ("existing_setup", "Existing installation · Enable controls"),
        ("existing_ready", "Existing installation · Ready"),
        ("fresh_install", "FIPS is not installed"),
        ("approval_required", "macOS approval required"),
        ("conflict", "Installation conflict"),
        ("rollback", "Recovered after rollback"),
        ("permission_denied", "Control socket permission denied"),
        ("incompatible", "Incompatible FIPS response"),
    ]
    .into_iter()
    .map(|(id, label)| PreviewScenario {
        id: id.into(),
        label: label.into(),
    })
    .collect()
}

fn scenario_exists(candidate: &str) -> bool {
    scenarios().iter().any(|scenario| scenario.id == candidate)
}

fn service_for(model: &PreviewModel) -> ServiceStatus {
    let running = model.running_override.unwrap_or(matches!(
        model.scenario.as_str(),
        "managed_running"
            | "existing_setup"
            | "existing_ready"
            | "conflict"
            | "rollback"
            | "permission_denied"
            | "incompatible"
    ));
    let (available, ownership, installation, registration, can_migrate, detail) = match model
        .scenario
        .as_str()
    {
        "existing_setup" => (
            false,
            "external",
            "standard",
            "not_registered",
            false,
            "The standard FIPS installation was found. Monitoring is available now; enable management to configure and control it.",
        ),
        "existing_ready" => (
            true,
            "app_managed",
            "standard",
            "enabled",
            false,
            "Using the standard FIPS installation in /usr/local.",
        ),
        "fresh_install" => (
            false,
            "none",
            "not_installed",
            "not_registered",
            false,
            "FIPS is ready to be installed from this app.",
        ),
        "approval_required" => (
            false,
            "none",
            "not_installed",
            "requires_approval",
            false,
            "macOS needs one-time approval before FIPS can run in the background.",
        ),
        "conflict" => (
            false,
            "conflict",
            "conflict",
            "enabled",
            true,
            "Two FIPS services are registered. Repair the installation before continuing.",
        ),
        "rollback" => (
            true,
            "app_managed",
            "standard",
            "enabled",
            false,
            "The last configuration could not start. FIPS restored the previous configuration and is running normally.",
        ),
        "permission_denied" => (
            true,
            "app_managed",
            "standard",
            "enabled",
            false,
            "The node is running, but this account cannot access its control socket.",
        ),
        "incompatible" => (
            true,
            "app_managed",
            "standard",
            "enabled",
            false,
            "The installed FIPS node returned a response this app does not understand.",
        ),
        _ => (
            true,
            "app_managed",
            "standard",
            "enabled",
            false,
            "Using the standard FIPS installation in /usr/local.",
        ),
    };
    ServiceStatus {
        available,
        state: if running { "running" } else { "stopped" }.into(),
        enabled: available && running,
        loaded: available,
        running: available && running,
        controller_version: available.then_some(4),
        pid: (available && running).then_some(4242),
        last_exit_status: (model.scenario == "rollback").then_some(78),
        detail: Some(detail.into()),
        ownership: ownership.into(),
        installation: installation.into(),
        can_migrate,
        config_path: match installation {
            "standard" => Some("/usr/local/etc/fips/fips.yaml".into()),
            _ => None,
        },
        registration: registration.into(),
    }
}

fn snapshot_for(model: &PreviewModel) -> MonitorSnapshot {
    let service = service_for(model);
    let node_running = match model.scenario.as_str() {
        "existing_setup" => true,
        "conflict" => true,
        _ => service.running,
    };
    let health = match model.scenario.as_str() {
        "rollback" | "conflict" => "degraded",
        "permission_denied" => "permission_denied",
        "incompatible" => "incompatible",
        _ if node_running => "healthy",
        _ => "stopped",
    };
    let detail = match model.scenario.as_str() {
        "rollback" => "The previous configuration was restored after a failed restart.",
        "conflict" => {
            "Two FIPS services were detected. Repair the installation before making changes."
        }
        "permission_denied" => "Access to the FIPS control socket was denied for this account.",
        "incompatible" => "The installed FIPS node returned an incompatible status response.",
        _ if node_running => "The FIPS node is running normally.",
        _ => "FIPS is turned off. Use the lifecycle controls to start it.",
    };
    let peers = peers_for(model);
    let status = (node_running
        && !matches!(
            model.scenario.as_str(),
            "permission_denied" | "incompatible"
        ))
    .then(|| {
        json!({
            "state": "Running",
            "version": "0.5.0-dev (preview42)",
            "npub": "npub1fipsproductpreview6t8k3up93ms7gk2u4n5h8p0qy9wd3jlcx7r2vaf",
            "ipv6_addr": "fdc5:e354:55a4:8702:bb31:3af5:57c9:959b",
            "uptime_secs": 262980,
            "estimated_mesh_size": 1847,
            "peer_count": peers.len(),
            "session_count": 5,
            "transport_count": preview_transports().len(),
            "tun_state": "Running",
            "tun_name": "fips0",
            "effective_ipv6_mtu": 1280,
            "is_root": false,
            "is_leaf_only": false,
            "persistent": true,
            "sparklines": {
                "peer_count": [3,3,4,4,4,5,5,5,4,5,5,5],
                "active_sessions": [3,3,3,4,4,4,4,5,5,5,5,5],
                "tree_depth": [2,2,3,2,2,2,3,2,2,2,2,2],
                "mesh_size": [1710,1724,1738,1762,1780,1798,1811,1807,1829,1838,1842,1847],
                "bytes_in": [14,22,18,31,27,38,33,45,41,53,47,58],
                "bytes_out": [9,12,15,13,22,19,28,24,31,29,35,38],
                "loss_rate": [0.004,0.002,0.003,0.002,0.001,0.002,0.001,0.001,0.002,0.001,0.001,0.001]
            }
        })
    });
    MonitorSnapshot {
        preview: true,
        health: health.into(),
        detail: detail.into(),
        socket_path: PREVIEW_SOCKET.into(),
        checked_at_ms: now_ms(),
        status,
        capabilities: Some(json!({ "preview": true, "config_api_version": 1 })),
        configuration_supported: service.ownership == "app_managed",
        service,
    }
}

fn peers_for(model: &PreviewModel) -> Vec<Value> {
    let mut peers = vec![
        json!({
            "node_addr": "44d833021320fc01",
            "npub": "npub1studio4h7qzud0q0w3h87zz4p2h8w2k9f3m5e6a7s8d9f0g1h2j3k",
            "display_name": "Studio gateway",
            "ipv6_addr": "fdc5:e354:55a4:8702:44d8:3302:1320:fc01",
            "connectivity": "Connected",
            "transport_type": "udp",
            "transport_addr": "192.168.1.42:2121",
            "direction": "outbound",
            "is_parent": true,
            "tree_depth": 1
        }),
        json!({
            "node_addr": "44d833021320fc03",
            "npub": "npub1office8m4k2j7s9d5f0g3h6l1p4q8w2e7r9t5y3u6i0o2a4s7d8f",
            "display_name": "Office Mac",
            "ipv6_addr": "fdc5:e354:55a4:8702:44d8:3302:1320:fc03",
            "connectivity": "Connected",
            "transport_type": "tcp",
            "transport_addr": "10.20.30.18:8443",
            "direction": "inbound",
            "is_child": true,
            "tree_depth": 2
        }),
        json!({
            "node_addr": "44d833021320fc04",
            "npub": "npub1remote3k7s2d9f5g8h1j4l6p0q3w7e9r2t5y8u1i4o6a0s3d7f9g",
            "display_name": "Remote relay",
            "ipv6_addr": "fdc5:e354:55a4:8702:44d8:3302:1320:fc04",
            "connectivity": "Connected",
            "transport_type": "tor",
            "transport_addr": "previewrelayexample.onion:9050",
            "direction": "outbound",
            "tree_depth": 3
        }),
    ];
    if let Some(peer) = model.connected_peer.clone() {
        peers.push(peer);
    }
    peers.retain(|peer| {
        peer.get("npub")
            .and_then(Value::as_str)
            .is_none_or(|npub| !model.removed_peers.contains(npub))
    });
    peers
}

fn preview_transports() -> Vec<Value> {
    vec![
        json!({
            "transport_id": 1,
            "type": "udp",
            "state": "Running",
            "name": "LAN UDP",
            "local_addr": "0.0.0.0:2121",
            "mtu": 1280
        }),
        json!({
            "transport_id": 2,
            "type": "tcp",
            "state": "Running",
            "name": "TCP listener",
            "local_addr": "0.0.0.0:8443",
            "mtu": 1280
        }),
        json!({
            "transport_id": 3,
            "type": "tor",
            "state": "Running",
            "name": "Tor hidden service",
            "local_addr": "127.0.0.1:9050",
            "onion_address": "previewrelayexample.onion",
            "mtu": 1280
        }),
    ]
}

fn revision(model: &PreviewModel) -> String {
    format!("preview-revision-{}", model.revision)
}

fn config_snapshot(model: &PreviewModel) -> Value {
    json!({
        "source": "managed",
        "base_path": "/Library/Application Support/FIPS/standard-management/fips.original.yaml",
        "managed_path": "/usr/local/etc/fips/fips.yaml",
        "revision": revision(model),
        "yaml": model.config_yaml,
        "secrets": { "identity": "preserved" },
        "last_apply": model.last_apply
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(debug_assertions)]
    #[test]
    fn preview_scenarios_are_safe_and_distinct() {
        if !crate::DEVELOPMENT_TOOLS_INCLUDED {
            return;
        }
        let preview = PreviewController::with_state(true, "existing_setup".into());
        let snapshot = preview.snapshot().unwrap();
        assert_eq!(snapshot.health, "healthy");
        assert_eq!(snapshot.service.ownership, "external");
        assert!(!snapshot.service.available);

        let service = preview.use_existing().unwrap();
        assert!(service.available);
        assert_eq!(service.ownership, "app_managed");
        assert_eq!(service.installation, "standard");

        let service = preview.service_action("stop").unwrap();
        assert!(!service.running);
        assert_eq!(preview.snapshot().unwrap().health, "stopped");

        let mmp = preview.mmp().unwrap();
        assert_eq!(mmp["peers"].as_array().unwrap().len(), 3);
        assert!(mmp["peers"][0]["link_layer"]["smoothed_loss"].is_number());
    }

    #[cfg(debug_assertions)]
    #[test]
    fn preview_can_switch_back_to_the_live_node() {
        if !crate::DEVELOPMENT_TOOLS_INCLUDED {
            return;
        }
        let preview = PreviewController::with_state(true, DEFAULT_SCENARIO.into());
        assert!(preview.snapshot().is_some());

        let status = preview.set(false, DEFAULT_SCENARIO).unwrap();
        assert!(!status.enabled);
        assert!(preview.snapshot().is_none());

        let status = preview.set(true, "managed_stopped").unwrap();
        assert!(status.enabled);
        assert_eq!(status.scenario, "managed_stopped");
        assert_eq!(preview.snapshot().unwrap().health, "stopped");
    }

    #[test]
    fn release_gate_is_reported_in_the_preview_status() {
        let preview = PreviewController::with_state(true, DEFAULT_SCENARIO.into());
        assert_eq!(
            preview.status().available,
            crate::DEVELOPMENT_TOOLS_INCLUDED
        );
        if !crate::DEVELOPMENT_TOOLS_INCLUDED {
            assert!(preview.set(true, DEFAULT_SCENARIO).is_err());
            assert!(preview.snapshot().is_none());
        }
    }

    #[cfg(debug_assertions)]
    #[test]
    fn preview_configuration_validates_and_applies_without_disk_io() {
        if !crate::DEVELOPMENT_TOOLS_INCLUDED {
            return;
        }
        let preview = PreviewController::with_state(true, DEFAULT_SCENARIO.into());
        let config = preview.config().unwrap().unwrap();
        let revision = config["revision"].as_str().unwrap();
        let edited = format!("{}\n# preview edit\n", config["yaml"].as_str().unwrap());
        assert_eq!(
            preview.validate_config(revision, &edited).unwrap()["valid"],
            true
        );
        let result = preview.apply_config(revision, edited).unwrap().unwrap();
        assert_eq!(result["activation"], "restart");
        assert_eq!(preview.apply_status().unwrap()["state"], "applied");
    }
}
