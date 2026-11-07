//! Compare final native HTTP requests against localhost captures from the Go executor.
use super::super::{contract::*, start, Options};
use axum::{http::HeaderMap, routing::post, Json, Router};
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

struct RuntimeWireHost {
    base: String,
}
impl Host for RuntimeWireHost {
    fn authorize<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async {
            Ok(Caller {
                id: "runtime-parity".into(),
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
            Ok(Route {
                continuity_account_id: None,
                installation_profile: None,
                official_authorization_revision: None,
                official_registration_id: None,
                official_fallback_registration_id: None,
                official_fallback_authorization_revision: None,
                authorization_scope_fingerprint: None,
                official_fallback_failure: None,
                transport_identity: None,
                account_id: "runtime-a".into(),
                owner_id: "runtime-owner".into(),
                base_url: self.base.clone(),
                model: request.model,
                headers: HeaderMap::new(),
                client: reqwest::Client::builder().no_proxy().build().unwrap(),
                proxy_url: None,
                proxy_route: None,
                chatgpt: true,
                chatgpt_plan: false,
                wire_api: WireApi::Responses,
                compact_endpoint: None,
                websocket_endpoint: None,
                default_service_tier: None,
                permit: None,
            })
        })
    }
    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { Ok(json!({"data":[]})) })
    }
    fn record(&self, _: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

#[tokio::test]
async fn parity22_runtime_http_final_wire_matches_go_executor() {
    let reference: Value =
        serde_json::from_str(include_str!("fixtures/parity22_runtime_go.json")).unwrap();
    let captures = Arc::new(Mutex::new(Vec::<Value>::new()));
    let seen = captures.clone();
    let upstream = Router::new().route("/responses",post(move |headers:HeaderMap,Json(body):Json<Value>| {
        let seen=seen.clone();
        async move {
            seen.lock().unwrap().push(json!({"stream_options":body.get("stream_options"),"lite_header":headers.get(super::LITE_HEADER).and_then(|v|v.to_str().ok()).unwrap_or(""),"parallel_tool_calls":body.get("parallel_tool_calls"),"tools":body.get("tools")}));
            ([("content-type","text/event-stream")],"data: {\"type\":\"response.completed\",\"response\":{\"id\":\"runtime-response\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":0,\"output_tokens\":0,\"total_tokens\":0}}}\n\n")
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let root = tempfile::tempdir().unwrap();
    let mut service = start(
        "127.0.0.1:0".parse().unwrap(),
        Arc::new(RuntimeWireHost {
            base: format!("http://{address}"),
        }),
        root.path().join("identity.sqlite"),
        Options::default(),
    )
    .await
    .unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    for case in reference["wire"].as_array().unwrap() {
        let response = client
            .post(format!("http://{}/v1/responses", service.addr))
            .bearer_auth("fixture-only")
            .json(&case["input"])
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "{}", case["name"]);
        response.bytes().await.unwrap();
        assert_eq!(
            captures.lock().unwrap().last().unwrap(),
            &case["expected"],
            "{}",
            case["name"]
        );
    }
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}
