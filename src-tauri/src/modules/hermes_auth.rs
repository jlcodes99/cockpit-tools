use crate::models::codex::CodexAccount;
use crate::modules::{codex_oauth, logger};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

const HERMES_CODEX_PROVIDER: &str = "openai-codex";
const HERMES_CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";

fn resolve_user_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if trimmed == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }

    if let Some(stripped) = trimmed
        .strip_prefix("~/")
        .or_else(|| trimmed.strip_prefix("~\\"))
    {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }

    PathBuf::from(trimmed)
}

fn get_hermes_home_dir() -> Result<PathBuf, String> {
    if let Ok(value) = std::env::var("HERMES_HOME") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(resolve_user_path(trimmed));
        }
    }

    dirs::home_dir()
        .map(|home| home.join(".hermes"))
        .ok_or_else(|| "无法推断 Hermes 配置目录".to_string())
}

fn get_hermes_auth_json_path() -> Result<PathBuf, String> {
    Ok(get_hermes_home_dir()?.join("auth.json"))
}

fn read_hermes_auth_json(path: &Path) -> Result<Value, String> {
    match fs::read_to_string(path) {
        Ok(content) => serde_json::from_str::<Value>(&content)
            .map_err(|e| format!("解析 Hermes auth.json 失败 ({}): {}", path.display(), e)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(err) => Err(format!(
            "读取 Hermes auth.json 失败 ({}): {}",
            path.display(),
            err
        )),
    }
}

fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    crate::modules::atomic_write::write_string_atomic(path, content)
        .map_err(|e| format!("写入 Hermes auth.json 失败: {}", e))
}

fn codex_account_id(account: &CodexAccount) -> String {
    account
        .account_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn codex_refresh_token(account: &CodexAccount) -> String {
    account
        .tokens
        .refresh_token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn build_hermes_codex_tokens(account: &CodexAccount) -> Result<Value, String> {
    if account.is_api_key_auth() {
        return Err("Codex API Key 账号不支持同步到 Hermes openai-codex 凭证".to_string());
    }
    if account.tokens.access_token.trim().is_empty() {
        return Err("Codex access_token 缺失，无法同步到 Hermes".to_string());
    }

    Ok(json!({
        "access_token": account.tokens.access_token,
        "account_id": codex_account_id(account),
        "id_token": account.tokens.id_token,
        "refresh_token": codex_refresh_token(account),
    }))
}

fn ensure_object(value: &mut Value) -> &mut serde_json::Map<String, Value> {
    if !value.is_object() {
        *value = json!({});
    }
    value.as_object_mut().expect("value must be object")
}

fn update_provider_entry(auth_json: &mut Value, tokens: Value, refreshed_at: &str) {
    let root = ensure_object(auth_json);
    let providers_value = root.entry("providers").or_insert_with(|| json!({}));
    let providers = ensure_object(providers_value);
    let provider_value = providers
        .entry(HERMES_CODEX_PROVIDER.to_string())
        .or_insert_with(|| json!({}));
    let provider = ensure_object(provider_value);

    provider.insert("tokens".to_string(), tokens);
    provider.insert(
        "last_refresh".to_string(),
        Value::String(refreshed_at.to_string()),
    );
    provider.insert(
        "auth_mode".to_string(),
        Value::String("chatgpt".to_string()),
    );

    for key in [
        "last_auth_error",
        "last_error_code",
        "last_error_reason",
        "last_error_message",
        "last_error_reset_at",
    ] {
        provider.remove(key);
    }
}

fn default_pool_entry() -> Value {
    json!({
        "id": "cockpit-codex",
        "label": "Cockpit Codex",
        "auth_type": "oauth",
        "priority": 8,
        "source": "cockpit",
        "base_url": HERMES_CODEX_BASE_URL,
        "request_count": 0,
    })
}

fn update_pool_entry(
    entry: &mut Value,
    access_token: &str,
    refresh_token: &str,
    refreshed_at: &str,
) {
    let entry_obj = ensure_object(entry);
    entry_obj.insert(
        "access_token".to_string(),
        Value::String(access_token.to_string()),
    );
    entry_obj.insert(
        "refresh_token".to_string(),
        Value::String(refresh_token.to_string()),
    );
    entry_obj.insert(
        "last_refresh".to_string(),
        Value::String(refreshed_at.to_string()),
    );
    entry_obj.insert("last_status".to_string(), Value::Null);
    entry_obj.insert("last_status_at".to_string(), Value::Null);
    entry_obj.insert("last_error_code".to_string(), Value::Null);
    entry_obj.insert("last_error_reason".to_string(), Value::Null);
    entry_obj.insert("last_error_message".to_string(), Value::Null);
    entry_obj.insert("last_error_reset_at".to_string(), Value::Null);
}

fn update_credential_pool(
    auth_json: &mut Value,
    access_token: &str,
    refresh_token: &str,
    refreshed_at: &str,
) {
    let root = ensure_object(auth_json);
    let pool_value = root.entry("credential_pool").or_insert_with(|| json!({}));
    let pool = ensure_object(pool_value);
    let entries_value = pool
        .entry(HERMES_CODEX_PROVIDER.to_string())
        .or_insert_with(|| json!([]));
    if !entries_value.is_array() {
        *entries_value = json!([]);
    }
    let entries = entries_value.as_array_mut().expect("value must be array");
    if entries.is_empty() {
        entries.push(default_pool_entry());
    }
    for entry in entries {
        update_pool_entry(entry, access_token, refresh_token, refreshed_at);
    }
}

fn apply_codex_account_to_hermes_auth_value(
    auth_json: &mut Value,
    account: &CodexAccount,
    refreshed_at: &str,
) -> Result<(), String> {
    let tokens = build_hermes_codex_tokens(account)?;
    let access_token = account.tokens.access_token.trim().to_string();
    let refresh_token = codex_refresh_token(account);

    update_provider_entry(auth_json, tokens, refreshed_at);
    update_credential_pool(auth_json, &access_token, &refresh_token, refreshed_at);
    ensure_object(auth_json).insert(
        "updated_at".to_string(),
        Value::String(refreshed_at.to_string()),
    );
    Ok(())
}

/// 使用当前 Codex 账号覆盖 Hermes openai-codex 凭证和凭证池，确保 Hermes 使用同一账号额度。
pub fn replace_openai_codex_entry_from_codex(account: &CodexAccount) -> Result<(), String> {
    if codex_oauth::is_token_expired(&account.tokens.access_token) {
        return Err("Codex access_token 已过期，无法同步到 Hermes".to_string());
    }

    let auth_path = get_hermes_auth_json_path()?;
    let mut auth_json = read_hermes_auth_json(&auth_path)?;
    let refreshed_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    apply_codex_account_to_hermes_auth_value(&mut auth_json, account, &refreshed_at)?;

    let content = serde_json::to_string_pretty(&auth_json)
        .map_err(|e| format!("序列化 Hermes auth.json 失败: {}", e))?;
    if let Some(parent) = auth_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("创建 Hermes 配置目录失败 ({}): {}", parent.display(), e))?;
    }
    atomic_write(&auth_path, &content)?;

    logger::log_info(&format!(
        "已更新 Hermes {} 凭证和 credential_pool: account_id={}, target_file={}",
        HERMES_CODEX_PROVIDER,
        account.id,
        auth_path.display()
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::codex::{CodexAuthMode, CodexTokens};

    fn make_oauth_account(refresh_token: Option<&str>) -> CodexAccount {
        let tokens = CodexTokens {
            id_token: "id.jwt.token".to_string(),
            access_token: "at-live-token".to_string(),
            refresh_token: refresh_token.map(|value| value.to_string()),
        };
        let mut account = CodexAccount::new(
            "codex_test_account".to_string(),
            "demo@example.com".to_string(),
            tokens,
        );
        account.account_id = Some("chatgpt-account-id".to_string());
        account
    }

    #[test]
    fn applies_codex_account_to_provider_and_pool() {
        let account = make_oauth_account(Some("rt-live-token"));
        let mut auth = json!({
            "providers": {
                "openai-codex": {
                    "auth_mode": "chatgpt",
                    "last_auth_error": { "code": "rate_limited" },
                    "last_error_code": "exhausted",
                    "tokens": {
                        "access_token": "old-access",
                        "refresh_token": "old-refresh",
                        "id_token": "old-id",
                        "account_id": "old-account"
                    }
                }
            },
            "credential_pool": {
                "openai-codex": [
                    {
                        "id": "primary",
                        "access_token": "old-access",
                        "refresh_token": "old-refresh",
                        "last_status": "exhausted",
                        "last_error_message": "old error"
                    }
                ]
            }
        });

        apply_codex_account_to_hermes_auth_value(&mut auth, &account, "2026-07-07T00:00:00Z")
            .expect("sync should succeed");

        let provider = &auth["providers"][HERMES_CODEX_PROVIDER];
        assert_eq!(provider["auth_mode"], "chatgpt");
        assert!(provider.get("last_auth_error").is_none());
        assert!(provider.get("last_error_code").is_none());
        assert_eq!(provider["tokens"]["access_token"], "at-live-token");
        assert_eq!(provider["tokens"]["refresh_token"], "rt-live-token");
        assert_eq!(provider["tokens"]["id_token"], "id.jwt.token");
        assert_eq!(provider["tokens"]["account_id"], "chatgpt-account-id");

        let pool_entry = &auth["credential_pool"][HERMES_CODEX_PROVIDER][0];
        assert_eq!(pool_entry["id"], "primary");
        assert_eq!(pool_entry["access_token"], "at-live-token");
        assert_eq!(pool_entry["refresh_token"], "rt-live-token");
        assert!(pool_entry["last_status"].is_null());
        assert!(pool_entry["last_error_message"].is_null());
        assert_eq!(auth["updated_at"], "2026-07-07T00:00:00Z");
    }

    #[test]
    fn creates_empty_refresh_token_pool_entry_for_access_token_only_accounts() {
        let account = make_oauth_account(None);
        let mut auth = json!({});

        apply_codex_account_to_hermes_auth_value(&mut auth, &account, "2026-07-07T00:00:00Z")
            .expect("access-token-only sync should succeed");

        assert_eq!(
            auth["providers"][HERMES_CODEX_PROVIDER]["tokens"]["refresh_token"],
            ""
        );
        assert_eq!(
            auth["credential_pool"][HERMES_CODEX_PROVIDER][0]["refresh_token"],
            ""
        );
        assert_eq!(
            auth["credential_pool"][HERMES_CODEX_PROVIDER][0]["base_url"],
            HERMES_CODEX_BASE_URL
        );
    }

    #[test]
    fn rejects_codex_api_key_accounts() {
        let mut account = make_oauth_account(Some("rt-live-token"));
        account.auth_mode = CodexAuthMode::Apikey;
        account.openai_api_key = Some("sk-test".to_string());
        let mut auth = json!({});

        let err =
            apply_codex_account_to_hermes_auth_value(&mut auth, &account, "2026-07-07T00:00:00Z")
                .expect_err("API key accounts should not sync to Hermes Codex OAuth");

        assert!(err.contains("API Key"));
        assert_eq!(auth, json!({}));
    }
}
