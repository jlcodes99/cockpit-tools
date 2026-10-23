use super::*;
use crate::modules::{
    codex_api_identity::InstallationProfile,
    codex_native_api::{
        self as native,
        contract::{Caller, Host, Route, RouteRequest, WireApi},
    },
};
use axum::{http::HeaderValue, routing::get, Router};
use futures_util::future::BoxFuture;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message as ClientMessage};

fn profile() -> InstallationProfile {
    InstallationProfile {
        version: 1,
        scope: "installation-scope".into(),
        installation_id: "00000000-0000-4000-8000-000000000001".into(),
        revision: 1,
    }
}
fn route() -> Route {
    Route {
        account_id: "a".into(),
        owner_id: "owner-a".into(),
        continuity_account_id: None,
        installation_profile: Some(profile()),
        official_authorization_revision: None,
        official_registration_id: None,
        official_fallback_registration_id: None,
        official_fallback_authorization_revision: None,
        authorization_scope_fingerprint: None,
                official_fallback_failure: None,
        transport_identity: Some("stable-transport".into()),
        base_url: "http://127.0.0.1:1/v1".into(),
        model: "m".into(),
        headers: HeaderMap::from_iter([(
            "authorization".parse().unwrap(),
            HeaderValue::from_static("Bearer old"),
        )]),
        client: reqwest::Client::builder().no_proxy().build().unwrap(),
        proxy_url: None,
        proxy_route: None,
        chatgpt: false,
        chatgpt_plan: false,
        wire_api: WireApi::Responses,
        compact_endpoint: None,
        websocket_endpoint: None,
        default_service_tier: None,
        permit: None,
    }
}

#[test]
fn websocket_reuse_requires_matching_installation_authentication_and_transport_config() {
    let original = route();
    assert!(!websocket_transport_changed(&original, &original));
    let mutations: Vec<Box<dyn Fn(&mut Route)>> = vec![
        Box::new(|v| v.account_id = "b".into()),
        Box::new(|v| v.continuity_account_id = Some("chatgpt_plan_other-grant".into())),
        Box::new(|v| v.official_registration_id = Some("other-grant".into())),
        Box::new(|v| v.owner_id = "another-owner".into()),
        Box::new(|v| v.transport_identity = Some("new-config".into())),
        Box::new(|v| v.installation_profile.as_mut().unwrap().revision += 1),
        Box::new(|v| {
            v.installation_profile.as_mut().unwrap().installation_id =
                "00000000-0000-4000-8000-000000000002".into()
        }),
        Box::new(|v| v.proxy_url = Some("http://127.0.0.1:9".into())),
        Box::new(|v| v.base_url = "http://127.0.0.1:1/another-realm".into()),
        Box::new(|v| v.wire_api = WireApi::Chat),
        Box::new(|v| {
            v.headers
                .insert("authorization", HeaderValue::from_static("Bearer fresh"));
        }),
        Box::new(|v| {
            v.websocket_endpoint = Some(super::super::contract::EndpointOverride {
                base_url: v.base_url.clone(),
                headers: HeaderMap::new(),
            })
        }),
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut next = original.clone();
        mutate(&mut next);
        assert!(
            websocket_transport_changed(&original, &next),
            "mutation {index}"
        );
    }
    let mut legacy = original.clone();
    legacy.transport_identity = None;
    legacy.installation_profile = None;
    let mut refreshed = legacy.clone();
    refreshed
        .headers
        .insert("authorization", HeaderValue::from_static("Bearer fresh"));
    assert!(websocket_transport_changed(&legacy, &refreshed));
    let mut previous = original.clone();
    previous.websocket_endpoint = Some(super::super::contract::EndpointOverride {
        base_url: previous.base_url.clone(),
        headers: previous.headers.clone(),
    });
    let mut next = previous.clone();
    next.websocket_endpoint.as_mut().unwrap().headers.insert(
        "authorization",
        HeaderValue::from_static("Bearer override-fresh"),
    );
    assert!(websocket_transport_changed(&previous, &next));
}

#[test]
fn websocket_agent_assertion_timestamp_can_change_only_with_a_matching_host_digest() {
    let mut previous = route();
    previous.headers.insert(
        "authorization",
        HeaderValue::from_static("AgentAssertion signed-before"),
    );
    let mut next = previous.clone();
    next.headers.insert(
        "authorization",
        HeaderValue::from_static("AgentAssertion signed-after"),
    );
    assert!(!websocket_transport_changed(&previous, &next));
    next.transport_identity = Some("changed-task-or-key".into());
    assert!(websocket_transport_changed(&previous, &next));
    previous.transport_identity = None;
    next.transport_identity = None;
    assert!(websocket_transport_changed(&previous, &next));
}

#[test]
fn websocket_handshake_identity_changes_reconnect_without_treating_each_turn_as_a_new_installation()
{
    let mut previous = HeaderMap::new();
    previous.insert(
        "x-codex-installation-id",
        HeaderValue::from_static("old-installation"),
    );
    previous.insert("session-id", HeaderValue::from_static("session"));
    previous.insert("thread-id", HeaderValue::from_static("thread"));
    let mut next = previous.clone();
    next.insert(
        "x-codex-turn-state",
        HeaderValue::from_static("another-turn"),
    );
    next.insert("x-client-request-id", HeaderValue::from_static("request"));
    assert!(!websocket_connection_headers_changed(&previous, &next));
    for name in [
        "x-codex-installation-id",
        "session-id",
        "thread-id",
        "user-agent",
        "openai-beta",
    ] {
        let mut changed = next.clone();
        changed.insert(name, HeaderValue::from_static("changed"));
        assert!(
            websocket_connection_headers_changed(&previous, &changed),
            "{name}"
        );
    }
}

struct RotatingHost {
    base: String,
    token_revision: Arc<AtomicUsize>,
    with_digest: bool,
}
impl Host for RotatingHost {
    fn authorize<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async {
            Ok(Caller {
                id: "caller".into(),
                label: "fixture".into(),
            })
        })
    }
    fn route<'a>(
        &'a self,
        _: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            let revision = self.token_revision.load(Ordering::SeqCst);
            let mut value = route();
            value.base_url = self.base.clone();
            value.model = request.model;
            value.headers.insert(
                "authorization",
                HeaderValue::from_str(&format!("Bearer token-{revision}")).unwrap(),
            );
            value.headers.insert(
                "x-codex-installation-id",
                HeaderValue::from_str(&profile().installation_id).unwrap(),
            );
            value.transport_identity = self
                .with_digest
                .then(|| format!("fixture-config-token-{revision}"));
            if !self.with_digest {
                value.installation_profile = None;
            }
            Ok(value)
        })
    }
    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { Ok(json!({"data":[]})) })
    }
    fn record(&self, _: native::contract::RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

#[tokio::test]
async fn websocket_next_turn_refreshes_authentication_without_changing_installation_or_session() {
    for with_digest in [false, true] {
        let expected_revision = Arc::new(AtomicUsize::new(0));
        let handshakes = Arc::new(Mutex::new(Vec::<HeaderMap>::new()));
        let requests = Arc::new(Mutex::new(Vec::<(HeaderMap, Value)>::new()));
        let captured_headers = handshakes.clone();
        let captured_requests = requests.clone();
        let expected = expected_revision.clone();
        let app=Router::new().route("/v1/responses",get(move|headers:HeaderMap,upgrade:WebSocketUpgrade|{
            captured_headers.lock().unwrap().push(headers.clone());
            let requests=captured_requests.clone();let expected=expected.clone();
            async move{upgrade.on_upgrade(move|mut socket|async move{
                while let Some(Ok(frame))=socket.recv().await {
                    match frame {
                        Message::Text(text)=>{
                            let body:Value=serde_json::from_str(text.as_str()).unwrap();
                            let count={let mut requests=requests.lock().unwrap();requests.push((headers.clone(),body));requests.len()};
                            let wanted=format!("Bearer token-{}",expected.load(Ordering::SeqCst));
                            let event=if headers.get("authorization").and_then(|v|v.to_str().ok())==Some(wanted.as_str()) {
                                json!({"type":"response.completed","response":{"id":format!("response-{count}"),"status":"completed","output":[]}})
                            }else{json!({"type":"error","status":401,"error":{"code":"stale_handshake","message":"Old handshake retained after credential refresh"}})};
                            if socket.send(Message::Text(event.to_string().into())).await.is_err(){break;}
                        },
                        Message::Ping(value)=>{if socket.send(Message::Pong(value)).await.is_err(){break;}},
                        Message::Close(_)=>break,
                        _=>{}
                    }
                }
            })}
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let host = Arc::new(RotatingHost {
            base,
            token_revision: expected_revision.clone(),
            with_digest,
        });
        let directory = tempfile::tempdir().unwrap();
        let mut handle = native::start(
            "127.0.0.1:0".parse().unwrap(),
            host,
            directory.path().join("identity.sqlite"),
            native::Options::default(),
        )
        .await
        .unwrap();
        let mut request = format!("ws://{}/v1/responses", handle.addr)
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("authorization", HeaderValue::from_static("Bearer client"));
        request
            .headers_mut()
            .insert("cookie", HeaderValue::from_static("client-session=private"));
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        for turn in 0..3 {
            if turn == 2 {
                expected_revision.store(1, Ordering::SeqCst);
            }
            socket.send(ClientMessage::Text(json!({"type":"response.create","model":"m","client_metadata":{"session_id":"session","thread_id":"thread"},"input":[]}).to_string().into())).await.unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(3), async {
                loop {
                    match socket.next().await.unwrap().unwrap() {
                        ClientMessage::Text(text) => {
                            let event: Value = serde_json::from_str(&text).unwrap();
                            assert_eq!(event["type"], "response.completed");
                            break;
                        }
                        ClientMessage::Ping(value) => {
                            socket.send(ClientMessage::Pong(value)).await.unwrap()
                        }
                        _ => {}
                    }
                }
            })
            .await
            .unwrap();
            assert_eq!(
                handshakes.lock().unwrap().len(),
                if turn == 2 { 2 } else { 1 }
            );
        }
        {
            let handshakes = handshakes.lock().unwrap();
            assert_eq!(handshakes[0]["authorization"], "Bearer token-0");
            assert_eq!(handshakes[1]["authorization"], "Bearer token-1");
            for name in ["x-codex-installation-id", "session-id", "thread-id"] {
                assert_eq!(handshakes[0].get(name), handshakes[1].get(name), "{name}");
            }
            assert!(handshakes
                .iter()
                .all(|headers| !headers.contains_key("cookie")));
        }
        socket.close(None).await.unwrap();
        handle
            .shutdown(std::time::Duration::from_secs(2))
            .await
            .unwrap();
        task.abort();
    }
}
