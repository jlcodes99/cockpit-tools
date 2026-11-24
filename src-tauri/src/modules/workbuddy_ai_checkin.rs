//! WorkBuddy AI (international `www.workbuddy.ai`) daily active check-in.
//!
//! Unlike domestic WorkBuddy (`copilot.tencent.com`), the international edition does not expose
//! `/v2/billing/meter/daily-checkin`. Instead, completing 1 conversation turn and reporting
//! desktop conversation lifecycle events lights up the daily growth heatmap
//! (`GET /activity/growth/heatmap`: `is_active = true`, `score > 0`), which qualifies for
//! next-day daily activity bonus settlement and advances the account's active streak.
//!
//! Safety invariants:
//! - Strictly fail-closed on local model-catalog check: before sending any chat request, we
//!   query `GET /v3/config` live, strictly validate raw response bytes as UTF-8, validate the
//!   raw JSON envelope (`code`, no duplicate keys), and validate raw `credits` tokens lexically
//!   as exact zero (`0`, `0.00`, `"x0.00"`), rejecting invalid UTF-8, scientific notation
//!   underflow (`1e-400`), negative zero (`-0.0`), object-spoofed codes, mutated/whitespace
//!   model IDs, duplicate model IDs, and non-200 responses.
//!   Note: `/v3/config` and `/v2/chat/completions` are separate HTTP requests; client-side checks
//!   cannot enforce server-side atomic pricing or prevent server-side fallback.
//! - Write-ahead at-most-once chat per `(account_id, activity_date)`: serialized under a
//!   per-account async lock, pre-checked via `GET /activity/growth/heatmap` and local persisted
//!   date records (`pendingActivityDate` or confirmed `activityDate`), and guarded by persisting
//!   `pendingActivityDate` BEFORE sending `POST /v2/chat/completions` (aborting with zero chat
//!   POSTs if persistence fails). The reservation is released only on provably pre-execution
//!   rejections (TCP connect failure or HTTP 400/401/403). Any HTTP 200 stream error (including
//!   first-event stream 401), 5xx/502/timeout, early exit, report timeout, UI re-click, forced
//!   run, restart, or lagging post-confirmation heatmap keeps the day read-only with no replay.
//! - Telemetry disclosure: `/v2/report` sends host OS/arch/CPU/memory/timezone metadata from
//!   `sysinfo` / `iana-time-zone`, account `userId`, and deterministic UID-derived
//!   `machineId`/`sessionId` identifiers as expected by the desktop event endpoint.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::Duration;

use chrono::{Local, NaiveDate};
use reqwest::{Client, RequestBuilder, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sysinfo::System;

use crate::models::workbuddy::WorkbuddyAccount;
use crate::modules::{logger, workbuddy_ai_account, workbuddy_ai_auto_checkin, workbuddy_ai_oauth};

pub const DEFAULT_ORIGIN: &str = "https://www.workbuddy.ai";
pub const DEFAULT_DOMAIN: &str = "www.workbuddy.ai";

const CLIENT_VERSION: &str = "5.11.4";
const CLIENT_COMMIT: &str = "0026dff06ce8414140e6f2fa731049855c3ecc6f";
const CLIENT_RELEASE_DATE_MS: i64 = 1_794_817_062_903;
const PREFERRED_FREE_MODEL_ID: &str = "hy4flash-free";
const REPORT_TRANSIENT_RETRY_DELAYS: &[Duration] =
    &[Duration::from_secs(1), Duration::from_secs(2)];
const POST_REPORT_VERIFY_DELAYS: &[Duration] = &[
    Duration::from_secs(2),
    Duration::from_secs(3),
    Duration::from_secs(4),
];

const GROWTH_USER_AGENT: &str = "workbuddy-ai/5.11.4 workbuddy-ai/5.11.4 CLI/2.176.0";
const CONFIG_USER_AGENT: &str = "WorkBuddy/5.11.4 CLI/2.176.0";
const CHAT_USER_AGENT: &str = "WorkBuddy/5.11.4 workbuddy-ai/5.11.4 CLI/2.176.0";

type SharedRenewResult = Result<WorkbuddyAccount, String>;

static ACCOUNT_CHECKIN_LOCKS: LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static IN_FLIGHT_RENEWALS: LazyLock<
    Mutex<HashMap<String, tokio::sync::watch::Receiver<Option<SharedRenewResult>>>>,
> = LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbuddyAiCheckinStatus {
    pub today_checked_in: bool,
    pub active: bool,
    pub streak_days: i32,
    pub daily_credit: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub today_credit: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkin_dates: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub today_date: Option<String>,
    pub today_score: i64,
    pub month_total_days: i32,
    #[serde(default)]
    pub pending_confirmation: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbuddyAiCheckinResult {
    pub success: bool,
    #[serde(default)]
    pub already_claimed: bool,
    #[serde(default)]
    pub inactive: bool,
    #[serde(default)]
    pub pending_confirmation: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub streak_days: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score_before: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score_after: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
}

pub(crate) enum GuardedCheckinOutcome {
    Skipped,
    Executed(WorkbuddyAiCheckinResult),
}

#[derive(Debug, Clone)]
struct HostTelemetry {
    os: &'static str,
    arch: &'static str,
    os_version: String,
    cpu_model: String,
    cpu_cores: usize,
    memory_gb: u64,
    timezone: String,
}

#[derive(Debug, Clone)]
struct HeatmapSummary {
    today_date: String,
    is_active: bool,
    score: i64,
    status_text: Option<String>,
    active_dates: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct StreakSummary {
    streak_days: i32,
    month_total_days: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FreeModel {
    id: String,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedSseMeta {
    request_id: String,
    response_model: String,
    finish_reason: String,
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
}

#[derive(Debug, Clone)]
struct ChatTurnArtifacts {
    free_model: FreeModel,
    cid: String,
    msg_id: String,
    trace_id: String,
    sse_meta: ParsedSseMeta,
}

#[derive(Debug, Clone)]
struct PreparedReport {
    request_id: String,
    payload: Value,
}

#[derive(Debug)]
enum ApiError {
    Unauthorized(String),
    Transient(String),
    Skipped,
    Other(String),
}

impl ApiError {
    fn is_unauthorized(&self) -> bool {
        matches!(self, Self::Unauthorized(_))
    }

    fn is_transient(&self) -> bool {
        matches!(self, Self::Transient(_))
    }

    fn into_message(self) -> String {
        match self {
            Self::Unauthorized(msg) | Self::Transient(msg) | Self::Other(msg) => msg,
            Self::Skipped => "WORKBUDDY_AI_CHECKIN_SKIPPED".to_string(),
        }
    }
}

struct ActiveSession<R> {
    account: WorkbuddyAccount,
    renewed_once: bool,
    renewer: R,
}

impl<R, Fut> ActiveSession<R>
where
    R: FnMut(WorkbuddyAccount) -> Fut,
    Fut: Future<Output = Result<WorkbuddyAccount, String>>,
{
    fn new(account: WorkbuddyAccount, renewer: R) -> Self {
        Self {
            account,
            renewed_once: false,
            renewer,
        }
    }

    async fn call<T, F, OpFut>(&mut self, mut op: F) -> Result<T, ApiError>
    where
        F: FnMut(&WorkbuddyAccount) -> OpFut,
        OpFut: Future<Output = Result<T, ApiError>>,
    {
        match op(&self.account).await {
            Ok(value) => Ok(value),
            Err(error) if error.is_unauthorized() && !self.renewed_once => {
                self.renewed_once = true;
                let refreshed = (self.renewer)(self.account.clone())
                    .await
                    .map_err(ApiError::Other)?;
                self.account = refreshed;
                op(&self.account).await
            }
            Err(error) => Err(error),
        }
    }
}

fn account_lock(
    map: &LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    account_id: &str,
) -> Result<Arc<tokio::sync::Mutex<()>>, String> {
    let mut guard = map.lock().map_err(|error| error.to_string())?;
    Ok(guard
        .entry(account_id.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone())
}

pub(crate) fn is_permanent_error(error: &str) -> bool {
    let trimmed = error.trim();
    let lower = trimmed.to_ascii_lowercase();
    matches!(
        trimmed,
        "WORKBUDDY_AI_TOKEN_EXPIRED"
            | "WORKBUDDY_AI_MISSING_SESSION"
            | "WORKBUDDY_AI_MISSING_ACCOUNT_ID"
            | "WORKBUDDY_AI_ACCOUNT_NOT_FOUND"
            | "WORKBUDDY_AI_INVALID_ID"
    ) || trimmed.contains("WORKBUDDY_AI_REFRESH_REJECTED")
        || trimmed.contains("HTTP 401")
        || trimmed.contains("HTTP 403")
        || trimmed.contains("WorkBuddy AI HTTP 400")
        || trimmed.contains("code 401")
        || trimmed.contains("code 10401")
        || trimmed.contains("code 11401")
        || lower.contains("invalid_grant")
        || (lower.contains("token/refresh")
            && (lower.contains("400") || lower.contains("invalid") || lower.contains("expired")))
}

fn append_warning(target: &mut Option<String>, warning: String) {
    match target {
        Some(existing) if !existing.is_empty() => {
            existing.push_str("; ");
            existing.push_str(&warning);
        }
        _ => *target = Some(warning),
    }
}

fn host_telemetry() -> &'static HostTelemetry {
    static TELEMETRY: OnceLock<HostTelemetry> = OnceLock::new();
    TELEMETRY.get_or_init(|| {
        let mut sys = System::new();
        sys.refresh_cpu_all();
        sys.refresh_memory();

        let os = if cfg!(target_os = "macos") {
            "darwin"
        } else if cfg!(target_os = "linux") {
            "linux"
        } else {
            "win32"
        };
        let arch = if cfg!(target_arch = "aarch64") {
            "arm64"
        } else {
            "x64"
        };
        let os_version = System::os_version()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "unknown".to_string());
        let cpu_model = sys
            .cpus()
            .first()
            .map(|cpu| cpu.brand().trim().to_string())
            .filter(|brand| !brand.is_empty())
            .unwrap_or_else(|| "Unknown CPU".to_string());
        let cpu_cores = sys.cpus().len().max(1);
        let memory_gb = (sys.total_memory() / (1024 * 1024 * 1024)).max(1);
        let timezone = iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".to_string());

        HostTelemetry {
            os,
            arch,
            os_version,
            cpu_model,
            cpu_cores,
            memory_gb,
            timezone,
        }
    })
}

fn derived_hex36(salt: &str, uid: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{salt}:{uid}").as_bytes());
    let hex = format!("{:x}", hasher.finalize());
    hex.chars().take(36).collect()
}

fn build_client() -> Result<Client, String> {
    Client::builder()
        .connect_timeout(Duration::from_secs(6))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| error.to_string())
}

fn account_domain(account: &WorkbuddyAccount) -> &str {
    account
        .domain
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            account
                .auth_raw
                .as_ref()
                .and_then(|raw| raw["auth"]["domain"].as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or(DEFAULT_DOMAIN)
}

fn account_uid(account: &WorkbuddyAccount) -> Option<&str> {
    account
        .uid
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            account
                .auth_raw
                .as_ref()
                .and_then(|raw| raw["account"]["uid"].as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
}

fn is_enterprise_account(account: &WorkbuddyAccount) -> bool {
    account
        .enterprise_id
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        || account
            .auth_raw
            .as_ref()
            .and_then(|raw| raw["account"]["enterpriseId"].as_str())
            .is_some_and(|value| !value.trim().is_empty())
}

fn has_refresh_token(account: &WorkbuddyAccount) -> bool {
    account
        .auth_raw
        .as_ref()
        .and_then(|raw| raw["auth"]["refreshToken"].as_str())
        .is_some_and(|token| !token.trim().is_empty())
}

fn apply_common_headers(
    request: RequestBuilder,
    access_token: &str,
    uid: Option<&str>,
    domain: &str,
    user_agent: &str,
) -> RequestBuilder {
    let mut req = request
        .bearer_auth(access_token)
        .header("Accept", "application/json, text/plain, */*")
        .header("User-Agent", user_agent)
        .header("X-Domain", domain)
        .header("X-Product", "SaaS");
    if let Some(uid) = uid.filter(|value| !value.trim().is_empty()) {
        req = req.header("X-User-Id", uid);
    }
    req
}

/// Validates a raw JSON integer literal token (e.g. `0`, `200`, `-1`, `10401`).
/// Rejects strings, floats, scientific notation, and object-spoofed serde numbers.
fn parse_raw_json_integer_token(raw_token: &str) -> Option<i64> {
    let trimmed = raw_token.trim();
    if trimmed.is_empty() {
        return None;
    }
    let digits = trimmed.strip_prefix('-').unwrap_or(trimmed);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if digits.len() > 1 && digits.starts_with('0') {
        return None;
    }
    trimmed.parse::<i64>().ok()
}

/// Validates the raw top-level JSON envelope `code` before `serde_json::Value` can collapse
/// duplicate keys (including escaped spellings like `\u0063ode`) or accept spoofed number objects.
fn validate_raw_envelope_code(raw_json: &str, body: &Value, label: &str) -> Result<(), ApiError> {
    let raw_code = find_object_field_raw(raw_json, "code")
        .map_err(|()| ApiError::Other(format!("{label}: malformed or duplicate top-level code")))?;
    if let Some(raw_token) = raw_code {
        let Some(code) = parse_raw_json_integer_token(raw_token) else {
            return Err(ApiError::Other(format!(
                "{label}: non-integer response code"
            )));
        };
        if matches!(code, 401 | 10401 | 11401) {
            return Err(ApiError::Unauthorized(format!("{label}: code {code}")));
        }
        if !matches!(code, 0 | 200) {
            let msg = body
                .get("message")
                .or_else(|| body.get("msg"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if msg.is_empty() {
                return Err(ApiError::Other(format!("{label}: code {code}")));
            }
            return Err(ApiError::Other(format!("{label}: code {code}: {msg}")));
        }
    }
    Ok(())
}

fn is_transient_reqwest_error(error: &reqwest::Error) -> bool {
    error.is_timeout()
        || error.is_connect()
        || (error.status().is_none() && (error.is_request() || error.is_body()))
}

fn is_transient_http_status(status: StatusCode) -> bool {
    matches!(status.as_u16(), 408 | 429 | 500 | 502 | 503 | 504)
}

fn map_reqwest_error(label: &str, error: reqwest::Error) -> ApiError {
    let sanitized = error.without_url();
    let msg = format!("{label}: {sanitized}");
    if is_transient_reqwest_error(&sanitized) {
        ApiError::Transient(msg)
    } else {
        ApiError::Other(msg)
    }
}

async fn send_json_raw(request: RequestBuilder, label: &str) -> Result<(String, Value), ApiError> {
    let response = request
        .send()
        .await
        .map_err(|error| map_reqwest_error(label, error))?;
    let status = response.status();
    if status == StatusCode::UNAUTHORIZED {
        return Err(ApiError::Unauthorized(format!("{label}: HTTP 401")));
    }
    if is_transient_http_status(status) {
        return Err(ApiError::Transient(format!(
            "{label}: HTTP {}",
            status.as_u16()
        )));
    }
    if status != StatusCode::OK {
        return Err(ApiError::Other(format!(
            "{label}: HTTP {}",
            status.as_u16()
        )));
    }
    let body_bytes = response
        .bytes()
        .await
        .map_err(|error| map_reqwest_error(label, error))?;
    let body_text = std::str::from_utf8(&body_bytes)
        .map_err(|_| ApiError::Other(format!("{label}: invalid UTF-8 response")))?
        .to_string();
    let body: Value = serde_json::from_str(&body_text)
        .map_err(|error| ApiError::Other(format!("{label}: invalid JSON ({error})")))?;
    if !body.is_object() {
        return Err(ApiError::Other(format!(
            "{label}: expected top-level JSON object"
        )));
    }
    validate_raw_envelope_code(&body_text, &body, label)?;
    Ok((body_text, body))
}

async fn send_json(request: RequestBuilder, label: &str) -> Result<Value, ApiError> {
    send_json_raw(request, label).await.map(|(_, body)| body)
}

fn parse_iso_date(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.len() != 10 {
        return None;
    }
    NaiveDate::parse_from_str(trimmed, "%Y-%m-%d")
        .ok()
        .map(|d| d.format("%Y-%m-%d").to_string())
}

async fn fetch_heatmap(
    client: &Client,
    origin: &str,
    access_token: &str,
    uid: Option<&str>,
    domain: &str,
) -> Result<HeatmapSummary, ApiError> {
    let url = format!("{origin}/activity/growth/heatmap");
    let req = apply_common_headers(
        client.get(&url).header("Content-Type", "application/json"),
        access_token,
        uid,
        domain,
        GROWTH_USER_AGENT,
    );
    let body = send_json(req, "growth/heatmap").await?;
    let data = body
        .get("data")
        .filter(|value| value.is_object())
        .ok_or_else(|| ApiError::Other("growth/heatmap: missing data".to_string()))?;
    let today = data
        .get("today")
        .filter(|value| value.is_object())
        .ok_or_else(|| ApiError::Other("growth/heatmap: missing today".to_string()))?;
    let today_date = today
        .get("date")
        .and_then(Value::as_str)
        .and_then(parse_iso_date)
        .ok_or_else(|| {
            ApiError::Other("growth/heatmap: missing or invalid today.date".to_string())
        })?;
    let is_active = today
        .get("is_active")
        .and_then(Value::as_bool)
        .ok_or_else(|| {
            ApiError::Other("growth/heatmap: missing or invalid today.is_active".to_string())
        })?;
    let score = today
        .get("score")
        .and_then(Value::as_i64)
        .filter(|&value| value >= 0)
        .ok_or_else(|| {
            ApiError::Other("growth/heatmap: missing or invalid today.score".to_string())
        })?;
    let status_text = today
        .get("status_text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);

    let mut active_dates: Vec<String> = data
        .get("cells")
        .and_then(Value::as_array)
        .map(|cells| {
            cells
                .iter()
                .filter(|cell| cell["score"].as_i64().unwrap_or(0) > 0)
                .filter_map(|cell| cell["date"].as_str())
                .filter_map(parse_iso_date)
                .collect()
        })
        .unwrap_or_default();
    active_dates.sort();
    active_dates.reverse();

    Ok(HeatmapSummary {
        today_date,
        is_active,
        score,
        status_text,
        active_dates,
    })
}

fn non_negative_i32(value: Option<&Value>) -> Option<i32> {
    value
        .and_then(Value::as_i64)
        .filter(|&v| v >= 0)
        .and_then(|v| i32::try_from(v).ok())
}

async fn fetch_streak(
    client: &Client,
    origin: &str,
    access_token: &str,
    uid: Option<&str>,
    domain: &str,
) -> Result<StreakSummary, ApiError> {
    let url = format!("{origin}/activity/growth/streak");
    let req = apply_common_headers(
        client.get(&url).header("Content-Type", "application/json"),
        access_token,
        uid,
        domain,
        GROWTH_USER_AGENT,
    );
    let body = send_json(req, "growth/streak").await?;
    let data = body
        .get("data")
        .filter(|value| value.is_object())
        .ok_or_else(|| ApiError::Other("growth/streak: missing data".to_string()))?;
    let streak = data
        .get("streak")
        .filter(|value| value.is_object())
        .ok_or_else(|| ApiError::Other("growth/streak: missing streak".to_string()))?;
    let streak_days = non_negative_i32(streak.get("days"))
        .ok_or_else(|| ApiError::Other("growth/streak: invalid streak.days".to_string()))?;
    let month_total_days = non_negative_i32(streak.get("month_total_days")).unwrap_or(0);

    Ok(StreakSummary {
        streak_days,
        month_total_days,
    })
}

/// Pure lexical check for an exact unsigned zero decimal string (`0`, `0.0`, `0.00`, ...).
/// Rejects signs (`+`/`-`), scientific notation (`1e-400`, `0e0`), whitespace, or non-zero digits.
fn is_zero_decimal_str(raw: &str) -> bool {
    if raw.is_empty() {
        return false;
    }
    let mut parts = raw.split('.');
    let Some(int_part) = parts.next() else {
        return false;
    };
    if int_part.is_empty() || !int_part.bytes().all(|b| b == b'0') {
        return false;
    }
    match parts.next() {
        None => true,
        Some(frac_part) => {
            parts.next().is_none() && !frac_part.is_empty() && frac_part.bytes().all(|b| b == b'0')
        }
    }
}

fn is_zero_credit_string(raw: &str) -> bool {
    let trimmed = raw.trim();
    let stripped = trimmed
        .strip_prefix('x')
        .or_else(|| trimmed.strip_prefix('X'))
        .unwrap_or(trimmed)
        .trim();
    is_zero_decimal_str(stripped)
}

/// Validates a raw JSON token slice for the `credits` field before `serde_json` can underflow
/// scientific notation numbers such as `1e-400` into `0.0_f64`.
fn is_zero_credit_raw_token(raw_token: &str) -> bool {
    let trimmed = raw_token.trim();
    if trimmed.starts_with('"') {
        serde_json::from_str::<String>(trimmed)
            .ok()
            .is_some_and(|decoded| is_zero_credit_string(&decoded))
    } else {
        is_zero_decimal_str(trimmed)
    }
}

struct RawJsonCursor<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> RawJsonCursor<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src,
            bytes: src.as_bytes(),
            pos: 0,
        }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b' ' | b'\t' | b'\r' | b'\n' => self.pos += 1,
                _ => break,
            }
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_ws();
        self.bytes.get(self.pos).copied()
    }

    fn expect_byte(&mut self, expected: u8) -> Result<(), ()> {
        self.skip_ws();
        if self.bytes.get(self.pos) == Some(&expected) {
            self.pos += 1;
            Ok(())
        } else {
            Err(())
        }
    }

    fn scan_string_slice(&mut self) -> Result<&'a str, ()> {
        self.skip_ws();
        let start = self.pos;
        if self.bytes.get(self.pos) != Some(&b'"') {
            return Err(());
        }
        self.pos += 1;
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b'\\' => {
                    self.pos += 2;
                    if self.pos > self.bytes.len() {
                        return Err(());
                    }
                }
                b'"' => {
                    self.pos += 1;
                    return Ok(&self.src[start..self.pos]);
                }
                _ => self.pos += 1,
            }
        }
        Err(())
    }

    fn scan_value_slice(&mut self) -> Result<&'a str, ()> {
        self.skip_ws();
        let start = self.pos;
        let first = *self.bytes.get(self.pos).ok_or(())?;
        match first {
            b'"' => self.scan_string_slice(),
            b'{' => {
                self.pos += 1;
                if self.peek() == Some(b'}') {
                    self.pos += 1;
                    return Ok(&self.src[start..self.pos]);
                }
                loop {
                    self.scan_string_slice()?;
                    self.expect_byte(b':')?;
                    self.scan_value_slice()?;
                    match self.peek() {
                        Some(b',') => self.pos += 1,
                        Some(b'}') => {
                            self.pos += 1;
                            return Ok(&self.src[start..self.pos]);
                        }
                        _ => return Err(()),
                    }
                }
            }
            b'[' => {
                self.pos += 1;
                if self.peek() == Some(b']') {
                    self.pos += 1;
                    return Ok(&self.src[start..self.pos]);
                }
                loop {
                    self.scan_value_slice()?;
                    match self.peek() {
                        Some(b',') => self.pos += 1,
                        Some(b']') => {
                            self.pos += 1;
                            return Ok(&self.src[start..self.pos]);
                        }
                        _ => return Err(()),
                    }
                }
            }
            _ => {
                while self.pos < self.bytes.len() {
                    match self.bytes[self.pos] {
                        b' ' | b'\t' | b'\r' | b'\n' | b',' | b']' | b'}' => break,
                        _ => self.pos += 1,
                    }
                }
                if self.pos == start {
                    return Err(());
                }
                Ok(&self.src[start..self.pos])
            }
        }
    }
}

fn find_object_field_raw<'a>(obj_json: &'a str, target_key: &str) -> Result<Option<&'a str>, ()> {
    let mut cursor = RawJsonCursor::new(obj_json);
    cursor.expect_byte(b'{')?;
    if cursor.peek() == Some(b'}') {
        return Ok(None);
    }
    let mut found = None;
    loop {
        let key_slice = cursor.scan_string_slice()?;
        let key: String = serde_json::from_str(key_slice).map_err(|_| ())?;
        cursor.expect_byte(b':')?;
        let val_slice = cursor.scan_value_slice()?;
        if key == target_key {
            if found.is_some() {
                // Reject duplicate keys (including escaped duplicates) in the same object.
                return Err(());
            }
            found = Some(val_slice);
        }
        match cursor.peek() {
            Some(b',') => cursor.pos += 1,
            Some(b'}') => return Ok(found),
            _ => return Err(()),
        }
    }
}

fn is_valid_literal_model_id(id: &str) -> bool {
    !id.is_empty() && id == id.trim() && !id.chars().any(|c| c.is_whitespace() || c.is_control())
}

struct RawModelEntry {
    id: String,
    name: String,
    is_zero_credit: bool,
}

fn parse_raw_model_entries(raw_json: &str) -> Result<Vec<RawModelEntry>, ()> {
    let Some(data_slice) = find_object_field_raw(raw_json, "data")? else {
        return Err(());
    };
    let Some(models_slice) = find_object_field_raw(data_slice, "models")? else {
        return Err(());
    };

    let mut cursor = RawJsonCursor::new(models_slice);
    cursor.expect_byte(b'[')?;
    let mut entries = Vec::new();
    if cursor.peek() == Some(b']') {
        return Ok(entries);
    }
    loop {
        let item_slice = cursor.scan_value_slice()?;
        if item_slice.trim_start().starts_with('{') {
            let id = match find_object_field_raw(item_slice, "id")? {
                Some(raw_id) => serde_json::from_str::<String>(raw_id)
                    .ok()
                    .filter(|s| is_valid_literal_model_id(s)),
                None => None,
            };
            if let Some(id) = id {
                let name = match find_object_field_raw(item_slice, "name")? {
                    Some(raw_name) => serde_json::from_str::<String>(raw_name)
                        .ok()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| id.clone()),
                    None => id.clone(),
                };
                let is_zero_credit = match find_object_field_raw(item_slice, "credits")? {
                    Some(raw_credits) => is_zero_credit_raw_token(raw_credits),
                    None => false,
                };
                entries.push(RawModelEntry {
                    id,
                    name,
                    is_zero_credit,
                });
            }
        }
        match cursor.peek() {
            Some(b',') => cursor.pos += 1,
            Some(b']') => return Ok(entries),
            _ => return Err(()),
        }
    }
}

/// Strictly selects a free model from the raw `/v3/config` JSON payload.
/// Deduplicates by exact literal model ID (excluding any ID appearing more than once),
/// rejects whitespace/control-char IDs, scientific-notation underflow (`1e-400`), negative
/// zero (`-0.0`), and empty catalogs. Prefers `PREFERRED_FREE_MODEL_ID`, otherwise picks the
/// first valid zero-credit candidate.
fn select_zero_credit_model_from_raw(raw_json: &str) -> Option<FreeModel> {
    let entries = parse_raw_model_entries(raw_json).ok()?;
    let mut id_counts: HashMap<&str, usize> = HashMap::new();
    for entry in &entries {
        *id_counts.entry(entry.id.as_str()).or_insert(0) += 1;
    }

    let mut fallback: Option<FreeModel> = None;
    for entry in &entries {
        if id_counts.get(entry.id.as_str()).copied().unwrap_or(0) != 1 {
            continue;
        }
        if !entry.is_zero_credit {
            continue;
        }
        let candidate = FreeModel {
            id: entry.id.clone(),
            name: entry.name.clone(),
        };
        if candidate.id == PREFERRED_FREE_MODEL_ID {
            return Some(candidate);
        }
        if fallback.is_none() {
            fallback = Some(candidate);
        }
    }
    fallback
}

async fn fetch_zero_credit_model(
    client: &Client,
    origin: &str,
    access_token: &str,
    uid: Option<&str>,
    domain: &str,
) -> Result<FreeModel, ApiError> {
    let url = format!("{origin}/v3/config");
    let req = apply_common_headers(
        client.get(&url),
        access_token,
        uid,
        domain,
        CONFIG_USER_AGENT,
    );
    let (raw_body, _body) = send_json_raw(req, "v3/config").await?;
    select_zero_credit_model_from_raw(&raw_body)
        .ok_or_else(|| ApiError::Other("WORKBUDDY_AI_NO_FREE_MODEL".to_string()))
}

/// Splits an SSE body into WHATWG event data payloads by blank-line boundaries, recognizing
/// CR, LF, and CRLF line endings and joining consecutive `data:` lines within the same event
/// with `\n`. Per WHATWG SSE spec, any incomplete event without a terminating blank line at
/// EOF is discarded.
fn collect_sse_event_payloads(sse_text: &str) -> Vec<String> {
    let mut events = Vec::new();
    let mut data_lines: Vec<&str> = Vec::new();
    let bytes = sse_text.as_bytes();
    let mut start = 0usize;
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] == b'\r' || bytes[i] == b'\n' {
            let line = &sse_text[start..i];
            if bytes[i] == b'\r' && i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                i += 2;
            } else {
                i += 1;
            }
            start = i;

            if line.is_empty() {
                if !data_lines.is_empty() {
                    events.push(data_lines.join("\n"));
                    data_lines.clear();
                }
                continue;
            }
            if line.starts_with(':') {
                continue;
            }
            if let Some(rest) = line.strip_prefix("data:") {
                let val = rest.strip_prefix(' ').unwrap_or(rest);
                data_lines.push(val);
            }
        } else {
            i += 1;
        }
    }
    events
}

fn parse_sse_metadata(sse_text: &str) -> Result<ParsedSseMeta, ApiError> {
    let trimmed_body = sse_text.trim();
    if trimmed_body.is_empty() {
        return Err(ApiError::Other(
            "WORKBUDDY_AI_INVALID_SSE_RESPONSE: empty body".to_string(),
        ));
    }

    // Reject HTTP 200 plain JSON bodies (not SSE). Once HTTP 200 has been returned from
    // /v2/chat/completions, never return retryable Unauthorized so no second chat POST is sent.
    if trimmed_body.starts_with('{') {
        return Err(ApiError::Other(
            "WORKBUDDY_AI_INVALID_SSE_RESPONSE: non-SSE JSON body".to_string(),
        ));
    }

    let events = collect_sse_event_payloads(sse_text);
    let mut valid_chunks: usize = 0;
    let mut saw_done = false;
    let mut finish_reason: Option<String> = None;
    let mut request_id: Option<String> = None;
    let mut response_model: Option<String> = None;
    let mut input_tokens: u64 = 0;
    let mut output_tokens: u64 = 0;
    let mut total_tokens: u64 = 0;

    for raw_payload in events {
        let payload = raw_payload.trim();
        if payload.is_empty() {
            continue;
        }
        if payload == "[DONE]" {
            if valid_chunks == 0 {
                return Err(ApiError::Other(
                    "WORKBUDDY_AI_INVALID_SSE_RESPONSE: premature [DONE] before content"
                        .to_string(),
                ));
            }
            if saw_done {
                return Err(ApiError::Other(
                    "WORKBUDDY_AI_INVALID_SSE_RESPONSE: duplicate [DONE]".to_string(),
                ));
            }
            saw_done = true;
            continue;
        }
        if saw_done {
            return Err(ApiError::Other(
                "WORKBUDDY_AI_INVALID_SSE_RESPONSE: trailing event after [DONE]".to_string(),
            ));
        }
        let chunk: Value = serde_json::from_str(payload).map_err(|_| {
            ApiError::Other("WORKBUDDY_AI_INVALID_SSE_RESPONSE: truncated JSON event".to_string())
        })?;
        if let Some(err_val) = chunk.get("error").filter(|v| !v.is_null()) {
            return Err(ApiError::Other(format!(
                "WORKBUDDY_AI_INVALID_SSE_RESPONSE: stream error ({err_val})"
            )));
        }
        if let Some(code_val) = chunk.get("code") {
            let Some(code) = code_val.as_i64() else {
                return Err(ApiError::Other(
                    "WORKBUDDY_AI_INVALID_SSE_RESPONSE: non-integer chunk code".to_string(),
                ));
            };
            if !matches!(code, 0 | 200) {
                // Once HTTP 200 SSE has started, NEVER treat any stream code (including a
                // first-event 401/10401/11401) as retryable Unauthorized.
                return Err(ApiError::Other(format!(
                    "WORKBUDDY_AI_STREAM_ABORTED_AFTER_START: stream code {code}"
                )));
            }
        }
        valid_chunks += 1;

        if let Some(id) = chunk
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if let Some(existing) = &request_id {
                if existing != id {
                    return Err(ApiError::Other(
                        "WORKBUDDY_AI_INVALID_SSE_RESPONSE: conflicting request ids".to_string(),
                    ));
                }
            } else {
                request_id = Some(id.to_string());
            }
        }
        if response_model.is_none() {
            response_model = chunk
                .get("model")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
        }
        if let Some(choices) = chunk.get("choices").and_then(Value::as_array) {
            for choice in choices {
                if let Some(reason) = choice
                    .get("finish_reason")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|r| !r.is_empty())
                {
                    if reason != "stop" {
                        return Err(ApiError::Other(format!(
                            "WORKBUDDY_AI_INVALID_SSE_RESPONSE: abnormal finish_reason ({reason})"
                        )));
                    }
                    finish_reason = Some(reason.to_string());
                }
            }
        }
        if let Some(usage) = chunk.get("usage").filter(|v| v.is_object()) {
            let p = usage
                .get("prompt_tokens")
                .and_then(Value::as_u64)
                .or_else(|| usage.get("input_tokens").and_then(Value::as_u64));
            let c = usage
                .get("completion_tokens")
                .and_then(Value::as_u64)
                .or_else(|| usage.get("output_tokens").and_then(Value::as_u64));
            let t = usage.get("total_tokens").and_then(Value::as_u64);
            if let Some(val) = p {
                input_tokens = val;
            }
            if let Some(val) = c {
                output_tokens = val;
            }
            if let Some(val) = t {
                total_tokens = val;
            }
            if total_tokens == 0 && input_tokens > 0 && output_tokens > 0 {
                total_tokens = input_tokens.saturating_add(output_tokens);
            }
        }
    }

    if valid_chunks == 0 || !(saw_done || finish_reason.as_deref() == Some("stop")) {
        return Err(ApiError::Other(
            "WORKBUDDY_AI_INVALID_SSE_RESPONSE: incomplete SSE stream".to_string(),
        ));
    }
    let Some(request_id) = request_id else {
        return Err(ApiError::Other(
            "WORKBUDDY_AI_INVALID_SSE_RESPONSE: missing request id".to_string(),
        ));
    };
    let Some(response_model) = response_model else {
        return Err(ApiError::Other(
            "WORKBUDDY_AI_INVALID_SSE_RESPONSE: missing response model".to_string(),
        ));
    };
    if input_tokens == 0 || output_tokens == 0 || total_tokens == 0 {
        return Err(ApiError::Other(
            "WORKBUDDY_AI_INVALID_SSE_RESPONSE: missing input/output usage tokens".to_string(),
        ));
    }

    Ok(ParsedSseMeta {
        request_id,
        response_model,
        finish_reason: finish_reason.unwrap_or_else(|| "stop".to_string()),
        input_tokens,
        output_tokens,
        total_tokens,
    })
}

async fn send_zero_credit_chat<M, MFut, C, CFut, W>(
    client: &Client,
    origin: &str,
    access_token: &str,
    uid: &str,
    domain: &str,
    free_model: &FreeModel,
    activity_date: &str,
    reserve_pending: &M,
    release_pending: &C,
    can_send_write: &W,
) -> Result<ChatTurnArtifacts, ApiError>
where
    M: Fn(String, String) -> MFut,
    MFut: Future<Output = Result<(), String>>,
    C: Fn(String) -> CFut,
    CFut: Future<Output = Result<(), String>>,
    W: Fn() -> bool,
{
    // Re-verify schedule/window/enabled gate immediately before committing the write-ahead
    // reservation and sending the HTTP POST so slow pre-checks can never cross the window cutoff.
    if !can_send_write() {
        return Err(ApiError::Skipped);
    }

    // Write-ahead reservation BEFORE sending POST /v2/chat/completions:
    // if persisting the reservation fails, abort immediately with zero chat POSTs.
    reserve_pending(activity_date.to_string(), free_model.id.clone())
        .await
        .map_err(|err| ApiError::Other(format!("WORKBUDDY_AI_PENDING_PERSIST_FAILED: {err}")))?;

    // Re-verify write gate again AFTER async persistence completes and immediately BEFORE sending
    // the HTTP POST. If permission expired while writing to disk, release this unsent reservation.
    if !can_send_write() {
        if let Err(rel_err) = release_pending(activity_date.to_string()).await {
            logger::log_warn(&format!(
                "[WorkBuddy AI Checkin] Failed to release unsent pending reservation after write gate closed: {rel_err}"
            ));
        }
        return Err(ApiError::Skipped);
    }

    let cid = uuid::Uuid::new_v4().to_string();
    let conv_req_id = uuid::Uuid::new_v4().simple().to_string();
    let msg_id = uuid::Uuid::new_v4().simple().to_string();
    let trace_id = uuid::Uuid::new_v4().simple().to_string();
    let span_id = msg_id[..16].to_string();

    let chat_payload = json!({
        "model": free_model.id,
        "messages": [
            { "role": "system", "content": "You are a helpful assistant." },
            { "role": "user", "content": "Hi" }
        ],
        "agent": "cli",
        "temperature": 0.9,
        "top_p": 1,
        "stream": true,
        "stream_options": { "include_usage": true }
    });

    let baggage = format!(
        "buddyId=,buddyName=,codebuddy.session_id={cid},codebuddy.conversation_request_id={conv_req_id}"
    );
    let chat_url = format!("{origin}/v2/chat/completions");
    let chat_response = match client
        .post(&chat_url)
        .bearer_auth(access_token)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("User-Agent", CHAT_USER_AGENT)
        .header("X-Requested-With", "XMLHttpRequest")
        .header("X-Product", "SaaS")
        .header("X-Domain", domain)
        .header("X-User-Id", uid)
        .header("X-Private-Data", "true")
        .header("X-CodeBuddy-Request", "1")
        .header("X-Agent-Runtime", "local-agent")
        .header("X-Agent-Purpose", "conversation")
        .header("X-Agent-Intent", "craft")
        .header("X-Agent-Type", "main")
        .header("X-IDE-Type", "WorkBuddy")
        .header("X-IDE-Name", "WorkBuddy")
        .header("X-IDE-Version", CLIENT_VERSION)
        .header("X-Conversation-ID", &cid)
        .header("X-Conversation-Request-ID", &conv_req_id)
        .header("X-Conversation-Message-ID", &msg_id)
        .header("X-Request-ID", &msg_id)
        .header("X-Root-Request-ID", &conv_req_id)
        .header("X-Trace-ID", &trace_id)
        .header("X-B3-TraceId", &trace_id)
        .header("X-B3-SpanId", &span_id)
        .header("X-B3-Sampled", "1")
        .header("baggage", &baggage)
        .json(&chat_payload)
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(error) => {
            // Only release the reservation if TCP/TLS never connected (provably pre-execution).
            // Any post-connect timeout or broken pipe leaves pending persisted on disk.
            if error.is_connect() {
                if let Err(rel_err) = release_pending(activity_date.to_string()).await {
                    logger::log_warn(&format!(
                        "[WorkBuddy AI Checkin] Failed to release pending chat reservation after connect error: {rel_err}"
                    ));
                }
            }
            return Err(ApiError::Other(format!(
                "v2/chat/completions: {}",
                error.without_url()
            )));
        }
    };

    let chat_status = chat_response.status();
    if chat_status == StatusCode::UNAUTHORIZED {
        // Provably rejected by auth middleware before execution: release reservation so the
        // caller may renew token, re-verify /v3/config, re-reserve, and retry once.
        if let Err(rel_err) = release_pending(activity_date.to_string()).await {
            logger::log_warn(&format!(
                "[WorkBuddy AI Checkin] Failed to release pending chat reservation after HTTP 401: {rel_err}"
            ));
            return Err(ApiError::Other(format!(
                "v2/chat/completions: HTTP 401 (failed to release reservation: {rel_err})"
            )));
        }
        return Err(ApiError::Unauthorized(
            "v2/chat/completions: HTTP 401".to_string(),
        ));
    }
    if matches!(chat_status, StatusCode::BAD_REQUEST | StatusCode::FORBIDDEN) {
        if let Err(rel_err) = release_pending(activity_date.to_string()).await {
            logger::log_warn(&format!(
                "[WorkBuddy AI Checkin] Failed to release pending chat reservation after HTTP {}: {rel_err}",
                chat_status.as_u16()
            ));
        }
        return Err(ApiError::Other(format!(
            "v2/chat/completions: HTTP {}",
            chat_status.as_u16()
        )));
    }
    if chat_status != StatusCode::OK {
        // Any other non-200 status (e.g. 500/502/503/504/408/429) has uncertain execution state:
        // KEEP the write-ahead pending reservation on disk and never replay.
        return Err(ApiError::Other(format!(
            "v2/chat/completions: HTTP {}",
            chat_status.as_u16()
        )));
    }

    // HTTP 200 reached: pending reservation is already persisted on disk before POST.
    let sse_bytes = chat_response.bytes().await.map_err(|error| {
        ApiError::Other(format!("v2/chat/completions: {}", error.without_url()))
    })?;
    let sse_text = std::str::from_utf8(&sse_bytes).map_err(|_| {
        ApiError::Other("WORKBUDDY_AI_INVALID_SSE_RESPONSE: invalid UTF-8 response".to_string())
    })?;

    let sse_meta = parse_sse_metadata(sse_text)?;
    Ok(ChatTurnArtifacts {
        free_model: free_model.clone(),
        cid,
        msg_id,
        trace_id,
        sse_meta,
    })
}

fn prepare_report_payload(uid: &str, turn: &ChatTurnArtifacts) -> PreparedReport {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let host = host_telemetry();
    let machine_id = derived_hex36("machine", uid);
    let session_id = derived_hex36("session", uid);
    let attempt_id = uuid::Uuid::new_v4().to_string();
    let assistant_msg_id = format!("{}-assistant", turn.msg_id);
    let report_req_id = uuid::Uuid::new_v4().simple().to_string();

    let base_event = || {
        json!({
            "timezone": host.timezone,
            "reportDelay": 2000,
            "userId": uid,
            "username": "",
            "userNickname": "",
            "product": "SaaS",
            "releaseDate": CLIENT_RELEASE_DATE_MS,
            "commit": CLIENT_COMMIT,
            "ideName": "WorkBuddy",
            "ideType": "WorkBuddy",
            "ideVersion": CLIENT_VERSION,
            "machineId": machine_id,
            "sessionId": session_id,
            "extName": "workbuddy-desktop",
            "extVersion": CLIENT_VERSION,
            "os": host.os,
            "arch": host.arch,
            "osVersion": host.os_version,
            "cpuModel": host.cpu_model,
            "cpuCores": host.cpu_cores,
            "memorySize": host.memory_gb,
            "timestamp": now_ms,
            "presentAt": now_ms,
        })
    };

    let merge_obj = |mut base: Value, extra: Value| -> Value {
        if let (Some(dst), Some(src)) = (base.as_object_mut(), extra.as_object()) {
            for (k, v) in src {
                dst.insert(k.clone(), v.clone());
            }
        }
        base
    };

    let ev1 = merge_obj(
        base_event(),
        json!({
            "eventCode": "agent_task_created",
            "source": "LOCAL",
            "name": "working",
            "task_target": "local",
            "mode": "craft",
            "requestModelId": turn.free_model.id,
            "requestModelName": turn.free_model.name,
            "has_repo": false,
            "repo_type": "none",
            "workspace_type": "empty",
            "has_connector": false,
            "connector_types": [],
            "has_mention": false,
            "mention_types": [],
            "has_template": false,
            "action": "",
            "template_name": "",
            "has_expert": false,
            "expert_id": "",
            "expert_name": "",
            "expert_industry_id": "",
            "has_skill": false,
            "skill_names": [],
            "conversationId": turn.cid,
            "messageId": turn.msg_id,
            "buddyId": "",
            "buddyName": "",
        }),
    );

    let ev2 = merge_obj(
        base_event(),
        json!({
            "eventCode": "chat_request_send",
            "mode": "craft",
            "conversationId": turn.cid,
            "requestId": turn.sse_meta.request_id,
            "inputLength": 2,
            "requestModelId": turn.free_model.id,
            "requestModelName": turn.free_model.name,
            "isPlan": false,
            "isAutoExecuteTerminal": false,
            "isAutoModify": false,
            "codebaseEnable": false,
            "isMaxMode": true,
            "source": "work",
            "mentionContexts": [],
            "knowledgeId": [],
            "knowledgeName": [],
            "codebaseId": "",
            "mentionContextCount": 0,
            "command": "",
            "inputAttemptId": attempt_id,
            "recommendId": "",
            "skillId": "",
            "traceId": turn.trace_id,
            "rootRequestId": turn.sse_meta.request_id,
            "parentConversationId": turn.cid,
            "agentName": "cli",
            "agentType": "main",
            "vcsType": "unknown",
            "vcsRepo": "",
            "vcsBranchName": "",
            "vcsRevId": "",
            "buddyId": "",
            "buddyName": "",
            "codebuddy.session_id": turn.cid,
            "codebuddy.conversation_request_id": turn.sse_meta.request_id,
        }),
    );

    let ev3 = merge_obj(
        base_event(),
        json!({
            "eventCode": "chat_message_response",
            "messageId": assistant_msg_id,
            "responseModelId": turn.sse_meta.response_model,
            "inputToken": turn.sse_meta.input_tokens,
            "outputToken": turn.sse_meta.output_tokens,
            "totalToken": turn.sse_meta.total_tokens,
            "cachedTokens": 0,
            "cachedWriteTokens": 0,
            "cachedMissTokens": 0,
            "isSuccessful": true,
            "messageErrorCode": "",
            "finishReason": turn.sse_meta.finish_reason,
            "firstTokenAt": now_ms,
            "traceId": turn.trace_id,
            "conversationId": turn.cid,
            "rootRequestId": turn.sse_meta.request_id,
            "parentConversationId": turn.cid,
            "agentName": "cli",
            "agentType": "main",
            "codebuddy.session_id": turn.cid,
            "codebuddy.conversation_request_id": turn.sse_meta.request_id,
        }),
    );

    let ev4 = merge_obj(
        base_event(),
        json!({
            "eventCode": "chat_request_response",
            "mode": "craft",
            "toolCallCount": 0,
            "inputToken": turn.sse_meta.input_tokens,
            "outputToken": turn.sse_meta.output_tokens,
            "totalToken": turn.sse_meta.total_tokens,
            "cachedTokens": 0,
            "cachedWriteTokens": 0,
            "cachedMissTokens": 0,
            "isSuccessful": true,
            "messageErrorCode": "",
            "finishReason": turn.sse_meta.finish_reason,
            "conversationId": turn.cid,
            "rootRequestId": turn.sse_meta.request_id,
            "parentConversationId": turn.cid,
        }),
    );

    PreparedReport {
        request_id: report_req_id,
        payload: Value::Array(vec![ev1, ev2, ev3, ev4]),
    }
}

async fn send_prepared_report(
    client: &Client,
    origin: &str,
    access_token: &str,
    uid: &str,
    domain: &str,
    prepared: &PreparedReport,
) -> Result<(), ApiError> {
    let report_url = format!("{origin}/v2/report");
    let report_req = apply_common_headers(
        client
            .post(&report_url)
            .header("Content-Type", "application/json;charset=UTF-8")
            .header("X-Request-ID", &prepared.request_id)
            .json(&prepared.payload),
        access_token,
        Some(uid),
        domain,
        GROWTH_USER_AGENT,
    );
    send_json(report_req, "v2/report").await.map(|_| ())
}

async fn send_prepared_report_with_session<R, Fut>(
    client: &Client,
    origin: &str,
    session: &mut ActiveSession<R>,
    prepared_report: &PreparedReport,
    retry_delays: &[Duration],
) -> Result<(), ApiError>
where
    R: FnMut(WorkbuddyAccount) -> Fut,
    Fut: Future<Output = Result<WorkbuddyAccount, String>>,
{
    let mut attempt = 0usize;
    loop {
        match session
            .call(|acc| {
                let domain = account_domain(acc).to_string();
                let uid = account_uid(acc).map(str::to_owned);
                let token = acc.access_token.clone();
                let report = prepared_report.clone();
                async move {
                    let uid_str = uid.as_deref().ok_or_else(|| {
                        ApiError::Other("WORKBUDDY_AI_MISSING_ACCOUNT_ID".to_string())
                    })?;
                    send_prepared_report(client, origin, &token, uid_str, &domain, &report).await
                }
            })
            .await
        {
            Ok(()) => return Ok(()),
            Err(err) if err.is_transient() && attempt < retry_delays.len() => {
                let delay = retry_delays[attempt];
                attempt += 1;
                logger::log_warn(&format!(
                    "[WorkBuddy AI Checkin] Transient v2/report failure (retry {attempt}/{}): {}",
                    retry_delays.len(),
                    err.into_message()
                ));
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
            }
            Err(err) => return Err(err),
        }
    }
}

async fn load_account_async(account_id: &str) -> Result<WorkbuddyAccount, String> {
    let id = account_id.to_string();
    tauri::async_runtime::spawn_blocking(move || workbuddy_ai_account::load(&id))
        .await
        .map_err(|error| error.to_string())?
}

struct InFlightRenewGuard<'a> {
    map: &'a Mutex<HashMap<String, tokio::sync::watch::Receiver<Option<SharedRenewResult>>>>,
    account_id: String,
    tx: Option<tokio::sync::watch::Sender<Option<SharedRenewResult>>>,
}

impl<'a> InFlightRenewGuard<'a> {
    fn finish(mut self, result: SharedRenewResult) -> SharedRenewResult {
        if let Ok(mut guard) = self.map.lock() {
            guard.remove(&self.account_id);
        }
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Some(result.clone()));
        }
        result
    }
}

impl Drop for InFlightRenewGuard<'_> {
    fn drop(&mut self) {
        if let Some(tx) = self.tx.take() {
            if let Ok(mut guard) = self.map.lock() {
                guard.remove(&self.account_id);
            }
            let _ = tx.send(Some(Err("WORKBUDDY_AI_REFRESH_CANCELLED".to_string())));
        }
    }
}

async fn renew_account_session_with_ops<L, LFut, R, RFut>(
    in_flight: &Mutex<HashMap<String, tokio::sync::watch::Receiver<Option<SharedRenewResult>>>>,
    account_id: &str,
    stale_access_token: &str,
    load_latest: L,
    perform_renew: R,
) -> Result<WorkbuddyAccount, String>
where
    L: Fn(String) -> LFut,
    LFut: Future<Output = Result<WorkbuddyAccount, String>>,
    R: Fn(WorkbuddyAccount) -> RFut,
    RFut: Future<Output = Result<WorkbuddyAccount, String>>,
{
    enum RenewRole<'a> {
        Join(tokio::sync::watch::Receiver<Option<SharedRenewResult>>),
        Lead(InFlightRenewGuard<'a>),
    }

    for _attempt in 0..2 {
        let role = {
            let mut map = in_flight.lock().map_err(|error| error.to_string())?;
            if let Some(existing_rx) = map.get(account_id).cloned() {
                RenewRole::Join(existing_rx)
            } else {
                let (tx, rx) = tokio::sync::watch::channel(None);
                map.insert(account_id.to_string(), rx);
                RenewRole::Lead(InFlightRenewGuard {
                    map: in_flight,
                    account_id: account_id.to_string(),
                    tx: Some(tx),
                })
            }
        };

        match role {
            RenewRole::Lead(guard) => {
                let result = match load_latest(account_id.to_string()).await {
                    Ok(latest) if latest.access_token != stale_access_token => Ok(latest),
                    Ok(latest) => perform_renew(latest).await,
                    Err(err) => Err(err),
                };
                return guard.finish(result);
            }
            RenewRole::Join(mut rx) => loop {
                if let Some(result) = rx.borrow().clone() {
                    match result {
                        Ok(acc) if acc.access_token != stale_access_token => return Ok(acc),
                        Ok(_) => break,
                        Err(err) => return Err(err),
                    }
                }
                if rx.changed().await.is_err() {
                    break;
                }
            },
        }
    }

    Err("WORKBUDDY_AI_REFRESH_IN_PROGRESS".to_string())
}

pub(crate) async fn renew_account_session_shared(
    account_id: &str,
    stale_access_token: &str,
) -> Result<WorkbuddyAccount, String> {
    renew_account_session_with_ops(
        &IN_FLIGHT_RENEWALS,
        account_id,
        stale_access_token,
        |id| async move { load_account_async(&id).await },
        |latest| async move {
            if !has_refresh_token(&latest) {
                return Err("WORKBUDDY_AI_TOKEN_EXPIRED".to_string());
            }
            let session = latest
                .auth_raw
                .clone()
                .ok_or_else(|| "WORKBUDDY_AI_MISSING_SESSION".to_string())?;
            let renewed = workbuddy_ai_oauth::renew_auth_classified(&session)
                .await
                .map_err(workbuddy_ai_oauth::RenewAuthError::into_message)?;
            tauri::async_runtime::spawn_blocking(move || {
                workbuddy_ai_account::store_refreshed_session(&latest, renewed)
            })
            .await
            .map_err(|error| error.to_string())?
        },
    )
    .await
}

async fn renew_account_session(account: WorkbuddyAccount) -> Result<WorkbuddyAccount, String> {
    renew_account_session_shared(&account.id, &account.access_token).await
}

fn build_status_from_parts(
    account: &WorkbuddyAccount,
    heatmap: HeatmapSummary,
    streak: Option<StreakSummary>,
) -> WorkbuddyAiCheckinStatus {
    let today_checked_in = heatmap.is_active && heatmap.score > 0;
    let pending_confirmation = !today_checked_in
        && workbuddy_ai_account::has_recorded_checkin_for_heatmap(
            account,
            &heatmap.today_date,
            &heatmap.active_dates,
        );
    let (streak_days, month_total_days) = match streak {
        Some(s) => (s.streak_days, s.month_total_days),
        None => (0, 0),
    };
    let checkin_dates = if heatmap.active_dates.is_empty() {
        None
    } else {
        Some(heatmap.active_dates)
    };
    WorkbuddyAiCheckinStatus {
        today_checked_in,
        active: true,
        streak_days,
        daily_credit: 0,
        today_credit: None,
        checkin_dates,
        today_date: Some(heatmap.today_date),
        today_score: heatmap.score,
        month_total_days,
        pending_confirmation,
        message: heatmap.status_text,
    }
}

async fn get_checkin_status_with_origin_and_renewer<R, Fut>(
    client: &Client,
    origin: &str,
    account: WorkbuddyAccount,
    renewer: R,
) -> Result<WorkbuddyAiCheckinStatus, ApiError>
where
    R: FnMut(WorkbuddyAccount) -> Fut,
    Fut: Future<Output = Result<WorkbuddyAccount, String>>,
{
    if is_enterprise_account(&account) {
        return Ok(WorkbuddyAiCheckinStatus {
            today_checked_in: false,
            active: false,
            streak_days: 0,
            daily_credit: 0,
            today_credit: None,
            checkin_dates: None,
            today_date: None,
            today_score: 0,
            month_total_days: 0,
            pending_confirmation: false,
            message: Some("WORKBUDDY_AI_ENTERPRISE_INACTIVE".to_string()),
        });
    }

    let mut session = ActiveSession::new(account, renewer);
    let heatmap = session
        .call(|acc| {
            let domain = account_domain(acc).to_string();
            let uid = account_uid(acc).map(str::to_owned);
            let token = acc.access_token.clone();
            async move { fetch_heatmap(client, origin, &token, uid.as_deref(), &domain).await }
        })
        .await?;

    let streak = match session
        .call(|acc| {
            let domain = account_domain(acc).to_string();
            let uid = account_uid(acc).map(str::to_owned);
            let token = acc.access_token.clone();
            async move { fetch_streak(client, origin, &token, uid.as_deref(), &domain).await }
        })
        .await
    {
        Ok(streak) => Some(streak),
        Err(error) if error.is_unauthorized() => return Err(error),
        Err(error) => {
            logger::log_warn(&format!(
                "[WorkBuddy AI Checkin] Failed to query growth/streak: {}",
                error.into_message()
            ));
            None
        }
    };

    Ok(build_status_from_parts(&session.account, heatmap, streak))
}

fn heatmap_confirmed_date(after: &HeatmapSummary, target_date: &str) -> Option<String> {
    if after.today_date == target_date {
        (after.is_active && after.score > 0).then(|| target_date.to_string())
    } else if after.today_date.as_str() > target_date {
        // Server rolled over to the next activity day during verification:
        // 1. Prefer target_date if it appears in historical active cells.
        // 2. Otherwise, if the in-flight chat itself crossed midnight into target_date + 1d and
        //    lit up that adjacent day, attribute confirmation to that day.
        if after.active_dates.iter().any(|d| d == target_date) {
            Some(target_date.to_string())
        } else if workbuddy_ai_account::is_adjacent_next_day(target_date, &after.today_date)
            && after.is_active
            && after.score > 0
        {
            Some(after.today_date.clone())
        } else {
            None
        }
    } else {
        None
    }
}

fn build_pending_confirmation_result(
    account: &WorkbuddyAccount,
    heatmap: HeatmapSummary,
    mut warning: Option<String>,
) -> WorkbuddyAiCheckinResult {
    append_warning(
        &mut warning,
        "WORKBUDDY_AI_PENDING_CONFIRMATION".to_string(),
    );
    let recorded_model_id = if workbuddy_ai_account::has_pending_checkin_turn_for_heatmap(
        account,
        &heatmap.today_date,
        &heatmap.active_dates,
    ) {
        account
            .checkin_rewards
            .as_ref()
            .and_then(|v| v.get("pendingModelId"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    } else {
        account
            .checkin_rewards
            .as_ref()
            .and_then(|v| v.get("modelId"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    WorkbuddyAiCheckinResult {
        success: false,
        already_claimed: false,
        inactive: false,
        pending_confirmation: true,
        result_code: Some("pending_confirmation".to_string()),
        activity_date: Some(heatmap.today_date),
        warning,
        message: None,
        credit: None,
        streak_days: None,
        score_before: Some(heatmap.score),
        score_after: None,
        model_id: recorded_model_id,
    }
}

async fn complete_already_active_checkin<R, Fut>(
    client: &Client,
    origin: &str,
    session: &mut ActiveSession<R>,
    active_heatmap: HeatmapSummary,
    mut warning: Option<String>,
) -> Result<WorkbuddyAiCheckinResult, ApiError>
where
    R: FnMut(WorkbuddyAccount) -> Fut,
    Fut: Future<Output = Result<WorkbuddyAccount, String>>,
{
    let streak = match session
        .call(|acc| {
            let domain = account_domain(acc).to_string();
            let uid = account_uid(acc).map(str::to_owned);
            let token = acc.access_token.clone();
            async move { fetch_streak(client, origin, &token, uid.as_deref(), &domain).await }
        })
        .await
    {
        Ok(streak) => Some(streak),
        Err(error) => {
            let msg = error.into_message();
            logger::log_warn(&format!(
                "[WorkBuddy AI Checkin] Failed to query growth/streak (already active): {msg}"
            ));
            append_warning(&mut warning, format!("streak: {msg}"));
            None
        }
    };
    Ok(WorkbuddyAiCheckinResult {
        success: true,
        already_claimed: true,
        inactive: false,
        pending_confirmation: false,
        result_code: Some("already_active".to_string()),
        activity_date: Some(active_heatmap.today_date),
        warning,
        message: None,
        credit: None,
        streak_days: streak.map(|s| s.streak_days),
        score_before: Some(active_heatmap.score),
        score_after: Some(active_heatmap.score),
        model_id: None,
    })
}

enum PreChatOutcome {
    AlreadyActive(HeatmapSummary),
    PendingConfirmation(HeatmapSummary),
    ChatSent {
        target_date: String,
        score_before: i64,
        turn: ChatTurnArtifacts,
    },
}

async fn perform_checkin_with_origin_and_delays<R, Fut, M, MFut, C, CFut, W>(
    client: &Client,
    origin: &str,
    account: WorkbuddyAccount,
    verify_delays: &[Duration],
    report_retry_delays: &[Duration],
    renewer: R,
    reserve_pending: M,
    release_pending: C,
    can_send_write: W,
) -> Result<WorkbuddyAiCheckinResult, ApiError>
where
    R: FnMut(WorkbuddyAccount) -> Fut,
    Fut: Future<Output = Result<WorkbuddyAccount, String>>,
    M: Fn(String, String) -> MFut,
    MFut: Future<Output = Result<(), String>>,
    C: Fn(String) -> CFut,
    CFut: Future<Output = Result<(), String>>,
    W: Fn() -> bool,
{
    if is_enterprise_account(&account) {
        return Ok(WorkbuddyAiCheckinResult {
            success: false,
            already_claimed: false,
            inactive: true,
            pending_confirmation: false,
            result_code: Some("inactive".to_string()),
            activity_date: None,
            warning: None,
            message: None,
            credit: None,
            streak_days: None,
            score_before: None,
            score_after: None,
            model_id: None,
        });
    }

    if account_uid(&account).is_none() {
        return Err(ApiError::Other(
            "WORKBUDDY_AI_MISSING_ACCOUNT_ID".to_string(),
        ));
    }

    let mut session = ActiveSession::new(account, renewer);
    let mut warning: Option<String> = None;
    let can_send_write = Arc::new(can_send_write);

    // Stage 1: Read-only heatmap pre-check.
    let before = session
        .call(|acc| {
            let domain = account_domain(acc).to_string();
            let uid = account_uid(acc).map(str::to_owned);
            let token = acc.access_token.clone();
            async move { fetch_heatmap(client, origin, &token, uid.as_deref(), &domain).await }
        })
        .await?;

    if before.is_active && before.score > 0 {
        return complete_already_active_checkin(client, origin, &mut session, before, warning)
            .await;
    }

    // Persistent at-most-once guard: if a chat turn was already reserved/submitted OR already
    // confirmed for this server activity date (or an unresolved recent previous-day reservation
    // that may have crossed midnight), stay read-only and never emit a second chat POST.
    if workbuddy_ai_account::has_recorded_checkin_for_heatmap(
        &session.account,
        &before.today_date,
        &before.active_dates,
    ) {
        return Ok(build_pending_confirmation_result(
            &session.account,
            before,
            warning,
        ));
    }

    // Stage 2 + 3: Live-verify free model from /v3/config, re-verify live server activity date
    // right before writing reservation (preventing midnight-rollover date mismatch), re-verify
    // write gate before & after reservation, and send 1 minimal chat turn.
    let reserve_pending = Arc::new(reserve_pending);
    let release_pending = Arc::new(release_pending);
    let pre_chat = session
        .call(|acc| {
            let domain = account_domain(acc).to_string();
            let uid = account_uid(acc).map(str::to_owned);
            let token = acc.access_token.clone();
            let acc_snapshot = acc.clone();
            let rp = reserve_pending.clone();
            let cp = release_pending.clone();
            let cw = can_send_write.clone();
            async move {
                let uid_str = uid.as_deref().ok_or_else(|| {
                    ApiError::Other("WORKBUDDY_AI_MISSING_ACCOUNT_ID".to_string())
                })?;
                let free_model =
                    fetch_zero_credit_model(client, origin, &token, Some(uid_str), &domain).await?;
                let live_heatmap =
                    fetch_heatmap(client, origin, &token, Some(uid_str), &domain).await?;
                if live_heatmap.is_active && live_heatmap.score > 0 {
                    return Ok(PreChatOutcome::AlreadyActive(live_heatmap));
                }
                if workbuddy_ai_account::has_recorded_checkin_for_heatmap(
                    &acc_snapshot,
                    &live_heatmap.today_date,
                    &live_heatmap.active_dates,
                ) {
                    return Ok(PreChatOutcome::PendingConfirmation(live_heatmap));
                }
                let turn = send_zero_credit_chat(
                    client,
                    origin,
                    &token,
                    uid_str,
                    &domain,
                    &free_model,
                    &live_heatmap.today_date,
                    rp.as_ref(),
                    cp.as_ref(),
                    cw.as_ref(),
                )
                .await?;
                Ok(PreChatOutcome::ChatSent {
                    target_date: live_heatmap.today_date,
                    score_before: live_heatmap.score,
                    turn,
                })
            }
        })
        .await?;

    let (target_date, score_before, turn) = match pre_chat {
        PreChatOutcome::AlreadyActive(live_heatmap) => {
            return complete_already_active_checkin(
                client,
                origin,
                &mut session,
                live_heatmap,
                warning,
            )
            .await;
        }
        PreChatOutcome::PendingConfirmation(live_heatmap) => {
            return Ok(build_pending_confirmation_result(
                &session.account,
                live_heatmap,
                warning,
            ));
        }
        PreChatOutcome::ChatSent {
            target_date,
            score_before,
            turn,
        } => (target_date, score_before, turn),
    };

    // Stage 4: Build report payload and X-Request-ID ONCE so both 401 refresh and bounded
    // transient retries resend the exact same request ID, attempt_id, timestamps, and events
    // without ever replaying chat.
    let report_uid = account_uid(&session.account)
        .ok_or_else(|| ApiError::Other("WORKBUDDY_AI_MISSING_ACCOUNT_ID".to_string()))?
        .to_string();
    let prepared_report = prepare_report_payload(&report_uid, &turn);

    send_prepared_report_with_session(
        client,
        origin,
        &mut session,
        &prepared_report,
        report_retry_delays,
    )
    .await?;

    // Stage 5: Bounded read-only verification loop confirming target activity date.
    let delays: &[Duration] = if verify_delays.is_empty() {
        &[Duration::ZERO]
    } else {
        verify_delays
    };
    let mut confirmed_after: Option<(HeatmapSummary, String)> = None;
    let mut last_verify_error: Option<String> = None;
    for delay in delays {
        if !delay.is_zero() {
            tokio::time::sleep(*delay).await;
        }
        match session
            .call(|acc| {
                let domain = account_domain(acc).to_string();
                let uid = account_uid(acc).map(str::to_owned);
                let token = acc.access_token.clone();
                async move { fetch_heatmap(client, origin, &token, uid.as_deref(), &domain).await }
            })
            .await
        {
            Ok(after) => {
                if let Some(confirmed_date) = heatmap_confirmed_date(&after, &target_date) {
                    confirmed_after = Some((after, confirmed_date));
                    break;
                }
            }
            Err(error) => {
                last_verify_error = Some(error.into_message());
            }
        }
    }

    let Some((after, confirmed_date)) = confirmed_after else {
        if let Some(err) = last_verify_error {
            append_warning(&mut warning, format!("verify: {err}"));
        }
        append_warning(
            &mut warning,
            "WORKBUDDY_AI_PENDING_CONFIRMATION".to_string(),
        );
        return Ok(WorkbuddyAiCheckinResult {
            success: false,
            already_claimed: false,
            inactive: false,
            pending_confirmation: true,
            result_code: Some("pending_confirmation".to_string()),
            activity_date: Some(target_date),
            warning,
            message: None,
            credit: None,
            streak_days: None,
            score_before: Some(score_before),
            score_after: None,
            model_id: Some(turn.free_model.id),
        });
    };

    // Stage 6: Read-only streak query (failure preserves None rather than overwriting with 0).
    let streak = match session
        .call(|acc| {
            let domain = account_domain(acc).to_string();
            let uid = account_uid(acc).map(str::to_owned);
            let token = acc.access_token.clone();
            async move { fetch_streak(client, origin, &token, uid.as_deref(), &domain).await }
        })
        .await
    {
        Ok(streak) => Some(streak),
        Err(error) => {
            let msg = error.into_message();
            logger::log_warn(&format!(
                "[WorkBuddy AI Checkin] Failed to query growth/streak after report: {msg}"
            ));
            append_warning(&mut warning, format!("streak: {msg}"));
            None
        }
    };

    let confirmed_score = if after.today_date == confirmed_date {
        Some(after.score)
    } else {
        None
    };

    Ok(WorkbuddyAiCheckinResult {
        success: true,
        already_claimed: false,
        inactive: false,
        pending_confirmation: false,
        result_code: Some("activated".to_string()),
        activity_date: Some(confirmed_date),
        warning,
        message: None,
        credit: None,
        streak_days: streak.map(|s| s.streak_days),
        score_before: Some(score_before),
        score_after: confirmed_score,
        model_id: Some(turn.free_model.id),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ReconcileTarget {
    Today {
        activity_date: String,
        score_after: i64,
    },
    PriorPending {
        activity_date: String,
    },
}

fn resolve_reconcile_target(
    account: &WorkbuddyAccount,
    status: &WorkbuddyAiCheckinStatus,
) -> Option<ReconcileTarget> {
    if !status.active {
        return None;
    }
    let today_date = status.today_date.as_deref()?;
    let active_dates = status.checkin_dates.as_deref().unwrap_or(&[]);

    if status.today_checked_in
        && workbuddy_ai_account::has_pending_checkin_turn_for_heatmap(
            account,
            today_date,
            active_dates,
        )
    {
        return Some(ReconcileTarget::Today {
            activity_date: today_date.to_string(),
            score_after: status.today_score,
        });
    }

    // Reconcile a prior day's persisted `pendingActivityDate` once that date appears in the
    // server's active `checkin_dates` (e.g. on D+1 before checking in for D+1), without
    // falsely marking today as checked in.
    let pending_date = account
        .checkin_rewards
        .as_ref()
        .and_then(|v| v.get("pendingActivityDate"))
        .and_then(Value::as_str)
        .and_then(parse_iso_date)?;

    let existing_confirmed_is_newer = account
        .checkin_rewards
        .as_ref()
        .and_then(|v| v.get("activityDate"))
        .and_then(Value::as_str)
        .is_some_and(|confirmed| confirmed > pending_date.as_str());

    if !existing_confirmed_is_newer
        && pending_date.as_str() < today_date
        && active_dates.iter().any(|d| d == &pending_date)
    {
        return Some(ReconcileTarget::PriorPending {
            activity_date: pending_date,
        });
    }

    None
}

fn build_reconcile_update(
    latest: &WorkbuddyAccount,
    status: &WorkbuddyAiCheckinStatus,
    now_ts: i64,
) -> Option<(String, Option<i64>, Option<i32>, Value)> {
    let target = resolve_reconcile_target(latest, status)?;
    let streak = (status.streak_days > 0).then_some(status.streak_days);
    match target {
        ReconcileTarget::Today {
            activity_date,
            score_after,
        } => {
            let rewards = json!({
                "activityDate": activity_date,
                "settlement": "next_day",
                "modelId": Value::Null,
                "scoreAfter": score_after,
            });
            Some((activity_date, Some(now_ts), streak, rewards))
        }
        ReconcileTarget::PriorPending { activity_date } => {
            let pending_created_at = latest
                .checkin_rewards
                .as_ref()
                .and_then(|v| v.get("pendingCreatedAt"))
                .and_then(Value::as_i64);
            let rewards = json!({
                "activityDate": activity_date,
                "settlement": "next_day",
                "modelId": Value::Null,
                "scoreAfter": Value::Null,
            });
            Some((activity_date, pending_created_at, streak, rewards))
        }
    }
}

async fn get_checkin_status_and_reconcile_with_ops<R, RFut, L, U, UFut, Rec, RecFut>(
    client: &Client,
    origin: &str,
    account: WorkbuddyAccount,
    renewer: R,
    acquire_lock: L,
    persist_reconcile: U,
    record_reconcile: Rec,
) -> Result<(WorkbuddyAiCheckinStatus, bool), String>
where
    R: FnMut(WorkbuddyAccount) -> RFut,
    RFut: Future<Output = Result<WorkbuddyAccount, String>>,
    L: FnOnce() -> Option<Arc<tokio::sync::Mutex<()>>>,
    U: FnOnce(WorkbuddyAiCheckinStatus, i64) -> UFut,
    UFut: Future<Output = Result<Option<(String, Option<String>)>, String>>,
    Rec: FnOnce(String, Option<String>) -> RecFut,
    RecFut: Future<Output = ()>,
{
    let status =
        get_checkin_status_with_origin_and_renewer(client, origin, account.clone(), renewer)
            .await
            .map_err(ApiError::into_message)?;

    let mut reconciled = false;
    if resolve_reconcile_target(&account, &status).is_some() {
        if let Some(lock) = acquire_lock() {
            if let Ok(_guard) = lock.try_lock() {
                let now_ts = Local::now().timestamp();
                match persist_reconcile(status.clone(), now_ts).await {
                    Ok(Some((reconciled_date, model_id))) => {
                        record_reconcile(reconciled_date, model_id).await;
                        reconciled = true;
                    }
                    Ok(None) => {}
                    Err(err) => {
                        logger::log_warn(&format!(
                            "[WorkBuddy AI Checkin] Failed to reconcile pending status for {}: {err}",
                            account.id
                        ));
                    }
                }
            }
        }
    }

    Ok((status, reconciled))
}

pub async fn get_account_checkin_status_with_reconcile(
    account_id: &str,
) -> Result<(WorkbuddyAiCheckinStatus, bool), String> {
    let account = load_account_async(account_id).await?;
    let client = build_client()?;
    let lock_id = account_id.to_string();
    let persist_id = account_id.to_string();
    let record_id = account_id.to_string();

    get_checkin_status_and_reconcile_with_ops(
        &client,
        DEFAULT_ORIGIN,
        account,
        renew_account_session,
        move || account_lock(&ACCOUNT_CHECKIN_LOCKS, &lock_id).ok(),
        move |status, now_ts| async move {
            tauri::async_runtime::spawn_blocking(move || {
                let latest = workbuddy_ai_account::load(&persist_id)?;
                let Some((reconcile_date, last_checkin_time, streak, rewards)) =
                    build_reconcile_update(&latest, &status, now_ts)
                else {
                    return Ok(None);
                };
                let updated = workbuddy_ai_account::update_checkin_info(
                    &persist_id,
                    last_checkin_time,
                    streak,
                    Some(rewards),
                )?;
                let model = updated
                    .checkin_rewards
                    .as_ref()
                    .and_then(|v| v.get("modelId"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                Ok::<Option<(String, Option<String>)>, String>(Some((reconcile_date, model)))
            })
            .await
            .map_err(|error| error.to_string())
            .and_then(|inner| inner)
        },
        move |reconcile_date, model_id| async move {
            let _ = tauri::async_runtime::spawn_blocking(move || {
                workbuddy_ai_auto_checkin::reconcile_confirmed_status(
                    &record_id,
                    &reconcile_date,
                    model_id,
                )
            })
            .await;
        },
    )
    .await
}

pub(crate) async fn checkin_account_with_precheck<F>(
    account_id: &str,
    precheck: F,
) -> Result<GuardedCheckinOutcome, String>
where
    F: Fn() -> bool + Send + Sync + 'static,
{
    let lock = account_lock(&ACCOUNT_CHECKIN_LOCKS, account_id)?;
    let _account_guard = lock.lock().await;

    let precheck = Arc::new(precheck);
    if !precheck() {
        return Ok(GuardedCheckinOutcome::Skipped);
    }

    let account = load_account_async(account_id).await?;
    let client = build_client()?;
    let reserve_account_id = account_id.to_string();
    let release_account_id = account_id.to_string();
    let write_gate = precheck.clone();

    let mut result = match perform_checkin_with_origin_and_delays(
        &client,
        DEFAULT_ORIGIN,
        account,
        POST_REPORT_VERIFY_DELAYS,
        REPORT_TRANSIENT_RETRY_DELAYS,
        renew_account_session,
        move |activity_date, model_id| {
            let id = reserve_account_id.clone();
            async move {
                tauri::async_runtime::spawn_blocking(move || {
                    workbuddy_ai_account::mark_pending_checkin_turn(&id, &activity_date, &model_id)
                        .map(|_| ())
                })
                .await
                .map_err(|error| error.to_string())?
            }
        },
        move |activity_date| {
            let id = release_account_id.clone();
            async move {
                tauri::async_runtime::spawn_blocking(move || {
                    workbuddy_ai_account::clear_pending_checkin_turn(&id, &activity_date)
                        .map(|_| ())
                })
                .await
                .map_err(|error| error.to_string())?
            }
        },
        move || write_gate(),
    )
    .await
    {
        Ok(res) => res,
        Err(ApiError::Skipped) => return Ok(GuardedCheckinOutcome::Skipped),
        Err(err) => return Err(err.into_message()),
    };

    if result.success {
        let id = account_id.to_string();
        let streak = result.streak_days;
        let rewards = json!({
            "activityDate": result.activity_date,
            "settlement": "next_day",
            "modelId": result.model_id,
            "scoreAfter": result.score_after,
        });
        let now_ts = Local::now().timestamp();
        let persist_result = tauri::async_runtime::spawn_blocking(move || {
            workbuddy_ai_account::update_checkin_info(&id, Some(now_ts), streak, Some(rewards))
        })
        .await
        .map_err(|error| error.to_string())
        .and_then(|inner| inner);

        if let Err(err) = persist_result {
            logger::log_warn(&format!(
                "[WorkBuddy AI Checkin] Failed to persist local checkin info for {account_id}: {err}"
            ));
            append_warning(&mut result.warning, format!("local_snapshot: {err}"));
        }
    }

    Ok(GuardedCheckinOutcome::Executed(result))
}

pub async fn checkin_account(account_id: &str) -> Result<WorkbuddyAiCheckinResult, String> {
    match checkin_account_with_precheck(account_id, || true).await? {
        GuardedCheckinOutcome::Executed(result) => Ok(result),
        GuardedCheckinOutcome::Skipped => Err("WORKBUDDY_AI_CHECKIN_SKIPPED".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::State,
        http::HeaderMap,
        response::IntoResponse,
        routing::{get, post},
        Json, Router,
    };
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};

    async fn perform_checkin_with_origin_and_hooks<R, Fut, M, MFut, C, CFut, W>(
        client: &Client,
        origin: &str,
        account: WorkbuddyAccount,
        verify_delays: &[Duration],
        renewer: R,
        reserve_pending: M,
        release_pending: C,
        can_send_write: W,
    ) -> Result<WorkbuddyAiCheckinResult, ApiError>
    where
        R: FnMut(WorkbuddyAccount) -> Fut,
        Fut: Future<Output = Result<WorkbuddyAccount, String>>,
        M: Fn(String, String) -> MFut,
        MFut: Future<Output = Result<(), String>>,
        C: Fn(String) -> CFut,
        CFut: Future<Output = Result<(), String>>,
        W: Fn() -> bool,
    {
        perform_checkin_with_origin_and_delays(
            client,
            origin,
            account,
            verify_delays,
            &[Duration::ZERO, Duration::ZERO],
            renewer,
            reserve_pending,
            release_pending,
            can_send_write,
        )
        .await
    }

    fn fixture_account(enterprise_id: Option<&str>) -> WorkbuddyAccount {
        serde_json::from_value(json!({
            "id": "workbuddy_ai_0000000000000000000000000000000000000000000000000000000000000001",
            "email": "tester@example.com",
            "uid": "11111111-2222-3333-4444-555555555555",
            "enterprise_id": enterprise_id,
            "access_token": "fixture-access-token",
            "domain": DEFAULT_DOMAIN,
            "created_at": 1_700_000_000,
            "last_used": 1_700_000_000,
        }))
        .unwrap()
    }

    async fn noop_renewer(_acc: WorkbuddyAccount) -> Result<WorkbuddyAccount, String> {
        Err("WORKBUDDY_AI_TOKEN_EXPIRED".to_string())
    }

    async fn noop_mark_pending(_date: String, _model: String) -> Result<(), String> {
        Ok(())
    }

    async fn noop_release_pending(_date: String) -> Result<(), String> {
        Ok(())
    }

    fn always_allow_write() -> bool {
        true
    }

    #[derive(Clone)]
    struct MockState {
        active: Arc<AtomicBool>,
        score: Arc<AtomicI64>,
        today_date: Arc<Mutex<String>>,
        config_flip_date_to: Arc<Mutex<Option<String>>>,
        chat_flip_date_to: Arc<Mutex<Option<String>>>,
        heatmap_override: Arc<Mutex<Option<(StatusCode, String)>>>,
        heatmap_activate_on_call: Arc<AtomicUsize>,
        heatmap_calls: Arc<AtomicUsize>,
        config_calls: Arc<AtomicUsize>,
        chat_calls: Arc<AtomicUsize>,
        chat_require_token: Arc<Mutex<Option<String>>>,
        chat_override: Arc<Mutex<Option<(StatusCode, String)>>>,
        chat_delay_ms: Arc<AtomicU64>,
        chat_received_notify: Arc<tokio::sync::Notify>,
        report_calls: Arc<AtomicUsize>,
        report_fail_status: Arc<Mutex<Option<StatusCode>>>,
        report_fail_sequence: Arc<Mutex<VecDeque<StatusCode>>>,
        report_require_token: Arc<Mutex<Option<String>>>,
        report_sets_active: Arc<AtomicBool>,
        report_request_ids: Arc<Mutex<Vec<String>>>,
        report_attempt_ids: Arc<Mutex<Vec<String>>>,
        streak_fail: Arc<AtomicBool>,
        streak_require_token: Arc<Mutex<Option<String>>>,
        config_raw_override: Arc<Mutex<Option<(StatusCode, String)>>>,
        config_bytes_override: Arc<Mutex<Option<(StatusCode, Vec<u8>)>>>,
        last_report_payload: Arc<Mutex<Option<Value>>>,
        extra_active_cells: Arc<Mutex<Vec<(String, i64)>>>,
    }

    impl MockState {
        fn new(active: bool, score: i64) -> Self {
            let default_config = json!({
                "code": 0,
                "data": {
                    "models": [
                        { "id": "deepseek-v4.1-flash-sg", "name": "Deepseek-V4.1-Flash", "credits": "x0.11" },
                        { "id": "hy4flash-free", "name": "Hy4 flash", "credits": "x0.00" }
                    ]
                }
            })
            .to_string();
            Self {
                active: Arc::new(AtomicBool::new(active)),
                score: Arc::new(AtomicI64::new(score)),
                today_date: Arc::new(Mutex::new("2026-11-24".to_string())),
                config_flip_date_to: Arc::new(Mutex::new(None)),
                chat_flip_date_to: Arc::new(Mutex::new(None)),
                heatmap_override: Arc::new(Mutex::new(None)),
                heatmap_activate_on_call: Arc::new(AtomicUsize::new(0)),
                heatmap_calls: Arc::new(AtomicUsize::new(0)),
                config_calls: Arc::new(AtomicUsize::new(0)),
                chat_calls: Arc::new(AtomicUsize::new(0)),
                chat_require_token: Arc::new(Mutex::new(None)),
                chat_override: Arc::new(Mutex::new(None)),
                chat_delay_ms: Arc::new(AtomicU64::new(0)),
                chat_received_notify: Arc::new(tokio::sync::Notify::new()),
                report_calls: Arc::new(AtomicUsize::new(0)),
                report_fail_status: Arc::new(Mutex::new(None)),
                report_fail_sequence: Arc::new(Mutex::new(VecDeque::new())),
                report_require_token: Arc::new(Mutex::new(None)),
                report_sets_active: Arc::new(AtomicBool::new(true)),
                report_request_ids: Arc::new(Mutex::new(Vec::new())),
                report_attempt_ids: Arc::new(Mutex::new(Vec::new())),
                streak_fail: Arc::new(AtomicBool::new(false)),
                streak_require_token: Arc::new(Mutex::new(None)),
                config_raw_override: Arc::new(Mutex::new(Some((StatusCode::OK, default_config)))),
                config_bytes_override: Arc::new(Mutex::new(None)),
                last_report_payload: Arc::new(Mutex::new(None)),
                extra_active_cells: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    fn bearer_matches(headers: &HeaderMap, expected: &Option<String>) -> bool {
        match expected {
            None => true,
            Some(tok) => headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v == format!("Bearer {tok}")),
        }
    }

    async fn spawn_mock_server(state: MockState) -> (String, tokio::task::JoinHandle<()>) {
        let app = Router::new()
            .route(
                "/activity/growth/heatmap",
                get(|State(st): State<MockState>| async move {
                    let call_no = st.heatmap_calls.fetch_add(1, Ordering::SeqCst) + 1;
                    if let Some((status, body)) = st.heatmap_override.lock().unwrap().clone() {
                        return (status, body).into_response();
                    }
                    let activate_at = st.heatmap_activate_on_call.load(Ordering::SeqCst);
                    if activate_at > 0 && call_no >= activate_at {
                        st.active.store(true, Ordering::SeqCst);
                        if st.score.load(Ordering::SeqCst) == 0 {
                            st.score.store(2, Ordering::SeqCst);
                        }
                    }
                    let is_active = st.active.load(Ordering::SeqCst);
                    let score = st.score.load(Ordering::SeqCst);
                    let today = st.today_date.lock().unwrap().clone();
                    let mut cells = vec![
                        json!({ "date": "2026-11-22", "score": 14 }),
                        json!({ "date": "2026-11-23", "score": 2 }),
                    ];
                    for (cell_date, cell_score) in st.extra_active_cells.lock().unwrap().iter() {
                        cells.push(json!({ "date": cell_date, "score": cell_score }));
                    }
                    cells.push(json!({ "date": today, "score": score }));
                    Json(json!({
                        "code": 0,
                        "data": {
                            "today": {
                                "date": today,
                                "is_active": is_active,
                                "score": score,
                                "status_text": if is_active { "Active today" } else { "Not active yet" }
                            },
                            "cells": cells
                        }
                    }))
                    .into_response()
                }),
            )
            .route(
                "/activity/growth/streak",
                get(|State(st): State<MockState>, headers: HeaderMap| async move {
                    let expected = st.streak_require_token.lock().unwrap().clone();
                    if !bearer_matches(&headers, &expected) {
                        return (StatusCode::UNAUTHORIZED, "expired").into_response();
                    }
                    if st.streak_fail.load(Ordering::SeqCst) {
                        return (StatusCode::INTERNAL_SERVER_ERROR, "streak down").into_response();
                    }
                    Json(json!({
                        "code": 0,
                        "data": {
                            "streak": {
                                "days": 7,
                                "month_total_days": 10
                            }
                        }
                    }))
                    .into_response()
                }),
            )
            .route(
                "/v3/config",
                get(|State(st): State<MockState>| async move {
                    st.config_calls.fetch_add(1, Ordering::SeqCst);
                    if let Some(new_date) = st.config_flip_date_to.lock().unwrap().take() {
                        *st.today_date.lock().unwrap() = new_date;
                    }
                    if let Some((status, bytes)) = st.config_bytes_override.lock().unwrap().clone()
                    {
                        return (status, bytes).into_response();
                    }
                    let cfg = st.config_raw_override.lock().unwrap().clone();
                    match cfg {
                        Some((status, body)) => (status, body).into_response(),
                        None => (StatusCode::INTERNAL_SERVER_ERROR, "upstream down").into_response(),
                    }
                }),
            )
            .route(
                "/v2/chat/completions",
                post(|State(st): State<MockState>, headers: HeaderMap| async move {
                    st.chat_calls.fetch_add(1, Ordering::SeqCst);
                    if let Some(new_date) = st.chat_flip_date_to.lock().unwrap().take() {
                        *st.today_date.lock().unwrap() = new_date;
                    }
                    st.chat_received_notify.notify_waiters();
                    let delay_ms = st.chat_delay_ms.load(Ordering::SeqCst);
                    if delay_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    }
                    let expected = st.chat_require_token.lock().unwrap().clone();
                    if !bearer_matches(&headers, &expected) {
                        return (StatusCode::UNAUTHORIZED, "expired").into_response();
                    }
                    if let Some((status, body)) = st.chat_override.lock().unwrap().clone() {
                        return (status, body).into_response();
                    }
                    let sse = concat!(
                        "data: {\"id\":\"8e1539bca2d84184bf0fd369198bdbec\",\"model\":\"hy4-flash-free\",\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n",
                        "data: {\"id\":\"8e1539bca2d84184bf0fd369198bdbec\",\"model\":\"hy4-flash-free\",\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":4,\"total_tokens\":13}}\n\n",
                        "data: [DONE]\n\n"
                    );
                    (StatusCode::OK, sse).into_response()
                }),
            )
            .route(
                "/v2/report",
                post(
                    |State(st): State<MockState>, headers: HeaderMap, Json(body): Json<Value>| async move {
                        st.report_calls.fetch_add(1, Ordering::SeqCst);
                        if let Some(req_id) = headers
                            .get("x-request-id")
                            .and_then(|v| v.to_str().ok())
                        {
                            st.report_request_ids
                                .lock()
                                .unwrap()
                                .push(req_id.to_string());
                        }
                        if let Some(attempt_id) = body
                            .as_array()
                            .and_then(|arr| arr.get(1))
                            .and_then(|ev| ev["inputAttemptId"].as_str())
                        {
                            st.report_attempt_ids
                                .lock()
                                .unwrap()
                                .push(attempt_id.to_string());
                        }
                        let expected = st.report_require_token.lock().unwrap().clone();
                        if !bearer_matches(&headers, &expected) {
                            return (StatusCode::UNAUTHORIZED, "expired").into_response();
                        }
                        if let Some(status) = st.report_fail_sequence.lock().unwrap().pop_front() {
                            return (status, "transient report error").into_response();
                        }
                        if let Some(status) = *st.report_fail_status.lock().unwrap() {
                            return (status, "report error").into_response();
                        }
                        *st.last_report_payload.lock().unwrap() = Some(body);
                        if st.report_sets_active.load(Ordering::SeqCst) {
                            st.active.store(true, Ordering::SeqCst);
                            st.score.fetch_add(2, Ordering::SeqCst);
                        }
                        Json(json!({ "code": 0, "data": {} })).into_response()
                    },
                ),
            )
            .with_state(state);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), handle)
    }

    #[test]
    fn raw_token_and_model_selector_reject_underflow_duplicates_and_whitespace_ids() {
        assert!(is_zero_credit_raw_token(r#""x0.00""#));
        assert!(is_zero_credit_raw_token(r#""X0""#));
        assert!(is_zero_credit_raw_token("0"));
        assert!(is_zero_credit_raw_token("0.00"));
        assert!(!is_zero_credit_raw_token(r#""x1e-400""#));
        assert!(!is_zero_credit_raw_token(r#""-0.0""#));
        assert!(!is_zero_credit_raw_token("1e-400"));
        assert!(!is_zero_credit_raw_token("-1e-400"));
        assert!(!is_zero_credit_raw_token("-0.0"));
        assert!(!is_zero_credit_raw_token("0e0"));

        let whitespace_id = r#"{"code":0,"data":{"models":[{"id":" hy4flash-free ","name":"Hy4","credits":"x0.00"}]}}"#;
        assert_eq!(select_zero_credit_model_from_raw(whitespace_id), None);

        let dup_conflict = r#"{"code":0,"data":{"models":[
            {"id":"hy4flash-free","name":"Hy4","credits":"x0.00"},
            {"id":"hy4flash-free","name":"Hy4","credits":"x0.11"}
        ]}}"#;
        assert_eq!(select_zero_credit_model_from_raw(dup_conflict), None);

        let valid =
            r#"{"code":0,"data":{"models":[{"id":"hy4flash-free","name":"Hy4","credits":0.00}]}}"#;
        assert_eq!(
            select_zero_credit_model_from_raw(valid),
            Some(FreeModel {
                id: "hy4flash-free".to_string(),
                name: "Hy4".to_string()
            })
        );
    }

    #[test]
    fn parse_sse_supports_multiline_events_and_rejects_content_filter_or_zero_io_tokens() {
        assert!(parse_sse_metadata("").is_err());
        assert!(parse_sse_metadata(r#"{"code":500,"message":"boom"}"#).is_err());
        assert!(parse_sse_metadata("data: {\"id\":\"abc\",\"model\":\"hy4\"}\n\n").is_err());

        // Abnormal finish_reason + total_tokens-only usage must be rejected.
        let content_filter_sse = concat!(
            "data: {\"id\":\"abc\",\"model\":\"hy4\",\"choices\":[{\"finish_reason\":\"content_filter\"}]}\n\n",
            "data: {\"usage\":{\"total_tokens\":5}}\n\n",
            "data: [DONE]\n\n"
        );
        assert!(parse_sse_metadata(content_filter_sse).is_err());

        // Mid-stream 401 after content chunk must NOT return retryable Unauthorized.
        let mid_stream_401 = concat!(
            "data: {\"id\":\"abc\",\"model\":\"hy4\",\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n",
            "data: {\"code\":401,\"message\":\"expired mid-stream\"}\n\n"
        );
        let err = parse_sse_metadata(mid_stream_401).unwrap_err();
        assert!(!err.is_unauthorized());

        // Valid multi-line WHATWG SSE data event must parse cleanly.
        let multiline_sse = concat!(
            "data: {\"id\":\"cmb-0123456789abcdef0123456789abcdef\",\n",
            "data: \"model\":\"hy4-flash-free\",\"choices\":[{\"finish_reason\":\"stop\"}]}\n\n",
            "data: {\"usage\":{\"prompt_tokens\":15,\"completion_tokens\":5,\"total_tokens\":20}}\n\n",
            "data: [DONE]\n\n"
        );
        let parsed = parse_sse_metadata(multiline_sse).unwrap();
        assert_eq!(parsed.request_id, "cmb-0123456789abcdef0123456789abcdef");
        assert_eq!(parsed.response_model, "hy4-flash-free");
        assert_eq!(parsed.finish_reason, "stop");
        assert_eq!(parsed.input_tokens, 15);
        assert_eq!(parsed.output_tokens, 5);
        assert_eq!(parsed.total_tokens, 20);
    }

    #[test]
    fn parse_sse_handles_bare_cr_discards_eof_unterminated_and_rejects_premature_done() {
        // 1. Bare-CR (\r) line endings per WHATWG SSE spec must be accepted.
        let bare_cr_sse = concat!(
            "data: {\"id\":\"cr-id\",\"model\":\"hy4-flash-free\",\"choices\":[{\"finish_reason\":\"stop\"}]}\r\r",
            "data: {\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3,\"total_tokens\":10}}\r\r",
            "data: [DONE]\r\r"
        );
        let parsed = parse_sse_metadata(bare_cr_sse).unwrap();
        assert_eq!(parsed.request_id, "cr-id");
        assert_eq!(parsed.total_tokens, 10);

        // 2. EOF without terminating blank line must discard the incomplete event.
        let eof_unterminated = "data: {\"id\":\"eof-id\",\"model\":\"hy4-flash-free\",\"choices\":[{\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3,\"total_tokens\":10}}\n";
        assert!(parse_sse_metadata(eof_unterminated).is_err());

        let eof_unterminated_done = concat!(
            "data: {\"id\":\"eof-id\",\"model\":\"hy4-flash-free\",\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3,\"total_tokens\":10}}\n\n",
            "data: [DONE]\n"
        );
        assert!(parse_sse_metadata(eof_unterminated_done).is_err());

        // 3. [DONE] appearing BEFORE valid content chunks must be rejected.
        let premature_done = concat!(
            "data: [DONE]\n\n",
            "data: {\"id\":\"pre-id\",\"model\":\"hy4-flash-free\",\"choices\":[{\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3,\"total_tokens\":10}}\n\n"
        );
        assert!(parse_sse_metadata(premature_done).is_err());

        // 4. Trailing event after [DONE] must be rejected.
        let trailing_after_done = concat!(
            "data: {\"id\":\"t-id\",\"model\":\"hy4-flash-free\",\"choices\":[{\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3,\"total_tokens\":10}}\n\n",
            "data: [DONE]\n\n",
            "data: {\"id\":\"t-id\",\"choices\":[{\"delta\":{\"content\":\"extra\"}}]}\n\n"
        );
        assert!(parse_sse_metadata(trailing_after_done).is_err());

        // 5. First SSE event being 401 must NOT return retryable Unauthorized.
        let first_event_401 = "data: {\"code\":401,\"message\":\"unauthorized\"}\n\n";
        let err = parse_sse_metadata(first_event_401).unwrap_err();
        assert!(!err.is_unauthorized());
    }

    #[tokio::test]
    async fn checkin_aborts_fail_closed_on_duplicate_or_spoofed_code_and_whitespace_id() {
        let cases = vec![
            (
                StatusCode::OK,
                r#"{"code":500,"code":0,"data":{"models":[{"id":"hy4flash-free","credits":"x0.00"}]}}"#.to_string(),
            ),
            (
                StatusCode::OK,
                r#"{"code":500,"\u0063ode":0,"data":{"models":[{"id":"hy4flash-free","credits":"x0.00"}]}}"#.to_string(),
            ),
            (
                StatusCode::OK,
                r#"{"code":{"$serde_json::private::Number":"0"},"data":{"models":[{"id":"hy4flash-free","credits":"x0.00"}]}}"#.to_string(),
            ),
            (
                StatusCode::OK,
                r#"{"code":0,"data":{"models":[{"id":" hy4flash-free ","credits":"x0.00"}]}}"#.to_string(),
            ),
            (
                StatusCode::OK,
                r#"{"code":0,"data":{"models":[{"id":"hy4flash-free","credits":1e-400}]}}"#.to_string(),
            ),
            (
                StatusCode::OK,
                r#"{"code":"403","data":{"models":[{"id":"hy4flash-free","credits":"x0.00"}]}}"#.to_string(),
            ),
            (StatusCode::NO_CONTENT, String::new()),
        ];

        for (status, raw) in cases {
            let state = MockState::new(false, 0);
            *state.config_raw_override.lock().unwrap() = Some((status, raw));
            let (origin, server) = spawn_mock_server(state.clone()).await;
            let client = Client::builder().no_proxy().build().unwrap();

            let res = perform_checkin_with_origin_and_hooks(
                &client,
                &origin,
                fixture_account(None),
                &[Duration::ZERO],
                noop_renewer,
                noop_mark_pending,
                noop_release_pending,
                always_allow_write,
            )
            .await;
            assert!(res.is_err());
            assert_eq!(state.chat_calls.load(Ordering::SeqCst), 0);
            assert_eq!(state.report_calls.load(Ordering::SeqCst), 0);
            server.abort();
        }
    }

    #[tokio::test]
    async fn invalid_utf8_in_v3_config_aborts_with_zero_chat_posts() {
        let state = MockState::new(false, 0);
        let mut raw_bytes = br#"{"code":0,"data":{"models":[{"id":"free-"#.to_vec();
        raw_bytes.push(0xFF);
        raw_bytes.extend_from_slice(br#"-model","name":"Bad","credits":0}]}}"#);
        *state.config_bytes_override.lock().unwrap() = Some((StatusCode::OK, raw_bytes));
        let (origin, server) = spawn_mock_server(state.clone()).await;
        let client = Client::builder().no_proxy().build().unwrap();

        let res = perform_checkin_with_origin_and_hooks(
            &client,
            &origin,
            fixture_account(None),
            &[Duration::ZERO],
            noop_renewer,
            noop_mark_pending,
            noop_release_pending,
            always_allow_write,
        )
        .await;
        assert!(res.is_err());
        assert_eq!(state.chat_calls.load(Ordering::SeqCst), 0);
        assert_eq!(state.report_calls.load(Ordering::SeqCst), 0);
        server.abort();
    }

    #[tokio::test]
    async fn pending_persist_failure_aborts_before_chat_post() {
        let state = MockState::new(false, 0);
        let (origin, server) = spawn_mock_server(state.clone()).await;
        let client = Client::builder().no_proxy().build().unwrap();

        let res = perform_checkin_with_origin_and_hooks(
            &client,
            &origin,
            fixture_account(None),
            &[Duration::ZERO],
            noop_renewer,
            |_date, _model| async { Err("disk full".to_string()) },
            noop_release_pending,
            always_allow_write,
        )
        .await;
        assert!(res.is_err());
        assert_eq!(state.chat_calls.load(Ordering::SeqCst), 0);
        assert_eq!(state.report_calls.load(Ordering::SeqCst), 0);
        server.abort();
    }

    #[tokio::test]
    async fn mid_stream_401_and_first_event_401_do_not_replay_chat_even_with_renew_budget() {
        let payloads = vec![
            concat!(
                "data: {\"id\":\"abc\",\"model\":\"hy4\",\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n",
                "data: {\"code\":401,\"message\":\"mid-stream unauthorized\"}\n\n"
            ),
            "data: {\"code\":401,\"message\":\"first-event unauthorized\"}\n\n",
        ];

        for body in payloads {
            let dir = tempfile::tempdir().unwrap();
            let account_file = dir.path().join("acc.json");
            let initial = fixture_account(None);
            workbuddy_ai_account::write_snapshot_to_path(&account_file, &initial).unwrap();

            let state = MockState::new(false, 0);
            *state.chat_override.lock().unwrap() = Some((StatusCode::OK, body.to_string()));
            let (origin, server) = spawn_mock_server(state.clone()).await;
            let client = Client::builder().no_proxy().build().unwrap();
            let renew_calls = Arc::new(AtomicUsize::new(0));
            let rc = renew_calls.clone();
            let path_reserve = account_file.clone();
            let path_release = account_file.clone();

            let res = perform_checkin_with_origin_and_hooks(
                &client,
                &origin,
                workbuddy_ai_account::load_snapshot_from_path(&account_file).unwrap(),
                &[Duration::ZERO],
                move |mut acc| {
                    let rc = rc.clone();
                    async move {
                        rc.fetch_add(1, Ordering::SeqCst);
                        acc.access_token = "renewed".to_string();
                        Ok(acc)
                    }
                },
                move |date, model| {
                    let p = path_reserve.clone();
                    async move {
                        let mut acc = workbuddy_ai_account::load_snapshot_from_path(&p)?;
                        workbuddy_ai_account::apply_pending_checkin_marker(&mut acc, &date, &model);
                        workbuddy_ai_account::write_snapshot_to_path(&p, &acc)
                    }
                },
                move |date| {
                    let p = path_release.clone();
                    async move {
                        let mut acc = workbuddy_ai_account::load_snapshot_from_path(&p)?;
                        workbuddy_ai_account::clear_pending_checkin_marker(&mut acc, &date);
                        workbuddy_ai_account::write_snapshot_to_path(&p, &acc)
                    }
                },
                always_allow_write,
            )
            .await;
            assert!(res.is_err());
            assert_eq!(renew_calls.load(Ordering::SeqCst), 0);
            assert_eq!(state.chat_calls.load(Ordering::SeqCst), 1);

            // Reload from real disk file: pending marker must block any second chat POST.
            let reloaded = workbuddy_ai_account::load_snapshot_from_path(&account_file).unwrap();
            let second = perform_checkin_with_origin_and_hooks(
                &client,
                &origin,
                reloaded,
                &[Duration::ZERO],
                noop_renewer,
                noop_mark_pending,
                noop_release_pending,
                always_allow_write,
            )
            .await
            .unwrap();
            assert!(second.pending_confirmation);
            assert_eq!(state.chat_calls.load(Ordering::SeqCst), 1);
            server.abort();
        }
    }

    #[tokio::test]
    async fn http_502_keeps_disk_reservation_and_reload_emits_zero_extra_chat() {
        let dir = tempfile::tempdir().unwrap();
        let account_file = dir.path().join("acc502.json");
        workbuddy_ai_account::write_snapshot_to_path(&account_file, &fixture_account(None))
            .unwrap();

        let state = MockState::new(false, 0);
        *state.chat_override.lock().unwrap() =
            Some((StatusCode::BAD_GATEWAY, "bad gateway".to_string()));
        let (origin, server) = spawn_mock_server(state.clone()).await;
        let client = Client::builder().no_proxy().build().unwrap();
        let p_res = account_file.clone();
        let p_rel = account_file.clone();

        let first = perform_checkin_with_origin_and_hooks(
            &client,
            &origin,
            workbuddy_ai_account::load_snapshot_from_path(&account_file).unwrap(),
            &[Duration::ZERO],
            noop_renewer,
            move |date, model| {
                let p = p_res.clone();
                async move {
                    let mut acc = workbuddy_ai_account::load_snapshot_from_path(&p)?;
                    workbuddy_ai_account::apply_pending_checkin_marker(&mut acc, &date, &model);
                    workbuddy_ai_account::write_snapshot_to_path(&p, &acc)
                }
            },
            move |date| {
                let p = p_rel.clone();
                async move {
                    let mut acc = workbuddy_ai_account::load_snapshot_from_path(&p)?;
                    workbuddy_ai_account::clear_pending_checkin_marker(&mut acc, &date);
                    workbuddy_ai_account::write_snapshot_to_path(&p, &acc)
                }
            },
            always_allow_write,
        )
        .await;
        assert!(first.is_err());
        assert_eq!(state.chat_calls.load(Ordering::SeqCst), 1);

        // Reload from disk (simulating restart): reservation persisted -> 0 extra chat POSTs.
        let reloaded = workbuddy_ai_account::load_snapshot_from_path(&account_file).unwrap();
        assert!(workbuddy_ai_account::has_pending_checkin_turn(
            &reloaded,
            "2026-11-24"
        ));
        let second = perform_checkin_with_origin_and_hooks(
            &client,
            &origin,
            reloaded,
            &[Duration::ZERO],
            noop_renewer,
            noop_mark_pending,
            noop_release_pending,
            always_allow_write,
        )
        .await
        .unwrap();
        assert!(second.pending_confirmation);
        assert_eq!(state.chat_calls.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn early_exit_after_chat_post_leaves_disk_reservation_blocking_restart_replay() {
        let dir = tempfile::tempdir().unwrap();
        let account_file = dir.path().join("acc_crash.json");
        workbuddy_ai_account::write_snapshot_to_path(&account_file, &fixture_account(None))
            .unwrap();

        let state = MockState::new(false, 0);
        state.chat_delay_ms.store(5_000, Ordering::SeqCst);
        let notify = state.chat_received_notify.clone();
        let (origin, server) = spawn_mock_server(state.clone()).await;
        let client = Client::builder().no_proxy().build().unwrap();
        let p_res = account_file.clone();
        let p_rel = account_file.clone();

        let handle = tokio::spawn({
            let client = client.clone();
            let origin = origin.clone();
            let acc = workbuddy_ai_account::load_snapshot_from_path(&account_file).unwrap();
            async move {
                let _ = perform_checkin_with_origin_and_hooks(
                    &client,
                    &origin,
                    acc,
                    &[Duration::ZERO],
                    noop_renewer,
                    move |date, model| {
                        let p = p_res.clone();
                        async move {
                            let mut acc = workbuddy_ai_account::load_snapshot_from_path(&p)?;
                            workbuddy_ai_account::apply_pending_checkin_marker(
                                &mut acc, &date, &model,
                            );
                            workbuddy_ai_account::write_snapshot_to_path(&p, &acc)
                        }
                    },
                    move |date| {
                        let p = p_rel.clone();
                        async move {
                            let mut acc = workbuddy_ai_account::load_snapshot_from_path(&p)?;
                            workbuddy_ai_account::clear_pending_checkin_marker(&mut acc, &date);
                            workbuddy_ai_account::write_snapshot_to_path(&p, &acc)
                        }
                    },
                    always_allow_write,
                )
                .await;
            }
        });

        // Abort the task immediately once POST reaches server (before HTTP 200 body returns).
        notify.notified().await;
        handle.abort();
        let _ = handle.await;
        assert_eq!(state.chat_calls.load(Ordering::SeqCst), 1);

        // Restart: reload encrypted snapshot from disk -> write-ahead marker already present.
        state.chat_delay_ms.store(0, Ordering::SeqCst);
        let reloaded = workbuddy_ai_account::load_snapshot_from_path(&account_file).unwrap();
        let after_restart = perform_checkin_with_origin_and_hooks(
            &client,
            &origin,
            reloaded,
            &[Duration::ZERO],
            noop_renewer,
            noop_mark_pending,
            noop_release_pending,
            always_allow_write,
        )
        .await
        .unwrap();
        assert!(after_restart.pending_confirmation);
        assert_eq!(state.chat_calls.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn confirmed_date_on_disk_blocks_replay_when_heatmap_returns_stale_inactive() {
        let dir = tempfile::tempdir().unwrap();
        let account_file = dir.path().join("acc_stale.json");
        workbuddy_ai_account::write_snapshot_to_path(&account_file, &fixture_account(None))
            .unwrap();

        let state = MockState::new(false, 0);
        let (origin, server) = spawn_mock_server(state.clone()).await;
        let client = Client::builder().no_proxy().build().unwrap();
        let p_res = account_file.clone();
        let p_rel = account_file.clone();

        // Call 1: succeeds and persists confirmed activityDate (clearing pendingActivityDate).
        let first = perform_checkin_with_origin_and_hooks(
            &client,
            &origin,
            workbuddy_ai_account::load_snapshot_from_path(&account_file).unwrap(),
            &[Duration::ZERO],
            noop_renewer,
            move |date, model| {
                let p = p_res.clone();
                async move {
                    let mut acc = workbuddy_ai_account::load_snapshot_from_path(&p)?;
                    workbuddy_ai_account::apply_pending_checkin_marker(&mut acc, &date, &model);
                    workbuddy_ai_account::write_snapshot_to_path(&p, &acc)
                }
            },
            move |date| {
                let p = p_rel.clone();
                async move {
                    let mut acc = workbuddy_ai_account::load_snapshot_from_path(&p)?;
                    workbuddy_ai_account::clear_pending_checkin_marker(&mut acc, &date);
                    workbuddy_ai_account::write_snapshot_to_path(&p, &acc)
                }
            },
            always_allow_write,
        )
        .await
        .unwrap();
        assert!(first.success);
        assert_eq!(state.chat_calls.load(Ordering::SeqCst), 1);

        let mut confirmed = workbuddy_ai_account::load_snapshot_from_path(&account_file).unwrap();
        workbuddy_ai_account::apply_checkin_info_update(
            &mut confirmed,
            Some(1_700_000_500),
            first.streak_days,
            Some(json!({
                "activityDate": first.activity_date,
                "modelId": first.model_id,
                "scoreAfter": first.score_after,
            })),
        );
        workbuddy_ai_account::write_snapshot_to_path(&account_file, &confirmed).unwrap();

        // Server heatmap returns stale inactive on the SAME day after confirmation.
        state.active.store(false, Ordering::SeqCst);
        state.score.store(0, Ordering::SeqCst);

        let reloaded = workbuddy_ai_account::load_snapshot_from_path(&account_file).unwrap();
        let status = get_checkin_status_with_origin_and_renewer(
            &client,
            &origin,
            reloaded.clone(),
            noop_renewer,
        )
        .await
        .unwrap();
        assert!(!status.today_checked_in);
        assert!(status.pending_confirmation);

        let second = perform_checkin_with_origin_and_hooks(
            &client,
            &origin,
            reloaded,
            &[Duration::ZERO],
            noop_renewer,
            noop_mark_pending,
            noop_release_pending,
            always_allow_write,
        )
        .await
        .unwrap();
        assert!(!second.success);
        assert!(second.pending_confirmation);
        assert_eq!(state.chat_calls.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn pre_write_gate_false_after_config_aborts_with_zero_chat_and_zero_reservation() {
        let state = MockState::new(false, 0);
        let (origin, server) = spawn_mock_server(state.clone()).await;
        let client = Client::builder().no_proxy().build().unwrap();
        let reserve_calls = Arc::new(AtomicUsize::new(0));
        let rc = reserve_calls.clone();

        let res = perform_checkin_with_origin_and_hooks(
            &client,
            &origin,
            fixture_account(None),
            &[Duration::ZERO],
            noop_renewer,
            move |_date, _model| {
                let rc = rc.clone();
                async move {
                    rc.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            },
            noop_release_pending,
            || false,
        )
        .await;
        assert!(matches!(res, Err(ApiError::Skipped)));
        assert_eq!(state.config_calls.load(Ordering::SeqCst), 1);
        assert_eq!(reserve_calls.load(Ordering::SeqCst), 0);
        assert_eq!(state.chat_calls.load(Ordering::SeqCst), 0);
        server.abort();
    }

    #[tokio::test]
    async fn chat_http_401_renews_and_revalidates_v3_config_before_retrying_chat() {
        let state = MockState::new(false, 0);
        *state.chat_require_token.lock().unwrap() = Some("renewed-token".to_string());
        let (origin, server) = spawn_mock_server(state.clone()).await;
        let client = Client::builder().no_proxy().build().unwrap();
        let released = Arc::new(AtomicUsize::new(0));
        let rel = released.clone();

        let result = perform_checkin_with_origin_and_hooks(
            &client,
            &origin,
            fixture_account(None),
            &[Duration::ZERO],
            |mut acc| async move {
                acc.access_token = "renewed-token".to_string();
                Ok(acc)
            },
            noop_mark_pending,
            move |_date| {
                let rel = rel.clone();
                async move {
                    rel.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            },
            always_allow_write,
        )
        .await
        .unwrap();

        assert!(result.success);
        assert_eq!(released.load(Ordering::SeqCst), 1);
        assert_eq!(state.config_calls.load(Ordering::SeqCst), 2);
        assert_eq!(state.chat_calls.load(Ordering::SeqCst), 2);
        assert_eq!(state.report_calls.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn report_401_retries_with_identical_request_id_and_attempt_id_without_replaying_chat() {
        let state = MockState::new(false, 0);
        *state.report_require_token.lock().unwrap() = Some("renewed-token".to_string());
        let (origin, server) = spawn_mock_server(state.clone()).await;
        let client = Client::builder().no_proxy().build().unwrap();

        let result = perform_checkin_with_origin_and_hooks(
            &client,
            &origin,
            fixture_account(None),
            &[Duration::ZERO],
            |mut acc| async move {
                acc.access_token = "renewed-token".to_string();
                Ok(acc)
            },
            noop_mark_pending,
            noop_release_pending,
            always_allow_write,
        )
        .await
        .unwrap();

        assert!(result.success);
        assert_eq!(state.chat_calls.load(Ordering::SeqCst), 1);
        assert_eq!(state.report_calls.load(Ordering::SeqCst), 2);
        let req_ids = state.report_request_ids.lock().unwrap();
        let attempt_ids = state.report_attempt_ids.lock().unwrap();
        assert_eq!(req_ids.len(), 2);
        assert_eq!(req_ids[0], req_ids[1]);
        assert_eq!(attempt_ids.len(), 2);
        assert_eq!(attempt_ids[0], attempt_ids[1]);
        server.abort();
    }

    #[tokio::test]
    async fn pending_guard_prevents_second_chat_after_report_failure_and_converges_on_activation() {
        let dir = tempfile::tempdir().unwrap();
        let account_file = dir.path().join("acc_report_504.json");
        workbuddy_ai_account::write_snapshot_to_path(&account_file, &fixture_account(None))
            .unwrap();

        let state = MockState::new(false, 0);
        *state.report_fail_status.lock().unwrap() = Some(StatusCode::GATEWAY_TIMEOUT);
        let (origin, server) = spawn_mock_server(state.clone()).await;
        let client = Client::builder().no_proxy().build().unwrap();
        let p_res = account_file.clone();
        let p_rel = account_file.clone();

        // Call 1: chat succeeds (1 chat POST), report returns 504 -> real disk file has pending marker.
        let first = perform_checkin_with_origin_and_hooks(
            &client,
            &origin,
            workbuddy_ai_account::load_snapshot_from_path(&account_file).unwrap(),
            &[Duration::ZERO],
            noop_renewer,
            move |date, model| {
                let p = p_res.clone();
                async move {
                    let mut acc = workbuddy_ai_account::load_snapshot_from_path(&p)?;
                    workbuddy_ai_account::apply_pending_checkin_marker(&mut acc, &date, &model);
                    workbuddy_ai_account::write_snapshot_to_path(&p, &acc)
                }
            },
            move |date| {
                let p = p_rel.clone();
                async move {
                    let mut acc = workbuddy_ai_account::load_snapshot_from_path(&p)?;
                    workbuddy_ai_account::clear_pending_checkin_marker(&mut acc, &date);
                    workbuddy_ai_account::write_snapshot_to_path(&p, &acc)
                }
            },
            always_allow_write,
        )
        .await;
        assert!(first.is_err());
        assert_eq!(state.chat_calls.load(Ordering::SeqCst), 1);

        // Call 2: reload from real disk file (simulating restart) -> stays read-only pending.
        let reloaded = workbuddy_ai_account::load_snapshot_from_path(&account_file).unwrap();
        assert!(workbuddy_ai_account::has_pending_checkin_turn(
            &reloaded,
            "2026-11-24"
        ));
        let second = perform_checkin_with_origin_and_hooks(
            &client,
            &origin,
            reloaded,
            &[Duration::ZERO],
            noop_renewer,
            noop_mark_pending,
            noop_release_pending,
            always_allow_write,
        )
        .await
        .unwrap();
        assert!(!second.success);
        assert!(second.pending_confirmation);
        assert_eq!(state.chat_calls.load(Ordering::SeqCst), 1);

        // Call 3: heatmap flips active -> converges to already_claimed with still 1 total chat POST.
        state.active.store(true, Ordering::SeqCst);
        state.score.store(2, Ordering::SeqCst);
        let reloaded2 = workbuddy_ai_account::load_snapshot_from_path(&account_file).unwrap();
        let third = perform_checkin_with_origin_and_hooks(
            &client,
            &origin,
            reloaded2,
            &[Duration::ZERO],
            noop_renewer,
            noop_mark_pending,
            noop_release_pending,
            always_allow_write,
        )
        .await
        .unwrap();
        assert!(third.success);
        assert!(third.already_claimed);
        assert_eq!(state.chat_calls.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn singleflight_renewal_shares_one_call_across_concurrent_callers() {
        let map: Arc<
            Mutex<HashMap<String, tokio::sync::watch::Receiver<Option<SharedRenewResult>>>>,
        > = Arc::new(Mutex::new(HashMap::new()));
        let renew_calls = Arc::new(AtomicUsize::new(0));
        let stored = Arc::new(Mutex::new(fixture_account(None)));

        let run_one = || {
            let map = map.clone();
            let renew_calls = renew_calls.clone();
            let stored = stored.clone();
            async move {
                renew_account_session_with_ops(
                    &map,
                    "acc-1",
                    "fixture-access-token",
                    {
                        let stored = stored.clone();
                        move |_| {
                            let stored = stored.clone();
                            async move { Ok(stored.lock().unwrap().clone()) }
                        }
                    },
                    move |mut acc| {
                        let renew_calls = renew_calls.clone();
                        let stored = stored.clone();
                        async move {
                            renew_calls.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(60)).await;
                            acc.access_token = "shared-renewed-token".to_string();
                            *stored.lock().unwrap() = acc.clone();
                            Ok(acc)
                        }
                    },
                )
                .await
            }
        };

        let (r1, r2) = tokio::join!(run_one(), run_one());
        assert_eq!(r1.unwrap().access_token, "shared-renewed-token");
        assert_eq!(r2.unwrap().access_token, "shared-renewed-token");
        assert_eq!(renew_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn status_reads_heatmap_streak_and_reports_pending_confirmation() {
        let state = MockState::new(false, 0);
        let (origin, server) = spawn_mock_server(state).await;
        let client = Client::builder().no_proxy().build().unwrap();
        let mut account = fixture_account(None);
        account.checkin_rewards = Some(json!({ "pendingActivityDate": "2026-11-24" }));

        let status =
            get_checkin_status_with_origin_and_renewer(&client, &origin, account, noop_renewer)
                .await
                .unwrap();
        assert!(status.active);
        assert!(!status.today_checked_in);
        assert!(status.pending_confirmation);
        assert_eq!(status.streak_days, 7);
        assert_eq!(status.daily_credit, 0);
        assert_eq!(status.today_date.as_deref(), Some("2026-11-24"));
        server.abort();
    }

    #[tokio::test]
    async fn heatmap_rejects_invalid_date_format_without_chat() {
        let state = MockState::new(false, 0);
        *state.heatmap_override.lock().unwrap() = Some((
            StatusCode::OK,
            r#"{"code":0,"data":{"today":{"date":"not-a-date","is_active":false,"score":0}}}"#
                .to_string(),
        ));
        let (origin, server) = spawn_mock_server(state.clone()).await;
        let client = Client::builder().no_proxy().build().unwrap();

        let res = perform_checkin_with_origin_and_hooks(
            &client,
            &origin,
            fixture_account(None),
            &[Duration::ZERO],
            noop_renewer,
            noop_mark_pending,
            noop_release_pending,
            always_allow_write,
        )
        .await;
        assert!(res.is_err());
        assert_eq!(state.chat_calls.load(Ordering::SeqCst), 0);
        server.abort();
    }

    #[tokio::test]
    async fn delayed_heatmap_confirms_on_second_poll_and_never_replays_chat() {
        let state = MockState::new(false, 0);
        state.report_sets_active.store(false, Ordering::SeqCst);
        // Call 1: initial precheck, Call 2: pre-write date recheck, Call 3: verify poll #1, Call 4: verify poll #2 (active).
        state.heatmap_activate_on_call.store(4, Ordering::SeqCst);
        let (origin, server) = spawn_mock_server(state.clone()).await;
        let client = Client::builder().no_proxy().build().unwrap();

        let result = perform_checkin_with_origin_and_hooks(
            &client,
            &origin,
            fixture_account(None),
            &[Duration::ZERO, Duration::ZERO, Duration::ZERO],
            noop_renewer,
            noop_mark_pending,
            noop_release_pending,
            always_allow_write,
        )
        .await
        .unwrap();

        assert!(result.success);
        assert!(!result.pending_confirmation);
        assert_eq!(state.chat_calls.load(Ordering::SeqCst), 1);
        assert_eq!(state.report_calls.load(Ordering::SeqCst), 1);
        assert_eq!(state.heatmap_calls.load(Ordering::SeqCst), 4);
        server.abort();
    }

    #[tokio::test]
    async fn midnight_rollover_during_config_reserves_new_date_and_blocks_second_chat_after_report_502(
    ) {
        let dir = tempfile::tempdir().unwrap();
        let account_file = dir.path().join("acc_rollover_config.json");
        workbuddy_ai_account::write_snapshot_to_path(&account_file, &fixture_account(None))
            .unwrap();

        let state = MockState::new(false, 0);
        // Server starts on 2026-11-24, crosses midnight to 2026-11-25 while /v3/config is fetched,
        // and /v2/report returns 502.
        *state.config_flip_date_to.lock().unwrap() = Some("2026-11-25".to_string());
        *state.report_fail_status.lock().unwrap() = Some(StatusCode::BAD_GATEWAY);
        let (origin, server) = spawn_mock_server(state.clone()).await;
        let client = Client::builder().no_proxy().build().unwrap();
        let p_res = account_file.clone();
        let p_rel = account_file.clone();

        let first = perform_checkin_with_origin_and_hooks(
            &client,
            &origin,
            workbuddy_ai_account::load_snapshot_from_path(&account_file).unwrap(),
            &[Duration::ZERO],
            noop_renewer,
            move |date, model| {
                let p = p_res.clone();
                async move {
                    let mut acc = workbuddy_ai_account::load_snapshot_from_path(&p)?;
                    workbuddy_ai_account::apply_pending_checkin_marker(&mut acc, &date, &model);
                    workbuddy_ai_account::write_snapshot_to_path(&p, &acc)
                }
            },
            move |date| {
                let p = p_rel.clone();
                async move {
                    let mut acc = workbuddy_ai_account::load_snapshot_from_path(&p)?;
                    workbuddy_ai_account::clear_pending_checkin_marker(&mut acc, &date);
                    workbuddy_ai_account::write_snapshot_to_path(&p, &acc)
                }
            },
            always_allow_write,
        )
        .await;
        assert!(first.is_err());
        assert_eq!(state.chat_calls.load(Ordering::SeqCst), 1);

        // Pre-write date recheck must have persisted reservation under 2026-11-25 (not 2026-11-24).
        let reloaded = workbuddy_ai_account::load_snapshot_from_path(&account_file).unwrap();
        assert!(workbuddy_ai_account::has_pending_checkin_turn(
            &reloaded,
            "2026-11-25"
        ));

        // Reloading from disk on 2026-11-25 must stay read-only pending with total 1 chat POST.
        let second = perform_checkin_with_origin_and_hooks(
            &client,
            &origin,
            reloaded,
            &[Duration::ZERO],
            noop_renewer,
            noop_mark_pending,
            noop_release_pending,
            always_allow_write,
        )
        .await
        .unwrap();
        assert!(!second.success);
        assert!(second.pending_confirmation);
        assert_eq!(second.activity_date.as_deref(), Some("2026-11-25"));
        assert_eq!(state.chat_calls.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn in_flight_midnight_rollover_during_chat_blocks_second_chat_after_report_502() {
        let dir = tempfile::tempdir().unwrap();
        let account_file = dir.path().join("acc_rollover_inflight.json");
        workbuddy_ai_account::write_snapshot_to_path(&account_file, &fixture_account(None))
            .unwrap();

        let state = MockState::new(false, 0);
        // Server crosses midnight AFTER pre-write heatmap check, right while chat POST arrives.
        *state.chat_flip_date_to.lock().unwrap() = Some("2026-11-25".to_string());
        *state.report_fail_status.lock().unwrap() = Some(StatusCode::BAD_GATEWAY);
        let (origin, server) = spawn_mock_server(state.clone()).await;
        let client = Client::builder().no_proxy().build().unwrap();
        let p_res = account_file.clone();
        let p_rel = account_file.clone();

        let first = perform_checkin_with_origin_and_hooks(
            &client,
            &origin,
            workbuddy_ai_account::load_snapshot_from_path(&account_file).unwrap(),
            &[Duration::ZERO],
            noop_renewer,
            move |date, model| {
                let p = p_res.clone();
                async move {
                    let mut acc = workbuddy_ai_account::load_snapshot_from_path(&p)?;
                    workbuddy_ai_account::apply_pending_checkin_marker(&mut acc, &date, &model);
                    workbuddy_ai_account::write_snapshot_to_path(&p, &acc)
                }
            },
            move |date| {
                let p = p_rel.clone();
                async move {
                    let mut acc = workbuddy_ai_account::load_snapshot_from_path(&p)?;
                    workbuddy_ai_account::clear_pending_checkin_marker(&mut acc, &date);
                    workbuddy_ai_account::write_snapshot_to_path(&p, &acc)
                }
            },
            always_allow_write,
        )
        .await;
        assert!(first.is_err());
        assert_eq!(state.chat_calls.load(Ordering::SeqCst), 1);

        // Disk has unresolved recent 2026-11-24 reservation while server is now 2026-11-25 inactive
        // and 2026-11-24 is absent from active cells -> must stay read-only pending with 1 chat POST.
        let reloaded = workbuddy_ai_account::load_snapshot_from_path(&account_file).unwrap();
        let second = perform_checkin_with_origin_and_hooks(
            &client,
            &origin,
            reloaded.clone(),
            &[Duration::ZERO],
            noop_renewer,
            noop_mark_pending,
            noop_release_pending,
            always_allow_write,
        )
        .await
        .unwrap();
        assert!(!second.success);
        assert!(second.pending_confirmation);
        assert_eq!(state.chat_calls.load(Ordering::SeqCst), 1);

        // Confirm that an old stale pending reservation (> 1h ago) does NOT block the next day.
        let mut stale = reloaded;
        stale.checkin_rewards.as_mut().unwrap()["pendingCreatedAt"] =
            json!(chrono::Utc::now().timestamp() - 7200);
        assert!(!workbuddy_ai_account::has_recorded_checkin_for_heatmap(
            &stale,
            "2026-11-25",
            &["2026-11-23".to_string()]
        ));
        server.abort();
    }

    #[tokio::test]
    async fn post_reserve_write_gate_false_releases_unsent_reservation_with_zero_chat() {
        let dir = tempfile::tempdir().unwrap();
        let account_file = dir.path().join("acc_post_reserve_gate.json");
        workbuddy_ai_account::write_snapshot_to_path(&account_file, &fixture_account(None))
            .unwrap();

        let state = MockState::new(false, 0);
        let (origin, server) = spawn_mock_server(state.clone()).await;
        let client = Client::builder().no_proxy().build().unwrap();
        let gate = Arc::new(AtomicBool::new(true));
        let gate_in_reserve = gate.clone();
        let gate_check = gate.clone();
        let p_res = account_file.clone();
        let p_rel = account_file.clone();

        let res = perform_checkin_with_origin_and_hooks(
            &client,
            &origin,
            workbuddy_ai_account::load_snapshot_from_path(&account_file).unwrap(),
            &[Duration::ZERO],
            noop_renewer,
            move |date, model| {
                let p = p_res.clone();
                let g = gate_in_reserve.clone();
                async move {
                    let mut acc = workbuddy_ai_account::load_snapshot_from_path(&p)?;
                    workbuddy_ai_account::apply_pending_checkin_marker(&mut acc, &date, &model);
                    workbuddy_ai_account::write_snapshot_to_path(&p, &acc)?;
                    // Simulate window cutoff or user disable completing during async WAL persist.
                    g.store(false, Ordering::SeqCst);
                    Ok(())
                }
            },
            move |date| {
                let p = p_rel.clone();
                async move {
                    let mut acc = workbuddy_ai_account::load_snapshot_from_path(&p)?;
                    workbuddy_ai_account::clear_pending_checkin_marker(&mut acc, &date);
                    workbuddy_ai_account::write_snapshot_to_path(&p, &acc)
                }
            },
            move || gate_check.load(Ordering::SeqCst),
        )
        .await;
        assert!(matches!(res, Err(ApiError::Skipped)));
        assert_eq!(state.chat_calls.load(Ordering::SeqCst), 0);

        // Unsent reservation must have been released cleanly from disk.
        let reloaded = workbuddy_ai_account::load_snapshot_from_path(&account_file).unwrap();
        assert!(!workbuddy_ai_account::has_pending_checkin_turn(
            &reloaded,
            "2026-11-24"
        ));
        server.abort();
    }

    #[tokio::test]
    async fn streak_failure_preserves_none_instead_of_zero_and_records_warning() {
        let state = MockState::new(false, 0);
        state.streak_fail.store(true, Ordering::SeqCst);
        let (origin, server) = spawn_mock_server(state.clone()).await;
        let client = Client::builder().no_proxy().build().unwrap();

        let result = perform_checkin_with_origin_and_hooks(
            &client,
            &origin,
            fixture_account(None),
            &[Duration::ZERO],
            noop_renewer,
            noop_mark_pending,
            noop_release_pending,
            always_allow_write,
        )
        .await
        .unwrap();

        assert!(result.success);
        assert_eq!(result.streak_days, None);
        assert!(result
            .warning
            .as_deref()
            .is_some_and(|w| w.contains("streak:")));
        server.abort();
    }

    #[tokio::test]
    async fn enterprise_account_returns_inactive_without_network_calls() {
        let state = MockState::new(false, 0);
        let (origin, server) = spawn_mock_server(state.clone()).await;
        let client = Client::builder().no_proxy().build().unwrap();

        let result = perform_checkin_with_origin_and_hooks(
            &client,
            &origin,
            fixture_account(Some("ent-123")),
            &[Duration::ZERO],
            noop_renewer,
            noop_mark_pending,
            noop_release_pending,
            always_allow_write,
        )
        .await
        .unwrap();
        assert!(!result.success);
        assert!(result.inactive);
        assert_eq!(state.chat_calls.load(Ordering::SeqCst), 0);
        server.abort();
    }

    #[test]
    fn permanent_error_classification_covers_auth_and_refresh_rejection() {
        assert!(is_permanent_error("WORKBUDDY_AI_TOKEN_EXPIRED"));
        assert!(is_permanent_error("WORKBUDDY_AI_MISSING_SESSION"));
        assert!(is_permanent_error("growth/heatmap: HTTP 401"));
        assert!(is_permanent_error("v2/report: HTTP 403"));
        assert!(is_permanent_error("token/refresh: HTTP 400 invalid"));
        assert!(is_permanent_error(
            "WorkBuddy AI HTTP 400 Bad Request, code 400"
        ));
        assert!(is_permanent_error(
            "WORKBUDDY_AI_REFRESH_REJECTED: WorkBuddy AI HTTP 400 Bad Request, code 400"
        ));
        assert!(!is_permanent_error("v2/report: HTTP 408"));
        assert!(!is_permanent_error("v2/report: HTTP 429"));
        assert!(!is_permanent_error("v2/report: HTTP 502"));
        assert!(!is_permanent_error("growth/heatmap: connection timed out"));
    }

    #[tokio::test]
    async fn report_transient_502_recovers_on_retry_with_identical_ids_and_zero_chat_replay() {
        let state = MockState::new(false, 0);
        state
            .report_fail_sequence
            .lock()
            .unwrap()
            .push_back(StatusCode::BAD_GATEWAY);
        let (origin, server) = spawn_mock_server(state.clone()).await;
        let client = Client::builder().no_proxy().build().unwrap();

        let result = perform_checkin_with_origin_and_hooks(
            &client,
            &origin,
            fixture_account(None),
            &[Duration::ZERO],
            noop_renewer,
            noop_mark_pending,
            noop_release_pending,
            always_allow_write,
        )
        .await
        .unwrap();

        assert!(result.success);
        assert!(!result.pending_confirmation);
        assert_eq!(result.result_code.as_deref(), Some("activated"));
        assert_eq!(result.model_id.as_deref(), Some("hy4flash-free"));
        assert_eq!(state.chat_calls.load(Ordering::SeqCst), 1);
        assert_eq!(state.report_calls.load(Ordering::SeqCst), 2);
        let req_ids = state.report_request_ids.lock().unwrap();
        let attempt_ids = state.report_attempt_ids.lock().unwrap();
        assert_eq!(req_ids.len(), 2);
        assert_eq!(req_ids[0], req_ids[1]);
        assert_eq!(attempt_ids.len(), 2);
        assert_eq!(attempt_ids[0], attempt_ids[1]);
        server.abort();
    }

    #[tokio::test]
    async fn report_retry_handles_408_429_401_bounds_503_and_never_retries_permanent_errors() {
        // 1. Permanent HTTP 400 and 403 must fail immediately on the first report attempt.
        for permanent_status in [StatusCode::BAD_REQUEST, StatusCode::FORBIDDEN] {
            let state = MockState::new(false, 0);
            *state.report_fail_status.lock().unwrap() = Some(permanent_status);
            let (origin, server) = spawn_mock_server(state.clone()).await;
            let client = Client::builder().no_proxy().build().unwrap();

            let res = perform_checkin_with_origin_and_hooks(
                &client,
                &origin,
                fixture_account(None),
                &[Duration::ZERO],
                noop_renewer,
                noop_mark_pending,
                noop_release_pending,
                always_allow_write,
            )
            .await;
            assert!(res.is_err());
            assert_eq!(state.chat_calls.load(Ordering::SeqCst), 1);
            assert_eq!(state.report_calls.load(Ordering::SeqCst), 1);
            server.abort();
        }

        // 2. Persistent 503 stops after 3 total attempts (1 initial + 2 retries) with identical IDs.
        {
            let state = MockState::new(false, 0);
            *state.report_fail_status.lock().unwrap() = Some(StatusCode::SERVICE_UNAVAILABLE);
            let (origin, server) = spawn_mock_server(state.clone()).await;
            let client = Client::builder().no_proxy().build().unwrap();

            let res = perform_checkin_with_origin_and_hooks(
                &client,
                &origin,
                fixture_account(None),
                &[Duration::ZERO],
                noop_renewer,
                noop_mark_pending,
                noop_release_pending,
                always_allow_write,
            )
            .await;
            assert!(res.is_err());
            assert_eq!(state.chat_calls.load(Ordering::SeqCst), 1);
            assert_eq!(state.report_calls.load(Ordering::SeqCst), 3);
            let req_ids = state.report_request_ids.lock().unwrap();
            let attempt_ids = state.report_attempt_ids.lock().unwrap();
            assert_eq!(req_ids.len(), 3);
            assert_eq!(req_ids[0], req_ids[1]);
            assert_eq!(req_ids[1], req_ids[2]);
            assert_eq!(attempt_ids.len(), 3);
            assert_eq!(attempt_ids[0], attempt_ids[1]);
            assert_eq!(attempt_ids[1], attempt_ids[2]);
            server.abort();
        }

        // 3. Transient 408 -> 429 -> 200 succeeds on the 3rd attempt with 1 chat POST.
        {
            let state = MockState::new(false, 0);
            *state.report_fail_sequence.lock().unwrap() =
                VecDeque::from([StatusCode::REQUEST_TIMEOUT, StatusCode::TOO_MANY_REQUESTS]);
            let (origin, server) = spawn_mock_server(state.clone()).await;
            let client = Client::builder().no_proxy().build().unwrap();

            let res = perform_checkin_with_origin_and_hooks(
                &client,
                &origin,
                fixture_account(None),
                &[Duration::ZERO],
                noop_renewer,
                noop_mark_pending,
                noop_release_pending,
                always_allow_write,
            )
            .await
            .unwrap();
            assert!(res.success);
            assert_eq!(state.chat_calls.load(Ordering::SeqCst), 1);
            assert_eq!(state.report_calls.load(Ordering::SeqCst), 3);
            let req_ids = state.report_request_ids.lock().unwrap();
            assert_eq!(req_ids.len(), 3);
            assert_eq!(req_ids[0], req_ids[1]);
            assert_eq!(req_ids[1], req_ids[2]);
            server.abort();
        }

        // 4. 401 refresh followed by transient 502 recovers on retry with identical report IDs.
        {
            let state = MockState::new(false, 0);
            *state.report_require_token.lock().unwrap() = Some("renewed-token".to_string());
            state
                .report_fail_sequence
                .lock()
                .unwrap()
                .push_back(StatusCode::BAD_GATEWAY);
            let (origin, server) = spawn_mock_server(state.clone()).await;
            let client = Client::builder().no_proxy().build().unwrap();

            let res = perform_checkin_with_origin_and_hooks(
                &client,
                &origin,
                fixture_account(None),
                &[Duration::ZERO],
                |mut acc| async move {
                    acc.access_token = "renewed-token".to_string();
                    Ok(acc)
                },
                noop_mark_pending,
                noop_release_pending,
                always_allow_write,
            )
            .await
            .unwrap();
            assert!(res.success);
            assert_eq!(state.chat_calls.load(Ordering::SeqCst), 1);
            assert_eq!(state.report_calls.load(Ordering::SeqCst), 3);
            let req_ids = state.report_request_ids.lock().unwrap();
            let attempt_ids = state.report_attempt_ids.lock().unwrap();
            assert_eq!(req_ids.len(), 3);
            assert_eq!(req_ids[0], req_ids[1]);
            assert_eq!(req_ids[1], req_ids[2]);
            assert_eq!(attempt_ids.len(), 3);
            assert_eq!(attempt_ids[0], attempt_ids[1]);
            assert_eq!(attempt_ids[1], attempt_ids[2]);
            server.abort();
        }
    }

    #[test]
    fn apply_checkin_info_update_promotes_pending_model_across_midnight_rollover() {
        let mut account = fixture_account(None);
        account.checkin_rewards = Some(json!({
            "activityDate": "2026-11-23",
            "modelId": "previous-confirmed-model"
        }));
        workbuddy_ai_account::apply_pending_checkin_marker(
            &mut account,
            "2026-11-24",
            "hy4flash-free",
        );

        // Reserved on D (2026-11-24), confirmed on D+1 (2026-11-25) with modelId=null.
        workbuddy_ai_account::apply_checkin_info_update(
            &mut account,
            Some(1_700_000_800),
            Some(8),
            Some(json!({
                "activityDate": "2026-11-25",
                "settlement": "next_day",
                "modelId": Value::Null,
                "scoreAfter": 2,
            })),
        );

        let rewards = account.checkin_rewards.as_ref().unwrap();
        assert_eq!(rewards["activityDate"], "2026-11-25");
        assert_eq!(rewards["modelId"], "hy4flash-free");
        assert!(rewards.get("pendingActivityDate").is_none());
        assert!(rewards.get("pendingModelId").is_none());
        assert!(rewards.get("pendingCreatedAt").is_none());
    }

    #[tokio::test]
    async fn reconcile_status_confirms_prior_day_pending_on_next_day_without_marking_today_checked()
    {
        let dir = tempfile::tempdir().unwrap();
        let account_file = dir.path().join("acc_prior_pending.json");
        let mut account = fixture_account(None);
        account.last_checkin_time = Some(1_699_900_000);
        account.checkin_rewards = Some(json!({
            "activityDate": "2026-11-23",
            "modelId": "old-model",
            "pendingActivityDate": "2026-11-24",
            "pendingModelId": "hy4flash-free",
            "pendingCreatedAt": 1_700_000_999,
        }));
        workbuddy_ai_account::write_snapshot_to_path(&account_file, &account).unwrap();

        // Today is D+1 (2026-11-25) and NOT checked in yet, while prior day P (2026-11-24)
        // now appears in active heatmap cells.
        let state = MockState::new(false, 0);
        *state.today_date.lock().unwrap() = "2026-11-25".to_string();
        state
            .extra_active_cells
            .lock()
            .unwrap()
            .push(("2026-11-24".to_string(), 2));
        let (origin, server) = spawn_mock_server(state).await;
        let client = Client::builder().no_proxy().build().unwrap();
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        let recorded: Arc<Mutex<Vec<(String, Option<String>)>>> = Arc::new(Mutex::new(Vec::new()));

        let path_for_persist = account_file.clone();
        let recorded_for_cb = recorded.clone();
        let (status, reconciled) = get_checkin_status_and_reconcile_with_ops(
            &client,
            &origin,
            workbuddy_ai_account::load_snapshot_from_path(&account_file).unwrap(),
            noop_renewer,
            move || Some(lock),
            move |status, now_ts| async move {
                let mut latest = workbuddy_ai_account::load_snapshot_from_path(&path_for_persist)?;
                let Some((reconcile_date, last_checkin_time, streak, rewards)) =
                    build_reconcile_update(&latest, &status, now_ts)
                else {
                    return Ok(None);
                };
                workbuddy_ai_account::apply_checkin_info_update(
                    &mut latest,
                    last_checkin_time,
                    streak,
                    Some(rewards),
                );
                workbuddy_ai_account::write_snapshot_to_path(&path_for_persist, &latest)?;
                let model = latest
                    .checkin_rewards
                    .as_ref()
                    .and_then(|v| v.get("modelId"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                Ok(Some((reconcile_date, model)))
            },
            move |date, model| async move {
                recorded_for_cb.lock().unwrap().push((date, model));
            },
        )
        .await
        .unwrap();

        assert!(reconciled);
        assert!(status.active);
        assert!(!status.today_checked_in);
        assert!(!status.pending_confirmation);
        assert_eq!(status.today_date.as_deref(), Some("2026-11-25"));
        assert_eq!(
            *recorded.lock().unwrap(),
            vec![("2026-11-24".to_string(), Some("hy4flash-free".to_string()))]
        );

        let reloaded = workbuddy_ai_account::load_snapshot_from_path(&account_file).unwrap();
        let rewards = reloaded.checkin_rewards.as_ref().unwrap();
        assert_eq!(rewards["activityDate"], "2026-11-24");
        assert_eq!(rewards["modelId"], "hy4flash-free");
        assert!(rewards.get("pendingActivityDate").is_none());
        assert!(rewards.get("pendingModelId").is_none());
        assert!(rewards.get("pendingCreatedAt").is_none());
        assert_eq!(reloaded.last_checkin_time, Some(1_700_000_999));
        assert!(workbuddy_ai_account::has_confirmed_checkin_date(
            &reloaded,
            "2026-11-24"
        ));
        assert!(!workbuddy_ai_account::has_confirmed_checkin_date(
            &reloaded,
            "2026-11-25"
        ));
        server.abort();
    }

    #[tokio::test]
    async fn reconcile_status_promotes_pending_model_on_same_day_and_rollover_confirmation() {
        // Case 1: Same-day pending confirmed on 2026-11-24.
        // Case 2: Midnight rollover pending (reserved 2026-11-24, confirmed on 2026-11-25 with 2026-11-24 absent).
        for (pending_date, server_today) in
            [("2026-11-24", "2026-11-24"), ("2026-11-24", "2026-11-25")]
        {
            let dir = tempfile::tempdir().unwrap();
            let account_file = dir.path().join("acc_reconcile.json");
            let mut account = fixture_account(None);
            workbuddy_ai_account::apply_pending_checkin_marker(
                &mut account,
                pending_date,
                "hy4flash-free",
            );
            workbuddy_ai_account::write_snapshot_to_path(&account_file, &account).unwrap();

            let state = MockState::new(true, 2);
            *state.today_date.lock().unwrap() = server_today.to_string();
            let (origin, server) = spawn_mock_server(state).await;
            let client = Client::builder().no_proxy().build().unwrap();
            let lock = Arc::new(tokio::sync::Mutex::new(()));
            let recorded: Arc<Mutex<Vec<(String, Option<String>)>>> =
                Arc::new(Mutex::new(Vec::new()));

            let path_for_persist = account_file.clone();
            let recorded_for_cb = recorded.clone();
            let (status, reconciled) = get_checkin_status_and_reconcile_with_ops(
                &client,
                &origin,
                workbuddy_ai_account::load_snapshot_from_path(&account_file).unwrap(),
                noop_renewer,
                move || Some(lock),
                move |status, now_ts| async move {
                    let mut latest =
                        workbuddy_ai_account::load_snapshot_from_path(&path_for_persist)?;
                    let Some((reconcile_date, last_checkin_time, streak, rewards)) =
                        build_reconcile_update(&latest, &status, now_ts)
                    else {
                        return Ok(None);
                    };
                    workbuddy_ai_account::apply_checkin_info_update(
                        &mut latest,
                        last_checkin_time,
                        streak,
                        Some(rewards),
                    );
                    workbuddy_ai_account::write_snapshot_to_path(&path_for_persist, &latest)?;
                    let model = latest
                        .checkin_rewards
                        .as_ref()
                        .and_then(|v| v.get("modelId"))
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    Ok(Some((reconcile_date, model)))
                },
                move |date, model| async move {
                    recorded_for_cb.lock().unwrap().push((date, model));
                },
            )
            .await
            .unwrap();

            assert!(reconciled);
            assert!(status.today_checked_in);
            assert_eq!(
                *recorded.lock().unwrap(),
                vec![(server_today.to_string(), Some("hy4flash-free".to_string()))]
            );
            let reloaded = workbuddy_ai_account::load_snapshot_from_path(&account_file).unwrap();
            let rewards = reloaded.checkin_rewards.as_ref().unwrap();
            assert_eq!(rewards["activityDate"], server_today);
            assert_eq!(rewards["modelId"], "hy4flash-free");
            assert!(rewards.get("pendingActivityDate").is_none());
            server.abort();
        }
    }

    #[test]
    fn heatmap_confirmed_date_attributes_same_day_prior_day_and_adjacent_rollover() {
        let same_day_active = HeatmapSummary {
            today_date: "2026-11-24".to_string(),
            is_active: true,
            score: 2,
            status_text: None,
            active_dates: vec!["2026-11-24".to_string()],
        };
        assert_eq!(
            heatmap_confirmed_date(&same_day_active, "2026-11-24").as_deref(),
            Some("2026-11-24")
        );

        // Rolled over to D+1 while target_date D appears in active_dates -> attribute to D.
        let rollover_target_present = HeatmapSummary {
            today_date: "2026-11-25".to_string(),
            is_active: false,
            score: 0,
            status_text: None,
            active_dates: vec!["2026-11-24".to_string()],
        };
        assert_eq!(
            heatmap_confirmed_date(&rollover_target_present, "2026-11-24").as_deref(),
            Some("2026-11-24")
        );

        // Rolled over to D+1, target_date D absent, adjacent D+1 active -> attribute to D+1.
        let rollover_adjacent_active = HeatmapSummary {
            today_date: "2026-11-25".to_string(),
            is_active: true,
            score: 2,
            status_text: None,
            active_dates: vec!["2026-11-25".to_string()],
        };
        assert_eq!(
            heatmap_confirmed_date(&rollover_adjacent_active, "2026-11-24").as_deref(),
            Some("2026-11-25")
        );

        // Non-adjacent jump (D+2 active, D absent) -> not confirmed for D.
        let non_adjacent = HeatmapSummary {
            today_date: "2026-11-26".to_string(),
            is_active: true,
            score: 2,
            status_text: None,
            active_dates: vec!["2026-11-26".to_string()],
        };
        assert_eq!(heatmap_confirmed_date(&non_adjacent, "2026-11-24"), None);
    }
}
