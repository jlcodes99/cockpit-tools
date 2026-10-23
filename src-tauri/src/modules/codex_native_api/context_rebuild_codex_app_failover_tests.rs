//! Loopback-only OAuth wire coverage for Codex host records during a simulated 429 retry.
//! Synthetic ciphertext tests preservation, not acceptance by a real upstream.
use super::*;
use crate::modules::codex_native_api::codex_cpa_wire::synthetic_gpt_signature;
use std::sync::atomic::AtomicUsize;

#[derive(Clone, Copy, Debug)]
enum LimitDelivery {
    Http,
    Sse,
}

impl LimitDelivery {
    fn streaming(self) -> bool {
        matches!(self, Self::Sse)
    }
}

struct LimitedUpstream {
    upstream: Arc<Upstream>,
    a_requests: AtomicUsize,
    delivery: LimitDelivery,
}

async fn limited_http(
    State(state): State<Arc<LimitedUpstream>>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    let account = headers["authorization"]
        .to_str()
        .unwrap()
        .trim_start_matches("Bearer synthetic-")
        .to_owned();
    // A first establishes a real checkpoint in this fixture. Its next attempt
    // exhausts its quota; any B attempt must be made by the actual retry code.
    if account == "a" && state.a_requests.fetch_add(1, Ordering::SeqCst) == 1 {
        state.upstream.sent.lock().unwrap().push(Sent {
            account,
            headers,
            body: request,
        });
        let error = json!({
            "type":"rate_limit_error",
            "code":"usage_limit_reached",
            "message":"Synthetic account A five-hour quota exhausted"
        });
        return match state.delivery {
            LimitDelivery::Http => {
                (StatusCode::TOO_MANY_REQUESTS, Json(json!({"error":error}))).into_response()
            }
            LimitDelivery::Sse => sse(vec![json!({
                "type":"error", "status_code":429, "error":error
            })]),
        };
    }
    upstream_http(State(state.upstream.clone()), headers, Json(request)).await
}

async fn limited_fixture(delivery: LimitDelivery, b: Vec<Step>) -> Fixture {
    let upstream = Arc::new(Upstream {
        steps: Mutex::new([
            vec![Step::CompleteItems("host-a-root", root_output())].into(),
            b.into(),
        ]),
        sent: Mutex::new(vec![]),
        ws_connections: Mutex::new(vec![]),
    });
    let app = Router::new()
        .route("/a/responses", post(limited_http))
        .route("/b/responses", post(limited_http))
        .with_state(Arc::new(LimitedUpstream {
            upstream: upstream.clone(),
            a_requests: AtomicUsize::new(0),
            delivery,
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let host = Arc::new(TestHost {
        base: format!("http://{address}"),
        enabled: AtomicBool::new(true),
        // Exercise the actual Codex OAuth path: HTTP previous expansion,
        // forced upstream SSE and codex_wire::calls::normalize all run here.
        oauth: AtomicBool::new(true),
        passthrough: AtomicBool::new(false),
        removed_a: AtomicBool::new(false),
        selections: Mutex::new(vec![]),
        failures: Mutex::new(vec![]),
        reports: Mutex::new(vec![]),
    });
    let root = tempfile::tempdir().unwrap();
    let handle = start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        root.path().join("identity.sqlite"),
        Options::default(),
    )
    .await
    .unwrap();
    Fixture {
        handle,
        host,
        upstream,
        task,
        root,
    }
}

fn host_record(name: &str) -> Value {
    json!({
        "type":"function_call_output",
        "id":format!("synthetic-host-{name}"),
        "namespace":"codex_app",
        "name":name,
        "output":"Synthetic host-origin task handoff; not a model tool result",
        "internal_chat_message_metadata_passthrough":{"source":"fixture","keep":true},
        "extension":{"preserve":[1,"two",null]}
    })
}

fn root_input() -> Vec<Value> {
    // Use canonical item IDs and a syntactically valid synthetic envelope so
    // CPA wire validation does not obscure the host-history preservation check.
    // This does not represent real encrypted content or test decryption.
    vec![
        message("user", "Synthetic task handoff history"),
        json!({"type":"function_call","id":"fc_fn-item","namespace":"functions","name":"lookup","call_id":"fn-call","arguments":"{\"key\":\"fixture\"}","status":"completed","extension":7}),
        // These envelopes sit between genuine calls and results deliberately:
        // wire normalization must not consume the pending genuine call IDs.
        host_record("create_thread"),
        json!({"type":"function_call_output","id":"fn-result","call_id":"fn-call","output":"Synthetic function result","extension":8}),
        json!({"type":"custom_tool_call","id":"ctc_custom-item","name":"synthetic_tool","call_id":"custom-call","input":"fixture-only-input","status":"completed","extension":9}),
        host_record("send_message_to_thread"),
        json!({"type":"custom_tool_call_output","id":"ctco_custom-result","call_id":"custom-call","output":"Synthetic custom result","extension":10}),
        json!({"type":"reasoning","id":"rs_inherited-reasoning","summary":[],"encrypted_content":synthetic_gpt_signature(53),"status":"completed"}),
        message("user", "First synthetic turn"),
    ]
}

fn root_output() -> Vec<Value> {
    vec![
        message("assistant", "Synthetic account A answer"),
        json!({"type":"reasoning","id":"rs_a-reasoning","summary":[],"encrypted_content":synthetic_gpt_signature(54),"status":"completed"}),
    ]
}

fn continuation(full: bool, streaming: bool) -> (Value, Value) {
    let mut expected = root_input();
    expected.extend(root_output());
    expected.push(message("user", "Continue without losing any history"));
    let input = if full {
        expected.clone()
    } else {
        vec![message("user", "Continue without losing any history")]
    };
    (
        body(input, (!full).then_some("host-a-root"), streaming),
        json!(expected),
    )
}

async fn complete_turn(fixture: &Fixture, request: &Value) -> Value {
    let response = fixture.post(request, None).await;
    let status = response.status();
    let text = response.text().await.unwrap();
    assert_eq!(status, 200, "{text}");
    if request["stream"] != true {
        return serde_json::from_str(&text).unwrap();
    }
    text.split("\n\n")
        .filter_map(|frame| frame.strip_prefix("data: "))
        .filter_map(|data| serde_json::from_str::<Value>(data).ok())
        .find(|event| event["type"] == "response.completed")
        .unwrap_or_else(|| panic!("Missing completed SSE response: {text}"))["response"]
        .clone()
}

fn portable_count(fixture: &Fixture, account: &str) -> i64 {
    Connection::open(fixture.path())
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM native_rebuild_portable_v1 WHERE caller='caller' AND account=?1",
            [account],
            |row| row.get(0),
        )
        .unwrap()
}

fn assert_no_host_call_ids(input: &Value) {
    let records = input
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["namespace"] == "codex_app")
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 2);
    for record in records {
        assert!(record.get("call_id").is_none(), "Host record was rewritten");
        assert_eq!(record, &host_record(record["name"].as_str().unwrap()));
    }
}

fn assert_oauth_wire(sent: &Sent) {
    // The nonstreaming client case especially proves the OAuth normalizer ran;
    // toggling a test flag without reaching the real wire path cannot pass.
    assert_eq!(sent.body["stream"], true);
    assert_eq!(sent.body["store"], false);
    assert!(sent.body["include"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == "reasoning.encrypted_content"));
}

async fn establish_root(fixture: &Fixture, delivery: LimitDelivery) {
    let root = body(root_input(), None, delivery.streaming());
    let unchanged = root.clone();
    assert_eq!(complete_turn(fixture, &root).await["id"], "host-a-root");
    assert_eq!(fixture.sent()[0].body["input"], root["input"]);
    assert_no_host_call_ids(&fixture.sent()[0].body["input"]);
    assert_oauth_wire(&fixture.sent()[0]);
    assert_eq!(portable_count(fixture, "a"), 1);
    assert_eq!(portable_count(fixture, "b"), 0);
    assert_eq!(root, unchanged);
}

#[tokio::test]
async fn codex_app_host_records_oauth_429_replays_complete_and_cached_history_exactly() {
    for delivery in [LimitDelivery::Http, LimitDelivery::Sse] {
        for full in [true, false] {
            let fixture =
                limited_fixture(delivery, vec![Step::Complete("host-b-accepted", None)]).await;
            establish_root(&fixture, delivery).await;
            let (request, expected) = continuation(full, delivery.streaming());
            let unchanged = request.clone();
            assert_eq!(
                complete_turn(&fixture, &request).await["id"],
                "host-b-accepted",
                "{delivery:?}, full={full}"
            );
            let sent = fixture.sent();
            assert_eq!(
                sent.iter()
                    .map(|sent| sent.account.as_str())
                    .collect::<Vec<_>>(),
                vec!["a", "a", "b"],
                "Quota failure must actually try A then B, not skip A"
            );
            assert!(fixture
                .host
                .failures
                .lock()
                .unwrap()
                .contains(&("a".into(), 429)));
            // OAuth HTTP expands cached previous history before even the A
            // attempt; both A and B must see the same complete original items.
            assert_eq!(sent[1].body["input"], expected);
            assert!(sent[1].body.get("previous_response_id").is_none());
            assert_fresh(&sent[1], &sent[2], expected.clone());
            assert_oauth_wire(&sent[1]);
            assert_oauth_wire(&sent[2]);
            assert_no_host_call_ids(&sent[2].body["input"]);
            assert_eq!(request, unchanged);
            assert_eq!(fixture.active().unwrap().0, "b");
            assert_eq!(portable_count(&fixture, "a"), 1);
            assert_eq!(portable_count(&fixture, "b"), 1);
            fixture.assert_owner("host-a-root", "a");
            fixture.assert_owner("host-b-accepted", "b");
            fixture.finish().await;
        }
    }
}

#[tokio::test]
async fn codex_app_host_records_oauth_failed_candidate_never_commits_portable_b() {
    for delivery in [LimitDelivery::Http, LimitDelivery::Sse] {
        for full in [true, false] {
            let fixture = limited_fixture(delivery, vec![Step::HttpOverload]).await;
            establish_root(&fixture, delivery).await;
            let before = fixture.active().unwrap();
            let (request, expected) = continuation(full, delivery.streaming());
            let unchanged = request.clone();
            let response = fixture.post(&request, None).await;
            assert_eq!(response.status(), 503);
            response.bytes().await.unwrap();
            let sent = fixture.sent();
            assert_eq!(
                sent.iter()
                    .map(|sent| sent.account.as_str())
                    .collect::<Vec<_>>(),
                vec!["a", "a", "b"]
            );
            assert_eq!(sent[1].body["input"], expected);
            assert_fresh(&sent[1], &sent[2], expected);
            assert_oauth_wire(&sent[1]);
            assert_oauth_wire(&sent[2]);
            assert_no_host_call_ids(&sent[2].body["input"]);
            assert_eq!(fixture.active().unwrap(), before);
            assert_eq!(portable_count(&fixture, "a"), 1);
            assert_eq!(portable_count(&fixture, "b"), 0);
            let b_checkpoints: i64 = Connection::open(fixture.path())
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM native_rebuild_checkpoints_v1 WHERE account='b'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(b_checkpoints, 0);
            assert_eq!(request, unchanged);
            fixture.assert_owner("host-a-root", "a");
            fixture.finish().await;
        }
    }
}

#[tokio::test]
async fn codex_app_host_record_oauth_exception_does_not_authorize_missing_real_tool_history() {
    let cases = [
        vec![host_record("unknown_host_operation")],
        vec![
            json!({"type":"function_call_output","name":"lookup","output":"Missing genuine function call"}),
        ],
        vec![json!({"type":"custom_tool_call_output","output":"Missing genuine custom call"})],
        vec![
            json!({"type":"function_call","namespace":"functions","name":"lookup","call_id":"unresolved-call","arguments":"{}"}),
        ],
        vec![
            json!({"type":"function_call","namespace":"codex_app","name":"send_message_to_thread","call_id":"real-host-call","arguments":"{}"}),
        ],
    ];
    for delivery in [LimitDelivery::Http, LimitDelivery::Sse] {
        for (index, invalid) in cases.iter().enumerate() {
            let fixture = limited_fixture(delivery, vec![]).await;
            establish_root(&fixture, delivery).await;
            let before = fixture.active().unwrap();
            // An explicit previous response preserves real A custody even when
            // this additional history fails strict tool-pair validation.
            let mut input = root_input();
            input.extend(root_output());
            input.extend(invalid.clone());
            input.push(message("user", "Do not repair missing calls by guessing"));
            let request = body(input, Some("host-a-root"), delivery.streaming());
            let unchanged = request.clone();
            let response = fixture.post(&request, None).await;
            assert!(
                !response.status().is_success(),
                "{delivery:?}, case={index}"
            );
            response.bytes().await.unwrap();
            let sent = fixture.sent();
            assert!(
                sent.iter().all(|sent| sent.account == "a"),
                "{delivery:?}, case={index}"
            );
            assert!(
                sent.len() <= 2,
                "Invalid history must not be retried across accounts"
            );
            assert_eq!(portable_count(&fixture, "b"), 0);
            assert_eq!(fixture.active().unwrap(), before);
            assert_eq!(request, unchanged);
            fixture.assert_owner("host-a-root", "a");
            fixture.finish().await;
        }
    }
}
