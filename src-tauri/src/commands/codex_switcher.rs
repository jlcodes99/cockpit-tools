use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::commands::codex;
use crate::models::codex::{
    CodexAccount, CodexApiProviderMode, CodexAuthMode, CodexQuota, CodexQuotaErrorInfo,
};
use crate::modules::{atomic_write, codex_account, codex_quota, config, logger, process, tray};

const SETTINGS_FILE_NAME: &str = "codex_switcher_settings.json";
const LOCAL_AD_FILE_NAME: &str = "codex_switcher_ad.json";
const REMOTE_AD_ENV: &str = "CODEX_SWITCHER_AD_URL";
const MAX_AD_CONFIG_BYTES: usize = 128 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct CodexSwitcherAccount {
    pub id: String,
    pub email: String,
    pub auth_mode: CodexAuthMode,
    pub api_base_url: Option<String>,
    pub api_provider_mode: CodexApiProviderMode,
    pub api_provider_id: Option<String>,
    pub api_provider_name: Option<String>,
    pub bound_oauth_account_id: Option<String>,
    pub user_id: Option<String>,
    pub plan_type: Option<String>,
    pub subscription_active_until: Option<String>,
    pub auth_file_plan_type: Option<String>,
    pub account_id: Option<String>,
    pub organization_id: Option<String>,
    pub account_name: Option<String>,
    pub account_structure: Option<String>,
    pub requires_reauth: bool,
    pub reauth_reason: Option<String>,
    pub quota: Option<CodexQuota>,
    pub quota_error: Option<CodexQuotaErrorInfo>,
    pub usage_updated_at: Option<i64>,
    pub created_at: i64,
    pub last_used: i64,
    pub is_api_key_auth: bool,
    pub is_current: bool,
    pub banned: bool,
    pub disabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodexSwitcherListResponse {
    pub accounts: Vec<CodexSwitcherAccount>,
    pub current_account_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexSwitcherSettings {
    #[serde(default)]
    pub restart_app_after_switch: bool,
    #[serde(default)]
    pub delete_banned_accounts: bool,
}

impl Default for CodexSwitcherSettings {
    fn default() -> Self {
        Self {
            restart_app_after_switch: false,
            delete_banned_accounts: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum CodexSwitcherAdBlock {
    Text { text: String },
    Markdown { markdown: String },
    Image {
        src: String,
        #[serde(default)]
        alt: Option<String>,
        #[serde(default)]
        href: Option<String>,
    },
    Video {
        src: String,
        #[serde(default)]
        poster: Option<String>,
        #[serde(default)]
        title: Option<String>,
    },
    Button { label: String, href: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexSwitcherRemoteAd {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub blocks: Vec<CodexSwitcherAdBlock>,
}

fn settings_path() -> Result<PathBuf, String> {
    Ok(config::get_shared_dir().join(SETTINGS_FILE_NAME))
}

fn local_ad_path() -> Result<PathBuf, String> {
    Ok(config::get_shared_dir().join(LOCAL_AD_FILE_NAME))
}

fn load_settings() -> Result<CodexSwitcherSettings, String> {
    let path = settings_path()?;
    if !path.exists() {
        return Ok(CodexSwitcherSettings::default());
    }

    let content =
        fs::read_to_string(&path).map_err(|err| format!("读取 Codex Switcher 设置失败: {}", err))?;
    if content.trim().is_empty() {
        return Ok(CodexSwitcherSettings::default());
    }

    serde_json::from_str::<CodexSwitcherSettings>(&content)
        .map(|settings| CodexSwitcherSettings {
            restart_app_after_switch: settings.restart_app_after_switch,
            delete_banned_accounts: settings.delete_banned_accounts,
        })
        .map_err(|err| format!("解析 Codex Switcher 设置失败: {}", err))
}

fn save_settings(settings: &CodexSwitcherSettings) -> Result<(), String> {
    let path = settings_path()?;
    let content = serde_json::to_string_pretty(settings)
        .map_err(|err| format!("序列化 Codex Switcher 设置失败: {}", err))?;
    atomic_write::write_string_atomic(&path, &content)
}

fn is_account_banned_or_disabled(account: &CodexAccount) -> bool {
    if account.requires_reauth {
        return true;
    }

    let message = account
        .quota_error
        .as_ref()
        .map(|error| {
            format!(
                "{} {}",
                error.code.as_deref().unwrap_or_default(),
                error.message
            )
            .to_ascii_lowercase()
        })
        .unwrap_or_default();

    [
        "account_deactivated",
        "account_disabled",
        "account_banned",
        "account_suspended",
        "user_suspended",
        "invalid_grant",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

fn json_string_at<'a>(value: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| value.get(key).and_then(|item| item.as_str()))
        .map(str::trim)
        .filter(|item| !item.is_empty())
}

fn resolve_current_account_id_from_auth_json(accounts: &[CodexAccount]) -> Option<Option<String>> {
    let auth_path = codex_account::get_auth_json_path();
    if !auth_path.exists() {
        return None;
    }

    let content = match fs::read_to_string(&auth_path) {
        Ok(content) => content,
        Err(error) => {
            logger::log_warn(&format!(
                "[CodexSwitcher] 读取当前 auth.json 失败: path={}, error={}",
                auth_path.display(),
                error
            ));
            return Some(None);
        }
    };
    let auth: serde_json::Value = match serde_json::from_str(&content) {
        Ok(value) => value,
        Err(error) => {
            logger::log_warn(&format!(
                "[CodexSwitcher] 解析当前 auth.json 失败: path={}, error={}",
                auth_path.display(),
                error
            ));
            return Some(None);
        }
    };

    let auth_mode = json_string_at(&auth, &["auth_mode", "authMode"])
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    let api_key = auth
        .get("OPENAI_API_KEY")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if auth_mode == "apikey" || api_key.is_some() {
        return Some(api_key.and_then(|current_api_key| {
            accounts
                .iter()
                .find(|account| {
                    account.is_api_key_auth()
                        && account
                            .openai_api_key
                            .as_deref()
                            .map(str::trim)
                            .map(|stored| stored == current_api_key)
                            .unwrap_or(false)
                })
                .map(|account| account.id.clone())
        }));
    }

    let access_token = auth
        .get("tokens")
        .and_then(|tokens| {
            json_string_at(tokens, &["access_token", "accessToken", "session_token", "sessionToken"])
        })?;
    let payload = codex_account::decode_jwt_payload(access_token).ok();
    let token_email = payload.as_ref().and_then(|payload| {
        payload
            .email
            .clone()
            .or_else(|| payload.profile_data.as_ref().and_then(|profile| profile.email.clone()))
    });
    let token_account_id = codex_account::extract_chatgpt_account_id_from_access_token(access_token);
    let token_organization_id =
        codex_account::extract_chatgpt_organization_id_from_access_token(access_token);

    Some(
        accounts
            .iter()
            .find(|account| {
                if account.is_api_key_auth() {
                    return false;
                }
                if account.tokens.access_token == access_token {
                    return true;
                }
                if let Some(email) = token_email.as_deref() {
                    if !account.email.eq_ignore_ascii_case(email) {
                        return false;
                    }
                }
                if let Some(account_id) = token_account_id.as_deref() {
                    if account.account_id.as_deref() != Some(account_id) {
                        return false;
                    }
                }
                if let Some(organization_id) = token_organization_id.as_deref() {
                    if account.organization_id.as_deref() != Some(organization_id) {
                        return false;
                    }
                }
                token_email.is_some()
                    || token_account_id.is_some()
                    || token_organization_id.is_some()
            })
            .map(|account| account.id.clone()),
    )
}

fn resolve_current_account_id(accounts: &[CodexAccount]) -> Option<String> {
    if let Ok(Some(account)) =
        codex_account::sync_current_official_account_from_dir(&codex_account::get_codex_home())
    {
        if accounts.iter().any(|item| item.id == account.id) {
            return Some(account.id);
        }
    }

    match resolve_current_account_id_from_auth_json(accounts) {
        Some(value) => value,
        None => codex_account::load_account_index()
            .current_account_id
            .filter(|id| accounts.iter().any(|account| account.id == *id)),
    }
}

fn validate_account_id_shape(account_id: &str) -> Result<(), String> {
    if account_id.is_empty() {
        return Err("账号 ID 不能为空".to_string());
    }
    if account_id.len() > 128 {
        return Err("账号 ID 过长".to_string());
    }
    if account_id == "." || account_id == ".." || account_id.starts_with('~') {
        return Err("账号 ID 非法".to_string());
    }
    if !account_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err("账号 ID 只能包含字母、数字、下划线和中划线".to_string());
    }
    Ok(())
}

fn require_indexed_account_id(account_id: &str) -> Result<String, String> {
    validate_account_id_shape(account_id)?;
    let index = codex_account::load_account_index();
    if index.accounts.iter().any(|account| account.id == account_id) {
        Ok(account_id.to_string())
    } else {
        Err(format!("账号不存在或未登记在索引中: {}", account_id))
    }
}

fn safe_remove_non_current_account(account_id: &str) -> Result<(), String> {
    let account_id = require_indexed_account_id(account_id)?;
    let accounts = codex_account::list_accounts_checked()?;
    let current_account_id = resolve_current_account_id(&accounts);
    if current_account_id.as_deref() == Some(account_id.as_str()) {
        return Err("当前正在使用的账号不能在此处删除。请先手动切换到其它账号，再删除该账号。".to_string());
    }
    codex_account::remove_account(&account_id)
}

fn decorate_account(account: CodexAccount, current_account_id: Option<&str>) -> CodexSwitcherAccount {
    let unavailable = is_account_banned_or_disabled(&account);
    let is_current = current_account_id
        .map(|current| current == account.id)
        .unwrap_or(false);
    let is_api_key_auth = account.is_api_key_auth();
    CodexSwitcherAccount {
        id: account.id,
        email: account.email,
        auth_mode: account.auth_mode,
        api_base_url: account.api_base_url,
        api_provider_mode: account.api_provider_mode,
        api_provider_id: account.api_provider_id,
        api_provider_name: account.api_provider_name,
        bound_oauth_account_id: account.bound_oauth_account_id,
        user_id: account.user_id,
        plan_type: account.plan_type,
        subscription_active_until: account.subscription_active_until,
        auth_file_plan_type: account.auth_file_plan_type,
        account_id: account.account_id,
        organization_id: account.organization_id,
        account_name: account.account_name,
        account_structure: account.account_structure,
        requires_reauth: account.requires_reauth,
        reauth_reason: account.reauth_reason,
        quota: account.quota,
        quota_error: account.quota_error,
        usage_updated_at: account.usage_updated_at,
        created_at: account.created_at,
        last_used: account.last_used,
        is_api_key_auth,
        is_current,
        banned: unavailable,
        disabled: unavailable,
    }
}

fn load_decorated_account(
    account_id: &str,
    current_account_id: Option<&str>,
) -> Result<CodexSwitcherAccount, String> {
    let account_id = require_indexed_account_id(account_id)?;
    let account =
        codex_account::load_account(&account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    Ok(decorate_account(account, current_account_id))
}

fn validate_single_access_token(raw: &str) -> Result<String, String> {
    let token = raw.trim();
    if token.is_empty() {
        return Err("access token 不能为空".to_string());
    }
    if token.lines().count() > 1 || token.starts_with('{') || token.starts_with('[') {
        return Err("此入口仅支持导入单个 Codex access token，不支持批量 JSON 导入".to_string());
    }
    let payload = codex_account::decode_jwt_payload(token)
        .map_err(|_| "access token 不是有效的 JWT，请确认复制的是你自己账号的 Codex token".to_string())?;
    if payload
        .exp
        .map(|exp| exp <= chrono::Utc::now().timestamp())
        .unwrap_or(true)
    {
        return Err("access token 已过期或缺少过期时间，请重新登录后导入。".to_string());
    }
    let has_openai_identity = payload.auth_data.is_some()
        || payload.profile_data.is_some()
        || payload
            .email
            .as_deref()
            .map(|email| email.contains('@'))
            .unwrap_or(false);
    if !has_openai_identity {
        return Err("access token 缺少可识别的 OpenAI/Codex 身份信息，已拒绝导入。".to_string());
    }
    Ok(token.to_string())
}

async fn restart_codex_app_if_requested(app: &AppHandle, restart_app: bool) {
    if !restart_app {
        return;
    }

    match process::restart_codex_default(&[], 20) {
        Ok(pid) => {
            logger::log_info(&format!(
                "[CodexSwitcher] Codex App 已按用户请求重启: pid={}",
                pid
            ));
        }
        Err(error) => {
            logger::log_warn(&format!(
                "[CodexSwitcher] Codex App 重启失败，账号切换结果保留: {}",
                error
            ));
            if error.starts_with("APP_PATH_NOT_FOUND:") {
                let _ = app.emit(
                    "app:path_missing",
                    serde_json::json!({ "app": "codex", "retry": { "kind": "default" } }),
                );
            }
        }
    }
}

async fn switch_account_inner(
    app: &AppHandle,
    account_id: String,
    restart_app: bool,
) -> Result<CodexSwitcherAccount, String> {
    let account_id = require_indexed_account_id(&account_id)?;
    let account = codex::switch_codex_account(app.clone(), account_id.clone()).await?;
    restart_codex_app_if_requested(app, restart_app).await;
    let _ = tray::update_tray_menu(app);
    Ok(decorate_account(account, Some(&account_id)))
}

fn should_delete_after_quota_error(error: &str, settings: &CodexSwitcherSettings) -> bool {
    if !settings.delete_banned_accounts {
        return false;
    }

    let lower = error.to_ascii_lowercase();
    [
        "account_deactivated",
        "account_disabled",
        "account_banned",
        "account_suspended",
        "user_suspended",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn parse_ad_from_content(content: &str) -> Result<Option<CodexSwitcherRemoteAd>, String> {
    if content.trim().is_empty() {
        return Ok(None);
    }
    if content.len() > MAX_AD_CONFIG_BYTES {
        return Err("广告配置过大，已拒绝加载".to_string());
    }

    let ad: CodexSwitcherRemoteAd =
        serde_json::from_str(content).map_err(|err| format!("解析广告配置失败: {}", err))?;
    let blocks: Vec<CodexSwitcherAdBlock> = ad
        .blocks
        .into_iter()
        .filter_map(sanitize_ad_block)
        .collect();
    if blocks.is_empty() {
        return Ok(None);
    }

    Ok(Some(CodexSwitcherRemoteAd { blocks, ..ad }))
}

fn is_safe_https_url(raw: &str) -> bool {
    url::Url::parse(raw)
        .map(|url| url.scheme() == "https")
        .unwrap_or(false)
}

fn sanitize_ad_block(block: CodexSwitcherAdBlock) -> Option<CodexSwitcherAdBlock> {
    match block {
        CodexSwitcherAdBlock::Text { text } => {
            if text.trim().is_empty() {
                None
            } else {
                Some(CodexSwitcherAdBlock::Text { text })
            }
        }
        CodexSwitcherAdBlock::Markdown { markdown } => {
            if markdown.trim().is_empty() {
                None
            } else {
                Some(CodexSwitcherAdBlock::Markdown { markdown })
            }
        }
        CodexSwitcherAdBlock::Image { src, alt, href } => {
            if !is_safe_https_url(&src) {
                return None;
            }
            let href = href.filter(|value| is_safe_https_url(value));
            Some(CodexSwitcherAdBlock::Image { src, alt, href })
        }
        CodexSwitcherAdBlock::Video { src, poster, title } => {
            if !is_safe_https_url(&src) {
                return None;
            }
            let poster = poster.filter(|value| is_safe_https_url(value));
            Some(CodexSwitcherAdBlock::Video { src, poster, title })
        }
        CodexSwitcherAdBlock::Button { label, href } => {
            if label.trim().is_empty() || !is_safe_https_url(&href) {
                None
            } else {
                Some(CodexSwitcherAdBlock::Button { label, href })
            }
        }
    }
}

#[tauri::command]
pub fn codex_switcher_list_accounts() -> Result<CodexSwitcherListResponse, String> {
    let accounts = codex_account::list_accounts_checked()?;
    let current_account_id = resolve_current_account_id(&accounts);
    let accounts = accounts
        .into_iter()
        .map(|account| decorate_account(account, current_account_id.as_deref()))
        .collect();
    Ok(CodexSwitcherListResponse {
        accounts,
        current_account_id,
    })
}

#[tauri::command]
pub async fn codex_switcher_import_access_token(
    access_token: String,
) -> Result<CodexSwitcherAccount, String> {
    let token = validate_single_access_token(&access_token)?;
    let previous_index = codex_account::load_account_index();
    let previous_accounts: HashMap<String, CodexAccount> = codex_account::list_accounts()
        .into_iter()
        .map(|account| (account.id.clone(), account))
        .collect();
    let account = codex_account::import_access_token(&token)?;
    if let Err(error) = codex_quota::refresh_account_quota(&account.id).await {
        if let Some(previous_account) = previous_accounts.get(&account.id) {
            let _ = codex_account::save_account(previous_account);
        } else {
            let _ = codex_account::remove_account(&account.id);
        }
        let _ = codex_account::save_account_index(&previous_index);
        logger::log_warn(&format!(
            "[CodexSwitcher] 导入后刷新额度失败: account_id={}, error={}",
            account.id, error
        ));
        return Err(format!(
            "导入的 access token 未通过远程额度校验，已移除本机保存记录。原始错误: {}",
            error
        ));
    }
    let accounts = codex_account::list_accounts_checked().unwrap_or_default();
    let current_account_id = resolve_current_account_id(&accounts);
    load_decorated_account(&account.id, current_account_id.as_deref())
}

#[tauri::command]
pub async fn codex_switcher_redeem_activation_code(
    _activation_code: String,
) -> Result<CodexSwitcherAccount, String> {
    Err("激活码兑换暂未启用。当前版本仅支持导入你自己合法持有的 Codex access token。".to_string())
}

#[tauri::command]
pub async fn codex_switcher_switch_account(
    app: AppHandle,
    account_id: String,
    restart_app: bool,
) -> Result<CodexSwitcherAccount, String> {
    switch_account_inner(&app, account_id, restart_app).await
}

#[tauri::command]
pub async fn codex_switcher_switch_best_account(
    _app: AppHandle,
    _restart_app: bool,
) -> Result<CodexSwitcherAccount, String> {
    Err("已禁用自动按额度选择账号。请在账号列表中手动选择要切换的账号。".to_string())
}

#[tauri::command]
pub async fn codex_switcher_refresh_account_quota(
    account_id: String,
) -> Result<CodexQuota, String> {
    let account_id = require_indexed_account_id(&account_id)?;
    match codex_quota::refresh_account_quota(&account_id).await {
        Ok(quota) => Ok(quota),
        Err(error) => {
            let settings = load_settings().unwrap_or_default();
            if should_delete_after_quota_error(&error, &settings) {
                logger::log_warn(&format!(
                    "[CodexSwitcher] 额度错误命中封禁自动删除规则: account_id={}, error={}",
                    account_id, error
                ));
                let _ = safe_remove_non_current_account(&account_id);
            }
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn codex_switcher_refresh_all_quotas() -> Result<i32, String> {
    let settings = load_settings().unwrap_or_default();
    let results = codex_quota::refresh_all_quotas().await?;
    let mut success_count = 0;

    for (account_id, result) in results {
        match result {
            Ok(_) => success_count += 1,
            Err(error) if should_delete_after_quota_error(&error, &settings) => {
                logger::log_warn(&format!(
                    "[CodexSwitcher] 批量刷新命中封禁自动删除规则: account_id={}, error={}",
                    account_id, error
                ));
                let _ = safe_remove_non_current_account(&account_id);
            }
            Err(_) => {}
        }
    }

    Ok(success_count)
}

#[tauri::command]
pub fn codex_switcher_delete_account(account_id: String) -> Result<(), String> {
    safe_remove_non_current_account(&account_id)
}

#[tauri::command]
pub fn codex_switcher_get_settings() -> Result<CodexSwitcherSettings, String> {
    load_settings()
}

#[tauri::command]
pub fn codex_switcher_update_settings(
    settings: CodexSwitcherSettings,
) -> Result<CodexSwitcherSettings, String> {
    save_settings(&settings)?;
    Ok(settings)
}

#[tauri::command]
pub async fn codex_switcher_fetch_remote_ad() -> Result<Option<CodexSwitcherRemoteAd>, String> {
    let local_path = local_ad_path()?;
    if local_path.exists() {
        let content = fs::read_to_string(&local_path)
            .map_err(|err| format!("读取本地广告配置失败: {}", err))?;
        return parse_ad_from_content(&content);
    }

    let url = match std::env::var(REMOTE_AD_ENV) {
        Ok(value) if is_safe_https_url(value.trim()) => value,
        _ => return Ok(None),
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|err| format!("创建广告请求客户端失败: {}", err))?;
    let content = client
        .get(url)
        .send()
        .await
        .map_err(|err| format!("拉取广告配置失败: {}", err))?
        .bytes()
        .await
        .map_err(|err| format!("读取广告配置响应失败: {}", err))?;
    if content.len() > MAX_AD_CONFIG_BYTES {
        return Err("广告配置响应过大，已拒绝加载".to_string());
    }
    let content = std::str::from_utf8(&content)
        .map_err(|err| format!("广告配置不是有效 UTF-8: {}", err))?;
    parse_ad_from_content(content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::codex::{CodexAppSpeed, CodexTokens};

    fn sample_account() -> CodexAccount {
        CodexAccount {
            id: "codex_test_account".to_string(),
            email: "owner@example.com".to_string(),
            auth_mode: CodexAuthMode::OAuth,
            openai_api_key: Some("sk-sensitive-api-key".to_string()),
            api_base_url: Some("https://api.openai.com".to_string()),
            api_provider_mode: CodexApiProviderMode::OpenaiBuiltin,
            api_provider_id: None,
            api_provider_name: None,
            bound_oauth_account_id: None,
            user_id: Some("user-123".to_string()),
            plan_type: Some("plus".to_string()),
            subscription_active_until: None,
            auth_file_plan_type: None,
            account_id: Some("account-123".to_string()),
            organization_id: Some("org-123".to_string()),
            account_name: Some("Owner Account".to_string()),
            account_structure: None,
            account_note: Some("private-note".to_string()),
            app_speed: CodexAppSpeed::Standard,
            tokens: CodexTokens {
                id_token: "sensitive-id-token".to_string(),
                access_token: "sensitive-access-token".to_string(),
                refresh_token: Some("sensitive-refresh-token".to_string()),
            },
            token_generation: 1,
            token_updated_at: Some(1_700_000_000),
            token_source_mode: "managed".to_string(),
            requires_reauth: false,
            reauth_reason: None,
            quota: None,
            quota_error: None,
            usage_updated_at: None,
            subscription_query_last_attempt_at: None,
            subscription_query_last_success_at: None,
            subscription_query_next_retry_at: None,
            subscription_query_last_error: None,
            tags: None,
            created_at: 1_700_000_000,
            last_used: 1_700_000_001,
        }
    }

    #[test]
    fn decorated_account_serialization_redacts_secrets() {
        let decorated = decorate_account(sample_account(), Some("codex_test_account"));
        let value = serde_json::to_value(&decorated).expect("serialize switcher account");
        let object = value.as_object().expect("account object");

        for key in ["tokens", "openai_api_key", "account_note"] {
            assert!(
                !object.contains_key(key),
                "switcher account must not expose sensitive field {key}"
            );
        }

        let serialized = value.to_string();
        for secret in [
            "sensitive-id-token",
            "sensitive-access-token",
            "sensitive-refresh-token",
            "sk-sensitive-api-key",
            "private-note",
        ] {
            assert!(
                !serialized.contains(secret),
                "switcher account leaked sensitive value {secret}"
            );
        }
    }

    #[test]
    fn account_id_validation_rejects_path_traversal_shapes() {
        for invalid in [
            "",
            ".",
            "..",
            "~/.codex",
            "../auth",
            "..\\auth",
            "C:\\Users\\PC\\secret",
            "\\\\server\\share",
            "codex/secret",
            "codex.secret",
            "codex secret",
        ] {
            assert!(
                validate_account_id_shape(invalid).is_err(),
                "invalid account id was accepted: {invalid}"
            );
        }

        for valid in ["codex_abcdef123456", "api-key_123", "account-01"] {
            assert!(
                validate_account_id_shape(valid).is_ok(),
                "valid account id was rejected: {valid}"
            );
        }
    }

    #[test]
    fn ad_config_rejects_unsafe_urls_and_oversized_payloads() {
        let content = r#"{
            "title": "safe",
            "blocks": [
                { "type": "text", "text": "hello" },
                { "type": "image", "src": "http://example.com/image.png" },
                { "type": "button", "label": "bad", "href": "javascript:alert(1)" },
                { "type": "video", "src": "https://example.com/video.mp4", "poster": "file:///tmp/poster.png" }
            ]
        }"#;

        let ad = parse_ad_from_content(content)
            .expect("parse ad")
            .expect("non-empty safe ad");
        assert_eq!(ad.blocks.len(), 2);
        assert!(matches!(ad.blocks[0], CodexSwitcherAdBlock::Text { .. }));
        match &ad.blocks[1] {
            CodexSwitcherAdBlock::Video { src, poster, .. } => {
                assert_eq!(src, "https://example.com/video.mp4");
                assert!(poster.is_none());
            }
            other => panic!("expected sanitized video block, got {other:?}"),
        }

        let oversized = "x".repeat(MAX_AD_CONFIG_BYTES + 1);
        assert!(parse_ad_from_content(&oversized).is_err());
    }
}
