//! Local reserve rejections never reach the synthetic upstream or poison its health.
use super::*;

fn deny(fixture: &Fixture, preparation: bool, ids: &[&str]) {
    let target = if preparation {
        &fixture.host.local_quota_preparation_denied
    } else {
        &fixture.host.local_quota_send_denied
    };
    *target.lock().unwrap() = ids.iter().map(|id| (*id).into()).collect();
    fixture.host.oauth.store(true, Ordering::SeqCst);
}

#[tokio::test]
async fn local_quota_http_unsent_rejection_uses_backup_without_remote_failure() {
    for preparation in [true, false] {
        let fixture =
            Fixture::new(false, vec![], vec![Step::Complete("b-local-quota", None)]).await;
        deny(&fixture, preparation, &["a"]);
        let response = fixture
            .post(&body(vec![message("user", "hello")], None, false), None)
            .await;
        let status = response.status();
        let output = response.text().await.unwrap();
        assert_eq!(status, 200, "preparation={preparation}: {output}");
        assert!(output.contains("b-local-quota"));
        let sent = fixture.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].account, "b");
        assert!(fixture.host.failures.lock().unwrap().is_empty());
        let selections = fixture.host.selections.lock().unwrap().clone();
        assert_eq!(selections.len(), 2);
        assert_eq!(selections[1].excluded_account_ids, ["a"]);
        fixture.finish().await;
    }
}

#[tokio::test]
async fn local_quota_all_candidates_denied_terminates_with_original_local_error() {
    for preparation in [true, false] {
        let fixture = Fixture::new(false, vec![], vec![]).await;
        deny(&fixture, preparation, &["a", "b"]);
        let response = tokio::time::timeout(
            Duration::from_secs(2),
            fixture.post(&body(vec![message("user", "hello")], None, false), None),
        )
        .await
        .expect("local exclusions must terminate without waiting for a network retry");
        assert_eq!(response.status(), 429);
        let output: Value = response.json().await.unwrap();
        assert_eq!(output["error"]["code"], "quota_reserved");
        assert!(fixture.sent().is_empty());
        assert!(fixture.host.failures.lock().unwrap().is_empty());
        assert!(fixture.host.selections.lock().unwrap().len() <= 3);
        fixture.finish().await;
    }
}

#[tokio::test]
async fn local_quota_unrebuildable_owner_is_never_sent_to_backup() {
    for preparation in [true, false] {
        let owned = json!({"type":"reasoning","id":"rs-local-owner-a","summary":[],"encrypted_content":"synthetic-local-quota-owner-a"});
        let fixture = Fixture::new(
            false,
            vec![Step::Complete("a-local-root", Some(owned))],
            vec![],
        )
        .await;
        let first = fixture.root().await;
        let previous = first["id"].as_str().unwrap();
        let mut input = vec![message("user", "first")];
        input.extend(first["output"].as_array().unwrap().iter().cloned());
        input.push(message("user", "continue"));
        deny(&fixture, preparation, &["a"]);
        let response = fixture.post(&body(input, None, false), None).await;
        let status = response.status();
        let output = response.text().await.unwrap();
        assert_eq!(status, 429, "{output}");
        let sent = fixture.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].account, "a");
        fixture.assert_owner(previous, "a");
        assert!(fixture.host.failures.lock().unwrap().is_empty());
        fixture.finish().await;
    }
}

#[tokio::test]
async fn local_quota_validated_history_rebuilds_before_switching_owner() {
    for preparation in [true, false] {
        let fixture = Fixture::new(
            true,
            vec![Step::Complete("a-local-root", None)],
            vec![Step::Complete("b-local-rebuilt", None)],
        )
        .await;
        fixture.root().await;
        deny(&fixture, preparation, &["a"]);
        let original = body(
            vec![message("user", "continue")],
            Some("a-local-root"),
            false,
        );
        let response = fixture
            .post(&original, Some("synthetic-state-a-local-root"))
            .await;
        let status = response.status();
        let output = response.text().await.unwrap();
        assert_eq!(status, 200, "preparation={preparation}: {output}");
        let sent = fixture.sent();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[1].account, "b");
        assert!(sent[1].body.get("previous_response_id").is_none());
        assert!(!sent[1].headers.contains_key("x-codex-turn-state"));
        assert_ne!(identity(&sent[0]), identity(&sent[1]));
        assert!(fixture.host.failures.lock().unwrap().is_empty());
        fixture.assert_owner("a-local-root", "a");
        fixture.finish().await;
    }
}

#[tokio::test]
async fn local_quota_unknown_opaque_input_does_not_cross_accounts() {
    for preparation in [true, false] {
        let fixture = Fixture::new(false, vec![], vec![]).await;
        deny(&fixture, preparation, &["a"]);
        let input = body(
            vec![
                message("user", "hello"),
                json!({"type":"reasoning","encrypted_content":"unknown-private-state","summary":[]}),
            ],
            None,
            false,
        );
        let response = fixture.post(&input, None).await;
        assert_ne!(response.status(), 200);
        response.bytes().await.unwrap();
        assert!(fixture.sent().is_empty());
        assert!(fixture.host.selections.lock().unwrap().len() <= 1);
        fixture.finish().await;
    }
}

#[tokio::test]
async fn local_quota_function_schema_property_names_are_not_account_state() {
    for rebuild in [false, true] {
        for preparation in [true, false] {
            let fixture =
                Fixture::new(rebuild, vec![], vec![Step::Complete("b-schema", None)]).await;
            deny(&fixture, preparation, &["a"]);
            let mut input = body(vec![message("user", "hello")], None, false);
            input["tools"] = json!([{"type":"function","name":"inspect_record","parameters":{
            "type":"object","properties":{
                "encrypted_content":{"type":"string"},
                "parent_response_id":{"type":"string"},
                "conversation_id":{"type":"string"}
            }}}]);
            let response = fixture.post(&input, None).await;
            let status = response.status();
            let output = response.text().await.unwrap();
            assert_eq!(
                status, 200,
                "rebuild={rebuild}, preparation={preparation}: {output}"
            );
            let sent = fixture.sent();
            assert_eq!(sent.len(), 1);
            assert_eq!(sent[0].account, "b");
            assert!(fixture.host.failures.lock().unwrap().is_empty());
            fixture.finish().await;
        }
    }
}

#[tokio::test]
async fn local_quota_rebuild_keeps_unrecognized_routing_references_off_backup() {
    for preparation in [true, false] {
        for reference in [
            json!({"parent_response_id":"unrecognized-account-state"}),
            json!({"x-codex-turn-state":"unrecognized-account-state"}),
            json!({"client_metadata":{"x-codex-turn-metadata":json!({"nested":{"parent_response_id":"unrecognized-account-state"}}).to_string()}}),
        ] {
            let fixture =
                Fixture::new(true, vec![Step::Complete("a-routing-root", None)], vec![]).await;
            fixture.root().await;
            deny(&fixture, preparation, &["a"]);
            let mut input = body(
                vec![message("user", "continue")],
                Some("a-routing-root"),
                false,
            );
            input
                .as_object_mut()
                .unwrap()
                .extend(reference.as_object().unwrap().clone());
            let response = fixture.post(&input, None).await;
            let status = response.status();
            let output = response.text().await.unwrap();
            assert_eq!(status, 429, "{output}");
            let sent = fixture.sent();
            assert_eq!(sent.len(), 1);
            assert_eq!(sent[0].account, "a");
            assert!(fixture.host.failures.lock().unwrap().is_empty());
            fixture.finish().await;
        }
    }
}

#[tokio::test]
async fn local_quota_duplicate_metadata_headers_all_require_portable_state() {
    for preparation in [true, false] {
        let fixture = Fixture::new(false, vec![], vec![]).await;
        deny(&fixture, preparation, &["a"]);
        let response = client()
            .post(format!("http://{}/v1/responses", fixture.handle.addr))
            .bearer_auth("synthetic-caller")
            .header(
                "x-codex-turn-metadata",
                json!({"session_id":"portable"}).to_string(),
            )
            .header(
                "x-codex-turn-metadata",
                json!({"parent_response_id":"private-state"}).to_string(),
            )
            .json(&body(vec![message("user", "hello")], None, false))
            .send()
            .await
            .unwrap();
        assert_ne!(response.status(), 200);
        response.bytes().await.unwrap();
        assert!(fixture.sent().is_empty());
        assert!(fixture.host.selections.lock().unwrap().len() <= 1);
        assert!(fixture.host.failures.lock().unwrap().is_empty());
        fixture.finish().await;
    }
}

#[tokio::test]
async fn local_quota_encoded_metadata_keeps_upstream_state_on_its_account() {
    for preparation in [true, false] {
        for in_header in [true, false] {
            for metadata in [
                json!({"x-codex-turn-state":"unknown-a"}).to_string(),
                json!({"parent_response_id":"unknown-a"}).to_string(),
                "invalid-json".to_owned(),
            ] {
                let fixture = Fixture::new(false, vec![], vec![]).await;
                deny(&fixture, preparation, &["a"]);
                let mut input = body(vec![message("user", "hello")], None, false);
                let mut request = client()
                    .post(format!("http://{}/v1/responses", fixture.handle.addr))
                    .bearer_auth("synthetic-caller");
                if in_header {
                    request = request.header("x-codex-turn-metadata", &metadata);
                } else {
                    input["client_metadata"] = json!({"x-codex-turn-metadata":metadata});
                }
                let response = request.json(&input).send().await.unwrap();
                assert_ne!(response.status(), 200);
                response.bytes().await.unwrap();
                assert!(fixture.sent().is_empty());
                assert!(fixture.host.selections.lock().unwrap().len() <= 1);
                assert!(fixture.host.failures.lock().unwrap().is_empty());
                fixture.finish().await;
            }
        }
    }
}

#[tokio::test]
async fn local_quota_portable_encoded_metadata_still_allows_unsent_reselection() {
    for preparation in [true, false] {
        for in_header in [true, false] {
            let fixture =
                Fixture::new(false, vec![], vec![Step::Complete("b-metadata", None)]).await;
            deny(&fixture, preparation, &["a"]);
            let metadata =
                json!({"session_id":"portable-session","thread_id":"portable-thread"}).to_string();
            let mut input = body(vec![message("user", "hello")], None, false);
            let mut request = client()
                .post(format!("http://{}/v1/responses", fixture.handle.addr))
                .bearer_auth("synthetic-caller");
            if in_header {
                request = request.header("x-codex-turn-metadata", &metadata);
            } else {
                input["client_metadata"] = json!({"x-codex-turn-metadata":metadata});
            }
            let response = request.json(&input).send().await.unwrap();
            let status = response.status();
            let output = response.text().await.unwrap();
            assert_eq!(status, 200, "{output}");
            let sent = fixture.sent();
            assert_eq!(sent.len(), 1);
            assert_eq!(sent[0].account, "b");
            assert!(fixture.host.failures.lock().unwrap().is_empty());
            fixture.finish().await;
        }
    }
}

#[tokio::test]
async fn local_quota_server_references_and_partial_tool_history_do_not_cross_accounts() {
    for item in [
        json!({"type":"item_reference","id":"msg_account_a"}),
        json!({"type":"message","role":"assistant","signature":"account-a-signature","content":[{"type":"output_text","text":"earlier answer"}]}),
        json!({"type":"function_call_output","call_id":"missing_call","output":"result"}),
        json!({"type":"message","role":"user","content":[{"type":"input_file","file_id":"file_account_a"}]}),
    ] {
        let fixture = Fixture::new(false, vec![], vec![]).await;
        deny(&fixture, true, &["a"]);
        let input = body(vec![message("user", "hello"), item], None, false);
        let response = fixture.post(&input, None).await;
        assert_ne!(response.status(), 200);
        response.bytes().await.unwrap();
        assert!(fixture.sent().is_empty());
        assert!(fixture.host.selections.lock().unwrap().len() <= 1);
        fixture.finish().await;
    }
}

#[tokio::test]
async fn local_quota_after_remote_failure_preserves_remote_error() {
    let fixture = Fixture::new(false, vec![Step::HttpRateLimit], vec![]).await;
    deny(&fixture, false, &["b"]);
    let response = fixture
        .post(&body(vec![message("user", "hello")], None, false), None)
        .await;
    assert_eq!(response.status(), 429);
    let output = response.text().await.unwrap();
    assert!(output.contains("Synthetic quota exhausted"), "{output}");
    let sent = fixture.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].account, "a");
    assert!(fixture
        .host
        .failures
        .lock()
        .unwrap()
        .iter()
        .all(|(id, _)| id == "a"));
    fixture.finish().await;
}

#[tokio::test]
async fn local_quota_conversation_header_does_not_cross_accounts() {
    let fixture = Fixture::new(false, vec![], vec![]).await;
    deny(&fixture, true, &["a"]);
    let response = client()
        .post(format!("http://{}/v1/responses", fixture.handle.addr))
        .bearer_auth("synthetic-caller")
        .header("conversation_id", "conversation-account-a")
        .json(&body(vec![message("user", "hello")], None, false))
        .send()
        .await
        .unwrap();
    assert_ne!(response.status(), 200);
    response.bytes().await.unwrap();
    assert!(fixture.sent().is_empty());
    assert!(fixture.host.selections.lock().unwrap().len() <= 1);
    fixture.finish().await;
}

#[tokio::test]
async fn local_quota_reused_websocket_reconnects_only_for_portable_next_turn() {
    for bound_owner in [false, true] {
        let fixture = Fixture::new(
            false,
            vec![Step::Complete("a-local-ws-root", None)],
            vec![Step::Complete("b-local-ws-next", None)],
        )
        .await;
        fixture.host.oauth.store(true, Ordering::SeqCst);
        fixture.host.passthrough.store(true, Ordering::SeqCst);
        let mut socket = fixture.socket().await;
        for turn in 0..2 {
            if turn == 1 {
                deny(&fixture, false, &["a"]);
            }
            let mut request = body(
                vec![message(
                    "user",
                    if turn == 0 {
                        "first"
                    } else {
                        "next complete input"
                    },
                )],
                (turn == 1 && bound_owner).then_some("a-local-ws-root"),
                true,
            );
            request["type"] = json!("response.create");
            socket
                .send(Message::Text(request.to_string().into()))
                .await
                .unwrap();
            let terminal = tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    let frame = socket
                        .next()
                        .await
                        .expect("socket closed before terminal frame")
                        .unwrap();
                    if let Message::Text(text) = frame {
                        let event: Value = serde_json::from_str(text.as_str()).unwrap();
                        if matches!(event["type"].as_str(), Some("error" | "response.completed")) {
                            break event;
                        }
                    }
                }
            })
            .await
            .expect("second turn must terminate within its opening budget");
            if turn == 0 {
                assert_eq!(terminal["response"]["id"], "a-local-ws-root");
            } else if bound_owner {
                assert_eq!(terminal["error"]["code"], "quota_reserved");
            } else {
                assert_eq!(terminal["response"]["id"], "b-local-ws-next", "{terminal}");
            }
        }
        let sent = fixture.sent();
        assert_eq!(sent.len(), if bound_owner { 1 } else { 2 });
        assert_eq!(sent[0].account, "a");
        if !bound_owner {
            assert_eq!(sent[1].account, "b");
        }
        let _ = socket.close(None).await;
        fixture.finish().await;
    }
}

#[tokio::test]
async fn local_quota_new_websocket_reselects_before_inference_send() {
    // Preparation rejection, rejection before connect, and rejection after handshake.
    for (preparation, after_handshake) in [(true, false), (false, false), (false, true)] {
        let fixture = Fixture::new(false, vec![], vec![Step::Complete("b-local-ws", None)]).await;
        deny(&fixture, preparation, &["a"]);
        fixture.host.passthrough.store(true, Ordering::SeqCst);
        fixture
            .host
            .local_quota_allow_first_check
            .store(after_handshake, Ordering::SeqCst);
        let mut socket = fixture.socket().await;
        let mut request = body(vec![message("user", "hello")], None, true);
        request["type"] = json!("response.create");
        socket
            .send(Message::Text(request.to_string().into()))
            .await
            .unwrap();
        let output = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let frame = socket
                    .next()
                    .await
                    .expect("socket closed before completion")
                    .unwrap();
                if let Message::Text(text) = frame {
                    let event: Value = serde_json::from_str(text.as_str()).unwrap();
                    assert_ne!(event["type"], "error", "{event}");
                    if event["type"] == "response.completed" {
                        break event;
                    }
                }
            }
        })
        .await
        .expect("local WebSocket rejection must not retry indefinitely");
        assert_eq!(output["response"]["id"], "b-local-ws");
        let sent = fixture.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].account, "b");
        assert!(fixture.host.failures.lock().unwrap().is_empty());
        let connections = fixture.upstream.ws_connections.lock().unwrap().clone();
        assert_eq!(connections.contains(&"a".to_owned()), after_handshake);
        socket.close(None).await.unwrap();
        fixture.finish().await;
    }
}
