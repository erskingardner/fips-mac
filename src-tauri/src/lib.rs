mod control;

use control::{ClientError, ControlClient, resolve_socket_path};
use serde::{Deserialize, Serialize};
use serde_json::Value;
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

const TRAY_ID: &str = "fips-monitor";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorSnapshot {
    pub health: String,
    pub detail: String,
    pub socket_path: String,
    pub checked_at_ms: u128,
    pub status: Option<Value>,
    pub capabilities: Option<Value>,
    pub configuration_supported: bool,
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
        }
    }
}

pub struct AppState {
    pub socket_path: Mutex<PathBuf>,
    pub last_snapshot: Mutex<MonitorSnapshot>,
    pub refresh: Notify,
    onboarding_opened: AtomicBool,
}

struct TrayMenuState {
    status_item: MenuItem<tauri::Wry>,
    peer_item: MenuItem<tauri::Wry>,
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
    ("healthy", "The FIPS node is running normally.".into())
}

fn snapshot_for_error(path: &std::path::Path, error: ClientError) -> MonitorSnapshot {
    let health = match error.kind.as_str() {
        "not_running" => "stopped",
        "permission_denied" => "permission_denied",
        "protocol" => "incompatible",
        _ => "degraded",
    };
    let detail = match health {
        "stopped" => "FIPS is not running or its control socket is not installed.".into(),
        "permission_denied" => {
            "Access was denied. Confirm this account belongs to the fips group, then log out and back in."
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
    }
}

async fn collect_snapshot(path: PathBuf) -> MonitorSnapshot {
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
            }
        }
        Err(error) => snapshot_for_error(&path, error),
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
        let snapshot = collect_snapshot(path).await;
        *app.state::<AppState>().last_snapshot.lock().unwrap() = snapshot.clone();
        update_tray(&app, &snapshot);
        let _ = app.emit("monitor://snapshot", &snapshot);

        if should_open_onboarding(&snapshot.health)
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

fn should_open_onboarding(health: &str) -> bool {
    matches!(health, "stopped" | "permission_denied")
}

fn update_tray(app: &AppHandle, snapshot: &MonitorSnapshot) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_icon_with_as_template(Some(icon_for_health(&snapshot.health)), true);
        let tooltip = format!("FIPS Monitor — {}", health_label(&snapshot.health));
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

fn show_window(app: &AppHandle, section: &str) {
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
    let quit_item = MenuItem::with_id(app, "quit", "Quit FIPS Monitor", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &status_item,
            &peer_item,
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
        launch_item: launch_item.clone(),
    });

    TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .icon(icon_for_health("stopped"))
        .icon_as_template(true)
        .tooltip("FIPS Monitor — Checking")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_window(app, "overview"),
            "settings" => show_window(app, "settings"),
            "refresh" => app.state::<AppState>().refresh.notify_one(),
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
    let initial_snapshot = MonitorSnapshot::starting(&socket_path);
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(AppState {
            socket_path: Mutex::new(socket_path),
            last_snapshot: Mutex::new(initial_snapshot),
            refresh: Notify::new(),
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
            copy_node_npub,
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.handle()
                .set_activation_policy(tauri::ActivationPolicy::Accessory)?;
            setup_tray(app)?;
            if let Some(window) = app.get_webview_window("main") {
                configure_window(window);
            }
            tauri::async_runtime::spawn(monitor(app.handle().clone()));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running FIPS Monitor");
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
        assert_eq!(classify_status(&Value::Null).0, "incompatible");
    }

    #[test]
    fn uses_visible_and_hidden_cadences() {
        assert_eq!(polling_interval(true), Duration::from_secs(2));
        assert_eq!(polling_interval(false), Duration::from_secs(10));
    }

    #[test]
    fn opens_onboarding_for_installation_and_permission_problems() {
        assert!(should_open_onboarding("stopped"));
        assert!(should_open_onboarding("permission_denied"));
        assert!(!should_open_onboarding("healthy"));
        assert!(!should_open_onboarding("degraded"));
    }
}
