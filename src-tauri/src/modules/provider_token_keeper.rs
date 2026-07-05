use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::Value;
use tauri::AppHandle;
use tokio::sync::Notify;

use crate::modules::{
    codebuddy_account, codebuddy_cn_account, codex_account, config, cursor_account, gemini_account,
    github_copilot_account, kiro_account, kiro_instance, logger, process, trae_account,
    windsurf_account, windsurf_instance, workbuddy_account,
};

const TOKEN_KEEPER_TICK_SECONDS: u64 = 60;
const TOKEN_KEEPER_STARTUP_DELAY_SECONDS: u64 = 5 * 60;
const TOKEN_KEEPER_MAX_REFRESHES_PER_PLATFORM: usize = 3;
const TOKEN_KEEPER_IDLE_SCAN_SECONDS: i64 = 10 * 60;
const TOKEN_KEEPER_ACTIVE_SCAN_SECONDS: i64 = 60;
const TOKEN_REFRESH_LEAD_SECONDS: i64 = 15 * 60;
const TOKEN_REFRESH_LEAD_MILLISECONDS: i64 = TOKEN_REFRESH_LEAD_SECONDS * 1000;
const REFRESH_FAILURE_BACKOFF_SECONDS: i64 = 15 * 60;
const TRAE_STRICT_CHECK_INTERVAL_SECONDS: i64 = 10 * 60;

static TOKEN_KEEPER_STARTED: AtomicBool = AtomicBool::new(false);
static TOKEN_KEEPER_CONFIG_CHANGED: LazyLock<Notify> = LazyLock::new(Notify::new);
static NEXT_ALLOWED_ATTEMPT_AT: LazyLock<Mutex<HashMap<String, i64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT_PLATFORM_SCAN_AT: LazyLock<Mutex<HashMap<&'static str, i64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT_TRAE_STRICT_CHECK_AT: LazyLock<Mutex<HashMap<String, i64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn ensure_started(app_handle: AppHandle) {
    if TOKEN_KEEPER_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }

    logger::log_info("[TokenKeeper] 后端 OAuth token 保活已启动");
    tauri::async_runtime::spawn(async move {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(TOKEN_KEEPER_STARTUP_DELAY_SECONDS)) => {}
            _ = TOKEN_KEEPER_CONFIG_CHANGED.notified() => {}
        }

        loop {
            run_refresh_cycle(&app_handle).await;
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(TOKEN_KEEPER_TICK_SECONDS)) => {}
                _ = TOKEN_KEEPER_CONFIG_CHANGED.notified() => {}
            }
        }
    });
}

pub fn notify_config_changed(app_handle: AppHandle, enabled: bool) {
    ensure_started(app_handle);
    reset_platform_scan_schedule();
    logger::log_info(&format!(
        "[TokenKeeper] 后台 OAuth token 保活设置已{}，已同步运行时状态",
        if enabled { "启用" } else { "停用" }
    ));
    TOKEN_KEEPER_CONFIG_CHANGED.notify_one();
}

async fn run_refresh_cycle(app_handle: &AppHandle) {
    if !config::get_user_config().token_keeper_enabled {
        return;
    }

    let mut refreshed_any = false;

    refreshed_any |= refresh_platform_if_due("codex", refresh_due_codex_accounts).await;
    refreshed_any |= refresh_platform_if_due("cursor", refresh_due_cursor_accounts).await;
    refreshed_any |= refresh_platform_if_due("gemini", refresh_due_gemini_accounts).await;
    refreshed_any |=
        refresh_platform_if_due("github_copilot", refresh_due_github_copilot_accounts).await;
    refreshed_any |= refresh_platform_if_due("windsurf", refresh_due_windsurf_accounts).await;
    refreshed_any |= refresh_platform_if_due("kiro", refresh_due_kiro_accounts).await;
    refreshed_any |= refresh_platform_if_due("codebuddy", refresh_due_codebuddy_accounts).await;
    refreshed_any |=
        refresh_platform_if_due("codebuddy_cn", refresh_due_codebuddy_cn_accounts).await;
    refreshed_any |= refresh_platform_if_due("workbuddy", refresh_due_workbuddy_accounts).await;
    refreshed_any |= refresh_platform_if_due("trae", refresh_due_trae_accounts).await;

    if refreshed_any {
        let _ = crate::modules::tray::update_tray_menu(app_handle);
    }
}

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

fn now_ts_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn allow_platform_scan(platform: &'static str) -> bool {
    let now = now_ts();
    let Ok(state) = NEXT_PLATFORM_SCAN_AT.lock() else {
        return true;
    };
    state.get(platform).map(|next| *next <= now).unwrap_or(true)
}

fn mark_platform_scan(platform: &'static str, refreshed_any: bool) {
    let next_delay = if refreshed_any {
        TOKEN_KEEPER_ACTIVE_SCAN_SECONDS
    } else {
        TOKEN_KEEPER_IDLE_SCAN_SECONDS
    };
    if let Ok(mut state) = NEXT_PLATFORM_SCAN_AT.lock() {
        state.insert(platform, now_ts() + next_delay);
    }
}

fn reset_platform_scan_schedule() {
    if let Ok(mut state) = NEXT_PLATFORM_SCAN_AT.lock() {
        state.clear();
    }
}

async fn refresh_platform_if_due<F, Fut>(platform: &'static str, refresh: F) -> bool
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = bool>,
{
    if !allow_platform_scan(platform) {
        return false;
    }

    let refreshed_any = refresh().await;
    mark_platform_scan(platform, refreshed_any);
    refreshed_any
}

fn reached_platform_refresh_limit(attempted_refreshes: usize) -> bool {
    attempted_refreshes >= TOKEN_KEEPER_MAX_REFRESHES_PER_PLATFORM
}

fn decode_jwt_exp(token: &str) -> Option<i64> {
    let payload_base64 = token.split('.').nth(1)?;
    let payload_bytes = URL_SAFE_NO_PAD.decode(payload_base64).ok()?;
    let payload: Value = serde_json::from_slice(&payload_bytes).ok()?;
    payload.get("exp").and_then(Value::as_i64)
}

fn jwt_token_expires_soon(token: &str, skew_seconds: i64) -> bool {
    decode_jwt_exp(token)
        .map(|exp| exp <= now_ts() + skew_seconds)
        .unwrap_or(true)
}

fn expires_at_seconds_due(expires_at: Option<i64>) -> bool {
    expires_at
        .map(|value| value <= now_ts() + TOKEN_REFRESH_LEAD_SECONDS)
        .unwrap_or(true)
}

fn expires_at_milliseconds_due(expires_at: Option<i64>) -> bool {
    expires_at
        .map(|value| value <= now_ts_ms() + TOKEN_REFRESH_LEAD_MILLISECONDS)
        .unwrap_or(true)
}

fn allow_attempt(key: &str) -> bool {
    let now = now_ts();
    let Ok(state) = NEXT_ALLOWED_ATTEMPT_AT.lock() else {
        return true;
    };
    state.get(key).map(|next| *next <= now).unwrap_or(true)
}

fn clear_attempt_backoff(key: &str) {
    if let Ok(mut state) = NEXT_ALLOWED_ATTEMPT_AT.lock() {
        state.remove(key);
    }
}

fn mark_attempt_failure(key: &str) {
    if let Ok(mut state) = NEXT_ALLOWED_ATTEMPT_AT.lock() {
        state.insert(key.to_string(), now_ts() + REFRESH_FAILURE_BACKOFF_SECONDS);
    }
}

fn should_run_trae_strict_check(account_id: &str) -> bool {
    let now = now_ts();
    let Ok(state) = NEXT_TRAE_STRICT_CHECK_AT.lock() else {
        return true;
    };
    state
        .get(account_id)
        .map(|next| *next <= now)
        .unwrap_or(true)
}

fn mark_trae_strict_check_done(account_id: &str) {
    if let Ok(mut state) = NEXT_TRAE_STRICT_CHECK_AT.lock() {
        state.insert(
            account_id.to_string(),
            now_ts() + TRAE_STRICT_CHECK_INTERVAL_SECONDS,
        );
    }
}

async fn refresh_due_codex_accounts() -> bool {
    let accounts = match codex_account::list_accounts_checked() {
        Ok(accounts) => accounts,
        Err(err) => {
            logger::log_warn(&format!(
                "[TokenKeeper][Codex] 读取账号列表失败，跳过本轮保活: {}",
                err
            ));
            return false;
        }
    };

    let mut refreshed_any = false;
    let mut attempted_refreshes = 0usize;
    for account in accounts
        .into_iter()
        .filter(|account| !account.is_api_key_auth())
    {
        if reached_platform_refresh_limit(attempted_refreshes) {
            break;
        }
        if !account.requires_reauth && !codex_account::is_managed_auth_refresh_due(&account) {
            continue;
        }

        let key = format!("codex:{}", account.id);
        if !allow_attempt(&key) {
            continue;
        }

        attempted_refreshes += 1;
        match codex_account::keepalive_managed_account(&account.id, "TokenKeeper 授权保活").await
        {
            Ok(updated) => {
                clear_attempt_backoff(&key);
                refreshed_any = true;
                logger::log_info(&format!(
                    "[TokenKeeper][Codex] Token 保活成功: account_id={}, email={}",
                    updated.id, updated.email
                ));
            }
            Err(err) => {
                mark_attempt_failure(&key);
                logger::log_warn(&format!(
                    "[TokenKeeper][Codex] Token 保活失败，进入退避: account_id={}, error={}",
                    account.id, err
                ));
            }
        }
    }

    refreshed_any
}

async fn refresh_due_cursor_accounts() -> bool {
    let accounts = match cursor_account::list_accounts_checked() {
        Ok(accounts) => accounts,
        Err(err) => {
            logger::log_warn(&format!(
                "[TokenKeeper][Cursor] 读取账号列表失败，跳过本轮保活: {}",
                err
            ));
            return false;
        }
    };

    let current_id = cursor_account::resolve_current_account_id(&accounts);
    let mut refreshed_any = false;
    let mut attempted_refreshes = 0usize;

    for account in accounts {
        if reached_platform_refresh_limit(attempted_refreshes) {
            break;
        }
        if !jwt_token_expires_soon(&account.access_token, TOKEN_REFRESH_LEAD_SECONDS) {
            continue;
        }

        let key = format!("cursor:{}", account.id);
        if !allow_attempt(&key) {
            continue;
        }

        attempted_refreshes += 1;
        match cursor_account::refresh_account_async(&account.id).await {
            Ok(updated) => {
                clear_attempt_backoff(&key);
                refreshed_any = true;
                if current_id.as_deref() == Some(updated.id.as_str()) {
                    if let Err(err) = cursor_account::inject_to_cursor(&updated.id) {
                        logger::log_warn(&format!(
                            "[TokenKeeper][Cursor] 当前本地登录回写失败: account_id={}, error={}",
                            updated.id, err
                        ));
                    }
                }
                logger::log_info(&format!(
                    "[TokenKeeper][Cursor] Token 保活成功: account_id={}, email={}",
                    updated.id, updated.email
                ));
            }
            Err(err) => {
                mark_attempt_failure(&key);
                logger::log_warn(&format!(
                    "[TokenKeeper][Cursor] Token 保活失败，进入退避: account_id={}, error={}",
                    account.id, err
                ));
            }
        }
    }

    refreshed_any
}

async fn refresh_due_gemini_accounts() -> bool {
    let accounts = match gemini_account::list_accounts_checked() {
        Ok(accounts) => accounts,
        Err(err) => {
            logger::log_warn(&format!(
                "[TokenKeeper][Gemini] 读取账号列表失败，跳过本轮保活: {}",
                err
            ));
            return false;
        }
    };

    let current_id = gemini_account::resolve_current_account(&accounts).map(|account| account.id);
    let mut refreshed_any = false;
    let mut attempted_refreshes = 0usize;

    for account in accounts {
        if reached_platform_refresh_limit(attempted_refreshes) {
            break;
        }
        if !expires_at_milliseconds_due(account.expiry_date) {
            continue;
        }

        let key = format!("gemini:{}", account.id);
        if !allow_attempt(&key) {
            continue;
        }

        attempted_refreshes += 1;
        match gemini_account::refresh_account_token(&account.id).await {
            Ok(updated) => {
                clear_attempt_backoff(&key);
                refreshed_any = true;
                if current_id.as_deref() == Some(updated.id.as_str()) {
                    if let Err(err) = gemini_account::inject_to_gemini(&updated.id) {
                        logger::log_warn(&format!(
                            "[TokenKeeper][Gemini] 当前本地登录回写失败: account_id={}, error={}",
                            updated.id, err
                        ));
                    }
                }
                logger::log_info(&format!(
                    "[TokenKeeper][Gemini] Token 保活成功: account_id={}, email={}",
                    updated.id, updated.email
                ));
            }
            Err(err) => {
                mark_attempt_failure(&key);
                logger::log_warn(&format!(
                    "[TokenKeeper][Gemini] Token 保活失败，进入退避: account_id={}, error={}",
                    account.id, err
                ));
            }
        }
    }

    refreshed_any
}

async fn refresh_due_github_copilot_accounts() -> bool {
    let accounts = match github_copilot_account::list_accounts_checked() {
        Ok(accounts) => accounts,
        Err(err) => {
            logger::log_warn(&format!(
                "[TokenKeeper][GitHubCopilot] 读取账号列表失败，跳过本轮保活: {}",
                err
            ));
            return false;
        }
    };

    let mut refreshed_any = false;
    let mut attempted_refreshes = 0usize;
    for account in accounts {
        if reached_platform_refresh_limit(attempted_refreshes) {
            break;
        }
        if !expires_at_seconds_due(account.copilot_expires_at) {
            continue;
        }

        let key = format!("github_copilot:{}", account.id);
        if !allow_attempt(&key) {
            continue;
        }

        attempted_refreshes += 1;
        match github_copilot_account::refresh_account_token(&account.id).await {
            Ok(updated) => {
                clear_attempt_backoff(&key);
                refreshed_any = true;
                logger::log_info(&format!(
                    "[TokenKeeper][GitHubCopilot] Token 保活成功: account_id={}, login={}",
                    updated.id, updated.github_login
                ));
            }
            Err(err) => {
                mark_attempt_failure(&key);
                logger::log_warn(&format!(
                    "[TokenKeeper][GitHubCopilot] Token 保活失败，进入退避: account_id={}, error={}",
                    account.id, err
                ));
            }
        }
    }

    refreshed_any
}

async fn refresh_due_windsurf_accounts() -> bool {
    let accounts = match windsurf_account::list_accounts_checked() {
        Ok(accounts) => accounts,
        Err(err) => {
            logger::log_warn(&format!(
                "[TokenKeeper][Windsurf] 读取账号列表失败，跳过本轮保活: {}",
                err
            ));
            return false;
        }
    };

    let current_id = windsurf_account::resolve_current_account_id(&accounts);
    let mut refreshed_any = false;
    let mut attempted_refreshes = 0usize;

    for account in accounts {
        if reached_platform_refresh_limit(attempted_refreshes) {
            break;
        }
        if !expires_at_seconds_due(account.copilot_expires_at) {
            continue;
        }

        let key = format!("windsurf:{}", account.id);
        if !allow_attempt(&key) {
            continue;
        }

        attempted_refreshes += 1;
        match windsurf_account::refresh_account_token(&account.id).await {
            Ok(updated) => {
                clear_attempt_backoff(&key);
                refreshed_any = true;
                if current_id.as_deref() == Some(updated.id.as_str()) {
                    match windsurf_instance::get_default_windsurf_user_data_dir() {
                        Ok(user_data_dir) => {
                            if let Err(err) = windsurf_instance::inject_account_to_profile(
                                user_data_dir.as_path(),
                                &updated.id,
                            ) {
                                logger::log_warn(&format!(
                                    "[TokenKeeper][Windsurf] 当前本地登录回写失败: account_id={}, error={}",
                                    updated.id, err
                                ));
                            }
                        }
                        Err(err) => {
                            logger::log_warn(&format!(
                                "[TokenKeeper][Windsurf] 获取默认用户目录失败，跳过本地回写: {}",
                                err
                            ));
                        }
                    }
                }
                logger::log_info(&format!(
                    "[TokenKeeper][Windsurf] Token 保活成功: account_id={}, login={}",
                    updated.id, updated.github_login
                ));
            }
            Err(err) => {
                mark_attempt_failure(&key);
                logger::log_warn(&format!(
                    "[TokenKeeper][Windsurf] Token 保活失败，进入退避: account_id={}, error={}",
                    account.id, err
                ));
            }
        }
    }

    refreshed_any
}

async fn refresh_due_kiro_accounts() -> bool {
    let accounts = match kiro_account::list_accounts_checked() {
        Ok(accounts) => accounts,
        Err(err) => {
            logger::log_warn(&format!(
                "[TokenKeeper][Kiro] 读取账号列表失败，跳过本轮保活: {}",
                err
            ));
            return false;
        }
    };

    let current_id = kiro_account::resolve_current_account_id(&accounts);
    let mut refreshed_any = false;
    let mut attempted_refreshes = 0usize;

    for account in accounts {
        if reached_platform_refresh_limit(attempted_refreshes) {
            break;
        }
        if !expires_at_seconds_due(account.expires_at) {
            continue;
        }

        let key = format!("kiro:{}", account.id);
        if !allow_attempt(&key) {
            continue;
        }

        attempted_refreshes += 1;
        match kiro_account::refresh_account_token(&account.id).await {
            Ok(updated) => {
                clear_attempt_backoff(&key);
                refreshed_any = true;
                if current_id.as_deref() == Some(updated.id.as_str()) {
                    match kiro_instance::get_default_kiro_user_data_dir() {
                        Ok(user_data_dir) => {
                            if let Err(err) = kiro_instance::inject_account_to_profile(
                                user_data_dir.as_path(),
                                &updated.id,
                            ) {
                                logger::log_warn(&format!(
                                    "[TokenKeeper][Kiro] 当前本地登录回写失败: account_id={}, error={}",
                                    updated.id, err
                                ));
                            }
                        }
                        Err(err) => {
                            logger::log_warn(&format!(
                                "[TokenKeeper][Kiro] 获取默认用户目录失败，跳过本地回写: {}",
                                err
                            ));
                        }
                    }
                }
                logger::log_info(&format!(
                    "[TokenKeeper][Kiro] Token 保活成功: account_id={}, email={}",
                    updated.id, updated.email
                ));
            }
            Err(err) => {
                mark_attempt_failure(&key);
                logger::log_warn(&format!(
                    "[TokenKeeper][Kiro] Token 保活失败，进入退避: account_id={}, error={}",
                    account.id, err
                ));
            }
        }
    }

    refreshed_any
}

async fn refresh_due_codebuddy_accounts() -> bool {
    let accounts = match codebuddy_account::list_accounts_checked() {
        Ok(accounts) => accounts,
        Err(err) => {
            logger::log_warn(&format!(
                "[TokenKeeper][CodeBuddy] 读取账号列表失败，跳过本轮保活: {}",
                err
            ));
            return false;
        }
    };

    let current_id = codebuddy_account::resolve_current_account_id(&accounts);
    let mut refreshed_any = false;
    let mut attempted_refreshes = 0usize;

    for account in accounts {
        if reached_platform_refresh_limit(attempted_refreshes) {
            break;
        }
        if !expires_at_seconds_due(account.expires_at) {
            continue;
        }

        let key = format!("codebuddy:{}", account.id);
        if !allow_attempt(&key) {
            continue;
        }

        attempted_refreshes += 1;
        match codebuddy_account::refresh_account_token(&account.id).await {
            Ok(updated) => {
                clear_attempt_backoff(&key);
                refreshed_any = true;
                if current_id.as_deref() == Some(updated.id.as_str()) {
                    if let Err(err) = codebuddy_account::sync_account_to_default_client(&updated.id)
                    {
                        logger::log_warn(&format!(
                            "[TokenKeeper][CodeBuddy] 当前本地登录回写失败: account_id={}, error={}",
                            updated.id, err
                        ));
                    }
                }
                logger::log_info(&format!(
                    "[TokenKeeper][CodeBuddy] Token 保活成功: account_id={}, email={}",
                    updated.id, updated.email
                ));
            }
            Err(err) => {
                mark_attempt_failure(&key);
                logger::log_warn(&format!(
                    "[TokenKeeper][CodeBuddy] Token 保活失败，进入退避: account_id={}, error={}",
                    account.id, err
                ));
            }
        }
    }

    refreshed_any
}

async fn refresh_due_codebuddy_cn_accounts() -> bool {
    let accounts = match codebuddy_cn_account::list_accounts_checked() {
        Ok(accounts) => accounts,
        Err(err) => {
            logger::log_warn(&format!(
                "[TokenKeeper][CodeBuddyCN] 读取账号列表失败，跳过本轮保活: {}",
                err
            ));
            return false;
        }
    };

    let current_id = codebuddy_cn_account::resolve_current_account_id(&accounts);
    let mut refreshed_any = false;
    let mut attempted_refreshes = 0usize;

    for account in accounts {
        if reached_platform_refresh_limit(attempted_refreshes) {
            break;
        }
        if !expires_at_seconds_due(account.expires_at) {
            continue;
        }

        let key = format!("codebuddy_cn:{}", account.id);
        if !allow_attempt(&key) {
            continue;
        }

        attempted_refreshes += 1;
        match codebuddy_cn_account::refresh_account_token(&account.id).await {
            Ok(updated) => {
                clear_attempt_backoff(&key);
                refreshed_any = true;
                if current_id.as_deref() == Some(updated.id.as_str()) {
                    if let Err(err) =
                        codebuddy_cn_account::sync_account_to_default_client(&updated.id)
                    {
                        logger::log_warn(&format!(
                            "[TokenKeeper][CodeBuddyCN] 当前本地登录回写失败: account_id={}, error={}",
                            updated.id, err
                        ));
                    }
                }
                logger::log_info(&format!(
                    "[TokenKeeper][CodeBuddyCN] Token 保活成功: account_id={}, email={}",
                    updated.id, updated.email
                ));
            }
            Err(err) => {
                mark_attempt_failure(&key);
                logger::log_warn(&format!(
                    "[TokenKeeper][CodeBuddyCN] Token 保活失败，进入退避: account_id={}, error={}",
                    account.id, err
                ));
            }
        }
    }

    refreshed_any
}

async fn refresh_due_workbuddy_accounts() -> bool {
    let accounts = match workbuddy_account::list_accounts_checked() {
        Ok(accounts) => accounts,
        Err(err) => {
            logger::log_warn(&format!(
                "[TokenKeeper][WorkBuddy] 读取账号列表失败，跳过本轮保活: {}",
                err
            ));
            return false;
        }
    };

    let current_id = workbuddy_account::resolve_current_account_id(&accounts);
    let mut refreshed_any = false;
    let mut attempted_refreshes = 0usize;

    for account in accounts {
        if reached_platform_refresh_limit(attempted_refreshes) {
            break;
        }
        if !expires_at_seconds_due(account.expires_at) {
            continue;
        }

        let key = format!("workbuddy:{}", account.id);
        if !allow_attempt(&key) {
            continue;
        }

        attempted_refreshes += 1;
        match workbuddy_account::refresh_account_token(&account.id).await {
            Ok(updated) => {
                clear_attempt_backoff(&key);
                refreshed_any = true;
                if current_id.as_deref() == Some(updated.id.as_str()) {
                    if let Err(err) = workbuddy_account::sync_account_to_default_client(&updated.id)
                    {
                        logger::log_warn(&format!(
                            "[TokenKeeper][WorkBuddy] 当前本地登录回写失败: account_id={}, error={}",
                            updated.id, err
                        ));
                    }
                }
                logger::log_info(&format!(
                    "[TokenKeeper][WorkBuddy] Token 保活成功: account_id={}, email={}",
                    updated.id, updated.email
                ));
            }
            Err(err) => {
                mark_attempt_failure(&key);
                logger::log_warn(&format!(
                    "[TokenKeeper][WorkBuddy] Token 保活失败，进入退避: account_id={}, error={}",
                    account.id, err
                ));
            }
        }
    }

    refreshed_any
}

async fn refresh_due_trae_accounts() -> bool {
    let accounts = match trae_account::list_accounts_checked() {
        Ok(accounts) => accounts,
        Err(err) => {
            logger::log_warn(&format!(
                "[TokenKeeper][Trae] 读取账号列表失败，跳过本轮保活: {}",
                err
            ));
            return false;
        }
    };

    let current_id = trae_account::resolve_current_account_id(&accounts);
    let protection_map = trae_account::resolve_running_account_refresh_protection_map(&accounts);
    let mut refreshed_any = false;
    let mut attempted_refreshes = 0usize;

    for account in accounts {
        if reached_platform_refresh_limit(attempted_refreshes) {
            break;
        }
        let refresh_due = trae_account::should_refresh_token_by_official_window(&account);

        if refresh_due {
            let key = format!("trae_refresh:{}", account.id);
            if !allow_attempt(&key) {
                continue;
            }

            attempted_refreshes += 1;
            if let Some(storage_path) = protection_map.get(account.id.as_str()) {
                logger::log_info(&format!(
                    "[TokenKeeper][Trae] 账号正在运行中的 Trae 客户端实例中使用，改为仅额度刷新: account_id={}, storage_path={}",
                    account.id,
                    storage_path
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "-".to_string())
                ));
                match trae_account::refresh_account_usage_only_async(
                    &account.id,
                    storage_path.as_deref(),
                )
                .await
                {
                    Ok(updated) => {
                        clear_attempt_backoff(&key);
                        mark_trae_strict_check_done(updated.id.as_str());
                        refreshed_any = true;
                        logger::log_info(&format!(
                            "[TokenKeeper][Trae] 仅额度刷新成功: account_id={}, email={}",
                            updated.id, updated.email
                        ));
                    }
                    Err(err) => {
                        mark_attempt_failure(&key);
                        logger::log_warn(&format!(
                            "[TokenKeeper][Trae] 仅额度刷新失败，进入退避: account_id={}, error={}",
                            account.id, err
                        ));
                    }
                }
                continue;
            }

            match trae_account::refresh_account_async(&account.id).await {
                Ok(updated) => {
                    clear_attempt_backoff(&key);
                    mark_trae_strict_check_done(updated.id.as_str());
                    refreshed_any = true;
                    if current_id.as_deref() == Some(updated.id.as_str()) {
                        if process::is_trae_running() {
                            logger::log_info(&format!(
                                "[TokenKeeper][Trae] Trae 运行中，跳过当前账号本地回写: account_id={}",
                                updated.id
                            ));
                        } else if let Err(err) = trae_account::inject_to_trae(&updated.id) {
                            logger::log_warn(&format!(
                                "[TokenKeeper][Trae] 当前本地登录回写失败: account_id={}, error={}",
                                updated.id, err
                            ));
                        }
                    }
                    logger::log_info(&format!(
                        "[TokenKeeper][Trae] Token 保活成功: account_id={}, email={}",
                        updated.id, updated.email
                    ));
                }
                Err(err) => {
                    mark_attempt_failure(&key);
                    logger::log_warn(&format!(
                        "[TokenKeeper][Trae] Token 保活失败，进入退避: account_id={}, error={}",
                        account.id, err
                    ));
                }
            }
            continue;
        }

        if current_id.as_deref() != Some(account.id.as_str()) {
            continue;
        }
        if !should_run_trae_strict_check(account.id.as_str()) {
            continue;
        }

        let strict_key = format!("trae_strict:{}", account.id);
        if !allow_attempt(&strict_key) {
            continue;
        }

        match trae_account::check_login_token(&account.id).await {
            Ok(verdict) => {
                clear_attempt_backoff(&strict_key);
                mark_trae_strict_check_done(account.id.as_str());
                if verdict.is_valid {
                    logger::log_info(&format!(
                        "[TokenKeeper][Trae] 严格校验通过: account_id={}",
                        account.id
                    ));
                } else {
                    logger::log_warn(&format!(
                        "[TokenKeeper][Trae] 严格校验未通过: account_id={}, error_code={}, is_login={}",
                        account.id,
                        verdict.error_code.as_deref().unwrap_or("-"),
                        verdict
                            .is_login
                            .map(|value| if value { "true" } else { "false" })
                            .unwrap_or("-")
                    ));
                }
            }
            Err(err) => {
                mark_attempt_failure(&strict_key);
                logger::log_warn(&format!(
                    "[TokenKeeper][Trae] 严格校验失败，进入退避: account_id={}, error={}",
                    account.id, err
                ));
            }
        }
    }

    refreshed_any
}
