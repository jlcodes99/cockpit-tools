// Codex Local Access：上游 `x-codex-turn-state` 旁路观测与账号风控检测。
// 只保存 state 长度与分级，不保存 state 原文；正常观测会自动摘除疑似风控标记。

const CODEX_TURN_STATE_FILE: &str = "codex_turn_state.json";
const CODEX_TURN_STATE_HEADER_NAME: &str = "x-codex-turn-state";
const CODEX_TURN_STATE_MAX_OBSERVATIONS: usize = 8;
const CODEX_TURN_STATE_MAX_ACCOUNTS: usize = 500;
const CODEX_TURN_STATE_STALE_MS: i64 = 90 * 24 * 60 * 60 * 1000;

const CODEX_TURN_STATE_CLASS_NORMAL: &str = "normal";
/// 产品口径：state 长度为 312 即疑似风控，单次即可判定。
const CODEX_TURN_STATE_CLASS_SUSPECTED: &str = "suspected";
const CODEX_TURN_STATE_CLASS_ABNORMAL: &str = "abnormal";
const CODEX_TURN_STATE_CLASS_MISSING: &str = "missing";

const CODEX_TURN_STATE_STATUS_UNKNOWN: &str = "unknown";
const CODEX_TURN_STATE_STATUS_NORMAL: &str = "normal";
const CODEX_TURN_STATE_STATUS_ABNORMAL: &str = "abnormal";
const CODEX_TURN_STATE_STATUS_SUSPECTED: &str = "suspected";

const CODEX_TURN_STATE_REASON_ABNORMAL: &str = "turnStateAbnormal";
const CODEX_TURN_STATE_REASON_MISSING: &str = "turnStateMissing";
const CODEX_TURN_STATE_REASON_SUSPECTED: &str = "turnStateSuspected";

/// 正常长度：292 / 332 / 780。
const CODEX_TURN_STATE_NORMAL_LENGTHS: [i64; 3] = [292, 332, 780];
/// 312 视为疑似风控信号。
const CODEX_TURN_STATE_SUSPECTED_LENGTH: i64 = 312;
const CODEX_TURN_STATE_PREFIX: &str = "gAAAAA";
/// 历史数据里的 312 曾记为 `renew`，读取时按疑似风控处理。
const CODEX_TURN_STATE_CLASS_RENEW_LEGACY: &str = "renew";

fn normalize_turn_state_class(value: Option<&str>) -> Option<&'static str> {
    match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some(CODEX_TURN_STATE_CLASS_NORMAL) => Some(CODEX_TURN_STATE_CLASS_NORMAL),
        Some(CODEX_TURN_STATE_CLASS_SUSPECTED) => Some(CODEX_TURN_STATE_CLASS_SUSPECTED),
        Some(CODEX_TURN_STATE_CLASS_RENEW_LEGACY) => Some(CODEX_TURN_STATE_CLASS_SUSPECTED),
        Some(CODEX_TURN_STATE_CLASS_ABNORMAL) => Some(CODEX_TURN_STATE_CLASS_ABNORMAL),
        Some(CODEX_TURN_STATE_CLASS_MISSING) => Some(CODEX_TURN_STATE_CLASS_MISSING),
        _ => None,
    }
}

/// 仅按长度分级；缺少长度时视为 missing。
fn classify_turn_state_length(length: Option<i64>) -> &'static str {
    match length {
        Some(value) if CODEX_TURN_STATE_NORMAL_LENGTHS.contains(&value) => {
            CODEX_TURN_STATE_CLASS_NORMAL
        }
        Some(value) if value == CODEX_TURN_STATE_SUSPECTED_LENGTH => {
            CODEX_TURN_STATE_CLASS_SUSPECTED
        }
        Some(_) => CODEX_TURN_STATE_CLASS_ABNORMAL,
        None => CODEX_TURN_STATE_CLASS_MISSING,
    }
}

/// state 是 ASCII 载荷：先 trim，再按长度与 `gAAAAA` 前缀判定；不保存原文。
fn observe_turn_state_header_value(value: Option<&str>) -> (Option<i64>, &'static str) {
    let Some(raw) = value else {
        return (None, CODEX_TURN_STATE_CLASS_MISSING);
    };
    let trimmed = raw.trim();
    let length = trimmed.len() as i64;
    let class = if !trimmed.starts_with(CODEX_TURN_STATE_PREFIX) {
        CODEX_TURN_STATE_CLASS_ABNORMAL
    } else {
        classify_turn_state_length(Some(length))
    };
    (Some(length), class)
}

/// 观测来源：手动检测 / 唤醒链路 / 网关旁路。
fn turn_state_observation_source_manual() -> String {
    "manual".to_string()
}

fn turn_state_observation_source_wakeup() -> String {
    "wakeup".to_string()
}

fn turn_state_observation_source_gateway() -> String {
    "gateway".to_string()
}

/// 判定依据：只看 state 长度分级，不做其它推演。
fn turn_state_observation_reason(class: &str) -> Option<String> {
    match class {
        CODEX_TURN_STATE_CLASS_SUSPECTED => Some(CODEX_TURN_STATE_REASON_SUSPECTED.to_string()),
        CODEX_TURN_STATE_CLASS_MISSING => Some(CODEX_TURN_STATE_REASON_MISSING.to_string()),
        CODEX_TURN_STATE_CLASS_ABNORMAL => Some(CODEX_TURN_STATE_REASON_ABNORMAL.to_string()),
        _ => None,
    }
}

fn turn_state_class_is_abnormal(class: &str) -> bool {
    matches!(
        class,
        CODEX_TURN_STATE_CLASS_ABNORMAL | CODEX_TURN_STATE_CLASS_MISSING
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexTurnStateObservation {
    pub observed_at: i64,
    /// manual / wakeup / gateway
    pub source: String,
    /// normal / renew / abnormal / missing
    pub class: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    /// 判定依据代码，由前端本地化；不写死展示文案。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexAccountTurnStateStatus {
    pub account_id: String,
    /// unknown / normal / abnormal / suspected
    pub status: String,
    pub suspected: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_length: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_http_status: Option<u16>,
    #[serde(default)]
    pub last_observed_at: i64,
    #[serde(default)]
    pub observations: Vec<CodexTurnStateObservation>,
}

impl CodexAccountTurnStateStatus {
    fn unknown(account_id: &str) -> Self {
        Self {
            account_id: account_id.to_string(),
            status: CODEX_TURN_STATE_STATUS_UNKNOWN.to_string(),
            suspected: false,
            reason: None,
            last_class: None,
            last_length: None,
            last_http_status: None,
            last_observed_at: 0,
            observations: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CodexTurnStateStoreFile {
    #[serde(default)]
    accounts: HashMap<String, Vec<CodexTurnStateObservation>>,
}

/// 观测是高频数据（每个 API 服务请求都会有一条），因此内存里维护唯一副本，
/// 落盘按最小间隔节流；手动检测与读取路径强制使用最新内存状态。
struct CodexTurnStateStoreCache {
    loaded: bool,
    store: CodexTurnStateStoreFile,
    last_flush_ms: i64,
}

static CODEX_TURN_STATE_CACHE: std::sync::LazyLock<Mutex<CodexTurnStateStoreCache>> =
    std::sync::LazyLock::new(|| {
        Mutex::new(CodexTurnStateStoreCache {
            loaded: false,
            store: CodexTurnStateStoreFile::default(),
            last_flush_ms: 0,
        })
    });

/// 两次落盘之间的最小间隔：观测只是诊断数据，允许在异常退出时丢几秒。
const CODEX_TURN_STATE_FLUSH_INTERVAL_MS: i64 = 5_000;

fn lock_codex_turn_state_cache() -> std::sync::MutexGuard<'static, CodexTurnStateStoreCache> {
    CODEX_TURN_STATE_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn codex_turn_state_file_path() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join(CODEX_TURN_STATE_FILE))
}

fn load_codex_turn_state_store() -> CodexTurnStateStoreFile {
    let Ok(path) = codex_turn_state_file_path() else {
        return CodexTurnStateStoreFile::default();
    };
    if !path.exists() {
        return CodexTurnStateStoreFile::default();
    }
    let Ok(content) = fs::read_to_string(&path) else {
        return CodexTurnStateStoreFile::default();
    };
    if content.trim().is_empty() {
        return CodexTurnStateStoreFile::default();
    }
    match serde_json::from_str::<CodexTurnStateStoreFile>(&content) {
        Ok(mut store) => {
            // 历史记录里的 `renew`（312 的旧分级）统一归一到 `suspected`。
            for observations in store.accounts.values_mut() {
                for observation in observations.iter_mut() {
                    observation.class =
                        normalize_turn_state_class(Some(observation.class.as_str()))
                            .unwrap_or(CODEX_TURN_STATE_CLASS_ABNORMAL)
                            .to_string();
                }
            }
            store
        }
        Err(error) => {
            logger::log_codex_api_warn(&format!(
                "[CodexTurnState] 风控观测记录解析失败，本次按空记录处理: {}",
                error
            ));
            CodexTurnStateStoreFile::default()
        }
    }
}

fn save_codex_turn_state_store(store: &CodexTurnStateStoreFile) -> Result<(), String> {
    let path = codex_turn_state_file_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建风控观测目录失败: {}", e))?;
    }
    let content = serde_json::to_string_pretty(store)
        .map_err(|e| format!("序列化风控观测记录失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
}

fn prune_codex_turn_state_store(store: &mut CodexTurnStateStoreFile, now: i64) {
    store.accounts.retain(|_, observations| {
        observations
            .last()
            .map(|item| now.saturating_sub(item.observed_at) <= CODEX_TURN_STATE_STALE_MS)
            .unwrap_or(false)
    });
    if store.accounts.len() <= CODEX_TURN_STATE_MAX_ACCOUNTS {
        return;
    }
    let mut entries = store
        .accounts
        .iter()
        .map(|(account_id, observations)| {
            (
                account_id.clone(),
                observations
                    .last()
                    .map(|item| item.observed_at)
                    .unwrap_or_default(),
            )
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| right.1.cmp(&left.1));
    let keep = entries
        .into_iter()
        .take(CODEX_TURN_STATE_MAX_ACCOUNTS)
        .map(|(account_id, _)| account_id)
        .collect::<HashSet<_>>();
    store.accounts.retain(|account_id, _| keep.contains(account_id));
}

/// 按最新观测推导账号风控状态：连续 2 次异常或上游风控直证 → 疑似风控。
fn derive_codex_account_turn_state_status(
    account_id: &str,
    observations: &[CodexTurnStateObservation],
) -> CodexAccountTurnStateStatus {
    let Some(latest) = observations.last() else {
        return CodexAccountTurnStateStatus::unknown(account_id);
    };
    let mut status = CodexAccountTurnStateStatus {
        account_id: account_id.to_string(),
        status: CODEX_TURN_STATE_STATUS_NORMAL.to_string(),
        suspected: false,
        reason: None,
        last_class: Some(latest.class.clone()),
        last_length: latest.length,
        last_http_status: latest.http_status,
        last_observed_at: latest.observed_at,
        observations: observations.to_vec(),
    };

    // 只看最近一次观测：312 → 疑似风控；292/332/780 → 正常；其它长度/缺失 → 异常（不升级为疑似风控）。
    let latest_class = normalize_turn_state_class(Some(latest.class.as_str()))
        .unwrap_or(CODEX_TURN_STATE_CLASS_ABNORMAL);
    if latest_class != CODEX_TURN_STATE_CLASS_NORMAL {
        if latest_class == CODEX_TURN_STATE_CLASS_SUSPECTED {
            status.status = CODEX_TURN_STATE_STATUS_SUSPECTED.to_string();
            status.suspected = true;
            status.reason = Some(CODEX_TURN_STATE_REASON_SUSPECTED.to_string());
            return status;
        }
        status.status = CODEX_TURN_STATE_STATUS_ABNORMAL.to_string();
        status.reason = Some(
            latest
                .reason
                .clone()
                .unwrap_or_else(|| CODEX_TURN_STATE_REASON_ABNORMAL.to_string()),
        );
        return status;
    }
    // 正常观测自动摘除疑似标记。
    status.reason = None;
    status
}

/// 记录一次 state 观测并返回更新后的账号风控状态。
fn record_codex_turn_state_observation(
    account_id: &str,
    observation: CodexTurnStateObservation,
) -> Result<CodexAccountTurnStateStatus, String> {
    record_codex_turn_state_observation_with_flush(account_id, observation, false)
}

/// `flush_now` 为用户主动触发（手动检测）时使用，保证结果立即落盘。
fn record_codex_turn_state_observation_with_flush(
    account_id: &str,
    observation: CodexTurnStateObservation,
    flush_now: bool,
) -> Result<CodexAccountTurnStateStatus, String> {
    let account_id = account_id.trim();
    if account_id.is_empty() {
        return Err("账号 ID 为空，无法记录风控观测".to_string());
    }
    let (status, pending_snapshot) = {
        let mut cache = lock_codex_turn_state_cache();
        if !cache.loaded {
            cache.store = load_codex_turn_state_store();
            cache.loaded = true;
        }
        {
            let entry = cache
                .store
                .accounts
                .entry(account_id.to_string())
                .or_default();
            entry.push(observation);
            if entry.len() > CODEX_TURN_STATE_MAX_OBSERVATIONS {
                let remove_count = entry.len() - CODEX_TURN_STATE_MAX_OBSERVATIONS;
                entry.drain(..remove_count);
            }
        }
        let now = now_ms();
        prune_codex_turn_state_store(&mut cache.store, now);
        let status = cache
            .store
            .accounts
            .get(account_id)
            .map(|observations| derive_codex_account_turn_state_status(account_id, observations))
            .unwrap_or_else(|| CodexAccountTurnStateStatus::unknown(account_id));
        let should_flush = flush_now
            || now.saturating_sub(cache.last_flush_ms) >= CODEX_TURN_STATE_FLUSH_INTERVAL_MS;
        let snapshot = if should_flush {
            cache.last_flush_ms = now;
            Some(cache.store.clone())
        } else {
            None
        };
        (status, snapshot)
    };
    if let Some(snapshot) = pending_snapshot {
        save_codex_turn_state_store(&snapshot)?;
    }
    Ok(status)
}

/// 网关/唤醒等异步链路使用的写入入口：文件写入放到阻塞线程池，避免占用 async 线程。
async fn record_codex_turn_state_observation_async(
    account_id: String,
    observation: CodexTurnStateObservation,
) {
    let result = tauri::async_runtime::spawn_blocking(move || {
        record_codex_turn_state_observation(account_id.as_str(), observation)
    })
    .await;
    match result {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => logger::log_codex_api_warn(&format!(
            "[CodexTurnState] 写入账号风控观测失败: {}",
            error
        )),
        Err(error) => logger::log_codex_api_warn(&format!(
            "[CodexTurnState] 账号风控观测后台任务失败: {}",
            error
        )),
    }
}

/// 读取全部账号的风控状态（无观测记录的账号返回 unknown）。
pub fn codex_account_turn_state_statuses() -> Vec<CodexAccountTurnStateStatus> {
    let mut cache = lock_codex_turn_state_cache();
    if !cache.loaded {
        cache.store = load_codex_turn_state_store();
        cache.loaded = true;
    }
    let mut statuses = cache
        .store
        .accounts
        .iter()
        .map(|(account_id, observations)| {
            derive_codex_account_turn_state_status(account_id, observations)
        })
        .collect::<Vec<_>>();
    statuses.sort_by(|left, right| right.last_observed_at.cmp(&left.last_observed_at));
    statuses
}

pub fn codex_account_turn_state_status(account_id: &str) -> CodexAccountTurnStateStatus {
    let account_id = account_id.trim();
    if account_id.is_empty() {
        return CodexAccountTurnStateStatus::unknown("");
    }
    let mut cache = lock_codex_turn_state_cache();
    if !cache.loaded {
        cache.store = load_codex_turn_state_store();
        cache.loaded = true;
    }
    match cache.store.accounts.get(account_id) {
        Some(observations) => {
            derive_codex_account_turn_state_status(account_id, observations)
        }
        None => CodexAccountTurnStateStatus::unknown(account_id),
    }
}

pub fn remove_codex_turn_state_records(account_ids: &[String]) -> Result<(), String> {
    let remove_ids = account_ids
        .iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect::<HashSet<_>>();
    if remove_ids.is_empty() {
        return Ok(());
    }
    let pending_snapshot = {
        let mut cache = lock_codex_turn_state_cache();
        if !cache.loaded {
            cache.store = load_codex_turn_state_store();
            cache.loaded = true;
        }
        let before = cache.store.accounts.len();
        cache
            .store
            .accounts
            .retain(|account_id, _| !remove_ids.contains(account_id));
        if cache.store.accounts.len() == before {
            None
        } else {
            cache.last_flush_ms = now_ms();
            Some(cache.store.clone())
        }
    };
    match pending_snapshot {
        Some(snapshot) => save_codex_turn_state_store(&snapshot),
        None => Ok(()),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexTurnStateProbeResult {
    pub account_id: String,
    pub status: CodexAccountTurnStateStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_length: Option<i64>,
    pub state_class: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub latency_ms: u64,
}

/// 手动风控检测：只发一条最小官方请求，读取响应头里的 state 后立即断开，不解析正文。
pub async fn probe_codex_account_turn_state(
    account_id: &str,
) -> Result<CodexTurnStateProbeResult, String> {
    let account = get_prepared_account(account_id).await?;
    if account.is_api_key_auth() {
        return Err("风控检测仅支持 OAuth / Agent Identity 账号。".to_string());
    }

    let model = crate::modules::codex_wakeup::DEFAULT_WAKEUP_MODEL;
    let request_body = json!({
        "model": model,
        "store": false,
        "stream": true,
        "instructions": "Reply with exactly: pong",
        "input": [
            {
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": "ping",
                    }
                ],
            }
        ],
    });
    let body = serde_json::to_vec(&request_body)
        .map_err(|e| format!("序列化风控检测请求失败: {}", e))?;
    let mut headers = HashMap::new();
    headers.insert("accept".to_string(), "text/event-stream".to_string());
    headers.insert("content-type".to_string(), "application/json".to_string());
    for header in CODEX_OFFICIAL_EMPTY_HEADERS {
        headers
            .entry((*header).to_string())
            .or_insert_with(String::new);
    }
    if account
        .agent_identity
        .as_ref()
        .is_some_and(|identity| identity.chatgpt_account_is_fedramp)
    {
        headers.insert("x-openai-fedramp".to_string(), "true".to_string());
    }

    let (upstream_proxy_url, timeouts) = official_wakeup_network_config().await;
    let upstream_connect_timeout = duration_from_millis(
        timeouts.legacy_upstream_connect_timeout_ms,
        DEFAULT_UPSTREAM_CONNECT_TIMEOUT,
    );
    let upstream_target = resolve_upstream_target(RESPONSES_PATH)?;
    let started_at = Instant::now();
    let (account, status, state_value, body_text) = if account.is_agent_identity_auth() {
        let response = send_agent_identity_wakeup_request_with_base_urls(
            &account,
            &upstream_target,
            &headers,
            &body,
            upstream_proxy_url.as_deref(),
            upstream_connect_timeout,
            &timeouts,
            UPSTREAM_CODEX_BASE_URL,
            codex_agent_identity::AGENT_IDENTITY_AUTH_API_BASE_URL,
        )
        .await?;
        let status = response.status;
        let body_text = if status.is_success() {
            String::new()
        } else {
            response.body
        };
        (response.account, status, response.turn_state_value, body_text)
    } else {
        let response = send_upstream_request(
            "POST",
            &upstream_target,
            &headers,
            &body,
            &account,
            upstream_proxy_url.as_deref(),
            upstream_connect_timeout,
            &timeouts,
            CodexLocalAccessImageGenerationMode::Disabled,
            CodexLocalAccessRequestKind::Text,
        )
        .await
        .map_err(|err| {
            format!(
                "Codex 上游连接失败，无法完成风控检测。技术细节: {}",
                truncate_diagnostic_text(err.trim(), 400)
            )
        })?;
        let status = response.status();
        let state_value = response
            .headers()
            .get(CODEX_TURN_STATE_HEADER_NAME)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        // 只读响应头：拿到 state 立即断开，不等待流式正文。
        let body_text = if status.is_success() {
            String::new()
        } else {
            response
                .text()
                .await
                .map_err(|e| format!("读取风控检测响应失败: {}", e))?
        };
        (account, status, state_value, body_text)
    };

    let (state_length, state_class) = observe_turn_state_header_value(state_value.as_deref());
    let error_message = if body_text.trim().is_empty() {
        None
    } else {
        Some(truncate_diagnostic_text(
            extract_upstream_error_message(&body_text)
                .unwrap_or_else(|| body_text.trim().to_string())
                .as_str(),
            600,
        ))
    };
    let observation = CodexTurnStateObservation {
        observed_at: now_ms(),
        source: turn_state_observation_source_manual(),
        class: state_class.to_string(),
        length: state_length,
        http_status: Some(status.as_u16()),
        reason: turn_state_observation_reason(state_class),
    };
    let status_snapshot = record_codex_turn_state_observation_with_flush(
        account.id.as_str(),
        observation,
        true,
    )?;
    if account.is_agent_identity_auth() {
        cache_prepared_account(&account).await;
    }
    Ok(CodexTurnStateProbeResult {
        account_id: account.id.clone(),
        status: status_snapshot,
        http_status: Some(status.as_u16()),
        state_length,
        state_class: state_class.to_string(),
        error_message,
        latency_ms: started_at.elapsed().as_millis() as u64,
    })
}
