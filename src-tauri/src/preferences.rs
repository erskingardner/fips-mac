use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
use tauri::{AppHandle, Manager};

const PREFERENCES_FILE: &str = "preferences.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct AppPreferences {
    pub show_dock_icon: bool,
    pub open_dashboard_at_launch: bool,
}

fn preferences_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|directory| directory.join(PREFERENCES_FILE))
        .map_err(|error| format!("Could not locate FIPS settings: {error}"))
}

pub fn load(app: &AppHandle) -> AppPreferences {
    let Ok(path) = preferences_path(app) else {
        return AppPreferences::default();
    };
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save(app: &AppHandle, preferences: &AppPreferences) -> Result<(), String> {
    let path = preferences_path(app)?;
    let parent = path
        .parent()
        .ok_or_else(|| "Preferences path has no parent directory.".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create the preferences directory: {error}"))?;
    let bytes = serde_json::to_vec_pretty(preferences)
        .map_err(|error| format!("Could not encode preferences: {error}"))?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, bytes)
        .map_err(|error| format!("Could not write preferences: {error}"))?;
    fs::rename(&temporary, &path).map_err(|error| format!("Could not save preferences: {error}"))
}

pub fn apply_dock_preference(app: &AppHandle, visible: bool) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        app.set_activation_policy(if visible {
            tauri::ActivationPolicy::Regular
        } else {
            tauri::ActivationPolicy::Accessory
        })
        .map_err(|error| format!("Could not change the macOS app mode: {error}"))?;
        app.set_dock_visibility(visible)
            .map_err(|error| format!("Could not change Dock visibility: {error}"))?;
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, visible);
    Ok(())
}

#[tauri::command]
pub fn get_app_preferences(app: AppHandle) -> AppPreferences {
    load(&app)
}

#[tauri::command]
pub fn set_app_preferences(
    app: AppHandle,
    show_dock_icon: bool,
    open_dashboard_at_launch: bool,
) -> Result<AppPreferences, String> {
    let preferences = AppPreferences {
        show_dock_icon,
        open_dashboard_at_launch,
    };
    apply_dock_preference(&app, show_dock_icon)?;
    save(&app, &preferences)?;
    if show_dock_icon {
        crate::show_window(&app, "settings");
    }
    Ok(preferences)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_are_menu_bar_first_by_default() {
        assert_eq!(
            AppPreferences::default(),
            AppPreferences {
                show_dock_icon: false,
                open_dashboard_at_launch: false,
            }
        );
    }

    #[test]
    fn preferences_require_known_fields() {
        assert!(
            serde_json::from_str::<AppPreferences>(
                r#"{"show_dock_icon":true,"open_dashboard_at_launch":false}"#
            )
            .is_ok()
        );
        assert!(
            serde_json::from_str::<AppPreferences>(
                r#"{"show_dock_icon":true,"open_dashboard_at_launch":false,"shell":"/bin/sh"}"#
            )
            .is_err()
        );
    }
}
