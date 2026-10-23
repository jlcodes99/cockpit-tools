use super::*;
use axum::http::StatusCode;

fn frame(event: Value) -> String {
    format!(
        "event: {}\ndata: {event}\n\n",
        event["type"].as_str().unwrap()
    )
}

fn events(text: &str) -> Vec<Value> {
    text.lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter_map(|data| serde_json::from_str(data).ok())
        .collect()
}

#[tokio::test]
async fn responses_coalesced_data_lines_preserve_tools_and_terminal_for_stream_and_json() {
    for stream in [false, true] {
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let tool = json!({"id":"tool_item","type":"function_call","status":"completed","call_id":"call","name":"lookup","arguments":"{\"query\":\"synthetic\"}"});
        let payload = [
            json!({"type":"response.created","response":{"id":"coalesced","status":"in_progress"}}),
            json!({"type":"response.function_call_arguments.delta","item_id":"tool_item","delta":"{\"query\":\"synthetic\"}"}),
            json!({"type":"response.output_item.done","output_index":0,"item":tool.clone()}),
            json!({"type":"response.completed","response":{"id":"coalesced","status":"completed","output":[]}}),
        ].iter().map(|event|format!("data: {event}\n")).collect::<String>() + "\n";
        let app = Router::new().route(
            "/v1/responses",
            post(move || {
                count.fetch_add(1, Ordering::SeqCst);
                let payload = payload.clone();
                async move { ([("content-type", "text/event-stream")], payload) }
            }),
        );
        let (handle, host, task, path) = start(
            app,
            ExecutionPolicy {
                bootstrap_retries: 2,
                ..Default::default()
            },
        )
        .await;
        let response = client()
            .post(format!("http://{}/v1/responses", handle.addr))
            .bearer_auth("key")
            .json(&json!({"model":"m","stream":stream,"input":[]}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let raw = response.text().await.unwrap();
        let terminal = if stream {
            let output = events(&raw);
            assert_eq!(output.len(), 4, "{raw}");
            assert_eq!(output[2]["item"], tool);
            assert_eq!(output[3]["type"], "response.completed");
            output[3]["response"].clone()
        } else {
            serde_json::from_str::<Value>(&raw).unwrap()
        };
        assert_eq!(terminal["output"], json!([tool]));
        assert_eq!(terminal["id"], "coalesced");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(host.reports.lock().unwrap()[0].success);
        shutdown(handle, task, path).await;
    }
}

#[tokio::test]
async fn responses_coalesced_data_lines_never_retry_error_after_visible_output() {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let payload=[
        json!({"type":"response.created","response":{"id":"coalesced-failed","status":"in_progress"}}),
        json!({"type":"response.output_text.delta","delta":"visible result"}),
        json!({"type":"response.failed","response":{"id":"coalesced-failed","status":"failed","error":{"type":"rate_limit_error","code":"rate_limit_exceeded","message":"synthetic quota"}}}),
    ].iter().map(|event|format!("data: {event}\n")).collect::<String>()+"\n";
    let app = Router::new().route(
        "/v1/responses",
        post(move || {
            count.fetch_add(1, Ordering::SeqCst);
            let payload = payload.clone();
            async move { ([("content-type", "text/event-stream")], payload) }
        }),
    );
    let (handle, host, task, path) = start(
        app,
        ExecutionPolicy {
            bootstrap_retries: 2,
            status_retries: 2,
            credential_retries: 2,
            ..Default::default()
        },
    )
    .await;
    let response = client()
        .post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("key")
        .json(&json!({"model":"m","stream":true,"input":[]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let raw = response.text().await.unwrap();
    let output = events(&raw);
    assert_eq!(output.len(), 3, "{raw}");
    assert_eq!(output[1]["delta"], "visible result");
    // Ordinary Responses clients receive the gateway's canonical public error
    // envelope; Codex clients receive response.failed through the same path.
    assert_eq!(output[2]["type"], "error");
    assert_eq!(output[2]["status"], 429);
    assert_eq!(output[2]["error"]["code"], "rate_limit_exceeded");
    assert_eq!(output[2]["error"]["message"], "synthetic quota");
    assert!(!raw.contains("response.completed"));
    assert!(!raw.contains("[DONE]"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(!host.reports.lock().unwrap()[0].success);
    let connection = rusqlite::Connection::open(&path).unwrap();
    let rows: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM native_opaque_v1 WHERE kind='response'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rows, 0);
    drop(connection);
    shutdown(handle, task, path).await;
}

#[tokio::test]
async fn responses_explicit_done_after_data_line_without_completion_never_commits_success() {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let app=Router::new().route("/v1/responses",post(move || {
        count.fetch_add(1,Ordering::SeqCst);
        async { ([("content-type","text/event-stream")],
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial only\"}\ndata: [DONE]\n\n") }
    }));
    let (handle, host, task, path) = start(
        app,
        ExecutionPolicy {
            bootstrap_retries: 2,
            credential_retries: 2,
            ..Default::default()
        },
    )
    .await;
    let response = client()
        .post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("key")
        .json(&json!({"model":"m","stream":true,"input":[]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let raw = response.text().await.unwrap();
    let output = events(&raw);
    assert_eq!(output[0]["delta"], "partial only");
    assert_eq!(
        output.last().unwrap()["error"]["code"],
        "incomplete_upstream_stream"
    );
    assert!(!raw.contains("response.completed"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    tokio::time::sleep(Duration::from_millis(20)).await;
    let reports = host.reports.lock().unwrap();
    assert_eq!(reports.len(), 1);
    assert!(!reports[0].success);
    drop(reports);
    let connection = rusqlite::Connection::open(&path).unwrap();
    let rows: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM native_opaque_v1 WHERE kind='response'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rows, 0);
    for table in ["native_rebuild_checkpoints_v1", "native_rebuild_history_v1"] {
        let rows: i64 = connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(rows, 0, "{table}");
    }
    drop(connection);
    shutdown(handle, task, path).await;
}

#[tokio::test]
async fn responses_midstream_disconnect_has_one_readable_failure_and_never_retries_delivered_output(
) {
    for (codex, immediate, broken_transport, chatgpt) in [
        (false, false, false, false),
        (true, false, false, true),
        (true, true, true, true),
        (false, true, true, false),
    ] {
        let calls = Arc::new(AtomicUsize::new(0));
        let app = Router::new().route("/v1/responses", post({
            let calls = calls.clone();
            move || {
                calls.fetch_add(1, Ordering::SeqCst);
                async move {
                    let initial = frame(json!({"type":"response.created","sequence_number":0,"response":{"id":"partial","status":"in_progress"}}))
                        + &frame(json!({"type":"response.output_text.delta","sequence_number":1,"delta":"visible text"}));
                    let prefix = futures_util::stream::once(async { Ok::<_,std::io::Error>(Bytes::from(initial)) });
                    let tail = futures_util::stream::once(async move {
                        tokio::time::sleep(Duration::from_millis(350)).await;
                        if broken_transport { Err(std::io::Error::other("simulated chunked EOF")) } else { Ok(Bytes::new()) }
                    });
                    ([ ("content-type","text/event-stream") ], Body::from_stream(prefix.chain(tail)))
                }
            }
        }));
        let policy = ExecutionPolicy {
            immediate_sse: immediate,
            bootstrap_retries: 2,
            ..Default::default()
        };
        let (handle, host, task, path) = start_with_chatgpt(app, policy, chatgpt).await;
        let response = client()
            .post(format!("http://{}/v1/responses", handle.addr))
            .bearer_auth("key")
            .header(
                "user-agent",
                if codex {
                    "codex_cli_rs/1"
                } else {
                    "third-party/1"
                },
            )
            .json(&json!({"model":"m","stream":true}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let text = response
            .text()
            .await
            .expect("a downstream stream failure must remain readable");
        let output = events(&text);
        assert_eq!(output.len(), 3, "{text}");
        assert_eq!(output[1]["delta"], "visible text");
        assert_eq!(
            output[2]["type"],
            if codex { "response.failed" } else { "error" }
        );
        assert_eq!(output[2]["sequence_number"], 2);
        let detail = if codex {
            &output[2]["response"]["error"]
        } else {
            &output[2]["error"]
        };
        assert_eq!(output[2]["status"], 502);
        assert_eq!(
            detail["code"],
            if broken_transport {
                "upstream_stream_error"
            } else {
                "incomplete_upstream_stream"
            }
        );
        assert!(!detail["message"]
            .as_str()
            .unwrap()
            .contains("stream disconnected before completion"));
        assert!(!detail["message"]
            .as_str()
            .unwrap()
            .contains("stream error:"));
        assert!(!text.contains("response.completed"));
        assert!(!text.contains("[DONE]"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        tokio::time::sleep(Duration::from_millis(20)).await;
        let reports = host.reports.lock().unwrap();
        assert_eq!(reports.len(), 1);
        assert!(!reports[0].success);
        assert!(matches!(
            reports[0].error.as_deref(),
            Some("incomplete_upstream_stream" | "upstream_stream_error")
        ));
        drop(reports);
        shutdown(handle, task, path).await;
    }
}

#[tokio::test]
async fn responses_bootstrap_retries_keep_eof_missing_terminal_and_timeout_distinct() {
    for (failure, code, status) in [
        ("eof", "upstream_stream_error", 502),
        ("missing_terminal", "incomplete_upstream_stream", 502),
        ("timeout", "upstream_idle_timeout", 504),
    ] {
        let calls = Arc::new(AtomicUsize::new(0));
        let app = Router::new().route("/v1/responses", post({
            let calls = calls.clone();
            move || {
                let first = calls.fetch_add(1, Ordering::SeqCst) == 0;
                async move {
                    let body = if !first {
                        Body::from(completed())
                    } else if failure == "timeout" {
                        Body::from_stream(futures_util::stream::pending::<Result<Bytes, std::io::Error>>())
                    } else {
                        let prefix = futures_util::stream::once(async {
                            Ok::<_, std::io::Error>(Bytes::from(frame(json!({"type":"response.created","response":{"id":"attempt","status":"in_progress"}}))))
                        });
                        let tail = futures_util::stream::once(async move {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                            if failure == "eof" {
                                Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "unexpected EOF during chunk size line"))
                            } else { Ok(Bytes::new()) }
                        });
                        Body::from_stream(prefix.chain(tail))
                    };
                    ([("content-type", "text/event-stream")], body)
                }
            }
        }));
        let (handle, host, task, path) = start_with_chatgpt(
            app,
            ExecutionPolicy {
                bootstrap_retries: 1,
                retry_base_delay: Duration::from_millis(1),
                idle_timeout: Duration::from_millis(100),
                ..Default::default()
            },
            true,
        )
        .await;
        let response = client()
            .post(format!("http://{}/v1/responses", handle.addr))
            .bearer_auth("key")
            .header("originator", "codex-tui")
            .json(&json!({"model":"m","stream":true}))
            .send()
            .await
            .unwrap();
        let text = response.text().await.unwrap();
        assert!(text.contains("response.completed"), "{failure}: {text}");
        assert!(!text.contains("response.failed"));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        shutdown(handle, task, path).await;
        let reports = host.reports.lock().unwrap();
        assert!(reports[0].success);
        let attempts = &reports[0].diagnostics.as_ref().unwrap().attempts;
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0].error_category.as_deref(), Some(code));
        assert_eq!(attempts[0].status, Some(status));
        assert_eq!(attempts[0].account_id, attempts[1].account_id);
    }
}

#[tokio::test]
async fn responses_active_stream_total_timeout_keeps_its_code_for_codex() {
    let app = Router::new().route("/v1/responses", post(|| async {
        let stream = futures_util::stream::unfold(0usize, |index| async move {
            if index > 0 { tokio::time::sleep(Duration::from_millis(20)).await; }
            let data = if index == 0 { frame(json!({"type":"response.created","response":{"id":"busy","status":"in_progress"}})) }
                else { frame(json!({"type":"response.output_text.delta","delta":"more"})) };
            Some((Ok::<_, std::io::Error>(Bytes::from(data)), index + 1))
        });
        ([("content-type", "text/event-stream")], Body::from_stream(stream))
    }));
    let (handle, host, task, path) = start_with_chatgpt(
        app,
        ExecutionPolicy {
            total_timeout: Duration::from_millis(200),
            idle_timeout: Duration::from_secs(5),
            ..Default::default()
        },
        true,
    )
    .await;
    let text = client()
        .post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("key")
        .header("originator", "codex-tui")
        .json(&json!({"model":"m","stream":true}))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let output = events(&text);
    let failure = output.last().unwrap();
    assert_eq!(failure["type"], "response.failed");
    assert_eq!(failure["status"], 504);
    assert_eq!(
        failure["response"]["error"]["code"],
        "upstream_total_timeout"
    );
    assert!(!text.contains("response.completed"));
    assert!(!text.contains("stream disconnected before completion:"));
    shutdown(handle, task, path).await;
    let reports = host.reports.lock().unwrap();
    assert_eq!(reports[0].error.as_deref(), Some("upstream_total_timeout"));
    assert!(!reports[0].success);
}

#[tokio::test]
async fn responses_websocket_bridge_preserves_read_failure_and_missing_terminal_codes() {
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
    for broken in [false, true] {
        let calls = Arc::new(AtomicUsize::new(0));
        let app = Router::new().route("/v1/responses", post({
            let calls = calls.clone();
            move || {
                calls.fetch_add(1, Ordering::SeqCst);
                async move {
                    let prefix = futures_util::stream::once(async {
                        Ok::<_, std::io::Error>(Bytes::from(frame(json!({"type":"response.created","response":{"id":"ws-partial","status":"in_progress"}}))
                            + &frame(json!({"type":"response.output_text.delta","delta":"visible"}))))
                    });
                    let tail = futures_util::stream::once(async move {
                        tokio::time::sleep(Duration::from_millis(350)).await;
                        if broken { Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "unexpected EOF")) }
                        else { Ok(Bytes::new()) }
                    });
                    ([("content-type", "text/event-stream")], Body::from_stream(prefix.chain(tail)))
                }
            }
        }));
        let (handle, host, task, path) =
            start_with_chatgpt(app, ExecutionPolicy::default(), true).await;
        let mut request = format!("ws://{}/v1/responses", handle.addr)
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("authorization", "Bearer key".parse().unwrap());
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        socket.send(Message::Text(json!({"type":"response.create","model":"m","input":[{"role":"user","content":"hello"}]}).to_string().into())).await.unwrap();
        let failure = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Message::Text(text) = socket.next().await.unwrap().unwrap() {
                    let event: Value = serde_json::from_str(&text).unwrap();
                    if event["type"] == "error" {
                        break event;
                    }
                    assert_ne!(event["type"], "response.completed");
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(failure["status"], 502);
        assert_eq!(
            failure["error"]["code"],
            if broken {
                "upstream_stream_error"
            } else {
                "incomplete_upstream_stream"
            }
        );
        assert!(!failure
            .to_string()
            .contains("stream disconnected before completion:"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        shutdown(handle, task, path).await;
        assert!(!host.reports.lock().unwrap()[0].success);
    }
}

#[tokio::test]
async fn responses_terminal_error_keeps_public_details_and_drops_duplicate_terminal() {
    for codex in [false, true] {
        let payload = frame(
            json!({"type":"response.created","sequence_number":0,"response":{"id":"failed","status":"in_progress"}}),
        ) + &frame(
            json!({"type":"response.output_text.delta","sequence_number":1,"delta":"partial"}),
        ) + &frame(
            json!({"type":"response.failed","sequence_number":6,"status":429,"retry_after":2,"response":{"id":"failed","status":"failed","error":{"type":"rate_limit_error","code":"rate_limit_exceeded","message":"retry later Bearer private-key", "param":"model", "retryable":true,"metadata":{"access_token":"private-token"}}}}),
        ) + completed();
        let app = Router::new().route(
            "/v1/responses",
            post(move || {
                let payload = payload.clone();
                async move { ([("content-type", "text/event-stream")], payload) }
            }),
        );
        let (handle, _, task, path) = start(app, ExecutionPolicy::default()).await;
        let response = client()
            .post(format!("http://{}/v1/responses", handle.addr))
            .bearer_auth("key")
            .header(
                "user-agent",
                if codex {
                    "Codex Desktop/1"
                } else {
                    "third-party/1"
                },
            )
            .json(&json!({"model":"m","stream":true}))
            .send()
            .await
            .unwrap();
        let text = response.text().await.unwrap();
        let output = events(&text);
        assert_eq!(output.len(), 3, "{text}");
        let terminal = &output[2];
        let detail = if codex {
            &terminal["response"]["error"]
        } else {
            &terminal["error"]
        };
        assert_eq!(terminal["sequence_number"], 6);
        assert_eq!(detail["param"], "model");
        assert_eq!(detail["retryable"], true);
        assert_eq!(detail["code"], "rate_limit_exceeded");
        assert!(!text.contains("private-"));
        assert!(!text.contains("response.completed"));
        shutdown(handle, task, path).await;
    }
}

#[tokio::test]
async fn responses_immediate_open_error_preserves_retry_header_and_codex_terminal_shape() {
    let app = Router::new().route("/v1/responses",post(|| async {
        (StatusCode::BAD_REQUEST,[("retry-after","3")],axum::Json(json!({"error":{"code":"invalid_input","type":"invalid_request_error","message":"bad input", "param":"input", "retryable":false}})))
    }));
    let (handle, _, task, path) = start(
        app,
        ExecutionPolicy {
            immediate_sse: true,
            ..Default::default()
        },
    )
    .await;
    let response = client()
        .post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("key")
        .header("originator", "codex-tui")
        .json(&json!({"model":"m","stream":true}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let text = response.text().await.unwrap();
    let output = events(&text);
    assert_eq!(output.len(), 1, "{text}");
    assert_eq!(output[0]["type"], "response.failed");
    assert_eq!(output[0]["response"]["error"]["param"], "input");
    assert_eq!(output[0]["response"]["error"]["retryable"], false);
    assert_eq!(output[0]["headers"]["retry-after"], "3");
    shutdown(handle, task, path).await;
}

#[tokio::test]
async fn responses_json_and_stream_complete_with_usage_defaults_and_safe_headers() {
    for stream in [false, true] {
        let app = Router::new().route("/v1/responses",post(move || async move {
            let value = json!({"id":"r","object":"response","status":"completed","output":[],"usage":{"input_tokens":7,"output_tokens":4,"extension":true}});
            ([("content-type",if stream {"text/event-stream"}else{"application/json"}),("x-request-id","safe-id"),("x-cpa-trace-id","private-trace"),("access-control-allow-origin","https://untrusted.invalid")],
             if stream { frame(json!({"type":"response.completed","response":value}))+"data: [DONE]\n\n" } else { value.to_string() })
        }));
        let (handle, host, task, path) = start(app, ExecutionPolicy::default()).await;
        let response = client()
            .post(format!("http://{}/v1/responses", handle.addr))
            .bearer_auth("key")
            .json(&json!({"model":"m","stream":stream}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.headers()["x-request-id"], "safe-id");
        assert!(!response.headers().contains_key("x-cpa-trace-id"));
        assert_ne!(
            response
                .headers()
                .get("access-control-allow-origin")
                .and_then(|v| v.to_str().ok()),
            Some("https://untrusted.invalid")
        );
        let text = response.text().await.unwrap();
        let value = if stream {
            let output = events(&text);
            assert_eq!(output.len(), 1, "{text}");
            assert!(!text.contains("[DONE]"));
            output[0]["response"].clone()
        } else {
            serde_json::from_str::<Value>(&text).unwrap()
        };
        assert_eq!(value["usage"]["input_tokens_details"]["cached_tokens"], 0);
        assert_eq!(
            value["usage"]["output_tokens_details"]["reasoning_tokens"],
            0
        );
        assert_eq!(value["usage"]["extension"], true);
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(host.reports.lock().unwrap()[0].success);
        shutdown(handle, task, path).await;
    }
}

#[tokio::test]
async fn responses_stream_idle_timeout_sends_failure_without_success_marker() {
    let app = Router::new().route("/v1/responses",post(|| async {
        let prefix = futures_util::stream::once(async { Ok::<_,std::io::Error>(Bytes::from(frame(json!({"type":"response.created","response":{"id":"idle","status":"in_progress"}})))) });
        ([ ("content-type","text/event-stream") ],Body::from_stream(prefix.chain(futures_util::stream::pending())))
    }));
    let (handle, _, task, path) = start(
        app,
        ExecutionPolicy {
            idle_timeout: Duration::from_millis(350),
            ..Default::default()
        },
    )
    .await;
    let response = client()
        .post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("key")
        .header("originator", "codex-tui")
        .json(&json!({"model":"m","stream":true}))
        .send()
        .await
        .unwrap();
    let text = response.text().await.unwrap();
    let output = events(&text);
    assert_eq!(
        output.last().unwrap()["response"]["error"]["code"],
        "upstream_idle_timeout",
        "{text}"
    );
    assert_eq!(output.last().unwrap()["status"], 504);
    assert!(!text.contains("response.completed"));
    shutdown(handle, task, path).await;
}

#[tokio::test]
async fn responses_clean_eof_delivers_complete_tail_before_failure() {
    let app = Router::new().route("/v1/responses", post(|| async {
        let prefix = futures_util::stream::once(async { Ok::<_,std::io::Error>(Bytes::from(frame(json!({"type":"response.created","response":{"id":"tail","status":"in_progress"}})))) });
        let tail = futures_util::stream::once(async {
            tokio::time::sleep(Duration::from_millis(350)).await;
            Ok::<_,std::io::Error>(Bytes::from_static(b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"tail text\"}"))
        });
        ([("content-type","text/event-stream")],Body::from_stream(prefix.chain(tail)))
    }));
    let (handle, _, task, path) = start(app, ExecutionPolicy::default()).await;
    let text = client()
        .post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("key")
        .json(&json!({"model":"m","stream":true}))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let output = events(&text);
    assert_eq!(output.len(), 3, "{text}");
    assert_eq!(output[1]["delta"], "tail text");
    assert_eq!(output[2]["type"], "error");
    shutdown(handle, task, path).await;
}

#[tokio::test]
async fn responses_nonstream_failure_keeps_error_details_and_never_reports_success() {
    for sse in [false, true] {
        let response = json!({"id":"failed-json","status":"failed","output":[],"error":{"type":"invalid_request_error","code":"context_length_exceeded","message":"too long","param":"input","retryable":false}});
        let payload = if sse {
            frame(json!({"type":"response.failed","response":response}))
        } else {
            response.to_string()
        };
        let app = Router::new().route(
            "/v1/responses",
            post(move || {
                let payload = payload.clone();
                async move {
                    (
                        [(
                            "content-type",
                            if sse {
                                "text/event-stream"
                            } else {
                                "application/json"
                            },
                        )],
                        payload,
                    )
                }
            }),
        );
        let (handle, host, task, path) = start(app, ExecutionPolicy::default()).await;
        let response = client()
            .post(format!("http://{}/v1/responses", handle.addr))
            .bearer_auth("key")
            .json(&json!({"model":"m","stream":false}))
            .send()
            .await
            .unwrap();
        assert!(!response.status().is_success());
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"]["code"], "context_length_exceeded");
        assert_eq!(body["error"]["param"], "input");
        assert_eq!(body["error"]["retryable"], false);
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!host.reports.lock().unwrap()[0].success);
        shutdown(handle, task, path).await;
    }
}
