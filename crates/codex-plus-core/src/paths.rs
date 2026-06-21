use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub const FREECODEX_HOME_DIR: &str = ".freecodex";
pub const FREECODEX_APP_STATE_DIR: &str = ".freecodex-plus";
const LEGACY_APP_STATE_DIR: &str = ".codex-session-delete";

const SETTINGS_FILE: &str = "settings.json";
const LATEST_STATUS_FILE: &str = "latest-status.json";
const DIAGNOSTIC_LOG_FILE: &str = "codex-plus.log";
const MODELS_CACHE_FILE: &str = "models_cache.json";
const VERSION_FILE: &str = "version.json";
const GLOBAL_STATE_FILE: &str = ".codex-global-state.json";
const GLOBAL_STATE_BACKUP_FILE: &str = ".codex-global-state.json.bak";
const ATOM_STATE_KEY: &str = "electron-persisted-atom-state";
const HIDE_FIRST_THREAD_PROMOS_KEY: &str = "electron:onboarding-hide-first-new-thread-promos";

pub fn user_home_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf())
}

/// Canonical Codex data directory for FreeCodex (`CODEX_HOME` → `~/.freecodex`).
pub fn resolve_codex_home_dir() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| user_home_dir().map(|home| home.join(FREECODEX_HOME_DIR)))
        .unwrap_or_else(|| PathBuf::from(FREECODEX_HOME_DIR))
}

pub fn default_app_state_dir() -> PathBuf {
    std::env::var_os("FREECODEX_APP_STATE")
        .map(PathBuf::from)
        .or_else(|| user_home_dir().map(|home| home.join(FREECODEX_APP_STATE_DIR)))
        .unwrap_or_else(|| PathBuf::from(FREECODEX_APP_STATE_DIR))
}

pub fn legacy_app_state_dir() -> PathBuf {
    user_home_dir()
        .map(|home| home.join(LEGACY_APP_STATE_DIR))
        .unwrap_or_else(|| PathBuf::from(LEGACY_APP_STATE_DIR))
}

pub fn default_settings_path() -> PathBuf {
    if let Some(path) = settings_path_for_tests() {
        return path;
    }
    default_app_state_dir().join(SETTINGS_FILE)
}

pub fn default_latest_status_path() -> PathBuf {
    default_app_state_dir().join(LATEST_STATUS_FILE)
}

pub fn default_diagnostic_log_path() -> PathBuf {
    default_app_state_dir().join(DIAGNOSTIC_LOG_FILE)
}

/// Initialize FreeCodex-owned directories. Does not import official `~/.codex` data.
pub fn ensure_freecodex_layout_initialized() -> std::io::Result<()> {
    let app_state = default_app_state_dir();
    fs::create_dir_all(&app_state)?;

    let legacy_settings = legacy_app_state_dir().join(SETTINGS_FILE);
    let settings = app_state.join(SETTINGS_FILE);
    if !settings.exists() && legacy_settings.exists() {
        let _ = fs::copy(&legacy_settings, &settings);
    }

    fs::create_dir_all(resolve_codex_home_dir())?;
    strip_model_promotion_fields_in_models_cache();
    suppress_codex_app_update_prompt();
    Ok(())
}

/// Prevent Codex Desktop from showing built-in app update prompts.
pub fn suppress_codex_app_update_prompt() -> serde_json::Value {
    let version_changed = suppress_codex_app_update_version_file();
    let promos_hidden = suppress_codex_app_update_global_state();
    serde_json::json!({
        "status": "ok",
        "version_dismissed": version_changed,
        "promos_hidden": promos_hidden,
    })
}

fn suppress_codex_app_update_version_file() -> bool {
    let path = resolve_codex_home_dir().join(VERSION_FILE);
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(_) => return false,
    };
    let mut doc = match serde_json::from_str::<serde_json::Value>(&contents) {
        Ok(doc) => doc,
        Err(_) => return false,
    };
    let Some(obj) = doc.as_object_mut() else {
        return false;
    };
    let Some(latest) = obj
        .get("latest_version")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
    else {
        return false;
    };
    if obj.get("dismissed_version") == Some(&serde_json::Value::String(latest.clone())) {
        return false;
    }
    obj.insert(
        "dismissed_version".to_string(),
        serde_json::Value::String(latest),
    );
    if let Ok(updated) = serde_json::to_string_pretty(&doc) {
        let _ = fs::write(path, updated);
        return true;
    }
    false
}

fn suppress_codex_app_update_global_state() -> bool {
    let path = resolve_codex_home_dir().join(GLOBAL_STATE_FILE);
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(_) => return false,
    };
    let mut doc = match serde_json::from_str::<serde_json::Value>(&contents) {
        Ok(doc) => doc,
        Err(_) => return false,
    };
    let Some(root) = doc.as_object_mut() else {
        return false;
    };
    let atom = root
        .entry(ATOM_STATE_KEY.to_string())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    let Some(atom_obj) = atom.as_object_mut() else {
        return false;
    };
    if atom_obj.get(HIDE_FIRST_THREAD_PROMOS_KEY) == Some(&serde_json::Value::Bool(true)) {
        return false;
    }
    atom_obj.insert(
        HIDE_FIRST_THREAD_PROMOS_KEY.to_string(),
        serde_json::Value::Bool(true),
    );
    if let Ok(updated) = serde_json::to_string_pretty(&doc) {
        let _ = fs::write(&path, &updated);
        let _ = fs::write(
            resolve_codex_home_dir().join(GLOBAL_STATE_BACKUP_FILE),
            updated,
        );
        return true;
    }
    false
}

/// Remove Codex model upgrade / availability announcement metadata.
pub fn strip_model_promotion_fields_in_models_cache() {
    let path = resolve_codex_home_dir().join(MODELS_CACHE_FILE);
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(_) => return,
    };
    let mut doc = match serde_json::from_str::<serde_json::Value>(&contents) {
        Ok(doc) => doc,
        Err(_) => return,
    };
    let Some(models) = doc.get_mut("models").and_then(|models| models.as_array_mut()) else {
        return;
    };

    let mut changed = false;
    for model in models {
        let Some(obj) = model.as_object_mut() else {
            continue;
        };
        if null_model_promotion_field(obj, "availability_nux") {
            changed = true;
        }
        if null_model_promotion_field(obj, "upgrade") {
            changed = true;
        }
    }

    if !changed {
        return;
    }

    if let Ok(updated) = serde_json::to_string_pretty(&doc) {
        let _ = fs::write(path, updated);
    }
}

fn null_model_promotion_field(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> bool {
    match obj.get(key) {
        None | Some(serde_json::Value::Null) => false,
        Some(_) => {
            obj.insert(key.to_string(), serde_json::Value::Null);
            true
        }
    }
}

fn settings_path_for_tests() -> Option<PathBuf> {
    SETTINGS_PATH_FOR_TESTS
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|path| path.clone())
}

static SETTINGS_PATH_FOR_TESTS: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

pub fn set_settings_path_for_tests(path: Option<PathBuf>) -> Option<PathBuf> {
    SETTINGS_PATH_FOR_TESTS
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|mut current| std::mem::replace(&mut *current, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_path_uses_freecodex_app_state_directory() {
        let path = default_settings_path();

        assert!(path.ends_with(".freecodex-plus/settings.json"));
    }

    #[test]
    fn default_latest_status_path_uses_freecodex_app_state_directory() {
        let path = default_latest_status_path();

        assert!(path.ends_with(".freecodex-plus/latest-status.json"));
    }

    #[test]
    fn default_diagnostic_log_path_uses_freecodex_app_state_directory() {
        let path = default_diagnostic_log_path();

        assert!(path.ends_with(".freecodex-plus/codex-plus.log"));
    }

    #[test]
    fn ensure_freecodex_layout_does_not_import_official_codex_home() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let official_codex = root.join(".codex");
        let freecodex = root.join(".freecodex");
        fs::create_dir_all(&official_codex).expect("official codex dir");
        fs::write(official_codex.join("marker.txt"), "official").expect("marker");

        let previous_home = std::env::var_os("CODEX_HOME");
        unsafe {
            std::env::set_var("CODEX_HOME", &freecodex);
        }
        ensure_freecodex_layout_initialized().expect("layout init");
        if let Some(value) = previous_home {
            unsafe {
                std::env::set_var("CODEX_HOME", value);
            }
        } else {
            unsafe {
                std::env::remove_var("CODEX_HOME");
            }
        }

        assert!(freecodex.exists());
        assert!(
            !freecodex.join("marker.txt").exists(),
            "official ~/.codex data must not be copied into FreeCodex home"
        );
    }

    #[test]
    fn strip_model_promotion_fields_in_models_cache_clears_upgrade_metadata() {
        let root = std::env::temp_dir().join(format!(
            "freecodex-paths-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        ));
        let home = root.join(".freecodex");
        std::fs::create_dir_all(&home).expect("create home");
        std::fs::write(
            home.join(MODELS_CACHE_FILE),
            r#"{"models":[{"slug":"gpt-5.5","availability_nux":{"message":"hello"},"upgrade":{"target":"gpt-5.4"}}]}"#,
        )
        .expect("write cache");

        let previous = std::env::var_os("CODEX_HOME");
        unsafe {
            std::env::set_var("CODEX_HOME", &home);
        }
        strip_model_promotion_fields_in_models_cache();
        let updated =
            std::fs::read_to_string(home.join(MODELS_CACHE_FILE)).expect("read updated cache");
        unsafe {
            match previous {
                Some(value) => std::env::set_var("CODEX_HOME", value),
                None => std::env::remove_var("CODEX_HOME"),
            }
        }
        let _ = std::fs::remove_dir_all(root);

        assert!(updated.contains("\"availability_nux\": null"));
        assert!(updated.contains("\"upgrade\": null"));
    }

    #[test]
    fn suppress_codex_app_update_prompt_dismisses_latest_version_and_hides_promos() {
        let root = std::env::temp_dir().join(format!(
            "freecodex-paths-update-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        ));
        let home = root.join(".freecodex");
        std::fs::create_dir_all(&home).expect("create home");
        std::fs::write(
            home.join(VERSION_FILE),
            r#"{"latest_version":"0.139.0","dismissed_version":"0.137.0"}"#,
        )
        .expect("write version");
        std::fs::write(
            home.join(GLOBAL_STATE_FILE),
            r#"{"electron-persisted-atom-state":{"electron:onboarding-hide-first-new-thread-promos":false}}"#,
        )
        .expect("write global state");

        let previous = std::env::var_os("CODEX_HOME");
        unsafe {
            std::env::set_var("CODEX_HOME", &home);
        }
        let result = suppress_codex_app_update_prompt();
        let version =
            std::fs::read_to_string(home.join(VERSION_FILE)).expect("read updated version");
        let state =
            std::fs::read_to_string(home.join(GLOBAL_STATE_FILE)).expect("read updated state");
        unsafe {
            match previous {
                Some(value) => std::env::set_var("CODEX_HOME", value),
                None => std::env::remove_var("CODEX_HOME"),
            }
        }
        let _ = std::fs::remove_dir_all(root);

        assert_eq!(result["status"], "ok");
        assert_eq!(result["version_dismissed"], true);
        assert_eq!(result["promos_hidden"], true);
        assert!(version.contains(r#""dismissed_version": "0.139.0""#));
        assert!(state.contains(r#""electron:onboarding-hide-first-new-thread-promos": true"#));
    }

    fn resolve_codex_home_dir_defaults_to_freecodex_directory() {
        let previous = std::env::var_os("CODEX_HOME");
        unsafe {
            std::env::remove_var("CODEX_HOME");
        }
        let path = resolve_codex_home_dir();
        if let Some(value) = previous {
            unsafe {
                std::env::set_var("CODEX_HOME", value);
            }
        }
        assert!(path.ends_with(".freecodex"));
    }
}