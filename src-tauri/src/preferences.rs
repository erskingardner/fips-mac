use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
use tauri::{AppHandle, Manager};

const PREFERENCES_FILE: &str = "preferences.json";
const LEGACY_IDENTIFIER: &str = "com.paper-robin.fips-monitor";
const CURRENT_PREFERENCES_VERSION: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppPreferences {
    #[serde(default = "visible_by_default")]
    pub show_dock_icon: bool,
    #[serde(default = "visible_by_default")]
    pub open_dashboard_at_launch: bool,
    #[serde(default)]
    preferences_version: u8,
}

const fn visible_by_default() -> bool {
    true
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            show_dock_icon: true,
            open_dashboard_at_launch: true,
            preferences_version: CURRENT_PREFERENCES_VERSION,
        }
    }
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

    let stored = fs::read(&path).ok().or_else(|| {
        path.parent()
            .and_then(std::path::Path::parent)
            .map(|application_support| {
                application_support
                    .join(LEGACY_IDENTIFIER)
                    .join(PREFERENCES_FILE)
            })
            .and_then(|legacy_path| fs::read(legacy_path).ok())
    });
    let preferences = stored
        .and_then(|bytes| serde_json::from_slice::<AppPreferences>(&bytes).ok())
        .unwrap_or_default();
    let (preferences, migrated) = migrate(preferences);

    // Before FIPS became a regular Mac app, both visibility options defaulted
    // to false. Migrate that prerelease state so an existing installation
    // cannot disappear into an inaccessible background-only process.
    if migrated {
        let _ = save(app, &preferences);
    }

    preferences
}

fn migrate(mut preferences: AppPreferences) -> (AppPreferences, bool) {
    if preferences.preferences_version >= CURRENT_PREFERENCES_VERSION {
        return (preferences, false);
    }
    preferences.show_dock_icon = true;
    preferences.open_dashboard_at_launch = true;
    preferences.preferences_version = CURRENT_PREFERENCES_VERSION;
    (preferences, true)
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
        preferences_version: CURRENT_PREFERENCES_VERSION,
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
    fn preferences_are_visible_by_default() {
        assert_eq!(
            AppPreferences::default(),
            AppPreferences {
                show_dock_icon: true,
                open_dashboard_at_launch: true,
                preferences_version: CURRENT_PREFERENCES_VERSION,
            }
        );
    }

    #[test]
    fn legacy_preferences_migrate_to_recoverable_visibility() {
        let preferences = serde_json::from_str::<AppPreferences>(
            r#"{"show_dock_icon":false,"open_dashboard_at_launch":false}"#,
        )
        .unwrap();
        let (preferences, migrated) = migrate(preferences);

        assert!(migrated);
        assert!(preferences.show_dock_icon);
        assert!(preferences.open_dashboard_at_launch);
        assert_eq!(preferences.preferences_version, CURRENT_PREFERENCES_VERSION);
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
