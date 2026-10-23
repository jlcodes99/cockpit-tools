use axum::body::Bytes;
use futures_util::stream;
use std::sync::atomic::AtomicUsize;

struct InterruptSource {
    requests: Mutex<Vec<(HeaderMap, Value)>>,
    cancelled: Arc<AtomicUsize>,
    changed: Arc<tokio::sync::Notify>,
    finish_second: Arc<tokio::sync::Notify>,
}
struct CancelProbe {
    count: Arc<AtomicUsize>,
    changed: Arc<tokio::sync::Notify>,
}
impl Drop for CancelProbe {
    fn drop(&mut self) {
        self.count.fetch_add(1, Ordering::SeqCst);
        self.changed.notify_waiters();
    }
}
async fn interrupt_response(
    State(source): State<Arc<InterruptSource>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let n = {
        let mut requests = source.requests.lock().unwrap();
        requests.push((headers, body));
        requests.len()
    };
    let id = format!("interrupt-{n}");
    let prefix = format!(concat!(
        "event: response.created\ndata: {{\"type\":\"response.created\",\"response\":{{\"id\":\"{}\",\"status\":\"in_progress\",\"usage\":{{\"input_tokens\":3,\"output_tokens\":1}}}}}}\n\n",
        "event: response.output_item.done\ndata: {{\"type\":\"response.output_item.done\",\"output_index\":2,\"item\":{{\"id\":\"later\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[{{\"type\":\"output_text\",\"text\":\"partial\"}}]}}}}\n\n",
        "event: response.output_item.done\ndata: {{\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{{\"id\":\"earlier\",\"type\":\"reasoning\",\"encrypted_content\":\"interrupt-cipher\"}}}}\n\n"
    ), id);
    let probe = CancelProbe {
        count: source.cancelled.clone(),
        changed: source.changed.clone(),
    };
    let finish = source.finish_second.clone();
    let tail = stream::once(async move {
        finish.notified().await;
        Ok::<_, std::io::Error>(Bytes::from(format!("event: response.completed\ndata: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"{id}\",\"status\":\"completed\",\"output\":[],\"usage\":{{\"input_tokens\":5,\"output_tokens\":2}}}}}}\n\n")))
    });
    let bytes = stream::once(async move { Ok::<_, std::io::Error>(Bytes::from(prefix)) })
        .chain(tail)
        .map(move |item| {
            let _keep = &probe;
            item
        });
    Response::builder()
        .header("content-type", "text/event-stream")
        .body(Body::from_stream(bytes))
        .unwrap()
}
fn interrupted_source() -> (Router, Arc<InterruptSource>) {
    let source = Arc::new(InterruptSource {
        requests: Mutex::new(Vec::new()),
        cancelled: Arc::new(AtomicUsize::new(0)),
        changed: Arc::new(tokio::sync::Notify::new()),
        finish_second: Arc::new(tokio::sync::Notify::new()),
    });
    (
        Router::new()
            .route("/v1/responses", post(interrupt_response))
            .with_state(source.clone()),
        source,
    )
}
async fn next_event(socket: &mut Socket) -> Value {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match socket.next().await.unwrap().unwrap() {
                Message::Text(text) => return serde_json::from_str(&text).unwrap(),
                Message::Ping(value) => socket.send(Message::Pong(value)).await.unwrap(),
                _ => {}
            }
        }
    })
    .await
    .unwrap()
}
async fn partial_response(socket: &mut Socket) -> String {
    let created = next_event(socket).await;
    assert_eq!(created["type"], "response.created");
    for _ in 0..2 {
        assert_eq!(
            next_event(socket).await["type"],
            "response.output_item.done"
        );
    }
    created["response"]["id"].as_str().unwrap().into()
}
async fn cancelled(source: &InterruptSource, count: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let changed = source.changed.notified();
            if source.cancelled.load(Ordering::SeqCst) >= count {
                return;
            }
            changed.await;
        }
    })
    .await
    .unwrap();
}
fn begin_interrupt_body() -> Value {
    json!({"type":"response.create","model":"m","client_metadata":{"session_id":"interrupt-thread","thread_id":"interrupt-thread"},"input":[{"role":"user","content":"original"}]})
}

#[tokio::test]
async fn websocket_http_interrupt_replays_same_owner_history_and_ignores_late_interrupts() {
    let (app, source) = interrupted_source();
    let (mut handle, host, task, dir) =
        fixture(app, WireApi::Responses, false, native::Options::default()).await;
    let mut ws = socket(&handle).await;
    send(&mut ws, begin_interrupt_body()).await;
    let id = partial_response(&mut ws).await;
    send(
        &mut ws,
        json!({"type":"response.interrupt","response_id":"unrelated"}),
    )
    .await;
    assert_eq!(terminal(&mut ws).await["status"], 409);
    assert_eq!(source.cancelled.load(Ordering::SeqCst), 0);
    send(
        &mut ws,
        json!({"type":"response.interrupt","response_id":id}),
    )
    .await;
    let interrupted = terminal(&mut ws).await;
    assert_eq!(interrupted["type"], "response.incomplete");
    assert_eq!(
        interrupted["response"]["incomplete_details"]["reason"],
        "interrupted"
    );
    assert_eq!(interrupted["response"]["output"][0]["id"], "earlier");
    assert_eq!(interrupted["response"]["output"][1]["id"], "later");
    cancelled(&source, 1).await;
    reports(&host, 1).await;
    {
        let connection = rusqlite::Connection::open(dir.path().join("identity.sqlite")).unwrap();
        let completed: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM native_opaque_v1 WHERE kind='response'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            completed, 0,
            "local interrupted IDs are not durable completed responses"
        );
    }
    send(&mut ws, json!({"type":"response.append","model":"other","input":[{"role":"user","content":"continue"}]})).await;
    let next_id = partial_response(&mut ws).await;
    send(
        &mut ws,
        json!({"type":"response.interrupt","response_id":id}),
    )
    .await;
    send(
        &mut ws,
        json!({"type":"response.interrupt","response_id":id}),
    )
    .await;
    source.finish_second.notify_one();
    let completed = terminal(&mut ws).await;
    assert_eq!(completed["type"], "response.completed");
    assert_eq!(completed["response"]["id"], next_id);
    reports(&host, 2).await;
    {
        let requests = source.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].0["session-id"], requests[1].0["session-id"]);
        assert_eq!(requests[0].0["thread-id"], requests[1].0["thread-id"]);
        assert!(requests[1].1.get("previous_response_id").is_none());
        assert_eq!(requests[1].1["input"][0]["content"], "original");
        assert_eq!(
            requests[1].1["input"][1]["encrypted_content"],
            "interrupt-cipher"
        );
        assert_eq!(requests[1].1["input"][3]["content"], "continue");
    }
    {
        let reports = host.reports.lock().unwrap();
        assert_eq!(reports.len(), 2);
        assert_eq!(reports[0].status, 499);
        assert!(!reports[0].success);
        assert_eq!(reports[0].usage.as_ref().unwrap()["input_tokens"], 3);
        assert!(reports[1].success);
        assert!(reports.iter().all(|report| report.account_id == "a"));
    }
    ws.close(None).await.unwrap();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    assert_eq!(
        host.reports.lock().unwrap().len(),
        2,
        "idle close creates no failed inference"
    );
    task.abort();
}

#[tokio::test]
async fn websocket_http_interrupted_history_cannot_cross_account_or_thread() {
    for change_thread in [false, true] {
        let (app, source) = interrupted_source();
        let (mut handle, host, task, _dir) =
            fixture(app, WireApi::Responses, false, native::Options::default()).await;
        let mut ws = socket(&handle).await;
        send(&mut ws, begin_interrupt_body()).await;
        let id = partial_response(&mut ws).await;
        send(
            &mut ws,
            json!({"type":"response.interrupt","response_id":id}),
        )
        .await;
        assert_eq!(terminal(&mut ws).await["type"], "response.incomplete");
        host.honor_preferred.store(change_thread, Ordering::SeqCst);
        let mut followup = json!({"type":"response.create","model":if change_thread{"m"}else{"other"},"previous_response_id":id,"input":[{"role":"user","content":"continue"}]});
        if change_thread {
            followup["client_metadata"] =
                json!({"session_id":"another-thread","thread_id":"another-thread"});
        }
        send(&mut ws, followup).await;
        let error = terminal(&mut ws).await;
        assert_eq!(error["type"], "error");
        assert_eq!(error["status"], 409);
        assert_eq!(source.requests.lock().unwrap().len(), 1);
        handle.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
    }
}

#[tokio::test]
async fn websocket_http_automatic_history_interrupt_preserves_cipher_without_commit_or_retry() {
    let (app, source) = interrupted_source();
    let (mut handle, host, task, dir) =
        fixture(app, WireApi::Responses, false, native::Options::default()).await;
    let path = dir.path().join("identity.sqlite");

    let mut ws = socket(&handle).await;
    let original = json!({"type":"response.create","model":"m","client_metadata":{"session_id":"recovery-thread","thread_id":"recovery-thread"},"input":[{"type":"reasoning","encrypted_content":"legacy-cipher"}]});
    send(&mut ws, original.clone()).await;
    let id = partial_response(&mut ws).await;
    send(
        &mut ws,
        json!({"type":"response.interrupt","response_id":id}),
    )
    .await;
    assert_eq!(terminal(&mut ws).await["type"], "response.incomplete");
    cancelled(&source, 1).await;
    reports(&host, 1).await;
    let connection = rusqlite::Connection::open(&path).unwrap();
    for table in ["native_rebuild_active_v1", "native_rebuild_checkpoints_v1"] {
        let count: i64 = connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "{table} is uncommitted after interruption");
    }
    {
        let requests = source.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].1["input"], original["input"]);
        assert_eq!(
            requests[0].1["previous_response_id"],
            original["previous_response_id"]
        );
    }
    assert_eq!(host.reports.lock().unwrap()[0].status, 499);
    ws.close(None).await.unwrap();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn websocket_http_active_disconnect_keeps_observed_usage_and_cancels_only_current_turn() {
    let (app, source) = interrupted_source();
    let (mut handle, host, task, _dir) =
        fixture(app, WireApi::Responses, false, native::Options::default()).await;
    let mut ws = socket(&handle).await;
    send(&mut ws, begin_interrupt_body()).await;
    partial_response(&mut ws).await;
    ws.close(None).await.unwrap();
    reports(&host, 1).await;
    cancelled(&source, 1).await;
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    let reports = host.reports.lock().unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].status, 499);
    assert_eq!(reports[0].usage.as_ref().unwrap()["input_tokens"], 3);
    assert_eq!(reports[0].usage.as_ref().unwrap()["output_tokens"], 1);
    assert!(!reports[0].success);
    task.abort();
}
