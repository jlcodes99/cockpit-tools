use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

use axum::{
    body::Body,
    extract::{DefaultBodyLimit, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use tokio::{
    net::TcpListener,
    sync::{watch, Semaphore},
    task::JoinHandle,
};

use super::{
    contract::{Caller, Host, ServiceError},
    identity::IdentityStore,
    lifecycle::{stopping_error, Activity, RequestTracker, StopSignal},
};

#[derive(Clone, Debug)]
pub struct Options {
    /// usize::MAX leaves conversation request sizes unrestricted by the gateway.
    pub max_body_bytes: usize,
    /// Per uploaded image file, rather than the sum of a multipart request.
    pub max_upload_bytes: usize,
    /// Ordinary Go responses are collected without a gateway size cap.
    pub max_response_bytes: usize,
    /// Independent Images API; OAuth images use ordinary Responses limits.
    pub max_image_response_bytes: usize,
    /// Codex executor SSE line budget.
    pub max_event_bytes: usize,
    /// Chat provider gateway SSE line budget.
    pub max_provider_event_bytes: usize,
    /// Zero preserves the host's per-account concurrency policy without another global cap.
    pub max_in_flight: usize,
    pub request_timeout: Duration,
    pub idle_timeout: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            max_body_bytes: usize::MAX,
            max_upload_bytes: 64 * 1024 * 1024,
            max_response_bytes: usize::MAX,
            max_image_response_bytes: 96 * 1024 * 1024,
            max_event_bytes: 50 * 1024 * 1024,
            max_provider_event_bytes: 4 * 1024 * 1024,
            max_in_flight: 0,
            request_timeout: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(180),
        }
    }
}

pub(super) struct AppState {
    pub host: Arc<dyn Host>,
    pub identity: Arc<IdentityStore>,
    pub options: Options,
    pub stop: StopSignal,
    activity: Arc<Activity>,
    capacity: Arc<Semaphore>,
}

impl AppState {
    pub(super) fn blocking_work(&self) -> super::lifecycle::BlockingWorkGuard {
        super::lifecycle::BlockingWorkGuard::new(self.activity.clone())
    }

    pub fn begin(&self) -> Result<RequestTracker, ServiceError> {
        if self.stop.stopped() || !self.activity.accepting.load(Ordering::SeqCst) {
            return Err(stopping_error());
        }
        let permit = if self.options.max_in_flight == 0 {
            None
        } else {
            Some(self.capacity.clone().try_acquire_owned().map_err(|_| {
                ServiceError::new(
                    429,
                    "service_busy",
                    "The API service has reached its concurrent request limit",
                )
            })?)
        };
        let tracker = RequestTracker::new(
            self.activity.clone(),
            self.host.clone(),
            self.stop.clone(),
            permit,
        );
        if self.stop.stopped() || !self.activity.accepting.load(Ordering::SeqCst) {
            return Err(stopping_error());
        }
        Ok(tracker)
    }

    pub async fn authorize(&self, headers: &HeaderMap) -> Result<Caller, ServiceError> {
        let bearer = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|value| value.split_once(' '))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
            .map(|(_, value)| value.trim())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ServiceError::new(401, "invalid_api_key", "A valid Bearer API key is required")
            })?;
        let mut stop = self.stop.clone();
        tokio::select! {
            _ = stop.wait() => Err(stopping_error()),
            result = tokio::time::timeout(self.options.request_timeout, self.host.authorize(bearer)) =>
                result.map_err(|_| ServiceError::new(504, "authorization_timeout", "API key validation timed out"))?,
        }
    }

    pub(super) async fn model_catalog(
        &self,
        caller: &Caller,
    ) -> Result<serde_json::Value, ServiceError> {
        let mut stop = self.stop.clone();
        tokio::select! {
            _ = stop.wait() => Err(stopping_error()),
            result = tokio::time::timeout(self.options.request_timeout, self.host.models(caller)) =>
                result.map_err(|_| ServiceError::new(504, "model_catalog_timeout", "Model catalog lookup timed out"))?,
        }
    }
}

pub struct Handle {
    pub addr: SocketAddr,
    host: Arc<dyn Host>,
    stop: watch::Sender<bool>,
    activity: Arc<Activity>,
    task: Option<JoinHandle<Result<(), std::io::Error>>>,
}

impl Handle {
    pub fn set_cooling_disabled(&self, disabled: bool) {
        self.host.set_cooling_disabled(disabled);
    }
    pub fn set_session_affinity_enabled(&self, enabled: bool) {
        self.host.set_session_affinity_enabled(enabled);
    }
    pub fn active_requests(&self) -> usize {
        self.activity.count.load(Ordering::Acquire)
    }
    pub fn pause_if_idle(&self) -> bool {
        self.activity.accepting.store(false, Ordering::SeqCst);
        if self.activity.count.load(Ordering::SeqCst) == 0 {
            return true;
        }
        self.activity.accepting.store(true, Ordering::SeqCst);
        false
    }

    pub fn is_finished(&self) -> bool {
        self.task.as_ref().is_none_or(JoinHandle::is_finished)
            && self.activity.tasks.load(Ordering::SeqCst) == 0
    }

    pub async fn shutdown(&mut self, grace: Duration) -> Result<(), String> {
        self.stop.send_replace(true);
        let deadline = tokio::time::Instant::now() + grace;
        if let Some(mut task) = self.task.take() {
            match tokio::time::timeout_at(deadline, &mut task).await {
                Ok(Ok(Ok(()))) => {}
                Ok(Ok(Err(error))) => return Err(error.to_string()),
                Ok(Err(error)) => return Err(error.to_string()),
                Err(_) => {
                    task.abort();
                    let _ = task.await;
                    return Err("Native API shutdown timed out".to_string());
                }
            }
        }
        while self.activity.tasks.load(Ordering::SeqCst) > 0 {
            let changed = self.activity.changed.notified();
            if self.activity.tasks.load(Ordering::SeqCst) == 0 {
                break;
            }
            if tokio::time::timeout_at(deadline, changed).await.is_err() {
                return Err("Native API connections did not stop before the deadline".to_string());
            }
        }
        Ok(())
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.stop.send_replace(true);
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

pub async fn start(
    addr: SocketAddr,
    host: Arc<dyn Host>,
    identity_path: PathBuf,
    options: Options,
) -> Result<Handle, String> {
    if options.max_body_bytes == 0 || options.max_upload_bytes == 0 || options.max_event_bytes == 0
    {
        return Err("Native API limits must be greater than zero".to_string());
    }
    let handoff = super::identity::begin_handoff(&identity_path)?;
    let (stop, receiver) = watch::channel(false);
    let identity = super::blocking::run(
        StopSignal(receiver.clone()),
        options.request_timeout,
        move || {
            let _handoff = handoff;
            let store = IdentityStore::open(identity_path.clone())
                .map_err(|message| ServiceError::new(500, "identity_state_error", message))?;
            store
                .import_handoff(
                    &super::identity::handoff_snapshot_path(&identity_path),
                    &super::identity::handoff_scope(&identity_path),
                )
                .map_err(|message| ServiceError::new(500, "identity_handoff_error", message))?;
            Ok(store)
        },
    )
    .await
    .map_err(|error| error.message)?;
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|error| error.to_string())?;
    let addr = listener.local_addr().map_err(|error| error.to_string())?;
    let activity = Arc::new(Activity::default());
    let state = Arc::new(AppState {
        host: host.clone(),
        identity: Arc::new(identity),
        capacity: Arc::new(Semaphore::new(options.max_in_flight)),
        options,
        stop: StopSignal(receiver.clone()),
        activity: activity.clone(),
    });
    let mut router = Router::new()
        .route("/v1/models", get(models))
        .route("/models", get(models))
        .route("/v1/cockpit/quota", get(super::diagnostics::management))
        .route(
            "/v1/cockpit/diagnostics/config",
            post(super::diagnostics::management),
        )
        .route(
            "/v1/cockpit/diagnostics/events",
            get(super::diagnostics::management),
        )
        .route(
            "/v1/cockpit/auth/reset",
            post(super::diagnostics::management),
        )
        .route(
            "/v1/cockpit/accounts/reset-scheduler",
            post(super::diagnostics::management),
        )
        .route(
            "/v1/cockpit/service-tier-default",
            post(super::diagnostics::management),
        )
        .route("/v1/messages", post(super::compat_http::handle))
        .route(
            "/v1/messages/count_tokens",
            post(super::compat_http::handle),
        )
        .route("/v1beta/models", get(super::compat_http::handle))
        .route(
            "/v1beta/models/{*action}",
            get(super::compat_http::handle).post(super::compat_http::handle),
        )
        .route("/api/version", get(super::compat_http::handle))
        .route("/api/tags", get(super::compat_http::handle))
        .route("/api/show", post(super::compat_http::handle))
        .route("/api/chat", post(super::compat_http::handle))
        .route("/v1/chat/completions", post(super::edge::chat_completions))
        .route("/chat/completions", post(super::edge::chat_completions))
        .route("/v1/completions", post(super::edge::completions))
        .route("/completions", post(super::edge::completions))
        .route(
            "/v1/responses",
            post(super::http::responses).get(super::websocket::upgrade),
        )
        .route(
            "/responses",
            post(super::http::responses).get(super::websocket::upgrade),
        )
        .route("/v1/responses/compact", post(super::http::compact))
        .route("/responses/compact", post(super::http::compact))
        .route("/v1/images/generations", post(super::images::generations))
        .route("/images/generations", post(super::images::generations))
        .route("/v1/images/edits", post(super::images::edits))
        .route("/images/edits", post(super::images::edits))
        .route(
            "/v1/chat/completions/v1/responses",
            post(super::http::responses),
        )
        .route(
            "/v1/chat/completions/v1/responses/compact",
            post(super::http::compact),
        );
    for path in [
        "/v1/alpha/search",
        "/backend-api/codex/alpha/search",
        "/v1/live",
        "/v1/realtime/calls",
        "/v1/realtime/client_secrets",
        "/v1/realtime/sessions",
        "/v1/realtime/transcription_sessions",
        "/v1/realtime/translations/client_secrets",
        "/backend-api/codex/realtime/calls",
        "/transcribe",
        "/v1/transcribe",
        "/backend-api/transcribe",
        "/backend-api/codex/transcribe",
        "/v1/audio/transcriptions",
        "/v1/audio/translations",
        "/v1/audio/speech",
        "/audio/transcriptions",
        "/audio/translations",
        "/audio/speech",
    ] {
        router = router.route(path, post(super::auxiliary::http));
    }
    for path in [
        "/v1/live/{call_id}",
        "/v1/realtime/calls/{call_id}",
        "/backend-api/codex/realtime/calls/{call_id}",
        "/backend-api/codex/{call_id}",
    ] {
        router = router.route(path, get(super::auxiliary::websocket));
    }
    for action in ["hangup", "accept", "reject", "refer"] {
        router = router.route(
            &format!("/v1/realtime/calls/{{call_id}}/{action}"),
            post(super::auxiliary::http),
        );
    }
    router = router
        .route(
            "/v1/realtime",
            post(super::auxiliary::http).get(super::auxiliary::websocket),
        )
        .route(
            "/v1/realtime/translations",
            post(super::auxiliary::http).get(super::auxiliary::http),
        );
    let router = router
        .fallback(|| async {
            ServiceError::new(
                404,
                "unsupported_endpoint",
                "This endpoint is not supported by the native Responses service",
            )
        })
        .layer(DefaultBodyLimit::max(
            state
                .options
                .max_body_bytes
                .max(state.options.max_upload_bytes),
        ))
        .layer(axum::middleware::from_fn(cors))
        .with_state(state);
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            StopSignal(receiver).wait().await;
        })
        .await
    });
    Ok(Handle {
        addr,
        host,
        stop,
        activity,
        task: Some(task),
    })
}

async fn cors(mut request: axum::extract::Request, next: axum::middleware::Next) -> Response {
    // Keep the public credential forms accepted by the existing service. Do
    // this before every route, including Responses, uploads and WS upgrades.
    if let Some(key) = client_api_key(&request) {
        if let Ok(value) = HeaderValue::from_str(&format!("Bearer {key}")) {
            request.headers_mut().insert(header::AUTHORIZATION, value);
        }
    }
    let requested_headers = request
        .headers()
        .get(header::ACCESS_CONTROL_REQUEST_HEADERS)
        .cloned();
    let mut response = if request.method() == axum::http::Method::OPTIONS {
        StatusCode::NO_CONTENT.into_response()
    } else {
        next.run(request).await
    };
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, PUT, PATCH, DELETE, OPTIONS"),
    );
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        requested_headers.unwrap_or_else(|| HeaderValue::from_static("*")),
    );
    response.headers_mut().insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static("x-request-id, retry-after, location, x-codex-turn-state"),
    );
    response
}

fn client_api_key(request: &axum::extract::Request) -> Option<String> {
    let headers = request.headers();
    if let Some(value) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        match value.split_once(' ') {
            Some((scheme, key)) if scheme.eq_ignore_ascii_case("bearer") => {
                if !key.trim().is_empty() {
                    return Some(key.trim().to_owned());
                }
            }
            _ => return Some(value.to_owned()),
        }
    }
    for name in ["x-goog-api-key", "x-api-key"] {
        if let Some(key) = headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            return Some(key.to_owned());
        }
    }
    let query = url::form_urlencoded::parse(request.uri().query().unwrap_or_default().as_bytes());
    for name in ["key", "auth_token"] {
        if let Some((_, key)) = query
            .clone()
            .find(|(key, value)| key == name && !value.trim().is_empty())
        {
            return Some(key.trim().to_owned());
        }
    }
    None
}

async fn models(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ServiceError> {
    let _request = state.begin()?;
    let caller = state.authorize(&headers).await?;
    state.model_catalog(&caller).await.map(Json)
}

impl IntoResponse for ServiceError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::BAD_GATEWAY);
        let mut response = (status, Json(super::output_error::json_error(&self))).into_response();
        if let Some(delay) = self.retry_after {
            let seconds = delay
                .as_secs()
                .saturating_add(u64::from(delay.subsec_nanos() != 0))
                .max(1);
            if let Ok(value) = HeaderValue::from_str(&seconds.to_string()) {
                response.headers_mut().insert(header::RETRY_AFTER, value);
            }
        }
        response
    }
}

pub(super) fn outbound_headers(headers: &HeaderMap) -> HeaderMap {
    let connection_fields: Vec<&str> = headers
        .get(header::CONNECTION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .collect();
    headers
        .iter()
        .filter(|(name, _)| {
            !name.as_str().starts_with("x-cockpit-")
                && !matches!(
                    name.as_str(),
                    "chatgpt-account-id"
                        | "x-api-key"
                        | "x-goog-api-key"
                        | "host"
                        | "authorization"
                        | "cookie"
                        | "content-length"
                        | "connection"
                        | "keep-alive"
                        | "proxy-authenticate"
                        | "proxy-authorization"
                        | "te"
                        | "trailer"
                        | "transfer-encoding"
                        | "upgrade"
                        | "accept-encoding"
                        | "sec-websocket-key"
                        | "sec-websocket-version"
                        | "sec-websocket-extensions"
                        | "sec-websocket-protocol"
                )
                && !connection_fields
                    .iter()
                    .any(|field| name.as_str().eq_ignore_ascii_case(field))
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

pub(super) fn downstream_response(status: StatusCode, headers: &HeaderMap, body: Body) -> Response {
    let mut response = Response::new(body);
    *response.status_mut() = status;
    let connection_scoped: std::collections::HashSet<String> = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|name| name.trim().to_ascii_lowercase())
        .collect();
    for (name, value) in headers {
        if !connection_scoped.contains(name.as_str())
            && ![
                "x-litellm-",
                "helicone-",
                "x-portkey-",
                "cf-aig-",
                "x-kong-",
                "x-bt-",
            ]
            .iter()
            .any(|prefix| name.as_str().starts_with(prefix))
            && !matches!(
                name.as_str(),
                "content-length"
                    | "content-encoding"
                    | "transfer-encoding"
                    | "connection"
                    | "keep-alive"
                    | "proxy-authenticate"
                    | "proxy-authorization"
                    | "te"
                    | "trailer"
                    | "set-cookie"
                    | "upgrade"
                    | "access-control-allow-origin"
                    | "access-control-allow-methods"
                    | "access-control-allow-headers"
                    | "access-control-allow-credentials"
                    | "access-control-expose-headers"
                    | "access-control-max-age"
                    | "x-cpa-trace-id"
            )
        {
            response.headers_mut().append(name.clone(), value.clone());
        }
    }
    response
}

#[cfg(test)]
mod shutdown_handoff_tests {
    use super::*;
    use futures_util::future::BoxFuture;
    use serde_json::Value;

    struct UnusedHost;

    impl Host for UnusedHost {
        fn authorize<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
            Box::pin(async { unreachable!() })
        }
        fn route<'a>(
            &'a self,
            _: &'a Caller,
            _: super::super::contract::RouteRequest,
        ) -> BoxFuture<'a, Result<super::super::contract::Route, ServiceError>> {
            Box::pin(async { unreachable!() })
        }
        fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
            Box::pin(async { unreachable!() })
        }
        fn record(&self, _: super::super::contract::RequestReport) -> BoxFuture<'_, ()> {
            Box::pin(async { unreachable!() })
        }
    }

    #[tokio::test]
    async fn shutdown_timeout_keeps_handle_alive_until_old_work_stops() {
        let (stop, _) = watch::channel(false);
        let activity = Arc::new(Activity::default());
        activity.tasks.store(1, Ordering::SeqCst);
        let mut handle = Handle {
            addr: "127.0.0.1:0".parse().unwrap(),
            host: Arc::new(UnusedHost),
            stop,
            activity: activity.clone(),
            task: Some(tokio::spawn(async { Ok(()) })),
        };
        assert!(handle.shutdown(Duration::from_millis(5)).await.is_err());
        assert!(
            !handle.is_finished(),
            "old work must remain visible to lifecycle refresh"
        );
        activity.tasks.store(0, Ordering::SeqCst);
        assert!(handle.shutdown(Duration::from_millis(5)).await.is_ok());
        assert!(handle.is_finished());
    }

    #[tokio::test]
    async fn host_default_has_no_global_cap_and_idle_connections_release_explicit_cap() {
        for limit in [0, 1] {
            let temp = tempfile::tempdir().unwrap();
            let (_, stopped) = watch::channel(false);
            let state = AppState {
                host: Arc::new(UnusedHost),
                identity: Arc::new(
                    IdentityStore::open(temp.path().join("identity.sqlite")).unwrap(),
                ),
                options: Options {
                    max_in_flight: limit,
                    ..Default::default()
                },
                stop: StopSignal(stopped),
                activity: Arc::new(Activity::default()),
                capacity: Arc::new(Semaphore::new(limit)),
            };
            if limit == 0 {
                let trackers = (0..128).map(|_| state.begin().unwrap()).collect::<Vec<_>>();
                assert_eq!(trackers.len(), 128);
                drop(trackers);
            } else {
                let mut first = state.begin().unwrap();
                assert_eq!(state.begin().err().unwrap().code, "service_busy");
                first.idle();
                let second = state.begin().unwrap();
                assert_eq!(first.start_turn().unwrap_err().code, "service_busy");
                drop(second);
                first.start_turn().unwrap();
            }
        }
    }

    #[tokio::test]
    async fn cancelled_identity_waiter_keeps_handoff_blocked_until_worker_finishes() {
        for timeout in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let (stop, stopped) = watch::channel(false);
            let activity = Arc::new(Activity::default());
            let state = Arc::new(AppState {
                host: Arc::new(UnusedHost),
                identity: Arc::new(
                    IdentityStore::open(temp.path().join("identity.sqlite")).unwrap(),
                ),
                options: Options {
                    request_timeout: if timeout {
                        Duration::from_millis(250)
                    } else {
                        Duration::from_secs(5)
                    },
                    ..Default::default()
                },
                stop: StopSignal(stopped),
                activity: activity.clone(),
                capacity: Arc::new(Semaphore::new(1)),
            });
            let (started, entered) = tokio::sync::oneshot::channel();
            let (release, released) = std::sync::mpsc::channel();
            let waiter = tokio::spawn(async move {
                super::super::blocking::run_tracked(&state, move || {
                    let _ = started.send(());
                    released.recv_timeout(Duration::from_secs(5)).unwrap();
                    Ok(())
                })
                .await
            });
            entered.await.unwrap();
            if !timeout {
                stop.send_replace(true);
            }
            assert!(waiter.await.unwrap().is_err());
            let mut handle = Handle {
                addr: "127.0.0.1:0".parse().unwrap(),
                host: Arc::new(UnusedHost),
                stop,
                activity: activity.clone(),
                task: Some(tokio::spawn(async { Ok(()) })),
            };
            assert_eq!(activity.tasks.load(Ordering::SeqCst), 1);
            assert!(handle.shutdown(Duration::from_millis(10)).await.is_err());
            assert!(!handle.is_finished());
            release.send(()).unwrap();
            handle.shutdown(Duration::from_secs(1)).await.unwrap();
            assert!(handle.is_finished());
        }
    }

    #[tokio::test]
    async fn timed_out_startup_keeps_its_identity_handoff_fence_until_worker_finishes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("identity.sqlite");
        let store = IdentityStore::open(path.clone()).unwrap();
        store
            .export_handoff(
                &super::super::identity::handoff_snapshot_path(&path),
                &super::super::identity::handoff_scope(&path),
            )
            .unwrap();
        // The importer will wait for this writer after its async startup waiter
        // has reached its deadline. No filesystem sleeps or live account needed.
        let writer = rusqlite::Connection::open(&path).unwrap();
        writer.execute_batch("BEGIN IMMEDIATE").unwrap();
        let options = Options {
            request_timeout: Duration::from_millis(100),
            ..Default::default()
        };
        let result = start(
            "127.0.0.1:0".parse().unwrap(),
            Arc::new(UnusedHost),
            path.clone(),
            options,
        )
        .await;
        assert!(result.is_err());
        assert!(super::super::identity::begin_handoff(&path).is_err());
        // A retry must fail immediately rather than starting a second importer.
        let retry = tokio::time::timeout(
            Duration::from_millis(100),
            start(
                "127.0.0.1:0".parse().unwrap(),
                Arc::new(UnusedHost),
                path.clone(),
                Options::default(),
            ),
        )
        .await
        .unwrap();
        assert!(retry.is_err());
        assert!(super::super::identity::begin_handoff(&temp.path().join("other.sqlite")).is_ok());
        writer.execute_batch("COMMIT").unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Ok(guard) = super::super::identity::begin_handoff(&path) {
                    drop(guard);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }
}
