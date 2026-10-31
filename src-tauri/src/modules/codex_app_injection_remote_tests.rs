use super::*;
use std::sync::{Arc, Mutex};

fn profile(base: &str, version: u32) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        format!(
            r#"model_provider = "remote"
[model_providers.remote]
base_url = "{base}"
experimental_bearer_token = "remote-fixture"
http_headers = {{ x-cockpit-instance-id = "instance-b", x-codex-installation-id = "install-b" }}
"#
        ),
    )
    .unwrap();
    let descriptor=serde_json::from_value(json!({"product":"cockpit-tools","protocolVersion":1,
        "serviceId":"adapter-fixture","serviceName":"Cockpit Tools","keyLabel":"B","status":"active",
        "models":["public-model"],"imageModels":[],"keyViewsVersion":version,
        "tokenLimit":null,"tokenUsed":0,"budget":{},"updatedAt":1,
        "capabilities":{"responses":true,"images":false,"imageEdits":false,"hostedImages":false,"usage":true,"websockets":true}})).unwrap();
    remote::remember_test_connection(base, "remote-fixture", descriptor);
    dir
}
fn input() -> DialogQuery {
    DialogQuery {
        request_token: "remote-test".into(),
        thread_id: Some("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into()),
        scope: "chat".into(),
        period: "today".into(),
        cursor: None,
        view: "overview".into(),
        binding: None,
    }
}

#[tokio::test]
async fn remote_dialog_uses_key_apis_without_local_database_and_projects_public_rows() {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let app=axum::Router::new().route("/v1/cockpit/{resource}",axum::routing::get({
        let captured=captured.clone();
        move |axum::extract::Path(resource):axum::extract::Path<String>,headers:axum::http::HeaderMap,
            axum::extract::Query(query):axum::extract::Query<std::collections::HashMap<String,String>>| {
            let captured=captured.clone();async move {
                assert_eq!(headers["authorization"],"Bearer remote-fixture");
                captured.lock().unwrap().push((resource.clone(),query));
                axum::Json(match resource.as_str() {
                    "stats"=>json!({"ready":true,"summary":{"requestCount":7,"successCount":6,"totalTokens":1200,
                        "estimatedCostUsd":0.25,"totalLatencyMs":1400},"startAt":0,"endAt":86400000}),
                    "session"=>json!({"ready":true,"session":{"model":"public-model","firstResponseMs":200}}),
                    "requests"=>json!({"ready":true,"items":[{"id":5,"requestId":"public-request","timestamp":42,
                        "model":"public-model","kind":"image_generation","outcome":"success","inputTokens":100,"outputTokens":200,
                        "outputGenerationMs":1000,"accountId":"secret-account","email":"secret@invalid.test","upstreamModel":"private-upstream"}],
                        "nextCursor":{"beforeTime":42,"beforeId":5}}),
                    _=>panic!("unexpected resource")
                })
            }
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let profile = profile(&base, 1);
    let path = profile.path().to_owned();
    let (overview, logs) = tokio::task::spawn_blocking(move || {
        let endpoint = endpoint(&path).unwrap().unwrap();
        let overview = dialog(&path, &endpoint, &input()).unwrap();
        let mut query = input();
        query.view = "logs".into();
        let logs = dialog(&path, &endpoint, &query).unwrap();
        (overview, logs)
    })
    .await
    .unwrap();
    assert_eq!(overview["available"], true);
    assert_eq!(overview["summary"]["requests"], 7);
    assert_eq!(overview["summary"]["averageLatencyMs"], 200.0);
    assert_eq!(overview["session"]["firstResponseMs"], 200);
    assert_eq!(logs["requests"]["rows"][0]["requestId"], "public-request");
    assert_eq!(logs["requests"]["rows"][0]["outputGenerationMs"], 1000);
    assert_eq!(logs["requests"]["rows"][0]["requestKind"],"image_generation");
    for secret in ["secret-account", "secret@invalid.test", "private-upstream"] {
        assert!(!logs.to_string().contains(secret));
    }
    let cursor = logs["requests"]["nextCursor"].as_str().unwrap();
    assert_eq!(serde_json::from_str::<Value>(cursor).unwrap()["id"], 5);
    let sent = captured.lock().unwrap();
    let stats = &sent
        .iter()
        .find(|(resource, _)| resource == "stats")
        .unwrap()
        .1;
    assert!(!stats.contains_key("threadId"));
    assert!(!stats.contains_key("instanceId"));
    for (resource, query) in sent.iter().filter(|(resource, _)| resource != "stats") {
        assert_eq!(query["instanceId"], "instance-b", "{resource}");
        assert_eq!(query["installationId"], "install-b", "{resource}");
    }
    drop(sent);
    task.abort();
}

#[test]
fn remote_dialog_legacy_and_binding_changes_do_not_query_local_logs() {
    let base = format!("http://192.0.2.10/{}/v1", uuid::Uuid::new_v4());
    let dir = profile(&base, 0);
    let endpoint = endpoint(dir.path()).unwrap().unwrap();
    assert_eq!(
        dialog(dir.path(), &endpoint, &input()).unwrap()["unavailableReason"],
        "remote_unsupported"
    );
    let mut query = input();
    query.binding = Some("0".repeat(64));
    assert_eq!(
        dialog(dir.path(), &endpoint, &query).unwrap()["unavailableReason"],
        "binding_changed"
    );
}
