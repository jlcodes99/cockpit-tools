// Codex 账号模块：Token refresh classification, runtime freshness and account identity extraction。
// 通过 include! 保持原 modules::codex_account 作用域，完整保留私有调用关系。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexRefreshErrorKind {
    RefreshTokenReused,
    RefreshTokenExpired,
    RefreshTokenInvalidated,
    InvalidGrant,
    UnsupportedCountryRegion,
    Other,
}

fn classify_refresh_error(message: &str) -> CodexRefreshErrorKind {
    let lower = message.to_ascii_lowercase();
    if lower.contains("unsupported_country_region_territory") {
        return CodexRefreshErrorKind::UnsupportedCountryRegion;
    }
    if lower.contains("refresh_token_reused") {
        return CodexRefreshErrorKind::RefreshTokenReused;
    }
    if lower.contains("refresh_token_expired") {
        return CodexRefreshErrorKind::RefreshTokenExpired;
    }
    if lower.contains("refresh_token_invalidated")
        || lower.contains("token_invalidated")
        || lower.contains("authentication token has been invalidated")
        || lower.contains("服务端撤销")
    {
        return CodexRefreshErrorKind::RefreshTokenInvalidated;
    }
    if lower.contains("invalid_grant")
        || lower.contains("invalid_refresh_token")
        || lower.contains("invalid refresh token")
    {
        return CodexRefreshErrorKind::InvalidGrant;
    }
    if lower.contains("status=401") || lower.contains("401 unauthorized") {
        return CodexRefreshErrorKind::InvalidGrant;
    }
    CodexRefreshErrorKind::Other
}

fn is_reauth_required_refresh_error(message: &str) -> bool {
    matches!(
        classify_refresh_error(message),
        CodexRefreshErrorKind::RefreshTokenExpired
            | CodexRefreshErrorKind::RefreshTokenInvalidated
            | CodexRefreshErrorKind::InvalidGrant
    )
}

pub(crate) fn is_refresh_token_reused_error(message: &str) -> bool {
    matches!(
        classify_refresh_error(message),
        CodexRefreshErrorKind::RefreshTokenReused
    )
}

/// 清理旧版本留下的 refresh_token_reused 状态。
///
/// 该错误不再作为账号健康状态或切号条件；仅保留手动强制刷新时的即时错误结果。
fn clear_refresh_token_reused_state(account: &mut CodexAccount) -> Result<(), String> {
    let reauth_reused = account
        .reauth_reason
        .as_deref()
        .is_some_and(is_refresh_token_reused_error);
    let quota_reused = account.quota_error.as_ref().is_some_and(|error| {
        error.code.as_deref().is_some_and(is_refresh_token_reused_error)
            || is_refresh_token_reused_error(&error.message)
    });
    if !reauth_reused && !quota_reused {
        return Ok(());
    }
    if reauth_reused {
        account.requires_reauth = false;
        account.reauth_reason = None;
    }
    if quota_reused {
        account.quota_error = None;
    }
    save_account(account)
}

/// 服务端明确撤销授权是账号级终止状态，不能再降级为仅客户端需授权。
fn is_server_revoked_refresh_error(message: &str) -> bool {
    matches!(
        classify_refresh_error(message),
        CodexRefreshErrorKind::RefreshTokenInvalidated
    )
}

fn format_refresh_error_for_user(raw: &str) -> String {
    match classify_refresh_error(raw) {
        CodexRefreshErrorKind::RefreshTokenReused => format!(
            "Codex 授权已失效：refresh_token 已被其它客户端或实例使用过。Codex 的 refresh_token 是轮换凭据，旧凭据再次刷新会被服务端拒绝。请重新登录，并避免官方 Codex、其它实例或外部工具同时刷新同一账号。原始错误: {}",
            raw
        ),
        CodexRefreshErrorKind::RefreshTokenExpired => format!(
            "Codex 登录授权已过期，无法自动刷新。请重新登录 Codex 账号。原始错误: {}",
            raw
        ),
        CodexRefreshErrorKind::RefreshTokenInvalidated => format!(
            "Codex 登录授权已被服务端撤销，无法自动刷新。请重新登录 Codex 账号。原始错误: {}",
            raw
        ),
        CodexRefreshErrorKind::InvalidGrant => format!(
            "Codex 登录授权无效，无法自动刷新。请重新登录 Codex 账号。原始错误: {}",
            raw
        ),
        CodexRefreshErrorKind::UnsupportedCountryRegion => format!(
            "当前网络地区不支持刷新 Codex 授权。OpenAI 授权服务拒绝了当前网络出口的刷新请求，请切换到支持的网络地区后重试。原始错误: {}",
            raw
        ),
        CodexRefreshErrorKind::Other => format!("Token 已过期且刷新失败: {}", raw),
    }
}

const CODEX_SWITCH_AUTH_REQUIRED_PREFIX: &str = "CODEX_SWITCH_AUTH_REQUIRED:";

fn switch_auth_reason_code(reason: &str) -> &'static str {
    match classify_refresh_error(reason) {
        CodexRefreshErrorKind::RefreshTokenReused => "refresh_token_reused",
        CodexRefreshErrorKind::RefreshTokenExpired => "refresh_token_expired",
        CodexRefreshErrorKind::RefreshTokenInvalidated => "refresh_token_invalidated",
        CodexRefreshErrorKind::InvalidGrant => "invalid_grant",
        CodexRefreshErrorKind::UnsupportedCountryRegion => "unsupported_country_region",
        CodexRefreshErrorKind::Other if reason.contains("id_token") => "id_token_unavailable",
        CodexRefreshErrorKind::Other if is_missing_refresh_token_reason(reason) => {
            "missing_refresh_token"
        }
        CodexRefreshErrorKind::Other => "authorization_required",
    }
}

/// 为前端切号弹框补充可机器识别的授权状态。
///
/// 只有账号已经被 Token Authority 明确标记为需要重新授权时才包装错误；
/// 其它启动、落盘或网络地区错误仍保持原错误，避免误导用户重新登录。
pub(crate) fn format_account_switch_error(account_id: &str, error: String) -> String {
    // 统一错误可能经过账号切换、默认实例和 API 服务多层转发；已经带有结构化
    // 授权标记时直接透传，避免重复嵌套并破坏前端解析。
    if error.trim_start().starts_with(CODEX_SWITCH_AUTH_REQUIRED_PREFIX) {
        return error;
    }
    let Some(account) = load_account(account_id) else {
        return error;
    };
    if account
        .reauth_reason
        .as_deref()
        .is_some_and(is_refresh_token_reused_error)
    {
        return error;
    }
    // CDP 客户端登录页观测只用于账号卡片展示，不把任何普通切号/启动错误包装成
    // 授权失败弹框。只有 Token Authority 明确写入 requires_reauth 时才进入此状态。
    if !account.requires_reauth {
        return error;
    }

    let reason = account
        .reauth_reason
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(error);
    let payload = serde_json::json!({
        "accountId": account.id,
        "reasonCode": switch_auth_reason_code(&reason),
        // 本地 JWT 尚未到 exp 不能覆盖已确认的远端 API 401/403；否则切号弹框
        // 会把实际不可用的账号误报成“API 服务可用”。
        "apiOnlyAvailable": !is_server_revoked_refresh_error(&reason)
            && !codex_oauth::is_token_expired(&account.tokens.access_token)
            && !account_has_remote_api_auth_rejection(&account),
        "accessTokenExpiresAt": codex_oauth::jwt_token_expiration_timestamp(
            &account.tokens.access_token,
        ),
        "message": reason,
    });
    format!("{}{}", CODEX_SWITCH_AUTH_REQUIRED_PREFIX, payload)
}

fn mark_account_requires_reauth(account: &mut CodexAccount, reason: &str) -> Result<(), String> {
    account.requires_reauth = true;
    account.reauth_reason = Some(reason.to_string());
    save_account(account)
}

fn is_missing_refresh_token_reason(reason: &str) -> bool {
    reason.contains("缺少 refresh_token")
}

pub(crate) fn account_has_refresh_token(account: &CodexAccount) -> bool {
    account
        .tokens
        .refresh_token
        .as_deref()
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .is_some()
}

/// 持久化官方客户端实际页面观察结果；重新读取最新账号后只更新观测字段，
/// 避免 CDP 后台任务把较旧的 Token 快照写回账号库。
pub(crate) async fn update_client_auth_observation(
    account_id: &str,
    instance_id: &str,
    status: &str,
    login_redirect: bool,
) -> Result<(), String> {
    let token_lock = codex_token_lock_for(account_id);
    let _token_guard = token_lock.lock().await;
    let _file_guard = acquire_codex_token_refresh_file_lock(account_id, "client-auth-observation").await?;
    let _guard = CODEX_ACCOUNT_MUTATION_LOCK
        .lock()
        .map_err(|_| "Codex 账号写入锁已损坏".to_string())?;
    let Some(mut account) = load_account(account_id) else {
        return Err(format!("账号不存在: {}", account_id));
    };
    let now = now_timestamp();
    account.client_auth_status = Some(status.to_string());
    account.last_client_auth_observed_at = Some(now);
    account.last_client_auth_instance_id = Some(instance_id.to_string());
    if login_redirect {
        account.last_client_login_redirect_at = Some(now);
    }
    save_account_with_tombstone_guard(&account)
}

/// 记录实例本次启动（或恢复监控）的时间，不触碰任何 Token 或授权状态。
pub(crate) async fn record_client_launch(
    account_id: &str,
    instance_id: &str,
    launched_at: i64,
) -> Result<(), String> {
    let token_lock = codex_token_lock_for(account_id);
    let _token_guard = token_lock.lock().await;
    let _file_guard =
        acquire_codex_token_refresh_file_lock(account_id, "client-launch-observation").await?;
    let _guard = CODEX_ACCOUNT_MUTATION_LOCK
        .lock()
        .map_err(|_| "Codex 账号写入锁已损坏".to_string())?;
    let Some(mut account) = load_account(account_id) else {
        return Err(format!("账号不存在: {}", account_id));
    };
    account.last_client_launch_at = Some(launched_at);
    account.last_client_auth_instance_id = Some(instance_id.to_string());
    save_account_with_tombstone_guard(&account)
}

/// 清除官方客户端登录页观测状态。
///
/// 这里只清理 CDP 观察字段，不清理 Token Authority 明确写入的
/// `requires_reauth`/`reauth_reason`，也不修改任何 Token，避免把真实的远端凭据
/// 失效伪装成正常。客户端观测状态只用于账号卡片展示，用户可随时手动清理。
pub(crate) async fn clear_client_auth_observation(
    account_id: &str,
) -> Result<bool, String> {
    let token_lock = codex_token_lock_for(account_id);
    let _token_guard = token_lock.lock().await;
    let _file_guard =
        acquire_codex_token_refresh_file_lock(account_id, "clear-client-auth-observation").await?;
    let _guard = CODEX_ACCOUNT_MUTATION_LOCK
        .lock()
        .map_err(|_| "Codex 账号写入锁已损坏".to_string())?;
    let Some(mut account) = load_account(account_id) else {
        return Err(format!("账号不存在: {}", account_id));
    };
    let had_observation = account.client_auth_status.is_some()
        || account.last_client_auth_observed_at.is_some()
        || account.last_client_login_redirect_at.is_some()
        || account.last_client_launch_at.is_some()
        || account.last_client_auth_instance_id.is_some();
    if !had_observation {
        return Ok(false);
    }
    account.client_auth_status = None;
    account.last_client_auth_observed_at = None;
    account.last_client_login_redirect_at = None;
    account.last_client_launch_at = None;
    account.last_client_auth_instance_id = None;
    save_account_with_tombstone_guard(&account)?;
    crate::modules::codex_auth_diagnostic::log_event(
        "client_auth_observation_cleared_by_user",
        serde_json::json!({
            "account_id": account.id,
            "email": account.email,
            "reason": "user_clear_client_auth_observation",
        }),
    );
    Ok(true)
}

pub(crate) fn managed_account_tokens_need_refresh(account: &CodexAccount) -> bool {
    // Codex app-server authenticates requests with access_token. An OAuth
    // refresh response may omit id_token, so treating an expired id_token as
    // a mandatory refresh condition would repeatedly rotate refresh_token and
    // eventually invalidate the account even while access_token is healthy.
    codex_oauth::is_token_expired(&account.tokens.access_token)
}

/// 判断额度/API 请求是否已经得到远端鉴权拒绝。
///
/// refresh_token 自身失效并不代表尚未过期的 access_token 立即失效；这里只匹配
/// 已明确标记为 API 请求 401/403 或 access-token 被撤销的错误，供 API 服务路由
/// 清理缓存并跳过该账号。
pub(crate) fn account_has_remote_api_auth_rejection(account: &CodexAccount) -> bool {
    let Some(error) = account.quota_error.as_ref() else {
        return false;
    };
    let message = error.message.trim();
    let lower = message.to_ascii_lowercase();
    let code = error
        .code
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let refresh_failure = lower.contains("刷新 token")
        || lower.contains("token 刷新")
        || lower.contains("refresh_token")
        || lower.contains("refresh token");
    if lower.contains("api 返回错误 401")
        || lower.contains("api 返回错误 403")
        || lower.contains("token_invalidated")
        || lower.contains("invalid_token")
        || lower.contains("your authentication token has been invalidated")
    {
        return !refresh_failure
            && !matches!(code.as_str(), "refresh_token_reused" | "refresh_token_expired");
    }
    !refresh_failure
        && matches!(code.as_str(), "token_invalidated" | "invalid_token")
}

/// 额度查询只依赖 access_token。官方客户端占用 refresh_token 时，属于内部协调状态，
/// 不应被持久化为配额错误或覆盖已有额度。
pub(crate) fn is_refresh_ownership_deferred_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("官方 chatgpt/codex 客户端正在使用此账号")
        || lower.contains("无法确认官方 chatgpt/codex 客户端是否正在使用此账号")
        || lower.contains("实例启动或受控转移")
        || lower.contains("为避免重复轮换 refresh_token")
}

/// 一轮额度刷新共享的 Codex 运行态快照。
///
/// 系统进程探测在 Windows 上会启动 PowerShell，因此批量刷新必须先采集一次，
/// 再由所有账号复用。只有真正进入 refresh_token 临界区时才重新采集。
#[derive(Clone)]
pub(crate) struct CodexQuotaRuntimeSnapshot {
    process_entries: Arc<Vec<(u32, Option<String>)>>,
    running_oauth_account_ids: Result<Arc<HashSet<String>>, Arc<String>>,
}

impl CodexQuotaRuntimeSnapshot {
    pub(crate) fn empty() -> Self {
        Self {
            process_entries: Arc::new(Vec::new()),
            running_oauth_account_ids: Ok(Arc::new(HashSet::new())),
        }
    }

    pub(crate) async fn capture() -> Result<Self, String> {
        tokio::task::spawn_blocking(Self::capture_blocking)
            .await
            .map_err(|error| format!("采集 Codex 额度运行态失败: {}", error))
    }

    fn capture_blocking() -> Self {
        let process_entries = crate::modules::process::collect_codex_process_entries();
        let running_oauth_account_ids =
            running_codex_oauth_account_ids_from_entries(&process_entries)
                .map(Arc::new)
                .map_err(Arc::new);
        Self {
            process_entries: Arc::new(process_entries),
            running_oauth_account_ids,
        }
    }

    pub(crate) fn process_entries(&self) -> &[(u32, Option<String>)] {
        self.process_entries.as_slice()
    }

    fn has_running_oauth_account(&self, account_id: &str) -> bool {
        self.running_oauth_account_ids
            .as_ref()
            .map(|account_ids| account_ids.contains(account_id))
            .unwrap_or(false)
    }

    pub(crate) fn running_oauth_account_ids(&self) -> Result<&HashSet<String>, &str> {
        self.running_oauth_account_ids
            .as_ref()
            .map(|account_ids| account_ids.as_ref())
            .map_err(|error| error.as_str())
    }
}

/// 额度查询专用凭据准备：只在 access_token 过期时尝试 Token Authority 刷新。
/// id_token 临期、8 天保活周期和历史 requires_reauth 标记都不应阻断额度请求。
pub async fn prepare_account_for_quota_query(account_id: &str) -> Result<CodexAccount, String> {
    let account = load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if account.is_api_key_auth()
        || account.is_agent_identity_auth()
        || account.is_web_session_auth()
    {
        return Ok(account);
    }
    let runtime_snapshot = CodexQuotaRuntimeSnapshot::capture().await?;
    prepare_account_for_quota_query_with_runtime_snapshot(account_id, &runtime_snapshot).await
}

pub(crate) async fn prepare_account_for_quota_query_with_runtime_snapshot(
    account_id: &str,
    runtime_snapshot: &CodexQuotaRuntimeSnapshot,
) -> Result<CodexAccount, String> {
    let lock = codex_token_lock_for(account_id);
    let _guard = lock.lock().await;
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;

    if account.is_api_key_auth()
        || account.is_agent_identity_auth()
        || account.is_web_session_auth()
    {
        return Ok(account);
    }

    let official_runtime_has_account = runtime_snapshot.has_running_oauth_account(&account.id);
    let sync_result = if official_runtime_has_account {
        sync_account_from_live_authority_sources_with_entries(
            &mut account,
            runtime_snapshot.process_entries(),
        )
    } else {
        sync_account_from_authority_sources_with_entries(
            &mut account,
            runtime_snapshot.process_entries(),
        )
    };
    if let Err(error) = sync_result {
        logger::log_warn(&format!(
            "Codex 额度查询前同步官方凭据失败，继续使用当前 access_token: account_id={}, error={}",
            account.id, error
        ));
    }

    clear_refresh_token_reused_state(&mut account)?;

    if account
        .quota_error
        .as_ref()
        .is_some_and(|error| is_refresh_ownership_deferred_error(&error.message))
    {
        account.quota_error = None;
        save_account(&account)?;
    }

    let access_token_expired = codex_oauth::is_token_expired(&account.tokens.access_token);
    crate::modules::codex_auth_diagnostic::log_event(
        "quota_prepare_token_decision",
        serde_json::json!({
            "account_id": account.id,
            "token_generation": account.token_generation,
            "access_token_expired": access_token_expired,
            "id_token_expired": codex_oauth::is_id_token_expired(&account.tokens.id_token),
            "requires_reauth": account.requires_reauth,
            "tokens": crate::modules::codex_auth_diagnostic::tokens_summary(&account.tokens),
        }),
    );
    if !access_token_expired {
        return Ok(account);
    }

    // 只有 AT 确认过期后才进入跨进程 RT 临界区。等待期间若其它 Cockpit 或
    // 官方客户端已经写回新 AT，重新加载后直接复用，不再次轮换 RT。
    let _file_guard = acquire_codex_token_refresh_file_lock(account_id, "quota-query").await?;
    account = load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if !codex_oauth::is_token_expired(&account.tokens.access_token) {
        return Ok(account);
    }
    let fresh_runtime_snapshot = CodexQuotaRuntimeSnapshot::capture().await?;
    let official_runtime_has_account =
        fresh_runtime_snapshot.has_running_oauth_account(&account.id);
    let sync_result = if official_runtime_has_account {
        sync_account_from_live_authority_sources_with_entries(
            &mut account,
            fresh_runtime_snapshot.process_entries(),
        )
    } else {
        sync_account_from_authority_sources_with_entries(
            &mut account,
            fresh_runtime_snapshot.process_entries(),
        )
    };
    if let Err(error) = sync_result {
        logger::log_warn(&format!(
            "Codex 额度查询进入 RT 临界区后同步官方凭据失败: account_id={}, error={}",
            account.id, error
        ));
    }
    if !codex_oauth::is_token_expired(&account.tokens.access_token) {
        return Ok(account);
    }

    if account.requires_reauth {
        return Err(account
            .reauth_reason
            .clone()
            .unwrap_or_else(|| "账号需要重新授权".to_string()));
    }

    perform_managed_token_refresh(account, "额度查询前 access_token 已过期", false).await
}

/// 客户端和后台请求均以 `access_token` 作为刷新依据；`id_token` 仅作为
/// 身份展示信息保存，不因本地 exp 过期而额外轮换 refresh_token。
pub(crate) fn managed_account_runtime_tokens_need_refresh(account: &CodexAccount) -> bool {
    codex_oauth::is_token_expired(&account.tokens.access_token)
}

fn managed_account_refresh_needed_for_request(
    account: &CodexAccount,
    refresh_id_token_for_client: bool,
    revalidate_known_reauth: bool,
) -> bool {
    let token_refresh_due = if refresh_id_token_for_client {
        managed_account_runtime_tokens_need_refresh(account)
    } else {
        managed_account_tokens_need_refresh(account)
    };
    token_refresh_due && (!account.requires_reauth || revalidate_known_reauth)
}

fn finish_managed_runtime_account_refresh(
    account: CodexAccount,
    validate_for_client: bool,
) -> Result<CodexAccount, String> {
    let _ = validate_for_client;
    Ok(account)
}

pub(crate) fn oauth_account_id_for_runtime_binding(binding_id: Option<&str>) -> Option<String> {
    let binding_id = binding_id
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    if crate::modules::codex_instance::is_api_service_bind_account_id(binding_id) {
        return None;
    }
    if let Some(account_id) =
        crate::modules::codex_instance::parse_provider_gateway_bind_account_id(binding_id)
    {
        let account = load_account(&account_id)?;
        return if account.is_api_key_auth() {
            account.bound_oauth_account_id.clone()
        } else if account.is_agent_identity_auth() || account.is_web_session_auth() {
            None
        } else {
            Some(account.id)
        };
    }

    let account = load_account(binding_id)?;
    if account.is_api_key_auth() {
        return account.bound_oauth_account_id.clone();
    }
    if account.is_agent_identity_auth() || account.is_web_session_auth() {
        return None;
    }
    Some(account.id)
}

fn oauth_account_id_for_runtime_snapshot(
    base_dir: &Path,
    accounts: &[CodexAccount],
) -> Option<String> {
    let snapshot = load_local_oauth_snapshot_from_official_store(base_dir)?;
    accounts
        .iter()
        .find(|account| {
            !account.is_api_key_auth()
                && !account.is_agent_identity_auth()
                && !account.is_web_session_auth()
                && local_oauth_snapshot_matches_account(&snapshot, account)
        })
        .map(|account| account.id.clone())
}

pub(crate) fn oauth_account_id_for_runtime_dir(base_dir: &Path) -> Option<String> {
    oauth_account_id_for_runtime_snapshot(base_dir, &list_accounts())
}

/// 返回当前仍由 Codex 运行态使用的 OAuth 账号。
///
/// 官方 app-server 启动后会把认证信息保存在进程内。后台刷新虽然会更新
/// auth.json/keychain，但不会把新 Token 注入已经运行的官方进程；对这些账号
/// 轮换 refresh_token 会让官方进程稍后在 cloud requirements/config 请求中
/// 收到 Auth/relogin。这里只采用运行 profile 中实际可读的 OAuth 快照，不再按
/// Cockpit 的绑定配置推断占用，避免旧绑定误拦截多开启动或 OAuth 绑定。
fn running_codex_oauth_account_ids_from_entries(
    process_entries: &[(u32, Option<String>)],
) -> Result<HashSet<String>, String> {
    let store = crate::modules::codex_instance::load_instance_store()?;
    let accounts = list_accounts();
    let mut account_ids = HashSet::new();

    // 正式版、dev 与其它 Cockpit 数据目录可能维护各自的实例列表，但系统进程
    // 是共享的。直接检查所有可识别 Codex 进程的 CODEX_HOME/auth 快照，避免
    // 另一个安装启动的实例漏出 TokenKeeper 保护范围。
    let default_home = get_codex_home();
    for (_, runtime_home) in process_entries {
        let runtime_dir = runtime_home
            .as_deref()
            .map(Path::new)
            .unwrap_or(default_home.as_path());
        if let Some(account_id) = oauth_account_id_for_runtime_snapshot(runtime_dir, &accounts) {
            account_ids.insert(account_id);
        }
    }

    let default_pid_matches = crate::modules::process::resolve_codex_pid_from_entries(
        store.default_settings.last_pid,
        None,
        &process_entries,
    )
    .is_some();
    // 官方 app-server 可能仍在运行，但 GUI PID 的记录在重启/接管过程中短暂失配。
    // 只要默认实例仍有受管 PID 且系统能确认存在 Codex 进程，就保守保护当前 OAuth，
    // 避免后台刷新先轮换 refresh_token、随后让官方客户端进入 Auth/relogin。
    let default_running = default_pid_matches
        || (store.default_settings.last_pid.is_some() && !process_entries.is_empty());
    if !default_pid_matches && default_running {
        logger::log_warn(
            "[Codex运行态] 默认实例 PID 暂时未匹配，但仍检测到 Codex 进程；本轮后台 OAuth 刷新将保护当前账号",
        );
    }
    if default_running {
        if let Some(account_id) =
            oauth_account_id_for_runtime_snapshot(&get_codex_home(), &accounts)
        {
            account_ids.insert(account_id);
        }
    }

    for instance in store.instances {
        let running = crate::modules::process::resolve_codex_pid_from_entries(
            instance.last_pid,
            Some(&instance.user_data_dir),
            &process_entries,
        )
        .is_some();
        if !running {
            continue;
        }
        if let Some(account_id) =
            oauth_account_id_for_runtime_snapshot(Path::new(&instance.user_data_dir), &accounts)
        {
            account_ids.insert(account_id);
        }
    }

    Ok(account_ids)
}

pub(crate) fn running_codex_oauth_account_ids() -> Result<HashSet<String>, String> {
    let process_entries = crate::modules::process::collect_codex_process_entries();
    running_codex_oauth_account_ids_from_entries(&process_entries)
}

pub fn is_pending_oauth_account(account: &CodexAccount) -> bool {
    !account.is_api_key_auth()
        && account
            .authorization_status
            .as_deref()
            .map(str::trim)
            .map(|value| value.eq_ignore_ascii_case(CODEX_AUTHORIZATION_STATUS_PENDING))
            .unwrap_or(false)
}

fn is_standard_oauth_account(account: &CodexAccount) -> bool {
    !account.is_api_key_auth()
        && account.agent_identity.is_none()
        && !is_pending_oauth_account(account)
        && account.token_source_mode.trim() != CODEX_TOKEN_SOURCE_WEB_SESSION
        && !account.tokens.access_token.trim().is_empty()
        && !account.tokens.access_token.trim().starts_with("at-")
        && (!account.tokens.id_token.trim().is_empty() || account_has_refresh_token(account))
}

fn clear_stale_missing_refresh_token_reauth(account: &mut CodexAccount) -> Result<(), String> {
    let is_missing_refresh_token_reauth = account
        .reauth_reason
        .as_deref()
        .map(is_missing_refresh_token_reason)
        .unwrap_or(false);

    if !account.requires_reauth || !is_missing_refresh_token_reauth {
        return Ok(());
    }
    if codex_oauth::is_token_expired(&account.tokens.access_token) {
        return Ok(());
    }

    account.requires_reauth = false;
    account.reauth_reason = None;
    save_account(account)
}

fn clear_stale_id_token_reauth(account: &mut CodexAccount) -> Result<(), String> {
    let is_legacy_id_token_reauth = account
        .reauth_reason
        .as_deref()
        .map(|reason| reason.contains("id_token"))
        .unwrap_or(false);
    if !account.requires_reauth
        || !is_legacy_id_token_reauth
        || codex_oauth::is_token_expired(&account.tokens.access_token)
    {
        return Ok(());
    }

    account.requires_reauth = false;
    account.reauth_reason = None;
    save_account(account)
}

/// 清理已撤回的 app-server 主动预检写入的误判状态。
///
/// 这里只匹配该版本写入的固定原因，不清理真实刷新链路产生的重新授权状态。
fn clear_retired_app_server_preflight_reauth(account: &mut CodexAccount) -> bool {
    if !account.requires_reauth
        || !account.reauth_reason.as_deref().is_some_and(|reason| {
            reason
                .trim()
                .starts_with(CODEX_RETIRED_APP_SERVER_PREFLIGHT_REAUTH_REASON)
        })
    {
        return false;
    }

    account.requires_reauth = false;
    account.reauth_reason = None;
    true
}

pub fn mark_access_token_only_account_requires_reauth(account_id: &str) -> Result<(), String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if account.is_api_key_auth() || account_has_refresh_token(&account) {
        return Ok(());
    }
    mark_account_requires_reauth(&mut account, CODEX_MISSING_REFRESH_TOKEN_REAUTH_REASON)
}

fn retain_existing_refresh_token_if_missing(
    mut tokens: CodexTokens,
    existing: Option<&CodexAccount>,
) -> CodexTokens {
    tokens.refresh_token = normalize_optional_value(tokens.refresh_token).or_else(|| {
        existing.and_then(|account| normalize_optional_ref(account.tokens.refresh_token.as_deref()))
    });
    tokens
}

pub fn extract_chatgpt_account_id_from_access_token(access_token: &str) -> Option<String> {
    let payload = decode_jwt_payload_value(access_token)?;
    let auth_data = payload.get("https://api.openai.com/auth")?;
    first_json_string(auth_data, &[&["chatgpt_account_id"], &["account_id"]])
}

pub fn extract_chatgpt_organization_id_from_access_token(access_token: &str) -> Option<String> {
    let payload = decode_jwt_payload_value(access_token)?;
    let auth_data = payload.get("https://api.openai.com/auth")?;
    const ORG_KEYS: [&str; 6] = [
        "organization_id",
        "chatgpt_organization_id",
        "chatgpt_org_id",
        "org_id",
        "poid",
        "POID",
    ];
    for key in ORG_KEYS {
        if let Some(value) = normalize_optional_ref(auth_data.get(key).and_then(|v| v.as_str())) {
            return Some(value);
        }
    }
    if let Some(orgs) = auth_data
        .get("organizations")
        .and_then(|value| value.as_array())
    {
        if let Some(default_org) = orgs.iter().find(|org| {
            org.get("is_default")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
        }) {
            if let Some(value) = first_json_string(default_org, &[&["id"]]) {
                return Some(value);
            }
        }
        if let Some(first_org) = orgs.first() {
            if let Some(value) = first_json_string(first_org, &[&["id"]]) {
                return Some(value);
            }
        }
    }
    None
}

fn extract_access_token_identity(
    access_token: &str,
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    let Some(payload) = decode_jwt_payload_value(access_token) else {
        return (None, None, None, None, None, None);
    };

    let auth_data = payload.get("https://api.openai.com/auth");
    let email = first_json_string(&payload, &[&["email"]])
        .or_else(|| first_json_string(&payload, &[&["https://api.openai.com/profile", "email"]]));
    let user_id = auth_data
        .and_then(|value| first_json_string(value, &[&["chatgpt_user_id"], &["user_id"]]))
        .or_else(|| first_json_string(&payload, &[&["sub"]]));
    let plan_type = auth_data.and_then(|value| first_json_string(value, &[&["chatgpt_plan_type"]]));
    let subscription_active_until = auth_data.and_then(|value| {
        value
            .get("chatgpt_subscription_active_until")
            .and_then(|item| normalize_optional_json_scalar(Some(item)))
    });
    let account_id = extract_chatgpt_account_id_from_access_token(access_token);
    let organization_id = extract_chatgpt_organization_id_from_access_token(access_token);

    (
        email,
        user_id,
        plan_type,
        subscription_active_until,
        account_id,
        organization_id,
    )
}

fn access_token_fingerprint(access_token: &str) -> String {
    let digest = format!("{:x}", md5::compute(access_token.as_bytes()));
    digest.chars().take(12).collect()
}

fn build_account_storage_id(
    email: &str,
    account_id: Option<&str>,
    organization_id: Option<&str>,
) -> String {
    let mut seed = email.trim().to_string();
    if let Some(id) = normalize_optional_ref(account_id) {
        seed.push('|');
        seed.push_str(&id);
    }
    if let Some(org) = normalize_optional_ref(organization_id) {
        seed.push('|');
        seed.push_str(&org);
    }
    format!("codex_{:x}", md5::compute(seed.as_bytes()))
}

fn find_existing_account_id(
    index: &CodexAccountIndex,
    email: &str,
    account_id: Option<&str>,
    organization_id: Option<&str>,
) -> Option<String> {
    let expected_account_id = normalize_optional_ref(account_id);
    let expected_org_id = normalize_optional_ref(organization_id);
    let mut first_email_match: Option<String> = None;
    let mut email_match_count = 0usize;
    let mut account_id_match_without_org: Option<String> = None;
    let mut legacy_email_only_candidate: Option<String> = None;
    let mut legacy_email_only_count = 0usize;

    for summary in &index.accounts {
        if !summary.email.eq_ignore_ascii_case(email) {
            continue;
        }
        email_match_count += 1;
        if first_email_match.is_none() {
            first_email_match = Some(summary.id.clone());
        }

        let Some(account) = load_account(&summary.id) else {
            continue;
        };

        let current_account_id = normalize_optional_ref(account.account_id.as_deref());
        let current_org_id = normalize_optional_ref(account.organization_id.as_deref());

        let is_exact_match =
            current_account_id == expected_account_id && current_org_id == expected_org_id;
        if is_exact_match {
            return Some(summary.id.clone());
        }

        if expected_account_id.is_some()
            && current_account_id == expected_account_id
            && current_org_id.is_none()
            && account_id_match_without_org.is_none()
        {
            account_id_match_without_org = Some(summary.id.clone());
        }

        if (expected_account_id.is_some() || expected_org_id.is_some())
            && current_account_id.is_none()
            && current_org_id.is_none()
        {
            legacy_email_only_count += 1;
            if legacy_email_only_candidate.is_none() {
                legacy_email_only_candidate = Some(summary.id.clone());
            }
        }
    }

    if expected_account_id.is_some() || expected_org_id.is_some() {
        return account_id_match_without_org.or_else(|| {
            if legacy_email_only_count == 1 {
                legacy_email_only_candidate
            } else {
                None
            }
        });
    }

    if email_match_count == 1 {
        return first_email_match;
    }

    None
}
