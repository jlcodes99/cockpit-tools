use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

const PROVIDER_CURRENT_STATE_FILE: &str = "provider_current_accounts.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProviderCurrentState {
    #[serde(default = "default_version")]
    version: String,
    #[serde(default)]
    current_accounts: HashMap<String, String>,
}

fn default_version() -> String {
    "1.0".to_string()
}

impl ProviderCurrentState {
    fn new() -> Self {
        Self {
            version: default_version(),
            current_accounts: HashMap::new(),
        }
    }
}

fn normalize_platform(platform: &str) -> Result<&'static str, String> {
    match platform.trim() {
        "windsurf" => Ok("windsurf"),
        "kiro" => Ok("kiro"),
        "cursor" => Ok("cursor"),
        "gemini" => Ok("gemini"),
        "claude_desktop_account" => Ok("claude_desktop_account"),
        "claude_code_account" => Ok("claude_code_account"),
        "codebuddy" => Ok("codebuddy"),
        "codebuddy_cn" | "codebuddy-cn" => Ok("codebuddy_cn"),
        "qoder" => Ok("qoder"),
        "trae" => Ok("trae"),
        "workbuddy" => Ok("workbuddy"),
        "github_copilot" | "github-copilot" | "ghcp" => Ok("github_copilot"),
        other => Err(format!("不支持的平台: {}", other)),
    }
}

fn get_state_path() -> Result<PathBuf, String> {
    Ok(crate::modules::account::get_data_dir()?.join(PROVIDER_CURRENT_STATE_FILE))
}

fn load_state() -> Result<ProviderCurrentState, String> {
    let path = get_state_path()?;
    if !path.exists() {
        return Ok(ProviderCurrentState::new());
    }

    let content = fs::read_to_string(&path)
        .map_err(|e| format!("读取当前账号映射失败: path={}, error={}", path.display(), e))?;
    if content.trim().is_empty() {
        return Ok(ProviderCurrentState::new());
    }

    crate::modules::atomic_write::parse_json_with_auto_restore::<ProviderCurrentState>(
        &path, &content,
    )
    .map_err(|e| format!("解析当前账号映射失败: path={}, error={}", path.display(), e))
}

fn save_state(state: &ProviderCurrentState) -> Result<(), String> {
    let path = get_state_path()?;
    let content = serde_json::to_string_pretty(state)
        .map_err(|e| format!("序列化当前账号映射失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("保存当前账号映射失败: path={}, error={}", path.display(), e))
}

pub fn get_current_account_id(platform: &str) -> Result<Option<String>, String> {
    let key = normalize_platform(platform)?;
    let state = load_state()?;
    Ok(state
        .current_accounts
        .get(key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty()))
}

pub fn resolve_existing_current_account_id<'a, I>(platform: &str, existing_ids: I) -> Option<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let current_id = get_current_account_id(platform).ok().flatten()?;
    if existing_ids.into_iter().any(|id| id == current_id.as_str()) {
        Some(current_id)
    } else {
        let _ = set_current_account_id(platform, None);
        None
    }
}

pub fn set_current_account_id(platform: &str, account_id: Option<&str>) -> Result<(), String> {
    let key = normalize_platform(platform)?;
    let mut state = load_state()?;
    if let Some(account_id) = account_id.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then_some(trimmed)
    }) {
        state
            .current_accounts
            .insert(key.to_string(), account_id.to_string());
    } else {
        state.current_accounts.remove(key);
    }
    save_state(&state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_data_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cockpit-provider-current-{}-{}",
            name,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp data dir");
        dir
    }

    struct DataDirGuard {
        dir: PathBuf,
        previous_data_dir: Option<String>,
    }

    impl DataDirGuard {
        fn new(name: &str) -> Self {
            let dir = temp_data_dir(name);
            let previous_data_dir = std::env::var("COCKPIT_TOOLS_DATA_DIR").ok();
            std::env::set_var("COCKPIT_TOOLS_DATA_DIR", &dir);
            Self {
                dir,
                previous_data_dir,
            }
        }
    }

    impl Drop for DataDirGuard {
        fn drop(&mut self) {
            match self.previous_data_dir.as_ref() {
                Some(value) => std::env::set_var("COCKPIT_TOOLS_DATA_DIR", value),
                None => std::env::remove_var("COCKPIT_TOOLS_DATA_DIR"),
            }
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn provider_current_state_normalizes_platform_aliases() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .expect("lock env");
        let _guard = DataDirGuard::new("aliases");

        set_current_account_id("github-copilot", Some("gh-account")).expect("set github alias");
        assert_eq!(
            get_current_account_id("github_copilot").expect("get github canonical"),
            Some("gh-account".to_string())
        );
        assert_eq!(
            get_current_account_id("ghcp").expect("get github short alias"),
            Some("gh-account".to_string())
        );

        set_current_account_id("codebuddy-cn", Some("cn-account")).expect("set cn alias");
        assert_eq!(
            get_current_account_id("codebuddy_cn").expect("get cn canonical"),
            Some("cn-account".to_string())
        );
    }

    #[test]
    fn provider_current_state_clears_stale_current_account_id() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .expect("lock env");
        let _guard = DataDirGuard::new("stale");

        set_current_account_id("cursor", Some("stale-account")).expect("set cursor current");
        assert_eq!(
            resolve_existing_current_account_id("cursor", ["other-account"].iter().copied()),
            None
        );
        assert_eq!(
            get_current_account_id("cursor").expect("get cursor after stale cleanup"),
            None
        );

        set_current_account_id("cursor", Some("active-account")).expect("set active cursor");
        assert_eq!(
            resolve_existing_current_account_id(
                "cursor",
                ["other-account", "active-account"].iter().copied()
            ),
            Some("active-account".to_string())
        );
    }
}
