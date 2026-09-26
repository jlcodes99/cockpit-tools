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
    let (again, _) =
        build_pelican_request("gpt-5.5", "high", "hi", "acct-1").expect("build again");
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
        assert_eq!(result.unwrap_err(), "PELICAN_STREAM_INCOMPLETE");
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
    assert!(matches!(result, Err(error) if error == "PELICAN_TIMEOUT"));
    assert_eq!(*collected.lock().unwrap(), "<html>partial");
    server.await.unwrap();
}

#[tokio::test]
async fn total_timeout_stops_a_nonterminating_operation() {
    let (_tx, rx) = watch::channel(false);
    let result =
        pelican_with_cancel::<()>(rx, Duration::from_millis(10), std::future::pending()).await;
    assert_eq!(result.unwrap_err(), "PELICAN_TIMEOUT");
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
