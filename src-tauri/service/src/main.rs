//! Privileged lifecycle controller bundled with FIPS for Mac.
//!
//! It accepts a small, fixed NDJSON protocol over a local Unix socket and
//! manages only the standard FIPS launchd job and configuration. It links the
//! exact pinned FIPS crate solely to validate configuration with the same types
//! as the installed node.

#[cfg(target_os = "macos")]
mod config_manager;

#[cfg(target_os = "macos")]
mod macos {
    use crate::config_manager::{
        ApplyActivation, ConfigManager, redact_message_secrets, validation_error_path,
    };
    use serde::{Deserialize, Serialize};
    use serde_json::{Value, json};
    use std::{
        ffi::CString,
        fs::{self, File, OpenOptions},
        io::{self, Write},
        net::{IpAddr, Ipv4Addr, Ipv6Addr},
        os::{
            unix::ffi::OsStrExt,
            unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        },
        path::{Path, PathBuf},
        process::Output,
        sync::Arc,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };
    use tokio::{
        io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
        net::{UnixListener, UnixStream},
        process::Command,
        sync::Mutex,
        time::timeout,
    };

    const SOCKET_PATH: &str = "/var/run/fips-mac/service.sock";
    const SOCKET_DIR: &str = "/var/run/fips-mac";
    const CONTROL_SOCKET: &str = "/var/run/fips/control.sock";
    const ADMIN_GROUP: &str = "admin";

    const FIPS_LABEL: &str = "com.fips.daemon";
    const FIPS_TARGET: &str = "system/com.fips.daemon";
    const FIPS_PLIST: &str = "/Library/LaunchDaemons/com.fips.daemon.plist";
    const FIPS_CONFIG: &str = "/usr/local/etc/fips/fips.yaml";

    const APP_STATE_DIR: &str = "/Library/Application Support/FIPS/standard-management";
    const RESOLVER_DIR: &str = "/etc/resolver";
    const RESOLVER_PATH: &str = "/etc/resolver/fips";

    const REQUEST_LIMIT: usize = 256 * 1024;
    const IO_TIMEOUT: Duration = Duration::from_secs(12);

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        command: String,
        #[serde(default)]
        params: Option<Value>,
    }

    #[derive(Debug, Serialize)]
    struct Response {
        status: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    }

    impl Response {
        fn ok(data: impl Serialize) -> Self {
            Self {
                status: "ok",
                data: serde_json::to_value(data).ok(),
                message: None,
            }
        }

        fn error(message: impl Into<String>) -> Self {
            Self {
                status: "error",
                data: None,
                message: Some(message.into()),
            }
        }

        fn is_ok(&self) -> bool {
            self.status == "ok"
        }
    }

    struct DispatchOutcome {
        response: Response,
        activation: Option<ApplyActivation>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ConfigDraftParams {
        expected_revision: String,
        yaml: String,
    }

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ConfigResetParams {
        expected_revision: String,
    }

    #[derive(Debug, Clone, Serialize)]
    struct ServiceStatus {
        controller_version: u32,
        state: &'static str,
        enabled: bool,
        loaded: bool,
        running: bool,
        ownership: &'static str,
        installation: &'static str,
        can_migrate: bool,
        config_path: Option<&'static str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pid: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        last_exit_status: Option<i32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    }

    #[derive(Debug, Clone)]
    struct LaunchStatus {
        enabled: bool,
        loaded: bool,
        running: bool,
        pid: Option<u32>,
        last_exit_status: Option<i32>,
    }

    #[derive(Debug, Deserialize, Default)]
    #[serde(default)]
    struct EffectiveConfig {
        dns: DnsConfig,
    }

    #[derive(Debug, Deserialize)]
    #[serde(default)]
    struct DnsConfig {
        enabled: bool,
        bind_addr: String,
        port: u16,
    }

    impl Default for DnsConfig {
        fn default() -> Self {
            Self {
                enabled: true,
                bind_addr: "::1".into(),
                port: 5354,
            }
        }
    }

    pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
        if unsafe { libc::geteuid() } != 0 {
            return Err("fips-mac-service must run as root".into());
        }

        let listener = bind_socket(Path::new(SOCKET_PATH))?;
        let operation_lock = Arc::new(Mutex::new(()));
        loop {
            let (stream, _) = listener.accept().await?;
            let operation_lock = operation_lock.clone();
            tokio::spawn(async move {
                let _ = handle_connection(stream, operation_lock).await;
            });
        }
    }

    fn bind_socket(path: &Path) -> io::Result<UnixListener> {
        ensure_directory(Path::new(SOCKET_DIR), 0o2770)?;
        if let Ok(metadata) = fs::symlink_metadata(path) {
            if !metadata.file_type().is_socket() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("refusing to replace non-socket path {}", path.display()),
                ));
            }
            if std::os::unix::net::UnixStream::connect(path).is_ok() {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    format!("service controller already listening at {}", path.display()),
                ));
            }
            fs::remove_file(path)?;
        }

        let listener = UnixListener::bind(path)?;
        set_admin_permissions(path, 0o660)?;
        Ok(listener)
    }

    async fn handle_connection(
        stream: UnixStream,
        operation_lock: Arc<Mutex<()>>,
    ) -> io::Result<()> {
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let mut request_line = Vec::new();
        let read = timeout(
            IO_TIMEOUT,
            (&mut reader)
                .take((REQUEST_LIMIT + 1) as u64)
                .read_until(b'\n', &mut request_line),
        )
        .await;

        let mut outcome = match read {
            Err(_) => DispatchOutcome::error("request timed out"),
            Ok(Err(error)) => DispatchOutcome::error(format!("could not read request: {error}")),
            Ok(Ok(_)) if request_line.len() > REQUEST_LIMIT => {
                DispatchOutcome::error("request exceeds 256 KiB limit")
            }
            Ok(Ok(0)) => DispatchOutcome::error("empty request"),
            Ok(Ok(_)) => match serde_json::from_slice::<Request>(&request_line) {
                Ok(request) => dispatch(&request, operation_lock.clone()).await,
                Err(error) => DispatchOutcome::error(format!("invalid request: {error}")),
            },
        };

        let mut encoded = serde_json::to_vec(&outcome.response)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        encoded.push(b'\n');
        timeout(IO_TIMEOUT, writer.write_all(&encoded))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "response timed out"))??;
        writer.shutdown().await?;
        if outcome.response.is_ok()
            && let Some(activation) = outcome.activation.take()
        {
            tokio::spawn(async move {
                let _guard = operation_lock.lock().await;
                activate_config(activation).await;
            });
        }
        Ok(())
    }

    impl DispatchOutcome {
        fn response(response: Response) -> Self {
            Self {
                response,
                activation: None,
            }
        }

        fn error(message: impl Into<String>) -> Self {
            Self::response(Response::error(message))
        }
    }

    async fn dispatch(request: &Request, operation_lock: Arc<Mutex<()>>) -> DispatchOutcome {
        match request.command.as_str() {
            "show_service" => return lifecycle_response(service_status().await),
            "show_config" => return config_snapshot(),
            "show_config_apply" => return show_config_apply(),
            _ => {}
        }

        let _guard = operation_lock.lock().await;
        let result = match request.command.as_str() {
            "prepare_install" => prepare_management().await,
            "migrate" | "finish_migration" | "rollback_migration" => Err(
                "FIPS now uses the standard installation directly; migration is not required."
                    .into(),
            ),
            "repair" => repair().await,
            "start" => start_service().await,
            "stop" => stop_service().await,
            "restart" => restart_service().await,
            "remove_keep_data" => remove_keep_data().await,
            "validate_config" => return validate_config(request),
            "apply_config" => return apply_config(request).await,
            "reset_managed_config" => return reset_config(request).await,
            _ => {
                return DispatchOutcome::error(format!("unknown command: {}", request.command));
            }
        };
        lifecycle_response(result)
    }

    fn lifecycle_response(result: Result<ServiceStatus, String>) -> DispatchOutcome {
        match result {
            Ok(status) => DispatchOutcome::response(Response::ok(status)),
            Err(error) => DispatchOutcome::error(error),
        }
    }

    fn require_managed_installation() -> Result<(), String> {
        if !Path::new(FIPS_PLIST).exists() || !Path::new(FIPS_CONFIG).exists() {
            return Err(
                "The standard FIPS installation was not found. Install FIPS, then try again."
                    .into(),
            );
        }
        prepare_management_files()
    }

    async fn require_exclusive_app_ownership() -> Result<(), String> {
        require_managed_installation()
    }

    fn config_manager() -> ConfigManager {
        ConfigManager::new(PathBuf::from(FIPS_CONFIG), PathBuf::from(APP_STATE_DIR))
    }

    fn config_snapshot() -> DispatchOutcome {
        let result = require_managed_installation().and_then(|()| config_manager().snapshot());
        match result {
            Ok(snapshot) => DispatchOutcome::response(Response::ok(snapshot)),
            Err(error) => DispatchOutcome::error(error),
        }
    }

    fn show_config_apply() -> DispatchOutcome {
        let result = require_managed_installation().map(|()| config_manager().last_apply());
        match result {
            Ok(apply) => DispatchOutcome::response(Response::ok(apply)),
            Err(error) => DispatchOutcome::error(error),
        }
    }

    fn parse_params<T: for<'de> Deserialize<'de>>(request: &Request) -> Result<T, String> {
        serde_json::from_value(request.params.clone().unwrap_or(Value::Null))
            .map_err(|error| format!("invalid {} parameters: {error}", request.command))
    }

    fn validate_config(request: &Request) -> DispatchOutcome {
        let result: Result<Value, String> = (|| {
            require_managed_installation()?;
            let params: ConfigDraftParams = parse_params(request)?;
            let manager = config_manager();
            match manager.validate(&params.expected_revision, &params.yaml) {
                Ok(validated) => Ok(json!({
                    "valid": true,
                    "errors": [],
                    "warnings": [],
                    "yaml": validated.redacted_yaml,
                    "diff": validated.diff,
                    "activation": validated.activation,
                })),
                Err(error) => Ok(json!({
                    "valid": false,
                    "errors": [{
                        "path": validation_error_path(&error),
                        "message": redact_message_secrets(
                            &manager.redact_error_message(&error),
                            &params.yaml,
                        ),
                    }],
                    "warnings": [],
                    "yaml": params.yaml,
                    "diff": [],
                    "activation": "none",
                })),
            }
        })();
        match result {
            Ok(value) => DispatchOutcome::response(Response::ok(value)),
            Err(error) => DispatchOutcome::error(error),
        }
    }

    async fn apply_config(request: &Request) -> DispatchOutcome {
        let result: Result<_, String> = async {
            require_exclusive_app_ownership().await?;
            let params: ConfigDraftParams = parse_params(request)?;
            let manager = config_manager();
            match manager.apply(&params.expected_revision, &params.yaml) {
                Ok((result, _)) => Ok(result),
                Err(error) => Err(redact_message_secrets(
                    &manager.redact_error_message(&error),
                    &params.yaml,
                )),
            }
        }
        .await;
        match result {
            Ok(result) => DispatchOutcome {
                activation: Some(result.activation),
                response: Response::ok(result),
            },
            Err(error) => DispatchOutcome::error(error),
        }
    }

    async fn reset_config(request: &Request) -> DispatchOutcome {
        let result: Result<_, String> = async {
            require_exclusive_app_ownership().await?;
            let params: ConfigResetParams = parse_params(request)?;
            config_manager().reset(&params.expected_revision)
        }
        .await;
        match result {
            Ok(result) => DispatchOutcome {
                activation: Some(result.activation),
                response: Response::ok(result),
            },
            Err(error) => DispatchOutcome::error(error),
        }
    }

    async fn activate_config(activation: ApplyActivation) {
        let manager = config_manager();
        if activation == ApplyActivation::None {
            if let Err(error) = sync_dns_resolver() {
                let _ = manager.mark_failed(manager.redact_error_message(&error));
            } else {
                let _ = manager.mark_applied();
            }
            return;
        }

        let launch = match launch_status(FIPS_LABEL, FIPS_TARGET).await {
            Ok(status) => status,
            Err(error) => {
                let _ = manager.rollback_pending(manager.redact_error_message(&error));
                return;
            }
        };
        if !launch.running {
            // A semantic draft is persisted but cannot be called active until
            // the next successful node start proves the runtime configuration.
            return;
        }

        let activation_result = async {
            restart_target(FIPS_LABEL, FIPS_TARGET, Some(FIPS_PLIST)).await?;
            wait_for_control_ready().await?;
            sync_dns_resolver()?;
            Ok::<(), String>(())
        }
        .await;
        match activation_result {
            Ok(()) => {
                let _ = manager.mark_applied();
            }
            Err(error) => {
                let redacted = manager.redact_error_message(&error);
                if manager.rollback_pending(redacted.clone()).unwrap_or(false) {
                    let restored = async {
                        restart_target(FIPS_LABEL, FIPS_TARGET, Some(FIPS_PLIST)).await?;
                        wait_for_control_ready().await?;
                        sync_dns_resolver()?;
                        Ok::<(), String>(())
                    }
                    .await;
                    if let Err(restore_error) = restored {
                        let combined = format!(
                            "{redacted}; the previous configuration was restored but failed to restart: {}",
                            manager.redact_error_message(&restore_error)
                        );
                        let _ = manager.mark_failed(combined);
                    }
                }
            }
        }
    }

    async fn wait_for_control_ready() -> Result<(), String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        loop {
            let ready = async {
                let mut stream = UnixStream::connect(CONTROL_SOCKET).await.ok()?;
                stream
                    .write_all(b"{\"command\":\"show_status\"}\n")
                    .await
                    .ok()?;
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                timeout(Duration::from_secs(1), reader.read_line(&mut line))
                    .await
                    .ok()?
                    .ok()?;
                serde_json::from_str::<Value>(&line)
                    .ok()?
                    .get("status")
                    .and_then(Value::as_str)
                    .is_some_and(|status| status == "ok")
                    .then_some(())
            }
            .await;
            if ready.is_some() {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err("FIPS did not open its control socket within 20 seconds".into());
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    async fn prepare_management() -> Result<ServiceStatus, String> {
        require_managed_installation()?;
        sync_dns_resolver()?;
        service_status().await
    }

    fn prepare_management_files() -> Result<(), String> {
        ensure_directory(Path::new(APP_STATE_DIR), 0o750).map_err(|e| e.to_string())?;
        require_root_regular_file(Path::new(FIPS_CONFIG))?;
        config_manager().bootstrap(None)
    }

    async fn repair() -> Result<ServiceStatus, String> {
        prepare_management().await
    }

    async fn start_service() -> Result<ServiceStatus, String> {
        require_managed_installation()?;
        activate_standard_service(false).await?;
        service_status().await
    }

    async fn stop_service() -> Result<ServiceStatus, String> {
        require_managed_installation()?;
        stop_target(FIPS_LABEL, FIPS_TARGET, Some(FIPS_PLIST)).await?;
        service_status().await
    }

    async fn restart_service() -> Result<ServiceStatus, String> {
        require_managed_installation()?;
        activate_standard_service(true).await?;
        service_status().await
    }

    async fn activate_standard_service(restart: bool) -> Result<(), String> {
        let manager = config_manager();
        let activation = async {
            if restart {
                restart_target(FIPS_LABEL, FIPS_TARGET, Some(FIPS_PLIST)).await?;
            } else {
                start_target(FIPS_LABEL, FIPS_TARGET, Some(FIPS_PLIST)).await?;
            }
            wait_for_control_ready().await?;
            sync_dns_resolver()?;
            Ok::<(), String>(())
        }
        .await;
        match activation {
            Ok(()) => manager.mark_applied(),
            Err(error) => {
                let redacted = manager.redact_error_message(&error);
                if manager.rollback_pending(redacted.clone()).unwrap_or(false) {
                    restart_target(FIPS_LABEL, FIPS_TARGET, Some(FIPS_PLIST)).await?;
                    wait_for_control_ready().await?;
                    sync_dns_resolver()?;
                    return Err(format!(
                        "The new configuration could not start and was rolled back: {redacted}"
                    ));
                }
                Err(redacted)
            }
        }
    }

    async fn remove_keep_data() -> Result<ServiceStatus, String> {
        Err("Removing FIPS is handled by the standard FIPS uninstaller; this app will not remove an existing installation.".into())
    }

    async fn service_status() -> Result<ServiceStatus, String> {
        let installed = Path::new(FIPS_PLIST).exists() && Path::new(FIPS_CONFIG).exists();
        let status = launch_status(FIPS_LABEL, FIPS_TARGET).await?;
        Ok(ServiceStatus {
            controller_version: 4,
            state: if status.running { "running" } else { "stopped" },
            enabled: status.enabled,
            loaded: status.loaded,
            running: status.running,
            ownership: if installed { "app_managed" } else { "none" },
            installation: if installed {
                "standard"
            } else {
                "not_installed"
            },
            can_migrate: false,
            config_path: installed.then_some(FIPS_CONFIG),
            pid: status.pid,
            last_exit_status: status.last_exit_status,
            detail: Some(if installed {
                "Using the standard FIPS installation in /usr/local.".into()
            } else {
                "The standard FIPS installation was not found.".into()
            }),
        })
    }

    async fn start_target(
        label: &str,
        target: &str,
        legacy_plist: Option<&str>,
    ) -> Result<LaunchStatus, String> {
        run_launchctl(&["enable", target]).await?;
        let current = launch_status(label, target).await?;
        if current.loaded || legacy_plist.is_none() {
            run_launchctl(&["kickstart", "-k", target]).await?;
        } else if let Some(plist) = legacy_plist {
            require_root_regular_file(Path::new(plist))?;
            run_launchctl(&["bootstrap", "system", plist]).await?;
        } else {
            return Err("The standard FIPS service is not registered with macOS.".into());
        }
        wait_for(label, target, |status| status.running, "start").await
    }

    async fn stop_target(
        label: &str,
        target: &str,
        _legacy_plist: Option<&str>,
    ) -> Result<LaunchStatus, String> {
        run_launchctl(&["disable", target]).await?;
        let current = launch_status(label, target).await?;
        if current.running {
            run_launchctl(&["kill", "SIGTERM", target]).await?;
        }
        wait_for(label, target, |status| !status.running, "stop").await
    }

    async fn restart_target(
        label: &str,
        target: &str,
        legacy_plist: Option<&str>,
    ) -> Result<LaunchStatus, String> {
        run_launchctl(&["enable", target]).await?;
        let current = launch_status(label, target).await?;
        if current.loaded || legacy_plist.is_none() {
            run_launchctl(&["kickstart", "-k", target]).await?;
        } else if let Some(plist) = legacy_plist {
            require_root_regular_file(Path::new(plist))?;
            run_launchctl(&["bootstrap", "system", plist]).await?;
        } else {
            return Err("The bundled FIPS service is not registered with macOS.".into());
        }
        wait_for(label, target, |status| status.running, "restart").await
    }

    async fn wait_for(
        label: &str,
        target: &str,
        predicate: impl Fn(&LaunchStatus) -> bool,
        operation: &str,
    ) -> Result<LaunchStatus, String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(12);
        loop {
            let status = launch_status(label, target).await?;
            if predicate(&status) {
                return Ok(status);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!("FIPS did not {operation} within 12 seconds"));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    async fn launch_status(label: &str, target: &str) -> Result<LaunchStatus, String> {
        let disabled = service_is_disabled(label).await?;
        let output = Command::new("/bin/launchctl")
            .args(["print", target])
            .output()
            .await
            .map_err(|error| format!("could not inspect {label}: {error}"))?;
        let (loaded, running, pid, last_exit_status) = if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            (
                true,
                field(&text, "state").is_some_and(|value| value == "running"),
                field(&text, "pid").and_then(|value| value.parse().ok()),
                field(&text, "last exit code").and_then(|value| value.parse().ok()),
            )
        } else {
            (false, false, None, None)
        };
        Ok(LaunchStatus {
            enabled: !disabled,
            loaded,
            running,
            pid,
            last_exit_status,
        })
    }

    async fn service_is_disabled(label: &str) -> Result<bool, String> {
        let output = Command::new("/bin/launchctl")
            .args(["print-disabled", "system"])
            .output()
            .await
            .map_err(|error| format!("could not inspect launchd disabled state: {error}"))?;
        if !output.status.success() {
            return Err(output_error("launchctl print-disabled", &output));
        }
        let text = String::from_utf8_lossy(&output.stdout);
        Ok(is_service_disabled(&text, label))
    }

    fn is_service_disabled(text: &str, label: &str) -> bool {
        text.lines()
            .any(|line| line.contains(&format!("\"{label}\"")) && line.contains("=> disabled"))
    }

    fn field<'a>(text: &'a str, name: &str) -> Option<&'a str> {
        text.lines().find_map(|line| {
            let (key, value) = line.trim().split_once(" = ")?;
            (key == name).then_some(value.trim())
        })
    }

    async fn run_launchctl(args: &[&str]) -> Result<(), String> {
        let output = Command::new("/bin/launchctl")
            .args(args)
            .output()
            .await
            .map_err(|error| format!("could not run launchctl: {error}"))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(output_error(
                &format!("launchctl {}", args.join(" ")),
                &output,
            ))
        }
    }

    fn output_error(operation: &str, output: &Output) -> String {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let message = stderr.trim();
        if message.is_empty() {
            format!("{operation} failed with {}", output.status)
        } else {
            format!("{operation} failed: {message}")
        }
    }

    fn ensure_directory(path: &Path, mode: u32) -> io::Result<()> {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{} is not a real directory", path.display()),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir_all(path)?,
            Err(error) => return Err(error),
        }
        set_admin_permissions(path, mode)
    }

    fn require_real_directory(path: &Path) -> Result<(), String> {
        let metadata = fs::symlink_metadata(path)
            .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!("{} is not a real directory", path.display()));
        }
        Ok(())
    }

    fn require_root_regular_file(path: &Path) -> Result<(), String> {
        let metadata = fs::symlink_metadata(path)
            .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(format!("{} is not a regular file", path.display()));
        }
        if metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
            return Err(format!(
                "{} must be root-owned and not group/other writable",
                path.display()
            ));
        }
        Ok(())
    }

    fn set_admin_permissions(path: &Path, mode: u32) -> io::Result<()> {
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
        let group = CString::new(ADMIN_GROUP).expect("fixed group name has no NUL");
        let group = unsafe { libc::getgrnam(group.as_ptr()) };
        if group.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "macOS admin group was not found",
            ));
        }
        let gid = unsafe { (*group).gr_gid };
        let path = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL"))?;
        if unsafe { libc::chown(path.as_ptr(), 0, gid) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn sync_dns_resolver() -> Result<(), String> {
        let config_path = Path::new(FIPS_CONFIG);
        if !config_path.exists() {
            return Ok(());
        }
        require_root_regular_file(config_path)?;
        let config: EffectiveConfig = serde_yaml::from_slice(
            &fs::read(config_path)
                .map_err(|error| format!("could not read {}: {error}", config_path.display()))?,
        )
        .map_err(|error| format!("could not parse DNS configuration: {error}"))?;
        if !config.dns.enabled {
            remove_dns_resolver()?;
            return Ok(());
        }
        let bind: IpAddr = config
            .dns
            .bind_addr
            .parse()
            .map_err(|_| "dns.bind_addr must be an IP address".to_string())?;
        let nameserver = match bind {
            IpAddr::V4(address) if address.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(address) if address.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
            address => address,
        };
        let contents = format!(
            "# Managed by FIPS\nnameserver {nameserver}\nport {}\n",
            config.dns.port
        );
        ensure_root_directory(Path::new(RESOLVER_DIR), 0o755)?;
        install_dns_resolver(contents.as_bytes())
    }

    fn install_dns_resolver(bytes: &[u8]) -> Result<(), String> {
        let path = Path::new(RESOLVER_PATH);
        if let Ok(existing) = fs::read(path) {
            if existing == bytes {
                return Ok(());
            }
            if !existing.starts_with(b"# Managed by FIPS\n") {
                const LEGACY_RESOLVER: &[u8] = b"nameserver ::1\nport 5354\n";
                if existing != LEGACY_RESOLVER {
                    return Err(format!(
                        "{} already exists and is not managed by FIPS",
                        path.display()
                    ));
                }
            }
        }
        atomic_write_root(path, bytes, 0o644)?;
        flush_dns_cache();
        Ok(())
    }

    fn remove_dns_resolver() -> Result<(), String> {
        let path = Path::new(RESOLVER_PATH);
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(
                format!("refusing to remove unsafe resolver path {}", path.display()),
            ),
            Ok(_) => {
                let contents = fs::read(path)
                    .map_err(|error| format!("could not read {}: {error}", path.display()))?;
                if !contents.starts_with(b"# Managed by FIPS\n") {
                    return Ok(());
                }
                fs::remove_file(path)
                    .map_err(|error| format!("could not remove {}: {error}", path.display()))?;
                File::open(RESOLVER_DIR)
                    .and_then(|directory| directory.sync_all())
                    .map_err(|error| format!("could not sync {RESOLVER_DIR}: {error}"))?;
                flush_dns_cache();
                Ok(())
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("could not inspect {}: {error}", path.display())),
        }
    }

    fn flush_dns_cache() {
        let _ = std::process::Command::new("/usr/bin/dscacheutil")
            .arg("-flushcache")
            .status();
        let _ = std::process::Command::new("/usr/bin/killall")
            .args(["-HUP", "mDNSResponder"])
            .status();
    }

    fn ensure_root_directory(path: &Path, mode: u32) -> Result<(), String> {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(format!("{} is not a real directory", path.display()));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir_all(path)
                .map_err(|error| format!("could not create {}: {error}", path.display()))?,
            Err(error) => return Err(format!("could not inspect {}: {error}", path.display())),
        }
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .map_err(|error| format!("could not set permissions on {}: {error}", path.display()))?;
        chown_root(path, 0)
    }

    fn atomic_write_root(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
        let parent = path
            .parent()
            .ok_or_else(|| "resolver path has no parent".to_string())?;
        require_real_directory(parent)?;
        if let Ok(existing) = fs::read(path)
            && existing == bytes
        {
            return Ok(());
        }
        if let Ok(metadata) = fs::symlink_metadata(path)
            && (metadata.file_type().is_symlink() || !metadata.is_file())
        {
            return Err(format!(
                "refusing to replace unsafe path {}",
                path.display()
            ));
        }
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let temporary = parent.join(format!(
            ".fips-mac-resolver-{}-{suffix}.tmp",
            std::process::id()
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&temporary)
            .map_err(|error| format!("could not create {}: {error}", temporary.display()))?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("could not persist resolver: {error}"))?;
        chown_root(&temporary, 0)?;
        fs::rename(&temporary, path)
            .map_err(|error| format!("could not install resolver: {error}"))?;
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("could not sync {}: {error}", parent.display()))
    }

    fn chown_root(path: &Path, gid: libc::gid_t) -> Result<(), String> {
        let path = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| "path contains a NUL".to_string())?;
        if unsafe { libc::chown(path.as_ptr(), 0, gid) } != 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use serde_json::json;

        #[test]
        fn rejects_lifecycle_paths_from_clients() {
            assert!(serde_json::from_value::<Request>(json!({"command": "start"})).is_ok());
            assert!(
                serde_json::from_value::<Request>(json!({
                    "command": "start",
                    "path": "/tmp/other.plist"
                }))
                .is_err()
            );
        }

        #[test]
        fn parses_launchctl_status_fields() {
            let output = "\tstate = running\n\tpid = 1234\n\tlast exit code = 0\n";
            assert_eq!(field(output, "state"), Some("running"));
            assert_eq!(field(output, "pid"), Some("1234"));
            assert_eq!(field(output, "last exit code"), Some("0"));
        }

        #[test]
        fn recognizes_disabled_labels_exactly() {
            let output = "disabled services = {\n\t\"com.example.node\" => disabled\n}";
            assert!(is_service_disabled(output, "com.example.node"));
            assert!(!is_service_disabled(output, "com.example"));
        }
    }
}

#[cfg(target_os = "macos")]
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    macos::run().await
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("fips-mac-service is only supported on macOS");
}
