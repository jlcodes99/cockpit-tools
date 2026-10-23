use super::{http, storage, types::*};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

const DYNAMIC_CLIENT: &str = "dynamic_agent_client";
const SCOPES: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
const LIFETIME: i64 = 600;

struct Session {
    id: String,
    registration_id: String,
    selected: Option<Registration>,
    state: String,
    nonce: String,
    verifier: String,
    redirect_uri: String,
    authorize_url: String,
    callback_received: Notify,
    expires_at: i64,
    result: Mutex<ChatgptPlanLoginStatus>,
    cancel: Notify,
}

static SESSIONS: LazyLock<Mutex<HashMap<String, Arc<Session>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn random_parameter() -> String {
    let mut bytes = [0_u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn authorize_url(
    session: &Session,
    host_id: &str,
    credentials: Option<&Credentials>,
) -> Result<String, String> {
    let mut url = url::Url::parse(&http::auth_url("/api/accounts/authorize"))
        .map_err(|_| "CHATGPT_PLAN_INVALID_CALLBACK".to_string())?;
    let client_id = session
        .selected
        .as_ref()
        .map(|record| record.summary.client_id.as_str())
        .unwrap_or(DYNAMIC_CLIENT);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(session.verifier.as_bytes()));
    let host_urn = format!("urn:uuid:{host_id}");
    {
        let mut query = url.query_pairs_mut();
        query.extend_pairs([
            ("client_id", client_id),
            ("ext_agent_host_id", host_urn.as_str()),
            ("response_type", "code"),
            ("redirect_uri", session.redirect_uri.as_str()),
            ("scope", SCOPES),
            ("resource", http::RESOURCE),
            ("state", session.state.as_str()),
            ("nonce", session.nonce.as_str()),
            ("code_challenge_method", "S256"),
            ("code_challenge", challenge.as_str()),
        ]);
        if session.selected.is_none() {
            query.append_pair("agent_name_hint", "Cockpit Tools");
        }
        if let Some(credentials) = credentials {
            query.append_pair("id_token_hint", &credentials.id_token);
        }
        if let Some(email) = session
            .selected
            .as_ref()
            .and_then(|record| record.summary.email.as_deref())
        {
            query.append_pair("login_hint", email);
        }
    }
    Ok(url.to_string())
}

pub async fn start_login(registration_id: Option<&str>) -> Result<ChatgptPlanLogin, String> {
    let selected = match registration_id {
        Some(id) => Some(storage::read(id).await?),
        None => None,
    };
    let retained = if let Some(record) = selected
        .as_ref()
        .filter(|record| record.encrypted_credentials.is_some())
    {
        let record = record.clone();
        Some(storage::blocking(move || record.credentials()).await?)
    } else {
        None
    };
    let host_id = storage::host_id().await?;
    // Bind before constructing/returning the URL; it never interferes with the
    // Codex client's fixed callback ports. Only the port varies on later login.
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|_| "CHATGPT_PLAN_BUSY".to_string())?;
    let port = listener
        .local_addr()
        .map_err(|_| "CHATGPT_PLAN_BUSY".to_string())?
        .port();
    let mut session = Session {
        id: uuid::Uuid::new_v4().to_string(),
        registration_id: selected
            .as_ref()
            .map(|record| record.summary.id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        selected,
        state: random_parameter(),
        nonce: random_parameter(),
        verifier: random_parameter(),
        redirect_uri: format!("http://127.0.0.1:{port}/auth/callback"),
        authorize_url: String::new(),
        callback_received: Notify::new(),
        expires_at: chrono::Utc::now().timestamp() + LIFETIME,
        result: Mutex::new(ChatgptPlanLoginStatus {
            status: "pending".into(),
            account: None,
            error: None,
        }),
        cancel: Notify::new(),
    };
    let url = authorize_url(&session, &host_id, retained.as_ref())?;
    session.authorize_url = url.clone();
    let session = Arc::new(session);
    {
        let mut sessions = SESSIONS
            .lock()
            .map_err(|_| "CHATGPT_PLAN_BUSY".to_string())?;
        let now = chrono::Utc::now().timestamp();
        sessions.retain(|_, session| session.expires_at + 600 > now);
        let active = sessions
            .values()
            .filter(|session| {
                session
                    .result
                    .lock()
                    .is_ok_and(|state| matches!(state.status.as_str(), "pending" | "exchanging"))
            })
            .count();
        if active >= 4 {
            return Err("CHATGPT_PLAN_BUSY".into());
        }
        if sessions.values().any(|existing| {
            existing.registration_id == session.registration_id
                && existing
                    .result
                    .lock()
                    .is_ok_and(|state| matches!(state.status.as_str(), "pending" | "exchanging"))
        }) {
            return Err("CHATGPT_PLAN_BUSY".into());
        }
        sessions.insert(session.id.clone(), session.clone());
    }
    let login = ChatgptPlanLogin {
        login_id: session.id.clone(),
        authorize_url: url,
        expires_at: session.expires_at,
    };
    tokio::spawn(async move {
        let result = tokio::time::timeout(
            Duration::from_secs(LIFETIME as u64),
            run_listener(&session, listener),
        )
        .await;
        let error = match result {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(error),
            Err(_) => Some("CHATGPT_PLAN_LOGIN_EXPIRED".to_string()),
        };
        if let Some(error) = error {
            finish_login_error(&session, error);
        }
    });
    Ok(login)
}

fn find_session(id: &str) -> Result<Arc<Session>, String> {
    SESSIONS
        .lock()
        .map_err(|_| "CHATGPT_PLAN_BUSY".to_string())?
        .get(id)
        .cloned()
        .ok_or("CHATGPT_PLAN_LOGIN_EXPIRED".into())
}

fn pending_session(session: &Session, state: &ChatgptPlanLoginStatus) -> Result<(), String> {
    if state.status == "cancelled" {
        return Err("CHATGPT_PLAN_LOGIN_CANCELLED".into());
    }
    if session.expires_at <= chrono::Utc::now().timestamp() || state.status == "expired" {
        return Err("CHATGPT_PLAN_LOGIN_EXPIRED".into());
    }
    if state.status != "pending" {
        return Err("CHATGPT_PLAN_BUSY".into());
    }
    Ok(())
}

fn finish_login_error(session: &Session, error: String) {
    if let Ok(mut state) = session.result.lock() {
        if matches!(state.status.as_str(), "pending" | "exchanging") {
            state.status = if error == "CHATGPT_PLAN_LOGIN_EXPIRED" {
                "expired"
            } else {
                "error"
            }
            .into();
            state.error = Some(error);
        }
    }
    close_session_window(session);
}

fn close_session_window(session: &Session) {
    use tauri::Manager;
    let Some(app) = crate::get_app_handle() else {
        return;
    };
    let handle = app.clone();
    let label = format!("chatgpt-plan-oauth-{}", session.id);
    // Queue cleanup after releasing session locks; never block token exchange
    // or cancellation on a WebView operation and never close a Codex window.
    if app
        .run_on_main_thread(move || {
            if let Some(window) = handle.get_webview_window(&label) {
                if window.destroy().is_err() {
                    crate::modules::logger::log_warn(
                        "[ChatGPT Plan] authorization window cleanup failed",
                    );
                }
            }
        })
        .is_err()
    {
        crate::modules::logger::log_warn(
            "[ChatGPT Plan] authorization window cleanup could not be queued",
        );
    }
}

/// Both loopback and pasted callbacks acquire the same pending -> exchanging
/// transition. Tokens, PKCE, nonce and durable commit have only one owner.
fn accept_callback(session: Arc<Session>, query: HashMap<String, String>) -> Result<(), String> {
    if query.get("state").map(String::as_str) != Some(session.state.as_str()) {
        return Err("CHATGPT_PLAN_INVALID_CALLBACK".into());
    }
    if session.selected.as_ref().is_some_and(|selected| {
        query
            .get("client_id")
            .is_some_and(|client| client != &selected.summary.client_id)
    }) {
        return Err("CHATGPT_PLAN_REGISTRATION_CHANGED".into());
    }
    if let Some(error) = query.get("error") {
        if error.is_empty() || query.contains_key("code") {
            return Err("CHATGPT_PLAN_INVALID_CALLBACK".into());
        }
        {
            let mut state = session.result.lock().map_err(|_| "CHATGPT_PLAN_BUSY")?;
            pending_session(&session, &state)?;
            state.status = "error".into();
            state.error = Some("CHATGPT_PLAN_AUTH_DENIED".into());
        }
        session.callback_received.notify_one();
        close_session_window(&session);
        return Err("CHATGPT_PLAN_AUTH_DENIED".into());
    }
    let (code, client_id) = callback_credentials(&session, &query)?;
    {
        let mut state = session.result.lock().map_err(|_| "CHATGPT_PLAN_BUSY")?;
        pending_session(&session, &state)?;
        state.status = "exchanging".into();
        state.error = None;
    }
    session.callback_received.notify_one();
    tokio::spawn(async move {
        let remaining = session
            .expires_at
            .saturating_sub(chrono::Utc::now().timestamp())
            .max(0) as u64;
        let result = tokio::select! {
            _ = session.cancel.notified() => return,
            result = tokio::time::timeout(Duration::from_secs(remaining), exchange_and_save(&session, &code, &client_id)) => result,
        };
        match result {
            Ok(Ok(())) => close_session_window(&session),
            Ok(Err(error)) => finish_login_error(&session, error),
            Err(_) => finish_login_error(&session, "CHATGPT_PLAN_LOGIN_EXPIRED".into()),
        }
    });
    Ok(())
}

pub async fn submit_callback_url(login_id: &str, callback_url: &str) -> Result<(), String> {
    let session = find_session(login_id)?;
    let query = parse_callback_url(&session, callback_url)?;
    accept_callback(session, query)
}

pub fn open_incognito_window(app: &tauri::AppHandle, login_id: &str) -> Result<(), String> {
    let session = find_session(login_id)?;
    {
        let state = session.result.lock().map_err(|_| "CHATGPT_PLAN_BUSY")?;
        pending_session(&session, &state)?;
    }
    let authorize =
        url::Url::parse(&session.authorize_url).map_err(|_| "CHATGPT_PLAN_INVALID_CALLBACK")?;
    let callback =
        url::Url::parse(&session.redirect_uri).map_err(|_| "CHATGPT_PLAN_INVALID_CALLBACK")?;
    crate::modules::codex_oauth::build_incognito_oauth_window(
        app,
        &format!("chatgpt-plan-oauth-{}", session.id),
        "ChatGPT OAuth",
        authorize,
        callback,
        None,
        &session.id,
    )
    .map_err(|_| "CHATGPT_PLAN_OPEN_BROWSER")?;
    // Completion/cancellation may race with native window creation.
    let active = session
        .result
        .lock()
        .map_err(|_| "CHATGPT_PLAN_BUSY")?
        .status
        .clone();
    if !matches!(active.as_str(), "pending" | "exchanging") {
        close_session_window(&session);
    }
    Ok(())
}

pub async fn poll_login(id: &str) -> Result<ChatgptPlanLoginStatus, String> {
    let session = find_session(id)?;
    let state = session
        .result
        .lock()
        .map_err(|_| "CHATGPT_PLAN_BUSY".to_string())?;
    Ok(ChatgptPlanLoginStatus {
        status: state.status.clone(),
        account: state.account.clone(),
        error: state.error.clone(),
    })
}

pub async fn cancel_login(id: &str) -> Result<(), String> {
    let session = find_session(id)?;
    cancel_session(&session);
    Ok(())
}

fn cancel_session(session: &Session) {
    if let Ok(mut result) = session.result.lock() {
        if matches!(result.status.as_str(), "pending" | "exchanging") {
            result.status = "cancelled".into();
            result.error = Some("CHATGPT_PLAN_LOGIN_CANCELLED".into());
            session.cancel.notify_one();
            session.callback_received.notify_one();
        }
    }
    close_session_window(session);
}

pub(super) async fn cancel_registration(id: &str) {
    let sessions: Vec<_> = SESSIONS
        .lock()
        .map(|sessions| {
            sessions
                .values()
                .filter(|session| session.registration_id == id)
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    for session in sessions {
        cancel_session(&session);
    }
}

async fn read_request(stream: &mut TcpStream) -> Result<String, String> {
    let mut bytes = Vec::new();
    loop {
        let mut chunk = [0_u8; 2048];
        let count = stream
            .read(&mut chunk)
            .await
            .map_err(|_| "CHATGPT_PLAN_INVALID_CALLBACK".to_string())?;
        if count == 0 {
            return Err("CHATGPT_PLAN_INVALID_CALLBACK".into());
        }
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.len() > 16_384 {
            return Err("CHATGPT_PLAN_INVALID_CALLBACK".into());
        }
        if bytes.windows(4).any(|part| part == b"\r\n\r\n") {
            return String::from_utf8(bytes)
                .map_err(|_| "CHATGPT_PLAN_INVALID_CALLBACK".to_string());
        }
    }
}

fn parse_callback(session: &Session, request: &str) -> Result<HashMap<String, String>, String> {
    let invalid = || "CHATGPT_PLAN_INVALID_CALLBACK".to_string();
    let mut lines = request.lines();
    let parts: Vec<_> = lines
        .next()
        .ok_or_else(invalid)?
        .split_whitespace()
        .collect();
    if parts.len() != 3 || parts[0] != "GET" || !matches!(parts[2], "HTTP/1.1" | "HTTP/1.0") {
        return Err(invalid());
    }
    let callback = url::Url::parse(&session.redirect_uri).map_err(|_| invalid())?;
    let expected_host = format!("127.0.0.1:{}", callback.port().ok_or_else(invalid)?);
    let hosts: Vec<_> = lines
        .take_while(|line| !line.is_empty())
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("host").then_some(value.trim())
        })
        .collect();
    if hosts.len() != 1 || hosts[0] != expected_host || !parts[1].starts_with("/auth/callback?") {
        return Err(invalid());
    }
    let url = callback.join(parts[1]).map_err(|_| invalid())?;
    parse_callback_url(session, url.as_str())
}

fn parse_callback_url(session: &Session, raw: &str) -> Result<HashMap<String, String>, String> {
    let invalid = || "CHATGPT_PLAN_INVALID_CALLBACK".to_string();
    let raw = raw.trim();
    if raw.len() > 16_384 || raw.chars().any(char::is_control) {
        return Err(invalid());
    }
    let url = url::Url::parse(raw).map_err(|_| invalid())?;
    let expected = url::Url::parse(&session.redirect_uri).map_err(|_| invalid())?;
    if url.origin() != expected.origin()
        || url.path() != expected.path()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    let mut query = HashMap::new();
    for (key, value) in url.query_pairs() {
        if query.insert(key.into_owned(), value.into_owned()).is_some() {
            return Err(invalid());
        }
    }
    if query.get("state").map(String::as_str) != Some(session.state.as_str()) {
        return Err(invalid());
    }
    Ok(query)
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn callback_html(locale: &str, received: bool) -> String {
    let key = if received {
        "chatgptPlan.callbackReceived"
    } else {
        "chatgptPlan.callbackFailed"
    };
    let text = crate::modules::i18n::translate(locale, key, &[]);
    format!(
        "<!doctype html><html lang=\"{}\"><meta charset=utf-8><meta name=viewport content=\"width=device-width, initial-scale=1\"><title>Cockpit Tools</title><body><h1>Cockpit Tools</h1><p>{}</p></body></html>",
        escape_html(locale),
        escape_html(&text),
    )
}

async fn respond(stream: &mut TcpStream, received: bool) {
    // Both the configuration cache and the bundled locale catalog initialize
    // lazily; their first load must not perform I/O or parse every locale on an
    // async runtime worker.
    let body = storage::blocking(move || {
        let locale = crate::modules::config::get_user_config().language;
        let locale = if locale.trim().is_empty() {
            "en"
        } else {
            locale.as_str()
        };
        Ok(callback_html(locale, received))
    })
    .await
    .unwrap_or_else(|_| "Cockpit Tools".to_string());
    let response = format!("HTTP/1.1 {}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'\r\nConnection: close\r\n\r\n{}", if received { "200 OK" } else { "400 Bad Request" }, body.len(), body);
    let _ = tokio::time::timeout(
        Duration::from_secs(2),
        stream.write_all(response.as_bytes()),
    )
    .await;
}

async fn run_listener(session: &Arc<Session>, listener: TcpListener) -> Result<(), String> {
    loop {
        let (mut stream, peer) = tokio::select! {
            _ = session.callback_received.notified() => return Ok(()),
            result = listener.accept() => result.map_err(|_| "CHATGPT_PLAN_INVALID_CALLBACK".to_string())?,
        };
        if !peer.ip().is_loopback() {
            continue;
        }
        let query = tokio::select! {
            _ = session.callback_received.notified() => return Ok(()),
            result = tokio::time::timeout(Duration::from_secs(3), read_request(&mut stream)) => {
                match result {
                    Ok(Ok(request)) => parse_callback(session, &request),
                    _ => Err("CHATGPT_PLAN_INVALID_CALLBACK".into()),
                }
            }
        };
        let query = match query {
            Ok(query) => query,
            Err(_) => {
                respond(&mut stream, false).await;
                continue;
            }
        };
        match accept_callback(session.clone(), query) {
            Ok(()) => {
                respond(&mut stream, true).await;
                return Ok(());
            }
            Err(_) => {
                respond(&mut stream, false).await;
                // Invalid callback input never consumes a pending session.
            }
        }
    }
}

fn callback_credentials(
    session: &Session,
    query: &HashMap<String, String>,
) -> Result<(String, String), String> {
    let code = query
        .get("code")
        .filter(|code| !code.is_empty())
        .ok_or("CHATGPT_PLAN_INVALID_CALLBACK")?;
    let client_id = if let Some(selected) = &session.selected {
        if query
            .get("client_id")
            .is_some_and(|client| client != &selected.summary.client_id)
        {
            return Err("CHATGPT_PLAN_REGISTRATION_CHANGED".into());
        }
        selected.summary.client_id.clone()
    } else {
        query
            .get("client_id")
            .filter(|client| !client.is_empty() && client.as_str() != DYNAMIC_CLIENT)
            .ok_or("CHATGPT_PLAN_INVALID_CALLBACK")?
            .clone()
    };
    if client_id.len() > 512 || code.len() > 8192 {
        return Err("CHATGPT_PLAN_INVALID_CALLBACK".into());
    }
    Ok((code.clone(), client_id))
}

async fn exchange_and_save(
    session: &Arc<Session>,
    code: &str,
    client_id: &str,
) -> Result<(), String> {
    let tokens = http::token(&[
        ("grant_type", "authorization_code"),
        ("client_id", client_id),
        ("code", code),
        ("code_verifier", session.verifier.as_str()),
        ("redirect_uri", session.redirect_uri.as_str()),
        ("resource", http::RESOURCE),
    ])
    .await;
    let tokens = match tokens {
        Ok(tokens) => tokens,
        Err(error) => {
            if error == "CHATGPT_PLAN_REAUTH_REQUIRED" && session.selected.is_none() {
                save_registration_draft(session, client_id).await?;
            }
            return Err(error);
        }
    };
    let scope = tokens
        .scope
        .as_deref()
        .ok_or("CHATGPT_PLAN_PERMISSION_REQUIRED")?;
    let plan_enabled = http::scopes(scope).is_ok();
    let scopes = scope.split_whitespace().map(str::to_string).collect();
    let id_token = tokens
        .id_token
        .filter(|token| !token.is_empty())
        .ok_or("CHATGPT_PLAN_INVALID_ID_TOKEN")?;
    let identity = http::validate_id_token(&id_token, client_id, Some(&session.nonce)).await?;
    if session.selected.as_ref().is_some_and(|record| {
        !record.summary.subject.is_empty() && record.summary.subject != identity.sub
    }) {
        return Err("CHATGPT_PLAN_IDENTITY_MISMATCH".into());
    }
    let credentials = Credentials {
        client_id: client_id.to_string(),
        subject: identity.sub.clone(),
        access_token: tokens.access_token,
        refresh_token: tokens
            .refresh_token
            .filter(|token| !token.is_empty())
            .ok_or("CHATGPT_PLAN_REAUTH_REQUIRED")?,
        id_token,
        scopes,
        expires_at: chrono::Utc::now().timestamp() + tokens.expires_in,
        earliest_refresh_at: tokens.earliest_refresh_at,
    };
    let _guard = storage::lock(&session.registration_id).await?;
    let session = session.clone();
    storage::blocking(move || {
        // Cancellation and durable commit serialize at this short final section.
        let mut state = session
            .result
            .lock()
            .map_err(|_| "CHATGPT_PLAN_BUSY".to_string())?;
        if state.status != "exchanging" {
            return Err("CHATGPT_PLAN_LOGIN_CANCELLED".into());
        }
        let now = chrono::Utc::now().timestamp();
        if now >= session.expires_at {
            return Err("CHATGPT_PLAN_LOGIN_EXPIRED".into());
        }
        let mut record = match &session.selected {
            Some(selected) => {
                let current = storage::load(&session.registration_id)?;
                if current.authorization_revision != selected.authorization_revision {
                    return Err("CHATGPT_PLAN_REGISTRATION_CHANGED".into());
                }
                current
            }
            None => {
                let email = identity
                    .email
                    .clone()
                    .filter(|email| email.len() <= 320 && !email.chars().any(char::is_control));
                let label = format!(
                    "{} · {}",
                    email.as_deref().unwrap_or("ChatGPT"),
                    &session.registration_id[..8]
                );
                Registration {
                    summary: ChatgptPlanAccountSummary {
                        id: session.registration_id.clone(),
                        label,
                        email,
                        subject: credentials.subject.clone(),
                        client_id: credentials.client_id.clone(),
                        source_account_id: None,
                        source_binding_valid: false,
                        source_binding_error: None,
                        connected: false,
                        needs_reauth: false,
                        last_checked_at: None,
                        check_error: None,
                        last_service_mode: None,
                        verified_at: None,
                        verified_model: None,
                        expires_at: None,
                        created_at: now,
                        updated_at: now,
                    },
                    revision: uuid::Uuid::new_v4().to_string(),
                    authorization_revision: uuid::Uuid::new_v4().to_string(),
                    encrypted_credentials: None,
                    revocation_confirmed: false,
                    source_binding: None,
            source_binding_required: false,
            model_catalog: Vec::new(),
                }
            }
        };
        // Historical model-test metadata is cleared; OAuth identity/scopes authorize use.
        record.summary.last_checked_at = None;
        record.summary.check_error = None;
        record.summary.verified_at = None;
        record.summary.verified_model = None;
        if record.summary.subject.is_empty() {
            record.summary.subject = credentials.subject.clone();
            record.summary.email = identity
                .email
                .filter(|email| email.len() <= 320 && !email.chars().any(char::is_control));
            record.summary.label = format!(
                "{} · {}",
                record.summary.email.as_deref().unwrap_or("ChatGPT"),
                &record.summary.id[..8]
            );
        }
        record.set_credentials(&credentials)?;
        record.authorization_revision = uuid::Uuid::new_v4().to_string();
        record.summary.needs_reauth = !plan_enabled;
        super::status::save_record(&record)?;
        super::status::emit_updated();
        state.status = "complete".into();
        state.account = Some(record.summary);
        Ok(())
    })
    .await
}

async fn save_registration_draft(session: &Arc<Session>, client_id: &str) -> Result<(), String> {
    let _guard = storage::lock(&session.registration_id).await?;
    let session = session.clone();
    let client_id = client_id.to_string();
    storage::blocking(move || {
        let mut state = session
            .result
            .lock()
            .map_err(|_| "CHATGPT_PLAN_BUSY".to_string())?;
        if state.status != "exchanging" {
            return Err("CHATGPT_PLAN_LOGIN_CANCELLED".into());
        }
        let now = chrono::Utc::now().timestamp();
        let record = Registration {
            summary: ChatgptPlanAccountSummary {
                id: session.registration_id.clone(),
                label: format!("ChatGPT · {}", &session.registration_id[..8]),
                email: None,
                subject: String::new(),
                client_id,
                source_account_id: None,
                source_binding_valid: false,
                source_binding_error: None,
                connected: false,
                needs_reauth: true,
                last_checked_at: None,
                check_error: None,
                last_service_mode: None,
                verified_at: None,
                verified_model: None,
                expires_at: None,
                created_at: now,
                updated_at: now,
            },
            revision: uuid::Uuid::new_v4().to_string(),
            authorization_revision: uuid::Uuid::new_v4().to_string(),
            encrypted_credentials: None,
            revocation_confirmed: false,
            source_binding: None,
            source_binding_required: false,
            model_catalog: Vec::new(),
        };
        storage::save(&record)?;
        state.account = Some(record.summary);
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        Session {
            id: uuid::Uuid::new_v4().to_string(),
            registration_id: uuid::Uuid::new_v4().to_string(),
            selected: None,
            state: "state".into(),
            nonce: "nonce".into(),
            verifier: "verifier".into(),
            redirect_uri: "http://127.0.0.1:54321/auth/callback".into(),
            authorize_url: "https://auth.openai.com/api/accounts/authorize".into(),
            callback_received: Notify::new(),
            expires_at: i64::MAX,
            result: Mutex::new(ChatgptPlanLoginStatus {
                status: "pending".into(),
                account: None,
                error: None,
            }),
            cancel: Notify::new(),
        }
    }

    #[test]
    fn pkce_and_initial_registration_parameters_are_bound_to_loopback() {
        let session = session();
        let host_id = "7c0f57d0-49ca-4bca-a15e-6d446d366dee";
        let url = url::Url::parse(&authorize_url(&session, host_id, None).unwrap()).unwrap();
        let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query["client_id"], DYNAMIC_CLIENT);
        assert_eq!(query["agent_name_hint"], "Cockpit Tools");
        assert_eq!(query["resource"], http::RESOURCE);
        assert_eq!(query["ext_agent_host_id"], format!("urn:uuid:{host_id}"));
        assert_eq!(query["redirect_uri"], session.redirect_uri);
        assert_eq!(
            query["code_challenge"],
            URL_SAFE_NO_PAD.encode(Sha256::digest(b"verifier"))
        );
        assert_eq!(query["code_challenge_method"], "S256");
    }

    #[test]
    fn callback_rejects_wrong_state_duplicate_parameters_and_dns_rebinding() {
        let session = session();
        let valid = "GET /auth/callback?state=state&code=code&client_id=oaiapp_test HTTP/1.1\r\nHost: 127.0.0.1:54321\r\n\r\n";
        assert!(parse_callback(&session, valid).is_ok());
        for invalid in [
            valid.replace("state=state", "state=other"),
            valid.replace("code=code", "code=code&code=again"),
            valid.replace("127.0.0.1:54321", "evil.example"),
            valid.replace("/auth/callback?", "/callback?"),
        ] {
            assert!(parse_callback(&session, &invalid).is_err());
        }
        let mut query = parse_callback(&session, valid).unwrap();
        query.remove("client_id");
        assert!(callback_credentials(&session, &query).is_err());
        query.insert("client_id".into(), DYNAMIC_CLIENT.into());
        assert!(callback_credentials(&session, &query).is_err());
    }

    #[test]
    fn cancelling_attempt_prevents_late_completion() {
        let session = session();
        cancel_session(&session);
        assert_eq!(session.result.lock().unwrap().status, "cancelled");
        cancel_session(&session);
        assert_eq!(session.result.lock().unwrap().status, "cancelled");
    }

    #[test]
    fn pasted_callbacks_require_exact_redirect_and_do_not_consume_invalid_input() {
        let session = Arc::new(session());
        let valid =
            "http://127.0.0.1:54321/auth/callback?state=state&code=code&client_id=oaiapp_test";
        assert!(parse_callback_url(&session, valid).is_ok());
        for invalid in [
            valid.replace("state=state", "state=other"),
            valid.replace("127.0.0.1", "localhost"),
            valid.replace("54321", "54322"),
            valid.replace("http://", "https://"),
            valid.replace("/auth/callback", "/other"),
            valid.replace("127.0.0.1", "user@127.0.0.1"),
            format!("{valid}&code=again"),
            format!("{valid}#fragment"),
        ] {
            assert!(parse_callback_url(&session, &invalid).is_err());
        }
        let mut query = parse_callback_url(&session, valid).unwrap();
        query.remove("client_id");
        assert!(accept_callback(session.clone(), query).is_err());
        assert_eq!(session.result.lock().unwrap().status, "pending");
        let query = HashMap::from([
            ("state".into(), "state".into()),
            ("error".into(), "access_denied".into()),
        ]);
        assert_eq!(
            accept_callback(session.clone(), query).unwrap_err(),
            "CHATGPT_PLAN_AUTH_DENIED"
        );
        assert_eq!(session.result.lock().unwrap().status, "error");
        assert_eq!(
            session.result.lock().unwrap().error.as_deref(),
            Some("CHATGPT_PLAN_AUTH_DENIED")
        );
    }

    #[test]
    fn expired_cancelled_and_exchanging_sessions_cannot_accept_another_callback() {
        let mut expired = session();
        expired.expires_at = 0;
        let expired = Arc::new(expired);
        let query = HashMap::from([
            ("state".into(), "state".into()),
            ("code".into(), "code".into()),
            ("client_id".into(), "oaiapp_test".into()),
        ]);
        assert_eq!(
            accept_callback(expired, query.clone()).unwrap_err(),
            "CHATGPT_PLAN_LOGIN_EXPIRED"
        );
        let cancelled = Arc::new(session());
        cancel_session(&cancelled);
        assert_eq!(
            accept_callback(cancelled, query.clone()).unwrap_err(),
            "CHATGPT_PLAN_LOGIN_CANCELLED"
        );
        let exchanging = Arc::new(session());
        exchanging.result.lock().unwrap().status = "exchanging".into();
        assert_eq!(
            accept_callback(exchanging, query).unwrap_err(),
            "CHATGPT_PLAN_BUSY"
        );
    }

    #[test]
    fn callback_page_explains_receipt_and_failure_in_the_selected_language() {
        let received = callback_html("zh-CN", true);
        assert!(received.contains("授权回调已接收，请返回 Cockpit Tools 查看结果。"));
        assert!(!received.contains("✓"));
        let failed = callback_html("zh-CN", false);
        assert!(failed.contains("回调无效或授权未完成，请返回 Cockpit Tools 重试。"));
        assert!(callback_html("en-US", true).contains("Return to Cockpit Tools"));
    }

    #[test]
    fn callback_html_escapes_text_and_language_without_reflecting_request_values() {
        assert_eq!(escape_html("<&>\"'"), "&lt;&amp;&gt;&quot;&#39;");
        let page = callback_html("\"><script>alert(1)</script>", true);
        assert!(!page.contains("<script>"));
        assert!(page.contains("&lt;script&gt;"));
        assert!(!page.contains("state="));
        assert!(!page.contains("code="));
        assert!(!page.contains("client_id="));
    }
}
