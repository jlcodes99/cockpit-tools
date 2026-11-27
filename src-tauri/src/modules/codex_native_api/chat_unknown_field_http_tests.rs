//! Strict Chat upstreams (Google's OpenAI-compatible endpoint) reject unknown
//! top-level names; the gateway must retry once without the optional hints.
use super::*;

#[tokio::test]
async fn strict_chat_upstream_rejecting_optional_fields_is_retried_without_them() {
    let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
    let app = Router::new().route(
        "/v1/chat/completions",
        post({
            let seen = seen.clone();
            move |Json(body): Json<Value>| {
                seen.lock().unwrap().push(body.clone());
                async move {
                    let rejected = ["client_metadata", "prompt_cache_key", "verbosity"]
                        .into_iter()
                        .filter(|name| body.get(*name).is_some())
                        .map(|name| format!("Invalid JSON payload received. Unknown name \"{name}\": Cannot find field."))
                        .collect::<Vec<_>>();
                    if !rejected.is_empty() {
                        let message = rejected.join("\n");
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(json!([{"error":{"code":400,"message":message,"status":"INVALID_ARGUMENT"}}])),
                        );
                    }
                    (
                        StatusCode::OK,
                        Json(json!({"id":"chat-1","object":"chat.completion","model":"m",
                            "choices":[{"index":0,"message":{"role":"assistant","content":"hola"},"finish_reason":"stop"}],
                            "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}})),
                    )
                }
            }
        }),
    );
    let (mut handle, _host, task, _dir) =
        fixture(app, WireApi::Chat, false, native::Options::default()).await;
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("key")
        .json(&json!({"model":"m","input":"hola",
            "client_metadata":{"session_id":"s","thread_id":"t"},
            "prompt_cache_key":"cache","text":{"verbosity":"low"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.json::<Value>().await.unwrap();
    assert_eq!(body["output"][0]["content"][0]["text"], "hola", "{body}");
    let requests = seen.lock().unwrap();
    // The first attempt already omits Codex-only metadata; the optional Chat
    // hints are only dropped after the upstream names them in a 400.
    assert_eq!(requests.len(), 2, "{requests:?}");
    assert!(requests[0].get("client_metadata").is_none());
    assert!(requests[0].get("prompt_cache_key").is_some());
    assert_eq!(requests[0]["verbosity"], "low");
    assert!(requests[1].get("prompt_cache_key").is_none());
    assert!(requests[1].get("verbosity").is_none());
    assert_eq!(requests[1]["messages"], requests[0]["messages"]);
    drop(requests);
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}
