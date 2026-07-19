use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::VecDeque;
use std::panic;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use url::Url;
use uuid::Uuid;

use crate::modules::{config, logger};

const SENTRY_CLIENT: &str = "cockpit-tools/1.0";
const FRONTEND_READY_TIMEOUT_MS: u64 = 15_000;
const SAME_EVENT_THROTTLE_MS: u128 = 60_000;
const MAX_EVENTS_PER_MINUTE: usize = 30;
const MAX_STRING_CHARS: usize = 2_000;
const MAX_CONTEXT_DEPTH: usize = 4;
const MAX_OBJECT_KEYS: usize = 50;
const MAX_ARRAY_ITEMS: usize = 20;

static FRONTEND_READY: AtomicBool = AtomicBool::new(false);
static WATCHDOG_STARTED: AtomicBool = AtomicBool::new(false);
static PANIC_HOOK_INSTALLED: AtomicBool = AtomicBool::new(false);
static LAST_FRONTEND_STAGE: LazyLock<Mutex<Option<String>>> = LazyLock::new(|| Mutex::new(None));
static RECENT_EVENTS: LazyLock<Mutex<VecDeque<RecentEvent>>> =
    LazyLock::new(|| Mutex::new(VecDeque::new()));

static EMAIL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b[a-z0-9._%+\-]+@[a-z0-9.\-]+\.[a-z]{2,}\b")
        .expect("email regex should be valid")
});
static BEARER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(bearer|token|api[_-]?key|secret|password)\s*[:=]\s*[^\s,;]+")
        .expect("bearer regex should be valid")
});
static LONG_TOKEN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[A-Za-z0-9_\-]{32,}\b").expect("token regex should be valid"));
static PHONE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:\+?\d[\d\s\-()]{8,}\d)\b").expect("phone regex should be valid")
});

fn profile_name() -> String {
    crate::modules::account::profile_name()
}

#[derive(Debug, Clone)]
struct RecentEvent {
    fingerprint: String,
    at: Instant,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsConfig {
    pub error_reporting_enabled: bool,
    pub error_reporting_debug: bool,
    pub endpoint_configured: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsClientEvent {
    pub level: Option<String>,
    pub message: String,
    pub name: Option<String>,
    pub stack: Option<String>,
    pub source: Option<String>,
    pub phase: Option<String>,
    pub platform_id: Option<String>,
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone)]
struct SentryDsn {
    public_key: String,
    host: String,
    project_id: String,
}

pub fn get_diagnostics_config() -> DiagnosticsConfig {
    let user_config = config::get_user_config();
    DiagnosticsConfig {
        error_reporting_enabled: user_config.diagnostics_error_reporting_enabled,
        error_reporting_debug: user_config.diagnostics_error_reporting_debug,
        endpoint_configured: current_dsn().is_some(),
    }
}

pub fn save_diagnostics_config(
    error_reporting_enabled: bool,
    error_reporting_debug: Option<bool>,
) -> Result<(), String> {
    config::patch_user_config(move |current| {
        current.diagnostics_error_reporting_enabled = error_reporting_enabled;
        if let Some(debug) = error_reporting_debug {
            current.diagnostics_error_reporting_debug = debug;
        }
        Ok(())
    })?;
    Ok(())
}

pub fn install_panic_hook() {
    if PANIC_HOOK_INSTALLED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }

    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let message = panic_message(info);
        let location = info
            .location()
            .map(|location| format!("{}:{}", location.file(), location.line()))
            .unwrap_or_else(|| "unknown".to_string());
        logger::log_error(&format!(
            "[Diagnostics] 捕获 Rust panic: location={}, error={}",
            sanitize_text(&location),
            sanitize_text(&message)
        ));
        let stack = std::backtrace::Backtrace::force_capture().to_string();
        capture_internal_event(
            "fatal",
            "Rust panic",
            Some("rust_panic"),
            Some("panic"),
            None,
            Some(json!({
                "location": location,
                "panicMessage": message,
                "backtrace": stack,
            })),
        );
        previous(info);
    }));
}

pub fn start_frontend_ready_watchdog() {
    if WATCHDOG_STARTED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }

    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(FRONTEND_READY_TIMEOUT_MS));
        if FRONTEND_READY.load(Ordering::SeqCst) {
            return;
        }
        let stage = LAST_FRONTEND_STAGE
            .lock()
            .ok()
            .and_then(|value| value.clone())
            .unwrap_or_else(|| "none".to_string());
        logger::log_warn(&format!(
            "[Diagnostics] 前端启动超时: timeoutMs={}, lastStage={}",
            FRONTEND_READY_TIMEOUT_MS, stage
        ));
        capture_internal_event(
            "warning",
            "Frontend did not report ready before timeout",
            Some("frontend_boot"),
            Some("ready_timeout"),
            None,
            Some(json!({
                "timeoutMs": FRONTEND_READY_TIMEOUT_MS,
                "lastStage": stage,
            })),
        );
    });
}

pub fn record_frontend_stage(stage: String, detail: Option<Value>) {
    let stage = sanitize_text(&stage);
    if let Ok(mut value) = LAST_FRONTEND_STAGE.lock() {
        *value = Some(stage.clone());
    }
    let detail = detail
        .map(|value| sanitize_value(value, 0, None))
        .unwrap_or(Value::Null);
    logger::log_info(&format!(
        "[Diagnostics] 前端启动阶段: stage={}, detail={}",
        stage, detail
    ));
}

pub fn mark_frontend_ready(stage: Option<String>) {
    let stage = stage
        .map(|value| sanitize_text(&value))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "ready".to_string());
    FRONTEND_READY.store(true, Ordering::SeqCst);
    if let Ok(mut value) = LAST_FRONTEND_STAGE.lock() {
        *value = Some(stage.clone());
    }
    logger::log_info(&format!("[Diagnostics] 前端已就绪: stage={}", stage));
}

pub fn capture_client_event(event: DiagnosticsClientEvent) {
    let level = normalize_level(event.level.as_deref());
    let message = sanitize_text(&event.message);
    let source = event.source.as_deref().map(sanitize_text);
    let phase = event.phase.as_deref().map(sanitize_text);
    let platform_id = event.platform_id.as_deref().map(sanitize_text);
    let log_text = format!(
        "[Diagnostics] 前端错误事件: level={}, source={}, phase={}, platform={}, message={}",
        level,
        source.as_deref().unwrap_or("-"),
        phase.as_deref().unwrap_or("-"),
        platform_id.as_deref().unwrap_or("-"),
        message
    );
    match level.as_str() {
        "fatal" | "error" => logger::log_error(&log_text),
        "warning" => logger::log_warn(&log_text),
        _ => logger::log_info(&log_text),
    }

    capture_event(
        level,
        message,
        event.name.as_deref().map(sanitize_text),
        event.stack.as_deref().map(sanitize_text),
        source,
        phase,
        platform_id,
        event.metadata.map(|value| sanitize_value(value, 0, None)),
    );
}

fn capture_internal_event(
    level: &str,
    message: &str,
    source: Option<&str>,
    phase: Option<&str>,
    platform_id: Option<&str>,
    metadata: Option<Value>,
) {
    capture_event(
        normalize_level(Some(level)),
        sanitize_text(message),
        None,
        None,
        source.map(sanitize_text),
        phase.map(sanitize_text),
        platform_id.map(sanitize_text),
        metadata.map(|value| sanitize_value(value, 0, None)),
    );
}

#[allow(clippy::too_many_arguments)]
fn capture_event(
    level: String,
    message: String,
    name: Option<String>,
    stack: Option<String>,
    source: Option<String>,
    phase: Option<String>,
    platform_id: Option<String>,
    metadata: Option<Value>,
) {
    if !should_send(&level, &message, source.as_deref(), phase.as_deref()) {
        return;
    }

    let Some(dsn) = current_dsn().and_then(|value| parse_dsn(&value)) else {
        if get_diagnostics_config().error_reporting_debug {
            logger::log_info("[Diagnostics] Sentry DSN 未配置，跳过错误上报");
        }
        return;
    };

    let event = build_sentry_event(
        &level,
        &message,
        name.as_deref(),
        stack.as_deref(),
        source.as_deref(),
        phase.as_deref(),
        platform_id.as_deref(),
        metadata,
    );

    std::thread::spawn(move || {
        if let Err(error) = send_sentry_event(&dsn, &event) {
            if get_diagnostics_config().error_reporting_debug {
                logger::log_warn(&format!("[Diagnostics] Sentry 上报失败: {}", error));
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn build_sentry_event(
    level: &str,
    message: &str,
    name: Option<&str>,
    stack: Option<&str>,
    source: Option<&str>,
    phase: Option<&str>,
    platform_id: Option<&str>,
    metadata: Option<Value>,
) -> Value {
    let mut tags = Map::new();
    tags.insert("os".to_string(), json!(std::env::consts::OS));
    tags.insert("arch".to_string(), json!(std::env::consts::ARCH));
    tags.insert(
        "profile".to_string(),
        json!(profile_name().to_ascii_lowercase()),
    );
    tags.insert(
        "build_mode".to_string(),
        json!(if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }),
    );
    tags.insert(
        "error_category".to_string(),
        json!(classify_error(message, stack)),
    );
    if let Some(value) = source {
        tags.insert("source".to_string(), json!(value));
    }
    if let Some(value) = phase {
        tags.insert("phase".to_string(), json!(value));
    }
    if let Some(value) = platform_id {
        tags.insert("platform_id".to_string(), json!(value));
    }

    let mut contexts = Map::new();
    contexts.insert(
        "runtime".to_string(),
        json!({
            "appName": "Cockpit Tools",
            "appVersion": env!("CARGO_PKG_VERSION"),
            "profile": profile_name(),
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "pid": std::process::id(),
            "debugAssertions": cfg!(debug_assertions),
        }),
    );
    if let Some(value) = metadata {
        contexts.insert("custom".to_string(), value);
    }
    if let Some(value) = stack {
        contexts.insert("stack".to_string(), json!({ "raw": value }));
    }

    let mut event = json!({
        "event_id": Uuid::new_v4().simple().to_string(),
        "timestamp": Utc::now().to_rfc3339(),
        "platform": "rust",
        "level": level,
        "release": format!("cockpit-tools@{}", env!("CARGO_PKG_VERSION")),
        "environment": if cfg!(debug_assertions) { "development" } else { "production" },
        "message": message,
        "tags": tags,
        "contexts": contexts,
    });

    if level == "error" || level == "fatal" {
        event["exception"] = json!({
            "values": [{
                "type": name.unwrap_or("Error"),
                "value": message,
                "mechanism": {
                    "type": source.unwrap_or("diagnostics"),
                    "handled": true
                }
            }]
        });
    }

    event
}

fn send_sentry_event(dsn: &SentryDsn, event: &Value) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|error| error.to_string())?;
    let url = format!("https://{}/api/{}/store/", dsn.host, dsn.project_id);
    let response = client
        .post(url)
        .header("content-type", "application/json")
        .header(
            "x-sentry-auth",
            format!(
                "Sentry sentry_version=7, sentry_key={}, sentry_client={}",
                dsn.public_key, SENTRY_CLIENT
            ),
        )
        .json(event)
        .send()
        .map_err(|error| error.to_string())?;

    if response.status().is_success() {
        return Ok(());
    }
    Err(format!("HTTP {}", response.status()))
}

fn current_dsn() -> Option<String> {
    let runtime = std::env::var("COCKPIT_SENTRY_DSN")
        .ok()
        .or_else(|| std::env::var("SENTRY_DSN").ok())
        .or_else(|| option_env!("COCKPIT_SENTRY_DSN").map(ToString::to_string))
        .or_else(|| option_env!("SENTRY_DSN").map(ToString::to_string))
        .unwrap_or_default();
    let trimmed = runtime.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("off")
        || trimmed.eq_ignore_ascii_case("disabled")
    {
        return None;
    }
    Some(trimmed.to_string())
}

fn parse_dsn(raw: &str) -> Option<SentryDsn> {
    let url = Url::parse(raw).ok()?;
    let public_key = url.username().trim().to_string();
    let host = url.host_str()?.trim().to_string();
    let project_id = url.path().trim_matches('/').to_string();
    if public_key.is_empty() || host.is_empty() || project_id.is_empty() {
        return None;
    }
    Some(SentryDsn {
        public_key,
        host,
        project_id,
    })
}

fn should_send(level: &str, message: &str, source: Option<&str>, phase: Option<&str>) -> bool {
    let env_disabled = std::env::var("COCKPIT_DISABLE_ERROR_REPORTING")
        .ok()
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            value == "1" || value == "true" || value == "yes"
        })
        .unwrap_or(false);
    if env_disabled {
        return false;
    }
    if !get_diagnostics_config().error_reporting_enabled {
        return false;
    }

    let fingerprint = format!(
        "{}|{}|{}|{}",
        level,
        source.unwrap_or("-"),
        phase.unwrap_or("-"),
        message
    );
    let now = Instant::now();
    let Ok(mut recent) = RECENT_EVENTS.lock() else {
        return true;
    };
    while recent
        .front()
        .map(|item| now.duration_since(item.at).as_millis() > 60_000)
        .unwrap_or(false)
    {
        recent.pop_front();
    }
    if recent.len() >= MAX_EVENTS_PER_MINUTE {
        return false;
    }
    if recent.iter().any(|item| {
        item.fingerprint == fingerprint
            && now.duration_since(item.at).as_millis() < SAME_EVENT_THROTTLE_MS
    }) {
        return false;
    }
    recent.push_back(RecentEvent {
        fingerprint,
        at: now,
    });
    true
}

fn normalize_level(level: Option<&str>) -> String {
    match level
        .unwrap_or("error")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "fatal" => "fatal".to_string(),
        "warning" | "warn" => "warning".to_string(),
        "info" => "info".to_string(),
        _ => "error".to_string(),
    }
}

fn classify_error(message: &str, stack: Option<&str>) -> String {
    let text = format!("{} {}", message, stack.unwrap_or("")).to_ascii_lowercase();
    if text.contains("failed to fetch dynamically imported module")
        || text.contains("chunkloaderror")
        || text.contains("importing a module script failed")
    {
        return "chunk_load".to_string();
    }
    if text.contains("unsupported platform ui protocol") || text.contains("remote ui") {
        return "platform_remote_ui".to_string();
    }
    if text.contains("connection refused") || text.contains("econnrefused") {
        return "connection_refused".to_string();
    }
    if text.contains("timeout") || text.contains("timed out") {
        return "timeout".to_string();
    }
    if text.contains("permission") || text.contains("denied") || text.contains("eacces") {
        return "permission_denied".to_string();
    }
    if text.contains("undefined")
        || text.contains("null")
        || text.contains("cannot read properties")
        || text.contains("cannot read property")
    {
        return "render_null_reference".to_string();
    }
    "unknown".to_string()
}

fn sanitize_value(value: Value, depth: usize, key: Option<&str>) -> Value {
    if key.map(is_sensitive_key).unwrap_or(false) {
        return Value::String("[redacted]".to_string());
    }
    if depth >= MAX_CONTEXT_DEPTH {
        return match value {
            Value::Array(items) => Value::String(format!("array({})", items.len())),
            Value::Object(items) => Value::String(format!("object({})", items.len())),
            Value::String(text) => Value::String(sanitize_text(&text)),
            other => other,
        };
    }
    match value {
        Value::String(text) => Value::String(sanitize_text(&text)),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .take(MAX_ARRAY_ITEMS)
                .map(|item| sanitize_value(item, depth + 1, None))
                .collect(),
        ),
        Value::Object(items) => {
            let mut next = Map::new();
            for (key, value) in items.into_iter().take(MAX_OBJECT_KEYS) {
                next.insert(key.clone(), sanitize_value(value, depth + 1, Some(&key)));
            }
            Value::Object(next)
        }
        other => other,
    }
}

fn sanitize_text(value: &str) -> String {
    let mut text = value.trim().to_string();
    if text.chars().count() > MAX_STRING_CHARS {
        text = text.chars().take(MAX_STRING_CHARS).collect::<String>();
        text.push_str("...[truncated]");
    }
    text = EMAIL_RE.replace_all(&text, "[email]").to_string();
    text = BEARER_RE.replace_all(&text, "$1=[redacted]").to_string();
    text = LONG_TOKEN_RE.replace_all(&text, "[token]").to_string();
    text = PHONE_RE.replace_all(&text, "[phone]").to_string();
    if let Some(home) = dirs::home_dir().and_then(|path| path.to_str().map(ToString::to_string)) {
        if !home.is_empty() {
            text = text.replace(&home, "~");
        }
    }
    text
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("password")
        || key.contains("token")
        || key.contains("secret")
        || key.contains("authorization")
        || key.contains("api_key")
        || key.contains("apikey")
        || key.contains("two_factor")
        || key.contains("2fa")
        || key.contains("phone")
        || key.contains("email")
}

fn panic_message(info: &panic::PanicHookInfo<'_>) -> String {
    if let Some(value) = info.payload().downcast_ref::<&str>() {
        return (*value).to_string();
    }
    if let Some(value) = info.payload().downcast_ref::<String>() {
        return value.clone();
    }
    "panic payload is not a string".to_string()
}

#[cfg(test)]
mod tests {
    use super::{parse_dsn, sanitize_text, sanitize_value};
    use serde_json::json;

    #[test]
    fn parse_sentry_dsn_extracts_store_parts() {
        let dsn = parse_dsn("https://public@example.sentry.io/123").expect("dsn should parse");

        assert_eq!(dsn.public_key, "public");
        assert_eq!(dsn.host, "example.sentry.io");
        assert_eq!(dsn.project_id, "123");
    }

    #[test]
    fn sanitize_text_redacts_common_sensitive_values() {
        let text = sanitize_text(
            "email a@example.com token=abcdef1234567890abcdef1234567890 /Users/demo/path",
        );

        assert!(!text.contains("a@example.com"));
        assert!(!text.contains("abcdef1234567890abcdef1234567890"));
    }

    #[test]
    fn sanitize_value_redacts_sensitive_keys() {
        let value = sanitize_value(
            json!({
                "password": "plain",
                "nested": {
                    "twoFactorSecret": "JBSWY3DPEHPK3PXP"
                }
            }),
            0,
            None,
        );

        assert_eq!(value["password"], "[redacted]");
        assert_eq!(value["nested"]["twoFactorSecret"], "[redacted]");
    }
}
