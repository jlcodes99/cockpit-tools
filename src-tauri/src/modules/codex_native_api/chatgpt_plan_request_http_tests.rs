//! Localhost-only request compatibility checks through the real SIWC pipeline.
use super::*;

#[tokio::test]
async fn plan_configuration_updates_are_collapsed_on_every_http_attempt() {
    let (host, captures, task, _root, mut service) = fixture(COMPLETED, true).await;
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!("http://{}/v1/responses", service.addr))
        .bearer_auth("local-key")
        .header("user-agent", "codex-tui/synthetic")
        .header("originator", "codex-tui")
        .header("x-codex-routing-hint", "model=incorrect")
        .header("x-codex-beta-features", "synthetic")
        .header("session-id", "synthetic-session")
        .header("thread-id", "synthetic-thread")
        .json(&json!({
            "model":"plan-model","stream":false,
            "reasoning":{"effort":"medium","summary":"auto"},
            "tools":[],"tool_choice":"none","parallel_tool_calls":false,
            "input":[
                {"type":"configuration_update","reasoning":{"effort":"low"}},
                {"type":"message","role":"user","content":"synthetic question"},
                {"type":"configuration_update","reasoning":{"effort":" High "}},
            ],
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let response: Value = response.json().await.unwrap();
    assert_eq!(response["status"], "completed");
    assert_eq!(host.refreshes.load(Ordering::SeqCst), 1);
    {
        let captures = captures.lock().unwrap();
        assert_eq!(captures.len(), 2);
        for (headers, body) in captures.iter() {
            assert_eq!(body["reasoning"], json!({"effort":"high","summary":"auto"}));
            assert_eq!(body["input"].as_array().unwrap().len(), 1);
            assert_eq!(body["input"][0]["role"], "user");
            assert_eq!(body["tools"], json!([]));
            assert_eq!(body["tool_choice"], "none");
            // CPA omits parallel-tool settings when there are no top-level
            // tools; the explicit tool_choice restriction remains unchanged.
            assert!(body.get("parallel_tool_calls").is_none());
            assert_eq!(
                headers["user-agent"],
                concat!("CockpitTools/", env!("CARGO_PKG_VERSION"))
            );
            assert!(!headers.contains_key("originator"));
            assert!(!headers.contains_key("x-codex-routing-hint"));
            assert!(!headers.contains_key("x-codex-beta-features"));
            assert_ne!(headers["session-id"], "synthetic-session");
            assert_ne!(headers["thread-id"], "synthetic-thread");
            assert_eq!(headers["thread-id"], headers["x-client-request-id"]);
            assert_ne!(headers["session-id"], headers["thread-id"]);
        }
    }
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn plan_configuration_updates_preserve_explicit_model_suffix_overrides() {
    let (_host, captures, task, _root, mut service) = fixture(COMPLETED, false).await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    for (model, expected) in [
        ("plan-model(low)", "low"),
        ("plan-model(none)", "none"),
        ("plan-model(16384)", "high"),
    ] {
        let response = client
            .post(format!("http://{}/v1/responses", service.addr))
            .bearer_auth("local-key")
            .json(&json!({
                "model":model,"reasoning":{"effort":"medium","summary":"auto"},
                "input":[
                    {"type":"configuration_update","reasoning":{"effort":"xhigh"}},
                    {"role":"user","content":"synthetic question"},
                ],
            }))
            .send()
            .await
            .unwrap();
        let status = response.status();
        let response: Value = response.json().await.unwrap();
        assert_eq!(status, 200, "{response}");
        let captures = captures.lock().unwrap();
        let body = &captures.last().unwrap().1;
        assert_eq!(body["reasoning"]["effort"], expected);
        assert_eq!(body["reasoning"]["summary"], "auto");
        assert!(body["input"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["type"] != "configuration_update"));
    }
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn plan_configuration_updates_do_not_drop_tool_restrictions_or_unknown_settings() {
    let (host, captures, task, _root, mut service) = fixture(COMPLETED, false).await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    for update in [
        json!({"type":"configuration_update","tools":[]}),
        json!({"type":"configuration_update","tool_choice":"none"}),
        json!({"type":"configuration_update","tools":[{"type":"code_interpreter"}]}),
        json!({"type":"configuration_update","future_setting":true}),
    ] {
        let response = client
            .post(format!("http://{}/v1/responses", service.addr))
            .bearer_auth("local-key")
            .json(&json!({"model":"plan-model","input":[update,{"role":"user","content":"synthetic question"}]}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 400);
        let response: Value = response.json().await.unwrap();
        assert_eq!(
            response["error"]["code"],
            "subscription_sharing_unsupported_capability"
        );
        assert_eq!(response["error"]["param"], "input[0]");
    }
    assert!(captures.lock().unwrap().is_empty());
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    assert!(host.reports.lock().unwrap().is_empty(),
        "unsupported local configuration never became an upstream request");
    task.abort();
}
