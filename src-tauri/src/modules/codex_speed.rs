use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::models::codex::{CodexAppSpeed, CodexAppSpeedConfig};

const APP_SPEED_PREFERENCE_FILE: &str = "codex_api_service_speed.json";
const GLOBAL_STATE_FILE: &str = ".codex-global-state.json";
const PERSISTED_ATOM_STATE_KEY: &str = "electron-persisted-atom-state";
const DEFAULT_SERVICE_TIER_KEY: &str = "default-service-tier";
const HAS_USER_CHANGED_SERVICE_TIER_KEY: &str = "has-user-changed-service-tier";
const FAST_SERVICE_TIER: &str = "fast";
const FLEX_SERVICE_TIER: &str = "flex";

#[derive(serde::Deserialize, serde::Serialize)]
struct AppSpeedPreference {
    speed: CodexAppSpeed,
}

fn get_preference_path() -> Result<PathBuf, String> {
    Ok(crate::modules::config::get_data_dir()?.join(APP_SPEED_PREFERENCE_FILE))
}

fn get_global_state_path() -> PathBuf {
    crate::modules::codex_account::get_codex_home().join(GLOBAL_STATE_FILE)
}

fn get_global_state_path_for_dir(base_dir: &Path) -> PathBuf {
    base_dir.join(GLOBAL_STATE_FILE)
}

fn read_global_state(path: &Path) -> Result<Map<String, Value>, String> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(err) => return Err(format!("读取 Codex 全局状态失败: {}", err)),
    };

    if content.trim().is_empty() {
        return Ok(Map::new());
    }

    let value = serde_json::from_str::<Value>(&content)
        .map_err(|err| format!("解析 Codex 全局状态失败: {}", err))?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| "Codex 全局状态不是合法 JSON 对象".to_string())
}

fn normalize_speed(value: Option<&Value>) -> CodexAppSpeed {
    match value.and_then(Value::as_str) {
        Some(FAST_SERVICE_TIER) | Some(FLEX_SERVICE_TIER) => CodexAppSpeed::Fast,
        _ => CodexAppSpeed::Standard,
    }
}

fn get_persisted_atom_state(state: &Map<String, Value>) -> Option<&Map<String, Value>> {
    state
        .get(PERSISTED_ATOM_STATE_KEY)
        .and_then(Value::as_object)
}

fn get_persisted_atom_state_mut(state: &mut Map<String, Value>) -> &mut Map<String, Value> {
    let value = state
        .entry(PERSISTED_ATOM_STATE_KEY.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    value.as_object_mut().expect("persisted atom state object")
}

fn get_service_tier_value(state: &Map<String, Value>) -> Option<&Value> {
    get_persisted_atom_state(state)
        .and_then(|atoms| atoms.get(DEFAULT_SERVICE_TIER_KEY))
        .or_else(|| state.get(DEFAULT_SERVICE_TIER_KEY))
}

fn build_config(path: &Path, state: &Map<String, Value>) -> CodexAppSpeedConfig {
    CodexAppSpeedConfig {
        speed: normalize_speed(get_service_tier_value(state)),
        global_state_path: path.to_string_lossy().to_string(),
    }
}

fn read_official_app_speed_config() -> Result<CodexAppSpeedConfig, String> {
    let path = get_global_state_path();
    let state = read_global_state(&path)?;
    Ok(build_config(&path, &state))
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
    let preference = serde_json::from_str::<AppSpeedPreference>(&content)
        .map_err(|err| format!("解析 Codex 速度启动配置失败: {}", err))?;
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

fn write_app_speed_for_global_state_path(
    path: PathBuf,
    speed: CodexAppSpeed,
) -> Result<CodexAppSpeedConfig, String> {
    let mut state = read_global_state(&path)?;

    let service_tier_value = match &speed {
        CodexAppSpeed::Standard => Value::Null,
        CodexAppSpeed::Fast => Value::String(FAST_SERVICE_TIER.to_string()),
    };

    match speed {
        CodexAppSpeed::Standard => {
            state.insert(DEFAULT_SERVICE_TIER_KEY.to_string(), Value::Null);
        }
        CodexAppSpeed::Fast => {
            state.insert(
                DEFAULT_SERVICE_TIER_KEY.to_string(),
                Value::String(FAST_SERVICE_TIER.to_string()),
            );
        }
    }
    state.insert(
        HAS_USER_CHANGED_SERVICE_TIER_KEY.to_string(),
        Value::Bool(true),
    );
    let atoms = get_persisted_atom_state_mut(&mut state);
    atoms.insert(DEFAULT_SERVICE_TIER_KEY.to_string(), service_tier_value);
    atoms.insert(
        HAS_USER_CHANGED_SERVICE_TIER_KEY.to_string(),
        Value::Bool(true),
    );

    let content = serde_json::to_string_pretty(&Value::Object(state.clone()))
        .map_err(|err| format!("序列化 Codex 全局状态失败: {}", err))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("创建 Codex 配置目录失败: {}", err))?;
    }
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|err| format!("写入 Codex 全局状态失败: {}", err))?;

    Ok(build_config(&path, &state))
}

pub fn write_official_app_speed(speed: CodexAppSpeed) -> Result<CodexAppSpeedConfig, String> {
    write_app_speed_for_global_state_path(get_global_state_path(), speed)
}

pub fn write_app_speed_for_dir(
    base_dir: &Path,
    speed: CodexAppSpeed,
) -> Result<CodexAppSpeedConfig, String> {
    write_app_speed_for_global_state_path(get_global_state_path_for_dir(base_dir), speed)
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
        build_config, normalize_speed, DEFAULT_SERVICE_TIER_KEY, PERSISTED_ATOM_STATE_KEY,
    };
    use crate::models::codex::CodexAppSpeed;
    use serde_json::{Map, Value};
    use std::path::Path;

    #[test]
    fn reads_fast_and_flex_as_fast_speed() {
        assert_eq!(
            normalize_speed(Some(&Value::String("fast".to_string()))),
            CodexAppSpeed::Fast
        );
        assert_eq!(
            normalize_speed(Some(&Value::String("flex".to_string()))),
            CodexAppSpeed::Fast
        );
    }

    #[test]
    fn reads_missing_or_unknown_speed_as_standard() {
        assert_eq!(normalize_speed(None), CodexAppSpeed::Standard);
        assert_eq!(
            normalize_speed(Some(&Value::String("default".to_string()))),
            CodexAppSpeed::Standard
        );
    }

    #[test]
    fn builds_config_with_global_state_path() {
        let mut state = Map::new();
        state.insert(
            DEFAULT_SERVICE_TIER_KEY.to_string(),
            Value::String("fast".to_string()),
        );

        let config = build_config(Path::new("/tmp/.codex-global-state.json"), &state);
        assert_eq!(config.speed, CodexAppSpeed::Fast);
        assert_eq!(config.global_state_path, "/tmp/.codex-global-state.json");
    }

    #[test]
    fn persisted_atom_state_takes_precedence_over_top_level_value() {
        let mut state = Map::new();
        state.insert(
            DEFAULT_SERVICE_TIER_KEY.to_string(),
            Value::String("fast".to_string()),
        );
        let mut atoms = Map::new();
        atoms.insert(DEFAULT_SERVICE_TIER_KEY.to_string(), Value::Null);
        state.insert(PERSISTED_ATOM_STATE_KEY.to_string(), Value::Object(atoms));

        let config = build_config(Path::new("/tmp/.codex-global-state.json"), &state);
        assert_eq!(config.speed, CodexAppSpeed::Standard);
    }
}
