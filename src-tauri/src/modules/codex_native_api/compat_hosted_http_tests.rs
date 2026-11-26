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
    oauth: bool,
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
                chatgpt: self.oauth,
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
async fn claude_tool_search_http_discovery_and_continuation_use_the_selected_backend() {
    use axum::response::IntoResponse;

    for oauth in [true, false] {
        let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
        let capture = seen.clone();
        let app = Router::new().route("/v1/responses", post(move |Json(body): Json<Value>| {
            let capture = capture.clone();
            async move {
                let turn = {
                    let mut requests = capture.lock().unwrap();
                    requests.push(body.clone());
                    requests.len()
                };
                let item = match (turn, oauth) {
                    (1, true) => json!({"type":"tool_search_call","id":"tsc_fixture",
                        "call_id":"call_search","execution":"client","status":"completed",
                        "arguments":{"query":"fetch"}}),
                    (1, false) => json!({"type":"function_call","id":"fc_fixture",
                        "call_id":"call_search","name":"ToolSearch","status":"completed",
                        "arguments":"{\"query\":\"fetch\"}"}),
                    _ => json!({"type":"message","id":"msg_fixture","role":"assistant",
                        "status":"completed","content":[{"type":"output_text","text":"Found."}]}),
                };
                let response = json!({"id":format!("resp_fixture_{turn}"),"object":"response",
                    "status":"completed","model":"model","output":[item]});
                if body["stream"] == true {
                    let done = json!({"type":"response.completed","response":response});
                    ([("content-type", "text/event-stream")], format!("data: {done}\n\n")).into_response()
                } else {
                    Json(response).into_response()
                }
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = Arc::new(FixtureHost {
            url: format!("http://{}/v1", listener.local_addr().unwrap()),
            oauth,
            selections: Mutex::new(Vec::new()),
        });
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let dir = tempfile::tempdir().unwrap();
        let mut service = native::start("127.0.0.1:0".parse().unwrap(), host,
            dir.path().join("identity.sqlite"), native::Options::default()).await.unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let endpoint = format!("http://{}/v1/messages", service.addr);
        let mut request = json!({"model":"model","max_tokens":32,
            "messages":[{"role":"user","content":"Find a tool."}],"tools":[
                {"name":"ToolSearch","input_schema":{"type":"object"}},
                {"name":"mcp__fixture__fetch","defer_loading":true,"input_schema":{"type":"object"}}
            ]});
        async fn call(client: &reqwest::Client, endpoint: &str, request: &Value) -> Value {
            let response = client.post(endpoint).bearer_auth("key").header("thread-id", "fixture")
                .json(request).send().await.unwrap();
            let status = response.status();
            let body: Value = response.json().await.unwrap();
            assert_eq!(status, 200, "{body}");
            body
        }
        let reply = call(&client, &endpoint, &request).await;
        assert_eq!(reply["content"][0]["name"], "ToolSearch");
        assert_eq!(reply["stop_reason"], "tool_use");
        request["messages"].as_array_mut().unwrap().extend([
            json!({"role":"assistant","content":reply["content"]}),
            json!({"role":"user","content":[{"type":"tool_result",
                "tool_use_id":reply["content"][0]["id"],
                "content":[{"type":"tool_reference","tool_name":"mcp__fixture__fetch"}]}]}),
        ]);
        let reply = call(&client, &endpoint, &request).await;
        assert_eq!(reply["content"][0]["text"], "Found.");
        {
            let requests = seen.lock().unwrap();
            assert_eq!(requests.len(), 2);
            assert_eq!(requests[0]["tools"][0]["type"], if oauth { "tool_search" } else { "function" });
            let items = requests[1]["input"].as_array().unwrap();
            let result = items.iter().find(|item| item["type"] ==
                if oauth { "tool_search_output" } else { "function_call_output" }).unwrap();
            if oauth {
                assert_eq!(requests[0]["tools"], requests[1]["tools"]);
                assert_eq!(result["tools"][0]["name"], "mcp__fixture__fetch");
            } else {
                assert_eq!(requests[1]["tools"][1]["name"], "mcp__fixture__fetch");
            }
        }
        service.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
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
                response["output"].as_array_mut().unwrap().insert(1, json!({
                    "id":"reasoning-hidden","type":"reasoning","summary":[],"encrypted_content":"hidden-cipher"
                }));
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
        oauth: false,
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
    assert_eq!(reply["content"][1], json!({"type":"redacted_thinking","data":"hidden-cipher"}));
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
        seen.lock().unwrap()[1].1["input"][2]["encrypted_content"],
        "hidden-cipher"
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
        for (headers, _) in requests.iter() {
            assert!(!headers.contains_key("session-id"));
            assert_eq!(headers["thread-id"], "original-thread");
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
    for (kind, value) in [
        ("encrypted", "cipher-1"),
        ("encrypted", "hidden-cipher"),
        ("response", "response-claude-1"),
    ] {
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
