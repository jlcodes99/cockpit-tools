//! APIKEY.FUN 一键 Gemini CLI 落盘。
//!
//! 按官方文档 https://apikey.fan/docs/gemini 把三个变量写入 `~/.gemini/.env`：
//!   GOOGLE_GEMINI_BASE_URL / GEMINI_API_KEY / GEMINI_MODEL
//! 并把 `~/.gemini/settings.json` 的认证方式切到 `gemini-api-key`
//! （`security.auth.selectedType`），否则 `GOOGLE_GEMINI_BASE_URL` 只在
//! API Key 认证方式下生效。
//!
//! 两个文件都保留用户已有内容（仅替换受管键 / 合并认证方式），并以 0600
//! 权限原子写入，不生成明文 .bak。

use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
};

use serde_json::{json, Value};

use super::atomic_write::replace_private_string_if_matches;

/// 官方记录的 Gemini CLI 根地址（不带 /v1）。
pub const GOOGLE_GEMINI_BASE_URL: &str = "https://api.apikey.fan";

const GEMINI_API_KEY_AUTH: &str = "gemini-api-key";
const MAX_ENV_BYTES: u64 = 64 * 1024;
const MAX_SETTINGS_BYTES: u64 = 64 * 1024;

static WRITES: Mutex<()> = Mutex::new(());

fn gemini_dir() -> Result<PathBuf, String> {
    dirs::home_dir()
        .ok_or_else(|| "无法定位用户主目录".to_string())
        .map(|home| home.join(".gemini"))
}

fn read_bounded(path: &Path, limit: u64) -> Result<Option<String>, String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() && meta.len() <= limit => (),
        _ => return Err(format!("读取 Gemini CLI 配置失败: {}", path.display())),
    }
    fs::read_to_string(path)
        .map(Some)
        .map_err(|error| format!("读取 Gemini CLI 配置失败: {} ({error})", path.display()))
}

fn escape(value: &str) -> Result<String, String> {
    if value.trim().is_empty() {
        return Err("Gemini CLI 密钥和模型不能为空".into());
    }
    if value.contains(['\n', '\r', '\0']) {
        return Err("Gemini CLI 配置值包含非法换行或空字符".into());
    }
    Ok(value.replace('\\', "\\\\").replace('"', "\\\""))
}

// Match a whole dotenv record, including quoted multiline values. Unsupported
// syntax must not turn a line inside an unrelated value into a managed key.
static ENV_RECORD: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r#"(?x)\A[\t\x20]*(?:
        (?:export[\t\x20]+)?(?P<key>[\w.-]+)[\t\x20]*=[\t\x20]*
        (?:"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|`(?:\\.|[^`\\])*`|[^'"\x60\r\n\t\x20][^\r\n]*|)[\t\x20]*
    )?(?:\#[^\r\n]*)?(?:\r?\n|\z)"#).expect("valid dotenv record pattern")
});

fn build_env(existing: Option<&str>, api_key: &str, model: &str) -> Result<String, String> {
    let mut kept = String::new();
    let mut remaining = existing.unwrap_or("");
    if let Some(rest) = remaining.strip_prefix('\u{feff}') {
        kept.push('\u{feff}');
        remaining = rest;
    }
    while !remaining.is_empty() {
        let record = ENV_RECORD
            .captures(remaining)
            .ok_or("Gemini CLI .env 格式无法安全合并")?;
        let text = record.get(0).unwrap().as_str();
        if !matches!(
            record.name("key").map(|m| m.as_str()),
            Some("GOOGLE_GEMINI_BASE_URL" | "GEMINI_API_KEY" | "GEMINI_MODEL")
        ) {
            kept.push_str(text);
        }
        remaining = &remaining[text.len()..];
    }
    if !kept.is_empty() && kept != "\u{feff}" && !kept.ends_with('\n') {
        kept.push('\n');
    }
    kept.push_str(&format!(
        "GOOGLE_GEMINI_BASE_URL=\"{}\"\n",
        GOOGLE_GEMINI_BASE_URL
    ));
    kept.push_str(&format!("GEMINI_API_KEY=\"{}\"\n", escape(api_key)?));
    kept.push_str(&format!("GEMINI_MODEL=\"{}\"\n", escape(model)?));
    Ok(kept)
}

/// 合并 settings.json：把 `security.auth.selectedType` 设为 `gemini-api-key`。
/// 已经是该值时返回 `None`，避免重排用户文件格式。
fn build_settings(existing: Option<&str>) -> Result<Option<String>, String> {
    let mut settings: Value = match existing {
        Some(text) => serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|error| format!("Gemini CLI settings.json 无法解析: {error}"))?,
        None => Value::Object(serde_json::Map::new()),
    };
    let root = settings
        .as_object_mut()
        .ok_or("Gemini CLI settings.json 不是对象")?;
    let security = root
        .entry("security")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("Gemini CLI settings.json 的 security 不是对象")?;
    let auth = security
        .entry("auth")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("Gemini CLI settings.json 的 security.auth 不是对象")?;
    if auth.get("selectedType").and_then(Value::as_str) == Some(GEMINI_API_KEY_AUTH) {
        return Ok(None);
    }
    auth.insert("selectedType".to_owned(), json!(GEMINI_API_KEY_AUTH));
    serde_json::to_string_pretty(&settings)
        .map(|text| Some(format!("{text}\n")))
        .map_err(|error| format!("序列化 Gemini CLI settings.json 失败: {error}"))
}

/// 合并配置。认证设置写入失败时，尝试恢复本次修改的 .env。
pub fn apply(api_key: &str, model: &str) -> Result<(), String> {
    apply_at(&gemini_dir()?, api_key, model)
}

fn apply_at(gemini_dir: &Path, api_key: &str, model: &str) -> Result<(), String> {
    let _lock = WRITES
        .lock()
        .map_err(|_| "Gemini CLI 配置写入锁已损坏".to_string())?;
    let env_file = gemini_dir.join(".env");
    let settings_file = gemini_dir.join("settings.json");
    let old_env = read_bounded(&env_file, MAX_ENV_BYTES)?;
    let old_settings = read_bounded(&settings_file, MAX_SETTINGS_BYTES)?;
    let env = build_env(old_env.as_deref(), api_key, model)?;
    let settings = build_settings(old_settings.as_deref())?;
    if env.len() as u64 > MAX_ENV_BYTES
        || settings
            .as_ref()
            .is_some_and(|text| text.len() as u64 > MAX_SETTINGS_BYTES)
    {
        return Err("Gemini CLI 配置超过大小限制".into());
    }

    if !replace_private_string_if_matches(&env_file, old_env.as_deref(), Some(&env))? {
        return Err("Gemini CLI .env 已更改，请重试".into());
    }
    if let Some(settings) = settings {
        let result = replace_private_string_if_matches(
            &settings_file,
            old_settings.as_deref(),
            Some(&settings),
        );
        let error = match result {
            Ok(true) => return Ok(()),
            Ok(false) => "Gemini CLI settings.json 已更改，请重试".to_owned(),
            Err(error) => error,
        };
        return match replace_private_string_if_matches(&env_file, Some(&env), old_env.as_deref()) {
            Ok(true) => Err(error),
            Ok(false) => Err(format!("{error}；.env 已被其他程序更改，已保留最新内容")),
            Err(restore) => Err(format!("{error}；恢复 .env 失败：{restore}")),
        };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_merges_managed_keys_and_preserves_others() {
        let existing = "# comment\nGOOGLE_CLOUD_PROJECT=\"p\"\nexport GEMINI_API_KEY=\"old\"\nGEMINI_MODEL='old-model'\n";
        let merged = build_env(Some(existing), "sk-new", "gemini-3-pro-preview").unwrap();
        assert!(merged.contains("# comment\n"));
        assert!(merged.contains("GOOGLE_CLOUD_PROJECT=\"p\"\n"));
        assert!(!merged.contains("old"));
        assert!(merged.contains("GOOGLE_GEMINI_BASE_URL=\"https://api.apikey.fan\"\n"));
        assert!(merged.contains("GEMINI_API_KEY=\"sk-new\"\n"));
        assert!(merged.contains("GEMINI_MODEL=\"gemini-3-pro-preview\"\n"));
    }

    #[test]
    fn multiline_unrelated_values_are_preserved_verbatim() {
        let other = "NOTES=\"start\r\nGEMINI_MODEL=part-of-notes\r\nend\"\r\n";
        let input = format!("{other}GEMINI_MODEL=`old\nmodel`\n");
        let merged = build_env(Some(&input), "sk-test", "gemini-test").unwrap();
        assert!(merged.starts_with(other));
        assert!(!merged.contains("old"));
        assert!(build_env(Some("NOTES=\"unfinished"), "sk-test", "gemini-test").is_err());
    }

    #[test]
    fn env_rejects_values_that_could_inject_new_variables() {
        assert!(build_env(None, "sk\nevil", "gemini-3").is_err());
        assert!(build_env(None, "sk", "gemini-3\r\nGEMINI_MODEL=other").is_err());
    }

    #[test]
    fn settings_sets_api_key_auth_and_skips_when_already_set() {
        assert_eq!(
            build_settings(None).unwrap(),
            Some("{\n  \"security\": {\n    \"auth\": {\n      \"selectedType\": \"gemini-api-key\"\n    }\n  }\n}\n".to_string())
        );
        let oauth =
            r#"{"security":{"auth":{"selectedType":"oauth-personal"}},"ui":{"theme":"dark"}}"#;
        let merged = build_settings(Some(oauth)).unwrap().unwrap();
        let value: Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(value["security"]["auth"]["selectedType"], "gemini-api-key");
        assert_eq!(value["ui"]["theme"], "dark");
        let already =
            r#"{"security":{"auth":{"selectedType":"gemini-api-key"}},"ui":{"theme":"dark"}}"#;
        assert_eq!(build_settings(Some(already)).unwrap(), None);
    }

    #[test]
    fn apply_writes_both_files_in_a_temp_profile() {
        // 用独立 HOME 隔离，避免触碰真实用户目录。
        let temp = tempfile::tempdir().unwrap();
        let fake_home = temp.path().join("home");
        fs::create_dir_all(&fake_home).unwrap();
        let gemini = fake_home.join(".gemini");

        let env = gemini.join(".env");
        let settings = gemini.join("settings.json");

        fs::create_dir_all(&gemini).unwrap();
        let backup = gemini.join(".env.bak");
        fs::write(&backup, "user backup").unwrap();
        apply_at(&gemini, "sk-test", "gemini-3-pro-preview").unwrap();

        assert_eq!(fs::read_to_string(backup).unwrap(), "user backup");
        let env_text = fs::read_to_string(&env).unwrap();
        assert!(env_text.contains("GEMINI_API_KEY=\"sk-test\""));
        let settings_text = fs::read_to_string(&settings).unwrap();
        let value: Value = serde_json::from_str(&settings_text).unwrap();
        assert_eq!(value["security"]["auth"]["selectedType"], "gemini-api-key");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&env).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(&settings).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
