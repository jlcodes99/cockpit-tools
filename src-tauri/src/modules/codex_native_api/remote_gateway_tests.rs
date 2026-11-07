//! Synthetic HTTP hop: no application UI or real account data is used.
use super::*;

#[tokio::test]
async fn remote_gateway_preserves_client_scope_and_thread_but_strips_management_headers() {
    let captured = Arc::new(Mutex::new(None));
    let (addr,task)=mock(Router::new().route("/v1/responses",post({
        let captured=captured.clone();
        move |headers:HeaderMap,axum::Json(body):axum::Json<Value>| {
            let captured=captured.clone();
            async move {
                *captured.lock().unwrap()=Some((headers,body));
                axum::Json(json!({"id":"resp-remote","object":"response","status":"completed","output":[]}))
            }
        }
    }))).await;
    let (mut service, host, path) = native(
        addr,
        false,
        false,
        Options {
            session_identity_mapping_enabled: false,
            ..Default::default()
        },
    )
    .await;
    let descriptor=serde_json::from_value(json!({"product":"cockpit-tools","protocolVersion":1,
        "serviceId":"scope-fixture","serviceName":"Cockpit Tools","keyLabel":"B","status":"active",
        "models":["test-model"],"imageModels":[],"tokenLimit":null,"tokenUsed":0,"budget":{},"updatedAt":1,
        "capabilities":{"responses":true,"images":false,"imageEdits":false,"hostedImages":false,"usage":true,"websockets":true}})).unwrap();
    crate::modules::codex_remote_service::remember_test_connection(
        &host.base_url,
        "upstream-test-key",
        descriptor,
    );
    let thread = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
    let response = client()
        .post(format!("http://{}/v1/responses", service.addr))
        .bearer_auth("local-test-key")
        .header("x-cockpit-instance-id", "instance-b")
        .header("x-codex-installation-id", "installation-b")
        .header("x-cockpit-account-id", "private-routing")
        .header("x-cockpit-management-key", "private-management")
        .header("thread_id", thread)
        .header("session_id", thread)
        .json(&json!({"model":"test-model","input":"hello"}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "{}",
        response.text().await.unwrap()
    );
    let (headers, _) = captured.lock().unwrap().clone().unwrap();
    assert_eq!(headers["authorization"], "Bearer upstream-test-key");
    assert_eq!(headers["x-cockpit-instance-id"], "instance-b");
    assert_eq!(headers["x-codex-installation-id"], "installation-b");
    assert_eq!(headers["thread_id"], thread);
    assert_eq!(headers["session_id"], thread);
    assert!(!headers.contains_key("x-cockpit-account-id"));
    assert!(!headers.contains_key("x-cockpit-management-key"));
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
    std::fs::remove_dir_all(path).unwrap();
}
