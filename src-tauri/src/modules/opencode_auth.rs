use crate::models::codex::CodexAccount;
use crate::models::github_copilot::GitHubCopilotAccount;
use crate::models::grok::GrokAccount;
use crate::modules::{codex_account, codex_oauth, logger};
use serde_json::json;
use std::fs;
use std::path::PathBuf;

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|item| item == &path) {
        paths.push(path);
    }
}

fn get_opencode_auth_json_path_candidates() -> Result<Vec<PathBuf>, String> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    // OpenCode CLI 以 XDG_DATA_HOME 为优先路径（未设置时默认 ~/.local/share）。
    if let Ok(xdg_data_home) = std::env::var("XDG_DATA_HOME") {
        let trimmed = xdg_data_home.trim();
        if !trimmed.is_empty() {
            push_unique_path(
                &mut candidates,
                PathBuf::from(trimmed).join("opencode").join("auth.json"),
            );
        }
    }

    if let Some(home) = dirs::home_dir() {
        push_unique_path(
            &mut candidates,
            home.join(".local")
                .join("share")
                .join("opencode")
                .join("auth.json"),
        );
    }

    // 兼容历史实现写入的位置，作为回退和迁移来源。
    if let Some(data_dir) = dirs::data_dir() {
        push_unique_path(&mut candidates, data_dir.join("opencode").join("auth.json"));
    }

    if candidates.is_empty() {
        return Err("无法推断 OpenCode auth.json 路径".to_string());
    }

    Ok(candidates)
}

/// 获取 OpenCode 的 auth.json 路径
///
/// - 优先使用 OpenCode CLI 同源路径：$XDG_DATA_HOME/opencode/auth.json 或 ~/.local/share/opencode/auth.json
/// - 兼容回退历史路径：系统数据目录/opencode/auth.json
pub fn get_opencode_auth_json_path() -> Result<PathBuf, String> {
    let candidates = get_opencode_auth_json_path_candidates()?;
    Ok(candidates
        .first()
        .cloned()
        .ok_or_else(|| "无法推断 OpenCode auth.json 路径".to_string())?)
}

fn atomic_write(path: &PathBuf, content: &str) -> Result<(), String> {
    crate::modules::atomic_write::write_string_atomic(path, content)
        .map_err(|e| format!("写入 auth.json 失败: {}", e))
}

fn build_openai_payload(account: &CodexAccount) -> Result<serde_json::Value, String> {
    let refresh = account
        .tokens
        .refresh_token
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Codex refresh_token 缺失，无法同步到 OpenCode".to_string())?;
    let expires = decode_token_exp_ms(&account.tokens.access_token)
        .ok_or_else(|| "Codex access_token 缺少 exp，无法同步到 OpenCode".to_string())?;

    let mut payload = json!({
        "type": "oauth",
        "access": account.tokens.access_token,
        "refresh": refresh,
        "expires": expires,
    });

    if let Some(account_id) = account.account_id.clone() {
        payload["accountId"] = json!(account_id);
    }

    Ok(payload)
}

fn build_github_copilot_payload(
    account: &GitHubCopilotAccount,
) -> Result<serde_json::Value, String> {
    let token = account.github_access_token.trim().to_string();
    if token.is_empty() {
        return Err("GitHub Copilot access_token 缺失，无法同步到 OpenCode".to_string());
    }

    Ok(json!({
        "type": "oauth",
        "access": token,
        "refresh": token,
        "expires": 0,
    }))
}

fn decode_token_exp_ms(access_token: &str) -> Option<i64> {
    let payload = codex_account::decode_jwt_payload(access_token).ok()?;
    payload.exp.map(|exp| exp * 1000)
}

fn grok_expires_ms(account: &GrokAccount) -> i64 {
    if let Some(expires_at) = account.expires_at {
        if expires_at > 10_000_000_000 {
            return expires_at;
        }
        return expires_at.saturating_mul(1000);
    }
    decode_token_exp_ms(&account.access_token).unwrap_or(0)
}

fn build_xai_payload_from_grok(account: &GrokAccount) -> Result<serde_json::Value, String> {
    if account.is_api_key_auth() {
        if account
            .api_base_url
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
        {
            return Err("Grok 第三方 API Key 账号不能写入 OpenCode 的 xai 条目".to_string());
        }
        let key = account
            .resolved_api_key()
            .ok_or_else(|| "Grok API Key 缺失，无法同步到 OpenCode".to_string())?;
        return Ok(json!({
            "type": "api",
            "key": key,
        }));
    }

    let access = account.access_token.trim();
    if access.is_empty() {
        return Err("Grok access_token 缺失，无法同步到 OpenCode".to_string());
    }
    let refresh = account
        .refresh_token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Grok refresh_token 缺失，无法同步到 OpenCode".to_string())?;

    Ok(json!({
        "type": "oauth",
        "access": access,
        "refresh": refresh,
        "expires": grok_expires_ms(account),
    }))
}

fn replace_provider_entry(provider_key: &str, payload: serde_json::Value) -> Result<(), String> {
    let auth_paths = get_opencode_auth_json_path_candidates()?;
    let target_auth_path = get_opencode_auth_json_path()?;
    let source_auth_path = auth_paths.iter().find(|path| path.exists()).cloned();

    let mut auth_json = if let Some(source_path) = source_auth_path.as_ref() {
        let content = fs::read_to_string(source_path).map_err(|e| {
            format!(
                "读取 OpenCode auth.json 失败 ({}): {}",
                source_path.display(),
                e
            )
        })?;
        serde_json::from_str::<serde_json::Value>(&content).map_err(|e| {
            format!(
                "解析 OpenCode auth.json 失败 ({}): {}",
                source_path.display(),
                e
            )
        })?
    } else {
        json!({})
    };

    if !auth_json.is_object() {
        auth_json = json!({});
    }

    if let Some(map) = auth_json.as_object_mut() {
        map.insert(provider_key.to_string(), payload);
    }

    let content = serde_json::to_string_pretty(&auth_json)
        .map_err(|e| format!("序列化 OpenCode auth.json 失败: {}", e))?;
    atomic_write(&target_auth_path, &content)?;

    // 若历史路径文件存在，保持同步，避免旧版本读取不到最新登录态。
    for extra_path in &auth_paths {
        if extra_path == &target_auth_path || !extra_path.exists() {
            continue;
        }
        if let Err(err) = atomic_write(extra_path, &content) {
            logger::log_warn(&format!(
                "同步 OpenCode 备用 auth.json 失败 ({}): {}",
                extra_path.display(),
                err
            ));
        }
    }

    if let Some(source_path) = source_auth_path {
        if source_path != target_auth_path {
            logger::log_info(&format!(
                "OpenCode auth.json 已迁移: {} -> {}",
                source_path.display(),
                target_auth_path.display()
            ));
        }
    }

    logger::log_info(&format!(
        "已更新 OpenCode auth.json 中的 {} 记录: {}",
        provider_key,
        target_auth_path.display()
    ));
    Ok(())
}

/// 使用 Codex 账号的 token 替换 OpenCode auth.json 中的 openai 记录
pub fn replace_openai_entry_from_codex(account: &CodexAccount) -> Result<(), String> {
    // 确保 token 未过期
    if codex_oauth::is_token_expired(&account.tokens.access_token) {
        return Err("Codex access_token 已过期，无法同步到 OpenCode".to_string());
    }

    let openai_payload = build_openai_payload(account)?;
    replace_provider_entry("openai", openai_payload)
}

/// 使用 GitHub Copilot 账号的 token 替换 OpenCode auth.json 中的 github-copilot 记录
pub fn replace_github_copilot_entry_from_account(
    account: &GitHubCopilotAccount,
) -> Result<(), String> {
    let payload = build_github_copilot_payload(account)?;
    replace_provider_entry("github-copilot", payload)
}

/// 使用 Grok 账号凭据替换 OpenCode auth.json 中的 xai 记录
pub fn replace_xai_entry_from_grok(account: &GrokAccount) -> Result<(), String> {
    let payload = build_xai_payload_from_grok(account)?;
    replace_provider_entry("xai", payload)
}

#[cfg(test)]
mod tests {
    use super::build_xai_payload_from_grok;
    use crate::models::grok::{GrokAccount, GrokAuthMode};

    fn sample_oauth_account() -> GrokAccount {
        GrokAccount {
            id: "account-1".to_string(),
            email: "person@example.com".to_string(),
            auth_mode: GrokAuthMode::Oauth,
            tags: None,
            first_name: None,
            last_name: None,
            user_id: None,
            principal_id: None,
            principal_type: None,
            team_id: None,
            profile_image_asset_id: None,
            coding_data_retention_opt_out: None,
            access_token: "secret-access".to_string(),
            api_key: None,
            api_base_url: None,
            api_model: None,
            refresh_token: Some("secret-refresh".to_string()),
            id_token: None,
            token_type: Some("Bearer".to_string()),
            expires_at: Some(1_900_000_000),
            expires_at_raw: None,
            oidc_issuer: None,
            oidc_client_id: None,
            token_endpoint: None,
            plan_type: None,
            quota: None,
            auth_raw: None,
            billing_raw: None,
            subscription_raw: None,
            user_raw: None,
            task_usage_raw: None,
            has_grok_code_access: None,
            status: None,
            status_reason: None,
            quota_query_last_error: None,
            quota_query_last_error_at: None,
            usage_updated_at: None,
            working_dir: None,
            created_at: 1,
            last_used: 1,
        }
    }

    #[test]
    fn grok_oauth_payload_uses_xai_oauth_shape() {
        let payload = build_xai_payload_from_grok(&sample_oauth_account()).expect("oauth payload");
        assert_eq!(payload["type"], "oauth");
        assert_eq!(payload["access"], "secret-access");
        assert_eq!(payload["refresh"], "secret-refresh");
        assert_eq!(payload["expires"], 1_900_000_000_000i64);
    }

    #[test]
    fn grok_oauth_payload_falls_back_to_jwt_exp() {
        let mut account = sample_oauth_account();
        account.expires_at = None;
        account.access_token = "eyJhbGciOiJub25lIn0.eyJleHAiOjE5MDAwMDAwMDB9.sig".to_string();
        let payload = build_xai_payload_from_grok(&account).expect("jwt payload");
        assert_eq!(payload["expires"], 1_900_000_000_000i64);
    }

    #[test]
    fn grok_oauth_payload_requires_refresh_token() {
        let mut account = sample_oauth_account();
        account.refresh_token = None;
        let error = build_xai_payload_from_grok(&account).expect_err("missing refresh");
        assert!(error.contains("refresh_token"));
    }

    #[test]
    fn grok_official_api_key_payload_uses_api_shape() {
        let mut account = sample_oauth_account();
        account.auth_mode = GrokAuthMode::ApiKey;
        account.access_token.clear();
        account.refresh_token = None;
        account.api_key = Some("xai-test-key".to_string());
        let payload = build_xai_payload_from_grok(&account).expect("api payload");
        assert_eq!(payload["type"], "api");
        assert_eq!(payload["key"], "xai-test-key");
    }

    #[test]
    fn grok_third_party_api_key_is_rejected() {
        let mut account = sample_oauth_account();
        account.auth_mode = GrokAuthMode::ApiKey;
        account.access_token.clear();
        account.refresh_token = None;
        account.api_key = Some("proxy-key".to_string());
        account.api_base_url = Some("https://example.com/v1".to_string());
        account.api_model = Some("custom-model".to_string());
        let error = build_xai_payload_from_grok(&account).expect_err("third-party");
        assert!(error.contains("第三方"));
    }
}
