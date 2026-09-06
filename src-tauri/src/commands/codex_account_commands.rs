// Codex 账号命令实现。
//
// 本片段由 `commands/codex.rs` 通过 `include!` 纳入 `commands::codex` 模块，负责账号读取、
// 切换、OAuth、导入导出、配额和唤醒命令。对外调用仍使用 `commands::codex::<command>`，
// 不会因为物理拆分改变 Tauri command 名称或 Rust 调用路径。

static CODEX_POST_REFRESH_CHECK_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
const CODEX_BATCH_DELETE_JOBS_DIR: &str = "codex_batch_delete_jobs";
const CODEX_MAIL_PREVIEW_MAX_BYTES: usize = 512 * 1024;
static CODEX_BATCH_DELETE_JOBS: LazyLock<Mutex<HashMap<String, CodexBatchDeleteJob>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static CODEX_SWITCH_CANCEL_REQUESTS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

fn request_codex_switch_cancel(account_id: &str) {
    CODEX_SWITCH_CANCEL_REQUESTS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(account_id.to_string());
}

fn clear_codex_switch_cancel(account_id: &str) {
    CODEX_SWITCH_CANCEL_REQUESTS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(account_id);
}

fn ensure_codex_switch_not_cancelled(account_id: &str) -> Result<(), String> {
    if CODEX_SWITCH_CANCEL_REQUESTS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .contains(account_id)
    {
        Err("CODEX_START_CANCELLED".to_string())
    } else {
        Ok(())
    }
}

struct CodexSwitchProgressGuard {
    app: AppHandle,
    account_id: String,
    completed: bool,
}

impl Drop for CodexSwitchProgressGuard {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        let _ = self.app.emit(
            "codex:switch-progress",
            serde_json::json!({
                "accountId": self.account_id,
                "type": "error",
            }),
        );
    }
}

fn emit_codex_switch_step(
    app: &AppHandle,
    account_id: &str,
    step: &str,
    status: &str,
    progress: u8,
    details: serde_json::Value,
) {
    let _ = app.emit(
        "codex:switch-progress",
        serde_json::json!({
            "accountId": account_id,
            "step": step,
            "stepStatus": status,
            "progress": progress,
            "details": details,
        }),
    );
}

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
    #[serde(default)]
    api_service_cleaned: bool,
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
    data_dir.join(CODEX_BATCH_DELETE_JOBS_DIR)
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

fn ensure_codex_batch_delete_jobs_dir(path: &Path) -> Result<(), String> {
    if path.is_dir() {
        return Ok(());
    }
    if path.exists() {
        return Err(format!(
            "创建 Codex 批量删除任务目录失败: path={} 不是目录",
            path.display()
        ));
    }
    fs::create_dir(path).map_err(|error| {
        format!(
            "创建 Codex 批量删除任务目录失败: path={}, error={}",
            path.display(),
            error
        )
    })
}

fn save_codex_batch_delete_job_snapshot(
    job_id: &str,
    job: &CodexBatchDeleteJob,
) -> Result<(), String> {
    let path = codex_batch_delete_job_snapshot_path(job_id)?;
    if let Some(parent) = path.parent() {
        ensure_codex_batch_delete_jobs_dir(parent)?;
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

async fn run_account_pool_cleanup_best_effort<F>(
    scope: &str,
    account_count: usize,
    timeout: Duration,
    cleanup: F,
) where
    F: std::future::Future<Output = Result<(), String>>,
{
    if account_count == 0 {
        return;
    }
    match tokio::time::timeout(timeout, cleanup).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            logger::log_warn(&format!(
                "[Codex Delete] 删除前移除 API 服务账号池引用失败，继续删除本地账号: scope={}, account_count={}, error={}",
                scope,
                account_count,
                error
            ));
        }
        Err(_) => {
            logger::log_warn(&format!(
                "[Codex Delete] 删除前移除 API 服务账号池引用超时，继续删除本地账号: scope={}, account_count={}",
                scope,
                account_count
            ));
        }
    }
}

async fn cleanup_accounts_from_api_service_best_effort(scope: &str, account_ids: &[String]) {
    run_account_pool_cleanup_best_effort(
        scope,
        account_ids.len(),
        Duration::from_secs(5),
        codex_local_access::remove_deleted_accounts_from_local_access_pool(account_ids),
    )
    .await;
}

fn spawn_accounts_cleanup_from_api_service(scope: String, account_ids: Vec<String>) {
    if account_ids.is_empty() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        cleanup_accounts_from_api_service_best_effort(&scope, &account_ids).await;
    });
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
                let cleanup_ids = if job.api_service_cleaned {
                    None
                } else {
                    job.api_service_cleaned = true;
                    let failed_ids = job
                        .errors
                        .iter()
                        .map(|error| error.account_id.as_str())
                        .collect::<HashSet<_>>();
                    Some(
                        job.account_ids
                            .iter()
                            .filter(|account_id| !failed_ids.contains(account_id.as_str()))
                            .cloned()
                            .collect(),
                    )
                };
                let snapshot = job.clone();
                drop(jobs);
                save_codex_batch_delete_job_snapshot_best_effort(&job_id, &snapshot);
                if let Some(account_ids) = cleanup_ids {
                    spawn_accounts_cleanup_from_api_service(
                        format!("batch_job:{}", job_id),
                        account_ids,
                    );
                }
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

        let result = match remove_result {
            Ok(()) => Ok(()),
            Err(error) => Err(error),
        };

        if result.is_ok() {
            let account_id = next_account_id.clone();
            if let Err(error) =
                codex_wakeup::remove_deleted_accounts_from_tasks(&[account_id.clone()])
            {
                logger::log_warn(&format!(
                    "[Codex Batch Delete] 清理唤醒任务账号引用失败: job_id={}, account_id={}, error={}",
                    job_id, account_id, error
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
        api_service_cleaned: false,
        created_at: now,
        updated_at: now,
    };
    // 任务快照用于崩溃恢复，但不能阻断实际删除。某些 Windows 数据目录位于
    // junction/reparse point 下时，创建新目录可能返回 ERROR_UNTRUSTED_MOUNT_POINT。
    save_codex_batch_delete_job_snapshot_best_effort(&job_id, &job);
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
        job.api_service_cleaned = false;
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

fn read_codex_launch_credential_snapshot_for_dir(
    base_dir: &Path,
    bind_account_id: Option<&str>,
    include_current_index: bool,
) -> Option<CodexLaunchCredentialSnapshot> {
    if let Some(account_id) = codex_account::read_managed_projection_account_id_from_dir(base_dir) {
        if let Some(snapshot) =
            codex_launch_credential_snapshot_for_account_id(&account_id, "profile:")
        {
            return Some(snapshot);
        }
    }

    // The official client may keep the current OAuth snapshot in Keychain or its
    // profile auth store even when the managed projection marker is missing.
    // Prefer that runtime evidence before falling back to the instance binding.
    if let Some(account_id) = codex_account::oauth_account_id_for_runtime_dir(base_dir) {
        if let Some(snapshot) =
            codex_launch_credential_snapshot_for_account_id(&account_id, "runtime-oauth:")
        {
            return Some(snapshot);
        }
    }

    if let Some(bind_account_id) = bind_account_id {
        if let Some(snapshot) =
            codex_launch_credential_snapshot_for_account_id(bind_account_id, "bind:")
        {
            return Some(snapshot);
        }
    }

    include_current_index
        .then(codex_account::get_current_account)
        .flatten()
        .map(|account| codex_launch_credential_snapshot_for_account(&account, "current-index:"))
}

fn read_current_codex_launch_credential_snapshot() -> Option<CodexLaunchCredentialSnapshot> {
    let codex_home = codex_account::get_codex_home();
    let bind_account_id = crate::modules::codex_instance::load_default_settings()
        .ok()
        .and_then(|settings| settings.bind_account_id);
    read_codex_launch_credential_snapshot_for_dir(&codex_home, bind_account_id.as_deref(), true)
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

async fn stop_default_codex_runtime_before_auth_commit() -> Result<(), String> {
    let codex_home = codex_account::get_codex_home();
    let launch_mode = crate::modules::codex_instance::load_default_settings()?.launch_mode;
    crate::modules::codex_app_injection::stop_for_profile(&codex_home);

    if launch_mode == crate::models::InstanceLaunchMode::App {
        tauri::async_runtime::spawn_blocking(|| process::close_codex_default(20))
            .await
            .map_err(|error| format!("停止 Codex 旧授权运行态后台任务失败: {}", error))??;
    } else {
        logger::log_info("[Codex Switch][Backend] CLI 模式无需关闭桌面运行态，继续提交新授权");
    }

    codex_local_access::stop_provider_gateways_for_profile(&codex_home).await;
    if let Err(error) = crate::modules::codex_instance::update_default_pid(None) {
        logger::log_warn(&format!(
            "[Codex Switch][Backend] 清理默认实例 PID 失败，继续提交新授权: {}",
            error
        ));
    }
    Ok(())
}

/// 列出所有 Codex 账号
#[tauri::command]
pub async fn list_codex_accounts() -> Result<Vec<CodexAccount>, String> {
    tauri::async_runtime::spawn_blocking(codex_account::list_accounts_checked)
        .await
        .map_err(|error| format!("读取 Codex 账号后台任务失败: {}", error))?
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
pub async fn get_codex_quick_config() -> Result<CodexQuickConfig, String> {
    tauri::async_runtime::spawn_blocking(codex_account::load_current_quick_config)
        .await
        .map_err(|error| format!("读取 Codex 快捷配置后台任务失败: {}", error))?
}

#[tauri::command]
pub async fn save_codex_context_management(
    experimental_mode: bool,
) -> Result<CodexQuickConfig, String> {
    tauri::async_runtime::spawn_blocking(move || {
        codex_account::save_context_management_for_base_dir(
            &codex_account::get_codex_home(),
            experimental_mode,
        )
    })
    .await
    .map_err(|error| format!("保存 Codex 上下文管理开关后台任务失败: {}", error))?
}

#[tauri::command]
pub async fn save_codex_quick_config(
    model_context_window: Option<i64>,
    auto_compact_token_limit: Option<i64>,
    experimental_model_catalog_enabled: Option<bool>,
    experimental_model_catalog_models: Option<
        Vec<crate::models::codex::CodexExperimentalModelDefinition>,
    >,
    experimental_model_catalog_default_model_id: Option<String>,
) -> Result<CodexQuickConfig, String> {
    let saved = tauri::async_runtime::spawn_blocking(move || {
        let saved = codex_account::save_current_quick_config(
            model_context_window,
            auto_compact_token_limit,
            experimental_model_catalog_enabled,
            experimental_model_catalog_models,
            experimental_model_catalog_default_model_id,
        )?;
        crate::modules::codex_local_access::refresh_api_service_experimental_model_ids();
        Ok::<CodexQuickConfig, String>(saved)
    })
    .await
    .map_err(|error| format!("保存 Codex 快捷配置后台任务失败: {}", error))??;
    crate::modules::codex_local_access::trigger_gateway_reload_in_background("实验模型目录已更新");
    Ok(saved)
}

#[tauri::command]
pub async fn save_codex_model_catalog(
    experimental_model_catalog_enabled: bool,
    experimental_model_catalog_models: Vec<crate::models::codex::CodexExperimentalModelDefinition>,
    experimental_model_catalog_default_model_id: Option<String>,
) -> Result<CodexQuickConfig, String> {
    let saved = tauri::async_runtime::spawn_blocking(move || {
        let saved = codex_account::save_current_model_catalog_preserving_context(
            experimental_model_catalog_enabled,
            experimental_model_catalog_models,
            experimental_model_catalog_default_model_id,
        )?;
        crate::modules::codex_local_access::refresh_api_service_experimental_model_ids();
        Ok::<CodexQuickConfig, String>(saved)
    })
    .await
    .map_err(|error| format!("保存 Codex 可见模型后台任务失败: {}", error))??;
    crate::modules::codex_local_access::trigger_gateway_reload_in_background("实验模型目录已更新");
    Ok(saved)
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

/// 用户手动强制轮换 OAuth Token。
///
/// 即使当前 access_token 仍有效，也会使用 refresh_token 获取新的 token 链；
/// 失败时保留账号与当前弹框状态，由前端提示用户重新授权或重试。
#[tauri::command]
pub async fn force_refresh_codex_tokens(account_id: String) -> Result<CodexAccount, String> {
    let account = codex_account::load_account(&account_id)
        .ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if account.is_api_key_auth() {
        return Err("API Key 账号没有可刷新的 OAuth Token".to_string());
    }
    if account.is_agent_identity_auth() {
        return Err("Agent Identity 账号没有可刷新的 OAuth Token".to_string());
    }
    if account.is_web_session_auth() {
        return Err("Web Session 账号没有可刷新的 OAuth Token".to_string());
    }
    if !codex_account::account_has_refresh_token(&account) {
        return Err("当前 OAuth 账号缺少 refresh_token，请重新授权".to_string());
    }

    codex_account::force_refresh_managed_account(&account_id, "用户手动强制刷新 Token").await
}

/// 清除账号上由 CDP 记录的“客户端跳转登录页”观测标识。
///
/// 该操作只影响账号卡片上的客户端观测状态，不会修改 Token、远端授权状态或
/// API 服务可用性判断。
#[tauri::command]
pub async fn codex_clear_client_auth_observation(account_id: String) -> Result<bool, String> {
    codex_account::clear_client_auth_observation(&account_id).await
}

/// 切换 Codex 账号（包含 token 刷新检查）
#[tauri::command]
pub async fn switch_codex_account(
    app: AppHandle,
    account_id: String,
    auto_repair_mode: Option<codex_session_visibility::CodexSessionVisibilityAutoRepairMode>,
    reauth_token_generation: Option<u64>,
    launch_after_switch: Option<bool>,
) -> Result<CodexAccount, String> {
    clear_codex_switch_cancel(&account_id);
    let _profile_lease = codex_account::try_acquire_profile_mutation_lease(
        &codex_account::get_codex_home(),
        "oauth-account-switch",
    )?;
    let mut progress_guard = CodexSwitchProgressGuard {
        app: app.clone(),
        account_id: account_id.clone(),
        completed: false,
    };
    emit_codex_switch_step(
        &app,
        &account_id,
        "credentials",
        "running",
        4,
        serde_json::json!({}),
    );
    let flow_started = Instant::now();
    logger::log_info(&format!(
        "[Codex Switch][Backend] switch_codex_account started: account_id={}",
        account_id
    ));
    let initial_account = codex_account::load_account(&account_id)
        .ok_or_else(|| format!("账号不存在: {}", account_id))?;
    ensure_codex_switch_not_cancelled(&account_id)?;
    let is_oauth_account = !initial_account.is_api_key_auth()
        && !initial_account.is_agent_identity_auth()
        && !initial_account.is_web_session_auth();
    let access_token_present = !initial_account.tokens.access_token.trim().is_empty();
    let refresh_token_present = codex_account::account_has_refresh_token(&initial_account);
    let access_token_expires_at =
        codex_oauth::jwt_token_expiration_timestamp(&initial_account.tokens.access_token);
    let access_token_refresh_due =
        is_oauth_account && codex_oauth::is_token_expired(&initial_account.tokens.access_token);
    let is_reauth_handoff = reauth_token_generation.is_some();
    let credentials_need_refresh =
        !is_reauth_handoff && is_oauth_account && access_token_refresh_due;
    let initial_token_generation = initial_account.token_generation;
    let user_config = config::get_user_config();
    let launch_after_switch = launch_after_switch.unwrap_or(user_config.codex_launch_on_switch);
    // client_auth_status 仅记录官方客户端运行后的观测结果，不参与切号或启动拦截。
    // 真实 Token Authority 失败仍由 prepare/switch 流程返回结构化授权错误。
    if launch_after_switch {
        let default_settings = crate::modules::codex_instance::load_default_settings()?;
        if default_settings.launch_mode != crate::models::InstanceLaunchMode::Cli {
            process::ensure_codex_launch_path_configured()?;
        }
    }

    emit_codex_switch_step(
        &app,
        &account_id,
        "credentials",
        "completed",
        10,
        serde_json::json!({
            "accountKind": if is_oauth_account { "oauth" } else { "other" },
            "hasAccessToken": access_token_present,
            "hasRefreshToken": refresh_token_present,
        }),
    );
    emit_codex_switch_step(
        &app,
        &account_id,
        "accessToken",
        if !is_oauth_account {
            "skipped"
        } else if access_token_refresh_due {
            "warning"
        } else {
            "running"
        },
        16,
        serde_json::json!({
            "present": access_token_present,
            "expiresAt": access_token_expires_at,
            "refreshDue": access_token_refresh_due,
            "opaque": access_token_present && access_token_expires_at.is_none(),
        }),
    );

    let previous_credential = read_current_codex_launch_credential_snapshot();
    logger::log_info(&format!(
        "[Codex Switch][Backend] previous credential resolved: account_id={}, elapsed_ms={}",
        account_id,
        flow_started.elapsed().as_millis()
    ));
    // 切换账号事务：先准备并验证目标凭据，通过后再按启动模式停止旧运行态并提交。
    // 桌面模式关闭官方客户端；CLI 模式保留终端进程，仅停止受管 profile 服务。
    let switch_started = Instant::now();
    let step_app = app.clone();
    let step_account_id = account_id.clone();
    if credentials_need_refresh {
        emit_codex_switch_step(
            &app,
            &account_id,
            "refreshTokens",
            "running",
            26,
            serde_json::json!({
                "required": true,
                "hasRefreshToken": refresh_token_present,
            }),
        );
    } else {
        emit_codex_switch_step(
            &app,
            &account_id,
            "refreshTokens",
            "skipped",
            30,
            serde_json::json!({ "required": false }),
        );
    }
    let before_commit = move || {
        let step_app = step_app.clone();
        let step_account_id = step_account_id.clone();
        async move {
            if credentials_need_refresh {
                let prepared_account = codex_account::load_account(&step_account_id);
                emit_codex_switch_step(
                    &step_app,
                    &step_account_id,
                    "refreshTokens",
                    "completed",
                    40,
                    serde_json::json!({
                        "required": true,
                        "tokenGenerationChanged": prepared_account.as_ref().is_some_and(|account| {
                            account.token_generation > initial_token_generation
                        }),
                    }),
                );
            }
            emit_codex_switch_step(
                &step_app,
                &step_account_id,
                "accessToken",
                if is_oauth_account {
                    "completed"
                } else {
                    "skipped"
                },
                42,
                serde_json::json!({
                    "present": access_token_present,
                    "expiresAt": codex_account::load_account(&step_account_id).as_ref().and_then(|account| {
                        codex_oauth::jwt_token_expiration_timestamp(&account.tokens.access_token)
                    }),
                    "refreshDue": false,
                    "opaque": access_token_present && access_token_expires_at.is_none(),
                }),
            );
            emit_codex_switch_step(
                &step_app,
                &step_account_id,
                "stopRuntime",
                "running",
                44,
                serde_json::json!({}),
            );
            stop_default_codex_runtime_before_auth_commit().await?;
            emit_codex_switch_step(
                &step_app,
                &step_account_id,
                "stopRuntime",
                "completed",
                54,
                serde_json::json!({}),
            );
            emit_codex_switch_step(
                &step_app,
                &step_account_id,
                "writeCredentials",
                "running",
                58,
                serde_json::json!({}),
            );
            Ok(())
        }
    };
    let switch_result = if let Some(expected_generation) = reauth_token_generation {
        codex_account::switch_account_managed_after_reauth_with_before_commit(
            &account_id,
            expected_generation,
            before_commit,
        )
        .await
    } else {
        codex_account::switch_account_managed_with_before_commit_and_revalidation_options(
            &account_id,
            before_commit,
        )
        .await
    };
    ensure_codex_switch_not_cancelled(&account_id)?;
    let account = match switch_result {
        Ok(account) => account,
        Err(error) => {
            let formatted_error = codex_account::format_account_switch_error(&account_id, error);
            let auth_failure = formatted_error.starts_with("CODEX_SWITCH_AUTH_REQUIRED:");
            let _ = app.emit(
                "codex:switch-progress",
                serde_json::json!({
                    "accountId": account_id,
                    "type": "error",
                    "error": formatted_error,
                    "canRetry": !auth_failure,
                }),
            );
            progress_guard.completed = true;
            return Err(formatted_error);
        }
    };
    if is_reauth_handoff {
        let quota_account_id = account.id.clone();
        tokio::spawn(async move {
            if let Err(error) = codex_quota::refresh_account_quota(&quota_account_id).await {
                logger::log_warn(&format!(
                    "重新授权切号完成后刷新配额失败: account_id={}, error={}",
                    quota_account_id, error
                ));
            }
        });
    }
    logger::log_info(&format!(
        "[Codex Switch][Backend] switch_account_managed finished: account_id={}, elapsed_ms={}, total_ms={}",
        account_id,
        switch_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));
    emit_codex_switch_step(
        &app,
        &account_id,
        "writeCredentials",
        "completed",
        68,
        serde_json::json!({}),
    );
    let account_speed = account.app_speed.clone();
    let speed_started = Instant::now();
    codex_speed::write_official_app_speed(account_speed.clone())?;
    logger::log_info(&format!(
        "[Codex Switch][Backend] write official app speed finished: account_id={}, elapsed_ms={}, total_ms={}",
        account_id,
        speed_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));
    emit_codex_switch_step(
        &app,
        &account_id,
        "syncSettings",
        "running",
        72,
        serde_json::json!({}),
    );
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
    let default_bind_account_id =
        if crate::modules::codex_local_access::account_requires_provider_gateway(&account) {
            crate::modules::codex_instance::provider_gateway_bind_account_id(&account.id)
                .unwrap_or_else(|| account.id.clone())
        } else {
            account.id.clone()
        };
    if let Err(e) = crate::modules::codex_instance::update_default_settings(
        Some(Some(default_bind_account_id.clone())),
        None,
        None,
        Some(false),
        None,
        None,
    ) {
        logger::log_warn(&format!("更新 Codex 默认实例绑定账号失败: {}", e));
    } else {
        logger::log_info(&format!(
            "已同步更新 Codex 默认实例绑定账号: {}",
            default_bind_account_id
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

    apply_codex_switch_auth_projections(&account, &user_config);

    // Full #1404: optional auto SSH sync after switch (hash-verified remote projection + app-server reload).
    if let Some(ssh_sync) =
        crate::modules::ssh_server::sync_selected_server_after_codex_switch(&account).await
    {
        if ssh_sync.verified {
            logger::log_info(&format!(
                "[Codex SSH] 切号后同步成功: server_id={}, account={}, verified={}",
                ssh_sync.server_id, ssh_sync.account_email, ssh_sync.verified
            ));
        } else {
            logger::log_warn(&format!(
                "[Codex SSH] 切号后同步失败: server_id={}, error={}",
                ssh_sync.server_id,
                ssh_sync.error.clone().unwrap_or_default()
            ));
        }
        let _ = app.emit("codex:ssh-sync-result", &ssh_sync);
    }

    emit_codex_switch_step(
        &app,
        &account_id,
        "syncSettings",
        "completed",
        82,
        serde_json::json!({}),
    );

    if launch_after_switch {
        ensure_codex_switch_not_cancelled(&account_id)?;
        emit_codex_switch_step(
            &app,
            &account_id,
            "startClient",
            "running",
            86,
            serde_json::json!({}),
        );
        let launch_started = Instant::now();
        #[cfg(target_os = "macos")]
        if process::is_codex_running() {
            logger::log_info("检测到 Codex 正在运行，将按默认实例 PID 逻辑重启");
        }
        // 调用统一默认实例启动方法：`codex_start_default_with_prepared_profile` 会继续进入
        // `codex_start_instance_internal`，复用多开实例的 Token 预检、profile 准备和启动流程。
        let launch_error =
            match crate::commands::codex_instance::codex_start_default_with_prepared_profile(
                app.clone(),
                true,
                Some("switch-and-start"),
                None,
            )
            .await
            {
                Ok(_) => None,
                Err(e) => {
                    let app_path_missing = e.starts_with("APP_PATH_NOT_FOUND:");
                    let formatted_error =
                        codex_account::format_account_switch_error(&account_id, e);
                    logger::log_warn(&format!("Codex 启动失败: {}", formatted_error));
                    if app_path_missing {
                        let _ = app.emit(
                            "app:path_missing",
                            serde_json::json!({ "app": "codex", "retry": { "kind": "default" } }),
                        );
                    }
                    Some(formatted_error)
                }
            };
        emit_codex_switch_step(
            &app,
            &account_id,
            "startClient",
            if launch_error.is_some() {
                "error"
            } else {
                "completed"
            },
            96,
            serde_json::json!({
                "error": launch_error.as_deref(),
                "canRetry": launch_error.is_some(),
            }),
        );
        logger::log_info(&format!(
            "[Codex Switch][Backend] codex_start_default_with_prepared_profile finished: account_id={}, elapsed_ms={}, total_ms={}",
            account_id,
            launch_started.elapsed().as_millis(),
            flow_started.elapsed().as_millis()
        ));
        if let Some(error) = launch_error {
            let auth_failure = error.starts_with("CODEX_SWITCH_AUTH_REQUIRED:");
            let _ = app.emit(
                "codex:switch-progress",
                serde_json::json!({
                    "accountId": account_id,
                    "type": "error",
                    "error": error.clone(),
                    "canRetry": !auth_failure,
                }),
            );
            progress_guard.completed = true;
            return Err(error);
        }
    } else {
        logger::log_info("已关闭切换 Codex 时自动启动 Codex App");
        emit_codex_switch_step(
            &app,
            &account_id,
            "startClient",
            "skipped",
            96,
            serde_json::json!({ "launchDisabled": true }),
        );
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
    let _ = app.emit(
        "codex:switch-progress",
        serde_json::json!({
            "accountId": account_id,
            "type": "complete",
            "stage": "completed",
            "progress": 100,
        }),
    );
    progress_guard.completed = true;
    clear_codex_switch_cancel(&account_id);
    Ok(account)
}

/// 请求停止账号切换及其后续启动事务；不会修改账号凭据或授权状态。
#[tauri::command]
pub async fn codex_cancel_account_switch(app: AppHandle, account_id: String) -> Result<(), String> {
    request_codex_switch_cancel(&account_id);
    let _ = app.emit(
        "codex:switch-progress",
        serde_json::json!({
            "accountId": account_id,
            "type": "cancelled",
            "error": "CODEX_START_CANCELLED",
            "cancelled": true,
        }),
    );
    Ok(())
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
            match switch_codex_account(app.clone(), target_id.clone(), None, None, None).await
            {
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
    if let Err(error) = codex_wakeup::remove_deleted_accounts_from_tasks(&[account_id.clone()]) {
        logger::log_warn(&format!(
            "[Codex] 清理唤醒任务账号引用失败: account_id={}, error={}",
            account_id, error
        ));
    }
    // 本地删除成功后立即返回；API 服务账号池持久化与网关重载在后台完成，
    // 避免外部进程延迟让用户误以为账号没有删除。
    spawn_accounts_cleanup_from_api_service("single_delete".to_string(), vec![account_id]);
    Ok(())
}

/// 批量删除 Codex 账号
#[tauri::command]
pub async fn delete_codex_accounts(account_ids: Vec<String>) -> Result<(), String> {
    codex_account::remove_accounts(&account_ids)?;
    if let Err(error) = codex_wakeup::remove_deleted_accounts_from_tasks(&account_ids) {
        logger::log_warn(&format!(
            "[Codex] 批量清理唤醒任务账号引用失败: count={}, error={}",
            account_ids.len(),
            error
        ));
    }
    spawn_accounts_cleanup_from_api_service("multi_delete".to_string(), account_ids);
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

/// Shared OpenCode / OpenClaw / Hermes projections after a successful Codex switch.
/// Failures are best-effort (warn only), matching the interactive switch path.
fn apply_codex_switch_auth_projections(account: &CodexAccount, user_config: &config::UserConfig) {
    let mut opencode_updated = false;
    if user_config.opencode_auth_overwrite_on_switch {
        match opencode_auth::replace_openai_entry_from_codex(account) {
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
        match openclaw_auth::replace_openai_codex_entry_from_codex(account) {
            Ok(()) => {}
            Err(e) => {
                logger::log_warn(&format!("OpenClaw auth 同步失败: {}", e));
            }
        }
    } else {
        logger::log_info("已关闭切换 Codex 时覆盖 OpenClaw 登录信息");
    }

    if user_config.hermes_auth_overwrite_on_switch {
        match hermes_auth::replace_openai_codex_entry_from_codex(account) {
            Ok(()) => {}
            Err(e) => {
                logger::log_warn(&format!("Hermes auth 同步失败: {}", e));
            }
        }
    } else {
        logger::log_info("已关闭切换 Codex 时覆盖 Hermes 登录信息");
    }

    // Full #1404: after switch, sync selected SSH host with verified remote projection.
    // Note: apply_codex_switch_auth_projections is sync; spawn is handled by callers that
    // are already async. Here we only log intent — actual auto-sync is triggered from
    // switch_codex_account after this returns.
}

/// Re-activate current account after import when needed, then project auth side effects.
async fn reactivate_imported_current_if_needed(imported: &[CodexAccount]) {
    if let Some(account) = codex_account::reactivate_if_imported_matches_current(imported).await {
        let user_config = config::get_user_config();
        apply_codex_switch_auth_projections(&account, &user_config);
        if let Err(e) = codex_speed::write_official_app_speed(account.app_speed.clone()) {
            logger::log_warn(&format!("[Codex导入] 重新激活后写入 app speed 失败: {}", e));
        }
    }
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

/// 导入 named Codex access token 账号（`at-*` / personal access token）。
#[tauri::command]
pub async fn import_codex_access_token_account(
    app: AppHandle,
    name: String,
    access_token: String,
) -> Result<CodexAccount, String> {
    let account = codex_account::import_access_token_account(name, access_token)?;
    let account_id = account.id.clone();
    if let Err(error) = codex_account::refresh_account_profile(&account_id).await {
        logger::log_warn(&format!(
            "Codex access token account profile refresh failed after import: account_id={}, error={}",
            account_id, error
        ));
    }
    let refreshed = codex_account::load_account(&account_id).unwrap_or(account);
    reactivate_imported_current_if_needed(std::slice::from_ref(&refreshed)).await;
    let mut accounts = refresh_imported_codex_accounts(&app, vec![refreshed]).await;
    accounts
        .pop()
        .ok_or_else(|| "Account could not be loaded after import".to_string())
}

/// 从官方 Codex 本机凭据存储导入账号（auth.json / macOS Keychain）
#[tauri::command]
pub async fn import_codex_from_local(app: AppHandle) -> Result<CodexAccount, String> {
    let account = codex_account::import_from_local()?;
    reactivate_imported_current_if_needed(std::slice::from_ref(&account)).await;
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
    reactivate_imported_current_if_needed(&accounts).await;
    Ok(refresh_imported_codex_accounts(&app, accounts).await)
}

/// 导出 Codex 账号
#[tauri::command]
pub fn export_codex_accounts(account_ids: Vec<String>) -> Result<String, String> {
    codex_account::export_accounts(&account_ids)
}

/// 从本地文件导入 Codex 账号。
///
/// 直导路径：只落盘账号，不做导入前/导入后额度检测，避免单账号也因网络刷新变慢。
#[tauri::command]
pub async fn import_codex_from_files(
    app: AppHandle,
    file_paths: Vec<String>,
) -> Result<codex_account::CodexFileImportResult, String> {
    let result = codex_account::import_from_files(file_paths).await?;
    reactivate_imported_current_if_needed(&result.imported).await;
    if !result.imported.is_empty() {
        let _ = crate::modules::tray::update_tray_menu(&app);
    }
    Ok(result)
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
pub async fn confirm_codex_batch_import(
    app: AppHandle,
    session_id: String,
    item_ids: Vec<String>,
) -> Result<codex_account::CodexBatchImportConfirmResult, String> {
    let result = codex_account::confirm_codex_batch_import(&app, &session_id, &item_ids)?;
    reactivate_imported_current_if_needed(&result.imported).await;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(result)
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
    // 分组策略「不刷新」：自动当前号刷新静默跳过
    if !codex_account::is_quota_refresh_enabled_for_account(&account.id) {
        logger::log_info(&format!(
            "[Codex Quota] 当前账号所属分组已关闭额度刷新，跳过: account_id={}",
            account.id
        ));
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

/// 按账号 ID 列表限流并发刷新配额（分组刷新 / 本地访问批量等）
/// 只在全部任务结束后做一次 tray / post-check，避免 N 次并发互踩。
///
/// `respect_group_quota_refresh` 缺省 true：跳过分组「不刷新」账号。
/// 显式「刷新分组」应传 false。
#[tauri::command]
pub async fn refresh_codex_quotas_batch(
    app: AppHandle,
    account_ids: Vec<String>,
    respect_group_quota_refresh: Option<bool>,
) -> Result<i32, String> {
    let respect = respect_group_quota_refresh.unwrap_or(true);
    let results =
        codex_quota::refresh_quotas_for_account_ids_with_options(&account_ids, respect).await?;
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

    // 旧官方客户端可能仍持有同一账号的旧 auth.json。普通新增授权使用刚落库的
    // 凭据直接查询配额，避免 live authority 把新 Token 覆盖回旧 Token；重新授权
    // 则等自动切号提交完成后再按正常流程刷新。
    if reauth_account_id.is_none() {
        if let Err(e) = codex_quota::refresh_freshly_authorized_account_quota(
            &account.id,
            account.token_generation,
        )
        .await
        {
            logger::log_error(&format!("刷新配额失败: {}", e));
        }
    }

    let loaded =
        codex_account::load_account(&account.id).ok_or_else(|| "账号保存后无法读取".to_string())?;
    if reauth_account_id.is_some() {
        if let Err(error) = codex_account::sync_bound_oauth_consumers_after_reauth(&loaded.id).await
        {
            logger::log_warn(&format!(
                "OAuth 重新授权后同步绑定消费者失败，保留已保存授权: account_id={}, error={}",
                loaded.id, error
            ));
        }
    }
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

/// OAuth：使用官方设备授权流程（无本地回调端口）
#[tauri::command]
pub async fn codex_oauth_device_auth_start(
    app_handle: AppHandle,
) -> Result<codex_oauth::CodexDeviceAuthStartResponse, String> {
    logger::log_info("Codex OAuth device-auth 命令触发");
    codex_oauth::start_device_auth(app_handle).await
}

/// OAuth：在内置无痕 WebView 中打开当前授权地址
#[tauri::command]
pub fn codex_oauth_open_incognito_window(
    app_handle: AppHandle,
    auth_url: String,
) -> Result<(), String> {
    codex_oauth::open_incognito_oauth_window(&app_handle, &auth_url)
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
pub fn codex_oauth_login_cancel(
    app_handle: AppHandle,
    login_id: Option<String>,
) -> Result<(), String> {
    logger::log_info(&format!(
        "Codex OAuth cancel 命令触发: login_id={}",
        login_id.as_deref().unwrap_or("<none>")
    ));
    let result = codex_oauth::cancel_oauth_flow_for(login_id.as_deref());
    logger::log_info(&format!(
        "Codex OAuth cancel 命令返回: {:?}",
        result.as_ref().map(|_| "ok").map_err(|e| e)
    ));
    result?;
    codex_oauth::close_oauth_window(&app_handle)
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
    codex_oauth::close_oauth_window(&app_handle)?;
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
    api_sync_model_catalog_to_codex: Option<bool>,
    api_wire_api: Option<String>,
    api_supports_websockets: Option<bool>,
    api_supports_vision: Option<bool>,
    api_model_vision_support: Option<std::collections::HashMap<String, bool>>,
    api_vision_routing_model: Option<String>,
    account_name: Option<String>,
    api_model_context_windows: Option<std::collections::HashMap<String, i64>>,
) -> Result<CodexAccount, String> {
    let account = codex_account::upsert_api_key_account(
        api_key,
        api_base_url,
        api_provider_mode,
        api_provider_id,
        api_provider_name,
        api_model_catalog.unwrap_or_default(),
        api_sync_model_catalog_to_codex,
        api_wire_api,
        api_supports_websockets.unwrap_or(false),
        api_supports_vision.unwrap_or(false),
        api_model_vision_support.unwrap_or_default(),
        api_vision_routing_model,
        account_name,
        api_model_context_windows,
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
    api_sync_model_catalog_to_codex: Option<bool>,
    api_wire_api: Option<String>,
    api_supports_websockets: Option<bool>,
    api_supports_vision: Option<bool>,
    api_model_vision_support: Option<std::collections::HashMap<String, bool>>,
    api_vision_routing_model: Option<String>,
    account_name: Option<String>,
    api_model_context_windows: Option<std::collections::HashMap<String, i64>>,
) -> Result<CodexAccount, String> {
    codex_account::update_api_key_credentials(
        &account_id,
        api_key,
        api_base_url,
        api_provider_mode,
        api_provider_id,
        api_provider_name,
        api_model_catalog.unwrap_or_default(),
        api_sync_model_catalog_to_codex,
        api_wire_api,
        api_supports_websockets.unwrap_or(false),
        api_supports_vision.unwrap_or(false),
        api_model_vision_support.unwrap_or_default(),
        api_vision_routing_model,
        account_name,
        api_model_context_windows,
    )
}

#[tauri::command]
pub async fn sync_codex_api_key_provider_accounts(
    account_ids: Vec<String>,
    api_base_url: Option<String>,
    api_provider_mode: Option<CodexApiProviderMode>,
    api_provider_id: Option<String>,
    api_provider_name: Option<String>,
    api_model_catalog: Option<Vec<String>>,
    api_wire_api: Option<String>,
    api_supports_websockets: Option<bool>,
    api_supports_vision: Option<bool>,
    api_model_vision_support: Option<std::collections::HashMap<String, bool>>,
    api_vision_routing_model: Option<String>,
    api_model_context_windows: Option<std::collections::HashMap<String, i64>>,
) -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(move || {
        codex_account::sync_api_key_provider_accounts(
            account_ids,
            api_base_url,
            api_provider_mode,
            api_provider_id,
            api_provider_name,
            api_model_catalog.unwrap_or_default(),
            api_wire_api,
            api_supports_websockets.unwrap_or(false),
            api_supports_vision.unwrap_or(false),
            api_model_vision_support.unwrap_or_default(),
            api_vision_routing_model,
            api_model_context_windows,
        )
    })
    .await
    .map_err(|error| format!("同步 Codex 供应商账号快照任务失败: {}", error))?
}

#[tauri::command]
pub async fn update_codex_api_key_bound_oauth_account(
    account_id: String,
    bound_oauth_account_id: Option<String>,
) -> Result<CodexAccount, String> {
    codex_account::update_api_key_bound_oauth_account(&account_id, bound_oauth_account_id).await
}

#[tauri::command]
pub async fn update_codex_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<CodexAccount, String> {
    codex_account::update_account_tags(&account_id, tags)
}

#[tauri::command]
pub async fn update_codex_accounts_fingerprint_mode(
    account_ids: Vec<String>,
    mode: String,
) -> Result<Vec<CodexAccount>, String> {
    codex_account::update_accounts_fingerprint_mode(&account_ids, mode)
}

#[tauri::command]
pub async fn update_codex_account_client_policy(
    account_id: String,
    codex_cli_only: bool,
    allow_app_server: bool,
) -> Result<CodexAccount, String> {
    codex_account::update_account_client_policy(&account_id, codex_cli_only, allow_app_server)
}

#[tauri::command]
pub async fn update_codex_account_instance_access(
    account_id: String,
    access_mode: Option<String>,
    startup_model: Option<String>,
) -> Result<CodexAccount, String> {
    tauri::async_runtime::spawn_blocking(move || {
        codex_account::update_account_instance_access(&account_id, access_mode, startup_model)
    })
    .await
    .map_err(|error| format!("保存 DeepSeek 接入方式失败: {}", error))?
}

#[tauri::command]
pub async fn update_codex_account_api_model_mappings(
    account_id: String,
    mappings: Vec<CodexApiModelMapping>,
    api_model_context_windows: Option<std::collections::HashMap<String, i64>>,
) -> Result<CodexAccount, String> {
    let account = tauri::async_runtime::spawn_blocking(move || {
        codex_account::update_account_api_model_mappings(
            &account_id,
            mappings,
            api_model_context_windows,
        )
    })
    .await
    .map_err(|error| format!("保存模型映射失败: {}", error))??;
    if codex_local_access::collection_contains_account(&account.id) {
        codex_local_access::trigger_gateway_reload_in_background("保存账号模型映射");
    }
    Ok(account)
}

#[tauri::command]
pub async fn update_codex_account_note(
    account_id: String,
    note: Option<String>,
    two_factor_secret: Option<String>,
    account_password: Option<String>,
    phone_number: Option<String>,
    mail_url: Option<String>,
    chatgpt_account_id: Option<String>,
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
        chatgpt_account_id,
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
    let parsed = reqwest::Url::parse(mail_url).map_err(|_| "MAIL_URL_INVALID".to_string())?;
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

/// 启动时根据配置自动恢复可见模型目录与代理接管状态
#[tauri::command]
pub async fn restore_codex_active_takeover_if_enabled(app: AppHandle) -> Result<bool, String> {
    let cfg = config::get_user_config();
    if !cfg.codex_auto_restore_takeover_on_launch {
        return Ok(false);
    }
    let base_dir = codex_account::get_codex_home();
    let reapply_catalog_result =
        codex_account::reapply_experimental_model_policy_if_enabled(&base_dir)?;
    logger::log_info(&format!(
        "[Codex Auto-Restore] restore_codex_active_takeover_if_enabled executed: reapply_catalog={}",
        reapply_catalog_result
    ));
    Ok(reapply_catalog_result)
}

// ─── Codex 账号分组持久化 ────────────────────────────────────────────
