//! Transport boundary regressions; these tests cannot start an upstream CLI.
use super::*;
use crate::modules::codex_native_api::contract::{
    Caller, Host, RequestReport, Route, RouteRequest, WireApi,
};
use futures_util::future::BoxFuture;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

struct BoundaryHost {
    delegate: bool,
    generic_routes: AtomicUsize,
    generic_policy: AtomicUsize,
}
impl Host for BoundaryHost {
    fn authorize<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async {
            Ok(Caller {
                id: "synthetic-delegate-key".into(),
                label: "fixture".into(),
            })
        })
    }
    fn workbuddy_delegate_route<'a>(
        &'a self,
        _: &'a Caller,
        _: &'a str,
    ) -> BoxFuture<'a, Result<Option<WorkbuddyDelegateSelection>, ServiceError>> {
        Box::pin(async move {
            Ok(self.delegate.then(|| WorkbuddyDelegateSelection {
                route: Route {
                    account_id: "synthetic-linked-reference".into(),
                    continuity_account_id: None,
                    owner_id: "synthetic-fixed-owner".into(),
                    installation_profile: None,
                    transport_identity: None,
                    official_authorization_revision: None,
                    official_registration_id: None,
                    official_fallback_registration_id: None,
                    official_fallback_authorization_revision: None,
                    authorization_scope_fingerprint: None,
                    official_fallback_failure: None,
                    base_url: BASE_URL.into(),
                    model: "fixture-model".into(),
                    headers: HeaderMap::new(),
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
                },
                source_generation: "synthetic-generation".into(),
                access_token: "not-a-credential".into(),
                domestic: true,
                effort: "low".into(),
            }))
        })
    }
    fn route<'a>(
        &'a self,
        _: &'a Caller,
        _: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        self.generic_routes.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Err(ServiceError::new(
                418,
                "generic_route_called",
                "Generic routing must not handle a native actor",
            ))
        })
    }
    fn execution_policy(
        &self,
    ) -> BoxFuture<'_, Result<super::super::contract::ExecutionPolicy, ServiceError>> {
        self.generic_policy.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(Default::default()) })
    }
    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { Ok(json!({"data":[]})) })
    }
    fn record(&self, _: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

async fn fixture(
    delegate: bool,
) -> (
    tempfile::TempDir,
    super::super::server::Handle,
    Arc<BoundaryHost>,
    reqwest::Client,
) {
    let directory = tempfile::tempdir().unwrap();
    let host = Arc::new(BoundaryHost {
        delegate,
        generic_routes: AtomicUsize::new(0),
        generic_policy: AtomicUsize::new(0),
    });
    let handle = super::super::server::start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        directory.path().join("identity.sqlite"),
        Default::default(),
    )
    .await
    .unwrap();
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    (directory, handle, host, client)
}

#[tokio::test]
async fn unsupported_delegate_requests_never_enter_generic_policy_or_routing() {
    let (_directory, mut handle, host, client) = fixture(true).await;
    for (path, extra) in [
        ("responses/compact", json!({})),
        ("responses", json!({"tools":[{"type":"web_search"}]})),
        ("responses", json!({"reasoning":{"effort":"high"}})),
        (
            "responses",
            json!({"text":{"format":{"type":"json_schema"}}}),
        ),
        (
            "responses",
            json!({"tool_choice":{"type":"function","name":"foo"}}),
        ),
    ] {
        let mut body = json!({"model":"workbuddy/fixture-model","input":"Synthetic fixture only","stream":true});
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let response = client
            .post(format!("http://{}/v1/{path}", handle.addr))
            .bearer_auth("synthetic-key")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body: Value = response.json().await.unwrap();
        assert_ne!(body["error"]["code"], "generic_route_called");
    }
    assert_eq!(host.generic_routes.load(Ordering::SeqCst), 0);
    assert_eq!(host.generic_policy.load(Ordering::SeqCst), 0);
    handle.shutdown(Duration::from_secs(5)).await.unwrap();
}

#[tokio::test]
async fn revoked_native_grant_cannot_replay_response_or_full_history_elsewhere() {
    let (_directory, mut handle, host, client) = fixture(false).await;
    for body in [
        json!({"model":"other-model","previous_response_id":"resp_wb_stale","input":"continue"}),
        json!({"model":"other-model","input":[{"type":"function_call_output","call_id":"call_wb_stale","output":"ok"}]}),
        json!({"model":"other-model","input":[{"type":"message","role":"assistant","id":"msg_wb_stale","content":[{"type":"output_text","text":"old answer"}]},{"role":"user","content":"continue"}]}),
    ] {
        let response = client
            .post(format!("http://{}/v1/responses", handle.addr))
            .bearer_auth("synthetic-key")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body: Value = response.json().await.unwrap();
        assert_eq!(
            body["error"]["code"],
            "workbuddy_delegate_grant_unavailable"
        );
    }
    assert_eq!(host.generic_routes.load(Ordering::SeqCst), 0);
    assert_eq!(host.generic_policy.load(Ordering::SeqCst), 0);
    handle.shutdown(Duration::from_secs(5)).await.unwrap();
}

#[tokio::test]
async fn lost_ready_actor_cannot_be_recreated_from_assistant_history() {
    let (_directory, mut handle, host, client) = fixture(true).await;
    for assistant in [
        json!({"type":"message","id":"msg_wb_stale","role":"assistant","content":"old answer"}),
        json!({"role":"assistant","content":"old answer with stripped id"}),
        json!({"type":"reasoning","summary":[]}),
    ] {
        let body = json!({"model":"workbuddy/fixture-model","input":[assistant,{"role":"user","content":"continue"}]});
        let response = client
            .post(format!("http://{}/v1/responses", handle.addr))
            .bearer_auth("synthetic-key")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body: Value = response.json().await.unwrap();
        assert!(
            matches!(
                body["error"]["code"].as_str(),
                Some("workbuddy_delegate_session_lost" | "workbuddy_conversation_expired")
            ),
            "{body}"
        );
    }
    assert_eq!(host.generic_routes.load(Ordering::SeqCst), 0);
    assert_eq!(host.generic_policy.load(Ordering::SeqCst), 0);
    handle.shutdown(Duration::from_secs(5)).await.unwrap();
}
