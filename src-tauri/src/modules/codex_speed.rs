use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use toml_edit::{value, Document};

use crate::models::codex::{CodexAppSpeed, CodexAppSpeedConfig};

const APP_SPEED_PREFERENCE_FILE: &str = "codex_api_service_speed.json";
const CONFIG_FILE: &str = "config.toml";
const GLOBAL_STATE_FILE: &str = ".codex-global-state.json";
const DESKTOP_SECTION_KEY: &str = "desktop";
const DESKTOP_DEFAULT_SERVICE_TIER_KEY: &str = "default-service-tier";
const ELECTRON_PERSISTED_ATOM_STATE_KEY: &str = "electron-persisted-atom-state";
const HAS_USER_CHANGED_SERVICE_TIER_KEY: &str = "has-user-changed-service-tier";
const FAST_SERVICE_TIER: &str = "fast";
const PRIORITY_SERVICE_TIER: &str = "priority";
const FLEX_SERVICE_TIER: &str = "flex";

#[derive(serde::Deserialize, serde::Serialize)]
struct AppSpeedPreference {
    speed: CodexAppSpeed,
}

fn get_preference_path() -> Result<PathBuf, String> {
    Ok(crate::modules::config::get_data_dir()?.join(APP_SPEED_PREFERENCE_FILE))
}

fn get_config_toml_path() -> PathBuf {
    crate::modules::codex_account::get_codex_home().join(CONFIG_FILE)
}

fn get_config_toml_path_for_dir(base_dir: &Path) -> PathBuf {
    base_dir.join(CONFIG_FILE)
}

fn get_global_state_path_for_dir(base_dir: &Path) -> PathBuf {
    base_dir.join(GLOBAL_STATE_FILE)
}

fn normalize_service_tier_speed(value: Option<&str>) -> CodexAppSpeed {
    match value {
        Some(FAST_SERVICE_TIER) | Some(PRIORITY_SERVICE_TIER) | Some(FLEX_SERVICE_TIER) => {
            CodexAppSpeed::Fast
        }
        _ => CodexAppSpeed::Standard,
    }
}

fn build_config_with_config_path(path: &Path, speed: CodexAppSpeed) -> CodexAppSpeedConfig {
    CodexAppSpeedConfig {
        speed,
        global_state_path: path.to_string_lossy().to_string(),
    }
}

fn read_desktop_service_tier_from_doc(doc: &Document) -> Option<&str> {
    doc.get(DESKTOP_SECTION_KEY)
        .and_then(|item| item.as_table())
        .and_then(|desktop| desktop.get(DESKTOP_DEFAULT_SERVICE_TIER_KEY))
        .and_then(|item| item.as_str())
}

fn read_config_toml(path: &Path) -> Result<Document, String> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Document::new()),
        Err(err) => return Err(format!("读取 Codex config.toml 失败: {}", err)),
    };
    if content.trim().is_empty() {
        return Ok(Document::new());
    }
    content
        .parse::<Document>()
        .map_err(|err| format!("解析 Codex config.toml 失败: {}", err))
}

fn read_global_state_json(path: &Path) -> Result<Map<String, Value>, String> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(err) => return Err(format!("读取 Codex 全局状态失败: {}", err)),
    };
    if content.trim().is_empty() {
        return Ok(Map::new());
    }
    let value = match serde_json::from_str::<Value>(&content) {
        Ok(value) => value,
        Err(error) => {
            match crate::modules::atomic_write::quarantine_file(path, "invalid-json") {
                Ok(Some(backup_path)) => crate::modules::logger::log_warn(&format!(
                    "Codex 全局状态解析失败，已隔离并使用空状态: path={}, backup={}, error={}",
                    path.display(),
                    backup_path.display(),
                    error
                )),
                Ok(None) => crate::modules::logger::log_warn(&format!(
                    "Codex 全局状态解析失败，文件已不存在，使用空状态: path={}, error={}",
                    path.display(),
                    error
                )),
                Err(backup_error) => crate::modules::logger::log_warn(&format!(
                    "Codex 全局状态解析失败，隔离失败，使用空状态: path={}, parse_error={}, backup_error={}",
                    path.display(),
                    error,
                    backup_error
                )),
            }
            return Ok(Map::new());
        }
    };
    Ok(value.as_object().cloned().unwrap_or_default())
}

fn write_global_state_json(path: &Path, state: &Map<String, Value>) -> Result<(), String> {
    let content = serde_json::to_string(state)
        .map_err(|err| format!("序列化 Codex 全局状态失败: {}", err))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("创建 Codex 配置目录失败: {}", err))?;
    }
    crate::modules::atomic_write::write_string_atomic(path, &content)
        .map_err(|err| format!("写入 Codex 全局状态失败: {}", err))?;
    if let Err(error) = crate::modules::atomic_write::write_string_atomic(
        &PathBuf::from(format!("{}.bak", path.to_string_lossy())),
        &content,
    ) {
        crate::modules::logger::log_warn(&format!(
            "写入 Codex 全局状态备份失败，主状态已保存: path={}, error={}",
            path.display(),
            error
        ));
    }
    Ok(())
}

fn legacy_service_tier_value(speed: &CodexAppSpeed) -> Value {
    match speed {
        CodexAppSpeed::Fast => Value::String(PRIORITY_SERVICE_TIER.to_string()),
        CodexAppSpeed::Standard => Value::Null,
    }
}

fn sync_legacy_service_tier_state(base_dir: &Path, speed: &CodexAppSpeed) -> Result<(), String> {
    let path = get_global_state_path_for_dir(base_dir);
    let mut state = read_global_state_json(&path)?;
    let atoms = state
        .entry(ELECTRON_PERSISTED_ATOM_STATE_KEY.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !atoms.is_object() {
        *atoms = Value::Object(Map::new());
    }
    let atoms = atoms
        .as_object_mut()
        .ok_or("Codex persisted atom 状态不是合法对象")?;
    atoms.insert(
        DESKTOP_DEFAULT_SERVICE_TIER_KEY.to_string(),
        legacy_service_tier_value(speed),
    );
    atoms.insert(
        HAS_USER_CHANGED_SERVICE_TIER_KEY.to_string(),
        Value::Bool(true),
    );
    write_global_state_json(&path, &state)
}

fn read_official_app_speed_config_from_config_toml(
    path: &Path,
) -> Result<CodexAppSpeedConfig, String> {
    let doc = read_config_toml(path)?;
    Ok(build_config_with_config_path(
        path,
        normalize_service_tier_speed(read_desktop_service_tier_from_doc(&doc)),
    ))
}

fn read_official_app_speed_config() -> Result<CodexAppSpeedConfig, String> {
    read_official_app_speed_config_from_config_toml(&get_config_toml_path())
}

fn read_preferred_speed() -> Result<Option<CodexAppSpeed>, String> {
    let path = get_preference_path()?;
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("读取 Codex 速度启动配置失败: {}", err)),
    };
    if content.trim().is_empty() {
        return Ok(None);
    }
    let preference = match serde_json::from_str::<AppSpeedPreference>(&content) {
        Ok(preference) => preference,
        Err(error) => {
            match crate::modules::atomic_write::quarantine_file(&path, "invalid-json") {
                Ok(Some(backup_path)) => crate::modules::logger::log_warn(&format!(
                    "Codex 速度启动配置解析失败，已隔离并回落默认速度: path={}, backup={}, error={}",
                    path.display(),
                    backup_path.display(),
                    error
                )),
                Ok(None) => crate::modules::logger::log_warn(&format!(
                    "Codex 速度启动配置解析失败，文件已不存在，回落默认速度: path={}, error={}",
                    path.display(),
                    error
                )),
                Err(backup_error) => crate::modules::logger::log_warn(&format!(
                    "Codex 速度启动配置解析失败，隔离失败，回落默认速度: path={}, parse_error={}, backup_error={}",
                    path.display(),
                    error,
                    backup_error
                )),
            }
            return Ok(None);
        }
    };
    Ok(Some(preference.speed))
}

fn write_preferred_speed(speed: &CodexAppSpeed) -> Result<(), String> {
    let path = get_preference_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("创建配置目录失败: {}", err))?;
    }
    let content = serde_json::to_string_pretty(&AppSpeedPreference {
        speed: speed.clone(),
    })
    .map_err(|err| format!("序列化 Codex 速度启动配置失败: {}", err))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|err| format!("写入 Codex 速度启动配置失败: {}", err))
}

fn build_config_with_speed(path: &Path, speed: CodexAppSpeed) -> CodexAppSpeedConfig {
    CodexAppSpeedConfig {
        speed,
        global_state_path: path.to_string_lossy().to_string(),
    }
}

pub fn get_app_speed_config() -> Result<CodexAppSpeedConfig, String> {
    let official = read_official_app_speed_config()?;
    if let Some(speed) = read_preferred_speed()? {
        return Ok(build_config_with_speed(
            Path::new(&official.global_state_path),
            speed,
        ));
    }
    Ok(official)
}

fn write_app_speed_for_config_toml_path(
    path: PathBuf,
    speed: CodexAppSpeed,
) -> Result<CodexAppSpeedConfig, String> {
    let mut doc = read_config_toml(&path)?;

    match speed {
        CodexAppSpeed::Standard => {
            if let Some(desktop) = doc
                .get_mut(DESKTOP_SECTION_KEY)
                .and_then(|item| item.as_table_mut())
            {
                let _ = desktop.remove(DESKTOP_DEFAULT_SERVICE_TIER_KEY);
            }
        }
        CodexAppSpeed::Fast => {
            if doc.get(DESKTOP_SECTION_KEY).is_none() {
                doc[DESKTOP_SECTION_KEY] = toml_edit::table();
            }
            let desktop = doc[DESKTOP_SECTION_KEY]
                .as_table_mut()
                .ok_or("config.toml 中 desktop 不是合法表结构")?;
            desktop[DESKTOP_DEFAULT_SERVICE_TIER_KEY] = value(PRIORITY_SERVICE_TIER);
        }
    }

    let content = doc.to_string();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("创建 Codex 配置目录失败: {}", err))?;
    }
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|err| format!("写入 Codex config.toml 失败: {}", err))?;

    Ok(build_config_with_config_path(&path, speed))
}

fn write_app_speed_for_base_dir(
    base_dir: &Path,
    speed: CodexAppSpeed,
) -> Result<CodexAppSpeedConfig, String> {
    let config = write_app_speed_for_config_toml_path(
        get_config_toml_path_for_dir(base_dir),
        speed.clone(),
    )?;
    sync_legacy_service_tier_state(base_dir, &speed)?;
    Ok(config)
}

pub fn write_official_app_speed(speed: CodexAppSpeed) -> Result<CodexAppSpeedConfig, String> {
    write_app_speed_for_base_dir(&crate::modules::codex_account::get_codex_home(), speed)
}

pub fn write_app_speed_for_dir(
    base_dir: &Path,
    speed: CodexAppSpeed,
) -> Result<CodexAppSpeedConfig, String> {
    write_app_speed_for_base_dir(base_dir, speed)
}

pub fn get_api_service_app_speed_config() -> Result<CodexAppSpeedConfig, String> {
    let official = read_official_app_speed_config()?;
    Ok(build_config_with_speed(
        Path::new(&official.global_state_path),
        read_preferred_speed()?.unwrap_or_default(),
    ))
}

pub fn save_api_service_app_speed(speed: CodexAppSpeed) -> Result<CodexAppSpeedConfig, String> {
    write_preferred_speed(&speed)?;
    write_official_app_speed(speed)
}

pub fn apply_api_service_speed_to_official_state() -> Result<CodexAppSpeedConfig, String> {
    let speed = read_preferred_speed()?.unwrap_or_default();
    write_official_app_speed(speed)
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_service_tier_speed, read_desktop_service_tier_from_doc,
        sync_legacy_service_tier_state, write_app_speed_for_config_toml_path,
        DESKTOP_DEFAULT_SERVICE_TIER_KEY, DESKTOP_SECTION_KEY, ELECTRON_PERSISTED_ATOM_STATE_KEY,
        GLOBAL_STATE_FILE, HAS_USER_CHANGED_SERVICE_TIER_KEY,
    };
    use crate::models::codex::CodexAppSpeed;
    use std::fs;
    use std::path::{Path, PathBuf};
    use toml_edit::Document;

    #[test]
    fn normalizes_official_desktop_service_tier_values() {
        assert_eq!(normalize_service_tier_speed(None), CodexAppSpeed::Standard);
        assert_eq!(
            normalize_service_tier_speed(Some("priority")),
            CodexAppSpeed::Fast
        );
        assert_eq!(
            normalize_service_tier_speed(Some("fast")),
            CodexAppSpeed::Fast
        );
        assert_eq!(
            normalize_service_tier_speed(Some("flex")),
            CodexAppSpeed::Fast
        );
        assert_eq!(
            normalize_service_tier_speed(Some("default")),
            CodexAppSpeed::Standard
        );
    }

    #[test]
    fn reads_desktop_default_service_tier_from_config_toml() {
        let doc = r#"
[desktop]
default-service-tier = "priority"
"#
        .parse::<Document>()
        .expect("parse config");

        assert_eq!(read_desktop_service_tier_from_doc(&doc), Some("priority"));
    }

    #[test]
    fn writes_fast_to_official_desktop_config_key() {
        let config_path = unique_temp_path("codex-speed-fast");
        fs::write(
            &config_path,
            r#"
[desktop]
appearanceTheme = "system"
"#,
        )
        .expect("write config");

        let config = write_app_speed_for_config_toml_path(config_path.clone(), CodexAppSpeed::Fast)
            .expect("write speed");
        let content = fs::read_to_string(&config_path).expect("read config");
        let doc = content.parse::<Document>().expect("parse config");

        assert_eq!(config.speed, CodexAppSpeed::Fast);
        assert_eq!(
            doc[DESKTOP_SECTION_KEY][DESKTOP_DEFAULT_SERVICE_TIER_KEY].as_str(),
            Some("priority")
        );

        let _ = fs::remove_file(config_path);
    }

    #[test]
    fn removes_desktop_default_service_tier_for_standard() {
        let config_path = unique_temp_path("codex-speed-standard");
        fs::write(
            &config_path,
            r#"
[desktop]
default-service-tier = "priority"
appearanceTheme = "system"
"#,
        )
        .expect("write config");

        let config =
            write_app_speed_for_config_toml_path(config_path.clone(), CodexAppSpeed::Standard)
                .expect("write speed");
        let content = fs::read_to_string(&config_path).expect("read config");
        let doc = content.parse::<Document>().expect("parse config");

        assert_eq!(config.speed, CodexAppSpeed::Standard);
        assert!(doc[DESKTOP_SECTION_KEY]
            .as_table()
            .expect("desktop table")
            .get(DESKTOP_DEFAULT_SERVICE_TIER_KEY)
            .is_none());

        let _ = fs::remove_file(config_path);
    }

    #[test]
    fn syncs_fast_to_legacy_service_tier_global_state() {
        let base_dir = unique_temp_dir("codex-speed-global-state");
        fs::create_dir_all(&base_dir).expect("create base dir");
        let global_state_path = base_dir.join(GLOBAL_STATE_FILE);
        fs::write(
            &global_state_path,
            r#"{"electron-persisted-atom-state":{"theme":"dark"},"other":1}"#,
        )
        .expect("write global state");

        sync_legacy_service_tier_state(&base_dir, &CodexAppSpeed::Fast).expect("sync service tier");

        let content = fs::read_to_string(&global_state_path).expect("read global state");
        let state: serde_json::Value = serde_json::from_str(&content).expect("parse state");
        assert_eq!(state["other"], 1);
        assert_eq!(state[ELECTRON_PERSISTED_ATOM_STATE_KEY]["theme"], "dark");
        assert_eq!(
            state[ELECTRON_PERSISTED_ATOM_STATE_KEY][DESKTOP_DEFAULT_SERVICE_TIER_KEY],
            "priority"
        );
        assert_eq!(
            state[ELECTRON_PERSISTED_ATOM_STATE_KEY][HAS_USER_CHANGED_SERVICE_TIER_KEY],
            true
        );
        assert!(Path::new(&format!("{}.bak", global_state_path.to_string_lossy())).exists());

        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn syncs_standard_to_legacy_service_tier_global_state() {
        let base_dir = unique_temp_dir("codex-speed-standard-global-state");
        fs::create_dir_all(&base_dir).expect("create base dir");
        let global_state_path = base_dir.join(GLOBAL_STATE_FILE);
        fs::write(
            &global_state_path,
            r#"{"electron-persisted-atom-state":{"default-service-tier":"priority"}}"#,
        )
        .expect("write global state");

        sync_legacy_service_tier_state(&base_dir, &CodexAppSpeed::Standard)
            .expect("sync service tier");

        let content = fs::read_to_string(&global_state_path).expect("read global state");
        let state: serde_json::Value = serde_json::from_str(&content).expect("parse state");
        assert!(
            state[ELECTRON_PERSISTED_ATOM_STATE_KEY][DESKTOP_DEFAULT_SERVICE_TIER_KEY].is_null()
        );
        assert_eq!(
            state[ELECTRON_PERSISTED_ATOM_STATE_KEY][HAS_USER_CHANGED_SERVICE_TIER_KEY],
            true
        );

        let _ = fs::remove_dir_all(base_dir);
    }

    fn unique_temp_path(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "{}-{}-{}.toml",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        path
    }

    fn unique_temp_dir(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "{}-{}-{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        path
    }
}
