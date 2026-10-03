//! Independent agy account support. The system keyring is the source of truth;
//! never fall back to the IDE database or Cockpit's global current-account marker.
use crate::{models::Account, modules};
use std::path::{Path, PathBuf};
use std::time::Duration;

static AUTH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(serde::Serialize)]
pub struct CliStatus {
    pub executable: Option<String>,
    pub version: Option<String>,
    pub version_error: Option<String>,
    pub config_dir: String,
    pub api_key_mode: bool,
    pub credential_present: bool,
    pub credential_error: Option<String>,
    pub current_account_id: Option<String>,
}

fn config_dir() -> Result<PathBuf, String> {
    dirs::home_dir()
        .map(|home| home.join(".gemini/antigravity-cli"))
        .ok_or_else(|| "无法定位 Antigravity CLI 配置目录".to_string())
}

fn uses_api_key(config: &Path) -> Result<bool, String> {
    let raw = match std::fs::read(config.join("settings.json")) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("无法读取 Antigravity CLI settings.json: {e}")),
    };
    let settings: serde_json::Value = serde_json::from_slice(&raw)
        .map_err(|_| "Antigravity CLI settings.json 不是有效的 JSON".to_string())?;
    Ok(settings.get("modelProvider").and_then(|v| v.as_str()) == Some("gemini"))
}

fn executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        return path
            .metadata()
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false);
    }
    #[cfg(not(unix))]
    true
}

pub fn find_executable() -> Option<PathBuf> {
    let name = if cfg!(windows) { "agy.exe" } else { "agy" };
    let mut candidates: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths)
                .filter(|p| p.is_absolute())
                .map(|p| p.join(name))
                .collect()
        })
        .unwrap_or_default();
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".local/bin").join(name));
        #[cfg(target_os = "windows")]
        candidates.push(home.join("AppData/Local/agy/bin").join(name));
    }
    #[cfg(target_os = "windows")]
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local).join("agy/bin").join(name));
    }
    candidates.into_iter().find(|path| executable_file(path))
}

async fn read_credential(
) -> Result<Option<modules::antigravity_credential::AntigravitySystemCredential>, String> {
    tokio::task::spawn_blocking(modules::antigravity_credential::read_antigravity_system_credential)
        .await
        .map_err(|_| "读取 Antigravity CLI 钥匙环任务失败".to_string())?
}

fn match_account(accounts: Vec<Account>, refresh_token: &str) -> Option<Account> {
    accounts
        .into_iter()
        .find(|account| !account.pending_oauth && account.token.refresh_token == refresh_token)
}

pub async fn current_account() -> Result<Option<Account>, String> {
    let _guard = AUTH_LOCK.lock().await;
    if uses_api_key(&config_dir()?)? {
        return Ok(None);
    }
    let Some(credential) = read_credential().await? else {
        return Ok(None);
    };
    Ok(match_account(
        modules::list_accounts()?,
        &credential.refresh_token,
    ))
}

pub async fn status() -> Result<CliStatus, String> {
    let config = config_dir()?;
    let api_key_mode = uses_api_key(&config)?;
    let executable = find_executable();
    let version_result = async {
        let Some(path) = executable.as_ref() else {
            return Ok(None);
        };
        let output = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::process::Command::new(path)
                .arg("--version")
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| "Antigravity CLI 版本检测超时".to_string())?
        .map_err(|e| format!("无法执行 agy --version: {e}"))?;
        if !output.status.success() {
            return Err("agy --version 执行失败".to_string());
        }
        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(Some(version))
    }
    .await;
    let (version, version_error) = match version_result {
        Ok(version) => (version, None),
        Err(error) => (None, Some(error)),
    };
    let _guard = AUTH_LOCK.lock().await;
    let (credential_present, credential_error, current_account_id) = match read_credential().await {
        Ok(Some(credential)) => {
            let current = if api_key_mode {
                None
            } else {
                match_account(modules::list_accounts()?, &credential.refresh_token).map(|a| a.id)
            };
            (true, None, current)
        }
        Ok(None) => (false, None, None),
        Err(error) => (false, Some(error), None),
    };
    Ok(CliStatus {
        executable: executable.map(|p| p.to_string_lossy().into_owned()),
        version,
        version_error,
        config_dir: config.to_string_lossy().into_owned(),
        api_key_mode,
        credential_present,
        credential_error,
        current_account_id,
    })
}

pub async fn import_account() -> Result<Account, String> {
    let _guard = AUTH_LOCK.lock().await;
    let credential = read_credential()
        .await?
        .ok_or_else(|| "未找到 Antigravity CLI 系统凭据，请先运行 agy 登录".to_string())?;
    if credential
        .auth_method
        .as_deref()
        .is_some_and(|method| method != "consumer")
    {
        return Err(
            "当前 CLI 凭据不是 Google 个人账号 OAuth 登录，暂不支持导入此认证方式".to_string(),
        );
    }
    let mut account = modules::import::import_from_refresh_token(
        credential.refresh_token,
        "Antigravity CLI 系统钥匙环",
    )
    .await?;
    // Some refresh responses omit the ID token. Preserve the CLI's identity
    // token in that case so a subsequent switch can round-trip the profile.
    if account.token.id_token.is_none() && credential.id_token.is_some() {
        account.token.id_token = credential.id_token;
        modules::save_account(&account)?;
    }
    Ok(account)
}

pub async fn switch_account(account_id: &str) -> Result<Account, String> {
    let _guard = AUTH_LOCK.lock().await;
    if uses_api_key(&config_dir()?)? {
        return Err("CLI 正在使用 Gemini API Key。请先从 ~/.gemini/antigravity-cli/settings.json 移除 modelProvider，再切换 Google 账号".to_string());
    }
    let mut account = modules::load_account(account_id)?;
    if account.pending_oauth || account.disabled || account.token.refresh_token.trim().is_empty() {
        return Err("该账号尚未授权或已失效，请先完成 OAuth 授权".to_string());
    }
    account.token = modules::oauth::ensure_fresh_token(&account.token).await?;
    // Persist refreshed tokens before touching the keyring. A failed keyring write
    // must never change the IDE/global current-account marker.
    modules::save_account(&account)?;
    let to_write = account.clone();
    tokio::task::spawn_blocking(move || {
        modules::antigravity_credential::write_antigravity_system_credential(&to_write)
    })
    .await
    .map_err(|_| "写入 Antigravity CLI 钥匙环任务失败".to_string())??;
    let stored = read_credential()
        .await?
        .ok_or_else(|| "钥匙环写入后未找到凭据，请重新检查 CLI 登录状态".to_string())?;
    if stored.refresh_token != account.token.refresh_token {
        return Err("钥匙环凭据已被其他客户端更改，请关闭相关客户端后重试".to_string());
    }
    account.update_last_used();
    if let Err(error) = modules::save_account(&account) {
        modules::logger::log_warn(&format!("CLI 账号已切换，保存最近使用时间失败: {error}"));
    }
    modules::websocket::broadcast_data_changed("antigravity_cli_switch");
    Ok(account)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TokenData;

    #[test]
    fn current_cli_account_is_matched_by_credential_not_global_selection() {
        let account = Account::new(
            "cli".into(),
            "cli@example.com".into(),
            TokenData::new(
                "access".into(),
                "cli-refresh".into(),
                3600,
                None,
                None,
                None,
            ),
        );
        assert!(match_account(vec![account.clone()], "external-refresh").is_none());
        assert_eq!(
            match_account(vec![account.clone()], "cli-refresh")
                .unwrap()
                .id,
            "cli"
        );
        let mut pending = account;
        pending.pending_oauth = true;
        assert!(match_account(vec![pending], "cli-refresh").is_none());
    }

    #[test]
    fn detects_api_key_mode_and_rejects_corrupt_settings() {
        let dir = std::env::temp_dir().join(format!("agy-settings-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!uses_api_key(&dir).unwrap());
        std::fs::write(dir.join("settings.json"), r#"{"modelProvider":"gemini"}"#).unwrap();
        assert!(uses_api_key(&dir).unwrap());
        std::fs::write(dir.join("settings.json"), "{}").unwrap();
        assert!(!uses_api_key(&dir).unwrap());
        std::fs::write(dir.join("settings.json"), "invalid").unwrap();
        assert!(uses_api_key(&dir).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
