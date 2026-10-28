use super::*;

fn account(id: &str) -> CodexAccount {
    let mut account = CodexAccount::new(
        id.into(),
        "fixture@example.invalid".into(),
        crate::models::codex::CodexTokens {
            access_token: format!("fixture-{id}"),
            id_token: "fixture-id".into(),
            refresh_token: None,
        },
    );
    account.user_id = Some(format!("principal-{id}"));
    account.account_id = Some(format!("workspace-{id}"));
    account.egress_proxy_disabled = true;
    account
}

#[test]
fn independent_direct_runs_have_fresh_sessions_and_preserve_history_verbatim() {
    let input = json!({"model":"fixture","input":[
        {"type":"reasoning","encrypted_content":"opaque-reasoning"},
        {"type":"function_call","name":"spawn_agent","arguments":"{\"message\":\"opaque-child\"}"},
        {"type":"function_call_output","call_id":"fixture-call","output":"verbatim tool result"}
    ],"tools":[{"type":"function","name":"spawn_agent"}]});
    let (body, headers) = direct_codex_request(input.clone()).unwrap();
    let (second, second_headers) = direct_codex_request(input.clone()).unwrap();
    assert_ne!(headers["session-id"], second_headers["session-id"]);
    assert_ne!(body, second);
    assert_eq!(headers["session-id"], headers["thread-id"]);
    assert_ne!(headers["session-id"], headers["x-client-request-id"]);
    let raw: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        raw["client_metadata"]["x-codex-turn-metadata"],
        headers["x-codex-turn-metadata"]
    );
    let dir = tempfile::tempdir().unwrap();
    let profile = crate::modules::codex_api_identity::resolve_at(
        &dir.path().join("identity.sqlite"),
        &account("a"),
    )
    .unwrap();
    let (bound, mapped, _) =
        direct_account_identity(&profile, &body, &headers, "Bearer fixture-a").unwrap();
    let bound: Value = serde_json::from_slice(&bound).unwrap();
    assert_eq!(bound["input"], input["input"]);
    assert_eq!(bound["tools"], input["tools"]);
    assert_eq!(
        bound["client_metadata"]["x-codex-installation-id"],
        profile.installation_id
    );
    assert_eq!(mapped["x-codex-installation-id"], profile.installation_id);
    assert_eq!(mapped["session-id"], headers["session-id"]);
    let (retried, retried_headers, _) =
        direct_account_identity(&profile, &body, &headers, "Bearer refreshed").unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&retried).unwrap(), bound);
    assert_eq!(retried_headers["session-id"], mapped["session-id"]);
}

#[test]
fn direct_wakeup_and_pelican_use_the_account_store_and_separate_connection_pools() {
    const CHILD: &str = "COCKPIT_TEST_DIRECT_IDENTITY";
    if std::env::var_os(CHILD).is_some() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(direct_wire_case());
        std::fs::write(std::env::var("COCKPIT_TEST_DIRECT_RESULT").unwrap(), "ok").unwrap();
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut command = crate::modules::test_support::isolated_test_process();
    let status = command.env(CHILD,"1")
        .env("COCKPIT_TOOLS_DATA_DIR",dir.path())
        .env("COCKPIT_TEST_DIRECT_RESULT",dir.path().join("verified"))
        .args(["--exact","modules::codex_local_access::direct_identity_tests::direct_wakeup_and_pelican_use_the_account_store_and_separate_connection_pools","--test-threads=1"])
        .status().unwrap();
    assert!(status.success());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("verified")).unwrap(),
        "ok"
    );
}

async fn direct_wire_case() {
    use axum::{extract::ConnectInfo, http::HeaderMap, routing::post, Json, Router};
    use std::net::SocketAddr;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route(
        "/responses",
        post(
            |ConnectInfo(peer): ConnectInfo<SocketAddr>,
             headers: HeaderMap,
             Json(body): Json<Value>| async move {
                Json(json!({"peer":peer.to_string(),"body":body,
            "installation":headers["x-codex-installation-id"].to_str().unwrap(),
            "session":headers["session-id"].to_str().unwrap(),
            "thread":headers["thread-id"].to_str().unwrap(),
            "authorization":headers["authorization"].to_str().unwrap()}))
            },
        ),
    );
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap()
    });
    let mut records = Vec::new();
    for id in ["a", "a", "b", "a"] {
        let account = account(id);
        let (body, headers) = if records.len() % 2 == 0 {
            build_pelican_request("fixture", "low", "<html/>", id).unwrap()
        } else {
            direct_codex_request(json!({"model":"fixture","input":"hi","stream":true})).unwrap()
        };
        let response = send_upstream_request_with_authorization_url(
            "POST",
            &format!("http://{addr}/responses"),
            "/responses",
            &headers,
            &body,
            &account,
            &format!("Bearer fixture-{id}"),
            None,
            Duration::from_secs(3),
            &CodexLocalAccessTimeouts::default(),
            CodexLocalAccessImageGenerationMode::Disabled,
            CodexLocalAccessRequestKind::Text,
        )
        .await
        .unwrap();
        let record: Value = response.json().await.unwrap();
        let expected = tokio::task::spawn_blocking(move || {
            crate::modules::codex_api_identity::resolve(&account)
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(record["installation"], expected.installation_id);
        assert_eq!(
            record["body"]["client_metadata"]["x-codex-installation-id"],
            expected.installation_id
        );
        assert_eq!(record["session"], headers["session-id"]);
        assert_eq!(record["thread"], record["session"]);
        assert_eq!(record["authorization"], format!("Bearer fixture-{id}"));
        records.push(record);
    }
    assert_eq!(records[0]["installation"], records[1]["installation"]);
    assert_eq!(records[0]["installation"], records[3]["installation"]);
    assert_ne!(records[0]["installation"], records[2]["installation"]);
    assert_eq!(records[0]["peer"], records[1]["peer"]);
    assert_eq!(records[0]["peer"], records[3]["peer"]);
    assert_ne!(records[0]["peer"], records[2]["peer"]);
    assert_ne!(records[0]["session"], records[1]["session"]);
    assert!(!NATIVE_API_RUNNING.load(Ordering::Acquire));
    task.abort();
}
