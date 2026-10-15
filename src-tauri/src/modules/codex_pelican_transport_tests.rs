use super::*;

#[test]
fn pelican_request_carries_codex_client_metadata_and_prompt_cache() {
    let (body, headers) =
        build_pelican_request("gpt-5.5", "high", "hi", "acct-1").expect("build request");
    let value: Value = serde_json::from_slice(&body).expect("parse body");
    assert_eq!(value.get("store"), Some(&Value::Bool(false)));
    assert_eq!(value.get("stream"), Some(&Value::Bool(true)));
    let metadata = value
        .get("client_metadata")
        .and_then(Value::as_object)
        .expect("client metadata");
    for key in [
        "x-codex-installation-id",
        "x-codex-window-id",
        "x-codex-turn-metadata",
    ] {
        assert!(
            metadata
                .get(key)
                .and_then(Value::as_str)
                .is_some_and(|text| !text.is_empty()),
            "{key} should be present"
        );
    }
    let cache_key = value
        .get("prompt_cache_key")
        .and_then(Value::as_str)
        .expect("prompt cache key");
    for header in ["session-id", "conversation_id", "x-client-request-id"] {
        assert_eq!(headers.get(header).map(String::as_str), Some(cache_key));
    }
    // 同一账号必须稳定，prompt cache 命中依赖这一点。
    let (again, _) = build_pelican_request("gpt-5.5", "high", "hi", "acct-1").expect("build again");
    assert_eq!(body, again);
}

#[test]
fn oauth_errors_do_not_leak_account_credentials() {
    let account = CodexAccount::new(
        "test-account".into(),
        "test@example.invalid".into(),
        crate::models::codex::CodexTokens {
            access_token: "secret-access".into(),
            id_token: "secret-id".into(),
            refresh_token: Some("secret-refresh".into()),
        },
    );
    assert_eq!(
        pelican_redact_error(&account, "secret-access secret-id secret-refresh"),
        "[redacted] [redacted] [redacted]"
    );
}

#[test]
fn decodes_split_utf8_crlf_and_preserves_html_without_trimming() {
    let text = "  <html>鹈鹕</html>\n";
    let events = format!(
        "data: {}\r\n\r\ndata: {}\r\n\r\n",
        json!({"type":"response.output_text.delta","delta":text}),
        json!({"type":"response.completed","response":{"id":"r1","model":"test-model","status":"completed","usage":{"total_tokens":12},"output":[{"type":"message","content":[{"type":"output_text","text":text}]}]}})
    );
    let collected = std::sync::Mutex::new(String::new());
    let on_delta = |delta: String| collected.lock().unwrap().push_str(&delta);
    let mut decoder = PelicanSseDecoder::default();
    for byte in events.as_bytes() {
        decoder.push(&[*byte], &on_delta).unwrap();
    }
    let result = decoder.finish(&on_delta).unwrap();
    assert_eq!(result.reply, text);
    assert_eq!(*collected.lock().unwrap(), text);
    assert_eq!(result.response_id.as_deref(), Some("r1"));
    assert_eq!(result.response_model.as_deref(), Some("test-model"));
    assert_eq!(result.usage, Some(json!({"total_tokens":12})));
}

#[test]
fn eof_done_and_incomplete_are_not_success() {
    for events in [
        "data: [DONE]\n\n",
        "data: {\"type\":\"response.incomplete\"}\n\n",
        "data: {\"type\":\"error\"}\n\n",
    ] {
        let mut decoder = PelicanSseDecoder::default();
        let result = decoder
            .push(events.as_bytes(), &|_| {})
            .and_then(|_| decoder.finish(&|_| {}).map(|_| ()));
        assert!(result
            .unwrap_err()
            .starts_with("PELICAN_STREAM_INCOMPLETE: "));
    }
}

#[test]
fn completion_without_delta_and_final_newline_is_supported() {
    let mut decoder = PelicanSseDecoder::default();
    let event = format!(
        "data: {}",
        json!({"type":"response.completed","response":{"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"<svg/>"}]}]}})
    );
    decoder.push(event.as_bytes(), &|_| {}).unwrap();
    assert_eq!(decoder.finish(&|_| {}).unwrap().reply, "<svg/>");
}

#[test]
fn bounds_response_before_buffering_and_rejects_malformed_events() {
    let mut decoder = PelicanSseDecoder {
        received: PELICAN_MAX_RESPONSE_BYTES,
        ..Default::default()
    };
    assert_eq!(
        decoder.push(b"x", &|_| {}).unwrap_err(),
        "PELICAN_RESPONSE_TOO_LARGE"
    );
    assert!(decoder.pending.is_empty());
    assert!(PelicanSseDecoder::default()
        .push(b"data: {oops}\n\n", &|_| {})
        .is_err());
}

#[test]
fn long_split_line_and_many_events_keep_a_linear_read_cursor() {
    let mut decoder = PelicanSseDecoder::default();
    // A large comment split across reads must not scan or copy the old prefix again.
    for _ in 0..256 {
        decoder.push(&vec![b'x'; 1024], &|_| {}).unwrap();
        assert_eq!(decoder.scanned, decoder.pending.len());
    }
    decoder.push(b"\n\n", &|_| {}).unwrap();
    assert!(decoder.pending.is_empty());
    let line = b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"x\"}\n\n";
    decoder.push(&line.repeat(4096), &|_| {}).unwrap();
    assert_eq!(decoder.reply.len(), 4096);
    assert!(decoder.pending.is_empty());
}

#[tokio::test]
async fn pre_cancel_does_not_prepare_accounts_or_send_requests() {
    let (_tx, rx) = watch::channel(true);
    let result = run_pelican_chat("nonexistent", "test-model", "medium", "test", rx, |_| {
        panic!("no output expected")
    })
    .await;
    assert!(matches!(result, Err(error) if error == "PELICAN_CANCELLED"));
}

async fn open_mock_stream() -> (reqwest::Response, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0u8; 4096];
        stream.read(&mut request).await.unwrap();
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
        let event = format!(
            "data: {}\n\n",
            json!({"type":"response.output_text.delta","delta":"<html>partial"})
        );
        stream
            .write_all(format!("{:x}\r\n{}\r\n", event.len(), event).as_bytes())
            .await
            .unwrap();
        // Peer EOF proves cancellation released the in-flight connection.
        let read = timeout(Duration::from_secs(2), stream.read(&mut request))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(read, 0, "cancel/timeout must close the in-flight stream");
    });
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(format!("http://{address}/"))
        .send()
        .await
        .unwrap();
    (response, server)
}

#[tokio::test]
async fn cancellation_drops_live_stream_and_preserves_received_delta() {
    let (response, server) = open_mock_stream().await;
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let collected = std::sync::Mutex::new(String::new());
    let on_delta = |delta: String| {
        collected.lock().unwrap().push_str(&delta);
        cancel_tx.send_replace(true);
    };
    let result = pelican_with_cancel(
        cancel_rx,
        Duration::from_secs(2),
        pelican_consume_response(response, Duration::from_secs(2), &on_delta),
    )
    .await;
    assert!(matches!(result, Err(error) if error == "PELICAN_CANCELLED"));
    assert_eq!(*collected.lock().unwrap(), "<html>partial");
    server.await.unwrap();
}

#[tokio::test]
async fn idle_timeout_does_not_report_a_partial_document_as_complete() {
    let (response, server) = open_mock_stream().await;
    let collected = std::sync::Mutex::new(String::new());
    let result = pelican_consume_response(response, Duration::from_millis(30), &|delta| {
        collected.lock().unwrap().push_str(&delta);
    })
    .await;
    assert!(
        matches!(result, Err(error) if error.starts_with("PELICAN_TIMEOUT: stage=waiting for SSE data;"))
    );
    assert_eq!(*collected.lock().unwrap(), "<html>partial");
    server.await.unwrap();
}

#[tokio::test]
async fn total_timeout_stops_a_nonterminating_operation() {
    let (_tx, rx) = watch::channel(false);
    let result =
        pelican_with_cancel::<()>(rx, Duration::from_millis(10), std::future::pending()).await;
    assert_eq!(
        result.unwrap_err(),
        "PELICAN_TIMEOUT: stage=total operation deadline"
    );
}

#[test]
fn parses_primary_rate_limit_headers() {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "x-codex-primary-used-percent",
        reqwest::header::HeaderValue::from_static("42.5"),
    );
    headers.insert(
        "x-codex-primary-window-minutes",
        reqwest::header::HeaderValue::from_static("300"),
    );
    headers.insert(
        "x-codex-primary-reset-after-seconds",
        reqwest::header::HeaderValue::from_static("120"),
    );
    let snapshot = pelican_quota_snapshot_from_headers(&headers).expect("quota snapshot");
    assert_eq!(snapshot.used_percent, 42.5);
    assert_eq!(snapshot.remaining_percent, 58);
    assert_eq!(snapshot.window_minutes, Some(300));
    assert!(snapshot.reset_at.is_some());
}

#[test]
fn ignores_missing_primary_rate_limit_headers() {
    assert!(pelican_quota_snapshot_from_headers(&reqwest::header::HeaderMap::new()).is_none());
}

#[test]
fn error_events_preserve_only_diagnostic_fields_and_partial_output() {
    for (event, expected) in [
        (
            json!({"type":"response.failed","response":{"status":"failed","error":{"message":"quota unavailable","code":"quota_exceeded","type":"limit_error"},"output":[{"text":"not diagnostic"}]}}),
            vec![
                "event=response.failed",
                "status=failed",
                "message=quota unavailable",
                "code=quota_exceeded",
                "type=limit_error",
            ],
        ),
        (
            json!({"type":"error","error":{"message":"account disabled","code":"account_disabled","type":"auth_error"}}),
            vec![
                "event=error",
                "message=account disabled",
                "code=account_disabled",
                "type=auth_error",
            ],
        ),
        (
            json!({"type":"error","message":"invalid request","code":"invalid_request"}),
            vec!["message=invalid request", "code=invalid_request"],
        ),
        (
            json!({"type":"error","error":"upstream disconnected"}),
            vec!["message=upstream disconnected"],
        ),
        (
            json!({"type":"response.incomplete","response":{"incomplete_details":{"reason":"max_output_tokens"}}}),
            vec!["event=response.incomplete", "reason=max_output_tokens"],
        ),
        (
            json!({"type":"response.completed","response":{"status":"failed","error":{"message":"failed during generation","code":"generation_error"}}}),
            vec![
                "status=failed",
                "message=failed during generation",
                "code=generation_error",
            ],
        ),
    ] {
        let collected = std::sync::Mutex::new(String::new());
        let on_delta = |delta: String| collected.lock().unwrap().push_str(&delta);
        let mut decoder = PelicanSseDecoder::default();
        decoder
            .push(
                b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n",
                &on_delta,
            )
            .unwrap();
        let error = decoder
            .push(format!("data: {event}\n\n").as_bytes(), &on_delta)
            .unwrap_err();
        for field in expected {
            assert!(error.contains(field), "expected {field} in {error}");
        }
        assert!(!error.contains("not diagnostic"));
        assert!(!error.contains("partial"));
        assert_eq!(*collected.lock().unwrap(), "partial");
    }
}

#[test]
fn missing_completion_malformed_data_and_empty_output_have_distinct_diagnostics() {
    for (bytes, expected) in [
        (
            b"data: [DONE]\n\n".as_slice(),
            "stream ended before response.completed",
        ),
        (b"data: {oops}\n\n".as_slice(), "invalid JSON in SSE event"),
        (b"data: \xff\n\n".as_slice(), "invalid UTF-8 in SSE line"),
        (
            b"data: {\"type\":\"response.completed\"}\n\n".as_slice(),
            "missing response object",
        ),
        (
            b"data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n"
                .as_slice(),
            "contained no output text",
        ),
        (
            b"data: {\"type\":\"response.done\",\"response\":{\"status\":\"completed\"}}\n\n"
                .as_slice(),
            "stream ended before response.completed",
        ),
    ] {
        let mut decoder = PelicanSseDecoder::default();
        let result = decoder
            .push(bytes, &|_| {})
            .and_then(|_| decoder.finish(&|_| {}).map(|_| ()));
        let error = result.unwrap_err();
        assert!(error.contains(expected), "{error}");
        assert!(error.contains("received_bytes="));
    }
}

#[test]
fn diagnostics_redact_before_truncation_including_multibyte_boundary() {
    let account = CodexAccount::new(
        "test-account".into(),
        "test@example.invalid".into(),
        crate::models::codex::CodexTokens {
            access_token: "access-secret-straddling-boundary".into(),
            id_token: "id-secret".into(),
            refresh_token: Some("refresh-secret".into()),
        },
    );
    let raw = format!(
        "{}{} {} {}",
        "界".repeat(1198),
        account.tokens.access_token,
        account.tokens.id_token,
        account.tokens.refresh_token.as_deref().unwrap()
    );
    let safe = pelican_safe_error(&account, &raw);
    assert!(safe.chars().count() <= 1203);
    assert!(
        !safe.contains("ac"),
        "truncated token prefix must not survive"
    );
    let safe = pelican_safe_error(
        &account,
        "PELICAN_STREAM_INCOMPLETE: access-secret-straddling-boundary id-secret refresh-secret",
    );
    assert_eq!(safe.matches("[redacted]").count(), 3);
}

#[tokio::test]
async fn broken_http_stream_preserves_source_error_and_received_text() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (close_tx, close_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0u8; 4096];
        stream.read(&mut request).await.unwrap();
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
        let event =
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial text\"}\n\n";
        stream
            .write_all(format!("{:x}\r\n{}\r\n", event.len(), event).as_bytes())
            .await
            .unwrap();
        close_rx.await.unwrap();
        // Deliberately omit the HTTP chunked terminator to force a real body-read error.
        stream.shutdown().await.unwrap();
    });
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(format!("http://{address}/private?token=do-not-log"))
        .send()
        .await
        .unwrap();
    let close_tx = std::sync::Mutex::new(Some(close_tx));
    let collected = std::sync::Mutex::new(String::new());
    let result = pelican_consume_response(response, Duration::from_secs(2), &|delta| {
        collected.lock().unwrap().push_str(&delta);
        if let Some(tx) = close_tx.lock().unwrap().take() {
            tx.send(()).unwrap();
        }
    })
    .await;
    let error = result.err().expect("broken stream must fail");
    assert!(error.contains("stage=reading SSE data"), "{error}");
    assert!(error.contains("caused by:"), "{error}");
    assert!(!error.contains("do-not-log"));
    assert!(!error.contains(&address.to_string()));
    assert_eq!(*collected.lock().unwrap(), "partial text");
    server.await.unwrap();
}

#[test]
fn untyped_and_named_sse_errors_preserve_diagnostics_without_inventing_success() {
    for events in [
        "data: {\"error\":{\"message\":\"upstream rejected\",\"code\":\"denied\"}}\n\n",
        "event: error\ndata: {\"message\":\"upstream rejected\",\"code\":\"denied\"}\n\n",
        "data: {\"type\":\"response.done\",\"response\":{\"error\":{\"message\":\"upstream rejected\",\"code\":\"denied\"}}}\n\n",
    ] {
        let mut decoder = PelicanSseDecoder::default();
        let error = decoder.push(events.as_bytes(), &|_| {}).unwrap_err();
        assert!(error.contains("message=upstream rejected"), "{error}");
        assert!(error.contains("code=denied"), "{error}");
    }
}

#[test]
fn completed_response_empty_metadata_keeps_existing_success_semantics() {
    let mut decoder = PelicanSseDecoder::default();
    let event = json!({"type":"response.completed","response":{
        "status":"completed", "error":{}, "incomplete_details":{},
        "output":[{"type":"message","content":[{"type":"output_text","text":"<html/>"}]}]
    }});
    decoder
        .push(format!("data: {event}\n\n").as_bytes(), &|_| {})
        .unwrap();
    assert_eq!(decoder.finish(&|_| {}).unwrap().reply, "<html/>");
}
