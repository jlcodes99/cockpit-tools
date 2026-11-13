use super::*;

#[tokio::test]
async fn claude_compat_cache_keys_are_stable_and_account_scoped() {
    for retry in [false, true] {
        let (mut handle, host, source, task, _dir) =
            identity_fixture(WireApi::Responses, false, retry).await;
        host.oauth.store(true, Ordering::SeqCst);
        let client = http_client();
        let body = json!({
            "model":"m", "max_tokens":32,
            "messages":[{"role":"user","content":"synthetic prefix"}],
            "metadata":{"user_id":"{\"session_id\":\"claude-metadata-session\"}"}
        });
        for (session, agent) in [
            (None, None),
            (Some("claude-metadata-session"), None),
            (Some("claude-metadata-session"), Some("agent-b")),
            (Some("claude-other-session"), None),
        ] {
            let mut request = client
                .post(format!("http://{}/v1/messages", handle.addr))
                .bearer_auth("key")
                .json(&body);
            if let Some(session) = session {
                request = request.header("x-claude-code-session-id", session);
            }
            if let Some(agent) = agent {
                request = request.header("x-claude-code-agent-id", agent);
            }
            let response = request.send().await.unwrap();
            let status = response.status();
            let text = response.text().await.unwrap();
            assert_eq!(status, 200, "{text}");
        }
        host.oauth.store(false, Ordering::SeqCst);
        let third_party = client
            .post(format!("http://{}/v1/messages", handle.addr))
            .bearer_auth("key")
            .json(&body)
            .send().await.unwrap();
        assert_eq!(third_party.status(), 200);
        third_party.bytes().await.unwrap();
        let seen = source.seen.lock().unwrap().clone();
        assert!(seen.iter().all(|record| {
            !record.headers.contains_key("x-cockpit-claude-cache-scope")
                && !record.headers.contains_key("x-claude-code-session-id")
        }));
        assert!(seen.last().unwrap().body.get("prompt_cache_key").is_none());
        let keys: Vec<_> = seen[..seen.len()-1].iter()
            .filter(|record| record.headers["x-test-account"] == if retry { "b" } else { "a" })
            .map(|record| record.body["prompt_cache_key"].as_str().unwrap())
            .collect();
        assert_eq!(keys.len(), 4);
        assert_eq!(keys[0].len(), 64);
        assert_eq!(keys[0], keys[1]);
        assert_ne!(keys[0], keys[2]);
        assert_ne!(keys[0], keys[3]);
        if retry {
            assert_ne!(seen[0].body["prompt_cache_key"], keys[0]);
        }
        handle.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
    }
}

#[tokio::test]
async fn session_mapping_defaults_on_and_reuses_mapping_after_reenabling() {
    let (mut handle, host, source, task, _dir) =
        identity_fixture(WireApi::Responses, false, false).await;
    host.oauth.store(true, Ordering::SeqCst);
    let body = json!({"model":"m","input":[],"client_metadata":{
        "session_id":"client-session","thread_id":"client-thread"
    }});
    for enabled in [None, Some(false), Some(true)] {
        if let Some(enabled) = enabled {
            handle.set_session_identity_mapping_enabled(enabled);
        }
        let response = http_client()
            .post(format!("http://{}/v1/responses", handle.addr))
            .bearer_auth("key")
            .header("session-id", "client-session")
            .header("thread-id", "client-thread")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let _: Value = response.json().await.unwrap();
    }
    let contexts = captured(&host, 3).await;
    let seen = source.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 3);
    for (context, record) in contexts.iter().zip(&seen) {
        assert_sent(
            context,
            record,
            Some("client-session"),
            Some("client-thread"),
        );
    }
    for (header, field, original) in [
        ("session-id", "session_id", "client-session"),
        ("thread-id", "thread_id", "client-thread"),
    ] {
        assert_ne!(seen[0].headers[header], original);
        assert_eq!(seen[1].headers[header], original);
        assert_eq!(seen[1].body["client_metadata"][field], original);
        assert_eq!(seen[2].headers[header], seen[0].headers[header]);
        assert_eq!(
            seen[2].body["client_metadata"][field],
            seen[0].body["client_metadata"][field]
        );
    }
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn disabled_session_mapping_preserves_distinct_header_flat_and_canonical_values() {
    let (mut handle, host, source, task, _dir) = identity_fixture_with_options(
        WireApi::Responses,
        false,
        false,
        Options {
            session_identity_mapping_enabled: false,
            ..Default::default()
        },
    )
    .await;
    host.oauth.store(true, Ordering::SeqCst);
    let canonical = json!({
        "session_id":"canonical-session","thread_id":"canonical-child",
        "parent_thread_id":"canonical-parent","turn_id":"canonical-turn",
        "window_id":"canonical-child:3"
    });
    let body = json!({"model":"m","input":[],"prompt_cache_key":"client-cache",
        "client_metadata":{
            "session_id":"flat-session","thread_id":"flat-thread",
            "x-codex-parent-thread-id":"flat-parent",
            "x-codex-turn-metadata":canonical.to_string()
        }
    });
    for path in ["responses", "responses/compact"] {
        let response = http_client()
            .post(format!("http://{}/v1/{path}", handle.addr))
            .bearer_auth("key")
            .header("Session-Id", "Header-Session")
            .header("Thread-Id", "Header-Thread")
            .header("x-codex-parent-thread-id", "header-parent")
            .header("x-client-request-id", "header-request")
            .header("x-codex-window-id", "Header-Thread:3")
            .header("x-codex-turn-metadata", canonical.to_string())
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let _: Value = response.json().await.unwrap();
    }
    let contexts = captured(&host, 2).await;
    let seen = source.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2);
    for (context, record) in contexts.iter().zip(&seen) {
        assert_sent(
            context,
            record,
            Some("Header-Session"),
            Some("Header-Thread"),
        );
        for (header, original) in [
            ("session-id", "Header-Session"),
            ("thread-id", "Header-Thread"),
            ("x-codex-parent-thread-id", "header-parent"),
            ("x-client-request-id", "header-request"),
            ("x-codex-window-id", "Header-Thread:3"),
        ] {
            assert_eq!(record.headers[header], original);
        }
        assert_eq!(record.body["client_metadata"], body["client_metadata"]);
        assert_eq!(record.body["prompt_cache_key"], "client-cache");
        let header_canonical: Value =
            serde_json::from_str(record.headers["x-codex-turn-metadata"].to_str().unwrap())
                .unwrap();
        assert_eq!(header_canonical, canonical);
    }
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn session_mapping_passthrough_keeps_original_ids_on_retries() {
    for (wire, oauth) in [
        (WireApi::Responses, true),
        (WireApi::Responses, false),
        (WireApi::Chat, false),
    ] {
        for session_header in ["session-id", "session_id"] {
            let (mut handle, host, source, task, _dir) = identity_fixture(wire, false, true).await;
            host.oauth.store(oauth, Ordering::SeqCst);
            handle.set_session_identity_mapping_enabled(!oauth);
            let (path, body) = if wire == WireApi::Chat {
                (
                    "chat/completions",
                    json!({"model":"m","messages":[{"role":"user","content":"hello"}]}),
                )
            } else {
                ("responses", json!({"model":"m","input":[],"prompt_cache_key":"client-cache"}))
            };
            let response = http_client()
                .post(format!("http://{}/v1/{path}", handle.addr))
                .bearer_auth("key")
                .header(session_header, "client-session")
                .header("thread-id", "client-thread")
                .json(&body)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200);
            let _: Value = response.json().await.unwrap();
            let contexts = captured(&host, 1).await;
            let context = &contexts[0];
            let seen = source.seen.lock().unwrap().clone();
            assert_eq!(
                seen.len(),
                2,
                "both account attempts must reach the fixture"
            );
            assert_eq!(context.attempts.len(), 2);
            assert_ne!(
                seen[0].headers["x-test-account"],
                seen[1].headers["x-test-account"]
            );
            for (attempt, record) in context.attempts.iter().zip(&seen) {
                assert_eq!(record.headers[session_header], "client-session");
                let absent_header = if session_header == "session-id" {
                    "session_id"
                } else {
                    "session-id"
                };
                assert!(
                    !record.headers.contains_key(absent_header),
                    "passthrough must not manufacture another session header"
                );
                assert_eq!(record.headers["thread-id"], "client-thread");
                if wire == WireApi::Responses {
                    assert_eq!(record.body["prompt_cache_key"], "client-cache");
                }
                assert_eq!(
                    attempt.upstream_session_id.as_deref(),
                    Some("client-session")
                );
                assert_eq!(attempt.upstream_thread_id.as_deref(), Some("client-thread"));
            }
            assert_sent(
                context,
                &seen[1],
                Some("client-session"),
                Some("client-thread"),
            );
            handle.shutdown(Duration::from_secs(2)).await.unwrap();
            task.abort();
        }
    }
}

#[tokio::test]
async fn provider_websocket_preserves_handshake_and_message_ids_with_mapping_enabled() {
    let (mut handle, host, source, task, _dir) =
        identity_fixture(WireApi::Responses, true, false).await;
    let mut request = format!("ws://{}/v1/responses", handle.addr)
        .into_client_request()
        .unwrap();
    for (name, value) in [
        ("authorization", "Bearer key"),
        ("session-id", "socket-session"),
        ("thread-id", "socket-thread"),
    ] {
        request.headers_mut().insert(name, value.parse().unwrap());
    }
    let mut socket = tokio_tungstenite::connect_async(request).await.unwrap().0;
    let metadata = json!({
        "session_id":"message-session","thread_id":"message-thread",
        "x-codex-turn-metadata":json!({
            "session_id":"canonical-session","thread_id":"canonical-thread"
        }).to_string()
    });
    for _ in 0..2 {
        socket
            .send(Message::Text(
                json!({"type":"response.create","model":"m","input":[],
                    "previous_response_id":null,"client_metadata":metadata})
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        complete_socket(&mut socket).await;
    }
    let contexts = captured(&host, 2).await;
    let seen = source.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2);
    assert_eq!(source.connections.load(Ordering::SeqCst), 1);
    for (context, record) in contexts.iter().zip(&seen) {
        assert_sent(
            context,
            record,
            Some("socket-session"),
            Some("socket-thread"),
        );
        assert_eq!(record.headers["session-id"], "socket-session");
        assert_eq!(record.headers["thread-id"], "socket-thread");
        assert_eq!(record.body["client_metadata"], metadata);
        assert_eq!(context.attempts[0].transport, "websocket");
    }
    socket.close(None).await.unwrap();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}
