//! Exercise real HTTP routing with host records, a 429 and durable replay on B.
use super::*;

fn host_result(name: &str) -> Value {
    json!({"type":"function_call_output","namespace":"codex_app","name":name,"output":"Original task handoff\n  preserve whitespace  "})
}

#[tokio::test]
async fn host_results_continue_after_quota_failure_and_restart_without_losing_history() {
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
                host_result("create_thread"),
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
        input.push(host_result("send_message_to_thread"));
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

#[tokio::test]
async fn host_results_invalid_ids_cannot_unlock_cross_account_history() {
    let fixture = Fixture::new(true, vec![], vec![]).await;
    for id in [json!("stale"), json!(" "), json!(null), json!(42)] {
        let mut item = host_result("create_thread");
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
            response.json::<Value>().await.unwrap()["error"]["code"],
            "continuation_history_incomplete"
        );
        assert!(fixture.sent().is_empty());
    }
    fixture.finish().await;
}
