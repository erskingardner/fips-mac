use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::atomic::Ordering,
    time::Duration,
};
use tauri::State;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    process::Command,
    time::timeout,
};

const DEFAULT_SOCKET_PATH: &str = "/var/run/fips-mac/service.sock";
const APP_CONTROL_SOCKET: &str = "/var/run/fips/control.sock";
const CONTROLLER_PLIST: &str = "com.paper-robin.fips-mac.service-control.plist";
const STANDARD_FIPS_PLIST: &str = "/Library/LaunchDaemons/com.fips.daemon.plist";
const STANDARD_FIPS_CONFIG: &str = "/usr/local/etc/fips/fips.yaml";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(12);
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;

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
        if std::path::Path::new(STANDARD_FIPS_PLIST).is_file()
            && std::path::Path::new(STANDARD_FIPS_CONFIG).is_file()
        {
            status.ownership = "external".into();
            status.installation = "standard".into();
            status.can_migrate = false;
            status.config_path = Some("/usr/local/etc/fips/fips.yaml".into());
            status.detail = Some(if status.registration == "bundle_incomplete" {
                "The standard FIPS installation was found. This development build can monitor it but does not include the management helper."
                    .into()
            } else {
                "The standard FIPS installation was found. Monitoring is available now; enable management to control and configure it."
                    .into()
            });
        } else {
            status.ownership = "none".into();
            status.installation = "not_installed".into();
            if status.registration == "bundle_incomplete" {
                status.detail = Some(
                    "This development build can monitor FIPS but does not include the installer or management helper."
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
        let payload = self.request(command, None).await?;
        let status = serde_json::from_value::<WireStatus>(payload).map_err(|error| {
            ServiceError::new(
                "protocol",
                format!("invalid service status payload: {error}"),
            )
        })?;
        Ok(status.into())
    }

    async fn query(&self, command: &str) -> Result<Value, ServiceError> {
        self.request(command, None).await
    }

    async fn query_with_params(&self, command: &str, params: Value) -> Result<Value, ServiceError> {
        self.request(command, Some(params)).await
    }

    async fn request(&self, command: &str, params: Option<Value>) -> Result<Value, ServiceError> {
        let envelope = match params {
            Some(params) => json!({ "command": command, "params": params }),
            None => json!({ "command": command }),
        };
        let mut request = serde_json::to_vec(&envelope)
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
                "service controller response exceeded 1 MiB",
            ));
        }

        let response: Value = serde_json::from_str(line.trim_end()).map_err(|error| {
            ServiceError::new("protocol", format!("invalid service response: {error}"))
        })?;
        match response.get("status").and_then(Value::as_str) {
            Some("ok") => Ok(response.get("data").cloned().unwrap_or(Value::Null)),
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

        match bundled_service_readiness() {
            BundleReadiness::Incomplete => return "bundle_incomplete".into(),
            BundleReadiness::OutsideApplications => return "app_not_installed".into(),
            BundleReadiness::Ready => {}
        }
        let controller = AppService::new(ServiceType::Daemon {
            plist_name: CONTROLLER_PLIST,
        });
        match controller.status() {
            RegistrationStatus::Enabled => "enabled".into(),
            RegistrationStatus::RequiresApproval => "requires_approval".into(),
            RegistrationStatus::NotRegistered => "not_registered".into(),
            // macOS can report NotFound before this bundled daemon has ever
            // been registered. Bundle readiness above is the reliable check
            // for missing resources; registration will return the actionable
            // error if ServiceManagement still cannot load the helper.
            RegistrationStatus::NotFound => "not_registered".into(),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        "unsupported".into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BundleReadiness {
    Ready,
    OutsideApplications,
    Incomplete,
}

#[cfg(target_os = "macos")]
fn bundled_service_readiness() -> BundleReadiness {
    std::env::current_exe()
        .ok()
        .map_or(BundleReadiness::Incomplete, |executable| {
            bundle_readiness_for_executable(&executable, installer_package_name())
        })
}

fn bundle_readiness_for_executable(executable: &Path, package_name: &str) -> BundleReadiness {
    let Some(contents) = executable
        .parent()
        .filter(|directory| directory.file_name().is_some_and(|name| name == "MacOS"))
        .and_then(Path::parent)
        .filter(|directory| directory.file_name().is_some_and(|name| name == "Contents"))
    else {
        return BundleReadiness::Incomplete;
    };
    let controller = contents
        .join("Library")
        .join("LaunchDaemons")
        .join(CONTROLLER_PLIST);
    let installer = contents.join("Resources").join(package_name);
    if !controller.is_file() || !installer.is_file() {
        return BundleReadiness::Incomplete;
    }
    let installed = contents
        .parent()
        .and_then(Path::parent)
        .is_some_and(|directory| directory == Path::new("/Applications"));
    if installed {
        BundleReadiness::Ready
    } else {
        BundleReadiness::OutsideApplications
    }
}

#[tauri::command]
pub async fn get_node_installation(
    state: State<'_, crate::AppState>,
) -> Result<ServiceStatus, ServiceError> {
    if let Some(status) = state.preview.service_status() {
        return Ok(status);
    }
    Ok(query_status(state.service_socket_path.clone()).await)
}

#[tauri::command]
pub async fn use_existing_node(
    state: State<'_, crate::AppState>,
) -> Result<ServiceStatus, ServiceError> {
    if let Some(status) = state.preview.use_existing() {
        state.refresh.notify_one();
        return Ok(status);
    }
    #[cfg(target_os = "macos")]
    {
        if !standard_fips_is_installed() {
            return Err(ServiceError::new(
                "not_installed",
                "The standard FIPS installation was not found.",
            ));
        }
        register_daemon(CONTROLLER_PLIST)?;
        if registration_status() == "requires_approval" {
            return Err(ServiceError::new(
                "requires_approval",
                "macOS registered the FIPS management helper, but an administrator still needs to approve FIPS in System Settings → General → Login Items.",
            ));
        }
        wait_for_controller(&state).await?;
        let status = perform_service_action(&state, "prepare_install").await?;
        if status.ownership != "app_managed" {
            return Err(ServiceError::new(
                "not_installed",
                "The standard FIPS installation was not found.",
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
    if let Some(status) = state.preview.install() {
        let _ = migrate;
        state.refresh.notify_one();
        return Ok(status);
    }
    #[cfg(target_os = "macos")]
    {
        let _ = migrate;
        if !standard_fips_is_installed() {
            run_standard_installer().await?;
        }
        if !standard_fips_is_installed() {
            return Err(ServiceError::new(
                "install_cancelled",
                "FIPS was not installed. Complete the macOS Installer to continue.",
            ));
        }
        register_daemon(CONTROLLER_PLIST)?;
        if registration_status() == "requires_approval" {
            return Err(ServiceError::new(
                "requires_approval",
                "macOS registered the FIPS management helper, but an administrator still needs to approve FIPS in System Settings → General → Login Items.",
            ));
        }
        wait_for_controller(&state).await?;

        perform_service_action(&state, "prepare_install").await?;
        let client = ServiceClient::new(state.service_socket_path.clone());
        let started = match client.command("start").await {
            Ok(status) => status,
            Err(error) => return Err(error),
        };
        wait_for_node_control(&state).await?;
        state.refresh.notify_one();
        Ok(started)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (state, migrate);
        Err(ServiceError::new(
            "unsupported",
            "Managing the FIPS node from this app is available only on macOS 13 or newer.",
        ))
    }
}

fn standard_fips_is_installed() -> bool {
    std::path::Path::new(STANDARD_FIPS_PLIST).is_file()
        && std::path::Path::new(STANDARD_FIPS_CONFIG).is_file()
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
    if let Some(status) = state.preview.service_action("remove_keep_data") {
        state.refresh.notify_one();
        return Ok(status);
    }
    #[cfg(target_os = "macos")]
    {
        use smappservice_rs::{AppService, ServiceManagementError, ServiceType};
        let service = AppService::new(ServiceType::Daemon {
            plist_name: CONTROLLER_PLIST,
        });
        match service.unregister() {
            Ok(()) | Err(ServiceManagementError::JobNotFound) => {}
            Err(error) => {
                return Err(ServiceError::new(
                    "registration",
                    format!("macOS could not unregister the FIPS management helper: {error}"),
                ));
            }
        }
        state.refresh.notify_one();
        Ok(ServiceStatus {
            available: false,
            state: "unknown".into(),
            enabled: false,
            loaded: false,
            running: false,
            controller_version: None,
            pid: None,
            last_exit_status: None,
            detail: Some(
                "FIPS management was disabled. The standard FIPS installation is unchanged.".into(),
            ),
            ownership: "external".into(),
            installation: "standard".into(),
            can_migrate: false,
            config_path: Some(STANDARD_FIPS_CONFIG.into()),
            registration: registration_status(),
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
pub fn open_background_settings(state: State<'_, crate::AppState>) {
    if state.preview.status().enabled {
        return;
    }
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

#[cfg(target_os = "macos")]
async fn run_standard_installer() -> Result<(), ServiceError> {
    let executable = std::env::current_exe().map_err(|error| {
        ServiceError::new("installer", format!("Could not locate FIPS.app: {error}"))
    })?;
    let contents = executable
        .parent()
        .and_then(std::path::Path::parent)
        .ok_or_else(|| ServiceError::new("installer", "FIPS.app has an invalid bundle layout."))?;
    let package = contents.join("Resources").join(installer_package_name());
    if !package.is_file() {
        return Err(ServiceError::new(
            "bundle_incomplete",
            "This build does not include the standard FIPS installer.",
        ));
    }
    let output = Command::new("/usr/bin/open")
        .arg("-W")
        .arg(&package)
        .output()
        .await
        .map_err(|error| {
            ServiceError::new(
                "installer",
                format!("Could not open macOS Installer: {error}"),
            )
        })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(ServiceError::new(
            "installer",
            "macOS Installer did not complete successfully.",
        ))
    }
}

#[cfg(target_os = "macos")]
fn installer_package_name() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "fips-macos-arm64.pkg"
    } else {
        "fips-macos-x86_64.pkg"
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
                "FIPS started but did not open a usable control socket. Check /usr/local/etc/fips/fips.yaml and the FIPS logs.",
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
    if let Some(status) = state.preview.service_action(command) {
        state.refresh.notify_one();
        return Ok(status);
    }
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

#[tauri::command]
pub async fn get_config(state: State<'_, crate::AppState>) -> Result<Value, ServiceError> {
    if let Some(result) = state.preview.config() {
        return result.map_err(|message| ServiceError::new("preview", message));
    }
    ServiceClient::new(state.service_socket_path.clone())
        .query("show_config")
        .await
}

#[tauri::command]
pub async fn validate_config(
    state: State<'_, crate::AppState>,
    expected_revision: String,
    yaml: String,
) -> Result<Value, ServiceError> {
    if let Some(result) = state.preview.validate_config(&expected_revision, &yaml) {
        return Ok(result);
    }
    ServiceClient::new(state.service_socket_path.clone())
        .query_with_params(
            "validate_config",
            json!({ "expected_revision": expected_revision, "yaml": yaml }),
        )
        .await
}

async fn mutate_config(
    state: &crate::AppState,
    command: &str,
    params: Value,
) -> Result<Value, ServiceError> {
    if state
        .service_action_busy
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(ServiceError::new(
            "busy",
            "Another FIPS service or configuration operation is already in progress.",
        ));
    }
    let result = ServiceClient::new(state.service_socket_path.clone())
        .query_with_params(command, params)
        .await;
    state.service_action_busy.store(false, Ordering::Release);
    state.refresh.notify_one();
    result
}

#[tauri::command]
pub async fn apply_config(
    state: State<'_, crate::AppState>,
    expected_revision: String,
    yaml: String,
) -> Result<Value, ServiceError> {
    if let Some(result) = state.preview.apply_config(&expected_revision, yaml.clone()) {
        state.refresh.notify_one();
        return result.map_err(|message| ServiceError::new("preview", message));
    }
    mutate_config(
        &state,
        "apply_config",
        json!({ "expected_revision": expected_revision, "yaml": yaml }),
    )
    .await
}

#[tauri::command]
pub async fn get_apply_status(state: State<'_, crate::AppState>) -> Result<Value, ServiceError> {
    if let Some(status) = state.preview.apply_status() {
        return Ok(status);
    }
    ServiceClient::new(state.service_socket_path.clone())
        .query("show_config_apply")
        .await
}

#[tauri::command]
pub async fn reset_config(
    state: State<'_, crate::AppState>,
    expected_revision: String,
) -> Result<Value, ServiceError> {
    if let Some(result) = state.preview.reset_config(&expected_revision) {
        state.refresh.notify_one();
        return result.map_err(|message| ServiceError::new("preview", message));
    }
    mutate_config(
        &state,
        "reset_managed_config",
        json!({ "expected_revision": expected_revision }),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;
    use tokio::{io::AsyncWriteExt, net::UnixListener};

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn selects_the_native_standard_installer() {
        assert_eq!(installer_package_name(), "fips-macos-arm64.pkg");
    }

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    #[test]
    fn selects_the_native_standard_installer() {
        assert_eq!(installer_package_name(), "fips-macos-x86_64.pkg");
    }

    #[test]
    fn distinguishes_incomplete_and_uninstalled_app_bundles() {
        let directory = tempdir().unwrap();
        let contents = directory.path().join("FIPS.app/Contents");
        let executable = contents.join("MacOS/fips-mac");
        let controller = contents
            .join("Library/LaunchDaemons")
            .join(CONTROLLER_PLIST);
        let installer = contents.join("Resources/fips-macos-arm64.pkg");

        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(&executable, "app").unwrap();
        assert_eq!(
            bundle_readiness_for_executable(&executable, "fips-macos-arm64.pkg"),
            BundleReadiness::Incomplete
        );

        fs::create_dir_all(controller.parent().unwrap()).unwrap();
        fs::create_dir_all(installer.parent().unwrap()).unwrap();
        fs::write(controller, "plist").unwrap();
        fs::write(installer, "package").unwrap();
        assert_eq!(
            bundle_readiness_for_executable(&executable, "fips-macos-arm64.pkg"),
            BundleReadiness::OutsideApplications
        );
    }

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

    #[tokio::test]
    async fn sends_configuration_payloads_to_the_service_controller() {
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
            let request: Value = serde_json::from_str(line.trim()).unwrap();
            assert_eq!(request["command"], "validate_config");
            assert_eq!(request["params"]["expected_revision"], "abc123");
            assert_eq!(request["params"]["yaml"], "node: {}\n");
            stream
                .write_all(b"{\"status\":\"ok\",\"data\":{\"valid\":true}}\n")
                .await
                .unwrap();
        });

        let response = ServiceClient::new(path)
            .query_with_params(
                "validate_config",
                json!({ "expected_revision": "abc123", "yaml": "node: {}\n" }),
            )
            .await
            .unwrap();
        assert_eq!(response["valid"], true);
    }
}
