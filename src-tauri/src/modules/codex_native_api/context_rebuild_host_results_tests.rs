//! Exercise real HTTP/WS routing with host records and durable history replay.
use super::*;

fn host_result(namespace: &str, name: &str) -> Value {
    json!({"type":"function_call_output","namespace":namespace,"name":name,"output":"Original task handoff\n  preserve whitespace  "})
}

#[tokio::test]
async fn page_context_results_rebuild_a_cooling_owner_without_losing_history() {
    for streaming in [false, true] {
        let cipher = json!({"type":"reasoning","id":"page-reasoning-a","summary":[],"encrypted_content":"original-page-cipher-a"});
        let fixture = Fixture::new(
            true,
            vec![Step::Complete("page-a", Some(cipher.clone()))],
            vec![Step::Complete("page-b", None)],
        )
        .await;
        let context = [
            ("codex_apps_open_page", "{\"page_id\":null}"),
            (
                "codex_apps_selected_page_part_1_of_2",
                "Companion Page snapshot part 1 of 2:\n  original first part  ",
            ),
            (
                "codex_apps_selected_page_part_2_of_2",
                "Companion Page snapshot part 2 of 2:\n  original second part  ",
            ),
        ];
        let mut input = vec![
            message("user", "original task"),
            json!({"type":"custom_tool_call","name":"exec","call_id":"page-real-call","input":"read original file"}),
        ];
        input.extend(context.map(|(name, output)| {
            json!({"type":"function_call_output","namespace":"agent","name":name,"output":output})
        }));
        input.push(json!({"type":"custom_tool_call_output","call_id":"page-real-call","output":"original file contents"}));
        let first = body(input.clone(), None, streaming);
        let response = fixture.post(&first, None).await;
        assert_eq!(response.status(), 200);
        assert!(response.text().await.unwrap().contains("page-a"));
        input.extend(
            completed("page-a", Some(cipher))["output"]
                .as_array()
                .unwrap()
                .clone(),
        );
        input.push(message("user", "continue after the owner starts cooling"));
        fixture.host.cooling_a.store(true, Ordering::SeqCst);
        let follow = body(input.clone(), None, streaming);
        let unchanged = follow.clone();
        let response = tokio::time::timeout(Duration::from_secs(2), fixture.post(&follow, None))
            .await
            .expect("a ready backup must not wait for the original owner's multi-day cooldown");
        let status = response.status();
        let output = response.text().await.unwrap();
        assert_eq!(status, 200, "{output}");
        assert!(output.contains("page-b"));
        let sent = fixture.sent();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].account, "a");
        assert_eq!(sent[1].account, "b");
        assert_fresh(&sent[0], &sent[1], json!(input));
        assert_eq!(follow, unchanged);
        fixture.assert_owner("page-a", "a");
        assert_eq!(fixture.active().unwrap().0, "b");
        assert!(fixture.host.failures.lock().unwrap().is_empty());
        fixture.finish().await;
    }
}

#[tokio::test]
async fn host_results_continue_after_quota_failure_and_restart_without_losing_history() {
    for namespace in ["codex_app", "codex_tui"] {
        for streaming in [false, true] {
            let cipher = json!({"type":"reasoning","id":"reasoning-a","summary":[],"encrypted_content":"original-cipher-a"});
            let mut fixture = Fixture::new(
                true,
                vec![
                    Step::Complete("host-a", Some(cipher.clone())),
                    Step::HttpRateLimit,
                ],
                vec![
                    Step::Complete("host-b", None),
                    Step::Complete("host-b-next", None),
                ],
            )
            .await;
            let first = body(
                vec![
                    message("user", "original task"),
                    json!({"type":"function_call","name":"read_file","call_id":"real-call","arguments":"{}"}),
                    host_result(namespace, "create_thread"),
                    json!({"type":"function_call_output","call_id":"real-call","output":"original file contents"}),
                ],
                None,
                streaming,
            );
            let response = fixture.post(&first, None).await;
            assert_eq!(response.status(), 200);
            assert!(response.text().await.unwrap().contains("host-a"));
            let mut input = first["input"].as_array().unwrap().clone();
            input.extend(
                completed("host-a", Some(cipher.clone()))["output"]
                    .as_array()
                    .unwrap()
                    .clone(),
            );
            input.push(host_result(namespace, "send_message_to_thread"));
            input.push(message("user", "continue the same task"));
            let follow = body(input.clone(), Some("host-a"), streaming);
            let response = fixture.post(&follow, None).await;
            assert_eq!(response.status(), 200);
            assert!(response.text().await.unwrap().contains("host-b"));
            assert_eq!(
                fixture.host.failures.lock().unwrap().as_slice(),
                &[("a".into(), 429)]
            );
            let sent = fixture.sent();
            assert_eq!(
                sent.iter()
                    .map(|request| request.account.as_str())
                    .collect::<Vec<_>>(),
                ["a", "a", "b"]
            );
            assert_eq!(sent[0].body["input"], first["input"]);
            assert_fresh(&sent[1], &sent[2], json!(input));
            assert_eq!(fixture.active().unwrap().0, "b");
            fixture.assert_owner("host-a", "a");
            fixture.assert_owner("host-b", "b");

            fixture
                .handle
                .shutdown(Duration::from_secs(3))
                .await
                .unwrap();
            fixture.handle = start(
                "127.0.0.1:0".parse().unwrap(),
                fixture.host.clone(),
                fixture.path(),
                Options::default(),
            )
            .await
            .unwrap();
            let next = body(
                vec![message("user", "continue after restart")],
                Some("host-b"),
                streaming,
            );
            let response = fixture.post(&next, None).await;
            assert_eq!(response.status(), 200);
            assert!(response.text().await.unwrap().contains("host-b-next"));
            assert_eq!(fixture.sent()[3].account, "b");
            // A pure response-ID follow-up must retain a complete durable checkpoint.
            let db = Connection::open(fixture.path()).unwrap();
            let transcript: String = db.query_row("SELECT transcript FROM native_rebuild_checkpoints_v1 WHERE caller='caller' AND response='host-b-next'", [], |row| row.get(0)).unwrap();
            let transcript: Vec<Value> = serde_json::from_str(&transcript).unwrap();
            assert!(transcript.starts_with(&input));
            assert!(transcript.contains(&cipher));
            fixture.finish().await;
        }
    }
}

#[tokio::test]
async fn host_results_invalid_ids_reach_upstream_without_unlocking_cross_account_history() {
    let upstream_error = json!({
        "type":"invalid_request_error",
        "code":"invalid_request_error",
        "message":"Synthetic upstream rejected the host result call_id",
        "param":"input",
        "upstream_marker":"original-host-result-diagnostic"
    });
    let event = json!({"type":"error","status":400,"error":upstream_error});
    let fixture = Fixture::new(true, vec![Step::RawEvents(vec![event]); 8], vec![]).await;
    for (index, (namespace, id)) in ["codex_app", "codex_tui"]
        .into_iter()
        .flat_map(|namespace| {
            [json!("stale"), json!(" "), json!(null), json!(42)]
                .into_iter()
                .map(move |id| (namespace, id))
        })
        .enumerate()
    {
        let mut item = host_result(namespace, "create_thread");
        item["call_id"] = id;
        let original = body(
            vec![
                message("user", "old task"),
                json!({"type":"reasoning","summary":[],"encrypted_content":"unregistered-cipher"}),
                item,
            ],
            None,
            false,
        );
        let response = fixture.post(&original, None).await;
        assert_eq!(response.status(), 400);
        assert_eq!(
            response.json::<Value>().await.unwrap()["error"],
            upstream_error
        );
        let sent = fixture.sent();
        assert_eq!(sent.len(), index + 1, "ordinary forwarding must not retry");
        assert!(sent.iter().all(|request| request.account == "a"));
        assert_eq!(sent[index].body["input"], original["input"]);
        assert_eq!(fixture.host.selections.lock().unwrap().len(), index + 1);
        assert!(fixture.host.failures.lock().unwrap().is_empty());
        assert!(fixture.active().is_none());
        let db = Connection::open(fixture.path()).unwrap();
        for table in [
            "native_opaque_v1",
            "native_rebuild_checkpoints_v1",
            "native_rebuild_history_v1",
        ] {
            let count: i64 = db
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0, "a rejected host result must not publish {table}");
        }
    }
    fixture.finish().await;
}

#[derive(Clone, Copy, Debug)]
enum HostContinuation {
    FullWithoutPrevious,
    ResultDelta,
    ExactFullPrefix,
    FullPrefixWithoutPlaintextMarker,
}

#[tokio::test]
async fn websocket_host_results_continue_when_client_omits_plaintext_marker() {
    for namespace in ["codex_app", "codex_tui"] {
        for shape in [
            HostContinuation::FullWithoutPrevious,
            HostContinuation::ResultDelta,
            HostContinuation::ExactFullPrefix,
            HostContinuation::FullPrefixWithoutPlaintextMarker,
        ] {
            let call = json!({"type":"function_call","id":"fc-followup","namespace":"collaboration",
                "name":"followup_task","call_id":"followup-call",
                "arguments":json!({"target":"worker","message":"Synthetic follow-up task"}).to_string(),
                "encrypted_function_args":[]});
            let fixture = Fixture::new(
                true,
                vec![
                    Step::CompleteItems("host-ws-first", vec![call.clone()]),
                    Step::CompleteItems("host-ws-second", vec![]),
                ],
                vec![],
            )
            .await;
            fixture.host.passthrough.store(true, Ordering::SeqCst);
            let mut socket = fixture.socket().await;
            let first = body(
                vec![
                    message("user", "original task"),
                    json!({"type":"function_call","name":"read_file","call_id":"real-call","arguments":"{}"}),
                    host_result(namespace, "send_message_to_thread"),
                    json!({"type":"function_call_output","call_id":"real-call","output":"original file contents"}),
                ],
                None,
                true,
            );
            ws_send(&mut socket, first.clone()).await;
            let response = ws_event(&mut socket, "response.completed").await;
            assert_eq!(response["response"]["id"], "host-ws-first");
            assert_eq!(response["response"]["output"], json!([call.clone()]));
            let encrypted: i64 = Connection::open(fixture.path())
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM native_opaque_v1 WHERE kind='encrypted'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                encrypted, 0,
                "The explicit plaintext output is not ciphertext"
            );

            let result = json!({"type":"function_call_output","call_id":"followup-call","output":"follow-up delivered"});
            let mut full = first["input"].as_array().unwrap().clone();
            let mut replayed_call = call;
            if matches!(
                shape,
                HostContinuation::FullWithoutPrevious
                    | HostContinuation::FullPrefixWithoutPlaintextMarker
            ) {
                replayed_call
                    .as_object_mut()
                    .unwrap()
                    .remove("encrypted_function_args");
            }
            full.extend([replayed_call, result.clone()]);
            let input = if matches!(shape, HostContinuation::ResultDelta) {
                vec![result]
            } else {
                full.clone()
            };
            let previous = (!matches!(shape, HostContinuation::FullWithoutPrevious))
                .then_some("host-ws-first");
            let follow = body(input, previous, true);
            let unchanged = follow.clone();
            ws_send(&mut socket, follow.clone()).await;
            let response = ws_event(&mut socket, "response.completed").await;
            assert_eq!(response["response"]["id"], "host-ws-second");
            let sent = fixture.sent();
            assert_eq!(sent.len(), 2, "{namespace} {shape:?}");
            assert_eq!(sent[0].body["input"], first["input"]);
            assert_eq!(sent[1].body["input"], follow["input"]);
            assert_eq!(follow, unchanged);
            assert_eq!(
                *fixture.upstream.ws_connections.lock().unwrap(),
                vec!["a"],
                "Both turns must use the same upstream WebSocket"
            );
            let raw: String = Connection::open(fixture.path())
                .unwrap()
                .query_row(
                    "SELECT transcript FROM native_rebuild_checkpoints_v1 WHERE caller='caller' AND response='host-ws-second'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(serde_json::from_str::<Value>(&raw).unwrap(), json!(full));
            socket.close(None).await.unwrap();
            drop(socket);
            fixture.finish().await;
        }
    }
}
