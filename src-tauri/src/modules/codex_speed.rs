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
/// 官方“标准”档位在 config.toml 顶层 `service_tier` 里显式写 `default`。
const DEFAULT_SERVICE_TIER: &str = "default";
/// 官方桌面端当前写入档位的顶层键（`config/batchWrite` → `service_tier`）。
const TOP_LEVEL_SERVICE_TIER_KEY: &str = "service_tier";

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
    match value.map(str::trim) {
        Some(FAST_SERVICE_TIER) | Some(PRIORITY_SERVICE_TIER) => CodexAppSpeed::Fast,
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

/// Profile overrides the root tier; older desktop clients use the desktop key.
fn read_effective_service_tier_from_doc(doc: &Document) -> Option<&str> {
    doc.get("profile")
        .and_then(|item| item.as_str())
        .and_then(|profile| {
            doc.get("profiles")?
                .get(profile)?
                .get(TOP_LEVEL_SERVICE_TIER_KEY)
        })
        .and_then(|item| item.as_str())
        .or_else(|| {
            doc.get(TOP_LEVEL_SERVICE_TIER_KEY)
                .and_then(|item| item.as_str())
                .or_else(|| read_desktop_service_tier_from_doc(doc))
        })
}

fn read_config_toml(path: &Path) -> Result<Document, String> {
    crate::modules::codex_config_format::load_codex_config_doc(path)
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
    let fast_disabled = doc
        .get("features")
        .and_then(|item| item.get("fast_mode"))
        .and_then(|item| item.as_bool())
        == Some(false);
    Ok(build_config_with_config_path(
        path,
        if fast_disabled {
            CodexAppSpeed::Standard
        } else {
            normalize_service_tier_speed(read_effective_service_tier_from_doc(&doc))
        },
    ))
}

fn read_official_app_speed_config() -> Result<CodexAppSpeedConfig, String> {
    read_official_app_speed_config_from_config_toml(&get_config_toml_path())
}

fn read_preferred_speed() -> Result<Option<CodexAppSpeed>, String> {
    let path = get_preference_path()?;
    read_preferred_speed_from_path(&path)
}

fn read_preferred_speed_from_path(path: &Path) -> Result<Option<CodexAppSpeed>, String> {
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
    write_preferred_speed_to_path(&path, speed)
}

fn write_preferred_speed_to_path(path: &Path, speed: &CodexAppSpeed) -> Result<(), String> {
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
    read_official_app_speed_config()
}

pub fn get_app_speed_config_for_dir(base_dir: &Path) -> Result<CodexAppSpeedConfig, String> {
    read_official_app_speed_config_from_config_toml(&get_config_toml_path_for_dir(base_dir))
}

fn write_app_speed_for_config_toml_path(
    path: PathBuf,
    speed: CodexAppSpeed,
) -> Result<CodexAppSpeedConfig, String> {
    let mut doc = read_config_toml(&path)?;

    match desktop_service_tier_value(&speed) {
        Some(tier) => {
            if doc.get(DESKTOP_SECTION_KEY).is_none() {
                doc[DESKTOP_SECTION_KEY] = toml_edit::table();
            }
            let desktop = doc[DESKTOP_SECTION_KEY]
                .as_table_mut()
                .ok_or("config.toml 中 desktop 不是合法表结构")?;
            desktop[DESKTOP_DEFAULT_SERVICE_TIER_KEY] = value(tier);
        }
        None => {
            if let Some(desktop) = doc
                .get_mut(DESKTOP_SECTION_KEY)
                .and_then(|item| item.as_table_mut())
            {
                let _ = desktop.remove(DESKTOP_DEFAULT_SERVICE_TIER_KEY);
            }
        }
    }
    // 官方桌面端通过 `config/batchWrite` 写顶层 `service_tier`（无 profile 时），
    // 标准档写 `default`；这里与官方落盘规则保持一致。
    doc[TOP_LEVEL_SERVICE_TIER_KEY] = value(top_level_service_tier_value(&speed));
    // Official batchWrite targets the selected profile. A profile tier takes
    // precedence over the root tier when the client/CLI loads configuration.
    if let Some(profile) = doc
        .get("profile")
        .and_then(|item| item.as_str())
        .map(str::to_owned)
    {
        if doc.get("profiles").is_none() {
            doc["profiles"] = toml_edit::table();
        }
        let profiles = doc["profiles"]
            .as_table_like_mut()
            .ok_or("config.toml 中 profiles 不是合法表结构")?;
        if profiles.get(&profile).is_none() {
            profiles.insert(&profile, toml_edit::table());
        }
        let selected = profiles
            .get_mut(&profile)
            .and_then(|item| item.as_table_like_mut())
            .ok_or("config.toml 中当前 profile 不是合法表结构")?;
        selected.insert(
            TOP_LEVEL_SERVICE_TIER_KEY,
            value(top_level_service_tier_value(&speed)),
        );
    }
    if speed == CodexAppSpeed::Fast {
        // An explicit Fast choice must work even after fast_mode was disabled.
        // Standard changes only the tier, leaving the official picker available.
        if doc.get("features").is_none() {
            doc["features"] = toml_edit::table();
        }
        let features = doc["features"]
            .as_table_like_mut()
            .ok_or("config.toml 中 features 不是合法表结构")?;
        features.insert("fast_mode", value(true));
    }

    let content = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("创建 Codex 配置目录失败: {}", err))?;
    }
    crate::modules::codex_config_format::write_codex_config_toml_atomic(&path, &content)
        .map_err(|err| format!("写入 Codex config.toml 失败: {}", err))?;

    Ok(build_config_with_config_path(&path, speed))
}

/// 旧客户端读取的 `[desktop] default-service-tier` 取值；标准档删除该键。
fn desktop_service_tier_value(speed: &CodexAppSpeed) -> Option<&'static str> {
    match speed {
        CodexAppSpeed::Fast => Some(PRIORITY_SERVICE_TIER),
        CodexAppSpeed::Standard => None,
    }
}

/// 官方桌面端顶层 `service_tier` 取值：标准档显式写 `default`。
fn top_level_service_tier_value(speed: &CodexAppSpeed) -> &'static str {
    desktop_service_tier_value(speed).unwrap_or(DEFAULT_SERVICE_TIER)
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
    Ok(build_config_with_speed(
        &get_preference_path()?,
        read_preferred_speed()?.unwrap_or_default(),
    ))
}

pub fn save_api_service_app_speed(speed: CodexAppSpeed) -> Result<CodexAppSpeedConfig, String> {
    write_preferred_speed(&speed)?;
    Ok(build_config_with_speed(&get_preference_path()?, speed))
}

pub fn apply_api_service_speed_to_official_state() -> Result<CodexAppSpeedConfig, String> {
    let speed = read_preferred_speed()?.unwrap_or_default();
    write_official_app_speed(speed)
}

#[cfg(test)]
mod tests {
    use super::{
        get_app_speed_config_for_dir, normalize_service_tier_speed,
        read_desktop_service_tier_from_doc, read_effective_service_tier_from_doc,
        sync_legacy_service_tier_state, write_app_speed_for_config_toml_path,
        DESKTOP_DEFAULT_SERVICE_TIER_KEY, DESKTOP_SECTION_KEY, ELECTRON_PERSISTED_ATOM_STATE_KEY,
        GLOBAL_STATE_FILE, HAS_USER_CHANGED_SERVICE_TIER_KEY, TOP_LEVEL_SERVICE_TIER_KEY,
    };
    use crate::models::codex::CodexAppSpeed;
    use std::fs;
    use std::path::{Path, PathBuf};
    use toml_edit::Document;

    #[test]
    fn fast_choice_survives_profile_readback_and_standard_clears_it() {
        let path = unique_temp_path("codex-speed-active-profile");
        fs::write(
            &path,
            r#"
profile = "work"
service_tier = "default"
[features]
fast_mode = false
js_repl = true
[profiles.work]
service_tier = "default"
model = "custom-model"
[profiles.other]
service_tier = "flex"
"#,
        )
        .unwrap();
        write_app_speed_for_config_toml_path(path.clone(), CodexAppSpeed::Fast).unwrap();
        let saved = fs::read_to_string(&path)
            .unwrap()
            .parse::<Document>()
            .unwrap();
        assert_eq!(
            read_effective_service_tier_from_doc(&saved),
            Some("priority")
        );
        assert_eq!(saved["features"]["fast_mode"].as_bool(), Some(true));
        assert_eq!(saved["features"]["js_repl"].as_bool(), Some(true));
        assert_eq!(
            saved["profiles"]["other"]["service_tier"].as_str(),
            Some("flex")
        );
        assert_eq!(
            saved["profiles"]["work"]["model"].as_str(),
            Some("custom-model")
        );
        write_app_speed_for_config_toml_path(path.clone(), CodexAppSpeed::Standard).unwrap();
        let saved = fs::read_to_string(&path)
            .unwrap()
            .parse::<Document>()
            .unwrap();
        assert_eq!(
            read_effective_service_tier_from_doc(&saved),
            Some("default")
        );
        assert_eq!(saved["service_tier"].as_str(), Some("default"));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn profile_tier_takes_precedence_and_invalid_tables_do_not_destroy_config() {
        let doc = "profile = 'work'\nservice_tier = 'priority'\n[profiles.work]\nservice_tier = 'default'\n".parse::<Document>().unwrap();
        assert_eq!(read_effective_service_tier_from_doc(&doc), Some("default"));
        let path = unique_temp_path("codex-speed-invalid-profile");
        for original in ["profile = 'work'\nprofiles = 42\n", "features = 42\n"] {
            fs::write(&path, original).unwrap();
            assert!(
                write_app_speed_for_config_toml_path(path.clone(), CodexAppSpeed::Fast).is_err()
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), original);
        }
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn api_preference_persists_separately_from_client_account_speed() {
        let dir = unique_temp_dir("codex-api-speed-separation");
        fs::create_dir_all(&dir).unwrap();
        let pref = dir.join("codex_api_service_speed.json");
        let config = dir.join("config.toml");
        assert_eq!(
            super::read_preferred_speed_from_path(&pref)
                .unwrap()
                .unwrap_or_default(),
            CodexAppSpeed::Standard
        );
        fs::write(&config, "service_tier = 'default'\n").unwrap();
        super::write_preferred_speed_to_path(&pref, &CodexAppSpeed::Fast).unwrap();
        assert_eq!(
            super::read_preferred_speed_from_path(&pref).unwrap(),
            Some(CodexAppSpeed::Fast)
        );
        assert_eq!(
            fs::read_to_string(&config).unwrap(),
            "service_tier = 'default'\n"
        );
        write_app_speed_for_config_toml_path(config.clone(), CodexAppSpeed::Standard).unwrap();
        assert_eq!(
            super::read_preferred_speed_from_path(&pref).unwrap(),
            Some(CodexAppSpeed::Fast)
        );
        super::write_preferred_speed_to_path(&pref, &CodexAppSpeed::Standard).unwrap();
        assert_eq!(
            super::read_preferred_speed_from_path(&pref).unwrap(),
            Some(CodexAppSpeed::Standard)
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn disabled_fast_feature_is_not_reported_as_effective_fast() {
        let dir = unique_temp_dir("codex-speed-disabled-feature");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("config.toml"),
            "service_tier = 'priority'\n[features]\nfast_mode = false\n",
        )
        .unwrap();
        assert_eq!(
            get_app_speed_config_for_dir(&dir).unwrap().speed,
            CodexAppSpeed::Standard
        );
        super::write_app_speed_for_dir(&dir, CodexAppSpeed::Fast).unwrap();
        assert_eq!(
            get_app_speed_config_for_dir(&dir).unwrap().speed,
            CodexAppSpeed::Fast
        );
        fs::remove_dir_all(dir).unwrap();
    }

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
            CodexAppSpeed::Standard
        );
        assert_eq!(
            normalize_service_tier_speed(Some("default")),
            CodexAppSpeed::Standard
        );
        assert_eq!(
            normalize_service_tier_speed(Some("ultrafast")),
            CodexAppSpeed::Standard
        );
        assert_eq!(
            normalize_service_tier_speed(Some(" ultrafast ")),
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
    fn prefers_top_level_service_tier_over_desktop_key() {
        let doc = r#"
service_tier = "ultrafast"

[desktop]
default-service-tier = "priority"
"#
        .parse::<Document>()
        .expect("parse config");

        assert_eq!(
            read_effective_service_tier_from_doc(&doc),
            Some("ultrafast")
        );
    }

    #[test]
    fn falls_back_to_desktop_service_tier_without_top_level_key() {
        let doc = r#"
[desktop]
default-service-tier = "priority"
"#
        .parse::<Document>()
        .expect("parse config");

        assert_eq!(read_effective_service_tier_from_doc(&doc), Some("priority"));
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
        assert_eq!(doc[TOP_LEVEL_SERVICE_TIER_KEY].as_str(), Some("priority"));

        let _ = fs::remove_file(config_path);
    }

    #[test]
    fn rewrites_legacy_ultrafast_to_standard_config_keys() {
        let config_path = unique_temp_path("codex-speed-ultrafast");
        fs::write(
            &config_path,
            r#"
service_tier = "ultrafast"

[desktop]
default-service-tier = "ultrafast"
appearanceTheme = "system"
"#,
        )
        .expect("write config");

        let preference: super::AppSpeedPreference =
            serde_json::from_str(r#"{"speed":"ultrafast"}"#).expect("read legacy preference");
        assert_eq!(preference.speed, CodexAppSpeed::Standard);
        assert_eq!(
            serde_json::to_value(&preference).expect("serialize preference"),
            serde_json::json!({"speed": "standard"})
        );
        let config = write_app_speed_for_config_toml_path(config_path.clone(), preference.speed)
            .expect("write speed");
        let content = fs::read_to_string(&config_path).expect("read config");
        let doc = content.parse::<Document>().expect("parse config");

        assert_eq!(config.speed, CodexAppSpeed::Standard);
        assert_eq!(doc[TOP_LEVEL_SERVICE_TIER_KEY].as_str(), Some("default"));
        assert!(doc[DESKTOP_SECTION_KEY]
            .as_table()
            .expect("desktop table")
            .get(DESKTOP_DEFAULT_SERVICE_TIER_KEY)
            .is_none());
        assert_eq!(
            doc[DESKTOP_SECTION_KEY]["appearanceTheme"].as_str(),
            Some("system")
        );
        assert_eq!(read_effective_service_tier_from_doc(&doc), Some("default"));

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
        assert_eq!(doc[TOP_LEVEL_SERVICE_TIER_KEY].as_str(), Some("default"));

        let _ = fs::remove_file(config_path);
    }

    #[test]
    fn reads_profile_app_speed_from_config_toml() {
        let base_dir = unique_temp_dir("codex-speed-profile-read");
        fs::create_dir_all(&base_dir).expect("create base dir");
        fs::write(
            base_dir.join("config.toml"),
            r#"
[desktop]
default-service-tier = "priority"
"#,
        )
        .expect("write config");

        let config = get_app_speed_config_for_dir(&base_dir).expect("read speed");

        assert_eq!(config.speed, CodexAppSpeed::Fast);
        assert_eq!(
            config.global_state_path,
            base_dir.join("config.toml").to_string_lossy()
        );

        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn clears_legacy_ultrafast_from_service_tier_global_state() {
        let base_dir = unique_temp_dir("codex-speed-ultrafast-global-state");
        fs::create_dir_all(&base_dir).expect("create base dir");
        let global_state_path = base_dir.join(GLOBAL_STATE_FILE);
        fs::write(
            &global_state_path,
            r#"{"electron-persisted-atom-state":{"default-service-tier":"ultrafast"},"other":1}"#,
        )
        .expect("write global state");

        let speed: CodexAppSpeed = serde_json::from_str(r#""ultrafast""#).expect("legacy speed");
        sync_legacy_service_tier_state(&base_dir, &speed).expect("sync service tier");

        let content = fs::read_to_string(&global_state_path).expect("read global state");
        let state: serde_json::Value = serde_json::from_str(&content).expect("parse state");
        assert_eq!(state["other"], 1);
        assert!(
            state[ELECTRON_PERSISTED_ATOM_STATE_KEY][DESKTOP_DEFAULT_SERVICE_TIER_KEY].is_null()
        );
        assert_eq!(
            state[ELECTRON_PERSISTED_ATOM_STATE_KEY][HAS_USER_CHANGED_SERVICE_TIER_KEY],
            true
        );

        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn reads_legacy_ultrafast_profile_app_speed_as_standard() {
        let base_dir = unique_temp_dir("codex-speed-profile-ultrafast");
        fs::create_dir_all(&base_dir).expect("create base dir");
        fs::write(
            base_dir.join("config.toml"),
            r#"
service_tier = "ultrafast"
"#,
        )
        .expect("write config");

        let config = get_app_speed_config_for_dir(&base_dir).expect("read speed");

        assert_eq!(config.speed, CodexAppSpeed::Standard);

        let _ = fs::remove_dir_all(base_dir);
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
