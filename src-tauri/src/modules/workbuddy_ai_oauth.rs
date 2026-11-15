use std::{sync::Mutex, time::Duration};

use reqwest::{Client, RequestBuilder, StatusCode};
use serde_json::{json, Value};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::models::workbuddy::WorkbuddyOAuthStartResponse;

const ORIGIN: &str = "https://www.workbuddy.ai";
const HOST: &str = "www.workbuddy.ai";
const TIMEOUT: Duration = Duration::from_secs(300);
const POLL_INTERVAL: Duration = Duration::from_secs(1);
const START_REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_TRANSIENT_FAILURES: u8 = 3;

struct PendingLogin {
    id: String,
    state: String,
    client: Client,
    deadline: Instant,
    cancel: CancellationToken,
    polling: bool,
}

#[derive(Default)]
struct LoginState {
    generation: u64,
    pending: Option<PendingLogin>,
}

static LOGIN: Mutex<LoginState> = Mutex::new(LoginState {
    generation: 0,
    pending: None,
});

fn client() -> Result<Client, String> {
    client_with_connect_timeout(Duration::from_secs(3))
}

fn client_with_connect_timeout(connect_timeout: Duration) -> Result<Client, String> {
    Client::builder()
        .connect_timeout(connect_timeout)
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
        .build()
        .map_err(|error| error.to_string())
}

fn anonymous(request: RequestBuilder) -> RequestBuilder {
    request
        .header("X-No-Authorization", "true")
        .header("X-No-User-Id", "true")
        .header("X-No-Enterprise-Id", "true")
        .header("X-No-Department-Info", "true")
}

enum RequestError {
    Transport(reqwest::Error),
    Http(StatusCode),
}

impl RequestError {
    fn transient(&self) -> bool {
        match self {
            Self::Transport(error) => {
                error.is_timeout()
                    || error.is_connect()
                    || (error.status().is_none() && (error.is_request() || error.is_body()))
            }
            Self::Http(status) => matches!(status.as_u16(), 408 | 429 | 500 | 502 | 503 | 504),
        }
    }

    fn detail(&self) -> String {
        match self {
            Self::Transport(error) => {
                use std::error::Error;
                let mut detail = error.to_string();
                let mut source = error.source();
                while let Some(cause) = source {
                    detail.push_str(&format!(": {cause}"));
                    source = cause.source();
                }
                detail
            }
            Self::Http(status) => format!("HTTP {status}"),
        }
    }

    fn oauth_message(&self, stage: &str) -> String {
        let detail = self.detail();
        crate::modules::logger::log_warn(&format!("[WorkBuddy AI OAuth] {stage}: {detail}"));
        match self {
            Self::Transport(error) if error.is_timeout() => "WORKBUDDY_AI_NETWORK_TIMEOUT".into(),
            Self::Transport(error)
                if error.is_connect() || error.is_request() || error.is_body() =>
            {
                "WORKBUDDY_AI_NETWORK_CONNECTION".into()
            }
            _ => detail,
        }
    }
}

async fn request_json(request: RequestBuilder) -> Result<(StatusCode, Value), RequestError> {
    let response = request
        .send()
        .await
        .map_err(|error| RequestError::Transport(error.without_url()))?;
    let status = response.status();
    if matches!(status.as_u16(), 408 | 429 | 500 | 502 | 503 | 504) {
        return Err(RequestError::Http(status));
    }
    match response.json().await {
        Ok(body) => Ok((status, body)),
        Err(error) if error.is_decode() && !status.is_success() => Err(RequestError::Http(status)),
        Err(error) => Err(RequestError::Transport(error.without_url())),
    }
}

async fn response(request: RequestBuilder) -> Result<(StatusCode, Value), String> {
    request_json(request).await.map_err(|error| error.detail())
}

async fn oauth_get(
    request: impl Fn() -> RequestBuilder,
    stage: &str,
) -> Result<(StatusCode, Value), RequestError> {
    for attempt in 1..=MAX_TRANSIENT_FAILURES {
        match request_json(request()).await {
            Ok(response) => return Ok(response),
            Err(error) if error.transient() && attempt < MAX_TRANSIENT_FAILURES => {
                crate::modules::logger::log_warn(&format!(
                    "[WorkBuddy AI OAuth] {stage}, retry {attempt}: {}",
                    error.detail()
                ));
                tokio::time::sleep(POLL_INTERVAL * u32::from(attempt)).await;
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!()
}

fn data(status: reqwest::StatusCode, body: &Value) -> Result<&Value, String> {
    let code = body.get("code").and_then(Value::as_i64);
    if status.is_success() && matches!(code, None | Some(0 | 200)) {
        if let Some(value) = body.get("data").filter(|value| value.is_object()) {
            return Ok(value);
        }
    }
    Err(format!(
        "WorkBuddy AI HTTP {status}, code {}",
        code.map_or_else(|| "—".into(), |value| value.to_string())
    ))
}

pub async fn start() -> Result<WorkbuddyOAuthStartResponse, String> {
    let generation = {
        let mut login = LOGIN.lock().map_err(|error| error.to_string())?;
        login.generation += 1;
        login.generation
    };
    let client = client()?;
    let request = || {
        anonymous(
            client
                .post(format!("{ORIGIN}/v2/plugin/auth/state"))
                .query(&[("platform", "workbuddy-ai")])
                .header("X-Domain", HOST)
                .json(&json!({})),
        )
    };
    // Only replay on failure to establish a connection. A read timeout may
    // arrive after the server has already created the authorization state.
    let (status, body) = tokio::time::timeout(START_REQUEST_TIMEOUT, async {
        match request_json(request()).await {
            Err(RequestError::Transport(error)) if error.is_connect() => {
                crate::modules::logger::log_warn(&format!(
                    "[WorkBuddy AI OAuth] auth/state reconnect: {}",
                    RequestError::Transport(error).detail(),
                ));
                request_json(request()).await
            }
            result => result,
        }
    })
    .await
    .map_err(|_| {
        crate::modules::logger::log_warn("[WorkBuddy AI OAuth] auth/state deadline exceeded");
        "WORKBUDDY_AI_NETWORK_TIMEOUT".to_string()
    })?
    .map_err(|error| error.oauth_message("auth/state"))?;
    let payload = data(status, &body)?;
    let state = payload["state"]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or("WORKBUDDY_AI_MISSING_STATE")?;
    let mut url = Url::parse(
        payload["authUrl"]
            .as_str()
            .ok_or("WORKBUDDY_AI_MISSING_AUTH_URL")?,
    )
    .map_err(|error| error.to_string())?;
    if url.scheme() != "https" || url.host_str() != Some(HOST) {
        return Err("WORKBUDDY_AI_INVALID_AUTH_URL".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    url.query_pairs_mut().append_pair("loginSessionId", &id);
    let mut login = LOGIN.lock().map_err(|error| error.to_string())?;
    if login.generation != generation {
        return Err("WORKBUDDY_AI_LOGIN_CANCELLED".into());
    }
    if let Some(old) = login.pending.replace(PendingLogin {
        id: id.clone(),
        state: state.into(),
        client,
        deadline: Instant::now() + TIMEOUT,
        cancel: CancellationToken::new(),
        polling: false,
    }) {
        old.cancel.cancel();
    }
    Ok(WorkbuddyOAuthStartResponse {
        login_id: id,
        verification_uri: url.to_string(),
        verification_uri_complete: Some(url.to_string()),
        expires_in: TIMEOUT.as_secs(),
        interval_seconds: POLL_INTERVAL.as_secs(),
    })
}

pub fn cancel(id: Option<&str>) -> Result<(), String> {
    let mut login = LOGIN.lock().map_err(|error| error.to_string())?;
    if id.is_none() {
        login.generation += 1;
    }
    let pending = &mut login.pending;
    if pending
        .as_ref()
        .is_some_and(|login| id.is_none_or(|id| id == login.id))
    {
        if let Some(login) = pending.take() {
            login.cancel.cancel();
        }
    }
    Ok(())
}

struct CompletionGuard<'a>(&'a str);
impl Drop for CompletionGuard<'_> {
    fn drop(&mut self) {
        let _ = cancel(Some(self.0));
    }
}

pub async fn complete(id: &str) -> Result<Value, String> {
    let (state, client, deadline, cancel_token) = {
        let mut state = LOGIN.lock().map_err(|error| error.to_string())?;
        let login = state
            .pending
            .as_mut()
            .filter(|login| login.id == id)
            .ok_or("WORKBUDDY_AI_LOGIN_CANCELLED")?;
        if login.polling {
            return Err("WORKBUDDY_AI_LOGIN_IN_PROGRESS".into());
        }
        login.polling = true;
        (
            login.state.clone(),
            login.client.clone(),
            login.deadline,
            login.cancel.clone(),
        )
    };
    let _guard = CompletionGuard(id);
    tokio::select! {
        biased;
        _ = cancel_token.cancelled() => Err("WORKBUDDY_AI_LOGIN_CANCELLED".into()),
        result = tokio::time::timeout_at(deadline, login(&client, &state)) => {
            result.map_err(|_| "WORKBUDDY_AI_LOGIN_TIMEOUT".to_string())?
        }
    }
}

fn domain(auth: &Value) -> &str {
    auth["domain"]
        .as_str()
        .filter(|value| !value.is_empty())
        .unwrap_or(HOST)
}

fn token(auth: &Value) -> Result<&str, String> {
    auth["accessToken"]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "WORKBUDDY_AI_MISSING_ACCESS_TOKEN".into())
}

fn normalize_auth(mut auth: Value) -> Value {
    let now = chrono::Utc::now().timestamp_millis();
    auth["lastRefreshTime"] = json!(now);
    for (absolute, relative) in [
        ("expiresAt", "expiresIn"),
        ("refreshExpiresAt", "refreshExpiresIn"),
    ] {
        if auth[absolute].is_null() {
            if let Some(seconds) = auth[relative].as_i64() {
                auth[absolute] = json!(now.saturating_add(seconds.saturating_mul(1000)));
            }
        }
    }
    auth
}

async fn poll(
    request: impl Fn() -> RequestBuilder,
    stage: &str,
    pending_code: i64,
    retry_account_auth: bool,
) -> Result<Value, String> {
    let mut retried = false;
    loop {
        tokio::time::sleep(POLL_INTERVAL).await;
        let (status, body) = match oauth_get(&request, stage).await {
            Err(RequestError::Http(StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN))
                if retry_account_auth && !retried =>
            {
                retried = true;
                continue;
            }
            result => result.map_err(|error| error.oauth_message(stage))?,
        };
        if body["code"].as_i64() == Some(pending_code) {
            continue;
        }
        if retry_account_auth && !retried && matches!(status.as_u16(), 401 | 403) {
            retried = true;
            continue;
        }
        return data(status, &body).cloned();
    }
}

async fn accounts(client: &Client, auth: &Value) -> Result<Vec<Value>, String> {
    let access_token = token(auth)?;
    let (status, body) = oauth_get(
        || {
            client
                .get(format!("{ORIGIN}/v2/plugin/accounts"))
                .header("X-Domain", domain(auth))
                .bearer_auth(access_token)
        },
        "accounts",
    )
    .await
    .map_err(|error| error.oauth_message("accounts"))?;
    data(status, &body)?["accounts"]
        .as_array()
        .cloned()
        .ok_or_else(|| "WORKBUDDY_AI_MISSING_ACCOUNTS".into())
}

fn uid(account: &Value) -> Option<&str> {
    account["uid"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
}

fn same_account(left: &Value, right: &Value) -> bool {
    uid(left).is_some()
        && uid(left) == uid(right)
        && left["enterpriseId"].as_str().filter(|v| !v.is_empty())
            == right["enterpriseId"].as_str().filter(|v| !v.is_empty())
}

fn session(auth: Value, selected: Value, all_accounts: Vec<Value>) -> Result<Value, String> {
    if uid(&selected).is_none() {
        return Err("WORKBUDDY_AI_MISSING_ACCOUNT_ID".into());
    }
    let mut account = selected;
    let mut snapshot = all_accounts;
    if let Some(previous) = snapshot.iter().find(|item| same_account(item, &account)) {
        if let (Some(current), Some(details)) = (account.as_object_mut(), previous.as_object()) {
            for (key, value) in details {
                current.entry(key.clone()).or_insert_with(|| value.clone());
            }
        }
    } else {
        snapshot.push(account.clone());
    }
    let available: Vec<_> = snapshot
        .iter()
        .filter(|item| item["pluginEnabled"].as_bool() == Some(true))
        .cloned()
        .collect();
    Ok(json!({"auth": auth, "account": account, "accounts": available, "allAccounts": snapshot}))
}

async fn login(client: &Client, state: &str) -> Result<Value, String> {
    let auth = normalize_auth(
        poll(
            || {
                anonymous(
                    client
                        .get(format!("{ORIGIN}/v2/plugin/auth/token"))
                        .query(&[("state", state)]),
                )
            },
            "auth/token",
            11217,
            false,
        )
        .await?,
    );
    let access_token = token(&auth)?;
    let selected = poll(
        || {
            client
                .get(format!("{ORIGIN}/v2/plugin/login/account"))
                .query(&[("state", state)])
                .bearer_auth(access_token)
                .header("X-Domain", domain(&auth))
                .header("X-No-User-Id", "true")
                .header("X-No-Enterprise-Id", "true")
                .header("X-No-Department-Info", "true")
        },
        "login/account",
        12151,
        true,
    )
    .await?;
    let snapshot = accounts(client, &auth).await?;
    session(auth, selected, snapshot)
}

pub async fn session_from_access_token(access_token: &str) -> Result<Value, String> {
    let client = client()?;
    let auth = json!({ "accessToken": access_token, "domain": HOST });
    let snapshot = accounts(&client, &auth).await?;
    let selected = snapshot
        .iter()
        .find(|item| item["lastLogin"].as_bool() == Some(true))
        .or_else(|| snapshot.first())
        .cloned()
        .ok_or("WORKBUDDY_AI_MISSING_ACCOUNTS")?;
    session(auth, selected, snapshot)
}

pub async fn renew_auth(session_value: &Value) -> Result<Value, String> {
    let client = client()?;
    let mut auth = session_value["auth"].clone();
    if let Some(refresh_token) = auth["refreshToken"]
        .as_str()
        .filter(|value| !value.is_empty())
    {
        let (status, body) = response(
            client
                .post(format!("{ORIGIN}/v2/plugin/auth/token/refresh"))
                .bearer_auth(token(&auth)?)
                .header("X-Domain", domain(&auth))
                .header("X-Refresh-Token", refresh_token)
                .header("X-Auth-Refresh-Source", "plugin")
                .json(&json!({})),
        )
        .await?;
        let mut renewed = normalize_auth(data(status, &body)?.clone());
        token(&renewed)?;
        if renewed["refreshToken"].is_null() {
            renewed["refreshToken"] = auth["refreshToken"].clone();
        }
        if renewed["domain"].is_null() {
            renewed["domain"] = auth["domain"].clone();
        }
        auth = renewed;
    }
    let mut renewed = session_value.clone();
    renewed["auth"] = auth;
    Ok(renewed)
}

pub async fn credit_usage(session: &Value) -> Result<Value, String> {
    let auth = &session["auth"];
    let account = &session["account"];
    let headers = super::workbuddy_oauth::billing_headers(
        token(auth)?,
        uid(account),
        account["enterpriseId"].as_str().filter(|id| !id.is_empty()),
        auth["domain"].as_str().filter(|value| !value.is_empty()),
        "en",
    )?;
    let client = client_with_connect_timeout(Duration::from_secs(6))?;
    if account["enterpriseId"]
        .as_str()
        .is_some_and(|id| !id.is_empty())
    {
        let (status, body) = request_json(
            client
                .post(format!(
                    "{ORIGIN}/v2/billing/meter/get-enterprise-user-usage"
                ))
                .headers(headers)
                .json(&json!({})),
        )
        .await
        .map_err(|error| match error {
            RequestError::Http(StatusCode::UNAUTHORIZED) => {
                "BUDDY_RESOURCE_HTTP: get-enterprise-user-usage 401".into()
            }
            error => error.detail(),
        })?;
        if status == StatusCode::UNAUTHORIZED {
            return Err("BUDDY_RESOURCE_HTTP: get-enterprise-user-usage 401".into());
        }
        data(status, &body)?;
        return super::workbuddy_oauth::wrap_enterprise_usage_as_resource(&body);
    }
    super::buddy_resource_api::fetch_international(&client, ORIGIN, headers).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{response::IntoResponse, routing::get, Json, Router};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    #[tokio::test]
    async fn polling_recovers_after_a_gateway_failure_and_pending_reply() {
        let calls = Arc::new(AtomicUsize::new(0));
        let app = Router::new().route(
            "/token",
            get({
                let calls = calls.clone();
                move || {
                    let attempt = calls.fetch_add(1, Ordering::SeqCst);
                    async move {
                        match attempt {
                            0 => (StatusCode::BAD_GATEWAY, "temporary gateway failure")
                                .into_response(),
                            1 => Json(json!({"code": 11217})).into_response(),
                            _ => Json(json!({"code": 0, "data": {"accessToken": "fixture"}}))
                                .into_response(),
                        }
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/token", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = Client::builder().no_proxy().build().unwrap();

        let value = poll(|| client.get(&url), "auth/token", 11217, false)
            .await
            .unwrap();

        assert_eq!(value["accessToken"], "fixture");
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        server.abort();
    }

    #[tokio::test]
    async fn polling_reports_a_permanent_denial_immediately() {
        let calls = Arc::new(AtomicUsize::new(0));
        let app = Router::new().route(
            "/token",
            get({
                let calls = calls.clone();
                move || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    async { (StatusCode::FORBIDDEN, Json(json!({"code": 12005}))) }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/token", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = Client::builder().no_proxy().build().unwrap();

        let failure = poll(|| client.get(&url), "auth/token", 11217, false)
            .await
            .unwrap_err();

        assert!(failure.contains("12005"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        server.abort();
    }
}
