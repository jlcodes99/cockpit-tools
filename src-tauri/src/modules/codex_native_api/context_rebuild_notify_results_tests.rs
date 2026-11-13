//! Synthetic protocol regression: exec notifications are distinct custom results.
//! These fixtures contain no real chat, image, account, or credential material.
use super::*;

fn custom_call(id: &str) -> Value {
    json!({"type":"custom_tool_call","call_id":id,"name":"exec",
        "input":"text('synthetic result'); notify('synthetic progress');"})
}

fn result_parts(label: &str) -> Vec<Value> {
    vec![
        json!({"type":"input_text","text":format!("Script completed\n  {label}  \n")}),
        json!({"type":"input_image","image_url":"data:image/png;base64,c3ludGhldGljLWltYWdl","detail":"original"}),
        json!({"type":"input_text","text":"  image receipt\n"}),
    ]
}

fn result_group(id: &str, label: &str) -> Vec<Value> {
    vec![
        json!({"type":"custom_tool_call_output","call_id":id,"output":result_parts(label)}),
        json!({"type":"custom_tool_call_output","call_id":id,"name":"exec","output":" first progress\n  "}),
        json!({"type":"custom_tool_call_output","call_id":id,"name":"exec","output":"\nsecond progress  "}),
    ]
}

fn combined_result(id: &str, label: &str) -> Value {
    let mut parts = result_parts(label);
    parts.extend([
        json!({"type":"input_text","text":" first progress\n  "}),
        json!({"type":"input_text","text":"\nsecond progress  "}),
    ]);
    json!({"type":"custom_tool_call_output","call_id":id,"output":parts})
}

fn first_input() -> Vec<Value> {
    let mut input = vec![
        message("user", "synthetic initial request"),
        custom_call("done-call"),
    ];
    input.extend(result_group("done-call", "first payload"));
    input
}

fn cipher() -> Value {
    json!({"type":"reasoning","summary":[],"encrypted_content":"synthetic-original-account-cipher"})
}

fn original_full_input() -> Vec<Value> {
    let mut input = first_input();
    input.extend([cipher(), custom_call("pending-call")]);
    input.extend(result_group("pending-call", "second payload"));
    input
}

fn expected_replay() -> Value {
    json!([
        message("user", "synthetic initial request"),
        custom_call("done-call"),
        combined_result("done-call", "first payload"),
        cipher(),
        custom_call("pending-call"),
        combined_result("pending-call", "second payload")
    ])
}

async fn seed(fixture: &Fixture, streaming: bool) {
    // Establish accepted portable history before introducing any multi-result
    // custom call. Starting with unpaired history would test admission instead
    // of failover and would not create a checkpoint on the baseline.
    let response = fixture
        .post(
            &body(
                vec![message("user", "synthetic initial request")],
                None,
                streaming,
            ),
            None,
        )
        .await;
    assert_eq!(response.status(), 200);
    assert!(response.text().await.unwrap().contains("notify-a-start"));
    let response = fixture
        .post(
            &body(
                result_group("done-call", "first payload"),
                Some("notify-a-start"),
                streaming,
            ),
            None,
        )
        .await;
    assert_eq!(response.status(), 200);
    assert!(response.text().await.unwrap().contains("notify-a-root"));
}

fn checkpoint(fixture: &Fixture, caller: &str, response: &str) -> Vec<Value> {
    use sha2::{Digest, Sha256};
    let db = Connection::open(fixture.path()).unwrap();
    let raw: Option<String> = db
        .query_row(
            "SELECT transcript FROM native_rebuild_checkpoints_v1 WHERE caller=?1 AND response=?2",
            [caller, response],
            |row| row.get(0),
        )
        .optional()
        .unwrap();
    if let Some(raw) = raw {
        return serde_json::from_str(&raw).unwrap();
    }
    // Match the production legacy/hot-first lookup, then independently verify
    // every ordered archived item instead of assuming a hot-cache row exists.
    let manifest: String = db
        .query_row(
            "SELECT manifest FROM native_rebuild_history_v1 WHERE caller=?1 AND response=?2",
            [caller, response],
            |row| row.get(0),
        )
        .unwrap();
    let digests: Vec<String> = serde_json::from_str(&manifest).unwrap();
    digests
        .into_iter()
        .map(|digest| {
            let compressed: Vec<u8> = db
                .query_row(
                    "SELECT body FROM native_rebuild_items_v1 WHERE caller=?1 AND digest=?2",
                    [caller, digest.as_str()],
                    |row| row.get(0),
                )
                .unwrap();
            let bytes = zstd::stream::decode_all(compressed.as_slice()).unwrap();
            assert_eq!(
                format!("{:x}", Sha256::digest(&bytes)),
                digest,
                "synthetic checkpoint item integrity"
            );
            serde_json::from_slice(&bytes).unwrap()
        })
        .collect()
}

fn assert_no_backup_checkpoint(fixture: &Fixture) {
    let db = Connection::open(fixture.path()).unwrap();
    for table in ["native_rebuild_checkpoints_v1", "native_rebuild_history_v1"] {
        let count: i64 = db
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE account='b'"),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 0,
            "an incomplete or unauthorized replay must not publish B history in {table}"
        );
    }
}

#[tokio::test]
async fn notify_results_http_quota_and_revoked_token_failover_preserve_parts_and_durable_source() {
    for (failure, failure_status) in [(Step::HttpRateLimit, 429), (Step::HttpTokenRevoked, 401)] {
        for streaming in [false, true] {
            let mut fixture = Fixture::new(
                true,
                vec![
                    Step::CompleteItems("notify-a-start", vec![custom_call("done-call")]),
                    Step::CompleteItems(
                        "notify-a-root",
                        vec![cipher(), custom_call("pending-call")],
                    ),
                    failure.clone(),
                ],
                vec![
                    Step::Complete("notify-b-root", None),
                    Step::Complete("notify-b-next", None),
                ],
            )
            .await;
            seed(&fixture, streaming).await;
            let old_checkpoint = checkpoint(&fixture, "caller", "notify-a-root");
            let follow = body(
                result_group("pending-call", "second payload"),
                Some("notify-a-root"),
                streaming,
            );
            let unchanged = follow.clone();
            let response = fixture.post(&follow, None).await;
            let status = response.status();
            let text = response.text().await.unwrap();
            assert_eq!(
                status, 200,
                "failure={failure_status}, streaming={streaming}: {text}"
            );
            assert!(text.contains("notify-b-root"));
            let sent = fixture.sent();
            assert_eq!(
                sent.iter().map(|v| v.account.as_str()).collect::<Vec<_>>(),
                ["a", "a", "a", "b"]
            );
            assert_eq!(
                sent[2].body["input"], follow["input"],
                "the same-owner request keeps its original result envelopes"
            );
            assert_fresh(&sent[2], &sent[3], expected_replay());
            assert_eq!(follow, unchanged);
            assert_eq!(
                fixture.host.failures.lock().unwrap().as_slice(),
                &[("a".into(), failure_status)]
            );
            assert_eq!(fixture.active().unwrap().0, "b");
            fixture.assert_owner("notify-a-root", "a");
            fixture.assert_owner("notify-b-root", "b");
            assert_eq!(
                checkpoint(&fixture, "caller", "notify-a-root"),
                old_checkpoint
            );
            let saved = checkpoint(&fixture, "caller", "notify-b-root");
            assert!(
                saved.starts_with(&original_full_input()),
                "the successful B checkpoint keeps original distinct results"
            );
            assert_eq!(
                saved
                    .iter()
                    .filter(|v| v["type"] == "custom_tool_call_output")
                    .count(),
                6
            );

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
                vec![message("user", "synthetic continuation after restart")],
                Some("notify-b-root"),
                streaming,
            );
            let response = fixture.post(&next, None).await;
            assert_eq!(response.status(), 200);
            assert!(response.text().await.unwrap().contains("notify-b-next"));
            let sent = fixture.sent();
            assert_eq!(sent[4].account, "b");
            assert_eq!(sent[4].body["input"], next["input"]);
            assert!(
                checkpoint(&fixture, "caller", "notify-b-next").starts_with(&original_full_input())
            );
            fixture.finish().await;
        }
    }
}

#[tokio::test]
async fn notify_results_long_websocket_replay_keeps_history_and_publishes_backup() {
    let fixture = Fixture::new(
        true,
        vec![
            Step::CompleteItems("notify-a-start", vec![custom_call("done-call")]),
            Step::CompleteItems("notify-a-root", vec![cipher(), custom_call("pending-call")]),
            Step::HttpRateLimit,
        ],
        vec![Step::Complete("notify-b-root", None)],
    )
    .await;
    seed(&fixture, false).await;
    let long_output = "synthetic history segment ".repeat(25_000);
    let mut results = result_group("pending-call", "second payload");
    results[0]["output"][0]["text"] = json!(long_output);
    let mut expected = expected_replay();
    expected[5]["output"][0]["text"] = json!(long_output);
    let mut socket = fixture.socket().await;
    ws_send(
        &mut socket,
        body(results.clone(), Some("notify-a-root"), true),
    )
    .await;
    assert_eq!(
        ws_event(&mut socket, "response.completed").await["response"]["id"],
        "notify-b-root"
    );

    let sent = fixture.sent();
    assert_eq!(sent[2].body["input"], json!(results));
    assert_fresh(&sent[2], &sent[3], expected);
    assert_eq!(fixture.active().unwrap().0, "b");
    fixture.assert_owner("notify-b-root", "b");
    let saved = checkpoint(&fixture, "caller", "notify-b-root");
    assert_eq!(&saved[7..10], results.as_slice());
    socket.close(None).await.unwrap();
    drop(socket);
    fixture.finish().await;
}

#[tokio::test]
async fn notify_results_cancelled_candidate_never_publishes_backup_owner() {
    let fixture = Fixture::new(
        true,
        vec![
            Step::CompleteItems("notify-a-start", vec![custom_call("done-call")]),
            Step::CompleteItems("notify-a-root", vec![cipher(), custom_call("pending-call")]),
            Step::HttpRateLimit,
        ],
        vec![Step::Hold("notify-b-pending")],
    )
    .await;
    seed(&fixture, false).await;
    let before = fixture.active().unwrap();
    let old_checkpoint = checkpoint(&fixture, "caller", "notify-a-root");
    let mut socket = fixture.socket().await;
    ws_send(
        &mut socket,
        body(
            result_group("pending-call", "second payload"),
            Some("notify-a-root"),
            true,
        ),
    )
    .await;
    assert_eq!(
        ws_event(&mut socket, "response.created").await["response"]["id"],
        "notify-b-pending"
    );
    assert_eq!(fixture.active().unwrap(), before);
    socket.close(None).await.unwrap();
    drop(socket);
    tokio::time::timeout(Duration::from_secs(4), async {
        while fixture.host.reports.lock().unwrap().len() < 3 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let sent = fixture.sent();
    assert_eq!(
        sent.iter().map(|v| v.account.as_str()).collect::<Vec<_>>(),
        ["a", "a", "a", "b"]
    );
    assert_fresh(&sent[2], &sent[3], expected_replay());
    assert_eq!(fixture.active().unwrap(), before);
    assert_eq!(
        checkpoint(&fixture, "caller", "notify-a-root"),
        old_checkpoint
    );
    fixture.assert_owner("notify-a-root", "a");
    assert_no_backup_checkpoint(&fixture);
    fixture.finish().await;
}

#[tokio::test]
async fn notify_results_wrong_caller_cannot_borrow_history_or_publish_owner() {
    let mut fixture = Fixture::with_key_continuity(
        true,
        vec![
            Step::CompleteItems("notify-a-start", vec![custom_call("done-call")]),
            Step::CompleteItems("notify-a-root", vec![cipher(), custom_call("pending-call")]),
        ],
        vec![],
        true,
    )
    .await;
    // Keep distinct synthetic caller IDs, but disable the explicit shared-key capability.
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
    seed(&fixture, false).await;
    let original = checkpoint(&fixture, "synthetic-caller", "notify-a-root");
    let request = body(
        result_group("pending-call", "second payload"),
        Some("notify-a-root"),
        false,
    );
    let response = client()
        .post(format!("http://{}/v1/responses", fixture.handle.addr))
        .bearer_auth("other-synthetic-caller")
        .json(&request)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let result = response.text().await.unwrap();
    assert!(
        !status.is_success(),
        "wrong caller unexpectedly continued: {result}"
    );
    assert_eq!(
        fixture.sent().len(),
        2,
        "wrong caller history must be rejected before an upstream attempt"
    );
    assert_eq!(
        checkpoint(&fixture, "synthetic-caller", "notify-a-root"),
        original
    );
    assert_no_backup_checkpoint(&fixture);
    let db = Connection::open(fixture.path()).unwrap();
    for table in [
        "native_rebuild_checkpoints_v1",
        "native_rebuild_history_v1",
        "native_rebuild_active_v1",
    ] {
        let count: i64 = db
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE caller='other-synthetic-caller'"),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "wrong caller must not publish {table}");
    }
    drop(db);
    fixture.finish().await;
}

#[tokio::test]
async fn notify_results_invalid_groups_never_unlock_backup() {
    for invalid in [
        "orphan",
        "noncontiguous",
        "wrong-kind",
        "nested-signature",
        "nested-encrypted",
    ] {
        let mut input = result_group("pending-call", "second payload");
        match invalid {
            "orphan" => {
                for item in &mut input {
                    item["call_id"] = json!("missing-call");
                }
            }
            "noncontiguous" => {
                input.insert(1, message("assistant", "intervening semantic event"));
            }
            "wrong-kind" => {
                input[1]["type"] = json!("function_call_output");
            }
            "nested-signature" => {
                input[0]["output"][0]["signature"] = json!("synthetic signature");
            }
            "nested-encrypted" => {
                input[0]["output"][0] =
                    json!({"type":"encrypted_content","encrypted_content":"synthetic cipher"});
            }
            _ => unreachable!(),
        }
        let fixture = Fixture::new(
            true,
            vec![
                Step::CompleteItems("notify-a-start", vec![custom_call("done-call")]),
                Step::CompleteItems("notify-a-root", vec![cipher(), custom_call("pending-call")]),
                Step::HttpRateLimit,
            ],
            vec![],
        )
        .await;
        seed(&fixture, false).await;
        let before = fixture.active().unwrap();
        let old_checkpoint = checkpoint(&fixture, "caller", "notify-a-root");
        let response = fixture
            .post(&body(input, Some("notify-a-root"), false), None)
            .await;
        let status = response.status();
        let text = response.text().await.unwrap();
        assert!(
            !status.is_success(),
            "{invalid} unexpectedly rebuilt: {text}"
        );
        assert!(
            fixture.sent().iter().all(|v| v.account == "a"),
            "{invalid} reached B"
        );
        assert_eq!(fixture.active().unwrap(), before);
        assert_eq!(
            checkpoint(&fixture, "caller", "notify-a-root"),
            old_checkpoint
        );
        assert_no_backup_checkpoint(&fixture);
        fixture.finish().await;
    }
}
