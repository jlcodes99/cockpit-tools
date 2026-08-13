use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::sync::{mpsc, Arc, LazyLock, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tauri::AppHandle;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use uuid::Uuid;

const SCHEMA_VERSION: u8 = 1;
const CHALLENGE_TTL_MINUTES: i64 = 120;
const MAX_REQUEST_BODY_BYTES: u64 = 32 * 1024;
const DESCRIPTOR_FILE_NAME: &str = "account-conversion-bridge.json";
const BRIDGE_CAPABILITIES: &[&str] = &["mfa_full_email_confirmation_v1"];

const ALLOWED_CHALLENGE_TYPES: &[&str] = &[
    "password_current",
    "password_new",
    "totp",
    "recovery_email_code",
    "phone_code",
    "passkey",
    "authenticator_setup",
    "backup_codes",
    "phone_removal",
    "session_signout",
    "captcha",
    "account_recovery",
    "extension_install",
    "generic",
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountConversionBridgeDescriptor {
    pub schema_version: u8,
    pub base_url: String,
    pub token: String,
    pub pid: u32,
    pub started_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountConversionChallengeStatus {
    Queued,
    Presented,
    UserConfirmed,
    Cancelled,
    Expired,
}

impl AccountConversionChallengeStatus {
    fn is_terminal(self) -> bool {
        matches!(
            self,
            AccountConversionChallengeStatus::UserConfirmed
                | AccountConversionChallengeStatus::Cancelled
                | AccountConversionChallengeStatus::Expired
        )
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountConversionChallenge {
    pub id: String,
    pub batch_id: String,
    pub run_id: String,
    pub slot: String,
    pub port: u16,
    pub chrome_pid: u32,
    pub expected_email: String,
    #[serde(rename = "type")]
    pub challenge_type: String,
    pub instructions: String,
    pub status: AccountConversionChallengeStatus,
    pub created_at: String,
    pub updated_at: String,
    pub expires_at: String,
    pub presented_at: Option<String>,
    pub confirmed_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateChallengeRequest {
    batch_id: String,
    run_id: String,
    slot: String,
    port: u16,
    chrome_pid: u32,
    expected_email: String,
    #[serde(rename = "type")]
    challenge_type: String,
    instructions: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountConversionBridgeStatus {
    pub running: bool,
    pub schema_version: u8,
    pub capabilities: Vec<&'static str>,
    pub base_url: Option<String>,
    pub pid: u32,
    pub started_at: Option<String>,
    pub queued_count: usize,
    pub presented_count: usize,
}

type ChallengeStore = Arc<Mutex<HashMap<String, AccountConversionChallenge>>>;
type IdempotencyStore = Arc<Mutex<HashMap<String, String>>>;

struct BridgeRuntime {
    descriptor: AccountConversionBridgeDescriptor,
    descriptor_path: PathBuf,
    challenges: ChallengeStore,
    idempotency_keys: IdempotencyStore,
    stop_tx: mpsc::Sender<()>,
    handle: Option<JoinHandle<()>>,
}

static BRIDGE_RUNTIME: LazyLock<Mutex<Option<BridgeRuntime>>> = LazyLock::new(|| Mutex::new(None));

fn now_iso() -> String {
    Utc::now().to_rfc3339()
}

fn descriptor_path() -> Result<PathBuf, String> {
    let base = dirs::data_local_dir()
        .ok_or_else(|| "Unable to resolve the local application data directory".to_string())?;
    Ok(base.join("Cockpit Tools").join(DESCRIPTOR_FILE_NAME))
}

fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn write_descriptor(
    path: &PathBuf,
    descriptor: &AccountConversionBridgeDescriptor,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Unable to create bridge descriptor directory: {error}"))?;
    }
    let payload = serde_json::to_vec_pretty(descriptor)
        .map_err(|error| format!("Unable to serialize bridge descriptor: {error}"))?;
    fs::write(path, payload).map_err(|error| format!("Unable to write bridge descriptor: {error}"))
}

fn validate_identifier(label: &str, value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 160 {
        return Err(format!("{label} must contain between 1 and 160 characters"));
    }
    if !trimmed
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "._:-".contains(character))
    {
        return Err(format!("{label} contains an unsupported character"));
    }
    Ok(trimmed.to_string())
}

fn validate_idempotency_key(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 160 {
        return Err("Idempotency key must contain between 1 and 160 characters".to_string());
    }
    if !trimmed
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "._:-".contains(character))
    {
        return Err("Idempotency key contains an unsupported character".to_string());
    }
    Ok(trimmed.to_string())
}

fn validate_email(value: &str) -> Result<String, String> {
    let normalized = value.trim().to_ascii_lowercase();
    let parts = normalized.split('@').collect::<Vec<_>>();
    if parts.len() != 2
        || parts[0].is_empty()
        || !parts[1].contains('.')
        || normalized.len() > 254
        || normalized.chars().any(char::is_whitespace)
    {
        return Err("expectedEmail must be a complete email address".to_string());
    }
    Ok(normalized)
}

fn validate_create_request(
    request: CreateChallengeRequest,
) -> Result<AccountConversionChallenge, String> {
    if !ALLOWED_CHALLENGE_TYPES.contains(&request.challenge_type.as_str()) {
        return Err("Unsupported account-conversion challenge type".to_string());
    }
    let instructions = request.instructions.trim();
    if instructions.is_empty() || instructions.len() > 1200 {
        return Err("instructions must contain between 1 and 1200 characters".to_string());
    }
    if request.port < 1024 || request.chrome_pid == 0 {
        return Err("port and chromePid must identify a live per-profile Chrome".to_string());
    }
    let now = Utc::now();
    Ok(AccountConversionChallenge {
        id: Uuid::new_v4().to_string(),
        batch_id: validate_identifier("batchId", &request.batch_id)?,
        run_id: validate_identifier("runId", &request.run_id)?,
        slot: validate_identifier("slot", &request.slot)?,
        port: request.port,
        chrome_pid: request.chrome_pid,
        expected_email: validate_email(&request.expected_email)?,
        challenge_type: request.challenge_type,
        instructions: instructions.to_string(),
        status: AccountConversionChallengeStatus::Queued,
        created_at: now.to_rfc3339(),
        updated_at: now.to_rfc3339(),
        expires_at: (now + ChronoDuration::minutes(CHALLENGE_TTL_MINUTES)).to_rfc3339(),
        presented_at: None,
        confirmed_at: None,
    })
}

fn expire_challenges(challenges: &mut HashMap<String, AccountConversionChallenge>) {
    let now = Utc::now();
    for challenge in challenges.values_mut() {
        if challenge.status.is_terminal() {
            continue;
        }
        let expires_at = DateTime::parse_from_rfc3339(&challenge.expires_at)
            .map(|value| value.with_timezone(&Utc))
            .unwrap_or(now);
        if expires_at <= now {
            challenge.status = AccountConversionChallengeStatus::Expired;
            challenge.updated_at = now.to_rfc3339();
        }
    }
}

fn challenge_for_idempotency_key(
    challenges: &ChallengeStore,
    idempotency_keys: &IdempotencyStore,
    key: &str,
) -> Option<AccountConversionChallenge> {
    let challenge_id = idempotency_keys.lock().ok()?.get(key).cloned()?;
    let mut guard = challenges.lock().ok()?;
    expire_challenges(&mut guard);
    guard.get(&challenge_id).cloned()
}

fn update_challenge_status(
    challenges: &ChallengeStore,
    id: &str,
    status: AccountConversionChallengeStatus,
) -> Result<AccountConversionChallenge, String> {
    let mut guard = challenges
        .lock()
        .map_err(|_| "Account-conversion challenge store is unavailable".to_string())?;
    expire_challenges(&mut guard);
    let challenge = guard
        .get_mut(id)
        .ok_or_else(|| "Account-conversion challenge was not found".to_string())?;
    if challenge.status.is_terminal() && challenge.status != status {
        return Err("A terminal account-conversion challenge cannot change state".to_string());
    }
    let now = now_iso();
    challenge.status = status;
    challenge.updated_at = now.clone();
    if status == AccountConversionChallengeStatus::Presented {
        challenge.presented_at = Some(now.clone());
    }
    if status == AccountConversionChallengeStatus::UserConfirmed {
        challenge.confirmed_at = Some(now);
    }
    Ok(challenge.clone())
}

fn confirm_challenge(
    challenges: &ChallengeStore,
    id: &str,
    mfa_match_email: Option<&str>,
) -> Result<AccountConversionChallenge, String> {
    let mut guard = challenges
        .lock()
        .map_err(|_| "Account-conversion challenge store is unavailable".to_string())?;
    expire_challenges(&mut guard);
    let challenge = guard
        .get_mut(id)
        .ok_or_else(|| "Account-conversion challenge was not found".to_string())?;
    if challenge.status.is_terminal()
        && challenge.status != AccountConversionChallengeStatus::UserConfirmed
    {
        return Err("A terminal account-conversion challenge cannot change state".to_string());
    }
    if matches!(
        challenge.challenge_type.as_str(),
        "totp" | "authenticator_setup"
    ) {
        let matched_email = mfa_match_email
            .ok_or_else(|| {
                "A full-email-matched MFA record must be selected before confirmation".to_string()
            })
            .and_then(validate_email)?;
        if matched_email != challenge.expected_email {
            return Err("The selected MFA record does not match the challenge email".to_string());
        }
    }
    let now = now_iso();
    challenge.status = AccountConversionChallengeStatus::UserConfirmed;
    challenge.updated_at = now.clone();
    challenge.confirmed_at = Some(now);
    Ok(challenge.clone())
}

fn store_for_commands() -> Result<ChallengeStore, String> {
    let guard = BRIDGE_RUNTIME
        .lock()
        .map_err(|_| "Account-conversion bridge state is unavailable".to_string())?;
    guard
        .as_ref()
        .map(|runtime| runtime.challenges.clone())
        .ok_or_else(|| "Account-conversion bridge is not running".to_string())
}

pub fn ensure_started(app: AppHandle) -> Result<AccountConversionBridgeDescriptor, String> {
    let mut runtime_guard = BRIDGE_RUNTIME
        .lock()
        .map_err(|_| "Account-conversion bridge state is unavailable".to_string())?;
    if let Some(runtime) = runtime_guard.as_ref() {
        return Ok(runtime.descriptor.clone());
    }

    let server = Server::http("127.0.0.1:0")
        .map_err(|error| format!("Unable to bind account-conversion bridge: {error}"))?;
    let port = server
        .server_addr()
        .to_ip()
        .ok_or_else(|| "Account-conversion bridge did not bind an IP socket".to_string())?
        .port();
    let descriptor = AccountConversionBridgeDescriptor {
        schema_version: SCHEMA_VERSION,
        base_url: format!("http://127.0.0.1:{port}"),
        token: random_token(),
        pid: std::process::id(),
        started_at: now_iso(),
    };
    let path = descriptor_path()?;
    write_descriptor(&path, &descriptor)?;

    let challenges = Arc::new(Mutex::new(HashMap::new()));
    let idempotency_keys = Arc::new(Mutex::new(HashMap::new()));
    let thread_challenges = challenges.clone();
    let thread_idempotency_keys = idempotency_keys.clone();
    let thread_token = descriptor.token.clone();
    let (stop_tx, stop_rx) = mpsc::channel();
    let handle = thread::Builder::new()
        .name("account-conversion-bridge".to_string())
        .spawn(move || {
            run_server(
                server,
                thread_token,
                thread_challenges,
                thread_idempotency_keys,
                stop_rx,
                app,
            )
        })
        .map_err(|error| format!("Unable to start account-conversion bridge: {error}"))?;

    *runtime_guard = Some(BridgeRuntime {
        descriptor: descriptor.clone(),
        descriptor_path: path,
        challenges,
        idempotency_keys,
        stop_tx,
        handle: Some(handle),
    });
    Ok(descriptor)
}

pub fn shutdown() {
    let runtime = BRIDGE_RUNTIME
        .lock()
        .ok()
        .and_then(|mut guard| guard.take());
    if let Some(mut runtime) = runtime {
        let _ = runtime.stop_tx.send(());
        if let Some(handle) = runtime.handle.take() {
            let _ = handle.join();
        }
        let _ = fs::remove_file(runtime.descriptor_path);
    }
}

fn run_server(
    server: Server,
    token: String,
    challenges: ChallengeStore,
    idempotency_keys: IdempotencyStore,
    stop_rx: mpsc::Receiver<()>,
    app: AppHandle,
) {
    loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }
        match server.recv_timeout(Duration::from_millis(250)) {
            Ok(Some(request)) => {
                handle_http_request(request, &token, &challenges, &idempotency_keys, &app)
            }
            Ok(None) => {}
            Err(_) => break,
        }
    }
}

fn request_authorized(request: &Request, token: &str) -> bool {
    request.headers().iter().any(|header| {
        let name = header.field.as_str().as_str().to_ascii_lowercase();
        let value = header.value.as_str().trim();
        (name == "x-cockpit-bridge-token" && value == token)
            || (name == "authorization"
                && value
                    .strip_prefix("Bearer ")
                    .is_some_and(|candidate| candidate.trim() == token))
    })
}

fn request_header(request: &Request, expected_name: &str) -> Option<String> {
    request.headers().iter().find_map(|header| {
        let name = header.field.as_str().as_str().to_ascii_lowercase();
        (name == expected_name).then(|| header.value.as_str().trim().to_string())
    })
}

fn read_json_body(request: &mut Request) -> Result<Value, String> {
    let mut bytes = Vec::new();
    request
        .as_reader()
        .take(MAX_REQUEST_BODY_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Unable to read request body".to_string())?;
    if bytes.len() as u64 > MAX_REQUEST_BODY_BYTES {
        return Err("Request body is too large".to_string());
    }
    serde_json::from_slice(&bytes).map_err(|_| "Request body must be valid JSON".to_string())
}

fn handle_http_request(
    mut request: Request,
    token: &str,
    challenges: &ChallengeStore,
    idempotency_keys: &IdempotencyStore,
    app: &AppHandle,
) {
    if !request_authorized(&request, token) {
        let _ = request.respond(json_response(401, json!({ "error": "Unauthorized" })));
        return;
    }
    let method = request.method().clone();
    let path = request
        .url()
        .split('?')
        .next()
        .unwrap_or(request.url())
        .trim_end_matches('/')
        .to_string();

    if method == Method::Get && path == "/v1/health" {
        let _ = request.respond(json_response(
            200,
            json!({
                "schemaVersion": SCHEMA_VERSION,
                "ok": true,
                "pid": std::process::id(),
                "capabilities": BRIDGE_CAPABILITIES,
            }),
        ));
        return;
    }

    if method == Method::Post && path == "/v1/challenges" {
        let idempotency_key = request_header(&request, "x-cockpit-idempotency-key")
            .map(|value| validate_idempotency_key(&value))
            .transpose();
        let existing_challenge = idempotency_key.as_ref().ok().and_then(|key| {
            key.as_ref()
                .and_then(|key| challenge_for_idempotency_key(challenges, idempotency_keys, key))
        });
        if let Some(challenge) = existing_challenge {
            let _ = crate::modules::floating_card_window::show_main_window_and_navigate(app, "2fa");
            let _ = request.respond(json_response(200, challenge_http_value(&challenge)));
            return;
        }
        let idempotency_key = match idempotency_key {
            Ok(value) => value,
            Err(error) => {
                let _ = request.respond(json_response(400, json!({ "error": error })));
                return;
            }
        };
        let result = read_json_body(&mut request)
            .and_then(|value| {
                serde_json::from_value::<CreateChallengeRequest>(value).map_err(|_| {
                    "Challenge request contains missing or unsupported fields".to_string()
                })
            })
            .and_then(validate_create_request)
            .and_then(|challenge| {
                let mut guard = challenges
                    .lock()
                    .map_err(|_| "Account-conversion challenge store is unavailable".to_string())?;
                guard.insert(challenge.id.clone(), challenge.clone());
                drop(guard);
                if let Some(key) = idempotency_key.as_ref() {
                    idempotency_keys
                        .lock()
                        .map_err(|_| {
                            "Account-conversion idempotency store is unavailable".to_string()
                        })?
                        .insert(key.clone(), challenge.id.clone());
                }
                Ok(challenge)
            });
        match result {
            Ok(challenge) => {
                let _ =
                    crate::modules::floating_card_window::show_main_window_and_navigate(app, "2fa");
                let _ = request.respond(json_response(201, challenge_http_value(&challenge)));
            }
            Err(error) => {
                let _ = request.respond(json_response(400, json!({ "error": error })));
            }
        }
        return;
    }

    let segments = path.trim_start_matches('/').split('/').collect::<Vec<_>>();
    if segments.len() >= 3 && segments[0] == "v1" && segments[1] == "challenges" {
        let id = segments[2];
        if method == Method::Get && segments.len() == 3 {
            let value = challenges.lock().ok().and_then(|mut guard| {
                expire_challenges(&mut guard);
                guard.get(id).cloned()
            });
            let _ = match value {
                Some(challenge) => {
                    request.respond(json_response(200, challenge_http_value(&challenge)))
                }
                None => request.respond(json_response(404, json!({ "error": "Not found" }))),
            };
            return;
        }
        if method == Method::Post && segments.len() == 4 && segments[3] == "present" {
            match update_challenge_status(
                challenges,
                id,
                AccountConversionChallengeStatus::Presented,
            ) {
                Ok(challenge) => {
                    let _ = crate::modules::floating_card_window::show_main_window_and_navigate(
                        app, "2fa",
                    );
                    let _ = request.respond(json_response(200, challenge_http_value(&challenge)));
                }
                Err(error) => {
                    let _ = request.respond(json_response(409, json!({ "error": error })));
                }
            }
            return;
        }
        if method == Method::Post && segments.len() == 4 && segments[3] == "cancel" {
            let result = update_challenge_status(
                challenges,
                id,
                AccountConversionChallengeStatus::Cancelled,
            );
            let _ = match result {
                Ok(challenge) => {
                    request.respond(json_response(200, challenge_http_value(&challenge)))
                }
                Err(error) => request.respond(json_response(409, json!({ "error": error }))),
            };
            return;
        }
    }

    let _ = request.respond(json_response(404, json!({ "error": "Not found" })));
}

fn json_response(status: u16, value: Value) -> Response<Cursor<Vec<u8>>> {
    let body = serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec());
    let headers = vec![
        Header::from_bytes("content-type", "application/json; charset=utf-8").unwrap(),
        Header::from_bytes("cache-control", "no-store").unwrap(),
        Header::from_bytes("x-content-type-options", "nosniff").unwrap(),
    ];
    Response::new(StatusCode(status), headers, Cursor::new(body), None, None)
}

fn challenge_http_value(challenge: &AccountConversionChallenge) -> Value {
    // HTTP intentionally returns only lifecycle state and timestamps. Account
    // metadata remains in Cockpit memory/UI and is never echoed to the CDP
    // orchestrator after challenge creation.
    json!({
        "id": challenge.id,
        "status": challenge.status,
        "createdAt": challenge.created_at,
        "updatedAt": challenge.updated_at,
        "expiresAt": challenge.expires_at,
        "presentedAt": challenge.presented_at,
        "confirmedAt": challenge.confirmed_at,
    })
}

#[tauri::command]
pub fn account_conversion_bridge_status() -> Result<AccountConversionBridgeStatus, String> {
    let guard = BRIDGE_RUNTIME
        .lock()
        .map_err(|_| "Account-conversion bridge state is unavailable".to_string())?;
    let Some(runtime) = guard.as_ref() else {
        return Ok(AccountConversionBridgeStatus {
            running: false,
            schema_version: SCHEMA_VERSION,
            capabilities: BRIDGE_CAPABILITIES.to_vec(),
            base_url: None,
            pid: std::process::id(),
            started_at: None,
            queued_count: 0,
            presented_count: 0,
        });
    };
    let mut challenges = runtime
        .challenges
        .lock()
        .map_err(|_| "Account-conversion challenge store is unavailable".to_string())?;
    expire_challenges(&mut challenges);
    Ok(AccountConversionBridgeStatus {
        running: true,
        schema_version: SCHEMA_VERSION,
        capabilities: BRIDGE_CAPABILITIES.to_vec(),
        base_url: Some(runtime.descriptor.base_url.clone()),
        pid: runtime.descriptor.pid,
        started_at: Some(runtime.descriptor.started_at.clone()),
        queued_count: challenges
            .values()
            .filter(|item| item.status == AccountConversionChallengeStatus::Queued)
            .count(),
        presented_count: challenges
            .values()
            .filter(|item| item.status == AccountConversionChallengeStatus::Presented)
            .count(),
    })
}

#[tauri::command]
pub fn account_conversion_list_challenges() -> Result<Vec<AccountConversionChallenge>, String> {
    let challenges = store_for_commands()?;
    let mut guard = challenges
        .lock()
        .map_err(|_| "Account-conversion challenge store is unavailable".to_string())?;
    expire_challenges(&mut guard);
    let mut values = guard.values().cloned().collect::<Vec<_>>();
    values.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    Ok(values)
}

#[tauri::command]
pub fn account_conversion_present_challenge(
    app: AppHandle,
    id: String,
) -> Result<AccountConversionChallenge, String> {
    let challenge = update_challenge_status(
        &store_for_commands()?,
        id.trim(),
        AccountConversionChallengeStatus::Presented,
    )?;
    crate::modules::floating_card_window::show_main_window_and_navigate(&app, "2fa")?;
    Ok(challenge)
}

#[tauri::command]
pub fn account_conversion_confirm_challenge(
    id: String,
    mfa_match_email: Option<String>,
) -> Result<AccountConversionChallenge, String> {
    confirm_challenge(
        &store_for_commands()?,
        id.trim(),
        mfa_match_email.as_deref(),
    )
}

#[tauri::command]
pub fn account_conversion_cancel_challenge(
    id: String,
) -> Result<AccountConversionChallenge, String> {
    update_challenge_status(
        &store_for_commands()?,
        id.trim(),
        AccountConversionChallengeStatus::Cancelled,
    )
}

#[tauri::command]
pub fn account_conversion_focus_chrome(chrome_pid: u32) -> Result<u32, String> {
    crate::modules::process::focus_process_pid(chrome_pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_request(challenge_type: &str) -> CreateChallengeRequest {
        CreateChallengeRequest {
            batch_id: "batch-1".to_string(),
            run_id: "run-1".to_string(),
            slot: "Imported-01".to_string(),
            port: 9601,
            chrome_pid: 1234,
            expected_email: "Example.Account@gmail.com".to_string(),
            challenge_type: challenge_type.to_string(),
            instructions: "Complete the official Google step in Chrome.".to_string(),
        }
    }

    #[test]
    fn validates_every_supported_challenge_type() {
        for challenge_type in ALLOWED_CHALLENGE_TYPES {
            let challenge = validate_create_request(valid_request(challenge_type)).unwrap();
            assert_eq!(challenge.challenge_type, *challenge_type);
            assert_eq!(challenge.expected_email, "example.account@gmail.com");
            assert_eq!(challenge.status, AccountConversionChallengeStatus::Queued);
        }
    }

    #[test]
    fn rejects_unknown_or_secret_bearing_fields() {
        let value = json!({
            "batchId": "batch-1",
            "runId": "run-1",
            "slot": "Imported-01",
            "port": 9601,
            "chromePid": 1234,
            "expectedEmail": "example@gmail.com",
            "type": "totp",
            "instructions": "Paste the code yourself.",
            "otp": "000000"
        });
        assert!(serde_json::from_value::<CreateChallengeRequest>(value).is_err());
    }

    #[test]
    fn terminal_challenge_cannot_be_reopened() {
        let challenge = validate_create_request(valid_request("passkey")).unwrap();
        let id = challenge.id.clone();
        let store = Arc::new(Mutex::new(HashMap::from([(id.clone(), challenge)])));
        update_challenge_status(&store, &id, AccountConversionChallengeStatus::UserConfirmed)
            .unwrap();
        assert!(
            update_challenge_status(&store, &id, AccountConversionChallengeStatus::Presented,)
                .is_err()
        );
    }

    #[test]
    fn mfa_challenge_confirmation_requires_the_exact_full_email() {
        for challenge_type in ["totp", "authenticator_setup"] {
            let challenge = validate_create_request(valid_request(challenge_type)).unwrap();
            let id = challenge.id.clone();
            let store = Arc::new(Mutex::new(HashMap::from([(id.clone(), challenge)])));
            assert!(confirm_challenge(&store, &id, None).is_err());
            assert!(confirm_challenge(&store, &id, Some("other@gmail.com")).is_err());
            let confirmed =
                confirm_challenge(&store, &id, Some("EXAMPLE.ACCOUNT@GMAIL.COM")).unwrap();
            assert_eq!(
                confirmed.status,
                AccountConversionChallengeStatus::UserConfirmed
            );
        }
    }

    #[test]
    fn non_mfa_challenge_confirmation_does_not_require_an_mfa_match() {
        let challenge = validate_create_request(valid_request("password_current")).unwrap();
        let id = challenge.id.clone();
        let store = Arc::new(Mutex::new(HashMap::from([(id.clone(), challenge)])));
        let confirmed = confirm_challenge(&store, &id, None).unwrap();
        assert_eq!(
            confirmed.status,
            AccountConversionChallengeStatus::UserConfirmed
        );
    }

    #[test]
    fn validates_non_secret_idempotency_keys() {
        assert_eq!(
            validate_idempotency_key("local-challenge:1234").unwrap(),
            "local-challenge:1234"
        );
        assert!(validate_idempotency_key("").is_err());
        assert!(validate_idempotency_key("contains a space").is_err());
        assert!(validate_idempotency_key(&"x".repeat(161)).is_err());
    }

    #[test]
    fn challenge_state_transitions_preserve_terminal_boundaries() {
        let challenge = validate_create_request(valid_request("password_current")).unwrap();
        let id = challenge.id.clone();
        let store = Arc::new(Mutex::new(HashMap::from([(id.clone(), challenge)])));

        let presented =
            update_challenge_status(&store, &id, AccountConversionChallengeStatus::Presented)
                .unwrap();
        assert_eq!(
            presented.status,
            AccountConversionChallengeStatus::Presented
        );
        assert!(presented.presented_at.is_some());

        let confirmed = confirm_challenge(&store, &id, None).unwrap();
        assert_eq!(
            confirmed.status,
            AccountConversionChallengeStatus::UserConfirmed
        );
        assert!(confirmed.confirmed_at.is_some());
        assert!(
            update_challenge_status(&store, &id, AccountConversionChallengeStatus::Cancelled,)
                .is_err()
        );
    }

    #[test]
    fn cancellation_is_terminal() {
        let challenge = validate_create_request(valid_request("captcha")).unwrap();
        let id = challenge.id.clone();
        let store = Arc::new(Mutex::new(HashMap::from([(id.clone(), challenge)])));
        let cancelled =
            update_challenge_status(&store, &id, AccountConversionChallengeStatus::Cancelled)
                .unwrap();
        assert_eq!(
            cancelled.status,
            AccountConversionChallengeStatus::Cancelled
        );
        assert!(confirm_challenge(&store, &id, None).is_err());
    }

    #[test]
    fn expired_challenge_cannot_be_presented_or_confirmed() {
        let mut challenge = validate_create_request(valid_request("passkey")).unwrap();
        challenge.expires_at = (Utc::now() - ChronoDuration::seconds(1)).to_rfc3339();
        let id = challenge.id.clone();
        let store = Arc::new(Mutex::new(HashMap::from([(id.clone(), challenge)])));

        assert!(
            update_challenge_status(&store, &id, AccountConversionChallengeStatus::Presented,)
                .is_err()
        );
        assert!(confirm_challenge(&store, &id, None).is_err());
        assert_eq!(
            store.lock().unwrap().get(&id).unwrap().status,
            AccountConversionChallengeStatus::Expired
        );
    }

    #[test]
    fn idempotency_lookup_reuses_the_same_id_and_applies_ttl() {
        let mut challenge = validate_create_request(valid_request("generic")).unwrap();
        challenge.expires_at = (Utc::now() - ChronoDuration::seconds(1)).to_rfc3339();
        let id = challenge.id.clone();
        let challenges = Arc::new(Mutex::new(HashMap::from([(id.clone(), challenge)])));
        let idempotency_keys = Arc::new(Mutex::new(HashMap::from([(
            "local-challenge:1234".to_string(),
            id.clone(),
        )])));

        let existing =
            challenge_for_idempotency_key(&challenges, &idempotency_keys, "local-challenge:1234")
                .unwrap();
        assert_eq!(existing.id, id);
        assert_eq!(existing.status, AccountConversionChallengeStatus::Expired);
        assert!(
            challenge_for_idempotency_key(&challenges, &idempotency_keys, "missing-key",).is_none()
        );
    }
}
