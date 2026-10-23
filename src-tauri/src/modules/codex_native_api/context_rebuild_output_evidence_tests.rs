//! Output presentation repair must not manufacture durable replay evidence.
use super::*;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug)]
enum Delivery {
    Json,
    Sse,
    WsHttp,
    WsNative,
}
impl Delivery {
    fn streaming(self) -> bool {
        !matches!(self, Self::Json)
    }
    fn websocket(self) -> bool {
        matches!(self, Self::WsHttp | Self::WsNative)
    }
    fn configure(self, fixture: &Fixture) {
        fixture
            .host
            .passthrough
            .store(matches!(self, Self::WsNative), Ordering::SeqCst);
    }
}
const DELIVERIES: [Delivery; 4] = [
    Delivery::Json,
    Delivery::Sse,
    Delivery::WsHttp,
    Delivery::WsNative,
];

#[derive(Clone, Copy, Debug)]
enum Repair {
    Empty,
    MissingOutput,
    Unindexed,
    ConflictingTool,
    MissingId,
    None,
}

fn observed_output() -> Vec<Value> {
    let mut answer = message("assistant", "observed answer");
    answer["id"] = json!("observed-message");
    vec![
        answer,
        json!({"type":"reasoning","id":"observed-reasoning","encrypted_content":"observed-a-cipher","summary":[]}),
        json!({"type":"custom_tool_call","id":"observed-tool","call_id":"observed-call","name":"shell","input":"pwd","status":"completed","extension":{"keep":true}}),
    ]
}

fn raw_events(id: &str, repair: Repair) -> Vec<Value> {
    let output = observed_output();
    let mut terminal = json!({"id":id,"status":"completed","output":output,"extension":true,"usage":{"input_tokens":4,"output_tokens":3}});
    match repair {
        Repair::Empty | Repair::Unindexed => terminal["output"] = json!([]),
        Repair::MissingOutput => {
            terminal.as_object_mut().unwrap().remove("output");
        }
        Repair::ConflictingTool => {
            terminal["output"][2] = json!({"type":"function_call","id":"observed-tool","call_id":"observed-call","name":"shell","arguments":"{\"different\":true}","status":"completed"})
        }
        Repair::MissingId => {
            terminal["output"][0].as_object_mut().unwrap().remove("id");
        }
        Repair::None => {}
    }
    let mut events = vec![
        json!({"type":"response.created","response":{"id":id,"status":"in_progress","output":[]}}),
    ];
    for (index, item) in output.iter().enumerate() {
        let mut done = json!({"type":"response.output_item.done","output_index":index,"item":item});
        if matches!(repair, Repair::Unindexed) {
            done.as_object_mut().unwrap().remove("output_index");
        }
        events.push(done);
    }
    events.push(json!({"type":"response.completed","response":terminal}));
    events
}

fn delta() -> Vec<Value> {
    vec![
        json!({"type":"custom_tool_call_output","call_id":"observed-call","output":"delivered result"}),
        message("user", "continue"),
    ]
}

async fn socket_terminal(socket: &mut Socket) -> Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match socket
                .next()
                .await
                .expect("socket ended before terminal")
                .unwrap()
            {
                Message::Text(text) => {
                    let event: Value = serde_json::from_str(&text).unwrap();
                    if matches!(
                        event["type"].as_str(),
                        Some(
                            "response.completed"
                                | "response.done"
                                | "response.failed"
                                | "response.incomplete"
                                | "error"
                        )
                    ) {
                        return event;
                    }
                }
                Message::Ping(value) => socket.send(Message::Pong(value)).await.unwrap(),
                _ => {}
            }
        }
    })
    .await
    .unwrap()
}

async fn completed_turn(fixture: &Fixture, delivery: Delivery, request: &Value) -> Value {
    delivery.configure(fixture);
    if delivery.websocket() {
        let mut socket = fixture.socket().await;
        ws_send(&mut socket, request.clone()).await;
        let event = socket_terminal(&mut socket).await;
        assert_eq!(event["type"], "response.completed", "{delivery:?}: {event}");
        socket.close(None).await.unwrap();
        return event["response"].clone();
    }
    let response = fixture.post(request, None).await;
    assert_eq!(response.status(), 200, "{delivery:?}");
    if matches!(delivery, Delivery::Json) {
        return response.json().await.unwrap();
    }
    let text = response.text().await.unwrap();
    text.split("\n\n")
        .filter_map(|frame| {
            let data = frame
                .lines()
                .filter_map(|line| line.strip_prefix("data: "))
                .collect::<Vec<_>>()
                .join("\n");
            serde_json::from_str::<Value>(&data).ok()
        })
        .find(|event| event["type"] == "response.completed")
        .unwrap_or_else(|| panic!("no completed SSE response: {text}"))["response"]
        .clone()
}

async fn rejected_turn(fixture: &Fixture, delivery: Delivery, request: &Value) -> Value {
    delivery.configure(fixture);
    if delivery.websocket() {
        let mut socket = fixture.socket().await;
        ws_send(&mut socket, request.clone()).await;
        let event = socket_terminal(&mut socket).await;
        assert_eq!(event["type"], "error", "{delivery:?}: {event}");
        return event;
    }
    let response = fixture.post(request, None).await;
    assert!(
        !response.status().is_success(),
        "{delivery:?}: rejected continuation reached upstream"
    );
    response.json().await.unwrap()
}

async fn restart(fixture: &mut Fixture) {
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
}

#[derive(Debug, PartialEq, Eq)]
struct ReplayState {
    checkpoints: i64,
    portable: i64,
    blobs: i64,
    active: Option<(String, String, String, i64)>,
}
fn replay_state(fixture: &Fixture) -> ReplayState {
    let connection = Connection::open(fixture.path()).unwrap();
    let count = |table: &str| {
        connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    };
    ReplayState {
        checkpoints: count("native_rebuild_checkpoints_v1"),
        portable: count("native_rebuild_portable_v1"),
        blobs: count("native_rebuild_blobs_v1"),
        active: fixture.active(),
    }
}
fn assert_no_replay_state(fixture: &Fixture) {
    assert_eq!(
        replay_state(fixture),
        ReplayState {
            checkpoints: 0,
            portable: 0,
            blobs: 0,
            active: None
        }
    );
}
fn assert_cipher_owner(fixture: &Fixture, cipher: &str, account: &str) {
    let digest = format!("{:x}", Sha256::digest(cipher.as_bytes()));
    let owner: (String, String) = Connection::open(fixture.path()).unwrap().query_row(
        "SELECT account,owner FROM native_opaque_v1 WHERE caller='caller' AND kind='encrypted' AND digest=?1",
        [digest], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
    assert_eq!(owner, (account.into(), format!("owner-{account}")));
}

#[tokio::test]
async fn output_evidence_repaired_root_cannot_become_portable_after_restart_or_reconnect() {
    for delivery in DELIVERIES {
        for repair in [
            Repair::Unindexed,
            Repair::ConflictingTool,
            Repair::MissingId,
        ] {
            let mut fixture = Fixture::new(
                true,
                vec![
                    Step::RawEvents(raw_events("a-repaired", repair)),
                    Step::Complete("a-continued", None),
                ],
                vec![Step::Complete("unexpected-b", None)],
            )
            .await;
            let root = body(vec![message("user", "first")], None, delivery.streaming());
            let response = completed_turn(&fixture, delivery, &root).await;
            assert_eq!(
                response["output"],
                json!(observed_output()),
                "{delivery:?} {repair:?}"
            );
            assert_eq!(response["extension"], true);
            assert_no_replay_state(&fixture);
            fixture.assert_owner("a-repaired", "a");
            assert_cipher_owner(&fixture, "observed-a-cipher", "a");

            restart(&mut fixture).await;
            assert_no_replay_state(&fixture);
            fixture.host.removed_a.store(true, Ordering::SeqCst);
            let continuation = body(delta(), Some("a-repaired"), delivery.streaming());
            let error = rejected_turn(&fixture, delivery, &continuation).await;
            assert_eq!(
                error["error"]["code"], "account_binding_unavailable",
                "{delivery:?} {repair:?}: {error}"
            );
            assert_eq!(
                fixture.sent().len(),
                1,
                "repaired evidence must never authorize B"
            );
            assert_no_replay_state(&fixture);
            fixture.assert_owner("a-repaired", "a");
            assert_cipher_owner(&fixture, "observed-a-cipher", "a");

            fixture.host.removed_a.store(false, Ordering::SeqCst);
            assert_eq!(
                completed_turn(&fixture, delivery, &continuation).await["id"],
                "a-continued"
            );
            let sent = fixture.sent();
            assert_eq!(
                sent.iter()
                    .map(|sent| sent.account.as_str())
                    .collect::<Vec<_>>(),
                vec!["a", "a"]
            );
            assert_eq!(identity(&sent[0]), identity(&sent[1]));
            assert_eq!(sent[1].body["previous_response_id"], "a-repaired");
            assert_eq!(sent[1].body["input"], continuation["input"]);
            fixture.assert_owner("a-continued", "a");
            assert_cipher_owner(&fixture, "observed-a-cipher", "a");
            fixture.finish().await;
        }
    }
}

#[tokio::test]
async fn output_evidence_repaired_candidate_cannot_activate_a_new_generation() {
    for delivery in DELIVERIES {
        for repair in [
            Repair::Unindexed,
            Repair::ConflictingTool,
            Repair::MissingId,
        ] {
            let fixture = Fixture::new(
                true,
                vec![Step::Complete("a-root", None)],
                vec![Step::RawEvents(raw_events("b-repaired", repair))],
            )
            .await;
            let root = body(vec![message("user", "first")], None, delivery.streaming());
            completed_turn(&fixture, delivery, &root).await;
            let before = replay_state(&fixture);
            assert_eq!(before.active.as_ref().unwrap().0, "a");
            fixture.host.removed_a.store(true, Ordering::SeqCst);
            let response = completed_turn(
                &fixture,
                delivery,
                &body(
                    vec![message("user", "second")],
                    Some("a-root"),
                    delivery.streaming(),
                ),
            )
            .await;
            assert_eq!(
                response["output"],
                json!(observed_output()),
                "{delivery:?} {repair:?}"
            );
            assert_eq!(
                replay_state(&fixture),
                before,
                "candidate repair cannot persist replay evidence"
            );
            fixture.assert_owner("a-root", "a");
            fixture.assert_owner("b-repaired", "b");
            assert_cipher_owner(&fixture, "observed-a-cipher", "b");
            assert_eq!(
                fixture
                    .sent()
                    .iter()
                    .map(|sent| sent.account.as_str())
                    .collect::<Vec<_>>(),
                vec!["a", "b"]
            );
            fixture.finish().await;
        }
    }
}

#[tokio::test]
async fn output_evidence_complete_terminal_still_rebuilds_after_restart_with_fresh_identity() {
    for delivery in DELIVERIES {
        // A sparse terminal backed by complete indexed upstream done items is
        // equivalent to the same explicit terminal output, without public repair.
        for repair in [Repair::None, Repair::Empty, Repair::MissingOutput] {
            let mut fixture = Fixture::new(
                true,
                vec![Step::RawEvents(raw_events("a-complete", repair))],
                vec![Step::Complete("b-complete", None)],
            )
            .await;
            let root = body(vec![message("user", "first")], None, delivery.streaming());
            assert_eq!(
                completed_turn(&fixture, delivery, &root).await["output"],
                json!(observed_output())
            );
            let before = replay_state(&fixture);
            assert_eq!(before.checkpoints, 1);
            assert_eq!(before.portable, 1);
            assert_eq!(before.active.as_ref().unwrap().0, "a");
            restart(&mut fixture).await;
            assert_eq!(replay_state(&fixture), before);
            fixture.host.removed_a.store(true, Ordering::SeqCst);
            let continuation = body(delta(), Some("a-complete"), delivery.streaming());
            assert_eq!(
                completed_turn(&fixture, delivery, &continuation).await["id"],
                "b-complete"
            );
            let sent = fixture.sent();
            assert_eq!(
                sent.iter()
                    .map(|sent| sent.account.as_str())
                    .collect::<Vec<_>>(),
                vec!["a", "b"]
            );
            let mut expected = root["input"].as_array().unwrap().clone();
            expected.extend(observed_output());
            expected.extend(delta());
            assert_fresh(&sent[0], &sent[1], json!(expected));
            fixture.assert_owner("a-complete", "a");
            fixture.assert_owner("b-complete", "b");
            assert_cipher_owner(&fixture, "observed-a-cipher", "a");
            assert_eq!(fixture.active().unwrap().0, "b");
            fixture.finish().await;
        }
    }
}

#[tokio::test]
async fn output_evidence_complete_done_can_activate_a_sparse_cross_account_candidate() {
    for delivery in DELIVERIES {
        let fixture = Fixture::new(
            true,
            vec![Step::Complete("a-root", None)],
            vec![Step::RawEvents(raw_events("b-sparse", Repair::Empty))],
        )
        .await;
        completed_turn(
            &fixture,
            delivery,
            &body(vec![message("user", "first")], None, delivery.streaming()),
        )
        .await;
        assert_eq!(fixture.active().unwrap().0, "a");
        fixture.host.removed_a.store(true, Ordering::SeqCst);
        let response = completed_turn(
            &fixture,
            delivery,
            &body(
                vec![message("user", "second")],
                Some("a-root"),
                delivery.streaming(),
            ),
        )
        .await;
        assert_eq!(response["output"], json!(observed_output()));
        let after = replay_state(&fixture);
        assert_eq!(after.checkpoints, 2);
        assert_eq!(after.portable, 2);
        assert_eq!(after.active.as_ref().unwrap().0, "b");
        fixture.assert_owner("a-root", "a");
        fixture.assert_owner("b-sparse", "b");
        assert_cipher_owner(&fixture, "observed-a-cipher", "b");
        fixture.finish().await;
    }
}

#[tokio::test]
async fn output_evidence_repair_does_not_adopt_unknown_input_or_invent_a_missing_checkpoint() {
    for delivery in DELIVERIES {
        for repair in [Repair::Empty, Repair::Unindexed] {
            let mut fixture = Fixture::new(
                true,
                vec![Step::RawEvents(raw_events("a-recovered", repair))],
                vec![Step::Complete("unexpected-b", None)],
            )
            .await;
            let legacy = json!({"type":"reasoning","encrypted_content":"authorized-legacy-cipher"});
            let request = body(vec![legacy], None, delivery.streaming());
            assert_eq!(
                completed_turn(&fixture, delivery, &request).await["output"],
                json!(observed_output())
            );
            let complete = matches!(repair, Repair::Empty);
            if complete {
                // Complete upstream done evidence admits the original full input;
                // unindexed presentation repair still cannot claim its provenance.
                assert_eq!(replay_state(&fixture).checkpoints, 1);
                assert_eq!(replay_state(&fixture).portable, 1);
            } else {
                assert_no_replay_state(&fixture);
            }
            let before = replay_state(&fixture);
            let sent = fixture.sent();
            assert_eq!(sent.len(), 1);
            assert_eq!(sent[0].account, "a");
            assert_eq!(sent[0].body["input"], request["input"]);
            assert!(sent[0].body.get("previous_response_id").is_none());
            assert_ne!(identity(&sent[0]).1, json!("client-thread"));
            fixture.assert_owner("a-recovered", "a");
            let digest = format!(
                "{:x}",
                <sha2::Sha256 as sha2::Digest>::digest(b"authorized-legacy-cipher")
            );
            let count: i64 = Connection::open(fixture.path())
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM native_opaque_v1 WHERE kind='encrypted' AND digest=?1",
                    [digest],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, i64::from(complete));
            assert_cipher_owner(&fixture, "observed-a-cipher", "a");
            restart(&mut fixture).await;
            let unauthorized = body(
                vec![message("user", "delta without its checkpoint")],
                Some("never-authorized-response"),
                delivery.streaming(),
            );
            let error = rejected_turn(&fixture, delivery, &unauthorized).await;
            // The unknown response is rejected before inspecting its unknown cipher.
            assert_eq!(
                error["error"]["code"], "previous_response_not_found",
                "{delivery:?}: {error}"
            );
            assert_eq!(
                fixture.sent().len(),
                1,
                "a missing checkpoint must not authorize an incomplete replay"
            );
            assert_eq!(replay_state(&fixture), before);
            fixture.finish().await;
        }
    }
}
