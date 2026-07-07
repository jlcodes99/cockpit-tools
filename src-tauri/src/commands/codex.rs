use crate::models::codex::{
    CodexAccount, CodexApiProviderMode, CodexAppSpeed, CodexAppSpeedConfig, CodexQuickConfig,
    CodexQuota, CodexTokens,
};
use crate::models::codex_local_access::{
    CodexLocalAccessAccountModelRule, CodexLocalAccessChatMessage, CodexLocalAccessChatResult,
    CodexLocalAccessClientBaseUrlHost, CodexLocalAccessCustomRoutingRule,
    CodexLocalAccessGatewayMode, CodexLocalAccessModelAlias, CodexLocalAccessModelPricing,
    CodexLocalAccessPortCleanupResult, CodexLocalAccessRequestKind,
    CodexLocalAccessRoutingStrategy, CodexLocalAccessScope, CodexLocalAccessState,
    CodexLocalAccessTestFailure, CodexLocalAccessTestResult, CodexLocalAccessTimeoutPreset,
    CodexLocalAccessTimeouts, CodexLocalAccessUsageEventPage,
};
use crate::modules::{
    account, codex_account, codex_local_access, codex_oauth, codex_quota, codex_session_visibility,
    codex_speed, codex_wakeup, codex_wakeup_scheduler, config, logger, openclaw_auth,
    opencode_auth, process,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use tauri::AppHandle;
use tauri::Emitter;
use tauri_plugin_opener::OpenerExt;

static CODEX_POST_REFRESH_CHECK_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
const CODEX_BATCH_DELETE_JOBS_DIR: &str = "codex_batch_delete_jobs";
const CODEX_MAIL_PREVIEW_MAX_BYTES: usize = 512 * 1024;
static CODEX_BATCH_DELETE_JOBS: LazyLock<Mutex<HashMap<String, CodexBatchDeleteJob>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexBatchDeleteJobStatus {
    job_id: String,
    status: String,
    total: usize,
    completed: usize,
    failed: usize,
    errors: Vec<CodexBatchDeleteError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexBatchDeleteError {
    account_id: String,
    error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexBatchDeleteJob {
    status: String,
    total: usize,
    completed: usize,
    failed: usize,
    errors: Vec<CodexBatchDeleteError>,
    account_ids: Vec<String>,
    next_index: usize,
    created_at: i64,
    updated_at: i64,
}

fn now_unix_seconds() -> i64 {
    chrono::Utc::now().timestamp()
}

fn get_codex_batch_delete_jobs_dir() -> PathBuf {
    let data_dir = account::get_data_dir()
        .or_else(|_| account::resolve_data_dir())
        .unwrap_or_else(|_| PathBuf::from(".antigravity_cockpit"));
    let dir = data_dir.join(CODEX_BATCH_DELETE_JOBS_DIR);
    let _ = fs::create_dir_all(&dir);
    dir
}

fn sanitize_codex_batch_delete_job_id(job_id: &str) -> Result<String, String> {
    let trimmed = job_id.trim();
    if trimmed.is_empty() {
        return Err("批量删除任务 ID 为空".to_string());
    }
    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err("批量删除任务 ID 不合法".to_string());
    }
    Ok(trimmed.to_string())
}

fn codex_batch_delete_job_snapshot_path(job_id: &str) -> Result<PathBuf, String> {
    let safe_id = sanitize_codex_batch_delete_job_id(job_id)?;
    Ok(get_codex_batch_delete_jobs_dir().join(format!("{}.json", safe_id)))
}

fn save_codex_batch_delete_job_snapshot(
    job_id: &str,
    job: &CodexBatchDeleteJob,
) -> Result<(), String> {
    let path = codex_batch_delete_job_snapshot_path(job_id)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建 Codex 批量删除任务目录失败: {}", error))?;
    }
    let content = serde_json::to_string_pretty(job)
        .map_err(|error| format!("序列化 Codex 批量删除任务失败: {}", error))?;
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, content).map_err(|error| {
        format!(
            "写入 Codex 批量删除任务快照失败: path={}, error={}",
            tmp_path.display(),
            error
        )
    })?;
    fs::rename(&tmp_path, &path).map_err(|error| {
        let _ = fs::remove_file(&tmp_path);
        format!(
            "更新 Codex 批量删除任务快照失败: path={}, error={}",
            path.display(),
            error
        )
    })
}

fn save_codex_batch_delete_job_snapshot_best_effort(job_id: &str, job: &CodexBatchDeleteJob) {
    if let Err(error) = save_codex_batch_delete_job_snapshot(job_id, job) {
        logger::log_warn(&format!(
            "[Codex Batch Delete] 保存任务快照失败: job_id={}, error={}",
            job_id, error
        ));
    }
}

fn load_codex_batch_delete_job_snapshot(
    job_id: &str,
) -> Result<Option<CodexBatchDeleteJob>, String> {
    let path = codex_batch_delete_job_snapshot_path(job_id)?;
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path).map_err(|error| {
        format!(
            "读取 Codex 批量删除任务快照失败: path={}, error={}",
            path.display(),
            error
        )
    })?;
    let mut job: CodexBatchDeleteJob = serde_json::from_str(&content).map_err(|error| {
        format!(
            "解析 Codex 批量删除任务快照失败: path={}, error={}",
            path.display(),
            error
        )
    })?;
    if job.status == "running" {
        job.status = "paused".to_string();
    }
    Ok(Some(job))
}

fn remove_codex_batch_delete_job_snapshot(job_id: &str) {
    if let Ok(path) = codex_batch_delete_job_snapshot_path(job_id) {
        let _ = fs::remove_file(path);
    }
}

fn ensure_codex_batch_delete_job_loaded(job_id: &str) -> Result<(), String> {
    {
        let jobs = CODEX_BATCH_DELETE_JOBS.lock().unwrap();
        if jobs.contains_key(job_id) {
            return Ok(());
        }
    }
    let Some(job) = load_codex_batch_delete_job_snapshot(job_id)? else {
        return Err("批量删除任务不存在".to_string());
    };
    let mut jobs = CODEX_BATCH_DELETE_JOBS.lock().unwrap();
    jobs.entry(job_id.to_string()).or_insert(job);
    Ok(())
}

fn codex_batch_delete_status(job_id: &str, job: &CodexBatchDeleteJob) -> CodexBatchDeleteJobStatus {
    CodexBatchDeleteJobStatus {
        job_id: job_id.to_string(),
        status: job.status.clone(),
        total: job.total,
        completed: job.completed,
        failed: job.failed,
        errors: job.errors.clone(),
    }
}

fn get_codex_batch_delete_job_status(job_id: &str) -> Result<CodexBatchDeleteJobStatus, String> {
    ensure_codex_batch_delete_job_loaded(job_id)?;
    let jobs = CODEX_BATCH_DELETE_JOBS.lock().unwrap();
    let job = jobs
        .get(job_id)
        .ok_or_else(|| "批量删除任务不存在".to_string())?;
    Ok(codex_batch_delete_status(job_id, job))
}

async fn run_codex_batch_delete_job(job_id: String) {
    loop {
        let next_account_id = {
            let mut jobs = CODEX_BATCH_DELETE_JOBS.lock().unwrap();
            let Some(job) = jobs.get_mut(&job_id) else {
                return;
            };
            if job.status == "paused" {
                job.updated_at = now_unix_seconds();
                let snapshot = job.clone();
                drop(jobs);
                save_codex_batch_delete_job_snapshot_best_effort(&job_id, &snapshot);
                return;
            }
            if job.next_index >= job.account_ids.len() {
                job.status = if job.failed > 0 {
                    "failed".to_string()
                } else {
                    "completed".to_string()
                };
                job.updated_at = now_unix_seconds();
                let snapshot = job.clone();
                drop(jobs);
                save_codex_batch_delete_job_snapshot_best_effort(&job_id, &snapshot);
                return;
            }
            job.status = "running".to_string();
            job.account_ids[job.next_index].clone()
        };

        let remove_account_id = next_account_id.clone();
        let remove_result = tauri::async_runtime::spawn_blocking(move || {
            codex_account::remove_account(&remove_account_id)
        })
        .await
        .map_err(|error| format!("批量删除后台任务失败: {}", error))
        .and_then(|result| result);

        let mut deleted_account_id_for_cleanup = None;
        let result = match remove_result {
            Ok(()) => {
                deleted_account_id_for_cleanup = Some(next_account_id.clone());
                Ok(())
            }
            Err(error) => Err(error),
        };

        if let Some(account_id) = deleted_account_id_for_cleanup {
            if let Err(error) =
                codex_local_access::remove_deleted_accounts_from_local_access_pool(&[account_id])
                    .await
            {
                logger::log_warn(&format!(
                    "[Codex Batch Delete] 清理 API 服务账号池引用失败: job_id={}, error={}",
                    job_id, error
                ));
            }
        }

        let snapshot = {
            let mut jobs = CODEX_BATCH_DELETE_JOBS.lock().unwrap();
            let Some(job) = jobs.get_mut(&job_id) else {
                return;
            };
            job.completed += 1;
            job.next_index += 1;
            if let Err(error) = result {
                job.failed += 1;
                job.errors.push(CodexBatchDeleteError {
                    account_id: next_account_id,
                    error,
                });
            }
            job.updated_at = now_unix_seconds();
            job.clone()
        };
        save_codex_batch_delete_job_snapshot_best_effort(&job_id, &snapshot);
    }
}

fn start_codex_batch_delete_job(
    account_ids: Vec<String>,
) -> Result<CodexBatchDeleteJobStatus, String> {
    let normalized_ids: Vec<String> = account_ids
        .into_iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    if normalized_ids.is_empty() {
        return Ok(CodexBatchDeleteJobStatus {
            job_id: String::new(),
            status: "completed".to_string(),
            total: 0,
            completed: 0,
            failed: 0,
            errors: Vec::new(),
        });
    }

    let job_id = format!("codex-delete-{}", uuid::Uuid::new_v4());
    let now = now_unix_seconds();
    let job = CodexBatchDeleteJob {
        status: "running".to_string(),
        total: normalized_ids.len(),
        completed: 0,
        failed: 0,
        errors: Vec::new(),
        account_ids: normalized_ids,
        next_index: 0,
        created_at: now,
        updated_at: now,
    };
    save_codex_batch_delete_job_snapshot(&job_id, &job)?;
    {
        let mut jobs = CODEX_BATCH_DELETE_JOBS.lock().unwrap();
        jobs.insert(job_id.clone(), job);
    }
    let task_job_id = job_id.clone();
    tauri::async_runtime::spawn(async move {
        run_codex_batch_delete_job(task_job_id).await;
    });

    get_codex_batch_delete_job_status(&job_id)
}

fn resume_codex_batch_delete_job(job_id: &str) -> Result<CodexBatchDeleteJobStatus, String> {
    ensure_codex_batch_delete_job_loaded(job_id)?;
    let should_spawn = {
        let mut jobs = CODEX_BATCH_DELETE_JOBS.lock().unwrap();
        let job = jobs
            .get_mut(job_id)
            .ok_or_else(|| "批量删除任务不存在".to_string())?;
        if matches!(job.status.as_str(), "completed" | "failed" | "running") {
            return Ok(codex_batch_delete_status(job_id, job));
        }
        job.status = "running".to_string();
        job.updated_at = now_unix_seconds();
        save_codex_batch_delete_job_snapshot_best_effort(job_id, job);
        true
    };
    if should_spawn {
        let task_job_id = job_id.to_string();
        tauri::async_runtime::spawn(async move {
            run_codex_batch_delete_job(task_job_id).await;
        });
    }
    get_codex_batch_delete_job_status(job_id)
}

fn pause_codex_batch_delete_job(job_id: &str) -> Result<CodexBatchDeleteJobStatus, String> {
    ensure_codex_batch_delete_job_loaded(job_id)?;
    let snapshot = {
        let mut jobs = CODEX_BATCH_DELETE_JOBS.lock().unwrap();
        let job = jobs
            .get_mut(job_id)
            .ok_or_else(|| "批量删除任务不存在".to_string())?;
        if job.status == "running" {
            job.status = "paused".to_string();
            job.updated_at = now_unix_seconds();
        }
        job.clone()
    };
    save_codex_batch_delete_job_snapshot_best_effort(job_id, &snapshot);
    Ok(codex_batch_delete_status(job_id, &snapshot))
}

fn retry_failed_codex_batch_delete_job(job_id: &str) -> Result<CodexBatchDeleteJobStatus, String> {
    ensure_codex_batch_delete_job_loaded(job_id)?;
    let should_spawn = {
        let mut jobs = CODEX_BATCH_DELETE_JOBS.lock().unwrap();
        let job = jobs
            .get_mut(job_id)
            .ok_or_else(|| "批量删除任务不存在".to_string())?;
        if job.status == "running" || job.errors.is_empty() {
            return Ok(codex_batch_delete_status(job_id, job));
        }
        let mut retry_ids = Vec::new();
        for error in &job.errors {
            if !retry_ids.contains(&error.account_id) {
                retry_ids.push(error.account_id.clone());
            }
        }
        job.account_ids = retry_ids;
        job.total = job.account_ids.len();
        job.completed = 0;
        job.failed = 0;
        job.errors = Vec::new();
        job.next_index = 0;
        job.status = "running".to_string();
        job.updated_at = now_unix_seconds();
        save_codex_batch_delete_job_snapshot_best_effort(job_id, job);
        !job.account_ids.is_empty()
    };
    if should_spawn {
        let task_job_id = job_id.to_string();
        tauri::async_runtime::spawn(async move {
            run_codex_batch_delete_job(task_job_id).await;
        });
    }
    get_codex_batch_delete_job_status(job_id)
}

fn clear_codex_batch_delete_job(job_id: &str) {
    {
        let mut jobs = CODEX_BATCH_DELETE_JOBS.lock().unwrap();
        jobs.remove(job_id);
    }
    remove_codex_batch_delete_job_snapshot(job_id);
}

#[derive(Clone)]
struct CodexLaunchCredentialSnapshot {
    kind: String,
    source: String,
}

fn codex_launch_credential_kind_for_account(account: &CodexAccount) -> &'static str {
    if account.is_api_key_auth() {
        "api"
    } else {
        "account"
    }
}

fn codex_launch_credential_snapshot_for_account(
    account: &CodexAccount,
    source_prefix: &str,
) -> CodexLaunchCredentialSnapshot {
    CodexLaunchCredentialSnapshot {
        kind: codex_launch_credential_kind_for_account(account).to_string(),
        source: format!("{}{}", source_prefix, account.id),
    }
}

fn codex_launch_credential_snapshot_for_account_id(
    account_id: &str,
    source_prefix: &str,
) -> Option<CodexLaunchCredentialSnapshot> {
    let account_id = account_id.trim();
    if account_id.is_empty() {
        return None;
    }

    if crate::modules::codex_instance::is_api_service_bind_account_id(account_id)
        || crate::modules::codex_instance::parse_provider_gateway_bind_account_id(account_id)
            .is_some()
        || codex_local_access::is_local_access_runtime_account_id(account_id)
    {
        return Some(CodexLaunchCredentialSnapshot {
            kind: "api".to_string(),
            source: format!("{}{}", source_prefix, account_id),
        });
    }

    codex_account::load_account(account_id)
        .map(|account| codex_launch_credential_snapshot_for_account(&account, source_prefix))
}

fn read_current_codex_launch_credential_snapshot() -> Option<CodexLaunchCredentialSnapshot> {
    let codex_home = codex_account::get_codex_home();
    if let Some(account_id) =
        codex_account::read_managed_projection_account_id_from_dir(&codex_home)
    {
        if let Some(snapshot) =
            codex_launch_credential_snapshot_for_account_id(&account_id, "profile:")
        {
            return Some(snapshot);
        }
    }

    if let Ok(settings) = crate::modules::codex_instance::load_default_settings() {
        if let Some(bind_account_id) = settings.bind_account_id.as_deref() {
            if let Some(snapshot) =
                codex_launch_credential_snapshot_for_account_id(bind_account_id, "default-bind:")
            {
                return Some(snapshot);
            }
        }
    }

    codex_account::get_current_account()
        .as_ref()
        .map(|account| codex_launch_credential_snapshot_for_account(account, "current-index:"))
}

fn repair_codex_session_visibility_after_credential_kind_change(
    context: &str,
    before: Option<CodexLaunchCredentialSnapshot>,
    after: Option<CodexLaunchCredentialSnapshot>,
    auto_repair_mode: Option<codex_session_visibility::CodexSessionVisibilityAutoRepairMode>,
) {
    let (Some(before), Some(after)) = (before, after) else {
        return;
    };
    if before.kind == after.kind {
        return;
    }

    let auto_repair_mode = auto_repair_mode.unwrap_or_default();
    logger::log_info(&format!(
        "[Codex Session Visibility] {}: credential kind changed, defer quick repair to frontend notice, mode={}, from_kind={}, to_kind={}, from_source={}, to_source={}",
        context,
        auto_repair_mode.label(),
        before.kind,
        after.kind,
        before.source,
        after.source
    ));
}

fn restart_codex_specified_app_if_enabled(user_config: &config::UserConfig) {
    if !user_config.codex_restart_specified_app_on_switch {
        logger::log_info("已关闭切换 Codex 时自动重启指定应用");
        return;
    }

    let path = user_config.codex_specified_app_path.trim();
    if path.is_empty() {
        logger::log_warn("已开启切换 Codex 时自动重启指定应用，但未配置应用路径，已跳过");
        return;
    }

    match process::restart_specified_app_by_path(path, 20) {
        Ok(()) => {
            logger::log_info(&format!("已重启指定应用: {}", path));
        }
        Err(error) => {
            logger::log_warn(&format!("重启指定应用失败（path={}）：{}", path, error));
        }
    }
}

/// 列出所有 Codex 账号
#[tauri::command]
pub fn list_codex_accounts() -> Result<Vec<CodexAccount>, String> {
    codex_account::list_accounts_checked()
}

/// 获取当前激活的 Codex 账号
#[tauri::command]
pub fn get_current_codex_account() -> Result<Option<CodexAccount>, String> {
    Ok(codex_account::get_current_account())
}

#[tauri::command]
pub fn get_codex_config_toml_path() -> Result<String, String> {
    let path = codex_account::get_codex_home().join("config.toml");
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn open_codex_config_toml(app: AppHandle) -> Result<(), String> {
    let path = codex_account::get_codex_home().join("config.toml");
    if !path.exists() {
        return Err(format!("未找到 Codex config.toml 文件: {}", path.display()));
    }

    app.opener()
        .open_path(path.to_string_lossy().to_string(), None::<String>)
        .map_err(|e| format!("打开 Codex config.toml 失败: {}", e))
}

#[tauri::command]
pub fn get_codex_quick_config() -> Result<CodexQuickConfig, String> {
    codex_account::load_current_quick_config()
}

#[tauri::command]
pub fn save_codex_quick_config(
    model_context_window: Option<i64>,
    auto_compact_token_limit: Option<i64>,
) -> Result<CodexQuickConfig, String> {
    codex_account::save_current_quick_config(model_context_window, auto_compact_token_limit)
}

#[tauri::command]
pub fn get_codex_app_speed_config() -> Result<CodexAppSpeedConfig, String> {
    codex_speed::get_app_speed_config()
}

#[tauri::command]
pub fn save_codex_app_speed(speed: CodexAppSpeed) -> Result<CodexAppSpeedConfig, String> {
    codex_speed::save_api_service_app_speed(speed)
}

#[tauri::command]
pub fn get_codex_api_service_app_speed_config() -> Result<CodexAppSpeedConfig, String> {
    codex_speed::get_api_service_app_speed_config()
}

#[tauri::command]
pub fn save_codex_api_service_app_speed(
    speed: CodexAppSpeed,
) -> Result<CodexAppSpeedConfig, String> {
    let saved = codex_speed::save_api_service_app_speed(speed.clone())?;
    if let Ok(settings) = crate::modules::codex_instance::load_default_settings() {
        if settings.bind_account_id.as_deref()
            == Some(crate::modules::codex_instance::CODEX_API_SERVICE_BIND_ACCOUNT_ID)
        {
            let _ = crate::modules::codex_instance::update_default_app_speed(speed);
        }
    }
    codex_local_access::trigger_gateway_reload_in_background("保存 API 服务速度配置");
    Ok(saved)
}

#[tauri::command]
pub fn update_codex_account_app_speed(
    account_id: String,
    speed: CodexAppSpeed,
) -> Result<CodexAccount, String> {
    let account = codex_account::update_account_app_speed(&account_id, speed)?;
    let account_speed = account.app_speed.clone();
    let current_account_id = codex_account::load_account_index().current_account_id;
    let provider_gateway_bind_account_id =
        crate::modules::codex_instance::provider_gateway_bind_account_id(&account_id);
    let default_bind_account_id = crate::modules::codex_instance::load_default_settings()
        .ok()
        .and_then(|settings| settings.bind_account_id);
    let default_bind_matches_provider_gateway = provider_gateway_bind_account_id
        .as_deref()
        .map(|bind_account_id| default_bind_account_id.as_deref() == Some(bind_account_id))
        .unwrap_or(false);
    if current_account_id.as_deref() == Some(account_id.as_str())
        || default_bind_account_id.as_deref() == Some(account_id.as_str())
        || default_bind_matches_provider_gateway
    {
        codex_speed::write_official_app_speed(account_speed.clone())?;
        let _ = crate::modules::codex_instance::update_default_app_speed(account_speed.clone());
        if default_bind_matches_provider_gateway {
            if let Ok(default_dir) = crate::modules::codex_instance::get_default_codex_home() {
                codex_local_access::reload_provider_gateway_for_profile_in_background(
                    default_dir,
                    account_id.clone(),
                    "更新默认 provider gateway 账号速度配置",
                );
            }
        }
    }

    let bound_instances = crate::modules::codex_instance::update_bound_instances_app_speed(
        &account_id,
        account_speed.clone(),
    )?;
    for instance in bound_instances {
        codex_speed::write_app_speed_for_dir(
            std::path::Path::new(&instance.user_data_dir),
            account_speed.clone(),
        )?;
    }

    if let Some(provider_gateway_bind_account_id) = provider_gateway_bind_account_id.as_deref() {
        let provider_gateway_bound_instances =
            crate::modules::codex_instance::update_bound_instances_app_speed(
                provider_gateway_bind_account_id,
                account_speed.clone(),
            )?;
        for instance in provider_gateway_bound_instances {
            codex_speed::write_app_speed_for_dir(
                std::path::Path::new(&instance.user_data_dir),
                account_speed.clone(),
            )?;
            codex_local_access::reload_provider_gateway_for_profile_in_background(
                std::path::PathBuf::from(instance.user_data_dir),
                account_id.clone(),
                "更新 provider gateway 账号速度配置",
            );
        }
    }
    Ok(account)
}

/// 刷新账号资料（团队名/结构）
#[tauri::command]
pub async fn refresh_codex_account_profile(account_id: String) -> Result<CodexAccount, String> {
    codex_account::refresh_account_profile(&account_id).await
}

/// 切换 Codex 账号（包含 token 刷新检查）
#[tauri::command]
pub async fn switch_codex_account(
    app: AppHandle,
    account_id: String,
    auto_repair_mode: Option<codex_session_visibility::CodexSessionVisibilityAutoRepairMode>,
) -> Result<CodexAccount, String> {
    let flow_started = Instant::now();
    logger::log_info(&format!(
        "[Codex Switch][Backend] switch_codex_account started: account_id={}",
        account_id
    ));
    let previous_credential = read_current_codex_launch_credential_snapshot();
    logger::log_info(&format!(
        "[Codex Switch][Backend] previous credential resolved: account_id={}, elapsed_ms={}",
        account_id,
        flow_started.elapsed().as_millis()
    ));
    // 切换账号（写入 auth.json）
    let switch_started = Instant::now();
    let account = codex_account::switch_account_managed(&account_id).await?;
    logger::log_info(&format!(
        "[Codex Switch][Backend] switch_account_managed finished: account_id={}, elapsed_ms={}, total_ms={}",
        account_id,
        switch_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));
    let account_speed = account.app_speed.clone();
    let speed_started = Instant::now();
    codex_speed::write_official_app_speed(account_speed.clone())?;
    logger::log_info(&format!(
        "[Codex Switch][Backend] write official app speed finished: account_id={}, elapsed_ms={}, total_ms={}",
        account_id,
        speed_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));
    let repair_started = Instant::now();
    repair_codex_session_visibility_after_credential_kind_change(
        "after-account-switch",
        previous_credential,
        Some(codex_launch_credential_snapshot_for_account(
            &account,
            "target-account:",
        )),
        auto_repair_mode,
    );
    logger::log_info(&format!(
        "[Codex Switch][Backend] session visibility repair stage finished: account_id={}, elapsed_ms={}, total_ms={}",
        account_id,
        repair_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));

    // 同步更新 Codex 默认实例的绑定账号（不同步到 Antigravity，因为账号体系不同）
    let default_settings_started = Instant::now();
    if let Err(e) = crate::modules::codex_instance::update_default_settings(
        Some(Some(account_id.clone())),
        None,
        Some(false),
        None,
        None,
    ) {
        logger::log_warn(&format!("更新 Codex 默认实例绑定账号失败: {}", e));
    } else {
        logger::log_info(&format!(
            "已同步更新 Codex 默认实例绑定账号: {}",
            account_id
        ));
    }
    if let Err(e) = crate::modules::codex_instance::update_default_app_speed(account_speed) {
        logger::log_warn(&format!("更新 Codex 默认实例速度失败: {}", e));
    }
    logger::log_info(&format!(
        "[Codex Switch][Backend] default settings update finished: account_id={}, elapsed_ms={}, total_ms={}",
        account_id,
        default_settings_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));

    let user_config = config::get_user_config();

    let mut opencode_updated = false;
    if user_config.opencode_auth_overwrite_on_switch {
        match opencode_auth::replace_openai_entry_from_codex(&account) {
            Ok(()) => {
                opencode_updated = true;
            }
            Err(e) => {
                logger::log_warn(&format!("OpenCode auth.json 更新跳过: {}", e));
            }
        }
    } else {
        logger::log_info("已关闭切换 Codex 时覆盖 OpenCode 登录信息");
    }

    if user_config.opencode_sync_on_switch {
        if user_config.opencode_auth_overwrite_on_switch && opencode_updated {
            if process::is_opencode_running() {
                if let Err(e) = process::close_opencode(20) {
                    logger::log_warn(&format!("OpenCode 关闭失败: {}", e));
                }
            } else {
                logger::log_info("OpenCode 未在运行，准备启动");
            }
            if let Err(e) = process::start_opencode_with_path(Some(&user_config.opencode_app_path))
            {
                logger::log_warn(&format!("OpenCode 启动失败: {}", e));
            }
        } else if !user_config.opencode_auth_overwrite_on_switch {
            logger::log_info("OpenCode 登录覆盖已关闭，跳过自动重启");
        } else {
            logger::log_info("OpenCode 未更新 auth.json，跳过启动/重启");
        }
    } else {
        logger::log_info("已关闭 OpenCode 自动重启");
    }

    if user_config.openclaw_auth_overwrite_on_switch {
        match openclaw_auth::replace_openai_codex_entry_from_codex(&account) {
            Ok(()) => {}
            Err(e) => {
                logger::log_warn(&format!("OpenClaw auth 同步失败: {}", e));
            }
        }
    } else {
        logger::log_info("已关闭切换 Codex 时覆盖 OpenClaw 登录信息");
    }

    if user_config.codex_launch_on_switch {
        let launch_started = Instant::now();
        #[cfg(target_os = "macos")]
        if process::is_codex_running() {
            logger::log_info("检测到 Codex 正在运行，将按默认实例 PID 逻辑重启");
        }
        match crate::commands::codex_instance::codex_start_default_with_prepared_profile().await {
            Ok(_) => {}
            Err(e) => {
                logger::log_warn(&format!("Codex 启动失败: {}", e));
                if e.starts_with("APP_PATH_NOT_FOUND:") {
                    let _ = app.emit(
                        "app:path_missing",
                        serde_json::json!({ "app": "codex", "retry": { "kind": "default" } }),
                    );
                }
            }
        }
        logger::log_info(&format!(
            "[Codex Switch][Backend] codex_start_default_with_prepared_profile finished: account_id={}, elapsed_ms={}, total_ms={}",
            account_id,
            launch_started.elapsed().as_millis(),
            flow_started.elapsed().as_millis()
        ));
    } else {
        logger::log_info("已关闭切换 Codex 时自动启动 Codex App");
    }

    let restart_specified_started = Instant::now();
    restart_codex_specified_app_if_enabled(&user_config);
    logger::log_info(&format!(
        "[Codex Switch][Backend] restart specified app stage finished: account_id={}, elapsed_ms={}, total_ms={}",
        account_id,
        restart_specified_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));

    let tray_started = Instant::now();
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Codex Switch][Backend] switch_codex_account finished: account_id={}, tray_elapsed_ms={}, total_ms={}",
        account_id,
        tray_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));
    Ok(account)
}

async fn run_codex_post_refresh_checks(app: &AppHandle) {
    if CODEX_POST_REFRESH_CHECK_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        logger::log_info("[AutoSwitch][Codex] 后置检查进行中，跳过本次执行");
        return;
    }

    let mut switched = false;

    match codex_account::pick_auto_switch_target_if_needed() {
        Ok(Some(target)) => {
            let target_id = target.id.clone();
            match switch_codex_account(app.clone(), target_id.clone(), None).await {
                Ok(switched_account) => {
                    logger::log_info(&format!(
                        "[AutoSwitch][Codex] 自动切号完成: target_id={}, email={}",
                        switched_account.id, switched_account.email
                    ));
                    switched = true;
                }
                Err(e) => {
                    logger::log_warn(&format!(
                        "[AutoSwitch][Codex] 自动切号失败: target_id={}, error={}",
                        target_id, e
                    ));
                }
            }
        }
        Ok(None) => {}
        Err(e) => {
            logger::log_warn(&format!("[AutoSwitch][Codex] 自动切号检查失败: {}", e));
        }
    }

    if !switched {
        if let Err(e) = codex_account::run_quota_alert_if_needed() {
            logger::log_warn(&format!("[QuotaAlert][Codex] 预警检查失败: {}", e));
        }
    }

    CODEX_POST_REFRESH_CHECK_IN_PROGRESS.store(false, Ordering::SeqCst);
}

/// 删除 Codex 账号
#[tauri::command]
pub async fn delete_codex_account(account_id: String) -> Result<(), String> {
    codex_account::remove_account(&account_id)?;
    codex_local_access::remove_deleted_accounts_from_local_access_pool(&[account_id]).await?;
    Ok(())
}

/// 批量删除 Codex 账号
#[tauri::command]
pub async fn delete_codex_accounts(account_ids: Vec<String>) -> Result<(), String> {
    codex_account::remove_accounts(&account_ids)?;
    codex_local_access::remove_deleted_accounts_from_local_access_pool(&account_ids).await?;
    Ok(())
}

#[tauri::command]
pub async fn start_codex_batch_delete(
    account_ids: Vec<String>,
) -> Result<CodexBatchDeleteJobStatus, String> {
    start_codex_batch_delete_job(account_ids)
}

#[tauri::command]
pub async fn get_codex_batch_delete(job_id: String) -> Result<CodexBatchDeleteJobStatus, String> {
    get_codex_batch_delete_job_status(&job_id)
}

#[tauri::command]
pub async fn resume_codex_batch_delete(
    job_id: String,
) -> Result<CodexBatchDeleteJobStatus, String> {
    resume_codex_batch_delete_job(&job_id)
}

#[tauri::command]
pub async fn pause_codex_batch_delete(job_id: String) -> Result<CodexBatchDeleteJobStatus, String> {
    pause_codex_batch_delete_job(&job_id)
}

#[tauri::command]
pub async fn retry_failed_codex_batch_delete(
    job_id: String,
) -> Result<CodexBatchDeleteJobStatus, String> {
    retry_failed_codex_batch_delete_job(&job_id)
}

#[tauri::command]
pub async fn clear_codex_batch_delete(job_id: String) -> Result<(), String> {
    clear_codex_batch_delete_job(&job_id);
    Ok(())
}

async fn refresh_imported_codex_accounts(
    app: &AppHandle,
    accounts: Vec<CodexAccount>,
) -> Vec<CodexAccount> {
    let mut result = Vec::with_capacity(accounts.len());
    let mut success_count = 0;
    let mut attempted = false;

    for account in accounts {
        if account.is_api_key_auth() {
            result.push(account);
            continue;
        }

        attempted = true;
        match codex_quota::refresh_account_quota(&account.id).await {
            Ok(_) => {
                success_count += 1;
            }
            Err(error) => {
                logger::log_warn(&format!(
                    "Codex 导入后刷新配额失败: account_id={}, email={}, error={}",
                    account.id, account.email, error
                ));
            }
        }

        result.push(codex_account::load_account(&account.id).unwrap_or(account));
    }

    if success_count > 0 {
        run_codex_post_refresh_checks(app).await;
    }
    if attempted || !result.is_empty() {
        let _ = crate::modules::tray::update_tray_menu(app);
    }

    result
}

/// 从本地 auth.json 导入账号
#[tauri::command]
pub async fn import_codex_from_local(app: AppHandle) -> Result<CodexAccount, String> {
    let account = codex_account::import_from_local()?;
    let mut accounts = refresh_imported_codex_accounts(&app, vec![account]).await;
    accounts
        .pop()
        .ok_or_else(|| "账号导入后无法读取".to_string())
}

/// 从 JSON 字符串导入账号
#[tauri::command]
pub async fn import_codex_from_json(
    app: AppHandle,
    json_content: String,
) -> Result<Vec<CodexAccount>, String> {
    let accounts = codex_account::import_from_json(&json_content).await?;
    Ok(refresh_imported_codex_accounts(&app, accounts).await)
}

/// 导出 Codex 账号
#[tauri::command]
pub fn export_codex_accounts(account_ids: Vec<String>) -> Result<String, String> {
    codex_account::export_accounts(&account_ids)
}

/// 从本地文件导入 Codex 账号
#[tauri::command]
pub async fn import_codex_from_files(
    app: AppHandle,
    file_paths: Vec<String>,
) -> Result<codex_account::CodexFileImportResult, String> {
    let result = codex_account::import_from_files(file_paths).await?;
    let imported = refresh_imported_codex_accounts(&app, result.imported).await;
    Ok(codex_account::CodexFileImportResult {
        imported,
        failed: result.failed,
    })
}

#[tauri::command]
pub fn start_codex_batch_import_from_files(
    app: AppHandle,
    file_paths: Vec<String>,
    check_quota: bool,
) -> Result<codex_account::CodexBatchImportStartResult, String> {
    codex_account::start_codex_batch_import_from_files(app, file_paths, check_quota)
}

#[tauri::command]
pub fn cancel_codex_batch_import(session_id: String) -> Result<(), String> {
    codex_account::cancel_codex_batch_import(&session_id)
}

#[tauri::command]
pub fn resume_codex_batch_import(app: AppHandle, session_id: String) -> Result<(), String> {
    codex_account::resume_codex_batch_import(app, &session_id)
}

#[tauri::command]
pub fn get_codex_batch_import_preview(
    session_id: String,
) -> Result<codex_account::CodexBatchImportPreview, String> {
    codex_account::get_codex_batch_import_preview(&session_id)
}

#[tauri::command]
pub fn confirm_codex_batch_import(
    session_id: String,
    item_ids: Vec<String>,
) -> Result<codex_account::CodexBatchImportConfirmResult, String> {
    codex_account::confirm_codex_batch_import(&session_id, &item_ids)
}

/// 刷新单个账号配额
#[tauri::command]
pub async fn refresh_codex_quota(app: AppHandle, account_id: String) -> Result<CodexQuota, String> {
    let result = codex_quota::refresh_account_quota(&account_id).await;
    if result.is_ok() {
        run_codex_post_refresh_checks(&app).await;
        let _ = crate::modules::tray::update_tray_menu(&app);
    }
    result
}

#[tauri::command]
pub async fn get_codex_reset_credits(
    account_id: String,
) -> Result<codex_quota::CodexResetCreditsSnapshot, String> {
    codex_quota::fetch_account_reset_credits(&account_id).await
}

#[tauri::command]
pub async fn consume_codex_reset_credit(account_id: String) -> Result<(), String> {
    codex_quota::consume_reset_credit(&account_id).await
}

#[tauri::command]
pub async fn refresh_codex_subscription_info(
    app: AppHandle,
    account_id: String,
) -> Result<CodexAccount, String> {
    let result = codex_quota::refresh_account_subscription_info(&account_id, true).await;
    if result.is_ok() {
        let _ = crate::modules::tray::update_tray_menu(&app);
    }
    result
}

#[tauri::command]
pub async fn refresh_current_codex_quota(app: AppHandle) -> Result<(), String> {
    let Some(account) = codex_account::get_current_account() else {
        return Err("未找到当前 Codex 账号".to_string());
    };
    if account.is_api_key_auth() {
        return Ok(());
    }

    let result = codex_quota::refresh_account_quota(&account.id).await;
    if result.is_ok() {
        run_codex_post_refresh_checks(&app).await;
        let _ = crate::modules::tray::update_tray_menu(&app);
        Ok(())
    } else {
        Err(result
            .err()
            .unwrap_or_else(|| "刷新 Codex 配额失败".to_string()))
    }
}

/// 刷新所有账号配额
#[tauri::command]
pub async fn refresh_all_codex_quotas(app: AppHandle) -> Result<i32, String> {
    let results = codex_quota::refresh_all_quotas().await?;
    let success_count = results.iter().filter(|(_, r)| r.is_ok()).count();
    if success_count > 0 {
        run_codex_post_refresh_checks(&app).await;
    }
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(success_count as i32)
}

async fn save_codex_oauth_tokens(
    tokens: CodexTokens,
    reauth_account_id: Option<&str>,
) -> Result<CodexAccount, String> {
    let account = if let Some(account_id) = reauth_account_id.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }) {
        codex_account::upsert_account_for_reauth(tokens, account_id)?
    } else {
        codex_account::upsert_account(tokens)?
    };

    if let Err(e) = codex_quota::refresh_account_quota(&account.id).await {
        logger::log_error(&format!("刷新配额失败: {}", e));
    }

    let loaded =
        codex_account::load_account(&account.id).ok_or_else(|| "账号保存后无法读取".to_string())?;
    logger::log_info(&format!(
        "Codex OAuth 账号已保存: account_id={}, email={}",
        loaded.id, loaded.email
    ));
    Ok(loaded)
}

/// OAuth：开始登录（返回 loginId + authUrl）
#[tauri::command]
pub async fn codex_oauth_login_start(
    app_handle: AppHandle,
) -> Result<codex_oauth::CodexOAuthLoginStartResponse, String> {
    logger::log_info("Codex OAuth start 命令触发");
    let response = codex_oauth::start_oauth_login(app_handle).await?;
    logger::log_info(&format!(
        "Codex OAuth start 命令成功: login_id={}",
        response.login_id
    ));
    Ok(response)
}

/// OAuth：浏览器授权完成后按 loginId 完成登录
#[tauri::command]
pub async fn codex_oauth_login_completed(
    login_id: String,
    reauth_account_id: Option<String>,
) -> Result<CodexAccount, String> {
    let started_at_ms = chrono::Utc::now().timestamp_millis();
    logger::log_info(&format!(
        "Codex OAuth completed 命令开始: login_id={}, started_at_ms={}",
        login_id, started_at_ms
    ));
    let tokens = match codex_oauth::complete_oauth_login(&login_id).await {
        Ok(tokens) => tokens,
        Err(e) => {
            logger::log_error(&format!(
                "Codex OAuth completed 命令失败: login_id={}, duration_ms={}, error={}",
                login_id,
                chrono::Utc::now().timestamp_millis() - started_at_ms,
                e
            ));
            return Err(e);
        }
    };
    let account = save_codex_oauth_tokens(tokens, reauth_account_id.as_deref()).await?;
    logger::log_info(&format!(
        "Codex OAuth completed 命令成功: login_id={}, duration_ms={}, account_id={}, account_email={}",
        login_id,
        chrono::Utc::now().timestamp_millis() - started_at_ms,
        account.id,
        account.email
    ));
    Ok(account)
}

/// OAuth：按 loginId 取消登录（login_id 为空时取消当前流程）
#[tauri::command]
pub fn codex_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    logger::log_info(&format!(
        "Codex OAuth cancel 命令触发: login_id={}",
        login_id.as_deref().unwrap_or("<none>")
    ));
    let result = codex_oauth::cancel_oauth_flow_for(login_id.as_deref());
    logger::log_info(&format!(
        "Codex OAuth cancel 命令返回: {:?}",
        result.as_ref().map(|_| "ok").map_err(|e| e)
    ));
    result
}

/// OAuth：手动提交回调链接（用于本地端口不可达时）
#[tauri::command]
pub fn codex_oauth_submit_callback_url(
    app_handle: AppHandle,
    login_id: String,
    callback_url: String,
) -> Result<(), String> {
    codex_oauth::submit_callback_url(login_id.as_str(), callback_url.as_str())?;
    let payload = serde_json::json!({ "loginId": login_id });
    let _ = app_handle.emit("codex-oauth-login-completed", payload.clone());
    let _ = app_handle.emit("ghcp-oauth-login-completed", payload);
    Ok(())
}

/// 通过 Token 添加账号
#[tauri::command]
pub async fn add_codex_account_with_token(
    id_token: String,
    access_token: String,
    refresh_token: Option<String>,
) -> Result<CodexAccount, String> {
    let tokens = CodexTokens {
        id_token,
        access_token,
        refresh_token,
    };

    let account = codex_account::upsert_account(tokens)?;

    // 刷新配额
    if let Err(e) = codex_quota::refresh_account_quota(&account.id).await {
        logger::log_error(&format!("刷新配额失败: {}", e));
    }

    codex_account::load_account(&account.id).ok_or_else(|| "账号保存后无法读取".to_string())
}

/// 通过 API Key 添加账号
#[tauri::command]
pub fn add_codex_account_with_api_key(
    api_key: String,
    api_base_url: Option<String>,
    api_provider_mode: Option<CodexApiProviderMode>,
    api_provider_id: Option<String>,
    api_provider_name: Option<String>,
    api_model_catalog: Option<Vec<String>>,
    api_wire_api: Option<String>,
    api_supports_vision: Option<bool>,
    api_model_vision_support: Option<std::collections::HashMap<String, bool>>,
    api_vision_routing_model: Option<String>,
    account_name: Option<String>,
) -> Result<CodexAccount, String> {
    let account = codex_account::upsert_api_key_account(
        api_key,
        api_base_url,
        api_provider_mode,
        api_provider_id,
        api_provider_name,
        api_model_catalog.unwrap_or_default(),
        api_wire_api,
        api_supports_vision.unwrap_or(false),
        api_model_vision_support.unwrap_or_default(),
        api_vision_routing_model,
        account_name,
    )?;
    codex_account::load_account(&account.id).ok_or_else(|| "账号保存后无法读取".to_string())
}

#[tauri::command]
pub fn update_codex_account_name(account_id: String, name: String) -> Result<CodexAccount, String> {
    codex_account::update_account_name(&account_id, name)
}

#[tauri::command]
pub fn update_codex_api_key_credentials(
    account_id: String,
    api_key: String,
    api_base_url: Option<String>,
    api_provider_mode: Option<CodexApiProviderMode>,
    api_provider_id: Option<String>,
    api_provider_name: Option<String>,
    api_model_catalog: Option<Vec<String>>,
    api_wire_api: Option<String>,
    api_supports_vision: Option<bool>,
    api_model_vision_support: Option<std::collections::HashMap<String, bool>>,
    api_vision_routing_model: Option<String>,
) -> Result<CodexAccount, String> {
    codex_account::update_api_key_credentials(
        &account_id,
        api_key,
        api_base_url,
        api_provider_mode,
        api_provider_id,
        api_provider_name,
        api_model_catalog.unwrap_or_default(),
        api_wire_api,
        api_supports_vision.unwrap_or(false),
        api_model_vision_support.unwrap_or_default(),
        api_vision_routing_model,
    )
}

#[tauri::command]
pub async fn update_codex_api_key_bound_oauth_account(
    account_id: String,
    bound_oauth_account_id: Option<String>,
    bound_oauth_use_local_gateway: Option<bool>,
) -> Result<CodexAccount, String> {
    codex_account::update_api_key_bound_oauth_account(
        &account_id,
        bound_oauth_account_id,
        bound_oauth_use_local_gateway.unwrap_or(false),
    )
    .await
}

#[tauri::command]
pub async fn update_codex_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<CodexAccount, String> {
    codex_account::update_account_tags(&account_id, tags)
}

#[tauri::command]
pub async fn update_codex_account_note(
    account_id: String,
    note: Option<String>,
    two_factor_secret: Option<String>,
    account_password: Option<String>,
    phone_number: Option<String>,
    mail_url: Option<String>,
) -> Result<CodexAccount, String> {
    codex_account::update_account_note(
        &account_id,
        codex_account::CodexAccountNoteUpdate {
            note,
            two_factor_secret,
            account_password,
            phone_number,
            mail_url,
        },
    )
}

#[tauri::command]
pub async fn create_pending_codex_oauth_account(
    email: String,
    note: Option<String>,
    two_factor_secret: Option<String>,
    account_password: Option<String>,
    phone_number: Option<String>,
    mail_url: Option<String>,
) -> Result<CodexAccount, String> {
    codex_account::create_pending_oauth_account(
        email,
        codex_account::CodexAccountNoteUpdate {
            note,
            two_factor_secret,
            account_password,
            phone_number,
            mail_url,
        },
    )
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexMailPreviewFetchResult {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: String,
    pub truncated: bool,
}

#[tauri::command]
pub async fn fetch_codex_account_note_mail_url(
    mail_url: String,
) -> Result<CodexMailPreviewFetchResult, String> {
    let mail_url = mail_url.trim();
    if mail_url.is_empty() {
        return Err("MAIL_URL_EMPTY".to_string());
    }
    let parsed = reqwest::Url::parse(mail_url)
        .map_err(|_| "MAIL_URL_INVALID".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("MAIL_URL_UNSUPPORTED_SCHEME".to_string());
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent("CockpitTools-MailPreview/1.0")
        .build()
        .map_err(|e| format!("MAIL_PREVIEW_CLIENT_FAILED: {}", e))?;
    let response = client
        .get(parsed)
        .send()
        .await
        .map_err(|e| format!("MAIL_PREVIEW_REQUEST_FAILED: {}", e))?;
    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("MAIL_PREVIEW_READ_FAILED: {}", e))?;
    let truncated = bytes.len() > CODEX_MAIL_PREVIEW_MAX_BYTES;
    let visible_bytes = if truncated {
        &bytes[..CODEX_MAIL_PREVIEW_MAX_BYTES]
    } else {
        &bytes[..]
    };
    let body = String::from_utf8_lossy(visible_bytes).into_owned();

    if !status.is_success() {
        return Err(format!("MAIL_PREVIEW_HTTP_FAILED:{}", status.as_u16()));
    }

    Ok(CodexMailPreviewFetchResult {
        status: status.as_u16(),
        content_type,
        body,
        truncated,
    })
}

/// 检查 Codex OAuth 端口是否被占用
#[tauri::command]
pub fn is_codex_oauth_port_in_use() -> Result<bool, String> {
    let port = codex_oauth::get_callback_port();
    process::is_port_in_use(port)
}

/// 关闭占用 Codex OAuth 端口的进程
#[tauri::command]
pub fn close_codex_oauth_port() -> Result<u32, String> {
    let port = codex_oauth::get_callback_port();
    let killed = process::kill_port_processes(port)?;
    Ok(killed as u32)
}

#[tauri::command]
pub fn codex_wakeup_get_cli_status() -> Result<codex_wakeup::CodexCliStatus, String> {
    Ok(codex_wakeup::wakeup_runtime_status())
}

#[tauri::command]
pub fn codex_wakeup_update_runtime_config(
    codex_cli_path: Option<String>,
    node_path: Option<String>,
) -> Result<codex_wakeup::CodexCliStatus, String> {
    codex_wakeup::save_runtime_config(&codex_wakeup::CodexWakeupRuntimeConfig {
        codex_cli_path,
        node_path,
    })?;
    Ok(codex_wakeup::wakeup_runtime_status())
}

#[tauri::command]
pub fn codex_wakeup_get_overview() -> Result<codex_wakeup::CodexWakeupOverview, String> {
    codex_wakeup::load_overview()
}

#[tauri::command]
pub fn codex_wakeup_get_state() -> Result<codex_wakeup::CodexWakeupState, String> {
    codex_wakeup::load_state()
}

#[tauri::command]
pub fn codex_wakeup_save_state(
    enabled: bool,
    tasks: Vec<codex_wakeup::CodexWakeupTask>,
    model_presets: Vec<codex_wakeup::CodexWakeupModelPreset>,
    model_preset_migrations: Vec<String>,
) -> Result<codex_wakeup::CodexWakeupState, String> {
    codex_wakeup::save_state(&codex_wakeup::CodexWakeupState {
        enabled,
        tasks,
        model_presets,
        model_preset_migrations,
    })
}

#[tauri::command]
pub fn codex_wakeup_load_history() -> Result<Vec<codex_wakeup::CodexWakeupHistoryItem>, String> {
    codex_wakeup::load_history()
}

#[tauri::command]
pub fn codex_wakeup_clear_history() -> Result<(), String> {
    codex_wakeup::clear_history()
}

#[tauri::command]
pub fn codex_wakeup_cancel_scope(cancel_scope_id: String) -> Result<(), String> {
    codex_wakeup::cancel_wakeup_scope(&cancel_scope_id)
}

#[tauri::command]
pub fn codex_wakeup_release_scope(cancel_scope_id: String) -> Result<(), String> {
    codex_wakeup::release_wakeup_scope(&cancel_scope_id)
}

#[tauri::command]
pub async fn codex_wakeup_test(
    app: AppHandle,
    account_ids: Vec<String>,
    prompt: Option<String>,
    model: Option<String>,
    model_display_name: Option<String>,
    model_reasoning_effort: Option<String>,
    run_id: Option<String>,
    cancel_scope_id: Option<String>,
) -> Result<codex_wakeup::CodexWakeupBatchResult, String> {
    codex_wakeup::run_batch(
        Some(&app),
        account_ids,
        prompt,
        codex_wakeup::CodexWakeupExecutionConfig {
            model,
            model_display_name,
            model_reasoning_effort,
        },
        codex_wakeup::TaskRunContext {
            trigger_type: "test".to_string(),
            task_id: None,
            task_name: None,
        },
        run_id,
        cancel_scope_id.as_deref(),
    )
    .await
}

#[tauri::command]
pub async fn codex_wakeup_run_task(
    app: AppHandle,
    task_id: String,
    run_id: Option<String>,
) -> Result<codex_wakeup::CodexWakeupBatchResult, String> {
    codex_wakeup_scheduler::run_task_now(Some(&app), &task_id, "manual_task", run_id).await
}

#[tauri::command]
pub async fn codex_wakeup_run_enabled_tasks(
    app: AppHandle,
    trigger_type: Option<String>,
) -> Result<u32, String> {
    let trigger = trigger_type.unwrap_or_else(|| "startup".to_string());
    codex_wakeup_scheduler::run_enabled_tasks_now(Some(&app), &trigger).await
}

// ─── Codex 账号分组持久化 ────────────────────────────────────────────

const CODEX_GROUPS_FILE: &str = "codex_account_groups.json";
const CODEX_MODEL_PROVIDERS_FILE: &str = "codex_model_providers.json";
const CODEX_MODEL_PROVIDER_TEST_TIMEOUT_SECS: u64 = 20;

#[tauri::command]
pub async fn load_codex_account_groups() -> Result<String, String> {
    let path = account::get_data_dir()?.join(CODEX_GROUPS_FILE);
    if !path.exists() {
        return Ok("[]".to_string());
    }
    std::fs::read_to_string(&path).map_err(|e| format!("Failed to read codex groups: {}", e))
}

#[tauri::command]
pub async fn save_codex_account_groups(data: String) -> Result<(), String> {
    let dir = account::get_data_dir()?;
    if !dir.exists() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create dir: {}", e))?;
    }
    let path = dir.join(CODEX_GROUPS_FILE);
    std::fs::write(&path, data).map_err(|e| format!("Failed to write codex groups: {}", e))
}

#[tauri::command]
pub async fn load_codex_model_providers() -> Result<String, String> {
    let path = account::get_data_dir()?.join(CODEX_MODEL_PROVIDERS_FILE);
    if !path.exists() {
        return Ok("[]".to_string());
    }
    std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read codex model providers: {}", e))
}

#[tauri::command]
pub async fn save_codex_model_providers(data: String) -> Result<(), String> {
    let dir = account::get_data_dir()?;
    if !dir.exists() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create dir: {}", e))?;
    }
    let path = dir.join(CODEX_MODEL_PROVIDERS_FILE);
    std::fs::write(&path, data).map_err(|e| format!("Failed to write codex model providers: {}", e))
}

fn codex_model_provider_models_url(base_url: &str) -> Result<String, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("PROVIDER_BASE_URL_INVALID".to_string());
    }
    let mut url =
        reqwest::Url::parse(trimmed).map_err(|_| "PROVIDER_BASE_URL_INVALID".to_string())?;
    match url.scheme() {
        "http" | "https" => {}
        _ => return Err("PROVIDER_BASE_URL_INVALID".to_string()),
    }
    let next_path = if url.path().is_empty() || url.path() == "/" {
        "/models".to_string()
    } else {
        format!("{}/models", url.path().trim_end_matches('/'))
    };
    url.set_path(&next_path);
    url.set_query(None);
    Ok(url.to_string())
}

fn codex_model_provider_usage_url(base_url: &str) -> Result<String, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("PROVIDER_BASE_URL_INVALID".to_string());
    }
    let mut url =
        reqwest::Url::parse(trimmed).map_err(|_| "PROVIDER_BASE_URL_INVALID".to_string())?;
    match url.scheme() {
        "http" | "https" => {}
        _ => return Err("PROVIDER_BASE_URL_INVALID".to_string()),
    }
    let next_path = if url.path().is_empty() || url.path() == "/" {
        "/usage".to_string()
    } else {
        format!("{}/usage", url.path().trim_end_matches('/'))
    };
    url.set_path(&next_path);
    url.set_query(None);
    Ok(url.to_string())
}

fn codex_model_provider_new_api_billing_url(
    base_url: &str,
    endpoint: &str,
) -> Result<String, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("PROVIDER_BASE_URL_INVALID".to_string());
    }
    let mut url =
        reqwest::Url::parse(trimmed).map_err(|_| "PROVIDER_BASE_URL_INVALID".to_string())?;
    match url.scheme() {
        "http" | "https" => {}
        _ => return Err("PROVIDER_BASE_URL_INVALID".to_string()),
    }
    let base_path = url.path().trim_end_matches('/');
    let next_path = if base_path.is_empty() {
        format!("/{}", endpoint.trim_start_matches('/'))
    } else {
        format!("{}/{}", base_path, endpoint.trim_start_matches('/'))
    };
    url.set_path(&next_path);
    url.set_query(None);
    Ok(url.to_string())
}

fn codex_model_provider_new_api_api_url(base_url: &str, endpoint: &str) -> Result<String, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("PROVIDER_BASE_URL_INVALID".to_string());
    }
    let mut url =
        reqwest::Url::parse(trimmed).map_err(|_| "PROVIDER_BASE_URL_INVALID".to_string())?;
    match url.scheme() {
        "http" | "https" => {}
        _ => return Err("PROVIDER_BASE_URL_INVALID".to_string()),
    }
    let mut base_path = url.path().trim_end_matches('/').to_string();
    if base_path == "/v1" {
        base_path.clear();
    }
    let next_path = if base_path.is_empty() {
        format!("/{}", endpoint.trim_start_matches('/'))
    } else {
        format!("{}/{}", base_path, endpoint.trim_start_matches('/'))
    };
    url.set_path(&next_path);
    url.set_query(None);
    Ok(url.to_string())
}

fn codex_model_provider_failure(
    title: &str,
    stage: &str,
    cause: String,
    suggestion: &str,
    status: Option<u16>,
    detail: Option<String>,
) -> CodexLocalAccessTestResult {
    CodexLocalAccessTestResult {
        model_id: None,
        latency_ms: None,
        output: None,
        failure: Some(CodexLocalAccessTestFailure {
            title: title.to_string(),
            stage: stage.to_string(),
            cause,
            suggestion: suggestion.to_string(),
            status,
            model_id: None,
            detail,
            gateway_output: None,
        }),
    }
}

const CODEX_MODEL_PROVIDER_CHAT_TEST_PROGRESS_EVENT: &str = "codex://model-provider-test-progress";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexModelProviderChatTestTarget {
    pub provider_id: String,
    pub provider_name: String,
    pub base_url: String,
    pub api_key_id: Option<String>,
    pub api_key_name: Option<String>,
    pub api_key: String,
    pub wire_api: Option<String>,
    #[serde(default)]
    pub model_catalog: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexModelProviderChatTestRecord {
    pub provider_id: String,
    pub provider_name: String,
    pub api_key_id: Option<String>,
    pub api_key_name: Option<String>,
    pub wire_api: String,
    pub access_mode: String,
    pub model_id: Option<String>,
    pub success: bool,
    pub prompt: String,
    pub reply: Option<String>,
    pub error: Option<String>,
    pub duration_ms: Option<u64>,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexModelProviderChatTestBatchResult {
    pub run_id: String,
    pub records: Vec<CodexModelProviderChatTestRecord>,
    pub success_count: usize,
    pub failure_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexModelProviderChatTestProgressPayload {
    pub run_id: String,
    pub total: usize,
    pub completed: usize,
    pub success_count: usize,
    pub failure_count: usize,
    pub running: bool,
    pub phase: String,
    pub current_provider_id: Option<String>,
    pub item: Option<CodexModelProviderChatTestRecord>,
}

fn emit_model_provider_chat_test_progress(
    app: &AppHandle,
    run_id: &str,
    total: usize,
    completed: usize,
    success_count: usize,
    failure_count: usize,
    running: bool,
    phase: &str,
    current_provider_id: Option<&str>,
    item: Option<CodexModelProviderChatTestRecord>,
) {
    let payload = CodexModelProviderChatTestProgressPayload {
        run_id: run_id.to_string(),
        total,
        completed,
        success_count,
        failure_count,
        running,
        phase: phase.to_string(),
        current_provider_id: current_provider_id.map(ToOwned::to_owned),
        item,
    };
    let _ = app.emit(CODEX_MODEL_PROVIDER_CHAT_TEST_PROGRESS_EVENT, payload);
}

fn normalize_model_provider_wire_api(value: Option<&str>, base_url: &str) -> String {
    match value.map(str::trim) {
        Some("chat_completions") => return "chat_completions".to_string(),
        Some("responses") => return "responses".to_string(),
        _ => {}
    }
    let lower = base_url.trim().to_ascii_lowercase();
    if lower.contains("/chat/completions")
        || lower.contains("api.deepseek.com")
        || lower.contains("api.moonshot.cn")
        || lower.contains("api.siliconflow.cn")
        || lower.contains("api.siliconflow.com")
        || lower.contains("open.bigmodel.cn")
        || lower.contains("api.z.ai")
        || lower.contains("volces.com")
        || lower.contains("bytepluses.com")
        || lower.contains("qianfan.baidubce.com")
        || lower.contains("dashscope.aliyuncs.com")
        || lower.contains("api.stepfun.com")
        || lower.contains("api.stepfun.ai")
        || lower.contains("modelscope.cn")
        || lower.contains("api.longcat.chat")
        || lower.contains("api.minimax.io")
        || lower.contains("api.mini-max.chat")
        || lower.contains("api.minimaxi.com")
        || lower.contains("api.mimo.dev")
        || lower.contains("token-plan-cn.xiaomimimo.com")
        || lower.contains("api.novita.ai")
        || lower.contains("integrate.api.nvidia.com")
        || lower.contains("runapi.co")
        || lower.contains("relaxycode.com")
        || lower.contains("compshare.cn")
        || lower.contains("api.lemondata.cc")
        || lower.contains("e-flowcode.cc")
        || lower.contains("cc-api.pipellm.ai")
        || lower.contains("openrouter.ai")
        || lower.contains("api.therouter.ai")
    {
        "chat_completions".to_string()
    } else {
        "responses".to_string()
    }
}

const RESPONSES_NATIVE_CHAT_TEST_MODEL_PRIORITY: &[&str] =
    &["gpt-5.5", "gpt-5.4", "gpt-5", "gpt-4.1", "gpt-4o"];

fn is_image_generation_model_id(model_id: &str) -> bool {
    let lower = model_id.trim().to_ascii_lowercase();
    lower.starts_with("gpt-image") || lower.starts_with("dall-e") || lower.contains("image-gen")
}

fn first_non_empty_model_id(models: &[String]) -> Option<String> {
    models
        .iter()
        .map(|item| item.trim())
        .find(|item| !item.is_empty())
        .map(ToOwned::to_owned)
}

fn select_model_provider_chat_test_model(
    wire_api: &str,
    explicit_model: Option<&str>,
    model_catalog: &[String],
) -> Option<String> {
    if let Some(model) = explicit_model
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(model.to_string());
    }

    if wire_api.trim() == "responses" {
        for preferred in RESPONSES_NATIVE_CHAT_TEST_MODEL_PRIORITY {
            if let Some(model) = model_catalog
                .iter()
                .map(|item| item.trim())
                .find(|item| item.eq_ignore_ascii_case(preferred))
            {
                return Some(model.to_string());
            }
        }
        if let Some(model) = model_catalog
            .iter()
            .map(|item| item.trim())
            .find(|item| !item.is_empty() && !is_image_generation_model_id(item))
        {
            return Some(model.to_string());
        }
    }

    first_non_empty_model_id(model_catalog)
}

fn model_ids_from_provider_models(body: &serde_json::Value) -> Vec<String> {
    body.get("data")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("id").and_then(|id| id.as_str()))
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn first_model_from_provider_models(body: &serde_json::Value, wire_api: &str) -> Option<String> {
    let models = model_ids_from_provider_models(body);
    select_model_provider_chat_test_model(wire_api, None, &models)
}

async fn discover_model_provider_model(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    wire_api: &str,
) -> Option<String> {
    let url = codex_model_provider_models_url(base_url).ok()?;
    let response = client
        .get(url)
        .bearer_auth(api_key.trim())
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let text = response.text().await.ok()?;
    let parsed = serde_json::from_str::<serde_json::Value>(&text).ok()?;
    first_model_from_provider_models(&parsed, wire_api)
}

async fn run_single_model_provider_chat_test(
    client: &reqwest::Client,
    target: CodexModelProviderChatTestTarget,
    prompt: &str,
    model: Option<&str>,
    run_id: &str,
) -> CodexModelProviderChatTestRecord {
    let wire_api = normalize_model_provider_wire_api(target.wire_api.as_deref(), &target.base_url);
    let access_mode = "gateway".to_string();
    let timestamp = chrono::Utc::now().timestamp_millis();
    let api_key = target.api_key.trim().to_string();
    if api_key.is_empty() {
        return CodexModelProviderChatTestRecord {
            provider_id: target.provider_id,
            provider_name: target.provider_name,
            api_key_id: target.api_key_id,
            api_key_name: target.api_key_name,
            wire_api,
            access_mode,
            model_id: None,
            success: false,
            prompt: prompt.to_string(),
            reply: None,
            error: Some("供应商缺少 API Key".to_string()),
            duration_ms: None,
            timestamp,
        };
    }
    let configured_model_id =
        select_model_provider_chat_test_model(&wire_api, model, &target.model_catalog);
    let model_id = match configured_model_id {
        Some(model_id) => Some(model_id),
        None => discover_model_provider_model(client, &target.base_url, &api_key, &wire_api).await,
    };
    let Some(model_id) = model_id else {
        return CodexModelProviderChatTestRecord {
            provider_id: target.provider_id,
            provider_name: target.provider_name,
            api_key_id: target.api_key_id,
            api_key_name: target.api_key_name,
            wire_api,
            access_mode,
            model_id: None,
            success: false,
            prompt: prompt.to_string(),
            reply: None,
            error: Some("无法确定测试模型，请先配置模型目录或确认 /models 可用".to_string()),
            duration_ms: None,
            timestamp,
        };
    };

    let result = codex_local_access::run_model_provider_gateway_chat_test(
        codex_local_access::CodexModelProviderGatewayChatTestRequest {
            run_id: run_id.to_string(),
            provider_id: target.provider_id.clone(),
            provider_name: target.provider_name.clone(),
            base_url: target.base_url.clone(),
            api_key_id: target.api_key_id.clone(),
            api_key_name: target.api_key_name.clone(),
            api_key,
            wire_api: wire_api.clone(),
            model_catalog: target.model_catalog.clone(),
            model_id: model_id.clone(),
            prompt: prompt.to_string(),
        },
    )
    .await
    .map(|result| (result.duration_ms, result.reply));

    match result {
        Ok((duration_ms, reply)) => CodexModelProviderChatTestRecord {
            provider_id: target.provider_id,
            provider_name: target.provider_name,
            api_key_id: target.api_key_id,
            api_key_name: target.api_key_name,
            wire_api,
            access_mode,
            model_id: Some(model_id),
            success: true,
            prompt: prompt.to_string(),
            reply: Some(reply),
            error: None,
            duration_ms: Some(duration_ms),
            timestamp,
        },
        Err(error) => CodexModelProviderChatTestRecord {
            provider_id: target.provider_id,
            provider_name: target.provider_name,
            api_key_id: target.api_key_id,
            api_key_name: target.api_key_name,
            wire_api,
            access_mode,
            model_id: Some(model_id),
            success: false,
            prompt: prompt.to_string(),
            reply: None,
            error: Some(error),
            duration_ms: None,
            timestamp,
        },
    }
}

fn summarize_model_provider_models(body: &serde_json::Value) -> (Option<String>, Option<String>) {
    let ids: Vec<String> = body
        .get("data")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("id").and_then(|id| id.as_str()))
                .take(8)
                .map(|id| id.to_string())
                .collect()
        })
        .unwrap_or_default();
    let first = ids.first().cloned();
    let output = if ids.is_empty() {
        None
    } else {
        Some(ids.join(", "))
    };
    (first, output)
}

fn list_model_provider_models(body: &serde_json::Value) -> Vec<CodexModelProviderModel> {
    let mut seen = std::collections::HashSet::new();
    body.get("data")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let id = item.get("id").and_then(|id| id.as_str())?.trim();
                    if id.is_empty() {
                        return None;
                    }
                    let key = id.to_ascii_lowercase();
                    if !seen.insert(key) {
                        return None;
                    }
                    Some(CodexModelProviderModel {
                        id: id.to_string(),
                        display_name: item
                            .get("display_name")
                            .or_else(|| item.get("displayName"))
                            .and_then(|value| value.as_str())
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexModelProviderUsageDetail {
    pub key: String,
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexModelProviderModel {
    pub id: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexModelProviderModelsResult {
    pub models: Vec<CodexModelProviderModel>,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexModelProviderUsageSummary {
    pub mode: Option<String>,
    pub is_valid: Option<bool>,
    pub status: Option<String>,
    pub plan_name: Option<String>,
    pub remaining: Option<f64>,
    pub balance: Option<f64>,
    pub unit: Option<String>,
    pub quota_unlimited: Option<bool>,
    pub quota_limit: Option<f64>,
    pub quota_used: Option<f64>,
    pub quota_remaining: Option<f64>,
    pub today_requests: Option<i64>,
    pub today_total_tokens: Option<i64>,
    pub today_cost: Option<f64>,
    pub total_requests: Option<i64>,
    pub total_total_tokens: Option<i64>,
    pub total_cost: Option<f64>,
    pub model_stats_count: usize,
    pub latency_ms: u64,
    pub details: Vec<CodexModelProviderUsageDetail>,
}

fn json_f64_at(value: &serde_json::Value, path: &[&str]) -> Option<f64> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_f64()
}

fn json_i64_at(value: &serde_json::Value, path: &[&str]) -> Option<i64> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_i64()
}

fn json_string_at(value: &serde_json::Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str().map(|item| item.to_string())
}

fn json_bool_at(value: &serde_json::Value, path: &[&str]) -> Option<bool> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_bool()
}

fn summarize_model_provider_usage(
    body: &serde_json::Value,
    latency_ms: u64,
) -> CodexModelProviderUsageSummary {
    let model_stats_count = body
        .get("model_stats")
        .and_then(|value| value.as_array())
        .map(|items| items.len())
        .unwrap_or(0);
    let mut details = Vec::new();
    push_usage_detail(
        &mut details,
        "mode",
        "Mode",
        json_string_at(body, &["mode"]),
    );
    push_usage_detail(
        &mut details,
        "status",
        "Status",
        json_string_at(body, &["status"]),
    );
    push_usage_detail(
        &mut details,
        "planName",
        "Plan",
        json_string_at(body, &["planName"]),
    );
    push_usage_detail(
        &mut details,
        "remaining",
        "Remaining",
        json_f64_at(body, &["remaining"]).map(format_usage_number),
    );
    push_usage_detail(
        &mut details,
        "balance",
        "Balance",
        json_f64_at(body, &["balance"]).map(format_usage_number),
    );
    push_usage_detail(
        &mut details,
        "todayRequests",
        "Today Requests",
        json_i64_at(body, &["usage", "today", "requests"]).map(|value| value.to_string()),
    );
    push_usage_detail(
        &mut details,
        "todayTokens",
        "Today Tokens",
        json_i64_at(body, &["usage", "today", "total_tokens"]).map(|value| value.to_string()),
    );
    push_usage_detail(
        &mut details,
        "todayCost",
        "Today Cost",
        json_f64_at(body, &["usage", "today", "cost"]).map(format_usage_number),
    );
    push_usage_detail(
        &mut details,
        "totalRequests",
        "Total Requests",
        json_i64_at(body, &["usage", "total", "requests"]).map(|value| value.to_string()),
    );
    push_usage_detail(
        &mut details,
        "totalTokens",
        "Total Tokens",
        json_i64_at(body, &["usage", "total", "total_tokens"]).map(|value| value.to_string()),
    );
    push_usage_detail(
        &mut details,
        "totalCost",
        "Total Cost",
        json_f64_at(body, &["usage", "total", "cost"]).map(format_usage_number),
    );

    CodexModelProviderUsageSummary {
        mode: json_string_at(body, &["mode"]),
        is_valid: json_bool_at(body, &["is_active"]).or_else(|| json_bool_at(body, &["isValid"])),
        status: json_string_at(body, &["status"]),
        plan_name: json_string_at(body, &["planName"]),
        remaining: json_f64_at(body, &["remaining"]),
        balance: json_f64_at(body, &["balance"]),
        unit: json_string_at(body, &["unit"]).or_else(|| json_string_at(body, &["quota", "unit"])),
        quota_unlimited: json_bool_at(body, &["quota", "unlimited"]),
        quota_limit: json_f64_at(body, &["quota", "limit"]),
        quota_used: json_f64_at(body, &["quota", "used"]),
        quota_remaining: json_f64_at(body, &["quota", "remaining"]),
        today_requests: json_i64_at(body, &["usage", "today", "requests"]),
        today_total_tokens: json_i64_at(body, &["usage", "today", "total_tokens"]),
        today_cost: json_f64_at(body, &["usage", "today", "cost"]),
        total_requests: json_i64_at(body, &["usage", "total", "requests"]),
        total_total_tokens: json_i64_at(body, &["usage", "total", "total_tokens"]),
        total_cost: json_f64_at(body, &["usage", "total", "cost"]),
        model_stats_count,
        latency_ms,
        details,
    }
}

fn format_usage_number(value: f64) -> String {
    if value.fract().abs() < f64::EPSILON {
        format!("{:.0}", value)
    } else {
        format!("{:.4}", value)
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    }
}

fn push_usage_detail(
    details: &mut Vec<CodexModelProviderUsageDetail>,
    key: &str,
    label: &str,
    value: Option<String>,
) {
    let Some(value) = value else {
        return;
    };
    if value.trim().is_empty() {
        return;
    }
    details.push(CodexModelProviderUsageDetail {
        key: key.to_string(),
        label: label.to_string(),
        value,
    });
}

fn summarize_new_api_model_provider_usage(
    subscription: &serde_json::Value,
    usage: &serde_json::Value,
    token_usage: Option<&serde_json::Value>,
    latency_ms: u64,
) -> CodexModelProviderUsageSummary {
    let raw_quota_limit = json_f64_at(subscription, &["hard_limit_usd"])
        .or_else(|| json_f64_at(subscription, &["soft_limit_usd"]))
        .or_else(|| json_f64_at(subscription, &["system_hard_limit_usd"]));
    let quota_used = json_f64_at(usage, &["total_usage"]).map(|value| value / 100.0);
    let token_data = token_usage.and_then(|value| value.get("data"));
    let quota_unlimited = token_data
        .and_then(|value| json_bool_at(value, &["unlimited_quota"]))
        .unwrap_or_else(|| {
            let hard = json_f64_at(subscription, &["hard_limit_usd"]);
            let soft = json_f64_at(subscription, &["soft_limit_usd"]);
            let system = json_f64_at(subscription, &["system_hard_limit_usd"]);
            matches!(
                (hard, soft, system),
                (Some(h), Some(s), Some(sys))
                    if (h - 100_000_000.0).abs() < f64::EPSILON
                        && (s - 100_000_000.0).abs() < f64::EPSILON
                        && (sys - 100_000_000.0).abs() < f64::EPSILON
            )
        });
    let quota_limit = if quota_unlimited {
        None
    } else {
        raw_quota_limit
    };
    let quota_remaining = match (quota_limit, quota_used) {
        (Some(limit), Some(used)) => Some((limit - used).max(0.0)),
        _ => None,
    };
    let mut details = Vec::new();
    push_usage_detail(
        &mut details,
        "hardLimitUsd",
        "Hard Limit USD",
        json_f64_at(subscription, &["hard_limit_usd"]).map(format_usage_number),
    );
    push_usage_detail(
        &mut details,
        "softLimitUsd",
        "Soft Limit USD",
        json_f64_at(subscription, &["soft_limit_usd"]).map(format_usage_number),
    );
    push_usage_detail(
        &mut details,
        "systemHardLimitUsd",
        "System Hard Limit USD",
        json_f64_at(subscription, &["system_hard_limit_usd"]).map(format_usage_number),
    );
    push_usage_detail(
        &mut details,
        "accessUntil",
        "Access Until",
        json_i64_at(subscription, &["access_until"]).map(|value| value.to_string()),
    );
    push_usage_detail(
        &mut details,
        "quotaUnlimited",
        "Unlimited Quota",
        Some(quota_unlimited.to_string()),
    );
    if let Some(token_data) = token_data {
        push_usage_detail(
            &mut details,
            "totalGranted",
            "Total Granted",
            json_f64_at(token_data, &["total_granted"]).map(format_usage_number),
        );
        push_usage_detail(
            &mut details,
            "totalAvailable",
            "Total Available",
            json_f64_at(token_data, &["total_available"]).map(format_usage_number),
        );
        push_usage_detail(
            &mut details,
            "expiresAt",
            "Expires At",
            json_i64_at(token_data, &["expires_at"]).map(|value| value.to_string()),
        );
        push_usage_detail(
            &mut details,
            "modelLimitsEnabled",
            "Model Limits",
            json_bool_at(token_data, &["model_limits_enabled"]).map(|value| value.to_string()),
        );
    }
    push_usage_detail(
        &mut details,
        "totalUsage",
        "Total Usage",
        json_f64_at(usage, &["total_usage"]).map(format_usage_number),
    );

    CodexModelProviderUsageSummary {
        mode: Some("new_api".to_string()),
        is_valid: None,
        status: None,
        plan_name: None,
        remaining: quota_remaining,
        balance: None,
        unit: Some("USD".to_string()),
        quota_unlimited: Some(quota_unlimited),
        quota_limit,
        quota_used,
        quota_remaining,
        today_requests: None,
        today_total_tokens: None,
        today_cost: None,
        total_requests: None,
        total_total_tokens: None,
        total_cost: quota_used,
        model_stats_count: 0,
        latency_ms,
        details,
    }
}

#[tauri::command]
pub async fn codex_test_model_provider_connection(
    base_url: String,
    api_key: String,
    wire_api: Option<String>,
) -> Result<CodexLocalAccessTestResult, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Ok(codex_model_provider_failure(
            "missing_api_key",
            "credential",
            "MISSING_API_KEY".to_string(),
            "add_api_key",
            None,
            None,
        ));
    }

    let url = match codex_model_provider_models_url(&base_url) {
        Ok(url) => url,
        Err(error) => {
            return Ok(codex_model_provider_failure(
                "invalid_base_url",
                "url",
                error,
                "check_base_url",
                None,
                None,
            ));
        }
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(CODEX_MODEL_PROVIDER_TEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("CREATE_HTTP_CLIENT_FAILED: {}", e))?;
    let started = Instant::now();
    let response = match client
        .get(&url)
        .bearer_auth(key)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            return Ok(codex_model_provider_failure(
                "network_failed",
                "network",
                error.to_string(),
                "check_network",
                None,
                Some(format!("GET {}", url)),
            ));
        }
    };
    let latency_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
    let status = response.status();
    let text = response.text().await.unwrap_or_default();

    if !status.is_success() {
        let suggestion = if status == reqwest::StatusCode::UNAUTHORIZED
            || status == reqwest::StatusCode::FORBIDDEN
        {
            "check_api_key"
        } else if status == reqwest::StatusCode::NOT_FOUND {
            "check_base_url"
        } else {
            "check_provider_status"
        };
        return Ok(codex_model_provider_failure(
            "provider_http_failed",
            "models",
            "HTTP_STATUS".to_string(),
            suggestion,
            Some(status.as_u16()),
            Some(text.chars().take(1000).collect()),
        ));
    }

    let parsed = match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(value) => value,
        Err(error) => {
            return Ok(codex_model_provider_failure(
                "response_parse_failed",
                "parse",
                error.to_string(),
                "check_openai_compatible_models",
                Some(status.as_u16()),
                Some(text.chars().take(1000).collect()),
            ));
        }
    };
    let (model_id, output) = summarize_model_provider_models(&parsed);
    let protocol = wire_api.unwrap_or_else(|| "auto".to_string());
    Ok(CodexLocalAccessTestResult {
        model_id,
        latency_ms: Some(latency_ms),
        output: output.or_else(|| Some(format!("{} connection ok", protocol))),
        failure: None,
    })
}

#[tauri::command]
pub async fn codex_model_provider_chat_test_batch(
    app: AppHandle,
    targets: Vec<CodexModelProviderChatTestTarget>,
    prompt: Option<String>,
    model: Option<String>,
    run_id: Option<String>,
) -> Result<CodexModelProviderChatTestBatchResult, String> {
    let cleaned_targets: Vec<CodexModelProviderChatTestTarget> = targets
        .into_iter()
        .filter(|target| {
            !target.provider_id.trim().is_empty() && !target.base_url.trim().is_empty()
        })
        .collect();
    if cleaned_targets.is_empty() {
        return Err("至少选择一个模型供应商".to_string());
    }
    let prompt = prompt
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(codex_wakeup::DEFAULT_PROMPT)
        .to_string();
    let model = model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let run_id = run_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let total = cleaned_targets.len();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(CODEX_MODEL_PROVIDER_TEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("CREATE_HTTP_CLIENT_FAILED: {}", e))?;

    emit_model_provider_chat_test_progress(
        &app,
        &run_id,
        total,
        0,
        0,
        0,
        true,
        "batch_started",
        None,
        None,
    );

    let mut records = Vec::with_capacity(total);
    let mut success_count = 0usize;
    let mut failure_count = 0usize;
    for (index, target) in cleaned_targets.into_iter().enumerate() {
        emit_model_provider_chat_test_progress(
            &app,
            &run_id,
            total,
            index,
            success_count,
            failure_count,
            true,
            "provider_started",
            Some(&target.provider_id),
            None,
        );
        let record = run_single_model_provider_chat_test(
            &client,
            target,
            &prompt,
            model.as_deref(),
            &run_id,
        )
        .await;
        if record.success {
            success_count += 1;
        } else {
            failure_count += 1;
        }
        emit_model_provider_chat_test_progress(
            &app,
            &run_id,
            total,
            index + 1,
            success_count,
            failure_count,
            true,
            "provider_completed",
            Some(&record.provider_id),
            Some(record.clone()),
        );
        records.push(record);
    }

    emit_model_provider_chat_test_progress(
        &app,
        &run_id,
        total,
        total,
        success_count,
        failure_count,
        false,
        "batch_completed",
        None,
        None,
    );

    Ok(CodexModelProviderChatTestBatchResult {
        run_id,
        records,
        success_count,
        failure_count,
    })
}

#[tauri::command]
pub async fn codex_list_model_provider_models(
    base_url: String,
    api_key: String,
) -> Result<CodexModelProviderModelsResult, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("MISSING_API_KEY".to_string());
    }
    let url = codex_model_provider_models_url(&base_url)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(CODEX_MODEL_PROVIDER_TEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("CREATE_HTTP_CLIENT_FAILED: {}", e))?;
    let started = Instant::now();
    let response = client
        .get(&url)
        .bearer_auth(key)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| format!("PROVIDER_MODELS_NETWORK_FAILED: {}", e))?;
    let latency_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "PROVIDER_MODELS_HTTP_{}: {}",
            status.as_u16(),
            text.chars().take(300).collect::<String>()
        ));
    }
    let parsed = serde_json::from_str::<serde_json::Value>(&text)
        .map_err(|e| format!("PROVIDER_MODELS_PARSE_FAILED: {}", e))?;
    Ok(CodexModelProviderModelsResult {
        models: list_model_provider_models(&parsed),
        latency_ms,
    })
}

#[tauri::command]
pub async fn codex_query_model_provider_usage(
    base_url: String,
    api_key: String,
    integration_type: Option<String>,
) -> Result<CodexModelProviderUsageSummary, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("MISSING_API_KEY".to_string());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(CODEX_MODEL_PROVIDER_TEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("CREATE_HTTP_CLIENT_FAILED: {}", e))?;

    let requested_type = integration_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match requested_type {
        Some("new_api") => query_new_api_model_provider_usage(&client, &base_url, key).await,
        Some("sub2api") => query_sub2api_model_provider_usage(&client, &base_url, key).await,
        Some(value) => Err(format!("PROVIDER_USAGE_TYPE_UNSUPPORTED: {}", value)),
        None => {
            let new_api_error =
                match query_new_api_model_provider_usage(&client, &base_url, key).await {
                    Ok(summary) => return Ok(summary),
                    Err(error) => error,
                };
            match query_sub2api_model_provider_usage(&client, &base_url, key).await {
                Ok(summary) => Ok(summary),
                Err(sub2api_error) => Err(format!(
                    "PROVIDER_USAGE_DETECT_FAILED: new_api: {}; sub2api: {}",
                    new_api_error, sub2api_error
                )),
            }
        }
    }
}

async fn query_new_api_model_provider_usage(
    client: &reqwest::Client,
    base_url: &str,
    key: &str,
) -> Result<CodexModelProviderUsageSummary, String> {
    let subscription_url =
        codex_model_provider_new_api_billing_url(base_url, "dashboard/billing/subscription")?;
    let usage_url = codex_model_provider_new_api_billing_url(base_url, "dashboard/billing/usage")?;
    let token_usage_url = codex_model_provider_new_api_api_url(base_url, "api/usage/token/")?;
    let started = Instant::now();
    let subscription_response = client
        .get(&subscription_url)
        .bearer_auth(key)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| format!("PROVIDER_USAGE_NETWORK_FAILED: {}", e))?;
    let subscription_status = subscription_response.status();
    let subscription_text = subscription_response.text().await.unwrap_or_default();
    if !subscription_status.is_success() {
        return Err(format!(
            "PROVIDER_USAGE_HTTP_{}: {}",
            subscription_status.as_u16(),
            subscription_text.chars().take(300).collect::<String>()
        ));
    }
    let usage_response = client
        .get(&usage_url)
        .bearer_auth(key)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| format!("PROVIDER_USAGE_NETWORK_FAILED: {}", e))?;
    let latency_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
    let usage_status = usage_response.status();
    let usage_text = usage_response.text().await.unwrap_or_default();
    if !usage_status.is_success() {
        return Err(format!(
            "PROVIDER_USAGE_HTTP_{}: {}",
            usage_status.as_u16(),
            usage_text.chars().take(300).collect::<String>()
        ));
    }
    let subscription = serde_json::from_str::<serde_json::Value>(&subscription_text)
        .map_err(|e| format!("PROVIDER_USAGE_PARSE_FAILED: {}", e))?;
    let usage = serde_json::from_str::<serde_json::Value>(&usage_text)
        .map_err(|e| format!("PROVIDER_USAGE_PARSE_FAILED: {}", e))?;
    let token_usage = match client
        .get(&token_usage_url)
        .bearer_auth(key)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => {
            let text = response.text().await.unwrap_or_default();
            serde_json::from_str::<serde_json::Value>(&text).ok()
        }
        _ => None,
    };
    Ok(summarize_new_api_model_provider_usage(
        &subscription,
        &usage,
        token_usage.as_ref(),
        latency_ms,
    ))
}

async fn query_sub2api_model_provider_usage(
    client: &reqwest::Client,
    base_url: &str,
    key: &str,
) -> Result<CodexModelProviderUsageSummary, String> {
    let url = codex_model_provider_usage_url(base_url)?;
    let started = Instant::now();
    let response = client
        .get(&url)
        .bearer_auth(key)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| format!("PROVIDER_USAGE_NETWORK_FAILED: {}", e))?;
    let latency_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "PROVIDER_USAGE_HTTP_{}: {}",
            status.as_u16(),
            text.chars().take(300).collect::<String>()
        ));
    }
    let parsed = serde_json::from_str::<serde_json::Value>(&text)
        .map_err(|e| format!("PROVIDER_USAGE_PARSE_FAILED: {}", e))?;
    Ok(summarize_model_provider_usage(&parsed, latency_ms))
}

#[tauri::command]
pub async fn codex_local_access_get_state() -> Result<CodexLocalAccessState, String> {
    codex_local_access::get_local_access_state().await
}

#[tauri::command]
pub async fn codex_local_access_save_accounts(
    account_ids: Vec<String>,
    restrict_free_accounts: Option<bool>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::save_local_access_accounts(
        account_ids,
        restrict_free_accounts.unwrap_or(true),
    )
    .await
}

#[tauri::command]
pub async fn codex_local_access_remove_account(
    account_id: String,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::remove_local_access_account(&account_id).await
}

#[tauri::command]
pub async fn codex_local_access_rotate_api_key() -> Result<CodexLocalAccessState, String> {
    codex_local_access::rotate_local_access_api_key().await
}

#[tauri::command]
pub async fn codex_local_access_update_bound_oauth_account(
    bound_oauth_account_id: Option<String>,
    bound_oauth_use_local_gateway: Option<bool>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_bound_oauth_account(
        bound_oauth_account_id,
        bound_oauth_use_local_gateway.unwrap_or(false),
    )
    .await
}

#[tauri::command]
pub async fn codex_local_access_clear_stats() -> Result<CodexLocalAccessState, String> {
    codex_local_access::clear_local_access_stats().await
}

#[tauri::command]
pub async fn codex_local_access_query_request_logs(
    page: u32,
    page_size: u32,
    stats_range: Option<String>,
    model_query: Option<String>,
    account_query: Option<String>,
    api_key_query: Option<String>,
    gateway_mode: Option<CodexLocalAccessGatewayMode>,
    request_kind: Option<CodexLocalAccessRequestKind>,
    success: Option<bool>,
    error_category: Option<String>,
) -> Result<CodexLocalAccessUsageEventPage, String> {
    codex_local_access::query_local_access_usage_events(
        page,
        page_size,
        stats_range,
        model_query,
        account_query,
        api_key_query,
        gateway_mode,
        request_kind,
        success,
        error_category,
    )
    .await
}

#[tauri::command]
pub async fn codex_local_access_prepare_restart() -> Result<CodexLocalAccessState, String> {
    codex_local_access::prepare_local_access_gateway_for_restart().await
}

#[tauri::command]
pub async fn codex_local_access_kill_port() -> Result<CodexLocalAccessPortCleanupResult, String> {
    codex_local_access::kill_local_access_port_processes().await
}

#[tauri::command]
pub async fn codex_local_access_update_port(port: u16) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_port(port).await
}

#[tauri::command]
pub async fn codex_local_access_update_routing_strategy(
    strategy: CodexLocalAccessRoutingStrategy,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_routing_strategy(strategy).await
}

#[tauri::command]
pub async fn codex_local_access_update_custom_routing(
    rules: Vec<CodexLocalAccessCustomRoutingRule>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_custom_routing(rules).await
}

#[tauri::command]
pub async fn codex_local_access_update_account_model_rules(
    rules: Vec<CodexLocalAccessAccountModelRule>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_account_model_rules(rules).await
}

#[tauri::command]
pub async fn codex_local_access_update_model_rules(
    model_aliases: Vec<CodexLocalAccessModelAlias>,
    excluded_models: Vec<String>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_model_rules(model_aliases, excluded_models).await
}

#[tauri::command]
pub async fn codex_local_access_update_model_pricings(
    model_pricings: Vec<CodexLocalAccessModelPricing>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_model_pricings(model_pricings).await
}

#[tauri::command]
pub async fn codex_local_access_reprice_request_logs() -> Result<CodexLocalAccessState, String> {
    codex_local_access::reprice_local_access_request_logs().await
}

#[tauri::command]
pub async fn codex_local_access_update_routing_options(
    session_affinity: bool,
    session_affinity_ttl_ms: i64,
    max_retry_credentials: u16,
    max_retry_interval_ms: u64,
    disable_cooling: bool,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_routing_options(
        session_affinity,
        session_affinity_ttl_ms,
        max_retry_credentials,
        max_retry_interval_ms,
        disable_cooling,
    )
    .await
}

#[tauri::command]
pub async fn codex_local_access_update_timeouts(
    timeouts: CodexLocalAccessTimeouts,
    active_timeout_preset_id: Option<String>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_timeouts(timeouts, active_timeout_preset_id).await
}

#[tauri::command]
pub async fn codex_local_access_update_timeout_presets(
    timeout_presets: Vec<CodexLocalAccessTimeoutPreset>,
    active_timeout_preset_id: Option<String>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_timeout_presets(
        timeout_presets,
        active_timeout_preset_id,
    )
    .await
}

#[tauri::command]
pub async fn codex_local_access_update_upstream_proxy_config(
    upstream_proxy_url: Option<String>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_upstream_proxy_config(upstream_proxy_url).await
}

#[tauri::command]
pub async fn codex_local_access_update_gateway_mode(
    gateway_mode: CodexLocalAccessGatewayMode,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_gateway_mode(gateway_mode).await
}

#[tauri::command]
pub async fn codex_local_access_update_debug_logs(
    debug_logs: bool,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_debug_logs(debug_logs).await
}

#[tauri::command]
pub async fn codex_local_access_update_access_scope(
    access_scope: CodexLocalAccessScope,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_scope(access_scope).await
}

#[tauri::command]
pub async fn codex_local_access_update_client_base_url_host(
    client_base_url_host: CodexLocalAccessClientBaseUrlHost,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_client_base_url_host(client_base_url_host).await
}

#[tauri::command]
pub async fn codex_local_access_update_image_generation_mode(
    image_generation_mode: crate::models::codex_local_access::CodexLocalAccessImageGenerationMode,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_image_generation_mode(image_generation_mode).await
}

#[tauri::command]
pub async fn codex_local_access_create_api_key(
    label: Option<String>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::create_local_access_api_key(label).await
}

#[tauri::command]
pub async fn codex_local_access_update_api_key(
    api_key_id: String,
    label: Option<String>,
    enabled: Option<bool>,
    model_prefix: Option<String>,
    allowed_models: Option<Vec<String>>,
    excluded_models: Option<Vec<String>>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_api_key(
        api_key_id,
        label,
        enabled,
        model_prefix,
        allowed_models,
        excluded_models,
    )
    .await
}

#[tauri::command]
pub async fn codex_local_access_rotate_named_api_key(
    api_key_id: String,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::rotate_local_access_named_api_key(api_key_id).await
}

#[tauri::command]
pub async fn codex_local_access_delete_api_key(
    api_key_id: String,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::delete_local_access_api_key(api_key_id).await
}

#[tauri::command]
pub async fn codex_local_access_set_enabled(
    enabled: bool,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::set_local_access_enabled(enabled).await
}

#[tauri::command]
pub async fn codex_local_access_activate(
    app: AppHandle,
    auto_repair_mode: Option<codex_session_visibility::CodexSessionVisibilityAutoRepairMode>,
) -> Result<CodexLocalAccessState, String> {
    let flow_started = Instant::now();
    logger::log_info("[Codex API Service Switch][Backend] codex_local_access_activate started");
    let codex_home = codex_account::get_codex_home();
    let previous_credential = read_current_codex_launch_credential_snapshot();
    logger::log_info(&format!(
        "[Codex API Service Switch][Backend] previous credential resolved: elapsed_ms={}",
        flow_started.elapsed().as_millis()
    ));
    let activate_started = Instant::now();
    let state = codex_local_access::activate_local_access_for_dir(&codex_home).await?;
    logger::log_info(&format!(
        "[Codex API Service Switch][Backend] activate_local_access_for_dir finished: elapsed_ms={}, total_ms={}",
        activate_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));
    let api_service_speed = codex_speed::get_api_service_app_speed_config()?.speed;
    let speed_started = Instant::now();
    codex_speed::write_official_app_speed(api_service_speed.clone())?;
    logger::log_info(&format!(
        "[Codex API Service Switch][Backend] write official app speed finished: elapsed_ms={}, total_ms={}",
        speed_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));

    let index_started = Instant::now();
    let mut index = codex_account::load_account_index();
    index.current_account_id = None;
    codex_account::save_account_index(&index)?;
    logger::log_info(&format!(
        "[Codex API Service Switch][Backend] account index cleared: elapsed_ms={}, total_ms={}",
        index_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));

    let default_settings_started = Instant::now();
    if let Err(e) = crate::modules::codex_instance::update_default_settings(
        Some(Some(
            crate::modules::codex_instance::CODEX_API_SERVICE_BIND_ACCOUNT_ID.to_string(),
        )),
        None,
        Some(false),
        None,
        None,
    ) {
        logger::log_warn(&format!("更新 Codex 默认实例为 API 服务模式失败: {}", e));
    } else {
        logger::log_info("已同步更新 Codex 默认实例为 API 服务模式");
    }
    if let Err(e) = crate::modules::codex_instance::update_default_app_speed(api_service_speed) {
        logger::log_warn(&format!("更新 Codex 默认实例 API 服务速度失败: {}", e));
    }
    logger::log_info(&format!(
        "[Codex API Service Switch][Backend] default settings update finished: elapsed_ms={}, total_ms={}",
        default_settings_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));
    let repair_started = Instant::now();
    repair_codex_session_visibility_after_credential_kind_change(
        "after-api-service-activate",
        previous_credential,
        Some(CodexLaunchCredentialSnapshot {
            kind: "api".to_string(),
            source: format!(
                "target-bind:{}",
                crate::modules::codex_instance::CODEX_API_SERVICE_BIND_ACCOUNT_ID
            ),
        }),
        auto_repair_mode,
    );
    logger::log_info(&format!(
        "[Codex API Service Switch][Backend] session visibility repair stage finished: elapsed_ms={}, total_ms={}",
        repair_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));

    let user_config = config::get_user_config();

    logger::log_info("API 服务启动模式下跳过 OpenCode / OpenClaw OAuth 同步");

    if user_config.codex_launch_on_switch {
        let launch_started = Instant::now();
        #[cfg(target_os = "macos")]
        if process::is_codex_running() {
            logger::log_info("检测到 Codex 正在运行，将按默认实例 PID 逻辑重启");
        }
        match crate::commands::codex_instance::codex_start_default_with_prepared_profile().await {
            Ok(_) => {}
            Err(e) => {
                logger::log_warn(&format!("Codex 启动失败: {}", e));
                if e.starts_with("APP_PATH_NOT_FOUND:") {
                    let _ = app.emit(
                        "app:path_missing",
                        serde_json::json!({ "app": "codex", "retry": { "kind": "default" } }),
                    );
                }
            }
        }
        logger::log_info(&format!(
            "[Codex API Service Switch][Backend] codex_start_default_with_prepared_profile finished: elapsed_ms={}, total_ms={}",
            launch_started.elapsed().as_millis(),
            flow_started.elapsed().as_millis()
        ));
    } else {
        logger::log_info("已关闭切换 Codex 时自动启动 Codex App");
    }

    let tray_started = Instant::now();
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Codex API Service Switch][Backend] codex_local_access_activate finished: tray_elapsed_ms={}, total_ms={}",
        tray_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));
    Ok(state)
}

#[tauri::command]
pub async fn codex_local_access_test() -> Result<CodexLocalAccessTestResult, String> {
    codex_local_access::test_local_access_with_dialog().await
}

#[tauri::command]
pub async fn codex_local_access_chat_test(
    model_id: String,
    messages: Vec<CodexLocalAccessChatMessage>,
) -> Result<CodexLocalAccessChatResult, String> {
    codex_local_access::chat_local_access_with_dialog(model_id, messages).await
}

#[tauri::command]
pub async fn codex_local_access_chat_test_stream(
    app: AppHandle,
    session_id: String,
    model_id: String,
    messages: Vec<CodexLocalAccessChatMessage>,
) -> Result<(), String> {
    codex_local_access::stream_chat_local_access_with_dialog(app, session_id, model_id, messages)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn models(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn responses_native_chat_test_prefers_gpt_55_over_image_model() {
        let catalog = models(&["gpt-image-2", "gpt-5.5", "gpt-5.4"]);

        assert_eq!(
            select_model_provider_chat_test_model("responses", None, &catalog).as_deref(),
            Some("gpt-5.5")
        );
    }

    #[test]
    fn responses_native_chat_test_skips_image_model_when_preferred_missing() {
        let catalog = models(&["gpt-image-2", "custom-text-model"]);

        assert_eq!(
            select_model_provider_chat_test_model("responses", None, &catalog).as_deref(),
            Some("custom-text-model")
        );
    }

    #[test]
    fn chat_completions_chat_test_keeps_catalog_order() {
        let catalog = models(&["provider-default", "gpt-5.5"]);

        assert_eq!(
            select_model_provider_chat_test_model("chat_completions", None, &catalog).as_deref(),
            Some("provider-default")
        );
    }

    #[test]
    fn explicit_chat_test_model_wins_over_responses_preference() {
        let catalog = models(&["gpt-image-2", "gpt-5.5"]);

        assert_eq!(
            select_model_provider_chat_test_model("responses", Some("custom-model"), &catalog)
                .as_deref(),
            Some("custom-model")
        );
    }
}
