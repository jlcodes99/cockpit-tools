use super::*;

async fn read_upstream(socket: &mut axum::extract::ws::WebSocket) -> Value {
    loop {
        match tokio::time::timeout(Duration::from_secs(4), socket.recv())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
        {
            Message::Text(text) => return serde_json::from_str(&text).unwrap(),
            Message::Ping(bytes) => socket.send(Message::Pong(bytes)).await.unwrap(),
            message => panic!("unexpected {message:?}"),
        }
    }
}
async fn connect(
    handle: &Handle,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let mut request = format!("ws://{}/v1/responses", handle.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", "Bearer local-test-key".parse().unwrap());
    tokio_tungstenite::connect_async(request).await.unwrap().0
}
async fn write(socket: &mut axum::extract::ws::WebSocket, event: Value) {
    socket
        .send(Message::Text(event.to_string().into()))
        .await
        .unwrap();
}

#[tokio::test]
async fn audit_interrupt_rejection_cannot_consume_a_later_steer_or_lose_its_successor() {
    let (addr, task) = mock(Router::new().route("/v1/responses",get(|upgrade:WebSocketUpgrade|async move {
        upgrade.on_upgrade(|mut socket|async move {
            read_upstream(&mut socket).await;
            write(&mut socket,json!({"type":"response.created","response":{"id":"parent"}})).await;
            assert_eq!(read_upstream(&mut socket).await["type"],"response.interrupt");
            assert_eq!(read_upstream(&mut socket).await["type"],"response.steer");
            write(&mut socket,json!({"type":"error","error":{"code":"unsupported_native_inflight_message"}})).await;
            write(&mut socket,json!({"type":"response.steer.accepted","steer":{"id":"s","previous_response_id":"parent"}})).await;
            write(&mut socket,json!({"type":"response.incomplete","response":{"id":"parent","status":"incomplete","output":[]}})).await;
            write(&mut socket,json!({"type":"response.created","response":{"id":"successor","previous_response_id":"parent"}})).await;
            write(&mut socket,json!({"type":"response.completed","response":{"id":"successor","status":"completed","output":[]}})).await;
            let _=socket.recv().await;
        })
    }))).await;
    let (mut handle,host,path)=native(addr,false,false,Options::default()).await;
    let mut socket=connect(&handle).await;
    socket.send(ClientMessage::Text(json!({"type":"response.create","model":"m","input":[]}).to_string().into())).await.unwrap();
    ws_json(&mut socket).await;
    for control in [json!({"type":"response.interrupt","response_id":"parent"}),json!({"type":"response.steer","event_id":"steer","previous_response_id":"parent","input":"new direction"})] {
        socket.send(ClientMessage::Text(control.to_string().into())).await.unwrap();
    }
    let rejected=ws_json(&mut socket).await;
    assert_eq!(rejected["error"]["code"],"unsupported_native_inflight_message");
    assert!(rejected.get("event_id").is_none());
    let accepted=ws_json(&mut socket).await;
    assert_eq!(accepted["type"],"response.steer.accepted");
    assert_eq!(accepted["event_id"],"steer");
    assert_eq!(ws_json(&mut socket).await["type"],"response.incomplete");
    assert_eq!(ws_json(&mut socket).await["response"]["id"],"successor");
    assert_eq!(ws_json(&mut socket).await["type"],"response.completed");
    socket.close(None).await.unwrap();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    assert_eq!(host.reports.lock().unwrap().len(),2);
    assert!(host.reports.lock().unwrap()[1].success);
    assert_eq!(host.capacity.available_permits(),1);
    task.abort();let _=std::fs::remove_dir_all(path);
}

#[tokio::test]
async fn subscription_steering_keeps_client_event_id_out_of_strict_upstream_payload() {
    let (addr, task) = mock(Router::new().route("/v1/responses", get(|ws: WebSocketUpgrade| async move {
        ws.on_upgrade(|mut socket| async move {
            read_upstream(&mut socket).await;
            write(&mut socket, json!({"type":"response.created","response":{"id":"p"}})).await;
            let steer = read_upstream(&mut socket).await;
            assert_eq!(steer, json!({"type":"response.steer","previous_response_id":"p","input":"new direction"}));
            write(&mut socket, json!({"type":"response.steer.accepted","steer":{"id":"s","previous_response_id":"p"}})).await;
            write(&mut socket, json!({"type":"response.incomplete","response":{"id":"p","status":"incomplete","output":[]}})).await;
            write(&mut socket, json!({"type":"response.created","response":{"id":"successor","previous_response_id":"p"}})).await;
            write(&mut socket, json!({"type":"response.completed","response":{"id":"successor","status":"completed","output":[]}})).await;
            let _ = socket.recv().await;
        })
    }))).await;
    let (mut handle, host, path) = native(addr, true, false, Options::default()).await;
    let mut socket = connect(&handle).await;
    socket.send(ClientMessage::Text(json!({"type":"response.create","model":"m","input":[]}).to_string().into())).await.unwrap();
    assert_eq!(ws_json(&mut socket).await["response"]["id"], "p");
    socket.send(ClientMessage::Text(json!({"type":"response.steer","event_id":"client-control","previous_response_id":"p","input":"new direction"}).to_string().into())).await.unwrap();
    let ack = ws_json(&mut socket).await;
    assert_eq!(ack["type"], "response.steer.accepted");
    assert_eq!(ack["event_id"], "client-control");
    assert_eq!(ws_json(&mut socket).await["type"], "response.incomplete");
    assert_eq!(ws_json(&mut socket).await["response"]["id"], "successor");
    assert_eq!(ws_json(&mut socket).await["type"], "response.completed");
    socket.close(None).await.unwrap();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    assert_eq!(host.capacity.available_permits(), 1);
    task.abort();
    let _ = std::fs::remove_dir_all(path);
}

#[tokio::test]
async fn full_compat_steering_automatic_successor_keeps_parent_settings_and_queues_create() {
    let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
    let (addr,task)=mock(Router::new().route("/v1/responses",get({let seen=seen.clone();move |ws:WebSocketUpgrade|{let seen=seen.clone();async move{ws.on_upgrade(move |mut socket|async move {
        let first=read_upstream(&mut socket).await;seen.lock().unwrap().push(first);
        write(&mut socket,json!({"type":"response.created","response":{"id":"parent","created_at":1}})).await;
        let steer=read_upstream(&mut socket).await;assert_eq!(steer["type"],"response.steer");seen.lock().unwrap().push(steer);
        write(&mut socket,json!({"type":"response.steer.accepted","sequence_number":8,"steer":{"id":"s","previous_response_id":"parent"},"unknown":"preserved"})).await;
        write(&mut socket,json!({"type":"response.incomplete","response":{"id":"parent","status":"incomplete","output":[],"incomplete_details":{"reason":"steered"}}})).await;
        write(&mut socket,json!({"type":"response.created","response":{"id":"auto","previous_response_id":"parent","created_at":2}})).await;
        write(&mut socket,json!({"type":"response.output_text.delta","response_id":"auto","delta":"automatic"})).await;
        write(&mut socket,json!({"type":"response.completed","response":{"id":"auto","status":"completed","output":[]}})).await;
        let next=read_upstream(&mut socket).await;seen.lock().unwrap().push(next);
        write(&mut socket,json!({"type":"response.completed","response":{"id":"queued","status":"completed","output":[]}})).await;
        let _=socket.recv().await;
    })}}}))).await;
    let (mut handle, host, path) = native(addr, false, false, Options::default()).await;
    let mut socket = connect(&handle).await;
    socket.send(ClientMessage::Text(json!({"type":"response.create","model":"m","input":[],"instructions":"parent settings"}).to_string().into())).await.unwrap();
    assert_eq!(ws_json(&mut socket).await["response"]["id"], "parent");
    socket.send(ClientMessage::Text(json!({"type":"response.create","model":"m","input":[],"instructions":"queued settings"}).to_string().into())).await.unwrap();
    let steer = json!({"type":"response.steer","event_id":"control","previous_response_id":"parent","input":[{"type":"configuration_update","reasoning":{"effort":"high"}},{"role":"user","content":[{"type":"input_text","text":"change direction"}]}],"extension":{"keep":true}});
    socket
        .send(ClientMessage::Text(steer.to_string().into()))
        .await
        .unwrap();
    let ack = ws_json(&mut socket).await;
    assert_eq!(ack["type"], "response.steer.accepted", "{ack}");
    assert_eq!(ack["sequence_number"], 8);
    assert_eq!(ack["unknown"], "preserved");
    assert_eq!(ws_json(&mut socket).await["type"], "response.incomplete");
    assert_eq!(ws_json(&mut socket).await["response"]["id"], "auto");
    assert_eq!(ws_json(&mut socket).await["delta"], "automatic");
    assert_eq!(ws_json(&mut socket).await["response"]["id"], "auto");
    assert_eq!(ws_json(&mut socket).await["response"]["id"], "queued");
    let requests = seen.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[1], steer);
    assert_eq!(requests[2]["instructions"], "queued settings");
    drop(requests);
    socket.close(None).await.unwrap();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    assert_eq!(host.capacity.available_permits(), 1);
    task.abort();
    let _ = std::fs::remove_dir_all(path);
}

#[tokio::test]
async fn full_compat_steering_pending_allows_tool_result_without_replaying_control() {
    let (sent, mut seen) = tokio::sync::mpsc::channel(4);
    let (addr,task)=mock(Router::new().route("/v1/responses",get(move |ws:WebSocketUpgrade|{let sent=sent.clone();async move{ws.on_upgrade(move |mut socket|async move {
        read_upstream(&mut socket).await;
        write(&mut socket,json!({"type":"response.created","response":{"id":"p","created_at":1}})).await;
        let steer=read_upstream(&mut socket).await;sent.send(steer).await.unwrap();
        write(&mut socket,json!({"type":"response.steer.accepted","steer":{"id":"s","previous_response_id":"p"}})).await;
        write(&mut socket,json!({"type":"response.incomplete","response":{"id":"p","status":"incomplete","output":[{"type":"function_call","call_id":"c","name":"lookup","arguments":"{}"}]}})).await;
        write(&mut socket,json!({"type":"response.steer.pending","steer":{"id":"s","previous_response_id":"p"},"reason":"waiting_for_required_input","required_input":[{"type":"function_call_output","call_id":"c"}]})).await;
        let continuation=read_upstream(&mut socket).await;sent.send(continuation).await.unwrap();
        write(&mut socket,json!({"type":"response.created","response":{"id":"next","previous_response_id":"p","created_at":2}})).await;
        write(&mut socket,json!({"type":"response.completed","response":{"id":"next","status":"completed","output":[]}})).await;
        let _=socket.recv().await;
    })}}))).await;
    let (mut handle, host, path) = native(addr, false, false, Options::default()).await;
    let mut socket = connect(&handle).await;
    socket
        .send(ClientMessage::Text(
            json!({"type":"response.create","model":"m","input":[]})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    ws_json(&mut socket).await;
    socket
        .send(ClientMessage::Text(
            json!({"type":"response.steer","previous_response_id":"p","input":"use tool result"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    assert_eq!(
        ws_json(&mut socket).await["type"],
        "response.steer.accepted"
    );
    assert_eq!(ws_json(&mut socket).await["type"], "response.incomplete");
    assert_eq!(ws_json(&mut socket).await["type"], "response.steer.pending");
    socket.send(ClientMessage::Text(json!({"type":"response.append","input":[{"type":"function_call_output","call_id":"c","output":"result"}]}).to_string().into())).await.unwrap();
    let next = ws_json(&mut socket).await;
    assert_eq!(next["response"]["id"], "next", "{next}");
    ws_json(&mut socket).await;
    assert_eq!(seen.recv().await.unwrap()["type"], "response.steer");
    let continuation = seen.recv().await.unwrap();
    assert_eq!(continuation["type"], "response.create");
    assert_eq!(continuation["previous_response_id"], "p");
    assert_eq!(continuation["input"].as_array().unwrap().len(), 1);
    socket.close(None).await.unwrap();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    assert_eq!(host.capacity.available_permits(), 1);
    task.abort();
    let _ = std::fs::remove_dir_all(path);
}

#[tokio::test]
async fn full_compat_steering_reauthorizes_and_reports_loss_without_replay() {
    let (addr,task)=mock(Router::new().route("/v1/responses",get(|ws:WebSocketUpgrade|async move{ws.on_upgrade(move |mut socket|async move{
        read_upstream(&mut socket).await;
        write(&mut socket,json!({"type":"response.created","response":{"id":"p","created_at":1}})).await;
        let rejected=read_upstream(&mut socket).await;assert_eq!(rejected["event_id"],"reject");
        write(&mut socket,json!({"type":"response.steer.failed","event_id":"reject","steer":{"id":"s","previous_response_id":"p"},"error":{"code":"invalid_input","message":"rejected"}})).await;
        write(&mut socket,json!({"type":"response.output_text.delta","delta":"still running"})).await;
        let control=read_upstream(&mut socket).await;assert_eq!(control["event_id"],"lost");
    })}))).await;
    let (mut handle, host, path) = native(addr, false, false, Options::default()).await;
    let mut socket = connect(&handle).await;
    socket
        .send(ClientMessage::Text(
            json!({"type":"response.create","model":"m","input":[]})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    ws_json(&mut socket).await;
    host.key_enabled.store(false, Ordering::SeqCst);
    socket.send(ClientMessage::Text(json!({"type":"response.steer","event_id":"denied","previous_response_id":"p","input":"x"}).to_string().into())).await.unwrap();
    let denied = ws_json(&mut socket).await;
    assert_eq!(denied["error"]["code"], "invalid_api_key");
    assert_eq!(denied["event_id"], "denied");
    host.key_enabled.store(true, Ordering::SeqCst);
    for event_id in ["reject", "lost"] {
        socket.send(ClientMessage::Text(json!({"type":"response.steer","event_id":event_id,"previous_response_id":"p","input":"x"}).to_string().into())).await.unwrap();
        if event_id == "reject" {
            assert_eq!(ws_json(&mut socket).await["type"], "response.steer.failed");
            assert_eq!(ws_json(&mut socket).await["delta"], "still running");
        }
    }
    assert_eq!(
        ws_json(&mut socket).await["error"]["code"],
        "upstream_websocket_lost"
    );
    let lost = ws_json(&mut socket).await;
    assert_eq!(lost["event_id"], "lost");
    assert!(lost["error"].get("not_sent").is_none());
    assert_eq!(host.selections.lock().unwrap().len(), 1);
    socket.close(None).await.unwrap();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
    let _ = std::fs::remove_dir_all(path);
}

#[tokio::test]
async fn full_compat_websocket_compression_negotiates_on_both_legs() {
    let negotiated = Arc::new(AtomicBool::new(false));
    let (addr,task)=mock(Router::new().route("/v1/responses",get({let negotiated=negotiated.clone();move |headers:HeaderMap,ws:yawc::IncomingUpgrade|{let negotiated=negotiated.clone();async move{
        negotiated.store(headers.get("sec-websocket-extensions").is_some_and(|v|v.to_str().unwrap().contains("permessage-deflate")),Ordering::SeqCst);
        let (response,upgrade)=ws.upgrade(super::super::websocket_io::options(1024*1024)).unwrap();
        tokio::spawn(async move{let mut socket=upgrade.await.unwrap();let request=socket.next().await.unwrap();let value:Value=serde_json::from_slice(request.payload()).unwrap();assert_eq!(value["input"][0]["content"],"long input ".repeat(4096));
            socket.send(yawc::Frame::text(json!({"type":"response.completed","response":{"id":"r","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"long output ".repeat(4096)}]}]}}).to_string())).await.unwrap();let _=socket.next().await;});
        response.map(Body::new)
    }}}))).await;
    let (mut handle, _, path) = native(addr, false, false, Options::default()).await;
    let mut socket = yawc::WebSocket::connect(
        format!("ws://{}/v1/responses", handle.addr)
            .parse()
            .unwrap(),
    )
    .with_options(super::super::websocket_io::options(1024 * 1024))
    .with_request(axum::http::Request::builder().header("authorization", "Bearer local-test-key"))
    .await
    .unwrap();
    socket.send(yawc::Frame::text(json!({"type":"response.create","model":"m","input":[{"role":"user","content":"long input ".repeat(4096)}]}).to_string())).await.unwrap();
    let event = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .unwrap()
        .unwrap();
    let event: Value = serde_json::from_slice(event.payload()).unwrap();
    assert_eq!(
        event["response"]["output"][0]["content"][0]["text"],
        "long output ".repeat(4096)
    );
    assert!(negotiated.load(Ordering::SeqCst));
    socket.close().await.unwrap();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
    let _ = std::fs::remove_dir_all(path);
}

#[tokio::test]
async fn full_compat_websocket_wire_uses_rsv1_and_bounds_decompressed_messages() {
    use tokio::io::AsyncReadExt;
    let (client, mut server) = tokio::io::duplex(8192);
    // The same adapter used by both production legs writes actual RFC 7692 frames.
    let mut writer = super::super::websocket_io::Socket::<_, ClientMessage>::new(
        yawc::WebSocket::from_stream_with_extensions(
            client,
            yawc::Role::Client,
            Some("permessage-deflate; client_no_context_takeover; server_no_context_takeover"),
            super::super::websocket_io::options(1024 * 1024),
        )
        .unwrap(),
    );
    writer
        .send(ClientMessage::Text("repeat".repeat(1000).into()))
        .await
        .unwrap();
    let first = server.read_u8().await.unwrap();
    assert_eq!(first & 0x40, 0x40);
    let (client, server) = tokio::io::duplex(8192);
    let mut writer = yawc::WebSocket::from_stream_with_extensions(
        client,
        yawc::Role::Client,
        Some("permessage-deflate"),
        super::super::websocket_io::options(8192),
    )
    .unwrap();
    let mut reader = super::super::websocket_io::Socket::<_, ClientMessage>::new(
        yawc::WebSocket::from_stream_with_extensions(
            server,
            yawc::Role::Server,
            Some("permessage-deflate"),
            super::super::websocket_io::options(128),
        )
        .unwrap(),
    );
    writer
        .send(yawc::Frame::text("x".repeat(4096)))
        .await
        .unwrap();
    let result = reader.next().await.unwrap();
    assert!(
        matches!(
            result,
            Err(tokio_tungstenite::tungstenite::Error::Capacity(_))
        ),
        "{result:?}"
    );
}

#[tokio::test]
async fn full_compat_websocket_field_repair_covers_initial_and_reused_turns() {
    use std::sync::atomic::AtomicUsize;
    for reused in [false, true] {
        let attempts = Arc::new(AtomicUsize::new(0));
        let app=Router::new().route("/v1/responses",get({let attempts=attempts.clone();move |ws:WebSocketUpgrade|{
            let index=attempts.fetch_add(1,Ordering::SeqCst);async move{ws.on_upgrade(move |mut socket|async move{
                let mut request=read_upstream(&mut socket).await;
                if reused && index==0 {
                    write(&mut socket,json!({"type":"response.completed","response":{"id":"first","status":"completed","output":[]}})).await;
                    request=read_upstream(&mut socket).await;
                }
                if index==0 {
                    assert_eq!(request["input"][0]["status"],"completed");
                    write(&mut socket,json!({"type":"error","status":400,"error":{"code":"unknown_parameter","param":"input[0].status","message":"Unknown parameter input[0].status"}})).await;
                } else {
                    assert!(request["input"][0].get("status").is_none());
                    write(&mut socket,json!({"type":"response.completed","response":{"id":"repaired","status":"completed","output":[]}})).await;
                }
                let _=socket.recv().await;
            })}
        }}));
        let (addr, task) = mock(app).await;
        let (mut handle, _, path) = native(addr, false, false, Options::default()).await;
        let mut socket = connect(&handle).await;
        if reused {
            socket
                .send(ClientMessage::Text(
                    json!({"type":"response.create","model":"m","input":[]})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            assert_eq!(ws_json(&mut socket).await["response"]["id"], "first");
        }
        socket.send(ClientMessage::Text(json!({"type":"response.create","model":"m","input":[{"type":"message","role":"user","content":"hello","status":"completed"}]}).to_string().into())).await.unwrap();
        let response = ws_json(&mut socket).await;
        assert_eq!(response["response"]["id"], "repaired", "{response}");
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        socket.close(None).await.unwrap();
        handle.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
        let _ = std::fs::remove_dir_all(path);
    }
}

#[tokio::test]
async fn native_lane_steer_rejection_does_not_settle_generation_or_release_account() {
    let (addr, task) = mock(Router::new().route("/v1/responses", get(|ws: WebSocketUpgrade| async move {
        ws.on_upgrade(|mut socket| async move {
            read_upstream(&mut socket).await;
            write(&mut socket,json!({"type":"response.created","response":{"id":"running","created_at":1}})).await;
            let steer=read_upstream(&mut socket).await;
            assert_eq!(steer["type"],"response.steer");
            write(&mut socket,json!({"type":"error","error":{"code":"unsupported_native_inflight_message","message":"The experimental native turn lane cannot accept stateful WebSocket messages while a native turn is running. Start a new independent response.create turn instead."}})).await;
            write(&mut socket,json!({"type":"response.output_text.delta","delta":"original continues"})).await;
            write(&mut socket,json!({"type":"response.completed","response":{"id":"running","status":"completed","output":[]}})).await;
            read_upstream(&mut socket).await;
            write(&mut socket,json!({"type":"response.completed","response":{"id":"after","status":"completed","output":[]}})).await;
            let _=socket.recv().await;
        })
    }))).await;
    let (mut handle, host, path) = native(addr, false, false, Options::default()).await;
    let mut socket = connect(&handle).await;
    let create = json!({"type":"response.create","model":"m","input":[]});
    socket
        .send(ClientMessage::Text(create.to_string().into()))
        .await
        .unwrap();
    ws_json(&mut socket).await;
    socket
        .send(ClientMessage::Text(
            json!({"type":"response.steer","previous_response_id":"running","input":"change"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    assert_eq!(
        ws_json(&mut socket).await["error"]["code"],
        "unsupported_native_inflight_message"
    );
    assert_eq!(ws_json(&mut socket).await["delta"], "original continues");
    assert_eq!(ws_json(&mut socket).await["response"]["id"], "running");
    socket
        .send(ClientMessage::Text(create.to_string().into()))
        .await
        .unwrap();
    assert_eq!(ws_json(&mut socket).await["response"]["id"], "after");
    socket.close(None).await.unwrap();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    let reports = host.reports.lock().unwrap();
    assert_eq!(reports.len(), 2);
    assert!(reports
        .iter()
        .all(|report| report.success && report.status == 200));
    task.abort();
    let _ = std::fs::remove_dir_all(path);
}
