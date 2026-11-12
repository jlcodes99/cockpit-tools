use crate::models::github_copilot::{
    GitHubCopilotAccount, GitHubCopilotAccountIndex, GitHubCopilotOAuthCompletePayload,
};
use crate::modules::{account, github_copilot_oauth, logger};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

const ACCOUNTS_INDEX_FILE: &str = "github_copilot_accounts.json";
const ACCOUNTS_DIR: &str = "github_copilot_accounts";
static GHCP_ACCOUNT_INDEX_LOCK: std::sync::LazyLock<Mutex<()>> =
    std::sync::LazyLock::new(|| Mutex::new(()));
static GHCP_QUOTA_ALERT_LAST_SENT: std::sync::LazyLock<Mutex<HashMap<String, i64>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
const GHCP_QUOTA_ALERT_COOLDOWN_SECONDS: i64 = 300;

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

fn get_data_dir() -> Result<PathBuf, String> {
    account::get_data_dir()
}

fn get_accounts_dir() -> Result<PathBuf, String> {
    let base = get_data_dir()?;
    let dir = base.join(ACCOUNTS_DIR);
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("创建 GitHub Copilot 账号目录失败: {}", e))?;
    }
    Ok(dir)
}

fn get_accounts_index_path() -> Result<PathBuf, String> {
    Ok(get_data_dir()?.join(ACCOUNTS_INDEX_FILE))
}

pub fn accounts_index_path_string() -> Result<String, String> {
    Ok(crate::modules::provider_account_index::storage_path::<GitHubCopilotAccountIndex>(&get_accounts_index_path()?)?
        .to_string_lossy().into_owned())
}

/// Load a single account by ID (public wrapper)
pub fn load_account(account_id: &str) -> Option<GitHubCopilotAccount> {
    load_account_file(account_id)
}

fn load_account_file(account_id: &str) -> Option<GitHubCopilotAccount> {
    let account_path = get_accounts_dir()
        .ok()
        .map(|dir| dir.join(format!("{}.json", account_id)))?;
    if !account_path.exists() {
        return None;
    }
    let account: GitHubCopilotAccount = crate::modules::secure_account_storage::read_account_file(&account_path).ok()?;
    (account.id == account_id).then_some(account)
}

fn save_account_file(account: &GitHubCopilotAccount) -> Result<(), String> {
    let path = get_accounts_dir()?.join(format!("{}.json", account.id));
    crate::modules::secure_account_storage::write_account_file(&path, "github_copilot", account)
        .map_err(|e| format!("保存账号失败: {}", e))
}

fn persist_quota_query_error(account_id: &str, message: &str) {
    let Some(mut account) = load_account_file(account_id) else {
        return;
    };
    account.quota_query_last_error = Some(message.to_string());
    account.quota_query_last_error_at = Some(chrono::Utc::now().timestamp_millis());
    let _ = upsert_account_record(account);
}

fn load_account_index() -> GitHubCopilotAccountIndex {
    load_account_index_checked().unwrap_or_else(|error| {
        logger::log_warn(&format!("[GitHubCopilot Account] {error}"));
        GitHubCopilotAccountIndex::new()
    })
}

fn load_account_index_checked() -> Result<GitHubCopilotAccountIndex, String> {
    crate::modules::provider_account_index::checked(
        &get_accounts_index_path()?,
        crate::modules::account::schedule_index_store,
        repair_account_index_from_details,
        |index| index.accounts.is_empty(),
    )
}

fn save_account_index(index: &GitHubCopilotAccountIndex) -> Result<(), String> {
    crate::modules::provider_account_index::save(&get_accounts_index_path()?, index)
}

fn repair_account_index_from_details(reason: &str) -> Option<GitHubCopilotAccountIndex> {
    let accounts_dir = get_accounts_dir().ok()?;
    let mut accounts = crate::modules::account_index_repair::load_accounts_from_details(
        &accounts_dir,
        |account_id| load_account_file(account_id),
    )
    .ok()?;

    if accounts.is_empty() {
        return None;
    }

    crate::modules::account_index_repair::sort_accounts_by_recency(
        &mut accounts,
        |account| account.last_used,
        |account| account.created_at,
        |account| account.id.as_str(),
    );

    let mut index = GitHubCopilotAccountIndex::new();
    index.accounts = accounts.iter().map(|account| account.summary()).collect();

    if let Err(error) = save_account_index(&index) {
        logger::log_warn(&format!("[GitHubCopilot Account] 保存恢复索引失败: reason={reason}, error={error}"));
        return None;
    }
    logger::log_info(&format!(
        "[GitHubCopilot Account] 已恢复账号索引: reason={reason}, accounts={}",
        index.accounts.len()
    ));
    Some(index)
}

fn refresh_summary(index: &mut GitHubCopilotAccountIndex, account: &GitHubCopilotAccount) {
    if let Some(summary) = index.accounts.iter_mut().find(|item| item.id == account.id) {
        *summary = account.summary();
        return;
    }
    index.accounts.push(account.summary());
}

fn upsert_account_record(account: GitHubCopilotAccount) -> Result<GitHubCopilotAccount, String> {
    let _lock = GHCP_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 GitHub Copilot 账号锁失败".to_string())?;
    let mut index = load_account_index();
    save_account_file(&account)?;
    refresh_summary(&mut index, &account);
    save_account_index(&index)?;
    Ok(account)
}

pub fn list_accounts() -> Vec<GitHubCopilotAccount> {
    let index = load_account_index();
    index
        .accounts
        .iter()
        .filter_map(|summary| load_account_file(&summary.id))
        .collect()
}

pub fn list_accounts_checked() -> Result<Vec<GitHubCopilotAccount>, String> {
    let index = load_account_index_checked()?;
    let accounts: Vec<_> = index
        .accounts
        .iter()
        .filter_map(|summary| load_account_file(&summary.id))
        .collect();
    if accounts.is_empty() && !index.accounts.is_empty() {
        return Err("GitHub Copilot 账号索引存在，但详情暂时无法读取".into());
    }
    Ok(accounts)
}

pub fn list_accounts_for_cli() -> Result<(Vec<GitHubCopilotAccount>, Option<String>), String> {
    let store = crate::modules::provider_account_index::store::<GitHubCopilotAccountIndex>(&get_accounts_index_path()?)?;
    if !store.has_legacy_source()? {
        return list_accounts_checked().map(|accounts| (accounts, None));
    }
    crate::modules::account::list_after_cli_index_batches(|| store.migrate_batch(), list_accounts_checked)
}

pub fn upsert_account(
    payload: GitHubCopilotOAuthCompletePayload,
) -> Result<GitHubCopilotAccount, String> {
    let _lock = GHCP_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 GitHub Copilot 账号锁失败".to_string())?;
    let now = now_ts();
    let mut index = load_account_index();
    let generated_id = format!(
        "ghcp_{:x}",
        md5::compute(format!("{}:{}", payload.github_login, payload.github_id))
    );
    // 以 github_id 唯一标识区分账号，同一邮箱不同 GitHub 账号不合并
    let account_id = index
        .accounts
        .iter()
        .filter_map(|item| load_account_file(&item.id).map(|acc| (item.id.clone(), acc)))
        .find(|(_, acc)| acc.github_id == payload.github_id)
        .map(|(id, _)| id)
        .unwrap_or(generated_id);

    let existing = load_account_file(&account_id);
    let tags = existing.as_ref().and_then(|acc| acc.tags.clone());
    let created_at = existing.as_ref().map(|acc| acc.created_at).unwrap_or(now);

    let mut account = existing.unwrap_or(GitHubCopilotAccount {
        id: account_id.clone(),
        github_login: payload.github_login.clone(),
        github_id: payload.github_id,
        github_name: payload.github_name.clone(),
        github_email: payload.github_email.clone(),
        tags,
        github_access_token: payload.github_access_token.clone(),
        github_token_type: payload.github_token_type.clone(),
        github_scope: payload.github_scope.clone(),
        copilot_token: payload.copilot_token.clone(),
        copilot_plan: payload.copilot_plan.clone(),
        copilot_chat_enabled: payload.copilot_chat_enabled,
        copilot_expires_at: payload.copilot_expires_at,
        copilot_refresh_in: payload.copilot_refresh_in,
        copilot_token_based_billing: None,
        copilot_quota_snapshots: payload.copilot_quota_snapshots.clone(),
        copilot_quota_reset_date: payload.copilot_quota_reset_date.clone(),
        copilot_limited_user_quotas: payload.copilot_limited_user_quotas.clone(),
        copilot_limited_user_reset_date: payload.copilot_limited_user_reset_date,
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: None,
        created_at,
        last_used: now,
    });

    account.github_login = payload.github_login;
    account.github_id = payload.github_id;
    account.github_name = payload.github_name;
    account.github_email = payload.github_email;
    account.github_access_token = payload.github_access_token;
    account.github_token_type = payload.github_token_type;
    account.github_scope = payload.github_scope;
    account.copilot_token = payload.copilot_token;
    account.copilot_plan = payload.copilot_plan;
    account.copilot_chat_enabled = payload.copilot_chat_enabled;
    account.copilot_expires_at = payload.copilot_expires_at;
    account.copilot_refresh_in = payload.copilot_refresh_in;
    account.copilot_quota_snapshots = payload.copilot_quota_snapshots;
    account.copilot_quota_reset_date = payload.copilot_quota_reset_date;
    account.copilot_limited_user_quotas = payload.copilot_limited_user_quotas;
    account.copilot_limited_user_reset_date = payload.copilot_limited_user_reset_date;
    account.quota_query_last_error = None;
    account.quota_query_last_error_at = None;
    account.created_at = created_at;
    account.last_used = now;

    save_account_file(&account)?;
    refresh_summary(&mut index, &account);
    save_account_index(&index)?;

    logger::log_info(&format!(
        "GitHub Copilot 账号已保存: id={}, login={}",
        account.id, account.github_login
    ));
    Ok(account)
}

async fn refresh_account_token_once(account_id: &str) -> Result<GitHubCopilotAccount, String> {
    let mut account = load_account_file(account_id).ok_or_else(|| "账号不存在".to_string())?;
    let bundle = github_copilot_oauth::refresh_copilot_token(&account.github_access_token).await?;

    account.copilot_token = bundle.token;
    account.copilot_plan = bundle.plan;
    account.copilot_chat_enabled = bundle.chat_enabled;
    account.copilot_expires_at = bundle.expires_at;
    account.copilot_refresh_in = bundle.refresh_in;
    account.copilot_quota_snapshots = bundle.quota_snapshots;
    account.copilot_quota_reset_date = bundle.quota_reset_date;
    account.copilot_limited_user_quotas = bundle.limited_user_quotas;
    account.copilot_limited_user_reset_date = bundle.limited_user_reset_date;
    account.quota_query_last_error = None;
    account.quota_query_last_error_at = None;
    let refreshed_at = now_ts();
    account.usage_updated_at = Some(refreshed_at);
    account.last_used = refreshed_at;

    let updated = account.clone();
    upsert_account_record(account)?;
    Ok(updated)
}

pub async fn refresh_account_token(account_id: &str) -> Result<GitHubCopilotAccount, String> {
    let result = refresh_account_token_once(account_id).await;
    if let Err(err) = &result {
        persist_quota_query_error(account_id, err);
    }
    result
}

pub async fn refresh_all_tokens(
) -> Result<Vec<(String, Result<GitHubCopilotAccount, String>)>, String> {
    use futures::future::join_all;
    use std::sync::Arc;
    use tokio::sync::Semaphore;

    const MAX_CONCURRENT: usize = 5;
    let accounts = list_accounts();
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT));
    let tasks: Vec<_> = accounts
        .into_iter()
        .map(|account| {
            let id = account.id;
            let semaphore = semaphore.clone();
            async move {
                let _permit = semaphore
                    .acquire_owned()
                    .await
                    .map_err(|e| format!("获取 GitHub Copilot 刷新并发许可失败: {}", e))?;
                let res = refresh_account_token(&id).await;
                Ok::<(String, Result<GitHubCopilotAccount, String>), String>((id, res))
            }
        })
        .collect();

    let mut results = Vec::with_capacity(tasks.len());
    for task in join_all(tasks).await {
        match task {
            Ok(item) => results.push(item),
            Err(err) => return Err(err),
        }
    }

    Ok(results)
}

pub fn remove_account(account_id: &str) -> Result<(), String> {
    let _lock = GHCP_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "获取 GitHub Copilot 账号锁失败".to_string())?;
    let path = get_accounts_dir()?.join(format!("{}.json", account_id));
    let _file_lock = crate::modules::account_file_crypto::lock_account_file_for_write(&path)?;
    let mut index = load_account_index();
    index.accounts.retain(|item| item.id != account_id);
    save_account_index(&index)?;
    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("删除账号文件失败: {error}")),
    }
    Ok(())
}

pub fn remove_accounts(account_ids: &[String]) -> Result<(), String> {
    for id in account_ids {
        remove_account(id)?;
    }
    Ok(())
}

pub fn update_account_tags(
    account_id: &str,
    tags: Vec<String>,
) -> Result<GitHubCopilotAccount, String> {
    let mut account = load_account_file(account_id).ok_or_else(|| "账号不存在".to_string())?;
    account.tags = Some(tags);
    account.last_used = now_ts();
    let updated = account.clone();
    upsert_account_record(account)?;
    Ok(updated)
}

pub fn import_from_json(json_content: &str) -> Result<Vec<GitHubCopilotAccount>, String> {
    if let Ok(account) = serde_json::from_str::<GitHubCopilotAccount>(json_content) {
        let saved = upsert_account_record(account)?;
        return Ok(vec![saved]);
    }

    if let Ok(accounts) = serde_json::from_str::<Vec<GitHubCopilotAccount>>(json_content) {
        let mut result = Vec::new();
        for account in accounts {
            let saved = upsert_account_record(account)?;
            result.push(saved);
        }
        return Ok(result);
    }

    Err("无法解析 JSON 内容".to_string())
}

pub fn export_accounts(account_ids: &[String]) -> Result<String, String> {
    let accounts: Vec<GitHubCopilotAccount> = account_ids
        .iter()
        .filter_map(|id| load_account_file(id))
        .collect();
    serde_json::to_string_pretty(&accounts).map_err(|e| format!("序列化失败: {}", e))
}

fn normalize_quota_alert_threshold(raw: i32) -> i32 {
    raw.clamp(0, 100)
}

fn clamp_percent(value: f64) -> i32 {
    value.round().clamp(0.0, 100.0) as i32
}

fn parse_token_map(token: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let prefix = token.split(':').next().unwrap_or(token);
    for item in prefix.split(';') {
        let mut parts = item.splitn(2, '=');
        let key = parts.next().unwrap_or("").trim();
        if key.is_empty() {
            continue;
        }
        let value = parts.next().unwrap_or("").trim();
        map.insert(key.to_string(), value.to_string());
    }
    map
}

fn parse_token_number(map: &HashMap<String, String>, key: &str) -> Option<f64> {
    map.get(key)
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite())
}

fn get_json_number(value: &serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Number(num) => num.as_f64(),
        serde_json::Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|value| value.is_finite())
}

fn calc_remaining_percent(remaining: f64, total: f64) -> Option<i32> {
    if total <= 0.0 {
        return None;
    }
    Some(clamp_percent((remaining.max(0.0) / total) * 100.0))
}

fn extract_limited_metrics(account: &GitHubCopilotAccount) -> Vec<(String, i32)> {
    let Some(limited) = account
        .copilot_limited_user_quotas
        .as_ref()
        .and_then(|value| value.as_object())
    else {
        return Vec::new();
    };

    let token_map = parse_token_map(&account.copilot_token);
    let mut metrics = Vec::new();

    if let Some(remaining_completions) = limited.get("completions").and_then(get_json_number) {
        let total_completions =
            parse_token_number(&token_map, "cq").unwrap_or(remaining_completions);
        if let Some(percent) = calc_remaining_percent(remaining_completions, total_completions) {
            metrics.push(("Inline Suggestions".to_string(), percent));
        }
    }

    if let Some(remaining_chat) = limited.get("chat").and_then(get_json_number) {
        let total_chat = parse_token_number(&token_map, "tq").unwrap_or(remaining_chat);
        if let Some(percent) = calc_remaining_percent(remaining_chat, total_chat) {
            metrics.push(("Chat Messages".to_string(), percent));
        }
    }

    metrics
}

fn extract_premium_metric(account: &GitHubCopilotAccount) -> Option<(String, i32)> {
    let snapshots = account
        .copilot_quota_snapshots
        .as_ref()
        .and_then(|value| value.as_object())?;

    let premium = snapshots
        .get("premium_models")
        .or_else(|| snapshots.get("premium_interactions"))
        .and_then(|value| value.as_object())?;

    if premium.get("unlimited").and_then(|value| value.as_bool()) == Some(true) {
        return Some(("Premium Interactions".to_string(), 100));
    }

    let entitlement = premium.get("entitlement").and_then(get_json_number);
    if entitlement.map(|value| value < 0.0).unwrap_or(false) {
        return Some(("Premium Interactions".to_string(), 100));
    }
    if entitlement.map(|value| value <= 0.0).unwrap_or_else(|| {
        premium.get("has_quota").and_then(|value| value.as_bool()) == Some(false)
    }) {
        return None;
    }

    let percent_remaining = premium
        .get("percent_remaining")
        .and_then(get_json_number)
        .map(clamp_percent)?;

    Some(("Premium Interactions".to_string(), percent_remaining))
}

pub(crate) fn extract_quota_metrics(account: &GitHubCopilotAccount) -> Vec<(String, i32)> {
    let mut metrics = extract_limited_metrics(account);
    if let Some(premium) = extract_premium_metric(account) {
        metrics.push(premium);
    }
    metrics
}

fn average_quota_percentage(metrics: &[(String, i32)]) -> f64 {
    if metrics.is_empty() {
        return 0.0;
    }
    let sum: i32 = metrics.iter().map(|(_, pct)| *pct).sum();
    sum as f64 / metrics.len() as f64
}

pub(crate) fn resolve_current_account_id(accounts: &[GitHubCopilotAccount]) -> Option<String> {
    if let Ok(settings) = crate::modules::github_copilot_instance::load_default_settings() {
        if let Some(bind_id) = settings.bind_account_id {
            let trimmed = bind_id.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }

    accounts
        .iter()
        .max_by_key(|account| account.last_used)
        .map(|account| account.id.clone())
}

fn display_email(account: &GitHubCopilotAccount) -> String {
    account
        .github_email
        .clone()
        .filter(|text| !text.trim().is_empty())
        .unwrap_or_else(|| account.github_login.clone())
}

fn build_quota_alert_cooldown_key(account_id: &str, threshold: i32) -> String {
    format!("github_copilot:{}:{}", account_id, threshold)
}

fn should_emit_quota_alert(cooldown_key: &str, now: i64) -> bool {
    let Ok(mut state) = GHCP_QUOTA_ALERT_LAST_SENT.lock() else {
        return true;
    };

    if let Some(last_sent) = state.get(cooldown_key) {
        if now - *last_sent < GHCP_QUOTA_ALERT_COOLDOWN_SECONDS {
            return false;
        }
    }

    state.insert(cooldown_key.to_string(), now);
    true
}

fn clear_quota_alert_cooldown(account_id: &str, threshold: i32) {
    if let Ok(mut state) = GHCP_QUOTA_ALERT_LAST_SENT.lock() {
        state.remove(&build_quota_alert_cooldown_key(account_id, threshold));
    }
}

fn pick_quota_alert_recommendation(
    accounts: &[GitHubCopilotAccount],
    current_id: &str,
) -> Option<GitHubCopilotAccount> {
    let mut candidates: Vec<GitHubCopilotAccount> = accounts
        .iter()
        .filter(|account| account.id != current_id)
        .filter(|account| !extract_quota_metrics(account).is_empty())
        .cloned()
        .collect();

    if candidates.is_empty() {
        return None;
    }

    candidates.sort_by(|a, b| {
        let avg_a = average_quota_percentage(&extract_quota_metrics(a));
        let avg_b = average_quota_percentage(&extract_quota_metrics(b));
        avg_b
            .partial_cmp(&avg_a)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.last_used.cmp(&b.last_used))
    });

    candidates.into_iter().next()
}

pub fn run_quota_alert_if_needed(
) -> Result<Option<crate::modules::account::QuotaAlertPayload>, String> {
    let cfg = crate::modules::config::get_user_config();
    if !cfg.ghcp_quota_alert_enabled {
        return Ok(None);
    }

    let threshold = normalize_quota_alert_threshold(cfg.ghcp_quota_alert_threshold);
    let accounts = list_accounts();
    let current_id = match resolve_current_account_id(&accounts) {
        Some(id) => id,
        None => return Ok(None),
    };

    let current = match accounts.iter().find(|account| account.id == current_id) {
        Some(account) => account,
        None => return Ok(None),
    };

    let metrics = extract_quota_metrics(current);
    let low_models: Vec<(String, i32)> = metrics
        .into_iter()
        .filter(|(_, pct)| *pct <= threshold)
        .collect();

    if low_models.is_empty() {
        clear_quota_alert_cooldown(&current_id, threshold);
        return Ok(None);
    }

    let now = chrono::Utc::now().timestamp();
    let cooldown_key = build_quota_alert_cooldown_key(&current_id, threshold);
    if !should_emit_quota_alert(&cooldown_key, now) {
        return Ok(None);
    }

    let recommendation = pick_quota_alert_recommendation(&accounts, &current_id);
    let lowest_percentage = low_models.iter().map(|(_, pct)| *pct).min().unwrap_or(0);
    let payload = crate::modules::account::QuotaAlertPayload {
        platform: "github_copilot".to_string(),
        current_account_id: current_id,
        current_email: display_email(current),
        threshold,
        threshold_display: None,
        lowest_percentage,
        low_models: low_models.into_iter().map(|(name, _)| name).collect(),
        recommended_account_id: recommendation.as_ref().map(|account| account.id.clone()),
        recommended_email: recommendation.as_ref().map(display_email),
        triggered_at: now,
    };

    crate::modules::account::dispatch_quota_alert(&payload);
    Ok(Some(payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn account_with_snapshots(snapshots: serde_json::Value) -> GitHubCopilotAccount {
        GitHubCopilotAccount {
            id: "ghcp_test".to_string(),
            github_login: "tester".to_string(),
            github_id: 1,
            github_name: None,
            github_email: None,
            tags: None,
            github_access_token: "github-token".to_string(),
            github_token_type: None,
            github_scope: None,
            copilot_token: "sku=individual;tq=500;cq=2000".to_string(),
            copilot_plan: Some("individual".to_string()),
            copilot_chat_enabled: Some(true),
            copilot_expires_at: None,
            copilot_refresh_in: None,
            copilot_token_based_billing: None,
            copilot_quota_snapshots: Some(snapshots),
            copilot_quota_reset_date: Some("2026-07-01".to_string()),
            copilot_limited_user_quotas: None,
            copilot_limited_user_reset_date: None,
            quota_query_last_error: None,
            quota_query_last_error_at: None,
            usage_updated_at: None,
            created_at: 0,
            last_used: 0,
        }
    }

    #[test]
    fn premium_metric_prefers_premium_models() {
        let account = account_with_snapshots(json!({
            "premium_interactions": {
                "entitlement": 100,
                "percent_remaining": 5,
                "remaining": 5,
                "has_quota": true
            },
            "premium_models": {
                "entitlement": 100,
                "percent_remaining": 70,
                "remaining": 70,
                "has_quota": true
            }
        }));

        let metric = extract_premium_metric(&account).expect("premium metric");

        assert_eq!(metric.1, 70);
    }

    #[test]
    fn premium_metric_ignores_zero_entitlement_without_quota() {
        let account = account_with_snapshots(json!({
            "premium_interactions": {
                "entitlement": 0,
                "percent_remaining": 0,
                "remaining": 0,
                "has_quota": false
            }
        }));

        assert!(extract_premium_metric(&account).is_none());
        assert!(extract_quota_metrics(&account).is_empty());
    }
}
