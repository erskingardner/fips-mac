use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{io::ErrorKind, path::PathBuf, sync::atomic::Ordering, time::Duration};
use tauri::State;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    time::timeout,
};

const DEFAULT_SOCKET_PATH: &str = "/var/run/fips-mac/service.sock";
const APP_CONTROL_SOCKET: &str = "/var/run/fips/control.sock";
const CONTROLLER_PLIST: &str = "com.paper-robin.fips-mac.service-control.plist";
const NODE_PLIST: &str = "com.paper-robin.fips-mac.node.plist";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(12);
const MAX_RESPONSE_BYTES: u64 = 16 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceStatus {
    pub available: bool,
    pub state: String,
    pub enabled: bool,
    pub loaded: bool,
    pub running: bool,
    pub controller_version: Option<u32>,
    pub pid: Option<u32>,
    pub last_exit_status: Option<i32>,
    pub detail: Option<String>,
    pub ownership: String,
    pub installation: String,
    pub can_migrate: bool,
    pub config_path: Option<String>,
    pub registration: String,
}

impl ServiceStatus {
    pub fn checking() -> Self {
        Self {
            available: false,
            state: "unknown".into(),
            enabled: false,
            loaded: false,
            running: false,
            controller_version: None,
            pid: None,
            last_exit_status: None,
            detail: Some("Looking for the FIPS service controller…".into()),
            ownership: "unknown".into(),
            installation: "checking".into(),
            can_migrate: false,
            config_path: None,
            registration: registration_status(),
        }
    }

    fn unavailable(error: &ServiceError) -> Self {
        let mut status = Self {
            detail: Some(error.message.clone()),
            ..Self::checking()
        };
        if std::path::Path::new("/Library/LaunchDaemons/com.fips.daemon.plist").is_file() {
            status.ownership = "external".into();
            status.installation = "external".into();
            status.can_migrate = true;
            status.config_path = Some("/usr/local/etc/fips/fips.yaml".into());
            status.detail = Some(
                "An existing package-managed FIPS node was found. FIPS can use or migrate it."
                    .into(),
            );
        } else {
            status.ownership = "none".into();
            status.installation = "not_installed".into();
            if status.registration == "bundle_incomplete" {
                status.detail = Some(
                    "This development or App Store build monitors FIPS but does not include the app-managed node bundle."
                        .into(),
                );
            }
        }
        status
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct ServiceError {
    pub kind: String,
    pub message: String,
}

impl ServiceError {
    fn new(kind: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            message: message.into(),
        }
    }

    fn io(operation: &str, error: std::io::Error) -> Self {
        let kind = match error.kind() {
            ErrorKind::PermissionDenied => "permission_denied",
            ErrorKind::NotFound | ErrorKind::ConnectionRefused => "not_installed",
            _ => "unavailable",
        };
        let message = match kind {
            "not_installed" => {
                "FIPS's background service is not running yet.".to_string()
            }
            "permission_denied" => {
                "Service control access was denied. An administrator can repair FIPS from onboarding."
                    .to_string()
            }
            _ => format!("{operation}: {error}"),
        };
        Self::new(kind, message)
    }
}

#[derive(Debug, Deserialize)]
struct WireStatus {
    controller_version: Option<u32>,
    state: String,
    enabled: bool,
    loaded: bool,
    running: bool,
    pid: Option<u32>,
    last_exit_status: Option<i32>,
    #[serde(default = "default_ownership")]
    ownership: String,
    #[serde(default = "default_installation")]
    installation: String,
    #[serde(default)]
    can_migrate: bool,
    config_path: Option<String>,
    detail: Option<String>,
}

fn default_ownership() -> String {
    "external".into()
}

fn default_installation() -> String {
    "external".into()
}

impl From<WireStatus> for ServiceStatus {
    fn from(status: WireStatus) -> Self {
        Self {
            available: true,
            state: status.state,
            enabled: status.enabled,
            loaded: status.loaded,
            running: status.running,
            controller_version: status.controller_version,
            pid: status.pid,
            last_exit_status: status.last_exit_status,
            detail: status.detail,
            ownership: status.ownership,
            installation: status.installation,
            can_migrate: status.can_migrate,
            config_path: status.config_path,
            registration: registration_status(),
        }
    }
}

#[derive(Clone)]
pub struct ServiceClient {
    socket_path: PathBuf,
    timeout: Duration,
}

impl ServiceClient {
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

    pub async fn command(&self, command: &str) -> Result<ServiceStatus, ServiceError> {
        let mut request = serde_json::to_vec(&json!({ "command": command }))
            .map_err(|error| ServiceError::new("protocol", error.to_string()))?;
        request.push(b'\n');

        let stream = timeout(self.timeout, UnixStream::connect(&self.socket_path))
            .await
            .map_err(|_| ServiceError::new("timeout", "service controller connection timed out"))?
            .map_err(|error| ServiceError::io("connect", error))?;
        let (reader, mut writer) = stream.into_split();
        timeout(self.timeout, writer.write_all(&request))
            .await
            .map_err(|_| ServiceError::new("timeout", "service controller write timed out"))?
            .map_err(|error| ServiceError::io("write", error))?;
        writer
            .shutdown()
            .await
            .map_err(|error| ServiceError::io("shutdown", error))?;

        let mut line = String::new();
        let mut reader = BufReader::new(reader).take(MAX_RESPONSE_BYTES + 1);
        let read = timeout(self.timeout, reader.read_line(&mut line))
            .await
            .map_err(|_| ServiceError::new("timeout", "service controller response timed out"))?
            .map_err(|error| ServiceError::io("read", error))?;
        if read == 0 {
            return Err(ServiceError::new(
                "protocol",
                "service controller closed without a response",
            ));
        }
        if read as u64 > MAX_RESPONSE_BYTES {
            return Err(ServiceError::new(
                "protocol",
                "service controller response exceeded 16 KiB",
            ));
        }

        let response: Value = serde_json::from_str(line.trim_end()).map_err(|error| {
            ServiceError::new("protocol", format!("invalid service response: {error}"))
        })?;
        match response.get("status").and_then(Value::as_str) {
            Some("ok") => {
                let status = serde_json::from_value::<WireStatus>(
                    response.get("data").cloned().unwrap_or(Value::Null),
                )
                .map_err(|error| {
                    ServiceError::new(
                        "protocol",
                        format!("invalid service status payload: {error}"),
                    )
                })?;
                Ok(status.into())
            }
            Some("error") => Err(ServiceError::new(
                "service",
                response
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("service controller rejected the request"),
            )),
            _ => Err(ServiceError::new(
                "protocol",
                "service response did not contain a recognized status",
            )),
        }
    }
}

pub fn resolve_service_socket_path() -> PathBuf {
    std::env::var_os("FIPS_MAC_SERVICE_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SOCKET_PATH))
}

fn registration_status() -> String {
    #[cfg(target_os = "macos")]
    {
        use smappservice_rs::{AppService, ServiceStatus as RegistrationStatus, ServiceType};
        let controller = AppService::new(ServiceType::Daemon {
            plist_name: CONTROLLER_PLIST,
        });
        let node = AppService::new(ServiceType::Daemon {
            plist_name: NODE_PLIST,
        });
        match (controller.status(), node.status()) {
            (RegistrationStatus::Enabled, RegistrationStatus::Enabled) => "enabled".into(),
            (RegistrationStatus::RequiresApproval, _)
            | (_, RegistrationStatus::RequiresApproval) => "requires_approval".into(),
            (RegistrationStatus::NotFound, _) | (_, RegistrationStatus::NotFound) => {
                "bundle_incomplete".into()
            }
            _ => "not_registered".into(),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        "unsupported".into()
    }
}

#[tauri::command]
pub async fn get_node_installation(
    state: State<'_, crate::AppState>,
) -> Result<ServiceStatus, ServiceError> {
    Ok(query_status(state.service_socket_path.clone()).await)
}

#[tauri::command]
pub async fn use_existing_node(
    state: State<'_, crate::AppState>,
) -> Result<ServiceStatus, ServiceError> {
    #[cfg(target_os = "macos")]
    {
        register_daemon(CONTROLLER_PLIST)?;
        if registration_status() == "requires_approval" {
            return Err(ServiceError::new(
                "requires_approval",
                "macOS registered the FIPS services, but an administrator still needs to approve FIPS in System Settings → General → Login Items.",
            ));
        }
        wait_for_controller(&state).await?;
        let status = ServiceClient::new(state.service_socket_path.clone())
            .command("show_service")
            .await?;
        if status.ownership != "external" {
            return Err(ServiceError::new(
                "not_installed",
                "No package-managed FIPS installation was found.",
            ));
        }
        state.refresh.notify_one();
        Ok(status)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = state;
        Err(ServiceError::new(
            "unsupported",
            "Existing node adoption is available only on macOS.",
        ))
    }
}

#[tauri::command]
pub async fn register_node_service(
    state: State<'_, crate::AppState>,
    migrate: bool,
) -> Result<ServiceStatus, ServiceError> {
    #[cfg(target_os = "macos")]
    {
        register_daemon(CONTROLLER_PLIST)?;
        register_daemon(NODE_PLIST)?;
        if registration_status() == "requires_approval" {
            return Err(ServiceError::new(
                "requires_approval",
                "macOS registered the FIPS services, but an administrator still needs to approve FIPS in System Settings → General → Login Items.",
            ));
        }
        wait_for_controller(&state).await?;

        let preparation = if migrate {
            "migrate"
        } else {
            "prepare_install"
        };
        if let Err(error) = perform_service_action(&state, preparation).await {
            let client = ServiceClient::new(state.service_socket_path.clone());
            let _ = client.command("rollback_migration").await;
            let _ = unregister_daemon(NODE_PLIST);
            return Err(error);
        }

        let client = ServiceClient::new(state.service_socket_path.clone());
        let previous_control_path = state.socket_path.lock().unwrap().clone();
        let started = match client.command("start").await {
            Ok(status) => status,
            Err(error) => {
                let _ = client.command("rollback_migration").await;
                let _ = unregister_daemon(NODE_PLIST);
                return Err(error);
            }
        };
        if let Err(error) = wait_for_node_control(&state).await {
            let _ = client.command("rollback_migration").await;
            let _ = unregister_daemon(NODE_PLIST);
            *state.socket_path.lock().unwrap() = previous_control_path;
            return Err(error);
        }
        if migrate && let Err(error) = client.command("finish_migration").await {
            let _ = client.command("rollback_migration").await;
            let _ = unregister_daemon(NODE_PLIST);
            *state.socket_path.lock().unwrap() = previous_control_path;
            return Err(error);
        }
        state.refresh.notify_one();
        Ok(started)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (state, migrate);
        Err(ServiceError::new(
            "unsupported",
            "App-managed FIPS is available only on macOS 13 or newer.",
        ))
    }
}

#[tauri::command]
pub async fn repair_node_service(
    state: State<'_, crate::AppState>,
) -> Result<ServiceStatus, ServiceError> {
    perform_service_action(&state, "repair").await
}

#[tauri::command]
pub async fn remove_node_service(
    state: State<'_, crate::AppState>,
) -> Result<ServiceStatus, ServiceError> {
    #[cfg(target_os = "macos")]
    {
        use smappservice_rs::{AppService, ServiceManagementError, ServiceType};
        if state.service_socket_path.exists() {
            let removal = perform_service_action(&state, "remove_keep_data").await?;
            if removal.ownership == "external" {
                let _ = wait_for_node_control(&state).await;
            }
        }
        for plist in [NODE_PLIST, CONTROLLER_PLIST] {
            let service = AppService::new(ServiceType::Daemon { plist_name: plist });
            match service.unregister() {
                Ok(()) | Err(ServiceManagementError::JobNotFound) => {}
                Err(error) => {
                    return Err(ServiceError::new(
                        "registration",
                        format!("macOS could not unregister {plist}: {error}"),
                    ));
                }
            }
        }
        state.refresh.notify_one();
        Ok(ServiceStatus {
            detail: Some(
                "The app-managed node was removed. Its configuration was preserved.".into(),
            ),
            registration: registration_status(),
            ..ServiceStatus::checking()
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = state;
        Err(ServiceError::new(
            "unsupported",
            "Node removal is only available on macOS.",
        ))
    }
}

#[tauri::command]
pub fn open_background_settings() {
    #[cfg(target_os = "macos")]
    smappservice_rs::AppService::open_system_settings_login_items();
}

async fn wait_for_controller(state: &crate::AppState) -> Result<(), ServiceError> {
    let deadline = tokio::time::Instant::now() + DEFAULT_TIMEOUT;
    loop {
        match ServiceClient::new(state.service_socket_path.clone())
            .command("show_service")
            .await
        {
            Ok(_) => return Ok(()),
            Err(error) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(250)).await;
                let _ = error;
            }
            Err(_) => {
                return Err(ServiceError::new(
                    "requires_approval",
                    "macOS registered the background service, but it still needs approval in System Settings → General → Login Items.",
                ));
            }
        }
    }
}

async fn wait_for_node_control(state: &crate::AppState) -> Result<PathBuf, ServiceError> {
    let current = state.socket_path.lock().unwrap().clone();
    let mut candidates = vec![
        PathBuf::from(APP_CONTROL_SOCKET),
        current,
        PathBuf::from("/tmp/fips-control.sock"),
    ];
    candidates.dedup();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        for candidate in &candidates {
            if !candidate.exists() {
                continue;
            }
            if crate::control::ControlClient::with_timeout(
                candidate.clone(),
                Duration::from_secs(1),
            )
            .query("show_status")
            .await
            .is_ok()
            {
                *state.socket_path.lock().unwrap() = candidate.clone();
                return Ok(candidate.clone());
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(ServiceError::new(
                "startup_failed",
                "The bundled FIPS node started but did not open a usable control socket. The installation was rolled back.",
            ));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[cfg(target_os = "macos")]
fn register_daemon(plist: &'static str) -> Result<(), ServiceError> {
    use smappservice_rs::{AppService, ServiceManagementError, ServiceType};
    let service = AppService::new(ServiceType::Daemon { plist_name: plist });
    match service.register() {
        Ok(()) | Err(ServiceManagementError::AlreadyRegistered) => Ok(()),
        Err(error) => Err(ServiceError::new(
            "registration",
            format!("macOS could not register {plist}: {error}"),
        )),
    }
}

#[cfg(target_os = "macos")]
fn unregister_daemon(plist: &'static str) -> Result<(), ServiceError> {
    use smappservice_rs::{AppService, ServiceManagementError, ServiceType};
    let service = AppService::new(ServiceType::Daemon { plist_name: plist });
    match service.unregister() {
        Ok(()) | Err(ServiceManagementError::JobNotFound) => Ok(()),
        Err(error) => Err(ServiceError::new(
            "registration",
            format!("macOS could not unregister {plist}: {error}"),
        )),
    }
}

pub async fn query_status(path: PathBuf) -> ServiceStatus {
    match ServiceClient::new(path).command("show_service").await {
        Ok(status) => status,
        Err(error) => ServiceStatus::unavailable(&error),
    }
}

pub async fn perform_service_action(
    state: &crate::AppState,
    command: &str,
) -> Result<ServiceStatus, ServiceError> {
    if state
        .service_action_busy
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(ServiceError::new(
            "busy",
            "Another FIPS service operation is already in progress.",
        ));
    }

    let result = ServiceClient::new(state.service_socket_path.clone())
        .command(command)
        .await;
    state.service_action_busy.store(false, Ordering::Release);
    state.refresh.notify_one();
    result
}

#[tauri::command]
pub async fn set_fips_service_running(
    state: State<'_, crate::AppState>,
    running: bool,
) -> Result<ServiceStatus, ServiceError> {
    perform_service_action(&state, if running { "start" } else { "stop" }).await
}

#[tauri::command]
pub async fn restart_fips_service(
    state: State<'_, crate::AppState>,
) -> Result<ServiceStatus, ServiceError> {
    perform_service_action(&state, "restart").await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use tokio::{io::AsyncWriteExt, net::UnixListener};

    #[tokio::test]
    async fn parses_service_status_from_controller() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("service.sock");
        let listener = UnixListener::bind(&path).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut line = String::new();
            BufReader::new(&mut stream)
                .read_line(&mut line)
                .await
                .unwrap();
            assert_eq!(line, "{\"command\":\"show_service\"}\n");
            stream
                .write_all(
                    b"{\"status\":\"ok\",\"data\":{\"controller_version\":1,\"state\":\"running\",\"enabled\":true,\"loaded\":true,\"running\":true,\"pid\":42}}\n",
                )
                .await
                .unwrap();
        });

        let status = ServiceClient::new(path)
            .command("show_service")
            .await
            .unwrap();
        assert!(status.available);
        assert!(status.running);
        assert_eq!(status.pid, Some(42));
    }

    #[tokio::test]
    async fn reports_response_timeouts() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("service.sock");
        let listener = UnixListener::bind(&path).unwrap();
        tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(1)).await;
        });

        let error = ServiceClient::with_timeout(path, Duration::from_millis(20))
            .command("show_service")
            .await
            .unwrap_err();
        assert_eq!(error.kind, "timeout");
    }
}
