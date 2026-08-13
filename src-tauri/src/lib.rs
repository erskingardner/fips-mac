mod control;
mod preferences;
mod service;

use control::{ClientError, ControlClient, resolve_socket_path};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use service::{ServiceStatus, resolve_service_socket_path};
use std::{
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{
    AppHandle, Emitter, Manager, WebviewWindow,
    image::Image,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tokio::sync::Notify;

const TRAY_ID: &str = "fips";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorSnapshot {
    pub health: String,
    pub detail: String,
    pub socket_path: String,
    pub checked_at_ms: u128,
    pub status: Option<Value>,
    pub capabilities: Option<Value>,
    pub configuration_supported: bool,
    pub service: ServiceStatus,
}

impl MonitorSnapshot {
    fn starting(socket_path: &std::path::Path) -> Self {
        Self {
            health: "stopped".into(),
            detail: "Looking for the FIPS daemon…".into(),
            socket_path: socket_path.display().to_string(),
            checked_at_ms: now_ms(),
            status: None,
            capabilities: None,
            configuration_supported: false,
            service: ServiceStatus::checking(),
        }
    }
}

pub struct AppState {
    pub socket_path: Mutex<PathBuf>,
    pub service_socket_path: PathBuf,
    pub last_snapshot: Mutex<MonitorSnapshot>,
    pub refresh: Notify,
    pub service_action_busy: AtomicBool,
    onboarding_opened: AtomicBool,
}

struct TrayMenuState {
    status_item: MenuItem<tauri::Wry>,
    peer_item: MenuItem<tauri::Wry>,
    service_toggle_item: MenuItem<tauri::Wry>,
    service_restart_item: MenuItem<tauri::Wry>,
    launch_item: CheckMenuItem<tauri::Wry>,
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn classify_status(status: &Value) -> (&'static str, String) {
    let Some(object) = status.as_object() else {
        return (
            "incompatible",
            "The daemon returned an unrecognized status payload.".into(),
        );
    };
    let state = object
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let tun_state = object
        .get("tun_state")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !state.eq_ignore_ascii_case("running") {
        return (
            "degraded",
            if state.is_empty() {
                "The daemon is reachable but did not report a running state.".into()
            } else {
                format!("The FIPS node is {state}.")
            },
        );
    }
    if matches!(
        tun_state.to_ascii_lowercase().as_str(),
        "failed" | "error" | "down"
    ) {
        return (
            "degraded",
            format!("The node is running, but TUN is {tun_state}."),
        );
    }
    if let Some(lan) = object.get("lan_discovery").and_then(Value::as_object)
        && lan.get("enabled").and_then(Value::as_bool) == Some(true)
    {
        if let Some(warning) = lan
            .get("warnings")
            .and_then(Value::as_array)
            .and_then(|warnings| warnings.first())
            .and_then(Value::as_str)
        {
            return ("degraded", warning.to_string());
        }
        let discovery_state = lan
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        if !discovery_state.eq_ignore_ascii_case("running") {
            return (
                "degraded",
                format!("LAN discovery is enabled, but its runtime is {discovery_state}."),
            );
        }
    }
    ("healthy", "The FIPS node is running normally.".into())
}

fn snapshot_for_error(
    path: &std::path::Path,
    error: ClientError,
    service: ServiceStatus,
) -> MonitorSnapshot {
    let health = match error.kind.as_str() {
        "not_running" => "stopped",
        "permission_denied" => "permission_denied",
        "protocol" => "incompatible",
        _ => "degraded",
    };
    let detail = match health {
        "stopped" if service.available && !service.enabled && service.state == "stopped" =>
            "FIPS is turned off. Use the service switch to start it.".into(),
        "stopped" => "FIPS is not running or its control socket is not installed.".into(),
        "permission_denied" => {
            "Access was denied. Repair an app-managed node, or confirm this account belongs to the fips group for a package-managed node."
                .into()
        }
        "incompatible" => format!("The daemon response is incompatible: {}", error.message),
        _ => error.message,
    };
    MonitorSnapshot {
        health: health.into(),
        detail,
        socket_path: path.display().to_string(),
        checked_at_ms: now_ms(),
        status: None,
        capabilities: None,
        configuration_supported: false,
        service,
    }
}

async fn collect_snapshot(path: PathBuf, service_path: PathBuf) -> MonitorSnapshot {
    let service = service::query_status(service_path).await;
    let client = ControlClient::new(path.clone());
    match client.query("show_status").await {
        Ok(status) => {
            let (health, detail) = classify_status(&status);
            let capabilities = client.query("show_capabilities").await.ok();
            let configuration_supported = capabilities
                .as_ref()
                .and_then(|value| value.get("config_api_version"))
                .and_then(Value::as_u64)
                .is_some_and(|version| version >= 1);
            MonitorSnapshot {
                health: health.into(),
                detail,
                socket_path: client.socket_path().display().to_string(),
                checked_at_ms: now_ms(),
                status: Some(status),
                capabilities,
                configuration_supported,
                service,
            }
        }
        Err(error) => snapshot_for_error(&path, error, service),
    }
}

fn polling_interval(visible: bool) -> Duration {
    if visible {
        Duration::from_secs(2)
    } else {
        Duration::from_secs(10)
    }
}

async fn monitor(app: AppHandle) {
    loop {
        let path = app.state::<AppState>().socket_path.lock().unwrap().clone();
        let service_path = app.state::<AppState>().service_socket_path.clone();
        let snapshot = collect_snapshot(path, service_path).await;
        *app.state::<AppState>().last_snapshot.lock().unwrap() = snapshot.clone();
        update_tray(&app, &snapshot);
        let _ = app.emit("monitor://snapshot", &snapshot);

        if should_open_onboarding(&snapshot)
            && !app
                .state::<AppState>()
                .onboarding_opened
                .swap(true, Ordering::Relaxed)
        {
            show_window(&app, "onboarding");
        }

        let visible = app
            .get_webview_window("main")
            .and_then(|window| window.is_visible().ok())
            .unwrap_or(false);
        let state = app.state::<AppState>();
        tokio::select! {
            _ = tokio::time::sleep(polling_interval(visible)) => {}
            _ = state.refresh.notified() => {}
        }
    }
}

fn should_open_onboarding(snapshot: &MonitorSnapshot) -> bool {
    snapshot.health == "permission_denied"
        || snapshot.service.ownership == "conflict"
        || snapshot.service.registration == "requires_approval"
        || (snapshot.health == "stopped"
            && !(snapshot.service.available
                && !snapshot.service.enabled
                && snapshot.service.state == "stopped"))
}

fn update_tray(app: &AppHandle, snapshot: &MonitorSnapshot) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_icon_with_as_template(Some(icon_for_health(&snapshot.health)), true);
        let tooltip = format!("FIPS — {}", health_label(&snapshot.health));
        let _ = tray.set_tooltip(Some(tooltip));
    }
    if let Some(menu) = app.try_state::<TrayMenuState>() {
        let _ = menu
            .status_item
            .set_text(format!("Node: {}", health_label(&snapshot.health)));
        let peers = snapshot
            .status
            .as_ref()
            .and_then(|status| status.get("peer_count"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let _ = menu.peer_item.set_text(format!(
            "{peers} authenticated peer{}",
            if peers == 1 { "" } else { "s" }
        ));
        if snapshot.service.available {
            let _ = menu.service_toggle_item.set_enabled(true);
            let _ = menu
                .service_toggle_item
                .set_text(if snapshot.service.running {
                    "Stop FIPS"
                } else {
                    "Start FIPS"
                });
            let _ = menu
                .service_restart_item
                .set_enabled(snapshot.service.running);
        } else {
            let _ = menu
                .service_toggle_item
                .set_text("FIPS service controls unavailable");
            let _ = menu.service_toggle_item.set_enabled(false);
            let _ = menu.service_restart_item.set_enabled(false);
        }
    }
}

fn health_label(health: &str) -> &'static str {
    match health {
        "healthy" => "Healthy",
        "degraded" => "Degraded",
        "permission_denied" => "Permission denied",
        "incompatible" => "Incompatible",
        _ => "Stopped",
    }
}

pub(crate) fn show_window(app: &AppHandle, section: &str) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        let _ = window.emit("app://navigate", section);
    }
}

fn icon_for_health(health: &str) -> Image<'static> {
    const LOGICAL_SIZE: i32 = 18;
    const SCALE: i32 = 2;
    const PIXEL_SIZE: i32 = LOGICAL_SIZE * SCALE;
    let mut pixels = vec![0_u8; (PIXEL_SIZE * PIXEL_SIZE * 4) as usize];
    let mut put = |x: i32, y: i32| {
        if (0..LOGICAL_SIZE).contains(&x) && (0..LOGICAL_SIZE).contains(&y) {
            for offset_y in 0..SCALE {
                for offset_x in 0..SCALE {
                    let pixel_x = x * SCALE + offset_x;
                    let pixel_y = y * SCALE + offset_y;
                    let index = ((pixel_y * PIXEL_SIZE + pixel_x) * 4) as usize;
                    pixels[index..index + 4].copy_from_slice(&[0, 0, 0, 255]);
                }
            }
        }
    };

    match health {
        "healthy" => {
            // A compact, network-built F: recognizable as the product mark,
            // with only the strokes and nodes that survive at menu-bar size.
            line(&mut put, 3, 3, 15, 3);
            line(&mut put, 3, 3, 3, 15);
            line(&mut put, 3, 9, 11, 9);
            for (x, y, radius) in [
                (3, 3, 2),
                (9, 3, 1),
                (15, 3, 2),
                (3, 9, 2),
                (11, 9, 2),
                (3, 15, 2),
            ] {
                disc(&mut put, x, y, radius);
            }
        }
        "degraded" => {
            line(&mut put, 9, 2, 2, 15);
            line(&mut put, 9, 2, 16, 15);
            line(&mut put, 2, 15, 16, 15);
            line(&mut put, 9, 6, 9, 11);
            disc(&mut put, 9, 14, 1);
        }
        "permission_denied" => {
            for y in 8..16 {
                for x in 4..15 {
                    put(x, y);
                }
            }
            line(&mut put, 6, 8, 6, 6);
            line(&mut put, 12, 8, 12, 6);
            line(&mut put, 6, 6, 8, 4);
            line(&mut put, 8, 4, 10, 4);
            line(&mut put, 10, 4, 12, 6);
        }
        "incompatible" => {
            circle(&mut put, 9, 9, 7);
            line(&mut put, 4, 14, 14, 4);
        }
        _ => {
            circle(&mut put, 9, 9, 6);
            line(&mut put, 6, 9, 12, 9);
        }
    }
    Image::new_owned(pixels, PIXEL_SIZE as u32, PIXEL_SIZE as u32)
}

fn line(put: &mut impl FnMut(i32, i32), mut x0: i32, mut y0: i32, x1: i32, y1: i32) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut error = dx + dy;
    loop {
        put(x0, y0);
        if x0 == x1 && y0 == y1 {
            break;
        }
        let doubled = error * 2;
        if doubled >= dy {
            error += dy;
            x0 += sx;
        }
        if doubled <= dx {
            error += dx;
            y0 += sy;
        }
    }
}

fn disc(put: &mut impl FnMut(i32, i32), cx: i32, cy: i32, radius: i32) {
    for y in -radius..=radius {
        for x in -radius..=radius {
            if x * x + y * y <= radius * radius {
                put(cx + x, cy + y);
            }
        }
    }
}

fn circle(put: &mut impl FnMut(i32, i32), cx: i32, cy: i32, radius: i32) {
    let mut x = radius;
    let mut y = 0;
    let mut error = 1 - radius;
    while x >= y {
        for (dx, dy) in [
            (x, y),
            (y, x),
            (-y, x),
            (-x, y),
            (-x, -y),
            (-y, -x),
            (y, -x),
            (x, -y),
        ] {
            put(cx + dx, cy + dy);
        }
        y += 1;
        if error < 0 {
            error += 2 * y + 1;
        } else {
            x -= 1;
            error += 2 * (y - x) + 1;
        }
    }
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let status_item = MenuItem::with_id(app, "status", "Node: Checking…", false, None::<&str>)?;
    let peer_item = MenuItem::with_id(app, "peers", "0 authenticated peers", false, None::<&str>)?;
    let service_toggle_item = MenuItem::with_id(
        app,
        "service_toggle",
        "FIPS service controls unavailable",
        false,
        None::<&str>,
    )?;
    let service_restart_item =
        MenuItem::with_id(app, "service_restart", "Restart FIPS", false, None::<&str>)?;
    let open_item = MenuItem::with_id(app, "open", "Open Dashboard", true, None::<&str>)?;
    let settings_item = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let refresh_item = MenuItem::with_id(app, "refresh", "Refresh", true, None::<&str>)?;
    let launch_enabled = app.autolaunch().is_enabled().unwrap_or(false);
    let launch_item = CheckMenuItem::with_id(
        app,
        "launch",
        "Launch at Login",
        true,
        launch_enabled,
        None::<&str>,
    )?;
    let separator_one = PredefinedMenuItem::separator(app)?;
    let separator_two = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit FIPS", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &status_item,
            &peer_item,
            &service_toggle_item,
            &service_restart_item,
            &separator_one,
            &open_item,
            &settings_item,
            &refresh_item,
            &launch_item,
            &separator_two,
            &quit_item,
        ],
    )?;

    app.manage(TrayMenuState {
        status_item,
        peer_item,
        service_toggle_item,
        service_restart_item,
        launch_item: launch_item.clone(),
    });

    TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .icon(icon_for_health("stopped"))
        .icon_as_template(true)
        .tooltip("FIPS — Checking")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_window(app, "overview"),
            "settings" => show_window(app, "settings"),
            "refresh" => app.state::<AppState>().refresh.notify_one(),
            "service_toggle" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let running = {
                        let state = app.state::<AppState>();
                        state
                            .last_snapshot
                            .lock()
                            .map(|snapshot| snapshot.service.running)
                            .unwrap_or(false)
                    };
                    let command = if running { "stop" } else { "start" };
                    let state = app.state::<AppState>();
                    if let Err(error) = service::perform_service_action(&state, command).await {
                        show_window(&app, "overview");
                        let _ = app.emit("service://error", error.message);
                    }
                });
            }
            "service_restart" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let state = app.state::<AppState>();
                    if let Err(error) = service::perform_service_action(&state, "restart").await {
                        show_window(&app, "overview");
                        let _ = app.emit("service://error", error.message);
                    }
                });
            }
            "launch" => {
                if let Some(menu) = app.try_state::<TrayMenuState>() {
                    let enabled = menu.launch_item.is_checked().unwrap_or(false);
                    let result = if enabled {
                        app.autolaunch().enable()
                    } else {
                        app.autolaunch().disable()
                    };
                    if result.is_err() {
                        let _ = menu.launch_item.set_checked(!enabled);
                    }
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

fn configure_window(window: WebviewWindow) {
    let close_window = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = close_window.hide();
        }
    });
}

#[tauri::command]
fn copy_node_npub(app: AppHandle) -> Result<(), String> {
    let npub = {
        let state = app.state::<AppState>();
        let snapshot = state
            .last_snapshot
            .lock()
            .map_err(|_| "The node status is temporarily unavailable.".to_string())?;
        snapshot
            .status
            .as_ref()
            .and_then(|status| status.get("npub"))
            .and_then(Value::as_str)
            .filter(|npub| !npub.is_empty())
            .ok_or_else(|| "The FIPS node has not reported an npub yet.".to_string())?
            .to_string()
    };

    app.clipboard()
        .write_text(npub)
        .map_err(|error| format!("Could not copy the node npub: {error}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let socket_path = resolve_socket_path(None);
    let service_socket_path = resolve_service_socket_path();
    let initial_snapshot = MonitorSnapshot::starting(&socket_path);
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(AppState {
            socket_path: Mutex::new(socket_path),
            service_socket_path,
            last_snapshot: Mutex::new(initial_snapshot),
            refresh: Notify::new(),
            service_action_busy: AtomicBool::new(false),
            onboarding_opened: AtomicBool::new(false),
        })
        .invoke_handler(tauri::generate_handler![
            control::get_snapshot,
            control::get_peers,
            control::get_transports,
            control::connect_peer,
            control::disconnect_peer,
            control::get_config,
            control::validate_config,
            control::apply_config,
            control::get_apply_status,
            control::reset_config,
            control::set_socket_path,
            control::refresh_now,
            service::set_fips_service_running,
            service::restart_fips_service,
            service::get_node_installation,
            service::use_existing_node,
            service::register_node_service,
            service::repair_node_service,
            service::remove_node_service,
            service::open_background_settings,
            preferences::get_app_preferences,
            preferences::set_app_preferences,
            copy_node_npub,
        ])
        .setup(|app| {
            let preferences = preferences::load(app.handle());
            preferences::apply_dock_preference(app.handle(), preferences.show_dock_icon)
                .map_err(std::io::Error::other)?;
            setup_tray(app)?;
            if let Some(window) = app.get_webview_window("main") {
                configure_window(window);
            }
            if preferences.open_dashboard_at_launch {
                show_window(app.handle(), "overview");
            }
            tauri::async_runtime::spawn(monitor(app.handle().clone()));
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building FIPS")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen {
                has_visible_windows: false,
                ..
            } = event
            {
                show_window(app, "overview");
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classifies_running_and_degraded_nodes() {
        assert_eq!(classify_status(&json!({"state": "Running"})).0, "healthy");
        assert_eq!(
            classify_status(&json!({"state": "Running", "tun_state": "Failed"})).0,
            "degraded"
        );
        assert_eq!(classify_status(&json!({"state": "Starting"})).0, "degraded");
        assert_eq!(
            classify_status(&json!({
                "state": "Running",
                "lan_discovery": {
                    "enabled": true,
                    "state": "running",
                    "warnings": ["LAN discovery is loopback-only"]
                }
            })),
            ("degraded", "LAN discovery is loopback-only".to_string())
        );
        assert_eq!(
            classify_status(&json!({
                "state": "Running",
                "lan_discovery": {
                    "enabled": true,
                    "state": "running",
                    "warnings": []
                }
            }))
            .0,
            "healthy"
        );
        assert_eq!(classify_status(&Value::Null).0, "incompatible");
    }

    #[test]
    fn uses_visible_and_hidden_cadences() {
        assert_eq!(polling_interval(true), Duration::from_secs(2));
        assert_eq!(polling_interval(false), Duration::from_secs(10));
    }

    #[test]
    fn opens_onboarding_for_installation_and_permission_problems() {
        let path = std::path::Path::new("/var/run/fips/control.sock");
        let stopped = snapshot_for_error(
            path,
            ClientError {
                kind: "not_running".into(),
                message: "not running".into(),
            },
            ServiceStatus::checking(),
        );
        assert!(should_open_onboarding(&stopped));

        let mut intentionally_stopped = stopped.clone();
        intentionally_stopped.service.available = true;
        intentionally_stopped.service.state = "stopped".into();
        assert!(!should_open_onboarding(&intentionally_stopped));

        let mut failed_service = intentionally_stopped.clone();
        failed_service.service.enabled = true;
        assert!(should_open_onboarding(&failed_service));

        let mut permission_denied = stopped.clone();
        permission_denied.health = "permission_denied".into();
        assert!(should_open_onboarding(&permission_denied));

        let mut healthy = stopped;
        healthy.health = "healthy".into();
        assert!(!should_open_onboarding(&healthy));
    }
}
