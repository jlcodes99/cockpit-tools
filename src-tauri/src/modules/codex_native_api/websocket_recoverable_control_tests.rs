#[tokio::test]
async fn websocket_bad_control_frames_keep_inference_alive_and_queued_followup_uses_completed_history(
) {
    let (app, source) = interrupted_source();
    let (mut handle, host, task, _dir) =
        fixture(app, WireApi::Responses, false, native::Options::default()).await;
    let mut ws = socket(&handle).await;
    ws.send(Message::Text("{invalid".into())).await.unwrap();
    assert_eq!(terminal(&mut ws).await["status"], 400);
    send(
        &mut ws,
        json!({"type":"response.create","model":"","input":[]}),
    )
    .await;
    assert_eq!(terminal(&mut ws).await["status"], 400);
    assert!(source.requests.lock().unwrap().is_empty());
    assert!(host.reports.lock().unwrap().is_empty());
    send(&mut ws, begin_interrupt_body()).await;
    let first_id = partial_response(&mut ws).await;
    ws.send(Message::Text("not json".into())).await.unwrap();
    assert_eq!(terminal(&mut ws).await["status"], 400);
    send(&mut ws, json!({"type":"unknown.control","extension":true})).await;
    assert_eq!(terminal(&mut ws).await["status"], 400);
    ws.send(Message::Binary(vec![1, 2, 3].into()))
        .await
        .unwrap();
    assert_eq!(terminal(&mut ws).await["status"], 400);
    assert_eq!(source.cancelled.load(Ordering::SeqCst), 0);
    assert!(host.reports.lock().unwrap().is_empty());
    // The append has no inherited fields yet. Those are resolved only after
    // the true first completion reaches the ledger and this frame is dequeued.
    send(&mut ws, json!({"type":"response.append","model":"other","input":[{"role":"user","content":"queued next"}],"extension":"queued"})).await;
    source.finish_second.notify_one();
    let first = terminal(&mut ws).await;
    assert_eq!(first["type"], "response.completed");
    assert_eq!(first["response"]["id"], first_id);
    let second_id = partial_response(&mut ws).await;
    {
        let requests = source.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].1.get("previous_response_id").is_none());
        assert_eq!(requests[1].1["extension"], "queued");
        assert_eq!(requests[1].1["input"][0]["content"], "original");
        assert_eq!(requests[1].1["input"][3]["content"], "queued next");
    }
    source.finish_second.notify_one();
    assert_eq!(terminal(&mut ws).await["response"]["id"], second_id);
    reports(&host, 2).await;
    send(
        &mut ws,
        json!({"type":"response.create","model":null,"input":[]}),
    )
    .await;
    assert_eq!(terminal(&mut ws).await["status"], 400);
    assert_eq!(source.requests.lock().unwrap().len(), 2);
    assert_eq!(host.reports.lock().unwrap().len(), 2);
    send(&mut ws, begin_interrupt_body()).await;
    let third_id = partial_response(&mut ws).await;
    source.finish_second.notify_one();
    assert_eq!(terminal(&mut ws).await["response"]["id"], third_id);
    reports(&host, 3).await;
    ws.close(None).await.unwrap();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    let reports = host.reports.lock().unwrap();
    assert_eq!(reports.len(), 3);
    assert!(reports
        .iter()
        .all(|report| report.success && report.status == 200 && report.account_id == "a"));
    task.abort();
}

#[tokio::test]
async fn websocket_initial_parser_errors_and_idle_close_do_not_create_inference() {
    let (app, source) = interrupted_source();
    let options = native::Options {
        request_timeout: Duration::from_secs(1),
        ..Default::default()
    };
    let (mut handle, host, task, _dir) = fixture(app, WireApi::Responses, false, options).await;
    let mut ws = socket(&handle).await;
    ws.send(Message::Text("bad".into())).await.unwrap();
    assert_eq!(terminal(&mut ws).await["status"], 400);
    // Closing before any inference is transport-only, even after a parser error.
    ws.close(None).await.unwrap();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    assert!(source.requests.lock().unwrap().is_empty());
    assert!(host.reports.lock().unwrap().is_empty());
    task.abort();
}

#[tokio::test]
async fn websocket_shutdown_after_parser_error_closes_only_idle_transport() {
    let (app, source) = interrupted_source();
    let (mut handle, host, task, _dir) =
        fixture(app, WireApi::Responses, false, native::Options::default()).await;
    let mut ws = socket(&handle).await;
    ws.send(Message::Text("bad".into())).await.unwrap();
    assert_eq!(terminal(&mut ws).await["status"], 400);
    // Keep the socket open so the initial reader deterministically exits on
    // shutdown, rather than relying on the ordering of a Close frame.
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    assert!(source.requests.lock().unwrap().is_empty());
    assert!(host.reports.lock().unwrap().is_empty());
    drop(ws);
    task.abort();
}

#[tokio::test]
async fn go_limits_websocket_invalid_first_frames_remain_recoverable_without_a_start_timer() {
    let (app, source) = interrupted_source();
    let (mut handle, host, task, _dir) =
        fixture(app, WireApi::Responses, false, native::Options::default()).await;
    host.initial_timeout_ms.store(50, Ordering::SeqCst);
    let mut ws = socket(&handle).await;
    for text in ["bad", "still bad"] {
        ws.send(Message::Text(text.into())).await.unwrap();
        assert_eq!(terminal(&mut ws).await["status"], 400);
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
    ws.send(Message::Ping(b"alive".to_vec().into())).await.unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(2), ws.next()).await.unwrap().unwrap().unwrap();
    assert!(matches!(reply, Message::Pong(value) if value.as_ref() == b"alive"));
    assert!(source.requests.lock().unwrap().is_empty());
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    assert!(host.reports.lock().unwrap().is_empty(),
        "invalid first frames and an idle shutdown must not create upstream inference");
    task.abort();
}
