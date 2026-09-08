// Codex Local Access：Gateway state types, request parsing and shared protocol helpers。
// 通过 include! 保持原 modules::codex_local_access 作用域和私有调用关系。
use crate::models::codex::{
    CodexAccount, CodexApiProviderMode, CodexAppSpeed, CodexAuthMode, CodexQuota,
};
use crate::models::codex_local_access::{
    CodexLocalAccessAccountCooldown, CodexLocalAccessAccountHealth,
    CodexLocalAccessAccountModelRule, CodexLocalAccessAccountPoolHealth,
    CodexLocalAccessAccountPoolMemberHealth, CodexLocalAccessAccountStats,
    CodexLocalAccessAccountWindowQuery, CodexLocalAccessAccountWindowStats, CodexLocalAccessApiKey,
    CodexLocalAccessApiKeyStats, CodexLocalAccessAppendAccountSkipped,
    CodexLocalAccessAppendAccountsResult, CodexLocalAccessChatMessage, CodexLocalAccessChatResult,
    CodexLocalAccessClientBaseUrlHost, CodexLocalAccessCollection,
    CodexLocalAccessCustomRoutingRule, CodexLocalAccessGatewayMode,
    CodexLocalAccessImageGenerationMode, CodexLocalAccessImageGenerationPolicy,
    CodexLocalAccessImageGenerationStatus, CodexLocalAccessModelAlias,
    CodexLocalAccessModelPricing, CodexLocalAccessModelRoute, CodexLocalAccessModelRouting,
    CodexLocalAccessModelStats, CodexLocalAccessPortCleanupResult,
    CodexLocalAccessProfileAttachment, CodexLocalAccessProviderGateway,
    CodexLocalAccessProviderGatewayModelCapability, CodexLocalAccessQuotaReserve,
    CodexLocalAccessQuotaReserveStatus, CodexLocalAccessRequestKind,
    CodexLocalAccessRoutingStrategy, CodexLocalAccessScope, CodexLocalAccessState,
    CodexLocalAccessStats, CodexLocalAccessStatsWindow, CodexLocalAccessTestFailure,
    CodexLocalAccessTestResult, CodexLocalAccessTimeoutPreset, CodexLocalAccessTimeouts,
    CodexLocalAccessUsageEvent, CodexLocalAccessUsageEventPage, CodexLocalAccessUsageStats,
    CodexLocalAccessUsageTrendPoint,
    CodexTokenBreakdown,
};
use crate::models::{CodexInstanceApiRoute, CodexInstanceModelRouting};
use crate::modules::atomic_write::{write_string_atomic, write_string_atomic_if_hash_matches};
use crate::modules::{
    account, codex_account, codex_agent_identity, codex_oauth, codex_protocol, codex_quota,
    codex_wakeup, config, logger, process,
};
use base64::{engine::general_purpose, Engine as _};
use chrono::{Datelike, Duration as ChronoDuration, Local, LocalResult, NaiveDate, TimeZone, Timelike};
use futures_util::{stream, SinkExt, StreamExt};
use rand::{distributions::Alphanumeric, seq::SliceRandom, Rng};
use reqwest::header::{HeaderName, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use reqwest::{Client, Method, Proxy, StatusCode, Url};
use rusqlite::{
    params, params_from_iter, types::Value as SqlValue, Connection, Error as SqliteError,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::error::Error as StdError;
use std::fs;
use std::net::{Ipv4Addr, TcpListener as StdTcpListener};
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command as TokioCommand};
use tokio::sync::{oneshot, watch, Mutex as TokioMutex, Notify};
use tokio::time::{timeout, Duration};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::client::Request as WsClientRequest;
use tokio_tungstenite::tungstenite::http::header::{
    HeaderName as WsHeaderName, HeaderValue as WsHeaderValue,
};
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{client_async_tls_with_config, MaybeTlsStream, WebSocketStream};
use toml_edit::{value, Document};

const CODEX_LOCAL_ACCESS_FILE: &str = "codex_local_access.json";
const CODEX_LOCAL_ACCESS_CHAT_TEST_STREAM_EVENT: &str = "codex-local-access-chat-test-stream";
const CODEX_LOCAL_ACCESS_MODEL_PRICING_REPRICE_EVENT: &str =
    "codex-local-access-model-pricing-reprice";
const CODEX_LOCAL_ACCESS_STATS_FILE: &str = "codex_local_access_stats.json";
const CODEX_LOCAL_ACCESS_LOGS_DB_FILE: &str = "codex_local_access_logs.sqlite";
const CODEX_LOCAL_ACCESS_TAKEOVER_BACKUPS_FILE: &str = "codex_local_access_takeover_backups.json";
const CODEX_LOCAL_ACCESS_SIDECAR_DIR: &str = "codex_local_access_sidecar";
const CODEX_PROVIDER_GATEWAY_SIDECAR_DIR: &str = "codex_provider_gateway_sidecars";
// Official client shells used for provider model display. Prefer list-friendly models first.
// This is not a hard client limit; it is our reusable full-template pool for mapping.
const CODEX_PROVIDER_MODEL_SHELL_POOL: &[&str] = &[
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-5.6-luna",
    "gpt-5.5",
    "gpt-5.4",
    "gpt-5.4-mini",
    "gpt-5.3-codex",
    "gpt-5.3-codex-spark",
    "gpt-5.2",
];
// Keep Astra available as an identity-preserving shell when an upstream account
// already exposes that exact model, without assigning it to unrelated overflow models.
const CODEX_PROVIDER_IDENTITY_ONLY_MODEL_IDS: &[&str] = &["gpt-6-astra"];
const CODEX_PROVIDER_GATEWAY_STATE_FILE: &str = "state.json";
const CODEX_LOCAL_ACCESS_SIDECAR_CONFIG_FILE: &str = "config.json";
const CODEX_LOCAL_ACCESS_SIDECAR_MANIFEST_FILE: &str = "manifest.json";
const SIDECAR_MESSAGE_LOCALES: &[&str] = &[
    "ar", "cs", "de", "en-us", "en", "es", "fr", "id", "it", "ja", "ko", "pl", "pt-br", "ru", "tr",
    "vi", "zh-cn", "zh-tw",
];
const CODEX_LOCAL_ACCESS_SIDECAR_API_KEY_PRIORITY_FILE: &str = "api-key-priorities.json";
const CODEX_LOCAL_ACCESS_SIDECAR_QUOTA_RESERVE_FILE: &str = "quota-reserve.json";
const CODEX_LOCAL_ACCESS_SIDECAR_QUOTA_POOL_FILE: &str = "quota-pool-state.json";
const CODEX_LOCAL_ACCESS_SIDECAR_AUTHS_DIR: &str = "auths";
const CODEX_LOCAL_ACCESS_SIDECAR_BIN_NAME: &str = "cockpit-cliproxy";
const SIDECAR_SERVICE_TIER_SUPPORTED_MODEL_PATTERN: &str = "*";
const SIDECAR_SERVICE_TIER_SUPPORTED_PAYLOAD_FORMATS: &[&str] =
    &["codex", "openai", "openai-response"];
const CODEX_LOCAL_ACCESS_LOCALHOST_BIND_HOST: &str = "127.0.0.1";
const CODEX_LOCAL_ACCESS_LAN_BIND_HOST: &str = "0.0.0.0";
const CODEX_LOCAL_ACCESS_DEFAULT_CLIENT_URL_HOST: &str = "localhost";
const CODEX_LOCAL_ACCESS_API_PORT_ENV: &str = "COCKPIT_TOOLS_API_PORT";
const CODEX_LOCAL_ACCESS_DEV_DEFAULT_PORT: u16 = 1456;
const CODEX_LOCAL_ACCESS_TAKEOVER_BACKUP_VERSION: u32 = 1;
const CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID: &str = "codex_local_access";
const CODEX_LOCAL_ACCESS_RUNTIME_ACCOUNT_ID: &str = "codex_local_access_runtime";
const CODEX_IMAGEGEN_ACTOR_HEADER: &str = "x-openai-actor-authorization";
const CODEX_LOCAL_ACCESS_DISABLE_HOSTED_IMAGE_GENERATION_HEADER: &str =
    "x-agtools-disable-image-generation";
const CODEX_LOCAL_ACCESS_DISABLE_HOSTED_IMAGE_GENERATION_HEADER_VALUE: &str = "chat";
const CODEX_PROFILE_AUTH_FILE: &str = "auth.json";
const CODEX_PROFILE_CONFIG_FILE: &str = "config.toml";
const CODEX_LOCAL_ACCESS_AUTH_PROJECTION_FILE: &str = ".cockpit_codex_auth.json";
const CODEX_MANAGED_MODEL_CATALOG_FILE: &str = "cockpit-model-catalog.json";
const CODEX_LOCAL_ACCESS_MODEL_CATALOG_FILE: &str = CODEX_MANAGED_MODEL_CATALOG_FILE;
const CODEX_PROVIDER_MODEL_CATALOG_FILE: &str = CODEX_MANAGED_MODEL_CATALOG_FILE;
const CODEX_LEGACY_LOCAL_ACCESS_MODEL_CATALOG_FILE: &str =
    "cockpit-local-access-model-catalog.json";
const CODEX_LEGACY_PROVIDER_MODEL_CATALOG_FILE: &str = "cockpit-provider-model-catalog.json";
const CODEX_MODEL_CACHE_FILE: &str = "models_cache.json";
const CODEX_PROVIDER_MODEL_BACKUP_FILE: &str = ".cockpit-provider-model-backup.json";
const MAX_HTTP_REQUEST_BYTES: usize = 256 * 1024 * 1024;
const DEFAULT_REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_REQUEST_RETRY_ATTEMPTS: usize = 1;
const DEFAULT_UPSTREAM_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

fn is_cockpit_managed_model_catalog_name(value: &str) -> bool {
    matches!(
        value.trim(),
        CODEX_MANAGED_MODEL_CATALOG_FILE
            | CODEX_LEGACY_LOCAL_ACCESS_MODEL_CATALOG_FILE
            | CODEX_LEGACY_PROVIDER_MODEL_CATALOG_FILE
    )
}
const DEFAULT_UPSTREAM_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_UPSTREAM_STREAM_TOTAL_TIMEOUT: Duration = Duration::from_secs(180);
const STATS_FLUSH_INTERVAL: Duration = Duration::from_secs(1);
const MAX_RETRY_CREDENTIALS_PER_REQUEST: usize = 8;
const SESSION_AFFINITY_TTL_MIN_MS: i64 = 60 * 1000;
const SESSION_AFFINITY_TTL_MAX_MS: i64 = 24 * 60 * 60 * 1000;
const DEFAULT_SESSION_AFFINITY_TTL_MS: i64 = 60 * 60 * 1000;
const MAX_RETRY_INTERVAL_MIN_MS: u64 = 0;
const MAX_RETRY_INTERVAL_MAX_MS: u64 = 30 * 1000;
const DEFAULT_MAX_RETRY_INTERVAL_MS: u64 = 3 * 1000;
const MAX_CONCURRENT_IMAGE_REQUESTS_PER_ACCOUNT: u16 = 16;
const LOCAL_ACCESS_TIMEOUT_MIN_MS: u64 = 1_000;
const LOCAL_ACCESS_TIMEOUT_MAX_MS: u64 = 600_000;
const LEGACY_STREAM_TOTAL_TIMEOUT_MAX_MS: u64 = 30 * 60 * 1000;
const SIDECAR_STREAM_OPEN_ATTEMPTS_MIN: u8 = 1;
const SIDECAR_STREAM_OPEN_ATTEMPTS_MAX: u8 = 3;
const SIDECAR_STREAM_KEEPALIVE_MIN_SECONDS: u16 = 0;
const SIDECAR_STREAM_KEEPALIVE_MAX_SECONDS: u16 = 300;
const LOCAL_ACCESS_RETRY_ATTEMPTS_MIN: u8 = 0;
const LOCAL_ACCESS_RETRY_ATTEMPTS_MAX: u8 = 5;
const LOCAL_ACCESS_RETRY_DELAY_MIN_MS: u64 = 50;
const LOCAL_ACCESS_RETRY_DELAY_MAX_MS: u64 = 10 * 1000;
const WEBSOCKET_IDLE_TIMEOUT_MAX_MS: u64 = 30 * 60 * 1000;
const BUILTIN_TIMEOUT_PRESET_LONG_WAIT_ID: &str = "long_wait";
const BUILTIN_TIMEOUT_PRESET_SHORT_WAIT_ID: &str = "short_wait";
const MAX_CUSTOM_TIMEOUT_PRESETS: usize = 20;
const TIMEOUT_PRESET_NAME_MAX_CHARS: usize = 40;
const RESPONSE_AFFINITY_TTL_MS: i64 = 24 * 60 * 60 * 1000;
const MAX_RESPONSE_AFFINITY_BINDINGS: usize = 4096;
const PREPARED_ACCOUNT_CACHE_TTL_MS: i64 = 30 * 1000;
const STATE_RECENT_USAGE_EVENT_LIMIT: usize = 100;
const DEFAULT_MODEL_PRICING_VERSION: u64 = 3;
const MODEL_PRICING_REPRICE_BATCH_SIZE: i64 = 1_000;
const MODEL_PRICING_REPRICE_PARALLEL_MIN_ROWS: usize = 2_000;
const LOCAL_ACCESS_LOGS_DB_BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const COOLDOWN_KEY_SEPARATOR: &str = "\u{1f}";
const CUSTOM_ROUTING_PRIORITY_MIN: i32 = 0;
const CUSTOM_ROUTING_PRIORITY_MAX: i32 = 100;
const CUSTOM_ROUTING_WEIGHT_MIN: u32 = 1;
const CUSTOM_ROUTING_WEIGHT_MAX: u32 = 100;
const BOUND_OAUTH_QUOTA_RESERVE_MIN_PERCENT: i32 = 1;
const BOUND_OAUTH_QUOTA_RESERVE_MAX_PERCENT: i32 = 100;
const BOUND_OAUTH_QUOTA_RESERVE_MAX_SNAPSHOT_AGE_SECONDS: i64 = 3 * 60;
const BOUND_OAUTH_QUOTA_RESERVE_REFRESH_INTERVAL: Duration = Duration::from_secs(60);
const BOUND_OAUTH_QUOTA_RESERVE_MONITOR_TICK: Duration = Duration::from_secs(5);
const BOUND_OAUTH_QUOTA_RESERVE_REQUEST_REFRESH_MIN_INTERVAL: Duration = Duration::from_secs(30);
const GATEWAY_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const GATEWAY_PORT_RELEASE_TIMEOUT: Duration = Duration::from_secs(5);
const GATEWAY_PORT_RELEASE_POLL_INTERVAL: Duration = Duration::from_millis(100);
const LOCAL_ACCESS_PORT_RECOVERY_ATTEMPTS: usize = 5;
const GATEWAY_ACCOUNT_REFRESH_CONCURRENCY: usize = 4;
const GATEWAY_ACCOUNT_REFRESH_TIMEOUT: Duration = Duration::from_secs(30);
const GATEWAY_PREPARATION_CANCELLED: &str = "GATEWAY_PREPARATION_CANCELLED";
const SIDECAR_READY_TIMEOUT: Duration = Duration::from_secs(15);
const UPSTREAM_CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
const DEFAULT_OPENAI_RESPONSES_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_CODEX_USER_AGENT: &str =
    "codex-tui/0.153.4 (Mac OS 26.5.0; arm64) iTerm.app/3.6.10 (codex-tui; 0.153.4)";
const DEFAULT_CODEX_ORIGINATOR: &str = "codex-tui";
const CODEX_RESPONSES_WEBSOCKET_BETA_HEADER_VALUE: &str = "responses_websockets=2026-02-06";
const CODEX_RESPONSES_LITE_HEADER: &str = "x-openai-internal-codex-responses-lite";
const MAX_GPT_REASONING_SIGNATURE_LEN: usize = 32 * 1024 * 1024;
const CODEX_WEBSOCKET_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const CODEX_WEBSOCKET_INITIAL_MESSAGE_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(not(test))]
const CODEX_WEBSOCKET_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
#[cfg(test)]
const CODEX_WEBSOCKET_HEARTBEAT_INTERVAL: Duration = Duration::from_millis(25);
const CODEX_WEBSOCKET_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const CODEX_WEBSOCKET_PROXY_CONNECT_MAX_BYTES: usize = 16 * 1024;
const CORS_ALLOW_HEADERS: &str = "Authorization, Content-Type, OpenAI-Beta, X-API-Key, X-Codex-Beta-Features, X-Codex-Turn-State, X-Codex-Turn-Metadata, X-Client-Request-Id, X-ResponsesAPI-Include-Timing-Metrics, Version, Originator, Session-Id, Session_id, Conversation_id, ChatGPT-Account-Id, X-Codex-Window-Id, Thread-Id";
const CODEX_OFFICIAL_EMPTY_HEADERS: &[&str] = &[
    "version",
    "x-codex-turn-state",
    "x-codex-turn-metadata",
    "x-client-request-id",
    "x-responsesapi-include-timing-metrics",
    "session-id",
    "thread-id",
    "x-codex-window-id",
];
const LEGACY_DEFAULT_CODEX_MODELS: &[&str] = &["gpt-5.5", "gpt-5.4", "gpt-5.4-mini"];
const COMPATIBILITY_CODEX_MODELS: &[&str] = &["gpt-5.3-codex", "gpt-5.3-codex-spark"];
const CODEX_IMAGE_MODEL_ID: &str = "gpt-image-2";
const CODEX_GPT_RESERVE_MODEL_ID: &str = "gpt-reserve";
const CODEX_AUTO_REVIEW_MODEL_ID: &str = "codex-auto-review";
const DEFAULT_IMAGES_MAIN_MODEL: &str = "gpt-5.4-mini";
const MAX_MODEL_PRICE_USD_PER_MILLION: f64 = 1_000_000.0;
const CODEX_LOCAL_ACCESS_LONG_CONTEXT_THRESHOLD_TOKENS: u64 = 272_000;
/// Long-context input multiplier (OpenAI above-272k rates).
const CODEX_LOCAL_ACCESS_LONG_CONTEXT_INPUT_MULTIPLIER: f64 = 2.0;
/// Long-context cached-input multiplier (OpenAI above-272k cache rates).
const CODEX_LOCAL_ACCESS_LONG_CONTEXT_CACHE_MULTIPLIER: f64 = 2.0;
/// Long-context output multiplier (OpenAI above-272k rates).
const CODEX_LOCAL_ACCESS_LONG_CONTEXT_OUTPUT_MULTIPLIER: f64 = 1.5;
const CHAT_COMPLETIONS_PATH: &str = "/v1/chat/completions";
const RESPONSES_PATH: &str = "/v1/responses";
const RESPONSES_COMPACT_PATH: &str = "/v1/responses/compact";
const BACKEND_CODEX_PREFIX: &str = "/backend-api/codex";
const BACKEND_CODEX_RESPONSES_PATH: &str = "/backend-api/codex/responses";
const BACKEND_CODEX_RESPONSES_COMPACT_PATH: &str = "/backend-api/codex/responses/compact";
const IMAGES_GENERATIONS_PATH: &str = "/v1/images/generations";
const IMAGES_EDITS_PATH: &str = "/v1/images/edits";
static GATEWAY_RUNTIME: OnceLock<TokioMutex<GatewayRuntime>> = OnceLock::new();
static GATEWAY_RUNTIME_LOAD_IN_FLIGHT: AtomicBool = AtomicBool::new(false);
static GATEWAY_RUNTIME_LOAD_NOTIFY: OnceLock<Notify> = OnceLock::new();
static API_SERVICE_EXPERIMENTAL_MODEL_CATALOG: OnceLock<Mutex<Option<Vec<String>>>> =
    OnceLock::new();
static GATEWAY_STATS_MAINTENANCE_RUNNING: AtomicBool = AtomicBool::new(false);
static GATEWAY_STATS_MAINTENANCE_COMPLETED: AtomicBool = AtomicBool::new(false);
static GATEWAY_COLLECTION_ACCOUNT_SANITIZE_RUNNING: AtomicBool = AtomicBool::new(false);
static GATEWAY_COLLECTION_ACCOUNT_SANITIZE_COMPLETED: AtomicBool = AtomicBool::new(false);
static GATEWAY_LIFECYCLE_LOCK: OnceLock<TokioMutex<()>> = OnceLock::new();
static GATEWAY_LIFECYCLE_GENERATION: AtomicU64 = AtomicU64::new(1);
static GATEWAY_LIFECYCLE_NOTIFY: OnceLock<Notify> = OnceLock::new();
static GATEWAY_PREPARING: AtomicBool = AtomicBool::new(false);
static GATEWAY_STOP_REQUESTS: AtomicUsize = AtomicUsize::new(0);
static GATEWAY_PREPARATION_TOTAL: AtomicUsize = AtomicUsize::new(0);
static GATEWAY_PREPARATION_COMPLETED: AtomicUsize = AtomicUsize::new(0);
static GATEWAY_ACCOUNT_REFRESH_RUNNING: AtomicBool = AtomicBool::new(false);
static GATEWAY_ACCOUNT_REFRESH_TOTAL: AtomicUsize = AtomicUsize::new(0);
static GATEWAY_ACCOUNT_REFRESH_COMPLETED: AtomicUsize = AtomicUsize::new(0);
static MODEL_PRICING_REPRICE_WORKER: OnceLock<TokioMutex<ModelPricingRepriceWorkerState>> =
    OnceLock::new();
// ponytail: 单进程 SQLite 写队列；只有写入吞吐成为瓶颈时再换专用 DB writer task。
static LOCAL_ACCESS_LOGS_DB_WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static PROVIDER_GATEWAY_RUNTIMES: OnceLock<TokioMutex<HashMap<String, ProviderGatewayRuntime>>> =
    OnceLock::new();
static PROVIDER_GATEWAY_LIFECYCLE_LOCK: OnceLock<TokioMutex<()>> = OnceLock::new();
static GATEWAY_ROUND_ROBIN_CURSOR: AtomicUsize = AtomicUsize::new(0);
static UPSTREAM_HTTP_CLIENT: OnceLock<Mutex<Option<CachedUpstreamHttpClient>>> = OnceLock::new();
static BOUND_OAUTH_QUOTA_REFRESH_FAILURES: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static BOUND_OAUTH_QUOTA_REFRESH_CONTROL: OnceLock<TokioMutex<BoundOauthQuotaRefreshControl>> =
    OnceLock::new();
static SIDECAR_AUTO_RESTART_CONTROL: OnceLock<Mutex<SidecarAutoRestartControl>> = OnceLock::new();
static SIDECAR_CRASH_RECOVERY_CONTROL: OnceLock<Mutex<SidecarAutoRestartControl>> = OnceLock::new();
static BOUND_OAUTH_QUOTA_MONITOR_STARTED: AtomicBool = AtomicBool::new(false);
static CODEX_CLIENT_POLICY_SYNC_RUNNING: AtomicBool = AtomicBool::new(false);
static MODEL_PROVIDER_CHAT_TEST_CANCELLATION: OnceLock<ModelProviderChatTestCancellationState> =
    OnceLock::new();

pub const MODEL_PROVIDER_CHAT_TEST_CANCELLED_ERROR: &str = "MODEL_PROVIDER_CHAT_TEST_CANCELLED";

const SIDECAR_AUTO_RESTART_MIN_INTERVAL: Duration = Duration::from_secs(30);
const SIDECAR_AUTO_RESTART_WINDOW: Duration = Duration::from_secs(10 * 60);
const SIDECAR_AUTO_RESTART_MAX_ATTEMPTS: u8 = 3;
const SIDECAR_CRASH_RECOVERY_MIN_INTERVAL: Duration = Duration::from_secs(3);
const SIDECAR_PROCESS_MONITOR_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Default)]
struct SidecarAutoRestartControl {
    in_flight: bool,
    window_started_at: Option<Instant>,
    last_started_at: Option<Instant>,
    attempts: u8,
}

#[derive(Debug, Clone)]
struct SidecarProcessExit {
    pid: u32,
    generation: u64,
    message: String,
}

#[derive(Default)]
struct ModelProviderChatTestCancellationInner {
    active_run_ids: HashSet<String>,
    cancelled_run_ids: HashSet<String>,
}

#[derive(Default)]
struct ModelProviderChatTestCancellationState {
    inner: Mutex<ModelProviderChatTestCancellationInner>,
    notify: Notify,
}

fn model_provider_chat_test_cancellation() -> &'static ModelProviderChatTestCancellationState {
    MODEL_PROVIDER_CHAT_TEST_CANCELLATION
        .get_or_init(ModelProviderChatTestCancellationState::default)
}

fn with_model_provider_chat_test_cancellation<R>(
    operation: impl FnOnce(&mut ModelProviderChatTestCancellationInner) -> R,
) -> R {
    let state = model_provider_chat_test_cancellation();
    let mut guard = state
        .inner
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    operation(&mut guard)
}

pub fn register_model_provider_chat_test_run(run_id: &str) {
    let run_id = run_id.trim();
    if run_id.is_empty() {
        return;
    }
    with_model_provider_chat_test_cancellation(|state| {
        state.cancelled_run_ids.remove(run_id);
        state.active_run_ids.insert(run_id.to_string());
    });
}

pub fn cancel_model_provider_chat_test_run(run_id: &str) -> bool {
    let run_id = run_id.trim();
    if run_id.is_empty() {
        return false;
    }
    let cancelled = with_model_provider_chat_test_cancellation(|state| {
        if !state.active_run_ids.contains(run_id) {
            return false;
        }
        state.cancelled_run_ids.insert(run_id.to_string());
        true
    });
    if cancelled {
        model_provider_chat_test_cancellation()
            .notify
            .notify_waiters();
    }
    cancelled
}

pub fn finish_model_provider_chat_test_run(run_id: &str) {
    let run_id = run_id.trim();
    if run_id.is_empty() {
        return;
    }
    with_model_provider_chat_test_cancellation(|state| {
        state.active_run_ids.remove(run_id);
        state.cancelled_run_ids.remove(run_id);
    });
}

pub fn is_model_provider_chat_test_cancelled(run_id: &str) -> bool {
    let run_id = run_id.trim();
    if run_id.is_empty() {
        return false;
    }
    with_model_provider_chat_test_cancellation(|state| state.cancelled_run_ids.contains(run_id))
}

async fn wait_for_model_provider_chat_test_cancellation(run_id: &str) {
    loop {
        let notified = model_provider_chat_test_cancellation().notify.notified();
        if is_model_provider_chat_test_cancelled(run_id) {
            return;
        }
        notified.await;
    }
}

#[cfg(test)]
mod model_provider_chat_test_cancellation_tests {
    use super::{
        cancel_model_provider_chat_test_run, finish_model_provider_chat_test_run,
        is_model_provider_chat_test_cancelled, register_model_provider_chat_test_run,
    };

    #[test]
    fn cancellation_only_applies_to_active_run() {
        let run_id = format!("provider-test-cancel-{}", uuid::Uuid::new_v4());
        assert!(!cancel_model_provider_chat_test_run(&run_id));

        register_model_provider_chat_test_run(&run_id);
        assert!(!is_model_provider_chat_test_cancelled(&run_id));
        assert!(cancel_model_provider_chat_test_run(&run_id));
        assert!(is_model_provider_chat_test_cancelled(&run_id));

        finish_model_provider_chat_test_run(&run_id);
        assert!(!is_model_provider_chat_test_cancelled(&run_id));
        assert!(!cancel_model_provider_chat_test_run(&run_id));
    }
}

#[derive(Default)]
struct BoundOauthQuotaRefreshControl {
    in_flight: bool,
    last_account_id: Option<String>,
    last_started_at: Option<Instant>,
}

#[derive(Default)]
struct GatewayRuntime {
    loaded: bool,
    collection: Option<CodexLocalAccessCollection>,
    collection_dirty: bool,
    stats: CodexLocalAccessStats,
    stats_dirty: bool,
    stats_revision: u64,
    stats_flush_inflight: bool,
    response_affinity: HashMap<String, ResponseAffinityBinding>,
    model_cooldowns: HashMap<String, AccountModelCooldown>,
    account_health: HashMap<String, RuntimeAccountHealth>,
    account_quota_cooldowns: HashMap<String, AccountQuotaCooldown>,
    account_pool_health: HashMap<String, RuntimeAccountPoolHealth>,
    prepared_accounts: HashMap<String, CachedPreparedAccount>,
    running: bool,
    actual_port: Option<u16>,
    actual_bind_host: Option<String>,
    sidecar_config_fingerprint: Option<String>,
    last_error: Option<String>,
    shutdown_sender: Option<watch::Sender<bool>>,
    task: Option<tokio::task::JoinHandle<()>>,
    sidecar_monitor_task: Option<tokio::task::JoinHandle<()>>,
    sidecar_generation: Option<u64>,
    sidecar_child: Option<Child>,
}

#[derive(Default)]
struct ModelPricingRepriceWorkerState {
    running: bool,
    pending: Option<ModelPricingRepriceJob>,
    next_job_id: u64,
    active_model_ids: Vec<String>,
}

#[derive(Clone)]
struct ModelPricingRepriceJob {
    job_id: u64,
    collection: CodexLocalAccessCollection,
    model_ids: Vec<String>,
}

#[derive(Debug, Clone)]
struct RequestLogRepriceRow {
    id: i64,
    event_key: String,
    timestamp: i64,
    account_id: String,
    api_key_id: String,
    model_id: String,
    usage: UsageCapture,
    previous_cost_usd: f64,
    previous_model_pricing_version: u64,
    previous_input_usd_per_million: f64,
    previous_output_usd_per_million: f64,
    previous_cached_input_usd_per_million: Option<f64>,
    service_tier: String,
}

struct RequestLogRepriceUpdate {
    id: i64,
    estimated_cost_usd: f64,
    model_pricing_version: u64,
    input_usd_per_million: f64,
    output_usd_per_million: f64,
    cached_input_usd_per_million: Option<f64>,
    change: Option<RequestLogRepriceChange>,
}

#[derive(Default)]
struct ProviderGatewayRuntime {
    actual_port: Option<u16>,
    actual_bind_host: Option<String>,
    task: Option<tokio::task::JoinHandle<()>>,
    sidecar_child: Option<Child>,
    sidecar_dir: Option<PathBuf>,
    collection: Option<CodexLocalAccessCollection>,
    oauth_account_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderGatewayProfileState {
    api_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CodexLocalAccessTakeoverBackups {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    profiles: Vec<CodexLocalAccessProfileTakeoverBackup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexLocalAccessProfileTakeoverBackup {
    profile_dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auth_json: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    config_toml: Option<String>,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Clone)]
struct GatewayBindEndpoint {
    bind_host: String,
    port: u16,
}

#[derive(Debug, Clone, Default)]
struct UsageCapture {
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
    cached_tokens: u64,
    reasoning_tokens: u64,
    token_breakdown: Option<CodexTokenBreakdown>,
}

#[derive(Debug, Clone, Default)]
struct ResponseCapture {
    usage: Option<UsageCapture>,
    response_id: Option<String>,
    response_model: Option<String>,
    terminal_error: Option<String>,
}

#[derive(Debug, Clone)]
struct UpstreamResponseFailedSignal {
    event_type: String,
    code: Option<String>,
    error_type: Option<String>,
    message: Option<String>,
    raw: String,
}

#[derive(Debug, Clone)]
struct WebSocketUpstreamError {
    status: u16,
    body: String,
    category: String,
    retry_after: Option<Duration>,
}

#[derive(Debug, Clone)]
struct WebSocketConnectError {
    status: Option<u16>,
    message: String,
    category: String,
}

#[derive(Debug, Clone, Default)]
struct WebSocketBridgeResult {
    capture: ResponseCapture,
    upstream_error: Option<WebSocketUpstreamError>,
}

#[derive(Debug, Clone, Default)]
struct ImageCallResult {
    result: String,
    revised_prompt: String,
    output_format: String,
    size: String,
    background: String,
    quality: String,
}

#[derive(Debug, Clone)]
struct MultipartFilePart {
    name: String,
    content_type: String,
    data: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
struct MultipartFormData {
    fields: HashMap<String, String>,
    files: Vec<MultipartFilePart>,
}

#[derive(Debug, Clone)]
struct ResponseAffinityBinding {
    account_id: String,
    updated_at_ms: i64,
}

#[derive(Debug, Clone)]
struct AccountModelCooldown {
    model_key: String,
    next_retry_at_ms: i64,
    reason: String,
}

#[derive(Debug, Clone, Default)]
struct RuntimeAccountHealth {
    email: String,
    consecutive_failures: u32,
    last_success_at: Option<i64>,
    last_failure_at: Option<i64>,
    last_failure_status: Option<u16>,
    last_failure_category: Option<String>,
    last_failure_message: Option<String>,
    image_generation_status: CodexLocalAccessImageGenerationStatus,
    image_generation_checked_at: Option<i64>,
    sidecar_scheduler_available: Option<bool>,
    sidecar_scheduler_reason: Option<String>,
    sidecar_scheduler_next_retry_at: Option<i64>,
}

#[derive(Debug, Clone, Default)]
struct RuntimeAccountPoolHealth {
    api_key_id: String,
    api_key_label: String,
    provider: String,
    model: String,
    request_kind: String,
    error_code: String,
    error_message: String,
    diagnostic_available: bool,
    candidate_auths: usize,
    scoped_auths: usize,
    available_auths: usize,
    unavailable_auths: usize,
    model_excluded_auths: usize,
    quota_reserved_auths: usize,
    image_policy_blocked_auths: usize,
    account_statuses: Vec<RuntimeAccountPoolMemberHealth>,
    last_failure_at: i64,
}

#[derive(Debug, Clone, Default)]
struct RuntimeAccountPoolMemberHealth {
    account_id: String,
    account_email: String,
    available: bool,
    reason_code: String,
    reason_message: String,
}

#[derive(Debug, Clone)]
struct CachedPreparedAccount {
    account: CodexAccount,
    cached_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UpstreamHttpClientSignature {
    proxy_source: UpstreamProxySource,
    proxy_url: Option<String>,
    connect_timeout_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpstreamProxySource {
    ApiService,
    Global,
    SystemEnv,
    SystemAuto,
}

#[derive(Debug, Clone)]
struct UpstreamProxyDiagnostics {
    proxy_source: UpstreamProxySource,
    proxy_url: Option<String>,
}

#[derive(Clone)]
struct CachedUpstreamHttpClient {
    signature: UpstreamHttpClientSignature,
    client: Client,
}

#[derive(Debug)]
struct ProxyDispatchSuccess {
    upstream: reqwest::Response,
    account_id: String,
    account_email: String,
}

#[derive(Debug)]
struct ProxyDispatchError {
    status: u16,
    message: String,
    account_id: Option<String>,
    account_email: Option<String>,
    error_category: Option<String>,
}

#[derive(Debug, Clone)]
struct ResolvedLocalApiKey {
    id: String,
    label: String,
    provider_gateway: Option<CodexLocalAccessProviderGateway>,
    inherit_account_pool: bool,
    account_ids: Vec<String>,
    model_prefix: Option<String>,
    allowed_models: Vec<String>,
    excluded_models: Vec<String>,
    token_limit: Option<u64>,
    token_used: u64,
}

#[derive(Debug, Clone)]
struct RequestStatsContext {
    request_kind: CodexLocalAccessRequestKind,
    model_id: String,
    api_key_id: String,
    api_key_label: String,
}

struct ResponseUsageCollector {
    is_stream: bool,
    body: Vec<u8>,
    stream_buffer: Vec<u8>,
    usage: Option<UsageCapture>,
    response_id: Option<String>,
    response_model: Option<String>,
    terminal_error: Option<String>,
}

#[derive(Debug, Clone)]
struct ParsedRequest {
    method: String,
    target: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

fn request_uses_responses_lite(request: &ParsedRequest) -> bool {
    request
        .headers
        .keys()
        .any(|name| name.eq_ignore_ascii_case(CODEX_RESPONSES_LITE_HEADER))
}

fn request_body_uses_responses_lite(request: &ParsedRequest) -> bool {
    request_uses_responses_lite(request)
        || parse_request_body_json(&request.body)
            .as_ref()
            .and_then(|body| body.get("model"))
            .and_then(Value::as_str)
            .is_some_and(codex_protocol::codex_model_uses_responses_lite)
}

#[derive(Debug, Clone)]
enum GatewayResponseAdapter {
    Passthrough {
        request_is_stream: bool,
    },
    ChatCompletions {
        stream: bool,
        requested_model: String,
        original_request_body: Vec<u8>,
    },
    Images {
        stream: bool,
        response_format: String,
        stream_prefix: String,
    },
}

#[derive(Debug, Clone, Default)]
struct RequestRoutingHint {
    model_key: String,
    previous_response_id: Option<String>,
    session_affinity_key: Option<String>,
}

#[derive(Debug)]
struct WebSocketDispatchSuccess {
    upstream: WebSocketStream<MaybeTlsStream<TcpStream>>,
    account: CodexAccount,
    account_id: String,
    account_email: String,
}

#[derive(Debug, Clone)]
struct RoutingCandidate {
    account_id: String,
    plan_rank: Option<i32>,
    remaining_quota: Option<i32>,
    subscription_expiry_ms: Option<i64>,
}

fn gateway_runtime() -> &'static TokioMutex<GatewayRuntime> {
    GATEWAY_RUNTIME.get_or_init(|| TokioMutex::new(GatewayRuntime::default()))
}

fn gateway_runtime_load_notify() -> &'static Notify {
    GATEWAY_RUNTIME_LOAD_NOTIFY.get_or_init(Notify::new)
}

struct GatewayRuntimeLoadGuard;

impl Drop for GatewayRuntimeLoadGuard {
    fn drop(&mut self) {
        GATEWAY_RUNTIME_LOAD_IN_FLIGHT.store(false, Ordering::SeqCst);
        gateway_runtime_load_notify().notify_waiters();
    }
}

fn model_pricing_reprice_worker() -> &'static TokioMutex<ModelPricingRepriceWorkerState> {
    MODEL_PRICING_REPRICE_WORKER
        .get_or_init(|| TokioMutex::new(ModelPricingRepriceWorkerState::default()))
}

fn local_access_logs_db_write_lock() -> &'static Mutex<()> {
    LOCAL_ACCESS_LOGS_DB_WRITE_LOCK.get_or_init(|| Mutex::new(()))
}

fn lock_local_access_logs_db_write() -> Result<std::sync::MutexGuard<'static, ()>, String> {
    local_access_logs_db_write_lock()
        .lock()
        .map_err(|_| "API 服务日志写入队列已损坏".to_string())
}

fn gateway_lifecycle_lock() -> &'static TokioMutex<()> {
    GATEWAY_LIFECYCLE_LOCK.get_or_init(|| TokioMutex::new(()))
}

fn gateway_lifecycle_notify() -> &'static Notify {
    GATEWAY_LIFECYCLE_NOTIFY.get_or_init(Notify::new)
}

fn current_gateway_lifecycle_generation() -> u64 {
    GATEWAY_LIFECYCLE_GENERATION.load(Ordering::SeqCst)
}

fn gateway_lifecycle_generation_changed(expected: u64) -> bool {
    current_gateway_lifecycle_generation() != expected
}

fn advance_gateway_lifecycle_generation() -> u64 {
    let next = GATEWAY_LIFECYCLE_GENERATION
        .fetch_add(1, Ordering::SeqCst)
        .wrapping_add(1);
    GATEWAY_PREPARING.store(false, Ordering::SeqCst);
    GATEWAY_PREPARATION_TOTAL.store(0, Ordering::SeqCst);
    GATEWAY_PREPARATION_COMPLETED.store(0, Ordering::SeqCst);
    GATEWAY_ACCOUNT_REFRESH_TOTAL.store(0, Ordering::SeqCst);
    GATEWAY_ACCOUNT_REFRESH_COMPLETED.store(0, Ordering::SeqCst);
    gateway_lifecycle_notify().notify_waiters();
    next
}

async fn wait_for_gateway_lifecycle_generation_change(expected: u64) {
    loop {
        let notified = gateway_lifecycle_notify().notified();
        if gateway_lifecycle_generation_changed(expected) {
            return;
        }
        notified.await;
    }
}

#[derive(Clone, Copy)]
struct GatewayPreparationContext {
    generation: u64,
    total: usize,
}

struct GatewayPreparationGuard {
    generation: u64,
}

struct GatewayStopRequestGuard;

impl GatewayStopRequestGuard {
    fn begin() -> Self {
        GATEWAY_STOP_REQUESTS.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for GatewayStopRequestGuard {
    fn drop(&mut self) {
        GATEWAY_STOP_REQUESTS.fetch_sub(1, Ordering::SeqCst);
    }
}

impl GatewayPreparationGuard {
    fn begin(total: usize) -> Self {
        let generation = current_gateway_lifecycle_generation();
        GATEWAY_PREPARATION_TOTAL.store(total, Ordering::SeqCst);
        GATEWAY_PREPARATION_COMPLETED.store(0, Ordering::SeqCst);
        GATEWAY_PREPARING.store(true, Ordering::SeqCst);
        Self { generation }
    }

    fn context(&self, total: usize) -> GatewayPreparationContext {
        GatewayPreparationContext {
            generation: self.generation,
            total,
        }
    }
}

impl Drop for GatewayPreparationGuard {
    fn drop(&mut self) {
        if !gateway_lifecycle_generation_changed(self.generation) {
            GATEWAY_PREPARING.store(false, Ordering::SeqCst);
        }
    }
}

fn update_gateway_preparation_progress(context: GatewayPreparationContext, completed: usize) {
    if gateway_lifecycle_generation_changed(context.generation) {
        return;
    }
    GATEWAY_PREPARATION_COMPLETED.store(completed.min(context.total), Ordering::SeqCst);
}

fn upstream_http_client_cache() -> &'static Mutex<Option<CachedUpstreamHttpClient>> {
    UPSTREAM_HTTP_CLIENT.get_or_init(|| Mutex::new(None))
}

fn bound_oauth_quota_refresh_failures() -> &'static Mutex<HashSet<String>> {
    BOUND_OAUTH_QUOTA_REFRESH_FAILURES.get_or_init(|| Mutex::new(HashSet::new()))
}

fn duration_to_millis(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

fn duration_from_millis(value: u64, fallback: Duration) -> Duration {
    if value == 0 {
        return fallback;
    }
    Duration::from_millis(value)
}

fn upstream_env_proxy_url() -> Option<String> {
    const ENV_PROXY_KEYS: [&str; 6] = [
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ];

    for key in ENV_PROXY_KEYS {
        if let Ok(value) = std::env::var(key) {
            let proxy_url = value.trim();
            if !proxy_url.is_empty() {
                return Some(proxy_url.to_string());
            }
        }
    }

    None
}

#[cfg_attr(
    not(any(test, target_os = "macos", target_os = "windows")),
    allow(dead_code)
)]
fn system_proxy_target_scheme(target_url: &str) -> String {
    Url::parse(target_url)
        .ok()
        .map(|url| url.scheme().to_ascii_lowercase())
        .filter(|scheme| !scheme.is_empty())
        .unwrap_or_else(|| "https".to_string())
}

#[cfg_attr(
    not(any(test, target_os = "macos", target_os = "windows")),
    allow(dead_code)
)]
fn system_proxy_url_with_scheme(scheme: &str, host: &str, port: u16) -> Option<String> {
    let host = host.trim();
    if host.is_empty() || port == 0 {
        return None;
    }

    let scheme = match scheme.to_ascii_lowercase().as_str() {
        "http" => "http",
        "https" => "https",
        "socks" | "socks5" | "socks5h" => "socks5",
        _ => return None,
    };
    let host = if host.contains(':') && !host.starts_with('[') && !host.ends_with(']') {
        format!("[{}]", host)
    } else {
        host.to_string()
    };
    Some(format!("{}://{}:{}", scheme, host, port))
}

#[cfg_attr(
    not(any(test, target_os = "macos", target_os = "windows")),
    allow(dead_code)
)]
fn system_proxy_host_port_url(entry_kind: &str, host: &str, port: u16) -> Option<String> {
    let scheme = match entry_kind.to_ascii_lowercase().as_str() {
        "socks" | "socks5" | "socks5h" => "socks5",
        _ => "http",
    };
    system_proxy_url_with_scheme(scheme, host, port)
}

#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
fn system_proxy_value_url(entry_kind: &str, value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    if let Ok(url) = Url::parse(value) {
        let scheme = match url.scheme().to_ascii_lowercase().as_str() {
            "http" => Some("http"),
            "https" => Some("https"),
            "socks" | "socks5" | "socks5h" => Some("socks5"),
            _ => None,
        };
        if let Some(scheme) = scheme {
            let host = url.host_str()?;
            let port = url.port_or_known_default()?;
            return system_proxy_url_with_scheme(scheme, host, port);
        }
        if value.contains("://") {
            return None;
        }
    }

    let (host, port) = value.rsplit_once(':')?;
    let port = port.trim().parse::<u16>().ok()?;
    system_proxy_host_port_url(entry_kind, host, port)
}

#[cfg_attr(not(any(test, target_os = "macos")), allow(dead_code))]
fn scutil_proxy_map(output: &str) -> HashMap<String, String> {
    output
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            let key = key.trim().trim_matches('"');
            let value = value.trim().trim_matches('"');
            if key.is_empty() || value.is_empty() {
                return None;
            }
            Some((key.to_string(), value.to_string()))
        })
        .collect()
}

#[cfg_attr(not(any(test, target_os = "macos")), allow(dead_code))]
fn system_proxy_flag_enabled(value: Option<&String>) -> bool {
    matches!(
        value.map(|value| value.trim().to_ascii_lowercase()),
        Some(value) if value == "1" || value == "true"
    )
}

#[cfg_attr(not(any(test, target_os = "macos")), allow(dead_code))]
fn macos_proxy_url_from_scutil_map(
    values: &HashMap<String, String>,
    target_scheme: &str,
) -> Option<String> {
    let https_entries = [
        ("HTTPSEnable", "HTTPSProxy", "HTTPSPort", "http"),
        ("HTTPEnable", "HTTPProxy", "HTTPPort", "http"),
        ("SOCKSEnable", "SOCKSProxy", "SOCKSPort", "socks"),
    ];
    let http_entries = [
        ("HTTPEnable", "HTTPProxy", "HTTPPort", "http"),
        ("SOCKSEnable", "SOCKSProxy", "SOCKSPort", "socks"),
    ];
    let entries: &[(&str, &str, &str, &str)] = if target_scheme.eq_ignore_ascii_case("https") {
        &https_entries
    } else {
        &http_entries
    };

    for (enable_key, host_key, port_key, entry_kind) in entries {
        if !system_proxy_flag_enabled(values.get(*enable_key)) {
            continue;
        }
        let host = values.get(*host_key)?;
        let port = values.get(*port_key)?.trim().parse::<u16>().ok()?;
        if let Some(proxy_url) = system_proxy_host_port_url(entry_kind, host, port) {
            return Some(proxy_url);
        }
    }

    None
}

#[cfg(target_os = "macos")]
fn system_proxy_url_for_target(target_url: &str) -> Option<String> {
    let output = StdCommand::new("scutil").arg("--proxy").output().ok()?;
    if !output.status.success() {
        return None;
    }

    let target_scheme = system_proxy_target_scheme(target_url);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let values = scutil_proxy_map(&stdout);
    macos_proxy_url_from_scutil_map(&values, &target_scheme)
}

#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
fn windows_reg_query_map(output: &str) -> HashMap<String, String> {
    output
        .lines()
        .filter_map(|line| {
            let parts = line.split_whitespace().collect::<Vec<_>>();
            if parts.len() < 3 || !parts[1].starts_with("REG_") {
                return None;
            }
            Some((parts[0].to_string(), parts[2..].join(" ")))
        })
        .collect()
}

#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
fn windows_reg_dword_enabled(value: Option<&String>) -> bool {
    value
        .map(|value| value.trim().eq_ignore_ascii_case("0x1") || value.trim() == "1")
        .unwrap_or(false)
}

#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
fn windows_proxy_url_from_server(proxy_server: &str, target_scheme: &str) -> Option<String> {
    let proxy_server = proxy_server.trim();
    if proxy_server.is_empty() {
        return None;
    }

    let entries = proxy_server
        .split(';')
        .filter_map(|entry| {
            let entry = entry.trim();
            let (kind, value) = entry.split_once('=')?;
            let kind = kind.trim().to_ascii_lowercase();
            let value = value.trim();
            if kind.is_empty() || value.is_empty() {
                return None;
            }
            Some((kind, value.to_string()))
        })
        .collect::<HashMap<_, _>>();

    if entries.is_empty() {
        return system_proxy_value_url("http", proxy_server);
    }

    let https_order = ["https", "http", "socks"];
    let http_order = ["http", "socks"];
    let order: &[&str] = if target_scheme.eq_ignore_ascii_case("https") {
        &https_order
    } else {
        &http_order
    };

    for kind in order {
        if let Some(value) = entries.get(*kind) {
            if let Some(proxy_url) = system_proxy_value_url(kind, value) {
                return Some(proxy_url);
            }
        }
    }

    None
}

#[cfg(target_os = "windows")]
fn system_proxy_url_for_target(target_url: &str) -> Option<String> {
    let mut command = StdCommand::new("reg");
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let target_scheme = system_proxy_target_scheme(target_url);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let values = windows_reg_query_map(&stdout);
    if !windows_reg_dword_enabled(values.get("ProxyEnable")) {
        return None;
    }
    windows_proxy_url_from_server(values.get("ProxyServer")?, &target_scheme)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn system_proxy_url_for_target(_target_url: &str) -> Option<String> {
    None
}

fn current_upstream_http_client_signature(
    upstream_proxy_url: Option<&str>,
    connect_timeout: Duration,
) -> UpstreamHttpClientSignature {
    let connect_timeout_ms = duration_to_millis(connect_timeout);
    if let Some(proxy_url) = upstream_proxy_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return UpstreamHttpClientSignature {
            proxy_source: UpstreamProxySource::ApiService,
            proxy_url: Some(proxy_url.to_string()),
            connect_timeout_ms,
        };
    }

    let config = crate::modules::config::get_user_config();
    if config.global_proxy_enabled {
        let proxy_url = config.global_proxy_url.trim();
        if !proxy_url.is_empty() {
            return UpstreamHttpClientSignature {
                proxy_source: UpstreamProxySource::Global,
                proxy_url: Some(proxy_url.to_string()),
                connect_timeout_ms,
            };
        }
    }

    if let Some(proxy_url) = upstream_env_proxy_url() {
        return UpstreamHttpClientSignature {
            proxy_source: UpstreamProxySource::SystemEnv,
            proxy_url: Some(proxy_url),
            connect_timeout_ms,
        };
    }

    UpstreamHttpClientSignature {
        proxy_source: UpstreamProxySource::SystemAuto,
        proxy_url: None,
        connect_timeout_ms,
    }
}

fn redact_proxy_url_for_log(proxy_url: &str) -> String {
    match Url::parse(proxy_url) {
        Ok(mut url) => {
            if !url.username().is_empty() {
                let _ = url.set_username("redacted");
            }
            if url.password().is_some() {
                let _ = url.set_password(Some("redacted"));
            }
            url.to_string()
        }
        Err(_) => "<invalid>".to_string(),
    }
}

fn current_upstream_proxy_diagnostics(
    upstream_proxy_url: Option<&str>,
) -> UpstreamProxyDiagnostics {
    let signature = current_upstream_http_client_signature(
        upstream_proxy_url,
        DEFAULT_UPSTREAM_CONNECT_TIMEOUT,
    );
    UpstreamProxyDiagnostics {
        proxy_source: signature.proxy_source,
        proxy_url: signature.proxy_url.as_deref().map(redact_proxy_url_for_log),
    }
}

fn build_upstream_http_client(signature: &UpstreamHttpClientSignature) -> Result<Client, String> {
    let mut builder = Client::builder().connect_timeout(duration_from_millis(
        signature.connect_timeout_ms,
        DEFAULT_UPSTREAM_CONNECT_TIMEOUT,
    ));

    if let Some(proxy_url) = signature.proxy_url.as_deref() {
        let proxy = Proxy::all(proxy_url).map_err(|e| format!("Codex 上游代理地址无效: {}", e))?;
        builder = builder.proxy(proxy);
    }

    builder
        .build()
        .map_err(|e| format!("创建 Codex 上游 HTTP 客户端失败: {}", e))
}

fn build_localhost_http_client(request_timeout: Duration, label: &str) -> Result<Client, String> {
    Client::builder()
        .no_proxy()
        .timeout(request_timeout)
        .build()
        .map_err(|e| format!("创建{}客户端失败: {}", label, e))
}

fn log_upstream_http_client_signature(signature: &UpstreamHttpClientSignature) {
    match (signature.proxy_source, signature.proxy_url.as_deref()) {
        (UpstreamProxySource::ApiService, Some(proxy_url)) => logger::log_info(&format!(
            "[CodexLocalAccess][legacy] 上游 HTTP 客户端已应用 API 服务代理 proxy_url={}",
            redact_proxy_url_for_log(proxy_url)
        )),
        (UpstreamProxySource::Global, Some(proxy_url)) => logger::log_info(&format!(
            "[CodexLocalAccess][legacy] 上游 HTTP 客户端已跟随全局代理 proxy_url={}，API 服务上游请求不应用 no_proxy 绕过",
            redact_proxy_url_for_log(proxy_url)
        )),
        (UpstreamProxySource::SystemEnv, Some(proxy_url)) => logger::log_info(&format!(
            "[CodexLocalAccess][legacy] 上游 HTTP 客户端已使用环境代理 proxy_url={}，API 服务上游请求不应用 no_proxy 绕过",
            redact_proxy_url_for_log(proxy_url)
        )),
        (UpstreamProxySource::SystemAuto, None) => logger::log_info(
            "[CodexLocalAccess][legacy] 未配置 API 服务代理、全局代理或环境代理，已回退到 reqwest 系统自动代理配置",
        ),
        _ => logger::log_warn("[CodexLocalAccess][legacy] 上游 HTTP 客户端代理状态异常"),
    }
}

fn log_sidecar_proxy_signature(signature: &UpstreamHttpClientSignature) {
    match (signature.proxy_source, signature.proxy_url.as_deref()) {
        (UpstreamProxySource::ApiService, Some(proxy_url)) => logger::log_info(&format!(
            "[CodexLocalAccess][sidecar] 上游代理已按旧网关规则应用 API 服务代理 proxy_url={}",
            redact_proxy_url_for_log(proxy_url)
        )),
        (UpstreamProxySource::Global, Some(proxy_url)) => logger::log_info(&format!(
            "[CodexLocalAccess][sidecar] 上游代理已按旧网关规则跟随全局代理 proxy_url={}",
            redact_proxy_url_for_log(proxy_url)
        )),
        (UpstreamProxySource::SystemEnv, Some(proxy_url)) => logger::log_info(&format!(
            "[CodexLocalAccess][sidecar] 上游代理已按旧网关规则使用环境代理 proxy_url={}",
            redact_proxy_url_for_log(proxy_url)
        )),
        (UpstreamProxySource::SystemAuto, Some(proxy_url)) => logger::log_info(&format!(
            "[CodexLocalAccess][sidecar] 上游代理已从系统代理配置解析并写入 sidecar proxy_url={}",
            redact_proxy_url_for_log(proxy_url)
        )),
        (UpstreamProxySource::SystemAuto, None) => logger::log_info(
            "[CodexLocalAccess][sidecar] 未解析到可写入 sidecar 的系统代理配置，sidecar 将按自身默认网络行为连接上游",
        ),
        _ => logger::log_warn("[CodexLocalAccess][sidecar] 上游代理状态异常"),
    }
}

fn upstream_http_client(
    upstream_proxy_url: Option<&str>,
    connect_timeout: Duration,
) -> Result<Client, String> {
    let signature = current_upstream_http_client_signature(upstream_proxy_url, connect_timeout);
    let mut cache = upstream_http_client_cache()
        .lock()
        .map_err(|_| "Codex 上游 HTTP 客户端缓存已损坏".to_string())?;

    if let Some(cached) = cache.as_ref() {
        if cached.signature == signature {
            return Ok(cached.client.clone());
        }
    }

    let client = build_upstream_http_client(&signature)?;
    log_upstream_http_client_signature(&signature);
    *cache = Some(CachedUpstreamHttpClient {
        signature,
        client: client.clone(),
    });
    Ok(client)
}

fn local_access_file_path() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join(CODEX_LOCAL_ACCESS_FILE))
}

fn local_access_stats_file_path() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join(CODEX_LOCAL_ACCESS_STATS_FILE))
}

fn local_access_logs_db_path() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join(CODEX_LOCAL_ACCESS_LOGS_DB_FILE))
}

fn local_access_takeover_backups_path() -> Result<PathBuf, String> {
    let dir =
        crate::modules::backup_storage::behavior_backup_dir("codex", "local-access", "state")?;
    Ok(dir.join(CODEX_LOCAL_ACCESS_TAKEOVER_BACKUPS_FILE))
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn is_prepared_account_cache_valid(entry: &CachedPreparedAccount, now: i64) -> bool {
    now.saturating_sub(entry.cached_at_ms) <= PREPARED_ACCOUNT_CACHE_TTL_MS
        && (entry.account.is_agent_identity_auth()
            || !codex_account::managed_account_tokens_need_refresh(&entry.account))
}

fn account_has_refresh_token(account: &CodexAccount) -> bool {
    account
        .tokens
        .refresh_token
        .as_deref()
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .is_some()
}

fn account_is_access_token_only(account: &CodexAccount) -> bool {
    !account.is_api_key_auth()
        && !account.is_agent_identity_auth()
        && !account_has_refresh_token(account)
}

fn account_uses_personal_access_token(account: &CodexAccount) -> bool {
    account_is_access_token_only(account) && account.tokens.access_token.trim().starts_with("at-")
}

fn account_uses_codex_fingerprint_convergence(account: &CodexAccount) -> bool {
    !account.is_api_key_auth()
        && !account.is_agent_identity_auth()
        && account.token_source_mode.trim() != "chatgpt_web_session"
        && !account_is_access_token_only(account)
}

fn prune_prepared_account_cache(runtime: &mut GatewayRuntime, now: i64) {
    let allowed_account_ids = runtime.collection.as_ref().map(|collection| {
        effective_sidecar_account_ids(collection)
            .into_iter()
            .collect::<HashSet<_>>()
    });

    runtime.prepared_accounts.retain(|account_id, entry| {
        let in_collection = allowed_account_ids
            .as_ref()
            .map(|ids| ids.contains(account_id))
            .unwrap_or(true);
        in_collection && is_prepared_account_cache_valid(entry, now)
    });
}

fn prune_runtime_account_state(runtime: &mut GatewayRuntime) {
    let Some(collection) = runtime.collection.as_ref() else {
        runtime.prepared_accounts.clear();
        if let Ok(mut failures) = bound_oauth_quota_refresh_failures().lock() {
            failures.clear();
        }
        runtime.account_health.clear();
        runtime.account_quota_cooldowns.clear();
        runtime.account_pool_health.clear();
        runtime.model_cooldowns.clear();
        runtime.response_affinity.clear();
        return;
    };

    let allowed_account_ids = effective_sidecar_account_ids(collection)
        .into_iter()
        .collect::<HashSet<_>>();

    runtime
        .prepared_accounts
        .retain(|account_id, _| allowed_account_ids.contains(account_id));
    let bound_reserve_account_id = collection
        .bound_oauth_quota_reserve
        .as_ref()
        .and_then(|_| normalize_optional_account_ref(collection.bound_oauth_account_id.as_deref()));
    if let Ok(mut failures) = bound_oauth_quota_refresh_failures().lock() {
        failures
            .retain(|account_id| bound_reserve_account_id.as_deref() == Some(account_id.as_str()));
    }
    runtime
        .account_health
        .retain(|account_id, _| allowed_account_ids.contains(account_id));
    runtime
        .account_quota_cooldowns
        .retain(|account_id, _| allowed_account_ids.contains(account_id));
    let allowed_api_key_ids = collection
        .api_keys
        .iter()
        .map(|api_key| api_key.id.as_str())
        .collect::<HashSet<_>>();
    runtime.account_pool_health.retain(|key, _| {
        key == UNSCOPED_ACCOUNT_POOL_HEALTH_KEY || allowed_api_key_ids.contains(key.as_str())
    });
    runtime
        .response_affinity
        .retain(|_, binding| allowed_account_ids.contains(&binding.account_id));
    runtime.model_cooldowns.retain(|key, _| {
        key.split_once(COOLDOWN_KEY_SEPARATOR)
            .map(|(account_id, _)| allowed_account_ids.contains(account_id))
            .unwrap_or(false)
    });
}

fn sync_runtime_collection(
    runtime: &mut GatewayRuntime,
    mut collection: CodexLocalAccessCollection,
) {
    if let Some(current) = runtime.collection.as_ref() {
        for api_key in &mut collection.api_keys {
            if let Some(current_api_key) =
                current.api_keys.iter().find(|item| item.id == api_key.id)
            {
                api_key.token_used = api_key.token_used.max(current_api_key.token_used);
            }
        }
    }
    runtime.collection = Some(collection);
    runtime.loaded = true;
    runtime.last_error = None;
    prune_runtime_account_state(runtime);
    prune_prepared_account_cache(runtime, now_ms());
}

fn normalize_optional_account_ref(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
}

fn validate_local_access_bound_oauth_account(
    bound_oauth_account_id: &str,
) -> Result<CodexAccount, String> {
    let bound_id = normalize_optional_account_ref(Some(bound_oauth_account_id))
        .ok_or_else(|| "请选择要绑定的 OAuth 账号".to_string())?;
    let oauth_account = codex_account::load_account(&bound_id)
        .ok_or_else(|| format!("绑定的 OAuth 账号不存在: {}", bound_id))?;
    validate_loaded_local_access_bound_oauth_account(oauth_account)
}

fn validate_loaded_local_access_bound_oauth_account(
    oauth_account: CodexAccount,
) -> Result<CodexAccount, String> {
    if oauth_account.is_api_key_auth() {
        return Err("API 服务只能绑定 OAuth 账号，不能绑定 API Key 账号".to_string());
    }
    if oauth_account.is_agent_identity_auth() {
        return Err("Agent Identity 账号仅用于 API 服务，不能作为 OAuth 绑定账号".to_string());
    }
    if !codex_account::account_has_refresh_token(&oauth_account) {
        return Err("API 服务只能绑定带 refresh_token 的 OAuth 账号".to_string());
    }
    Ok(oauth_account)
}

fn validate_bound_oauth_quota_reserve(
    reserve: Option<CodexLocalAccessQuotaReserve>,
    has_bound_oauth_account: bool,
) -> Result<Option<CodexLocalAccessQuotaReserve>, String> {
    let Some(reserve) = reserve else {
        return Ok(None);
    };
    if !has_bound_oauth_account {
        return Err("设置 OAuth 保留额度前必须先绑定 OAuth 账号".to_string());
    }
    if !(BOUND_OAUTH_QUOTA_RESERVE_MIN_PERCENT..=BOUND_OAUTH_QUOTA_RESERVE_MAX_PERCENT)
        .contains(&reserve.hourly_percent)
    {
        return Err("5 小时 OAuth 保留额度必须在 1% 到 100% 之间".to_string());
    }
    if !(BOUND_OAUTH_QUOTA_RESERVE_MIN_PERCENT..=BOUND_OAUTH_QUOTA_RESERVE_MAX_PERCENT)
        .contains(&reserve.weekly_percent)
    {
        return Err("周 OAuth 保留额度必须在 1% 到 100% 之间".to_string());
    }
    Ok(Some(reserve))
}

fn normalize_bound_oauth_quota_reserve(
    reserve: &mut Option<CodexLocalAccessQuotaReserve>,
    has_bound_oauth_account: bool,
) -> bool {
    let original = reserve.clone();
    if !has_bound_oauth_account {
        *reserve = None;
        return *reserve != original;
    }
    if let Some(reserve) = reserve.as_mut() {
        reserve.hourly_percent = reserve.hourly_percent.clamp(
            BOUND_OAUTH_QUOTA_RESERVE_MIN_PERCENT,
            BOUND_OAUTH_QUOTA_RESERVE_MAX_PERCENT,
        );
        reserve.weekly_percent = reserve.weekly_percent.clamp(
            BOUND_OAUTH_QUOTA_RESERVE_MIN_PERCENT,
            BOUND_OAUTH_QUOTA_RESERVE_MAX_PERCENT,
        );
    }
    *reserve != original
}

fn valid_quota_remaining_percent(value: i32) -> Option<i32> {
    (0..=100).contains(&value).then_some(value)
}

fn quota_refresh_fail_closed_for_account(account_id: &str) -> bool {
    bound_oauth_quota_refresh_failures()
        .lock()
        .map(|failures| failures.contains(account_id))
        .unwrap_or(true)
}

fn fresh_quota_for_bound_oauth_reserve(
    account: &CodexAccount,
) -> Option<&crate::models::codex::CodexQuota> {
    if account.quota_error.is_some() || quota_refresh_fail_closed_for_account(&account.id) {
        return None;
    }
    let updated_at = account.usage_updated_at?;
    let now = chrono::Utc::now().timestamp();
    if updated_at <= 0
        || updated_at > now
        || now.saturating_sub(updated_at) > BOUND_OAUTH_QUOTA_RESERVE_MAX_SNAPSHOT_AGE_SECONDS
    {
        return None;
    }
    account.quota.as_ref()
}

fn quota_reserve_window_blocks(
    window_present: Option<bool>,
    remaining_percent: Option<i32>,
    threshold_percent: i32,
) -> bool {
    if window_present == Some(false) {
        return false;
    }
    remaining_percent
        .map(|remaining| remaining <= threshold_percent)
        .unwrap_or(true)
}

fn bound_oauth_quota_reserve_blocks_account(
    reserve: &CodexLocalAccessQuotaReserve,
    account: Option<&CodexAccount>,
) -> bool {
    let Some(account) = account else {
        return true;
    };
    let Some(quota) = fresh_quota_for_bound_oauth_reserve(account) else {
        return true;
    };

    quota_reserve_window_blocks(
        quota.hourly_window_present,
        valid_quota_remaining_percent(quota.hourly_percentage),
        reserve.hourly_percent,
    ) || quota_reserve_window_blocks(
        quota.weekly_window_present,
        valid_quota_remaining_percent(quota.weekly_percentage),
        reserve.weekly_percent,
    )
}

fn quota_reserve_warning_threshold(reserve_percent: i32) -> i32 {
    20.max(reserve_percent.saturating_add(5)).min(100)
}

fn build_quota_reserve_status(
    collection: &CodexLocalAccessCollection,
) -> Option<CodexLocalAccessQuotaReserveStatus> {
    let reserve = collection.bound_oauth_quota_reserve.as_ref()?;
    let account_id = normalize_optional_account_ref(collection.bound_oauth_account_id.as_deref())?;
    let account = codex_account::load_account(&account_id);
    let snapshot_updated_at = account.as_ref().and_then(|item| item.usage_updated_at);
    let quota = account
        .as_ref()
        .and_then(fresh_quota_for_bound_oauth_reserve);
    let blocked = bound_oauth_quota_reserve_blocks_account(reserve, account.as_ref());

    let mut effective: Option<(&str, i32, i32)> = None;
    if let Some(quota) = quota {
        let candidates = [
            (
                "hourly",
                quota.hourly_window_present,
                valid_quota_remaining_percent(quota.hourly_percentage),
                reserve.hourly_percent,
            ),
            (
                "weekly",
                quota.weekly_window_present,
                valid_quota_remaining_percent(quota.weekly_percentage),
                reserve.weekly_percent,
            ),
        ];
        for (window, present, remaining, reserve_percent) in candidates {
            if present == Some(false) {
                continue;
            }
            let Some(remaining) = remaining else {
                continue;
            };
            if remaining > quota_reserve_warning_threshold(reserve_percent) {
                continue;
            }
            let replace = effective
                .map(|(current_window, current_remaining, _)| {
                    remaining < current_remaining
                        || (remaining == current_remaining
                            && window == "weekly"
                            && current_window != "weekly")
                })
                .unwrap_or(true);
            if replace {
                effective = Some((window, remaining, reserve_percent));
            }
        }
    }

    Some(CodexLocalAccessQuotaReserveStatus {
        account_id,
        snapshot_updated_at,
        snapshot_fresh: quota.is_some(),
        blocked,
        warning: effective.is_some(),
        effective_window: effective.map(|item| item.0.to_string()),
        effective_remaining_percent: effective.map(|item| item.1),
        effective_reserve_percent: effective.map(|item| item.2),
    })
}

fn apply_bound_oauth_quota_reserve(
    collection: &CodexLocalAccessCollection,
    scoped_account_ids: Vec<String>,
) -> Vec<String> {
    let Some(reserve) = collection.bound_oauth_quota_reserve.as_ref() else {
        return scoped_account_ids;
    };
    let Some(bound_account_id) =
        normalize_optional_account_ref(collection.bound_oauth_account_id.as_deref())
    else {
        return scoped_account_ids;
    };
    if !scoped_account_ids
        .iter()
        .any(|account_id| account_id == &bound_account_id)
    {
        return scoped_account_ids;
    }

    let account = codex_account::load_account(&bound_account_id);
    filter_bound_oauth_quota_reserve_account(
        scoped_account_ids,
        &bound_account_id,
        reserve,
        account.as_ref(),
    )
}

fn filter_bound_oauth_quota_reserve_account(
    mut scoped_account_ids: Vec<String>,
    bound_account_id: &str,
    reserve: &CodexLocalAccessQuotaReserve,
    account: Option<&CodexAccount>,
) -> Vec<String> {
    if bound_oauth_quota_reserve_blocks_account(reserve, account) {
        scoped_account_ids.retain(|account_id| account_id != bound_account_id);
    }
    scoped_account_ids
}

async fn cache_prepared_account(account: &CodexAccount) {
    let mut runtime = gateway_runtime().lock().await;
    let now = now_ms();
    prune_prepared_account_cache(&mut runtime, now);
    runtime.prepared_accounts.insert(
        account.id.clone(),
        CachedPreparedAccount {
            account: account.clone(),
            cached_at_ms: now,
        },
    );
}

async fn invalidate_prepared_account(account_id: &str) {
    let mut runtime = gateway_runtime().lock().await;
    runtime.prepared_accounts.remove(account_id);
}

fn invalidate_prepared_account_if_unlocked(account_id: &str) {
    if let Ok(mut runtime) = gateway_runtime().try_lock() {
        runtime.prepared_accounts.remove(account_id);
    }
}

fn try_get_cached_account_for_routing(account_id: &str) -> Option<CodexAccount> {
    let Ok(mut runtime) = gateway_runtime().try_lock() else {
        return None;
    };
    let now = now_ms();
    prune_prepared_account_cache(&mut runtime, now);
    runtime
        .prepared_accounts
        .get(account_id)
        .filter(|entry| is_prepared_account_cache_valid(entry, now))
        .map(|entry| entry.account.clone())
}

async fn get_prepared_account(account_id: &str) -> Result<CodexAccount, String> {
    if let Some(account) = codex_account::load_account(account_id) {
        if codex_account::account_has_remote_api_auth_rejection(&account) {
            invalidate_prepared_account(account_id).await;
            return Err("access_token 已被 API 服务远端拒绝，请重新授权后再使用".to_string());
        }
    }
    {
        let mut runtime = gateway_runtime().lock().await;
        let now = now_ms();
        prune_prepared_account_cache(&mut runtime, now);
        if let Some(entry) = runtime.prepared_accounts.get(account_id) {
            if is_prepared_account_cache_valid(entry, now) {
                return Ok(entry.account.clone());
            }
        }
    }

    // refresh_token 已失效但 access_token 尚在安全有效期内时，账号仍可临时用于
    // API 服务。这里禁止再触碰 refresh_token，仅把现有 bearer token 交给 sidecar。
    if let Some(account) = codex_account::load_account(account_id).filter(|account| {
        account.requires_reauth
            && !account.is_api_key_auth()
            && !account.is_agent_identity_auth()
            && !account.is_web_session_auth()
            && !codex_oauth::is_token_expired(&account.tokens.access_token)
    }) {
        logger::log_codex_api_info(&format!(
            "[CodexLocalAccess] 账号客户端授权需更新，临时复用有效 access_token: account_id={}",
            account.id
        ));
        cache_prepared_account(&account).await;
        return Ok(account);
    }

    let account = codex_account::prepare_account_for_injection(account_id).await?;
    cache_prepared_account(&account).await;
    Ok(account)
}

fn sidecar_account_needs_background_refresh(account: &CodexAccount) -> bool {
    !account.is_api_key_auth()
        && !account.requires_reauth
        && codex_account::account_has_refresh_token(account)
        && codex_account::managed_account_tokens_need_refresh(account)
}

/// 返回需要维护 sidecar OAuth 文件的完整账号范围。
///
/// 绑定 OAuth 可能只存在于 API Key 或 collection 的绑定字段中，并不一定
/// 出现在普通账号池 `account_ids` 里；这些账号仍必须接收重新授权后的新 Token。
fn sidecar_auth_account_ids(collection: &CodexLocalAccessCollection) -> Vec<String> {
    let mut scoped_account_ids = effective_sidecar_account_ids(collection);
    let mut seen = scoped_account_ids.iter().cloned().collect::<HashSet<_>>();

    // API Key 账号自身不持有 OAuth refresh_token；如果它绑定了 OAuth，
    // sidecar 的后台刷新范围也必须包含绑定主体，否则会等到首次请求才发现授权已失效。
    for account_id in scoped_account_ids.clone() {
        let Some(account) = codex_account::load_account(&account_id) else {
            continue;
        };
        if !account.is_api_key_auth() {
            continue;
        }
        if let Some(bound_id) = account
            .bound_oauth_account_id
            .as_deref()
            .and_then(|value| normalize_optional_account_ref(Some(value)))
        {
            if seen.insert(bound_id.clone()) {
                scoped_account_ids.push(bound_id);
            }
        }
    }
    if let Some(bound_id) = collection
        .bound_oauth_account_id
        .as_deref()
        .and_then(|value| normalize_optional_account_ref(Some(value)))
    {
        if seen.insert(bound_id.clone()) {
            scoped_account_ids.push(bound_id);
        }
    }

    scoped_account_ids
}

fn sidecar_background_refresh_account_ids(collection: &CodexLocalAccessCollection) -> Vec<String> {
    sidecar_auth_account_ids(collection)
        .into_iter()
        .filter(|account_id| {
            codex_account::load_account(account_id)
                .is_some_and(|account| sidecar_account_needs_background_refresh(&account))
        })
        .collect()
}

fn sidecar_auth_account_is_scoped(
    collection: &CodexLocalAccessCollection,
    account_id: &str,
) -> bool {
    sidecar_auth_account_ids(collection)
        .iter()
        .any(|scoped_id| scoped_id == account_id)
}

fn trigger_sidecar_account_refresh_in_background(collection: CodexLocalAccessCollection) {
    if collection_gateway_mode(&collection) != CodexLocalAccessGatewayMode::Sidecar
        || !collection.enabled
        || GATEWAY_STOP_REQUESTS.load(Ordering::SeqCst) > 0
        || GATEWAY_ACCOUNT_REFRESH_RUNNING.swap(true, Ordering::SeqCst)
    {
        return;
    }

    let generation = current_gateway_lifecycle_generation();
    tauri::async_runtime::spawn(async move {
        let candidate_collection = collection.clone();
        let candidates = match tauri::async_runtime::spawn_blocking(move || {
            sidecar_background_refresh_account_ids(&candidate_collection)
        })
        .await
        {
            Ok(candidates) => candidates,
            Err(error) => {
                logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] 后台筛选待刷新账号失败: {}",
                    error
                ));
                Vec::new()
            }
        };

        if !gateway_lifecycle_generation_changed(generation) {
            GATEWAY_ACCOUNT_REFRESH_TOTAL.store(candidates.len(), Ordering::SeqCst);
            GATEWAY_ACCOUNT_REFRESH_COMPLETED.store(0, Ordering::SeqCst);

            stream::iter(candidates)
                .for_each_concurrent(GATEWAY_ACCOUNT_REFRESH_CONCURRENCY, |account_id| async move {
                    if gateway_lifecycle_generation_changed(generation) {
                        return;
                    }
                    let refresh = timeout(
                        GATEWAY_ACCOUNT_REFRESH_TIMEOUT,
                        codex_account::ensure_managed_account_fresh(&account_id),
                    );
                    tokio::select! {
                        _ = wait_for_gateway_lifecycle_generation_change(generation) => {}
                        result = refresh => {
                            match result {
                                Ok(Ok(account)) => {
                                    if let Err(error) = sync_sidecar_auth_file_for_account(&account) {
                                        logger::log_codex_api_warn(&format!(
                                            "[CodexLocalAccess] 后台刷新账号后同步 sidecar 认证失败: account_id={}, error={}",
                                            account_id, error
                                        ));
                                    }
                                }
                                Ok(Err(error)) => logger::log_codex_api_warn(&format!(
                                    "[CodexLocalAccess] 后台刷新账号失败，保留其他可用账号: account_id={}, error={}",
                                    account_id, error
                                )),
                                Err(_) => logger::log_codex_api_warn(&format!(
                                    "[CodexLocalAccess] 后台刷新账号超时，保留本地凭据: account_id={}, timeout_secs={}",
                                    account_id,
                                    GATEWAY_ACCOUNT_REFRESH_TIMEOUT.as_secs()
                                )),
                            }
                        }
                    }
                    if !gateway_lifecycle_generation_changed(generation) {
                        GATEWAY_ACCOUNT_REFRESH_COMPLETED.fetch_add(1, Ordering::SeqCst);
                    }
                })
                .await;
        }

        let should_retry_for_new_generation = gateway_lifecycle_generation_changed(generation);
        GATEWAY_ACCOUNT_REFRESH_RUNNING.store(false, Ordering::SeqCst);
        if should_retry_for_new_generation {
            let next_collection = {
                let runtime = gateway_runtime().lock().await;
                runtime
                    .collection
                    .clone()
                    .filter(|item| item.enabled && runtime.running)
            };
            if GATEWAY_STOP_REQUESTS.load(Ordering::SeqCst) == 0
                && !GATEWAY_PREPARING.load(Ordering::SeqCst)
            {
                if let Some(next_collection) = next_collection {
                    trigger_sidecar_account_refresh_in_background(next_collection);
                }
            }
        }
    });
}

pub struct CodexOfficialWakeupChatResult {
    pub account: CodexAccount,
    pub reply: String,
    pub duration_ms: u64,
}

struct CodexOfficialWakeupHttpResponse {
    account: CodexAccount,
    status: StatusCode,
    body: String,
}

async fn official_wakeup_network_config() -> (Option<String>, CodexLocalAccessTimeouts) {
    if let Err(err) = ensure_runtime_loaded_without_start().await {
        logger::log_warn(&format!(
            "[CodexWakeup] 加载官方直连网络配置失败，使用默认网络配置: {}",
            err
        ));
        return (None, CodexLocalAccessTimeouts::default());
    }

    let runtime = gateway_runtime().lock().await;
    runtime
        .collection
        .as_ref()
        .map(|collection| {
            (
                collection.upstream_proxy_url.clone(),
                collection_timeouts(collection),
            )
        })
        .unwrap_or_else(|| (None, CodexLocalAccessTimeouts::default()))
}

async fn send_agent_identity_wakeup_request_with_base_urls(
    account: &CodexAccount,
    target: &str,
    headers: &HashMap<String, String>,
    body: &[u8],
    upstream_proxy_url: Option<&str>,
    connect_timeout: Duration,
    timeouts: &CodexLocalAccessTimeouts,
    upstream_base_url: &str,
    agent_auth_base_url: &str,
) -> Result<CodexOfficialWakeupHttpResponse, String> {
    let upstream_url = format!("{}{}", upstream_base_url.trim_end_matches('/'), target);
    Url::parse(&upstream_url).map_err(|e| format!("Codex 上游 URL 无效: {}", e))?;
    let mut current = account.clone();
    let mut expected_task_id: Option<String> = None;

    for attempt in 0..=1 {
        let (updated, auth_headers, assertion_task_id) =
            codex_agent_identity::build_authentication_headers_with_base_url(
                &current,
                expected_task_id.as_deref(),
                agent_auth_base_url,
            )
            .await?;
        current = updated;
        let authorization = auth_headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| "Agent Identity 未生成有效 Authorization 头".to_string())?;
        let response = send_upstream_request_with_authorization_url(
            "POST",
            &upstream_url,
            target,
            headers,
            body,
            &current,
            authorization,
            upstream_proxy_url,
            connect_timeout,
            timeouts,
            CodexLocalAccessImageGenerationMode::Disabled,
            CodexLocalAccessRequestKind::Text,
        )
        .await?;
        let status = response.status();
        let raw_body = response
            .text()
            .await
            .map_err(|e| format!("读取官方直连唤醒响应失败: {}", e))?;

        if attempt == 0 && codex_agent_identity::is_task_invalid_response(status, &raw_body) {
            expected_task_id = Some(assertion_task_id);
            continue;
        }

        let body = if status.is_success() {
            raw_body
        } else {
            codex_agent_identity::redact_sensitive_body(&current, &raw_body)
        };
        return Ok(CodexOfficialWakeupHttpResponse {
            account: current,
            status,
            body,
        });
    }

    Err("Agent Identity task 恢复后官方直连唤醒仍失败".to_string())
}

pub async fn run_official_wakeup_chat(
    account_id: &str,
    model: Option<&str>,
    reasoning_effort: Option<&str>,
    prompt: &str,
) -> Result<CodexOfficialWakeupChatResult, String> {
    let account = get_prepared_account(account_id).await?;
    if account.is_api_key_auth() {
        return Err("Codex 官方直连唤醒仅支持 OAuth 账号。".to_string());
    }

    let model = model
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("gpt-5.4");
    let reasoning_effort = reasoning_effort
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("medium");
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err("唤醒提示词不能为空".to_string());
    }

    let request_body = json!({
        "model": model,
        "input": [
            {
                "type": "message",
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": prompt,
                    }
                ],
            }
        ],
        "instructions": "",
        "reasoning": {
            "effort": reasoning_effort,
            "summary": "auto",
        },
        "include": ["reasoning.encrypted_content"],
        "parallel_tool_calls": true,
        "store": false,
        "stream": true,
    });
    let body = serde_json::to_vec(&request_body)
        .map_err(|e| format!("序列化官方直连唤醒请求失败: {}", e))?;
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
    let format_transport_error = |err: String| {
        let detail = err
            .split_once("技术细节:")
            .map(|(_, detail)| detail.trim())
            .filter(|detail| !detail.is_empty())
            .unwrap_or(err.as_str());
        format!(
            "Codex 官方服务暂时不可用，未能连接到所选账号的官方对话服务。请检查网络和代理配置。技术细节: {}",
            detail
        )
    };
    let (account, status, body_text) = if account.is_agent_identity_auth() {
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
        .await
        .map_err(format_transport_error)?;
        (response.account, response.status, response.body)
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
        .map_err(format_transport_error)?;
        let status = response.status();
        let body_text = response
            .text()
            .await
            .map_err(|e| format!("读取官方直连唤醒响应失败: {}", e))?;
        (account, status, body_text)
    };

    if !status.is_success() {
        let message = extract_upstream_error_message(&body_text)
            .unwrap_or_else(|| truncate_diagnostic_text(body_text.trim(), 4000));
        return Err(format!(
            "官方直连唤醒失败({}): {}",
            status.as_u16(),
            message
        ));
    }

    let response_body = parse_responses_payload_from_upstream(body_text.as_bytes())
        .map_err(|e| format!("解析官方直连唤醒响应失败: {}", e))?;
    let reply = extract_output_text_from_response(&response_body);
    if reply.trim().is_empty() {
        return Err("官方直连唤醒未返回可读回复。".to_string());
    }
    if account.is_agent_identity_auth() {
        cache_prepared_account(&account).await;
    }

    Ok(CodexOfficialWakeupChatResult {
        account,
        reply,
        duration_ms: started_at.elapsed().as_millis() as u64,
    })
}

async fn schedule_stats_flush_if_needed() {
    let should_spawn = {
        let mut runtime = gateway_runtime().lock().await;
        if runtime.stats_flush_inflight {
            false
        } else {
            runtime.stats_flush_inflight = true;
            true
        }
    };

    if !should_spawn {
        return;
    }

    tokio::spawn(async move {
        loop {
            tokio::time::sleep(STATS_FLUSH_INTERVAL).await;

            let (stats_snapshot, collection_snapshot) = {
                let mut runtime = gateway_runtime().lock().await;
                if !runtime.stats_dirty && !runtime.collection_dirty {
                    runtime.stats_flush_inflight = false;
                    return;
                }
                let collection_snapshot = runtime
                    .collection_dirty
                    .then(|| runtime.collection.clone())
                    .flatten();
                runtime.stats_dirty = false;
                runtime.collection_dirty = false;
                (stats_snapshot_without_events(&runtime.stats), collection_snapshot)
            };

            if let Err(err) = save_stats_to_disk(&stats_snapshot) {
                logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] 后台写入请求统计失败: {}",
                    err
                ));
                let mut runtime = gateway_runtime().lock().await;
                runtime.stats_dirty = true;
                runtime.collection_dirty |= collection_snapshot.is_some();
                runtime.stats_flush_inflight = false;
                return;
            }
            if let Some(collection_snapshot) = collection_snapshot.as_ref() {
                if let Err(err) = save_collection_to_disk(collection_snapshot) {
                    logger::log_codex_api_warn(&format!(
                        "[CodexLocalAccess] background API key token usage save failed: {}",
                        err
                    ));
                    let mut runtime = gateway_runtime().lock().await;
                    runtime.collection_dirty = true;
                    runtime.stats_flush_inflight = false;
                    return;
                }
            }
        }
    });
}

fn normalize_model_key(model: &str) -> String {
    model.trim().to_ascii_lowercase()
}

fn has_date_snapshot_suffix(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 11
        && bytes[0] == b'-'
        && bytes[5] == b'-'
        && bytes[8] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 0 | 5 | 8) || byte.is_ascii_digit())
}

pub(crate) fn supported_codex_model_ids() -> Vec<String> {
    let mut model_ids = default_codex_model_ids();
    let mut seen_model_ids: HashSet<String> = model_ids
        .iter()
        .map(|model| model.to_ascii_lowercase())
        .collect();
    if let Ok(state) = codex_wakeup::load_state_for_scheduler() {
        for preset in state.model_presets {
            let model = preset.model.trim();
            if !model.is_empty() && seen_model_ids.insert(model.to_ascii_lowercase()) {
                model_ids.push(model.to_string());
            }
        }
    }
    if seen_model_ids.insert(CODEX_IMAGE_MODEL_ID.to_string()) {
        model_ids.push(CODEX_IMAGE_MODEL_ID.to_string());
    }
    if seen_model_ids.insert(CODEX_AUTO_REVIEW_MODEL_ID.to_string()) {
        model_ids.push(CODEX_AUTO_REVIEW_MODEL_ID.to_string());
    }

    model_ids
}

fn merge_api_service_experimental_model_ids(
    mut model_ids: Vec<String>,
    experimental_model_ids: &[String],
) -> Vec<String> {
    let mut seen = model_ids
        .iter()
        .map(|model| model.trim().to_ascii_lowercase())
        .filter(|model| !model.is_empty())
        .collect::<HashSet<_>>();
    for model in experimental_model_ids {
        let model = model.trim();
        if !model.is_empty() && seen.insert(model.to_ascii_lowercase()) {
            model_ids.push(model.to_string());
        }
    }
    model_ids
}

fn api_service_experimental_model_catalog() -> Option<Vec<String>> {
    API_SERVICE_EXPERIMENTAL_MODEL_CATALOG
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

fn api_service_supported_codex_model_ids() -> Vec<String> {
    api_service_experimental_model_catalog().unwrap_or_else(supported_codex_model_ids)
}

fn apply_codex_image_model_visibility(
    mut model_ids: Vec<String>,
    image_allowed: bool,
) -> Vec<String> {
    if image_allowed
        && !model_ids
            .iter()
            .any(|model| model.eq_ignore_ascii_case(CODEX_IMAGE_MODEL_ID))
    {
        model_ids.push(CODEX_IMAGE_MODEL_ID.to_string());
    }
    model_ids
        .into_iter()
        .filter(|model| image_allowed || !model.eq_ignore_ascii_case(CODEX_IMAGE_MODEL_ID))
        .collect()
}

pub(crate) fn refresh_api_service_experimental_model_ids() {
    let mut profile_dirs = vec![codex_account::get_codex_home()];
    if let Ok(store) = crate::modules::codex_instance::load_instance_store() {
        profile_dirs.extend(store.instances.into_iter().filter_map(|instance| {
            let profile = instance.user_data_dir.trim();
            (!profile.is_empty()).then(|| PathBuf::from(profile))
        }));
    }

    let mut seen_profiles = HashSet::new();
    let mut seen_models = HashSet::new();
    let mut model_ids = Vec::new();
    let mut has_enabled_catalog = false;
    for profile_dir in profile_dirs {
        let profile_key = normalize_profile_dir_key(&profile_dir);
        if profile_key.is_empty() || !seen_profiles.insert(profile_key) {
            continue;
        }
        let quick_config = match codex_account::read_quick_config_from_config_toml(&profile_dir) {
            Ok(config) => config,
            Err(error) => {
                logger::log_codex_api_warn(&format!(
                    "刷新 API 服务实验模型时跳过无效配置: profile={}, error={}",
                    profile_dir.display(),
                    error
                ));
                continue;
            }
        };
        if !quick_config.experimental_model_catalog_enabled {
            continue;
        }
        has_enabled_catalog = true;
        for model in quick_config.experimental_model_catalog_models {
            let model_id = model.model_id.trim();
            if !model_id.is_empty() && seen_models.insert(model_id.to_ascii_lowercase()) {
                model_ids.push(model_id.to_string());
            }
        }
    }

    *API_SERVICE_EXPERIMENTAL_MODEL_CATALOG
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
        has_enabled_catalog.then_some(model_ids);
}

fn default_codex_model_ids() -> Vec<String> {
    codex_protocol::managed_codex_model_ids()
        .into_iter()
        .chain(
            LEGACY_DEFAULT_CODEX_MODELS
                .iter()
                .map(|model| model.to_string()),
        )
        .chain(
            COMPATIBILITY_CODEX_MODELS
                .iter()
                .map(|model| model.to_string()),
        )
        .collect()
}

fn account_health_allows_image_generation(health: Option<&RuntimeAccountHealth>) -> bool {
    !matches!(
        health.map(|item| item.image_generation_status),
        Some(
            CodexLocalAccessImageGenerationStatus::Unavailable
                | CodexLocalAccessImageGenerationStatus::Disabled
        )
    )
}

fn account_failure_category_blocks_routing(category: Option<&str>) -> bool {
    matches!(
        category.map(str::trim).filter(|value| !value.is_empty()),
        Some(
            "auth_unavailable"
                | "auth_refresh_failed"
                | "account_prepare_failed"
                | "free_account_restricted"
        )
    )
}

fn account_health_blocks_routing(health: Option<&RuntimeAccountHealth>) -> bool {
    health
        .map(|item| {
            item.consecutive_failures >= 3
                && account_failure_category_blocks_routing(item.last_failure_category.as_deref())
        })
        .unwrap_or(false)
}

fn sidecar_scheduler_blocks_account(health: Option<&RuntimeAccountHealth>, now: i64) -> bool {
    health
        .filter(|item| item.sidecar_scheduler_available == Some(false))
        .map(|item| {
            item.sidecar_scheduler_next_retry_at
                .map(|next_retry_at| next_retry_at > now)
                .unwrap_or(true)
        })
        .unwrap_or(false)
}

fn account_health_blocks_dispatch(health: Option<&RuntimeAccountHealth>, now: i64) -> bool {
    account_health_blocks_routing(health) || sidecar_scheduler_blocks_account(health, now)
}

async fn account_id_blocked_by_health(account_id: &str) -> bool {
    let account_id = account_id.trim();
    if account_id.is_empty() {
        return false;
    }
    let runtime = gateway_runtime().lock().await;
    account_health_blocks_dispatch(runtime.account_health.get(account_id), now_ms())
}

fn selected_accounts_have_image_generation_capacity(
    collection: &CodexLocalAccessCollection,
    health_by_account_id: Option<&HashMap<String, RuntimeAccountHealth>>,
) -> bool {
    let accounts = codex_account::list_accounts_checked().ok();
    selected_account_ids_have_image_generation_capacity(
        &collection.account_ids,
        collection.image_generation_mode,
        accounts.as_deref(),
        health_by_account_id,
    )
}

fn selected_account_ids_have_image_generation_capacity(
    account_ids: &[String],
    image_generation_mode: CodexLocalAccessImageGenerationMode,
    accounts: Option<&[CodexAccount]>,
    health_by_account_id: Option<&HashMap<String, RuntimeAccountHealth>>,
) -> bool {
    if image_generation_mode == CodexLocalAccessImageGenerationMode::Disabled {
        return false;
    }
    let Some(accounts) = accounts else {
        return true;
    };
    let selected: HashSet<&str> = account_ids.iter().map(String::as_str).collect();
    accounts.into_iter().any(|account| {
        selected.contains(account.id.as_str())
            && !account.is_api_key_auth()
            && !is_free_plan_type(account.plan_type.as_deref())
            && account_health_allows_image_generation(
                health_by_account_id.and_then(|health| health.get(account.id.as_str())),
            )
    })
}

fn base_codex_model_ids_for_collection(
    collection: &CodexLocalAccessCollection,
    health_by_account_id: Option<&HashMap<String, RuntimeAccountHealth>>,
) -> Vec<String> {
    let image_allowed =
        selected_accounts_have_image_generation_capacity(collection, health_by_account_id);
    let mut model_ids =
        apply_codex_image_model_visibility(api_service_supported_codex_model_ids(), image_allowed);
    if !model_ids
            .iter()
            .any(|model| model.eq_ignore_ascii_case(CODEX_GPT_RESERVE_MODEL_ID))
    {
        model_ids.push(CODEX_GPT_RESERVE_MODEL_ID.to_string());
    }
    let mut seen = model_ids
        .iter()
        .map(|model| model.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    for account_id in &collection.account_ids {
        let Some(account) = codex_account::load_account(account_id) else {
            continue;
        };
        for mapping in &account.api_model_mappings {
            for model in [&mapping.client_model, &mapping.upstream_model] {
                let model = model.trim();
                if model.is_empty() {
                    continue;
                }
                if seen.insert(model.to_ascii_lowercase()) {
                    model_ids.push(model.to_string());
                }
            }
        }
    }
    model_ids
}

fn normalize_model_rule_value(value: &str) -> String {
    value.trim().to_string()
}

fn normalize_model_rule_list(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .map(|value| normalize_model_rule_value(&value))
        .filter(|value| !value.is_empty())
        .filter(|value| seen.insert(value.to_ascii_lowercase()))
        .collect()
}

fn normalize_account_id_list(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn normalize_model_prefix_value(value: Option<String>) -> Option<String> {
    value
        .map(|item| item.trim().trim_matches('/').trim().to_ascii_lowercase())
        .filter(|item| !item.is_empty())
}

fn normalize_model_aliases(
    values: Vec<CodexLocalAccessModelAlias>,
) -> Vec<CodexLocalAccessModelAlias> {
    let mut seen_aliases = HashSet::new();
    values
        .into_iter()
        .filter_map(|item| {
            let source_model = normalize_model_rule_value(&item.source_model);
            let alias = normalize_model_rule_value(&item.alias);
            if source_model.is_empty() || alias.is_empty() {
                return None;
            }
            let alias_key = alias.to_ascii_lowercase();
            if source_model.eq_ignore_ascii_case(&alias) || !seen_aliases.insert(alias_key) {
                return None;
            }
            Some(CodexLocalAccessModelAlias {
                source_model,
                alias,
                fork: item.fork,
            })
        })
        .collect()
}

fn wildcard_model_matches(pattern: &str, model: &str) -> bool {
    let pattern = pattern.trim().to_ascii_lowercase();
    let model = model.trim().to_ascii_lowercase();
    if pattern.is_empty() || model.is_empty() {
        return false;
    }
    if pattern == "*" {
        return true;
    }
    if !pattern.contains('*') {
        return pattern == model;
    }

    let anchored_start = !pattern.starts_with('*');
    let anchored_end = !pattern.ends_with('*');
    let parts: Vec<&str> = pattern.split('*').filter(|part| !part.is_empty()).collect();
    if parts.is_empty() {
        return true;
    }

    let mut remaining = model.as_str();
    for (index, part) in parts.iter().enumerate() {
        let Some(found) = remaining.find(part) else {
            return false;
        };
        if index == 0 && anchored_start && found != 0 {
            return false;
        }
        let next_start = found + part.len();
        remaining = &remaining[next_start..];
    }

    if anchored_end {
        if let Some(last) = parts.last() {
            return model.ends_with(last);
        }
    }
    true
}

fn model_matches_any_rule(model: &str, rules: &[String]) -> bool {
    rules.iter().any(|rule| wildcard_model_matches(rule, model))
}

fn apply_model_aliases_to_ids(
    model_ids: Vec<String>,
    aliases: &[CodexLocalAccessModelAlias],
) -> Vec<String> {
    if aliases.is_empty() {
        return model_ids;
    }

    let alias_map: HashMap<String, &CodexLocalAccessModelAlias> = aliases
        .iter()
        .map(|alias| (alias.source_model.to_ascii_lowercase(), alias))
        .collect();
    let mut seen = HashSet::new();
    let mut visible = Vec::new();

    for model in model_ids {
        let key = model.to_ascii_lowercase();
        if let Some(alias) = alias_map.get(&key) {
            if alias.fork && seen.insert(key) {
                visible.push(model.clone());
            }
            if seen.insert(alias.alias.to_ascii_lowercase()) {
                visible.push(alias.alias.clone());
            }
        } else if seen.insert(key) {
            visible.push(model);
        }
    }

    visible
}

fn apply_model_filters(
    model_ids: Vec<String>,
    allowed: &[String],
    excluded: &[String],
) -> Vec<String> {
    model_ids
        .into_iter()
        .filter(|model| allowed.is_empty() || model_matches_any_rule(model, allowed))
        .filter(|model| !model_matches_any_rule(model, excluded))
        .collect()
}

fn strip_model_prefix<'a>(model: &'a str, prefix: Option<&str>) -> &'a str {
    let Some(prefix) = prefix.map(str::trim).filter(|item| !item.is_empty()) else {
        return model.trim();
    };
    let trimmed = model.trim();
    let expected = format!("{}/", prefix.trim_matches('/'));
    trimmed
        .strip_prefix(expected.as_str())
        .map(str::trim)
        .unwrap_or(trimmed)
}

fn add_model_prefix(model_ids: Vec<String>, prefix: Option<&str>) -> Vec<String> {
    let Some(prefix) = prefix.map(str::trim).filter(|item| !item.is_empty()) else {
        return model_ids;
    };
    model_ids
        .into_iter()
        .map(|model| format!("{}/{}", prefix.trim_matches('/'), model))
        .collect()
}

fn visible_codex_model_ids_for_collection(
    collection: &CodexLocalAccessCollection,
    health_by_account_id: Option<&HashMap<String, RuntimeAccountHealth>>,
) -> Vec<String> {
    let base = base_codex_model_ids_for_collection(collection, health_by_account_id);
    let aliased = apply_model_aliases_to_ids(base, &collection.model_aliases);
    apply_model_filters(aliased, &[], &collection.excluded_models)
}

fn visible_codex_model_ids_for_api_key(
    collection: &CodexLocalAccessCollection,
    api_key: &ResolvedLocalApiKey,
    health_by_account_id: Option<&HashMap<String, RuntimeAccountHealth>>,
) -> Vec<String> {
    let accounts = codex_account::list_accounts_checked().ok();
    visible_codex_model_ids_for_api_key_with_optional_accounts(
        collection,
        api_key,
        accounts.as_deref(),
        health_by_account_id,
    )
}

fn visible_codex_model_ids_for_api_key_with_accounts(
    collection: &CodexLocalAccessCollection,
    api_key: &ResolvedLocalApiKey,
    accounts: &[CodexAccount],
    health_by_account_id: Option<&HashMap<String, RuntimeAccountHealth>>,
) -> Vec<String> {
    visible_codex_model_ids_for_api_key_with_optional_accounts(
        collection,
        api_key,
        Some(accounts),
        health_by_account_id,
    )
}

fn visible_codex_model_ids_for_api_key_with_optional_accounts(
    collection: &CodexLocalAccessCollection,
    api_key: &ResolvedLocalApiKey,
    accounts: Option<&[CodexAccount]>,
    health_by_account_id: Option<&HashMap<String, RuntimeAccountHealth>>,
) -> Vec<String> {
    visible_codex_model_ids_for_api_key_with_supported_models(
        collection,
        api_key,
        accounts,
        health_by_account_id,
        api_service_supported_codex_model_ids(),
    )
}

fn visible_codex_model_ids_for_api_key_with_supported_models(
    collection: &CodexLocalAccessCollection,
    api_key: &ResolvedLocalApiKey,
    accounts: Option<&[CodexAccount]>,
    health_by_account_id: Option<&HashMap<String, RuntimeAccountHealth>>,
    supported_model_ids: Vec<String>,
) -> Vec<String> {
    let scoped_account_ids = scoped_collection_account_ids(collection, api_key);
    let image_allowed = selected_account_ids_have_image_generation_capacity(
        &scoped_account_ids,
        collection.image_generation_mode,
        accounts,
        health_by_account_id,
    );
    let base = apply_codex_image_model_visibility(supported_model_ids, image_allowed);
    let mut base = base;
    if !base
            .iter()
            .any(|model| model.eq_ignore_ascii_case(CODEX_GPT_RESERVE_MODEL_ID))
    {
        base.push(CODEX_GPT_RESERVE_MODEL_ID.to_string());
    }
    let mut visible = apply_model_filters(
        apply_model_aliases_to_ids(base, &collection.model_aliases),
        &[],
        &collection.excluded_models,
    );
    if let Some(provider_gateway) = api_key.provider_gateway.as_ref() {
        let mut seen: HashSet<String> = visible
            .iter()
            .map(|model| model.trim().to_ascii_lowercase())
            .filter(|model| !model.is_empty())
            .collect();
        for model in apply_model_aliases_to_ids(
            provider_gateway.upstream_models.clone(),
            &collection.model_aliases,
        ) {
            let model = model.trim();
            if !model.is_empty() && seen.insert(model.to_ascii_lowercase()) {
                visible.push(model.to_string());
            }
        }
    }
    let filtered = apply_model_filters(visible, &api_key.allowed_models, &api_key.excluded_models);
    append_codex_internal_model_ids(add_model_prefix(filtered, api_key.model_prefix.as_deref()))
}

fn is_codex_internal_model(model: &str) -> bool {
    model
        .trim()
        .eq_ignore_ascii_case(CODEX_AUTO_REVIEW_MODEL_ID)
}

fn append_codex_internal_model_ids(mut model_ids: Vec<String>) -> Vec<String> {
    if !model_ids.iter().any(|model| is_codex_internal_model(model)) {
        model_ids.push(CODEX_AUTO_REVIEW_MODEL_ID.to_string());
    }
    model_ids
}

fn canonical_model_for_client_model(
    model: &str,
    collection: &CodexLocalAccessCollection,
    api_key: &ResolvedLocalApiKey,
) -> String {
    let without_prefix = strip_model_prefix(model, api_key.model_prefix.as_deref());
    if is_codex_internal_model(without_prefix) {
        return CODEX_AUTO_REVIEW_MODEL_ID.to_string();
    }
    for alias in &collection.model_aliases {
        if alias.alias.eq_ignore_ascii_case(without_prefix) {
            return alias.source_model.clone();
        }
    }
    resolve_supported_model_alias(without_prefix)
}

fn validate_client_model_visible(
    model: &str,
    canonical_model: &str,
    collection: &CodexLocalAccessCollection,
    api_key: &ResolvedLocalApiKey,
    health_by_account_id: Option<&HashMap<String, RuntimeAccountHealth>>,
) -> bool {
    let without_prefix = strip_model_prefix(model, api_key.model_prefix.as_deref());
    if is_codex_internal_model(without_prefix) || is_codex_internal_model(canonical_model) {
        return true;
    }
    let visible = visible_codex_model_ids_for_api_key(collection, api_key, health_by_account_id);
    let visible_match = visible.iter().any(|item| {
        item.eq_ignore_ascii_case(without_prefix)
            || item.eq_ignore_ascii_case(canonical_model)
            || resolve_supported_model_alias(item).eq_ignore_ascii_case(canonical_model)
    });
    if !visible_match {
        return false;
    }
    if !api_key.allowed_models.is_empty()
        && !model_matches_any_rule(without_prefix, &api_key.allowed_models)
        && !model_matches_any_rule(canonical_model, &api_key.allowed_models)
    {
        return false;
    }
    !model_matches_any_rule(without_prefix, &api_key.excluded_models)
        && !model_matches_any_rule(canonical_model, &api_key.excluded_models)
}

fn rewrite_request_model_for_access_policy_value(
    body_value: &mut Value,
    collection: &CodexLocalAccessCollection,
    api_key: &ResolvedLocalApiKey,
    health_by_account_id: Option<&HashMap<String, RuntimeAccountHealth>>,
) -> Result<bool, String> {
    let Some(body_obj) = body_value.as_object_mut() else {
        return Ok(false);
    };
    let Some(model) = body_obj
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return Ok(false);
    };

    let canonical_model = canonical_model_for_client_model(&model, collection, api_key);
    if !validate_client_model_visible(
        &model,
        &canonical_model,
        collection,
        api_key,
        health_by_account_id,
    ) {
        return Err(format!(
            "模型 {} 不在当前 API Key 的可用模型范围内",
            model.trim()
        ));
    }

    if canonical_model == model {
        return Ok(false);
    }
    body_obj.insert("model".to_string(), Value::String(canonical_model));
    Ok(true)
}

fn rewrite_request_model_for_access_policy(
    request: &mut ParsedRequest,
    collection: &CodexLocalAccessCollection,
    api_key: &ResolvedLocalApiKey,
    health_by_account_id: Option<&HashMap<String, RuntimeAccountHealth>>,
) -> Result<(), String> {
    let Some(mut body_value) = parse_request_body_json(&request.body) else {
        return Ok(());
    };
    if !rewrite_request_model_for_access_policy_value(
        &mut body_value,
        collection,
        api_key,
        health_by_account_id,
    )? {
        return Ok(());
    }
    request.body = serde_json::to_vec(&body_value)
        .map_err(|e| format!("序列化模型访问规则后的请求体失败: {}", e))?;
    Ok(())
}

fn resolve_supported_model_alias(model: &str) -> String {
    let trimmed = model.trim();
    let normalized = trimmed.to_ascii_lowercase();

    for alias in supported_codex_model_ids() {
        if normalized == alias {
            return alias;
        }

        if let Some(suffix) = normalized.strip_prefix(&alias) {
            if has_date_snapshot_suffix(suffix) {
                return alias;
            }
        }
    }

    trimmed.to_string()
}

fn rewrite_request_model_alias(body: &[u8]) -> Result<Option<Vec<u8>>, String> {
    let Some(mut body_value) = parse_request_body_json(body) else {
        return Ok(None);
    };

    if !rewrite_request_model_alias_value(&mut body_value) {
        return Ok(None);
    }

    serde_json::to_vec(&body_value)
        .map(Some)
        .map_err(|e| format!("重写请求 model 失败: {}", e))
}

fn rewrite_request_model_alias_value(body_value: &mut Value) -> bool {
    let Some(body_obj) = body_value.as_object_mut() else {
        return false;
    };
    let Some(model) = body_obj.get("model").and_then(Value::as_str) else {
        return false;
    };

    let resolved_model = resolve_supported_model_alias(model);
    if resolved_model == model {
        return false;
    }

    body_obj.insert("model".to_string(), Value::String(resolved_model));
    true
}

fn parse_request_body_json(body: &[u8]) -> Option<Value> {
    if body.is_empty() {
        return None;
    }
    serde_json::from_slice::<Value>(body).ok()
}

fn proxy_target_path(target: &str) -> &str {
    target.split('?').next().unwrap_or(target).trim()
}

fn is_images_generations_request(target: &str) -> bool {
    let path = proxy_target_path(target);
    path == IMAGES_GENERATIONS_PATH || path.ends_with("/images/generations")
}

fn is_images_edits_request(target: &str) -> bool {
    let path = proxy_target_path(target);
    path == IMAGES_EDITS_PATH || path.ends_with("/images/edits")
}

fn is_responses_request(target: &str) -> bool {
    let path = proxy_target_path(target);
    path == RESPONSES_PATH || path == BACKEND_CODEX_RESPONSES_PATH || path.ends_with("/responses")
}

fn is_responses_compact_request(target: &str) -> bool {
    let path = proxy_target_path(target);
    path == RESPONSES_COMPACT_PATH
        || path == BACKEND_CODEX_RESPONSES_COMPACT_PATH
        || path.ends_with("/responses/compact")
}

fn is_backend_codex_request(target: &str) -> bool {
    let path = proxy_target_path(target);
    path == BACKEND_CODEX_PREFIX || path.starts_with(&format!("{}/", BACKEND_CODEX_PREFIX))
}

fn is_backend_codex_responses_websocket_request(target: &str) -> bool {
    proxy_target_path(target) == BACKEND_CODEX_RESPONSES_PATH
}

fn is_supported_proxy_target(target: &str) -> bool {
    target.starts_with("/v1/") || is_backend_codex_request(target)
}

fn request_kind_is_image(request_kind: CodexLocalAccessRequestKind) -> bool {
    matches!(
        request_kind,
        CodexLocalAccessRequestKind::ImageGeneration | CodexLocalAccessRequestKind::ImageEdit
    )
}

fn request_kind_from_adapter(adapter: &GatewayResponseAdapter) -> CodexLocalAccessRequestKind {
    match adapter {
        GatewayResponseAdapter::ChatCompletions { .. } => CodexLocalAccessRequestKind::Text,
        GatewayResponseAdapter::Images { stream_prefix, .. } => {
            if stream_prefix == "image_edit" {
                CodexLocalAccessRequestKind::ImageEdit
            } else {
                CodexLocalAccessRequestKind::ImageGeneration
            }
        }
        GatewayResponseAdapter::Passthrough { .. } => CodexLocalAccessRequestKind::Text,
    }
}

fn request_kind_from_target(target: &str) -> CodexLocalAccessRequestKind {
    if is_images_generations_request(target) {
        CodexLocalAccessRequestKind::ImageGeneration
    } else if is_images_edits_request(target) {
        CodexLocalAccessRequestKind::ImageEdit
    } else {
        CodexLocalAccessRequestKind::Text
    }
}

fn extract_request_model_id(body: &[u8]) -> Option<String> {
    parse_request_body_json(body)
        .and_then(|value| {
            value
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn stats_model_id_for_request_kind(
    body: &[u8],
    request_kind: CodexLocalAccessRequestKind,
) -> String {
    if request_kind_is_image(request_kind) {
        return extract_request_model_id(body).unwrap_or_else(|| CODEX_IMAGE_MODEL_ID.to_string());
    }
    extract_request_model_id(body).unwrap_or_default()
}

fn stats_model_id_from_adapter(
    request: &ParsedRequest,
    adapter: &GatewayResponseAdapter,
) -> String {
    match adapter {
        GatewayResponseAdapter::ChatCompletions {
            requested_model, ..
        } => requested_model.clone(),
        GatewayResponseAdapter::Images { .. } => CODEX_IMAGE_MODEL_ID.to_string(),
        GatewayResponseAdapter::Passthrough { .. } => {
            stats_model_id_for_request_kind(&request.body, request_kind_from_adapter(adapter))
        }
    }
}

fn build_request_stats_context(
    request: &ParsedRequest,
    adapter: &GatewayResponseAdapter,
    api_key: &ResolvedLocalApiKey,
) -> RequestStatsContext {
    let request_kind = request_kind_from_adapter(adapter);
    RequestStatsContext {
        request_kind,
        model_id: stats_model_id_from_adapter(request, adapter),
        api_key_id: api_key.id.clone(),
        api_key_label: api_key.label.clone(),
    }
}

fn normalize_image_model_base(model: &str) -> String {
    let mut base_model = model.trim();
    if let Some(index) = base_model.rfind('/') {
        if index < base_model.len().saturating_sub(1) {
            base_model = base_model[index + 1..].trim();
        }
    }
    base_model.to_string()
}

fn is_gpt_image_generation_model(model: &str) -> bool {
    normalize_image_model_base(model)
        .to_ascii_lowercase()
        .starts_with("gpt-image-")
}

fn normalize_image_response_format(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .unwrap_or("b64_json")
        .to_ascii_lowercase()
}

fn validate_image_model(model: &str) -> Result<String, String> {
    let trimmed = model.trim();
    let base_model = normalize_image_model_base(trimmed);
    if base_model == CODEX_IMAGE_MODEL_ID {
        return Ok(CODEX_IMAGE_MODEL_ID.to_string());
    }

    Err(format!(
        "Model {} is not supported on {} or {}. Use {}.",
        if trimmed.is_empty() {
            "<empty>"
        } else {
            trimmed
        },
        IMAGES_GENERATIONS_PATH,
        IMAGES_EDITS_PATH,
        CODEX_IMAGE_MODEL_ID
    ))
}

fn json_string_field<'a>(object: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn insert_json_string_field(
    target: &mut Map<String, Value>,
    source: &Map<String, Value>,
    key: &str,
) {
    if let Some(value) = json_string_field(source, key) {
        target.insert(key.to_string(), Value::String(value.to_string()));
    }
}

fn insert_json_number_field(
    target: &mut Map<String, Value>,
    source: &Map<String, Value>,
    key: &str,
) {
    if let Some(value) = source.get(key).filter(|item| item.is_number()) {
        target.insert(key.to_string(), value.clone());
    }
}

fn build_image_generation_tool(
    source: &Map<String, Value>,
    action: &str,
    include_edit_fields: bool,
) -> Result<Value, String> {
    let image_model = json_string_field(source, "model").unwrap_or(CODEX_IMAGE_MODEL_ID);
    let canonical_model = validate_image_model(image_model)?;

    let mut tool = Map::new();
    tool.insert(
        "type".to_string(),
        Value::String("image_generation".to_string()),
    );
    tool.insert("action".to_string(), Value::String(action.to_string()));
    tool.insert("model".to_string(), Value::String(canonical_model));

    for key in [
        "size",
        "quality",
        "background",
        "output_format",
        "moderation",
    ] {
        insert_json_string_field(&mut tool, source, key);
    }
    if include_edit_fields {
        insert_json_string_field(&mut tool, source, "input_fidelity");
    }
    for key in ["output_compression", "partial_images"] {
        insert_json_number_field(&mut tool, source, key);
    }

    Ok(Value::Object(tool))
}

fn should_inject_image_generation_tool(model: &str) -> bool {
    let normalized = model.trim().to_ascii_lowercase();
    !normalized.is_empty()
        && !normalized.ends_with("spark")
        && !codex_protocol::codex_model_uses_responses_lite(&normalized)
}

fn is_image_gen_function_name(name: &str) -> bool {
    name.trim().eq_ignore_ascii_case("image_gen.imagegen")
}

fn tool_conflicts_with_hosted_image_generation(tool: &Value) -> bool {
    if tool
        .get("name")
        .and_then(Value::as_str)
        .is_some_and(is_image_gen_function_name)
        || tool
            .pointer("/function/name")
            .and_then(Value::as_str)
            .is_some_and(is_image_gen_function_name)
    {
        return true;
    }

    let image_namespace = ["name", "namespace"].iter().any(|key| {
        tool.get(*key)
            .and_then(Value::as_str)
            .is_some_and(|namespace| namespace.trim().eq_ignore_ascii_case("image_gen"))
    });
    image_namespace
        && tool
            .get("tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| {
                tools.iter().any(|child| {
                    child
                        .get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|name| name.trim().eq_ignore_ascii_case("imagegen"))
                        || child
                            .pointer("/function/name")
                            .and_then(Value::as_str)
                            .is_some_and(|name| name.trim().eq_ignore_ascii_case("imagegen"))
                })
            })
}

fn has_hosted_image_generation_tool_conflict(object: &Map<String, Value>) -> bool {
    let local_conflict = object
        .get("tools")
        .and_then(Value::as_array)
        .is_some_and(|tools| {
            tools
                .iter()
                .any(tool_conflicts_with_hosted_image_generation)
        });
    if local_conflict {
        return true;
    }

    let input_conflict = object
        .get("input")
        .and_then(Value::as_array)
        .is_some_and(|input| {
            input.iter().any(|item| {
                item.as_object().is_some_and(|item_object| {
                    item_object.get("type").and_then(Value::as_str) == Some("additional_tools")
                        && has_hosted_image_generation_tool_conflict(item_object)
                })
            })
        });
    input_conflict
        || object
            .get("response")
            .and_then(Value::as_object)
            .is_some_and(has_hosted_image_generation_tool_conflict)
}

fn ensure_image_generation_tool_in_object(object: &mut Map<String, Value>) -> bool {
    let model = object.get("model").and_then(Value::as_str).unwrap_or("");
    if !should_inject_image_generation_tool(model) {
        return false;
    }

    let tool = json!({
        "type": "image_generation",
        "output_format": "png",
    });

    match object.get_mut("tools") {
        Some(Value::Array(tools)) => {
            if tools
                .iter()
                .any(|item| item.get("type").and_then(Value::as_str) == Some("image_generation"))
            {
                false
            } else {
                tools.push(tool);
                true
            }
        }
        _ => {
            object.insert("tools".to_string(), Value::Array(vec![tool]));
            true
        }
    }
}

fn remove_hosted_image_generation_tool_from_object(object: &mut Map<String, Value>) -> bool {
    let mut changed = false;
    if let Some(Value::Array(tools)) = object.get_mut("tools") {
        let before = tools.len();
        tools.retain(|item| item.get("type").and_then(Value::as_str) != Some("image_generation"));
        changed |= tools.len() != before;
    }

    let remove_tool_choice = object
        .get("tool_choice")
        .map(|choice| {
            choice.as_str() == Some("image_generation")
                || choice.get("type").and_then(Value::as_str) == Some("image_generation")
                || (choice.get("type").and_then(Value::as_str) == Some("tool")
                    && choice.get("name").and_then(Value::as_str) == Some("image_generation"))
        })
        .unwrap_or(false);
    if remove_tool_choice {
        object.remove("tool_choice");
        changed = true;
    }

    changed
}

fn remove_hosted_image_generation_capabilities_from_object(
    object: &mut Map<String, Value>,
) -> bool {
    let mut changed = remove_hosted_image_generation_tool_from_object(object);

    if let Some(Value::Array(input)) = object.get_mut("input") {
        let before = input.len();
        input.retain_mut(|item| {
            let Some(item_object) = item.as_object_mut() else {
                return true;
            };
            if item_object.get("type").and_then(Value::as_str) != Some("additional_tools") {
                return true;
            }
            changed |= remove_hosted_image_generation_capabilities_from_object(item_object);
            !item_object
                .get("tools")
                .and_then(Value::as_array)
                .is_some_and(Vec::is_empty)
        });
        changed |= input.len() != before;
    }

    if let Some(Value::Object(response)) = object.get_mut("response") {
        changed |= remove_hosted_image_generation_capabilities_from_object(response);
    }

    changed
}

fn is_image_generation_capability_name(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "image_generation" | "image_gen" | "image_gen.imagegen"
    )
}

fn tool_declares_image_generation_capability(tool: &Value) -> bool {
    let Some(tool) = tool.as_object() else {
        return false;
    };
    if tool
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case("image_generation"))
    {
        return true;
    }
    let is_image_namespace = tool
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case("namespace"))
        && ["name", "namespace"].iter().any(|key| {
            tool.get(*key)
                .and_then(Value::as_str)
                .is_some_and(|value| value.trim().eq_ignore_ascii_case("image_gen"))
        });
    is_image_namespace
        || tool
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(is_image_gen_function_name)
        || tool
            .get("function")
            .and_then(Value::as_object)
            .and_then(|function| function.get("name"))
            .and_then(Value::as_str)
            .is_some_and(is_image_gen_function_name)
}

fn tool_choice_selects_image_generation(choice: &Value) -> bool {
    if choice
        .as_str()
        .is_some_and(is_image_generation_capability_name)
    {
        return true;
    }
    let Some(choice) = choice.as_object() else {
        return false;
    };
    if ["type", "name", "namespace"].iter().any(|key| {
        choice
            .get(*key)
            .and_then(Value::as_str)
            .is_some_and(is_image_generation_capability_name)
    }) {
        return true;
    }
    ["tool", "function"].iter().any(|key| {
        choice
            .get(*key)
            .is_some_and(tool_choice_selects_image_generation)
    }) || ["tools", "allowed_tools"].iter().any(|key| {
        choice
            .get(*key)
            .and_then(Value::as_array)
            .is_some_and(|items| items.iter().any(tool_choice_selects_image_generation))
    })
}
