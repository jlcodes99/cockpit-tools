// Codex Local Access：实例级本地网关只读快照（provider gateway / 混合模型路由 / 绑定 OAuth 本地网关）。
// 通过 include! 保持原 modules::codex_local_access 作用域和私有调用关系。

const INSTANCE_GATEWAY_KIND_PROVIDER: &str = "providerGateway";
const INSTANCE_GATEWAY_KIND_MIXED_MODEL: &str = "mixedModel";
const INSTANCE_GATEWAY_KIND_BOUND_OAUTH: &str = "boundOauth";

const INSTANCE_GATEWAY_STATUS_RUNNING: &str = "running";
const INSTANCE_GATEWAY_STATUS_UNREACHABLE: &str = "unreachable";
const INSTANCE_GATEWAY_STATUS_PORT_CONFLICT: &str = "portConflict";
const INSTANCE_GATEWAY_STATUS_STOPPED: &str = "stopped";
const INSTANCE_GATEWAY_STATUS_NOT_STARTED: &str = "notStarted";

const INSTANCE_GATEWAY_READY_PROBE_TIMEOUT: Duration = Duration::from_millis(400);
const INSTANCE_GATEWAY_PORT_PROBE_TIMEOUT: Duration = Duration::from_millis(300);
const INSTANCE_GATEWAY_RECOVERY_ATTEMPTS: u32 = 3;
const INSTANCE_GATEWAY_RECOVERY_RETRY_DELAY: Duration = Duration::from_secs(2);

/// 启动自愈失败原因（按 profile + runtime 记录），供实例网关弹框展示。
static INSTANCE_GATEWAY_RECOVERY_ERRORS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

fn instance_gateway_recovery_store() -> &'static Mutex<HashMap<String, String>> {
    INSTANCE_GATEWAY_RECOVERY_ERRORS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn set_instance_gateway_recovery_error(runtime_key: &str, error: Option<String>) {
    let Ok(mut store) = instance_gateway_recovery_store().lock() else {
        return;
    };
    match error {
        Some(error) => {
            store.insert(runtime_key.to_string(), error);
        }
        None => {
            store.remove(runtime_key);
        }
    }
}

fn instance_gateway_recovery_error(runtime_key: &str) -> Option<String> {
    instance_gateway_recovery_store()
        .lock()
        .ok()
        .and_then(|store| store.get(runtime_key).cloned())
}

struct InstanceGatewayTarget {
    instance_id: String,
    instance_name: String,
    is_default: bool,
    last_pid: Option<u32>,
    profile_dir: PathBuf,
    kind: &'static str,
    runtime_id: String,
    account: CodexAccount,
}

/// 判断某个 profile 当前绑定是否需要实例级本地网关；不需要时返回 None。
fn instance_gateway_target_for_profile(
    instance_id: String,
    instance_name: String,
    is_default: bool,
    last_pid: Option<u32>,
    profile_dir: PathBuf,
    bind_account_id: Option<&str>,
    model_routing: Option<&CodexInstanceModelRouting>,
) -> Option<InstanceGatewayTarget> {
    if model_routing.is_some_and(|routing| routing.enabled) {
        let account_id = codex_account::oauth_account_id_for_runtime_binding(bind_account_id)?;
        let account = codex_account::load_account(&account_id)?;
        return Some(InstanceGatewayTarget {
            instance_id,
            instance_name,
            is_default,
            last_pid,
            profile_dir,
            kind: INSTANCE_GATEWAY_KIND_MIXED_MODEL,
            runtime_id: MIXED_MODEL_ROUTING_RUNTIME_ID.to_string(),
            account,
        });
    }

    let bind_account_id = bind_account_id.map(str::trim).filter(|value| !value.is_empty())?;
    if crate::modules::codex_instance::is_api_service_bind_account_id(bind_account_id) {
        // 全局 API 服务在账号总览卡片上单独展示，这里只列出实例级网关。
        return None;
    }
    let account_id = crate::modules::codex_instance::parse_provider_gateway_bind_account_id(
        bind_account_id,
    )
    .unwrap_or_else(|| bind_account_id.to_string());
    let account = codex_account::load_account(&account_id)?;

    if account_requires_provider_gateway(&account) {
        return Some(InstanceGatewayTarget {
            instance_id,
            instance_name,
            is_default,
            last_pid,
            profile_dir,
            kind: INSTANCE_GATEWAY_KIND_PROVIDER,
            runtime_id: account_id,
            account,
        });
    }
    // 说明：`account_requires_bound_oauth_local_gateway` 目前是占位实现（恒为 false），
    // 因此 boundOauth 类型在补齐真实判定前不会出现在实例网关快照里；这里保留分支与
    // 自愈入口，避免判定恢复后又漏掉本地网关的启动恢复。
    if account_requires_bound_oauth_local_gateway(&account) {
        return Some(InstanceGatewayTarget {
            instance_id,
            instance_name,
            is_default,
            last_pid,
            profile_dir,
            kind: INSTANCE_GATEWAY_KIND_BOUND_OAUTH,
            runtime_id: account_id,
            account,
        });
    }
    None
}

fn collect_instance_gateway_targets() -> Result<Vec<InstanceGatewayTarget>, String> {
    let store = crate::modules::codex_instance::load_instance_store()?;
    let mut targets = Vec::new();

    let default_profile_dir = crate::modules::codex_instance::get_default_codex_home()?;
    if let Some(target) = instance_gateway_target_for_profile(
        "__default__".to_string(),
        String::new(),
        true,
        store.default_settings.last_pid,
        default_profile_dir,
        store.default_settings.bind_account_id.as_deref(),
        store.default_settings.model_routing.as_ref(),
    ) {
        targets.push(target);
    }

    for instance in store.instances {
        let user_data_dir = instance.user_data_dir.trim();
        if user_data_dir.is_empty() {
            continue;
        }
        if let Some(target) = instance_gateway_target_for_profile(
            instance.id.clone(),
            instance.name.clone(),
            false,
            instance.last_pid,
            PathBuf::from(user_data_dir),
            instance.bind_account_id.as_deref(),
            instance.model_routing.as_ref(),
        ) {
            targets.push(target);
        }
    }

    Ok(targets)
}

/// 实例（含默认实例）当前是否真的有 Codex 进程在运行。
fn instance_target_is_running(
    target: &InstanceGatewayTarget,
    process_entries: &[(u32, Option<String>)],
) -> bool {
    let home = (!target.is_default).then(|| target.profile_dir.to_string_lossy().to_string());
    crate::modules::process::resolve_codex_pid_from_entries(
        target.last_pid,
        home.as_deref(),
        process_entries,
    )
    .is_some()
}

fn find_instance_gateway_target(
    instance_id: &str,
    kind: &str,
) -> Result<InstanceGatewayTarget, String> {
    let instance_id = instance_id.trim();
    let kind = kind.trim();
    collect_instance_gateway_targets()?
        .into_iter()
        .find(|target| target.instance_id == instance_id && target.kind == kind)
        .ok_or_else(|| "未找到对应的实例网关".to_string())
}

/// 清掉持久化的网关状态，避免实例未运行或用户手动停止后又被自动拉起。
fn clear_instance_gateway_profile_state(profile_dir: &Path, runtime_id: &str) {
    let Ok(path) = provider_gateway_state_path(profile_dir, runtime_id) else {
        return;
    };
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => logger::log_codex_api_warn(&format!(
            "[CodexLocalAccess][instance-gateway] 清理网关状态失败: profile={}, runtime_id={}, error={}",
            profile_dir.display(),
            runtime_id,
            error
        )),
    }
}

/// 停止该 profile 的全部实例级网关，并还原它写入的接管状态与持久状态。
///
/// 实例不运行（被关闭 / 手动停止网关）时使用：停进程后必须一并清理状态，
/// 否则下次启动自愈或后台监控会把它再拉起来。
pub async fn release_instance_gateway_for_profile(profile_dir: &Path) -> Result<(), String> {
    stop_provider_gateways_for_profile(profile_dir).await;
    restore_mixed_model_gateway_profile(profile_dir)?;
    cleanup_provider_gateway_profile_model_overrides(profile_dir)?;
    clear_instance_gateway_profile_state(profile_dir, MIXED_MODEL_ROUTING_RUNTIME_ID);
    set_instance_gateway_recovery_error(
        &provider_gateway_runtime_key(profile_dir, MIXED_MODEL_ROUTING_RUNTIME_ID),
        None,
    );
    Ok(())
}

/// 停止某个实例网关，并清理它对该 profile 的接管状态。
///
/// 混合模型路由网关与路由本身同生共死：只停进程会被后台监控立刻恢复，因此这里同时关闭
/// 该实例的混合路由（渠道配置保留）。
pub async fn stop_instance_gateway_for(instance_id: &str, kind: &str) -> Result<(), String> {
    let target = find_instance_gateway_target(instance_id, kind)?;
    let runtime_key = provider_gateway_runtime_key(&target.profile_dir, &target.runtime_id);
    if target.kind == INSTANCE_GATEWAY_KIND_MIXED_MODEL {
        crate::modules::codex_instance::disable_model_routing(&target.instance_id)?;
        return release_instance_gateway_for_profile(&target.profile_dir).await;
    }
    if let Some(endpoint) = stop_provider_gateway_runtime(&runtime_key).await {
        let _ = wait_for_gateway_port_release(&endpoint.bind_host, endpoint.port).await;
    }
    cleanup_provider_gateway_profile_model_overrides(&target.profile_dir)?;
    clear_instance_gateway_profile_state(&target.profile_dir, &target.runtime_id);
    set_instance_gateway_recovery_error(&runtime_key, None);
    Ok(())
}

/// 重新启动某个实例网关（按当前绑定账号 / 混合路由配置重建）。
pub async fn restart_instance_gateway_for(instance_id: &str, kind: &str) -> Result<(), String> {
    let target = find_instance_gateway_target(instance_id, kind)?;
    let runtime_key = provider_gateway_runtime_key(&target.profile_dir, &target.runtime_id);
    set_instance_gateway_recovery_error(&runtime_key, None);
    let result = ensure_instance_gateway_started(&target).await;
    if let Err(error) = result.as_ref() {
        set_instance_gateway_recovery_error(&runtime_key, Some(error.clone()));
    }
    result
}

/// 网关已健康（本次不重新走接管流程）时同步 DeepSeek 参数兼容配置。
///
/// 升级场景下实例可能仍在运行、网关也一直健康，此时启动自愈不会重新接管 profile，
/// 这里按 profile 幂等同步有效参数，并清理有备份依据的旧受管压缩开关；
/// 失败只记日志，不影响网关可用性。
fn ensure_instance_gateway_profile_overrides(target: &InstanceGatewayTarget) {
    if target.kind != INSTANCE_GATEWAY_KIND_PROVIDER {
        return;
    }
    if let Err(error) =
        reapply_deepseek_profile_config_overrides(&target.profile_dir, &target.account)
    {
        logger::log_codex_api_warn(&format!(
            "[CodexLocalAccess][instance-gateway] 同步 DeepSeek 参数兼容配置失败: instance_id={}, profile={}, error={}",
            target.instance_id,
            target.profile_dir.display(),
            error
        ));
    }
}

/// 取当前由本进程托管且仍然存活的 sidecar 端点（端口 + 客户端密钥）。
///
/// provider gateway / 绑定 OAuth 网关的端口是每次启动随机分配的，且不写入 state.json，
/// 因此只有运行态内存里的 collection 才是权威来源；state.json 仅用于混合模型路由的持久端口。
async fn live_provider_gateway_endpoints() -> HashMap<String, (u16, String)> {
    let mut endpoints = HashMap::new();
    let mut runtimes = provider_gateway_runtime_store().lock().await;
    for (key, runtime) in runtimes.iter_mut() {
        let alive = runtime
            .sidecar_child
            .as_mut()
            .is_some_and(|child| matches!(child.try_wait(), Ok(None)));
        if !alive {
            continue;
        }
        if let Some(collection) = runtime.collection.as_ref() {
            endpoints.insert(key.clone(), (collection.port, collection.api_key.clone()));
        }
    }
    endpoints
}

async fn instance_gateway_port_accepts_connections(port: u16) -> bool {
    matches!(
        timeout(
            INSTANCE_GATEWAY_PORT_PROBE_TIMEOUT,
            TcpStream::connect((CODEX_LOCAL_ACCESS_LOCALHOST_BIND_HOST, port)),
        )
        .await,
        Ok(Ok(_))
    )
}

/// 取实例网关当前可用端点：优先内存中的真实运行态，其次混合模型路由持久化的端口。
async fn current_instance_gateway_endpoint(
    profile_dir: &Path,
    runtime_id: &str,
) -> Option<(u16, String)> {
    let runtime_key = provider_gateway_runtime_key(profile_dir, runtime_id);
    {
        let mut runtimes = provider_gateway_runtime_store().lock().await;
        if let Some(runtime) = runtimes.get_mut(&runtime_key) {
            let alive = runtime
                .sidecar_child
                .as_mut()
                .is_some_and(|child| matches!(child.try_wait(), Ok(None)));
            if alive {
                if let Some(collection) = runtime.collection.as_ref() {
                    return Some((collection.port, collection.api_key.clone()));
                }
            }
        }
    }
    let state = load_provider_gateway_profile_state(profile_dir, runtime_id)
        .ok()
        .flatten()?;
    let port = state.port.filter(|port| *port > 0)?;
    Some((port, state.api_key))
}

async fn instance_gateway_runtime_is_healthy(profile_dir: &Path, runtime_id: &str) -> bool {
    let Some((port, api_key)) = current_instance_gateway_endpoint(profile_dir, runtime_id).await
    else {
        return false;
    };
    if api_key.trim().is_empty() {
        return false;
    }
    probe_sidecar_ready_endpoint(port, &api_key, INSTANCE_GATEWAY_READY_PROBE_TIMEOUT)
        .await
        .is_ok()
}

/// 放弃已持久化的端口，下一次启动会重新分配一个空闲端口。
///
/// 只在同端口启动失败时调用：端口可能已被其它实例的网关或外部进程占用。
fn reset_instance_gateway_profile_port(profile_dir: &Path, runtime_id: &str) {
    let Ok(Some(mut state)) = load_provider_gateway_profile_state(profile_dir, runtime_id) else {
        return;
    };
    if state.port.is_none() {
        return;
    }
    state.port = None;
    state.updated_at = now_ms();
    if let Err(error) = save_provider_gateway_profile_state(profile_dir, runtime_id, &state) {
        logger::log_codex_api_warn(&format!(
            "[CodexLocalAccess][instance-gateway] 重置持久端口失败: profile={}, runtime_id={}, error={}",
            profile_dir.display(),
            runtime_id,
            error
        ));
    }
}

async fn ensure_instance_gateway_started(target: &InstanceGatewayTarget) -> Result<(), String> {
    match target.kind {
        INSTANCE_GATEWAY_KIND_PROVIDER => {
            ensure_provider_gateway_for_dir(&target.profile_dir, &target.runtime_id).await
        }
        INSTANCE_GATEWAY_KIND_BOUND_OAUTH => {
            ensure_bound_oauth_local_gateway_for_dir(&target.profile_dir, &target.runtime_id).await
        }
        INSTANCE_GATEWAY_KIND_MIXED_MODEL => {
            let routing = crate::modules::codex_instance::load_enabled_model_routing(
                &target.instance_id,
            )?;
            ensure_mixed_model_gateway_for_dir(&target.profile_dir, &target.account.id, &routing)
                .await
        }
        _ => Ok(()),
    }
}

/// 启动自愈：宿主重启后重建此前已启用的实例级网关。
///
/// 与 API 服务保持一致的语义：只要 profile 仍然绑定需要网关的账号，并且该网关此前启动过
/// （sidecar 目录存在）或对应 Codex 实例仍在运行，就在后台把它拉起来。混合模型路由由既有
/// watchdog 负责，这里跳过，避免两个恢复流程互相抢启动。
pub async fn restore_instance_gateways_on_startup() {
    let targets = match collect_instance_gateway_targets() {
        Ok(targets) => targets,
        Err(error) => {
            logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess][instance-gateway] 启动恢复读取实例失败，跳过本次恢复: {}",
                error
            ));
            return;
        }
    };
    if targets.is_empty() {
        return;
    }
    let process_entries = crate::modules::process::collect_codex_process_entries();

    for target in targets {
        if target.kind == INSTANCE_GATEWAY_KIND_MIXED_MODEL {
            continue;
        }
        if crate::modules::app_lifecycle::is_shutdown_started() {
            return;
        }
        let runtime_key = provider_gateway_runtime_key(&target.profile_dir, &target.runtime_id);
        let instance_running = {
            let home =
                (!target.is_default).then(|| target.profile_dir.to_string_lossy().to_string());
            crate::modules::process::resolve_codex_pid_from_entries(
                target.last_pid,
                home.as_deref(),
                &process_entries,
            )
            .is_some()
        };
        if !instance_running {
            // 实例没运行时不再保留实例级网关：停掉残留进程并还原接管状态，
            // 等下次通过 Cockpit 启动该实例时再重建。
            if let Err(error) =
                release_instance_gateway_for_profile(&target.profile_dir).await
            {
                logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess][instance-gateway] 清理未运行实例的网关失败: kind={}, instance_id={}, error={}",
                    target.kind, target.instance_id, error
                ));
            }
            continue;
        }
        if instance_gateway_runtime_is_healthy(&target.profile_dir, &target.runtime_id).await {
            set_instance_gateway_recovery_error(&runtime_key, None);
            ensure_instance_gateway_profile_overrides(&target);
            continue;
        }

        logger::log_codex_api_info(&format!(
            "[CodexLocalAccess][instance-gateway] 启动恢复开始: kind={}, instance_id={}, runtime_id={}, profile={}",
            target.kind,
            target.instance_id,
            target.runtime_id,
            target.profile_dir.display()
        ));
        let mut last_error = String::new();
        let mut recovered = false;
        for attempt in 1..=INSTANCE_GATEWAY_RECOVERY_ATTEMPTS {
            if attempt > 1 {
                tokio::time::sleep(INSTANCE_GATEWAY_RECOVERY_RETRY_DELAY).await;
            }
            if crate::modules::app_lifecycle::is_shutdown_started() {
                return;
            }
            match ensure_instance_gateway_started(&target).await {
                Ok(()) => {
                    recovered = true;
                    break;
                }
                Err(error) => {
                    last_error = error;
                    logger::log_codex_api_warn(&format!(
                        "[CodexLocalAccess][instance-gateway] 启动恢复失败: kind={}, instance_id={}, runtime_id={}, attempt={}, error={}",
                        target.kind, target.instance_id, target.runtime_id, attempt, last_error
                    ));
                    // 下次尝试前放弃持久端口：端口可能已被其它进程占用，换一个空闲端口再试。
                    reset_instance_gateway_profile_port(&target.profile_dir, &target.runtime_id);
                }
            }
        }
        if recovered {
            set_instance_gateway_recovery_error(&runtime_key, None);
            logger::log_codex_api_info(&format!(
                "[CodexLocalAccess][instance-gateway] 启动恢复完成: kind={}, instance_id={}, runtime_id={}",
                target.kind, target.instance_id, target.runtime_id
            ));
        } else {
            set_instance_gateway_recovery_error(&runtime_key, Some(last_error));
        }
    }
}

async fn resolve_instance_gateway_status(
    port: Option<u16>,
    api_key: &str,
    managed_running: bool,
    ever_started: bool,
) -> &'static str {
    let Some(port) = port else {
        return if ever_started {
            INSTANCE_GATEWAY_STATUS_STOPPED
        } else {
            INSTANCE_GATEWAY_STATUS_NOT_STARTED
        };
    };
    if !api_key.trim().is_empty()
        && probe_sidecar_ready_endpoint(port, api_key, INSTANCE_GATEWAY_READY_PROBE_TIMEOUT)
            .await
            .is_ok()
    {
        return INSTANCE_GATEWAY_STATUS_RUNNING;
    }
    if managed_running {
        return INSTANCE_GATEWAY_STATUS_UNREACHABLE;
    }
    if instance_gateway_port_accepts_connections(port).await {
        // 端口有人监听但不是我们的 sidecar（密钥校验失败），提示端口被占用。
        return INSTANCE_GATEWAY_STATUS_PORT_CONFLICT;
    }
    INSTANCE_GATEWAY_STATUS_STOPPED
}

fn build_instance_gateway_view(
    target: InstanceGatewayTarget,
    port: Option<u16>,
    managed_running: bool,
    status: &'static str,
) -> CodexInstanceGatewayView {
    let account = target.account;
    let last_error =
        instance_gateway_recovery_error(&provider_gateway_runtime_key(
            &target.profile_dir,
            &target.runtime_id,
        ));
    let account_label = account
        .account_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            let email = account.email.trim();
            (!email.is_empty()).then(|| email.to_string())
        });
    let is_mixed_model = target.kind == INSTANCE_GATEWAY_KIND_MIXED_MODEL;
    let log_api_key_id = if is_mixed_model {
        mixed_model_routing_api_key_id()
    } else {
        provider_gateway_api_key_id(&target.runtime_id)
    };

    CodexInstanceGatewayView {
        id: format!(
            "{}::{}",
            normalize_profile_dir_key(&target.profile_dir),
            target.runtime_id
        ),
        kind: target.kind.to_string(),
        runtime_id: target.runtime_id,
        profile_dir: target.profile_dir.to_string_lossy().to_string(),
        instance_id: target.instance_id,
        instance_name: target.instance_name,
        is_default: target.is_default,
        account_id: Some(account.id.clone()),
        account_label,
        bind_host: CODEX_LOCAL_ACCESS_LOCALHOST_BIND_HOST.to_string(),
        port,
        base_url: port.map(build_base_url),
        wire_api: (!is_mixed_model).then(|| provider_gateway_wire_api_for_account(&account)),
        upstream_models: if is_mixed_model {
            Vec::new()
        } else {
            provider_gateway_models_for_account(&account)
        },
        status: status.to_string(),
        managed: managed_running,
        log_api_key_id,
        last_error,
    }
}

/// 列出所有实例级本地网关的只读快照。探测并发执行，单个网关最多等待几百毫秒。
pub async fn snapshot_instance_gateways() -> Result<Vec<CodexInstanceGatewayView>, String> {
    // 实例级网关只在实例运行时存在：实例已关闭就不显示，也不做任何自愈。
    let process_entries = crate::modules::process::collect_codex_process_entries();
    let targets = collect_instance_gateway_targets()?
        .into_iter()
        .filter(|target| instance_target_is_running(target, &process_entries))
        .collect::<Vec<_>>();
    if targets.is_empty() {
        return Ok(Vec::new());
    }
    let live_endpoints = live_provider_gateway_endpoints().await;

    let pending = targets
        .into_iter()
        .map(|target| {
            let runtime_key = provider_gateway_runtime_key(
                &target.profile_dir,
                &target.runtime_id,
            );
            // 生成过的 sidecar 目录说明这个 profile 曾经接管过，用于区分“未启动”与“已停止”。
            let ever_started = provider_gateway_sidecar_dir(&target.profile_dir, &target.runtime_id)
                .map(|dir| sidecar_config_path(&dir).exists())
                .unwrap_or(false);
            if let Some((port, api_key)) = live_endpoints.get(&runtime_key) {
                return (target, Some(*port), api_key.clone(), true, ever_started);
            }
            let state = load_provider_gateway_profile_state(&target.profile_dir, &target.runtime_id)
                .ok()
                .flatten();
            let port = state
                .as_ref()
                .and_then(|state| state.port)
                .filter(|port| *port > 0);
            let api_key = state.map(|state| state.api_key).unwrap_or_default();
            (target, port, api_key, false, ever_started)
        })
        .collect::<Vec<_>>();

    let statuses = futures::future::join_all(
        pending
            .iter()
            .map(|(_, port, api_key, managed_running, ever_started)| {
                resolve_instance_gateway_status(*port, api_key, *managed_running, *ever_started)
            }),
    )
    .await;

    let mut views = pending
        .into_iter()
        .zip(statuses)
        .map(|((target, port, _, managed_running, _), status)| {
            if status == INSTANCE_GATEWAY_STATUS_RUNNING {
                set_instance_gateway_recovery_error(
                    &provider_gateway_runtime_key(&target.profile_dir, &target.runtime_id),
                    None,
                );
            }
            build_instance_gateway_view(target, port, managed_running, status)
        })
        .collect::<Vec<_>>();
    views.sort_by(|left, right| {
        right
            .is_default
            .cmp(&left.is_default)
            .then_with(|| left.instance_name.cmp(&right.instance_name))
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(views)
}

#[cfg(test)]
mod instance_gateway_tests {
    use super::*;

    fn test_api_key_account() -> CodexAccount {
        let mut account = CodexAccount::new_api_key(
            "codex_apikey_test".to_string(),
            "api-key-test".to_string(),
            "sk-upstream".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.example.com/v1".to_string()),
            Some("example".to_string()),
            Some("Example Provider".to_string()),
            Vec::new(),
        );
        account.api_model_catalog = vec!["deepseek-chat".to_string()];
        account
    }

    fn test_target(kind: &'static str, runtime_id: &str) -> InstanceGatewayTarget {
        InstanceGatewayTarget {
            instance_id: "inst-1".to_string(),
            instance_name: "Instance 1".to_string(),
            is_default: false,
            last_pid: None,
            profile_dir: PathBuf::from("/tmp/cockpit-instance-gateway-test"),
            kind,
            runtime_id: runtime_id.to_string(),
            account: test_api_key_account(),
        }
    }

    #[test]
    fn provider_gateway_view_exposes_address_wire_api_and_log_key() {
        let view = build_instance_gateway_view(
            test_target(INSTANCE_GATEWAY_KIND_PROVIDER, "codex_apikey_test"),
            Some(53118),
            true,
            INSTANCE_GATEWAY_STATUS_RUNNING,
        );
        assert_eq!(view.kind, "providerGateway");
        assert_eq!(view.base_url.as_deref(), Some("http://localhost:53118/v1"));
        assert_eq!(view.port, Some(53118));
        assert_eq!(view.status, "running");
        assert!(view.managed);
        assert_eq!(view.log_api_key_id, "provider_gateway_codex_apikey_test");
        assert!(view.wire_api.is_some());
        assert!(!view.upstream_models.is_empty());
    }

    #[test]
    fn mixed_model_view_uses_mixed_log_key_and_no_wire_api() {
        let view = build_instance_gateway_view(
            test_target(
                INSTANCE_GATEWAY_KIND_MIXED_MODEL,
                MIXED_MODEL_ROUTING_RUNTIME_ID,
            ),
            None,
            false,
            INSTANCE_GATEWAY_STATUS_STOPPED,
        );
        assert_eq!(view.kind, "mixedModel");
        assert_eq!(view.base_url, None);
        assert_eq!(view.log_api_key_id, "mixed_model_routing");
        assert_eq!(view.wire_api, None);
        assert!(view.upstream_models.is_empty());
        assert_eq!(view.status, "stopped");
        assert!(!view.managed);
    }

    #[test]
    fn recovery_error_store_tracks_and_clears_runtime_keys() {
        let key = "profile\ninstance-gateway-recovery-test";
        assert!(instance_gateway_recovery_error(key).is_none());
        set_instance_gateway_recovery_error(key, Some("boom".to_string()));
        assert_eq!(instance_gateway_recovery_error(key).as_deref(), Some("boom"));
        set_instance_gateway_recovery_error(key, None);
        assert!(instance_gateway_recovery_error(key).is_none());
    }
}
