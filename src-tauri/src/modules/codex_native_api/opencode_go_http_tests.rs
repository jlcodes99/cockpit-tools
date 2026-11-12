use super::{contract::*, start, Options};
use axum::http::{HeaderMap, Response, StatusCode};
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

struct OpenCodeFixture {
    client: reqwest::Client,
}

impl Host for OpenCodeFixture {
    fn authorize<'a>(&'a self, bearer: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async move {
            Ok(Caller {
                id: bearer.into(),
                label: "fixture".into(),
            })
        })
    }

    fn route<'a>(
        &'a self,
        caller: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            let mut headers = HeaderMap::new();
            let account_id = if caller.id == "local-retry" {
                let account = if request
                    .excluded_account_ids
                    .iter()
                    .any(|id| id == "primary")
                {
                    "backup"
                } else {
                    "primary"
                };
                headers.insert("x-fixture-account", account.parse().unwrap());
                account
            } else {
                "fixture"
            };
            let base_url = if request.model == "other" {
                "https://opencode.ai/v1"
            } else {
                "https://opencode.ai/zen/go/v1"
            };
            Ok(Route {
                account_id: account_id.into(),
                owner_id: account_id.into(),
                continuity_account_id: None,
                installation_profile: None,
                transport_identity: None,
                official_authorization_revision: None,
                official_registration_id: None,
                official_fallback_registration_id: None,
                official_fallback_authorization_revision: None,
                authorization_scope_fingerprint: None,
                official_fallback_failure: None,
                base_url: base_url.into(),
                model: request.model.clone(),
                headers,
                client: self.client.clone(),
                proxy_url: None,
                proxy_route: None,
                chatgpt: false,
                chatgpt_plan: false,
                wire_api: if request.model == "go-chat" {
                    WireApi::Chat
                } else {
                    WireApi::Responses
                },
                compact_endpoint: None,
                websocket_endpoint: None,
                default_service_tier: None,
                permit: None,
            })
        })
    }

    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { Ok(json!({"data": []})) })
    }

    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async {
            Ok(ExecutionPolicy {
                credential_retries: 2,
                ..Default::default()
            })
        })
    }

    fn record(&self, _: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

#[tokio::test]
async fn opencode_go_session_header_reaches_upstream_for_responses_chat_and_compact() {
    let cert = rcgen::generate_simple_self_signed(vec!["opencode.ai".into()]).unwrap();
    let root = reqwest::Certificate::from_der(cert.cert.der()).unwrap();
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![cert.cert.der().clone()],
        rustls::pki_types::PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()).into(),
    )
    .unwrap();
    config.alpn_protocols = vec![b"h2".to_vec()];
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let captured = Arc::new(Mutex::new(Vec::new()));
    let observed = captured.clone();
    let retry_attempts = Arc::new(Mutex::new(Vec::new()));
    let observed_retries = retry_attempts.clone();
    let upstream = tokio::spawn(async move {
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let (socket, _) = listener.accept().await.unwrap();
        let tls = acceptor.accept(socket).await.unwrap();
        let mut connection = h2::server::handshake(tls).await.unwrap();
        while let Some(request) = connection.accept().await {
            let observed = observed.clone();
            let observed_retries = observed_retries.clone();
            tokio::spawn(async move {
                let (request, mut respond) = request.unwrap();
                let path = request.uri().path().to_owned();
                let streaming = request
                    .headers()
                    .get("accept")
                    .is_some_and(|value| value == "text/event-stream");
                let session = request
                    .headers()
                    .get("x-opencode-session")
                    .map(|v| v.to_str().unwrap().to_owned());
                let primary = if let Some(account) = request.headers().get("x-fixture-account") {
                    let account = account.to_str().unwrap().to_owned();
                    let client_session = request.headers().get("session_id").cloned();
                    let (_, mut stream) = request.into_parts();
                    let mut bytes = Vec::new();
                    while let Some(data) = stream.data().await {
                        let data = data.unwrap();
                        stream.flow_control().release_capacity(data.len()).unwrap();
                        bytes.extend_from_slice(&data);
                    }
                    let body: Value = serde_json::from_slice(&bytes).unwrap();
                    let primary = account == "primary";
                    observed_retries.lock().unwrap().push((
                        path.clone(),
                        account,
                        session.clone(),
                        client_session,
                        body["prompt_cache_key"].clone(),
                    ));
                    primary
                } else {
                    observed
                        .lock()
                        .unwrap()
                        .push((path.clone(), session.clone()));
                    false
                };
                let (status, body) = if primary {
                    (
                        429,
                        json!({"error":{"code":"rate_limit_exceeded","message":"fixture limit"}}),
                    )
                } else if path.starts_with("/zen/go/") && session.is_none() {
                    (
                        400,
                        json!({"error":{"code":"MissingSessionID","message":"OpenCode Go needs a session header"}}),
                    )
                } else if path.ends_with("/compact") {
                    (
                        200,
                        json!({"object":"response.compaction","id":"compaction-fixture","output":[]}),
                    )
                } else if path.ends_with("/chat/completions") {
                    (
                        200,
                        json!({"id":"chat-fixture","object":"chat.completion","model":"go-chat","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}),
                    )
                } else {
                    (
                        200,
                        json!({"id":"response-fixture","object":"response","status":"completed","output":[]}),
                    )
                };
                let (content_type, payload) = if status == 200 && streaming {
                    let event = json!({"type":"response.completed","response":body});
                    (
                        "text/event-stream",
                        format!("event: response.completed\ndata: {event}\n\n"),
                    )
                } else {
                    ("application/json", body.to_string())
                };
                let response = Response::builder()
                    .status(status)
                    .header("content-type", content_type)
                    .body(())
                    .unwrap();
                respond
                    .send_response(response, false)
                    .unwrap()
                    .send_data(payload.into(), true)
                    .unwrap();
            });
        }
    });
    let upstream_client = reqwest::Client::builder()
        .no_proxy()
        .use_rustls_tls()
        .tls_built_in_root_certs(false)
        .add_root_certificate(root)
        .resolve("opencode.ai", address)
        .build()
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let mut gateway = start(
        "127.0.0.1:0".parse().unwrap(),
        Arc::new(OpenCodeFixture {
            client: upstream_client,
        }),
        temp.path().join("identity.sqlite"),
        Options::default(),
    )
    .await
    .unwrap();
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    for (path, model, key, session, explicit) in [
        (
            "responses",
            "go-responses",
            "local-a",
            Some("conversation-a"),
            None,
        ),
        (
            "responses/compact",
            "go-responses",
            "local-a",
            Some("conversation-a"),
            None,
        ),
        (
            "chat/completions",
            "go-chat",
            "local-a",
            Some("conversation-a"),
            None,
        ),
        (
            "responses",
            "go-responses",
            "local-b",
            Some("conversation-a"),
            None,
        ),
        (
            "responses",
            "go-responses",
            "local-a",
            Some("conversation-b"),
            None,
        ),
        (
            "responses",
            "go-responses",
            "local-a",
            None,
            Some("client-managed"),
        ),
        ("responses", "go-responses", "local-a", None, None),
        ("responses", "go-responses", "local-a", None, None),
        (
            "responses",
            "other",
            "local-a",
            Some("conversation-a"),
            None,
        ),
        (
            "responses",
            "go-stream",
            "local-a",
            Some("conversation-a"),
            None,
        ),
    ] {
        let input = if path == "chat/completions" {
            json!({"model":model,"messages":[{"role":"user","content":"fixture"}]})
        } else {
            json!({"model":model,"stream":model == "go-stream","input":[{"role":"user","content":"fixture"}]})
        };
        let mut request = client
            .post(format!("http://{}/v1/{path}", gateway.addr))
            .bearer_auth(key)
            .json(&input);
        if let Some(session) = session {
            request = request.header("session-id", session);
        }
        if let Some(value) = explicit {
            request = request.header("x-opencode-session", value);
        }
        let response = request.send().await.unwrap();
        let status = response.status();
        let body = response.text().await.unwrap();
        assert_eq!(status, StatusCode::OK, "{path}/{model}: {body}");
        if model == "go-stream" {
            assert!(body.contains("response.completed"), "{body}");
        }
    }
    for (path, input) in [
        ("responses", json!({"model":"go-responses","input":[]})),
        (
            "chat/completions",
            json!({"model":"go-chat","messages":[{"role":"user","content":"fixture"}]}),
        ),
    ] {
        for explicit in [false, true] {
            let mut input = input.clone();
            input["prompt_cache_key"] = json!("client-cache-key");
            let mut request = client
                .post(format!("http://{}/v1/{path}", gateway.addr))
                .bearer_auth("local-retry")
                .json(&input);
            if explicit {
                request = request
                    .header("session_id", format!("client-session-{path}"))
                    .header("x-opencode-session", "client-managed");
            }
            let response = request.send().await.unwrap();
            let status = response.status();
            let body = response.text().await.unwrap();
            assert_eq!(
                status,
                StatusCode::OK,
                "{path}, explicit={explicit}: {body}"
            );
        }
    }
    gateway.shutdown(Duration::from_secs(2)).await.unwrap();
    upstream.abort();
    let wire = captured.lock().unwrap();
    assert_eq!(wire.len(), 10);
    assert_eq!(wire[1].0, "/zen/go/v1/responses/compact");
    assert_eq!(wire[2].0, "/zen/go/v1/chat/completions");
    let conversation = wire[0].1.as_deref().unwrap();
    assert_ne!(conversation, "conversation-a");
    assert_eq!(wire[1].1.as_deref(), Some(conversation));
    assert_eq!(wire[2].1.as_deref(), Some(conversation));
    assert_ne!(wire[3].1.as_deref().unwrap(), conversation);
    assert_ne!(wire[4].1.as_deref().unwrap(), conversation);
    assert_eq!(wire[5].1.as_deref(), Some("client-managed"));
    assert_ne!(wire[6].1.as_deref().unwrap(), wire[7].1.as_deref().unwrap());
    assert_eq!(wire[8].1, None);
    assert_eq!(wire[9].1.as_deref(), Some(conversation));
    let retries = retry_attempts.lock().unwrap();
    assert_eq!(retries.len(), 8);
    for (index, attempts) in retries.chunks_exact(2).enumerate() {
        let [first, second] = attempts else {
            unreachable!()
        };
        let path = if index < 2 {
            "responses"
        } else {
            "chat/completions"
        };
        let expected_session = format!("client-session-{path}");
        assert_eq!(first.1, "primary");
        assert_eq!(second.1, "backup");
        assert!(first.2.is_some());
        assert_eq!(first.2, second.2);
        let explicit = index % 2 == 1;
        if explicit {
            assert_eq!(first.2.as_deref(), Some("client-managed"));
        }
        for (upstream_path, _, _, client_session, cache_key) in attempts {
            assert_eq!(upstream_path, &format!("/zen/go/v1/{path}"));
            assert_eq!(
                client_session.as_ref().map(|value| value.to_str().unwrap()),
                explicit.then_some(expected_session.as_str()),
            );
            assert_eq!(cache_key, "client-cache-key");
        }
    }
}
