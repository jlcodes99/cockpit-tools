use super::*;

fn with_installation(mut body: Value) -> Value {
    body["installation_id"] = json!("client-installation");
    body["client_metadata"]["x-codex-installation-id"] = json!("client-installation");
    body["client_metadata"]["x-codex-turn-metadata"] = json!(json!({
        "installation_id":"client-installation", "session_id":"client-session", "thread_id":"client-thread"
    }).to_string());
    body
}

fn check_installation(sent: &Sent) {
    let profile = installation_profile(&sent.account);
    assert_eq!(
        sent.headers["x-codex-installation-id"],
        profile.installation_id
    );
    assert_eq!(sent.body["installation_id"], profile.installation_id);
    assert_eq!(
        sent.body["client_metadata"]["x-codex-installation-id"],
        profile.installation_id
    );
    let canonical: Value = serde_json::from_str(
        sent.body["client_metadata"]["x-codex-turn-metadata"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(canonical["installation_id"], profile.installation_id);
    assert_eq!(
        canonical["thread_id"],
        sent.body["client_metadata"]["thread_id"]
    );
    assert_eq!(
        canonical["session_id"],
        sent.body["client_metadata"]["session_id"]
    );
}

#[tokio::test]
async fn account_installation_http_and_sse_rebuild_keep_cipher_and_fresh_account_identity() {
    for streaming in [false, true] {
        let fixture = Fixture::new(true,
            vec![Step::Complete("a-first", Some(json!({"type":"reasoning","id":"reasoning-a","encrypted_content":"opaque-reasoning"})))],
            vec![Step::Complete("b-replayed", None)],
        ).await;
        let first = with_installation(body(vec![message("user", "first")], None, streaming));
        let response = fixture.post(&first, None).await;
        assert_eq!(response.status(), 200);
        assert!(response.text().await.unwrap().contains("a-first"));
        fixture.host.removed_a.store(true, Ordering::SeqCst);
        let next = with_installation(body(
            vec![message("user", "continue")],
            Some("a-first"),
            streaming,
        ));
        let response = fixture.post(&next, None).await;
        assert_eq!(response.status(), 200);
        assert!(response.text().await.unwrap().contains("b-replayed"));
        let sent = fixture.sent();
        assert_eq!(sent.len(), 2);
        for record in &sent {
            check_installation(record);
        }
        assert_eq!(sent[0].account, "a");
        assert_eq!(sent[1].account, "b");
        assert_ne!(
            sent[0].headers["x-codex-installation-id"],
            sent[1].headers["x-codex-installation-id"]
        );
        assert_ne!(identity(&sent[0]), identity(&sent[1]));
        assert!(sent[1].body["input"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["encrypted_content"] == "opaque-reasoning"));
        assert!(sent[1].body.get("previous_response_id").is_none());
        fixture.assert_owner("b-replayed", "b");
        fixture.finish().await;
    }
}

#[tokio::test]
async fn account_installation_websocket_and_http_bridge_rebuild_use_selected_profiles() {
    for passthrough in [false, true] {
        let fixture = Fixture::new(
            true,
            vec![Step::Complete("a-first", None)],
            vec![Step::Complete("b-replayed", None)],
        )
        .await;
        fixture
            .host
            .passthrough
            .store(passthrough, Ordering::SeqCst);
        let mut socket = fixture.socket().await;
        let mut first = with_installation(body(vec![message("user", "first")], None, false));
        first["type"] = json!("response.create");
        socket
            .send(Message::Text(first.to_string().into()))
            .await
            .unwrap();
        assert_eq!(
            ws_event(&mut socket, "response.completed").await["response"]["id"],
            "a-first"
        );
        fixture.host.removed_a.store(true, Ordering::SeqCst);
        let mut next = with_installation(body(
            vec![message("user", "continue")],
            Some("a-first"),
            false,
        ));
        next["type"] = json!("response.create");
        socket
            .send(Message::Text(next.to_string().into()))
            .await
            .unwrap();
        assert_eq!(
            ws_event(&mut socket, "response.completed").await["response"]["id"],
            "b-replayed"
        );
        let sent = fixture.sent();
        assert_eq!(sent.len(), 2);
        for record in &sent {
            check_installation(record);
        }
        assert_ne!(
            sent[0].headers["x-codex-installation-id"],
            sent[1].headers["x-codex-installation-id"]
        );
        assert_ne!(identity(&sent[0]), identity(&sent[1]));
        socket.close(None).await.unwrap();
        fixture.finish().await;
    }
}
