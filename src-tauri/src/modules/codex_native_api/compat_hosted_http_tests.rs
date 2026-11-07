use super::*;
use crate::modules::codex_native_api::{self as native, contract::*};
use axum::{http::HeaderMap, routing::post, Json, Router};
use futures_util::future::BoxFuture;
use sha2::{Digest, Sha256};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

struct FixtureHost {
    url: String,
    selections: Mutex<Vec<RouteRequest>>,
}
impl Host for FixtureHost {
    fn authorize<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async move {
            Ok(Caller {
                id: if key == "other" { "other" } else { "caller" }.into(),
                label: "test".into(),
            })
        })
    }
    fn route<'a>(
        &'a self,
        _: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            let account = request.target_account_id.as_deref().unwrap_or("account-a");
            let route = Route {
                continuity_account_id: None,
                installation_profile: None,
                official_authorization_revision: None,
                official_registration_id: None,
                official_fallback_registration_id: None,
                official_fallback_authorization_revision: None,
                authorization_scope_fingerprint: None,
                official_fallback_failure: None,
                transport_identity: None,
                account_id: account.into(),
                owner_id: format!("owner-{account}"),
                base_url: self.url.clone(),
                model: request.model.clone(),
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
            };
            self.selections.lock().unwrap().push(request);
            Ok(route)
        })
    }
    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { Ok(json!({"data":[{"id":"model"}]})) })
    }
    fn record(&self, _: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

#[tokio::test]
async fn claude_signature_http_continuation_survives_restart_and_replays_for_only_original_caller()
{
    let seen = Arc::new(Mutex::new(Vec::<(HeaderMap, Value)>::new()));
    let capture = seen.clone();
    let app = Router::new().route(
        "/v1/responses",
        post(move |headers: HeaderMap, Json(body): Json<Value>| {
            let capture = capture.clone();
            async move {
                let index = {
                    let mut requests = capture.lock().unwrap();
                    requests.push((headers, body));
                    requests.len()
                };
                let mut response = output();
                // A new upstream completion has its own resource IDs. Echoing
                // an input signature must not transfer its original provenance.
                response["id"] = json!(format!("response-claude-{index}"));
                for (item_index, item) in response["output"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .enumerate()
                {
                    if item.get("id").is_some() {
                        item["id"] = json!(format!("item-{index}-{item_index}"));
                    }
                }
                Json(response)
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = Arc::new(FixtureHost {
        url: format!("http://{}/v1", listener.local_addr().unwrap()),
        selections: Mutex::new(Vec::new()),
    });
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identity.sqlite");
    let mut service = native::start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        path.clone(),
        native::Options::default(),
    )
    .await
    .unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let body = json!({"model":"model","max_tokens":32,"messages":[{"role":"user","content":"hello"}],"tools":[{"type":"web_search_20250305","name":"web_search"}]});
    let response = client
        .post(format!("http://{}/v1/messages", service.addr))
        .bearer_auth("key")
        .header("thread-id", "original-thread")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let reply: Value = response.json().await.unwrap();
    assert_eq!(reply["content"][0]["signature"], "cipher-1");
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    drop(service);
    let mut service = native::start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        path.clone(),
        native::Options::default(),
    )
    .await
    .unwrap();
    let mut continuation = body.clone();
    continuation["messages"]
        .as_array_mut()
        .unwrap()
        .push(json!({"role":"assistant","content":reply["content"]}));
    let response = client
        .post(format!("http://{}/v1/messages", service.addr))
        .bearer_auth("key")
        .header("thread-id", "original-thread")
        .json(&continuation)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    response.bytes().await.unwrap();
    assert_eq!(
        seen.lock().unwrap()[1].1["input"][1]["encrypted_content"],
        "cipher-1"
    );
    assert_eq!(
        host.selections.lock().unwrap()[1]
            .preferred_account_id
            .as_deref(),
        Some("account-a")
    );
    let response = client
        .post(format!("http://{}/v1/messages", service.addr))
        .bearer_auth("key")
        .header("thread-id", "original-thread")
        .header("x-cockpit-target-account-id", "account-b")
        .json(&continuation)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<Value>().await.unwrap()["content"][0]["signature"],
        "cipher-1"
    );
    {
        let requests = seen.lock().unwrap();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[2].1["input"], requests[1].1["input"]);
        for header in ["session-id", "thread-id"] {
            assert_ne!(requests[2].0[header], requests[1].0[header]);
        }
    }
    assert_eq!(
        host.selections
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .target_account_id
            .as_deref(),
        Some("account-b")
    );
    let connection = rusqlite::Connection::open(&path).unwrap();
    for (kind, value) in [("encrypted", "cipher-1"), ("response", "response-claude-1")] {
        let digest = format!("{:x}", Sha256::digest(value.as_bytes()));
        let owner: (String, String) = connection
            .query_row(
                "SELECT account,owner FROM native_opaque_v1 WHERE caller='caller' AND kind=?1 AND digest=?2",
                rusqlite::params![kind, digest],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(owner, ("account-a".into(), "owner-account-a".into()));
    }
    let response = client
        .post(format!("http://{}/v1/messages", service.addr))
        .bearer_auth("other")
        .header("thread-id", "original-thread")
        .json(&continuation)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 409);
    assert_eq!(
        response.json::<Value>().await.unwrap()["error"]["type"],
        "invalid_request_error"
    );
    assert_eq!(seen.lock().unwrap().len(), 3);
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}
