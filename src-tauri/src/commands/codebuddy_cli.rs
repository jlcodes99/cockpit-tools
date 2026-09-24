use std::time::Instant;

use tauri::AppHandle;

use crate::models::workbuddy::WorkbuddyAccount;
use crate::modules::{logger, provider_current_state, workbuddy_account, workbuddy_oauth};

/// 切号前预留的过期缓冲（毫秒）：access token 距过期不足该值时先刷新。
const CODEBUDDY_CLI_TOKEN_EXPIRY_BUFFER_MS: i64 = 60 * 1000;

/// 将 WorkBuddy 账号库中的账号写入 CodeBuddy CLI 官方认证文件
/// （%LOCALAPPDATA%/CodeBuddyExtension/Data/Public/auth/Tencent-Cloud.coding-copilot.info）。
///
/// 流程：加载账号 → 若 access token 即将过期则先刷新（刷新结果回写账号库）→
/// 原子写入 CLI 认证文件 → 记录 codebuddy_cli 当前账号 → 更新托盘。
#[tauri::command]
pub async fn inject_workbuddy_to_codebuddy_cli(
    app: AppHandle,
    account_id: String,
) -> Result<String, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[CodeBuddy CLI Switch] 开始切换账号: account_id={}",
        account_id
    ));

    let mut account = workbuddy_account::load_account(&account_id)
        .ok_or_else(|| format!("WorkBuddy account not found: {}", account_id))?;

    let expires_at = account.expires_at.unwrap_or(0);
    let now_ms = chrono::Utc::now().timestamp_millis();
    if expires_at <= 0 || expires_at < now_ms + CODEBUDDY_CLI_TOKEN_EXPIRY_BUFFER_MS {
        logger::log_info(&format!(
            "[CodeBuddy CLI Switch] access token 即将过期，先刷新: account_id={}, expires_at={}, now_ms={}",
            account_id, expires_at, now_ms
        ));
        account = workbuddy_account::refresh_account_token(&account_id).await?;
    }

    workbuddy_account::write_account_to_codebuddy_cli(&account)?;
    provider_current_state::set_current_account_id(
        "codebuddy_cli",
        Some(account_id.as_str()),
    )?;

    let _ = crate::modules::tray::update_tray_menu(&app);

    logger::log_info(&format!(
        "[CodeBuddy CLI Switch] 切号成功: account_id={}, email={}, elapsed={}ms",
        account.id,
        account.email,
        started_at.elapsed().as_millis()
    ));
    Ok(format!(
        "切换完成: {}（运行中的 CodeBuddy CLI 需重启后生效）",
        account.email
    ))
}

/// 从本机 CodeBuddy CLI 官方认证文件导入登录账号到 WorkBuddy 账号库。
#[tauri::command]
pub async fn import_codebuddy_cli_from_local(
    app: AppHandle,
) -> Result<Vec<WorkbuddyAccount>, String> {
    let mut local_payload = match workbuddy_account::import_codebuddy_cli_payload_from_local()? {
        Some(payload) => payload,
        None => {
            return Err(
                "未在本机 CodeBuddy CLI 官方认证文件中找到登录信息（%LOCALAPPDATA%/CodeBuddyExtension/Data/Public/auth/Tencent-Cloud.coding-copilot.info）"
                    .to_string(),
            )
        }
    };

    match workbuddy_oauth::build_payload_from_token(&local_payload.access_token).await {
        Ok(mut payload) => {
            if payload.uid.is_none() {
                payload.uid = local_payload.uid.clone();
            }
            if payload.nickname.is_none() {
                payload.nickname = local_payload.nickname.clone();
            }
            if payload.refresh_token.is_none() {
                payload.refresh_token = local_payload.refresh_token.clone();
            }
            if payload.domain.is_none() {
                payload.domain = local_payload.domain.clone();
            }
            if payload.token_type.is_none() {
                payload.token_type = local_payload.token_type.clone();
            }
            if payload.expires_at.is_none() {
                payload.expires_at = local_payload.expires_at;
            }
            if payload.auth_raw.is_none() {
                payload.auth_raw = local_payload.auth_raw.clone();
            }
            if payload.profile_raw.is_none() {
                payload.profile_raw = local_payload.profile_raw.clone();
            }
            if payload.email.trim().is_empty() || payload.email == "unknown" {
                payload.email = local_payload.email.clone();
            }
            local_payload = payload;
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[CodeBuddy CLI Import Local] 拉取账号资料失败，将保留本地导入结果：{}",
                err
            ));
        }
    }

    let mut account = workbuddy_account::upsert_account(local_payload.clone())?;

    for existing in workbuddy_account::list_accounts() {
        if existing.id == account.id {
            continue;
        }
        if existing.access_token != account.access_token {
            continue;
        }
        let is_placeholder = existing.email.trim().eq_ignore_ascii_case("unknown")
            || existing.email.trim().is_empty()
            || existing
                .uid
                .as_deref()
                .map(|s| s.trim().is_empty())
                .unwrap_or(true);
        if is_placeholder {
            if let Err(err) = workbuddy_account::remove_account(&existing.id) {
                logger::log_warn(&format!(
                    "[CodeBuddy CLI Import Local] 清理占位账号失败：id={}, error={}",
                    existing.id, err
                ));
            }
        }
    }

    let account_id = account.id.clone();
    match workbuddy_account::refresh_account_token(&account_id).await {
        Ok(refreshed) => {
            account = refreshed;
        }
        Err(e) => {
            logger::log_warn(&format!(
                "[CodeBuddy CLI Import Local] 导入后刷新失败，保留原账号信息：account_id={}, error={}",
                account_id, e
            ));
        }
    }

    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(vec![account])
}

