#[tokio::test]
async fn websocket_http_natural_incomplete_replays_partial_history_without_promoting_success() {
    let requests = Arc::new(Mutex::new(Vec::<(HeaderMap, Value)>::new()));
    let capture = requests.clone();
    let app = Router::new().route("/v1/responses", post(move |headers: HeaderMap, Json(body): Json<Value>| {
        let capture = capture.clone();
        async move {
            let n = { let mut requests = capture.lock().unwrap(); requests.push((headers, body)); requests.len() };
            let event = if n == 1 {
                json!({"type":"response.incomplete","response":{"id":"natural-partial","status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"output":[{"type":"reasoning","id":"partial-reasoning","encrypted_content":"natural-cipher"},{"type":"message","role":"assistant","id":"partial-message","content":[{"type":"output_text","text":"partial result"}]}],"usage":{"input_tokens":3,"output_tokens":1}}})
            } else {
                json!({"type":"response.completed","response":{"id":"natural-completed","status":"completed","output":[],"usage":{"input_tokens":5,"output_tokens":2}}})
            };
            Response::builder().header("content-type", "text/event-stream").body(Body::from(format!("event: {}\ndata: {event}\n\n", event["type"].as_str().unwrap()))).unwrap()
        }
    }));
    let (mut handle, host, task, dir) =
        fixture(app, WireApi::Responses, false, native::Options::default()).await;
    let mut ws = socket(&handle).await;
    send(&mut ws, begin_interrupt_body()).await;
    let partial = terminal(&mut ws).await;
    assert_eq!(partial["type"], "response.incomplete");
    reports(&host, 1).await;
    {
        let connection = rusqlite::Connection::open(dir.path().join("identity.sqlite")).unwrap();
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM native_opaque_v1 WHERE kind='response'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }
    send(&mut ws, json!({"type":"response.append","model":"other","input":[{"role":"user","content":"continue partial"}]})).await;
    let completed = terminal(&mut ws).await;
    assert_eq!(completed["type"], "response.completed");
    reports(&host, 2).await;
    {
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].0["session-id"], requests[1].0["session-id"]);
        assert_eq!(requests[0].0["thread-id"], requests[1].0["thread-id"]);
        assert!(requests[1].1.get("previous_response_id").is_none());
        assert_eq!(requests[1].1["input"][0]["content"], "original");
        assert_eq!(
            requests[1].1["input"][1]["encrypted_content"],
            "natural-cipher"
        );
        assert_eq!(
            requests[1].1["input"][2]["content"][0]["text"],
            "partial result"
        );
        assert_eq!(requests[1].1["input"][3]["content"], "continue partial");
    }
    {
        let reports = host.reports.lock().unwrap();
        assert_eq!(reports.len(), 2);
        assert!(!reports[0].success);
        assert_eq!(reports[0].error.as_deref(), Some("response.incomplete"));
        assert_eq!(reports[0].usage.as_ref().unwrap()["output_tokens"], 1);
        assert!(reports[1].success);
        assert!(reports.iter().all(|report| report.account_id == "a"));
    }
    ws.close(None).await.unwrap();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn websocket_http_automatic_history_natural_incomplete_replays_only_connection_local_history()
{
    let count = Arc::new(AtomicUsize::new(0));
    let calls = count.clone();
    let app = Router::new().route("/v1/responses", post(move || { let calls = calls.clone(); async move {
        calls.fetch_add(1, Ordering::SeqCst);
        let event = json!({"type":"response.incomplete","response":{"id":"failed-recovery-partial","status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"output":[{"type":"reasoning","encrypted_content":"recovery-partial-cipher"}],"usage":{"input_tokens":3,"output_tokens":1}}});
        Response::builder().header("content-type", "text/event-stream").body(Body::from(format!("event: response.incomplete\ndata: {event}\n\n"))).unwrap()
    }}));
    let (mut handle, host, task, dir) =
        fixture(app, WireApi::Responses, false, native::Options::default()).await;
    let path = dir.path().join("identity.sqlite");

    let mut ws = socket(&handle).await;
    send(&mut ws, json!({"type":"response.create","model":"m","client_metadata":{"session_id":"recovery-thread","thread_id":"recovery-thread"},"input":[{"type":"reasoning","encrypted_content":"legacy-cipher"}]})).await;
    assert_eq!(terminal(&mut ws).await["type"], "response.incomplete");
    reports(&host, 1).await;
    send(
        &mut ws,
        json!({"type":"response.append","input":[{"role":"user","content":"continue"}]}),
    )
    .await;
    assert_eq!(terminal(&mut ws).await["type"], "response.incomplete");
    assert_eq!(count.load(Ordering::SeqCst), 2);
    let connection = rusqlite::Connection::open(&path).unwrap();
    for table in ["native_rebuild_active_v1", "native_rebuild_checkpoints_v1"] {
        let rows: i64 = connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(rows, 0);
    }
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}
