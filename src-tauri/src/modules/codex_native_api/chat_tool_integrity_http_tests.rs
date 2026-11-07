//! Exercise tool sealing through real loopback HTTP and downstream WS delivery.
use super::*;

#[derive(Clone, Copy, Debug)]
enum Delivery {
    Json,
    Sse,
    WebSocket,
}

#[tokio::test]
async fn chat_collaboration_plaintext_markers_reach_clients_before_identity_persistence() {
    for delivery in [Delivery::Json, Delivery::Sse, Delivery::WebSocket] {
        for name in ["spawn_agent", "send_message", "followup_task"] {
            let app = Router::new().route(
                "/v1/chat/completions",
                post(move || async move {
                    let chunk = json!({"id":"parent","choices":[{"index":0,"delta":{"tool_calls":[{
                        "index":0,"id":"call-plain","type":"function","function":{
                        "name":format!("collaboration__{name}"),"arguments":"{\"message\":\"Please inspect the code\"}"}}]},
                        "finish_reason":"tool_calls"}]});
                    ([("content-type", "text/event-stream")], format!("data: {chunk}\n\ndata: [DONE]\n\n"))
                }),
            );
            let (mut handle, host, task, dir) =
                fixture(app, WireApi::Chat, false, native::Options::default()).await;
            let body = json!({"model":"m","stream":!matches!(delivery,Delivery::Json),
                "client_metadata":{"session_id":"parent","thread_id":"parent"},
                "input":[{"role":"user","content":"delegate"}],"tools":[{
                    "type":"namespace","name":"collaboration","tools":[{
                    "type":"function","name":name,"parameters":{"type":"object",
                    "properties":{"message":{"type":"string","encrypted":true}}}}]}]});
            let response = if matches!(delivery, Delivery::WebSocket) {
                let mut ws = socket(&handle).await;
                let mut body = body;
                body["type"] = json!("response.create");
                send(&mut ws, body).await;
                let event = terminal(&mut ws).await;
                assert_eq!(event["type"], "response.completed", "{event}");
                event["response"].clone()
            } else {
                let result = reqwest::Client::builder()
                    .no_proxy()
                    .build()
                    .unwrap()
                    .post(format!("http://{}/v1/responses", handle.addr))
                    .bearer_auth("key")
                    .json(&body)
                    .send()
                    .await
                    .unwrap();
                assert!(result.status().is_success(), "{delivery:?}: {name}");
                if matches!(delivery, Delivery::Json) {
                    result.json::<Value>().await.unwrap()
                } else {
                    result
                        .text()
                        .await
                        .unwrap()
                        .lines()
                        .filter_map(|line| line.strip_prefix("data: "))
                        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                        .find(|event| event["type"] == "response.completed")
                        .unwrap()["response"]
                        .clone()
                }
            };
            assert_eq!(response["output"][0]["encrypted_function_args"], json!([]));
            assert_eq!(response["output"][0]["namespace"], "collaboration");
            assert_eq!(response["output"][0]["name"], name);
            reports(&host, 1).await;
            assert!(host.reports.lock().unwrap()[0].success);
            let connection =
                rusqlite::Connection::open(dir.path().join("identity.sqlite")).unwrap();
            let encrypted: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM native_opaque_v1 WHERE kind='encrypted'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                encrypted, 0,
                "Plaintext must not acquire ciphertext ownership: {delivery:?} {name}"
            );
            handle.shutdown(Duration::from_secs(2)).await.unwrap();
            task.abort();
        }
    }
}

#[tokio::test]
async fn truncated_chat_function_arguments_never_create_a_client_success_or_checkpoint() {
    for delivery in [Delivery::Json, Delivery::Sse, Delivery::WebSocket] {
        for done in [false, true] {
            let app = Router::new().route("/v1/chat/completions", post(move || async move {
                let event = json!({"id":"truncated","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"broken-call","type":"function","function":{"name":"lookup","arguments":"{\"x\":"}}]},"finish_reason":"tool_calls"}]});
                let body = format!("data: {event}\n\n{}",if done {"data: [DONE]\n\n"} else {""});
                ([("content-type","text/event-stream")],body)
            }));
            let (mut handle, host, task, dir) =
                fixture(app, WireApi::Chat, false, native::Options::default()).await;
            let request = json!({"model":"m","stream":!matches!(delivery,Delivery::Json),"input":[{"role":"user","content":"use lookup"}],"tools":[{"type":"function","name":"lookup","parameters":{"type":"object"}}]});
            let events: Vec<Value> = if matches!(delivery, Delivery::WebSocket) {
                let mut ws = socket(&handle).await;
                let mut create = request.clone();
                create["type"] = json!("response.create");
                send(&mut ws, create).await;
                let mut events = vec![];
                tokio::time::timeout(Duration::from_secs(5), async {
                    loop {
                        match ws.next().await.unwrap().unwrap() {
                            Message::Text(text) => {
                                let event: Value = serde_json::from_str(&text).unwrap();
                                let failed = matches!(
                                    event["type"].as_str(),
                                    Some("error" | "response.failed" | "response.incomplete")
                                );
                                events.push(event);
                                if failed {
                                    break;
                                }
                            }
                            Message::Ping(value) => ws.send(Message::Pong(value)).await.unwrap(),
                            _ => {}
                        }
                    }
                })
                .await
                .unwrap();
                events
            } else {
                let response = reqwest::Client::builder()
                    .no_proxy()
                    .build()
                    .unwrap()
                    .post(format!("http://{}/v1/responses", handle.addr))
                    .bearer_auth("key")
                    .json(&request)
                    .send()
                    .await
                    .unwrap();
                if matches!(delivery, Delivery::Json) {
                    assert!(!response.status().is_success(), "{delivery:?} done={done}");
                    vec![response.json().await.unwrap()]
                } else {
                    let text = response
                        .text()
                        .await
                        .expect("truncated tools need a readable failure");
                    text.lines()
                        .filter_map(|line| line.strip_prefix("data: "))
                        .filter_map(|line| serde_json::from_str(line).ok())
                        .collect()
                }
            };
            assert!(!events.is_empty(), "{delivery:?} done={done}");
            assert!(
                events
                    .iter()
                    .all(|event| event["type"] != "response.completed"
                        && event["type"] != "response.function_call_arguments.done"
                        && event["type"] != "response.output_item.done"),
                "{delivery:?} done={done}: {events:?}"
            );
            let last = events.last().unwrap();
            assert!(
                last.get("error").is_some() || last.pointer("/response/error").is_some(),
                "{last}"
            );
            reports(&host, 1).await;
            assert!(!host.reports.lock().unwrap()[0].success);
            let connection =
                rusqlite::Connection::open(dir.path().join("identity.sqlite")).unwrap();
            for query in [
                "SELECT COUNT(*) FROM native_opaque_v1 WHERE kind='response'",
                "SELECT COUNT(*) FROM native_rebuild_checkpoints_v1",
                "SELECT COUNT(*) FROM native_rebuild_portable_v1",
            ] {
                let count: i64 = connection.query_row(query, [], |row| row.get(0)).unwrap();
                assert_eq!(count, 0, "{delivery:?} done={done}: {query}");
            }
            handle.shutdown(Duration::from_secs(2)).await.unwrap();
            task.abort();
        }
    }
}
