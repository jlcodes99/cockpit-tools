mod request_diagnostics_poll_tests {
    use super::super::*;

    struct FakeDiagnosticResponse {
        body: Vec<u8>,
        status: u16,
        body_delay: Duration,
        chunked: bool,
        advertised_length: Option<usize>,
    }

    impl FakeDiagnosticResponse {
        fn json(value: Value) -> Self {
            Self {
                body: serde_json::to_vec(&value).unwrap(),
                status: 200,
                body_delay: Duration::ZERO,
                chunked: false,
                advertised_length: None,
            }
        }
    }

    async fn fake_diagnostic_server(
        responses: Vec<FakeDiagnosticResponse>,
    ) -> (u16, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for response in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    let mut bytes = [0_u8; 1024];
                    let count = socket.read(&mut bytes).await.unwrap();
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&bytes[..count]);
                }
                requests.push(String::from_utf8_lossy(&request).into_owned());
                let transfer = if response.chunked {
                    "Transfer-Encoding: chunked\r\n".to_string()
                } else {
                    format!(
                        "Content-Length: {}\r\n",
                        response.advertised_length.unwrap_or(response.body.len())
                    )
                };
                let headers = format!("HTTP/1.1 {} OK\r\nContent-Type: application/json\r\nConnection: close\r\n{}\r\n", response.status, transfer);
                if socket.write_all(headers.as_bytes()).await.is_err() {
                    continue;
                }
                tokio::time::sleep(response.body_delay).await;
                let body = if response.chunked {
                    let mut chunk = format!("{:x}\r\n", response.body.len()).into_bytes();
                    chunk.extend_from_slice(&response.body);
                    chunk.extend_from_slice(b"\r\n0\r\n\r\n");
                    chunk
                } else {
                    response.body
                };
                // A limit/cancel/timeout test deliberately closes its client early.
                let _ = socket.write_all(&body).await;
            }
            requests
        });
        (port, task)
    }

    fn fake_detail(request_id: &str) -> Value {
        json!({"requestId":request_id,"firstResponseMs":17,"capturedAtMs":now_ms(),
            "attempts":[{"sequence":1,"accountId":"a","modelId":"gpt-5.5","transport":"sse",
                "startedAtMs":now_ms()-100,"latencyMs":100,"status":429,"success":false,
                "errorCategory":"usage_limit_reached","failurePhase":"first_response"},
                {"sequence":2,"accountId":"b","modelId":"gpt-5.5","transport":"websocket",
                    "startedAtMs":now_ms()-50,"latencyMs":50,"status":200,"success":true}],
            "payloads":[{"stage":"client","transport":"http","contentType":"application/json",
                "headers":{"content-type":"application/json"},"body":"{\"model\":\"gpt-5.5\"}",
                "originalBytes":19,"sha256":"redacted-hash","truncated":false}],"truncated":false})
    }

    #[tokio::test]
    async fn request_diagnostics_pull_uses_private_auth_and_drains_multiple_rounds() {
        let (port, server) = fake_diagnostic_server(vec![
            FakeDiagnosticResponse::json(json!({"events":[fake_detail("first")],"dropped":2})),
            FakeDiagnosticResponse::json(json!({"events":[fake_detail("second")]})),
            FakeDiagnosticResponse::json(json!({"events":[]})),
        ])
        .await;
        let client = build_localhost_http_client(REQUEST_DIAGNOSTICS_POLL_TIMEOUT, "test").unwrap();
        let mut ids = Vec::new();
        for index in 0..3 {
            let batch =
                pull_request_diagnostics_from_port(&client, port, REQUEST_DIAGNOSTICS_POLL_TIMEOUT)
                    .await
                    .unwrap();
            if index == 0 {
                assert_eq!(batch.dropped, 2);
                assert_eq!(batch.events[0].attempts.len(), 2);
                assert_eq!(
                    batch.events[0].attempts[0].failure_phase.as_deref(),
                    Some("first_response")
                );
                assert_eq!(
                    batch.events[0].payloads[0]
                        .headers
                        .get("content-type")
                        .map(String::as_str),
                    Some("application/json")
                );
            }
            ids.extend(batch.events.into_iter().map(|detail| detail.request_id));
        }
        assert_eq!(ids, vec!["first", "second"]);
        for request in server.await.unwrap() {
            assert!(request.starts_with("GET /v1/cockpit/diagnostics/events HTTP/1.1\r\n"));
            assert!(request
                .lines()
                .any(|line| line.eq_ignore_ascii_case(&format!(
                    "authorization: Bearer {}",
                    internal_api_service_key()
                ))));
            assert!(!request.contains("{\"events\""));
        }
    }

    #[tokio::test]
    async fn request_diagnostics_pull_body_capture_off_still_preserves_attempts_and_first_response()
    {
        let mut metadata = fake_detail("body-off");
        metadata["payloads"] = Value::Null;
        let (port, server) = fake_diagnostic_server(vec![FakeDiagnosticResponse::json(
            json!({"events":[metadata]}),
        )])
        .await;
        let client = build_localhost_http_client(REQUEST_DIAGNOSTICS_POLL_TIMEOUT, "test").unwrap();
        let batch =
            pull_request_diagnostics_from_port(&client, port, REQUEST_DIAGNOSTICS_POLL_TIMEOUT)
                .await
                .unwrap();
        assert!(batch.events[0].payloads.is_empty());
        assert_eq!(batch.events[0].attempts.len(), 2);
        assert_eq!(batch.events[0].first_response_ms, Some(17));
        server.await.unwrap();
        let legacy:CodexLocalAccessRequestDetail=serde_json::from_value(json!({"requestId":"validation","attempts":null,"payloads":null,"failurePhase":"request_validation"})).unwrap();
        assert!(legacy.attempts.is_empty());
        assert!(legacy.payloads.is_empty());
    }

    #[tokio::test]
    async fn request_diagnostics_pull_limits_advertised_and_chunked_response_bytes() {
        let mut advertised = FakeDiagnosticResponse::json(json!({"events":[]}));
        advertised.advertised_length = Some(REQUEST_DIAGNOSTICS_RESPONSE_LIMIT + 1);
        let mut chunked = FakeDiagnosticResponse::json(json!({"events":[]}));
        chunked.chunked = true;
        chunked.body = vec![b' '; REQUEST_DIAGNOSTICS_RESPONSE_LIMIT + 1];
        let (port, server) = fake_diagnostic_server(vec![advertised, chunked]).await;
        let client = build_localhost_http_client(REQUEST_DIAGNOSTICS_POLL_TIMEOUT, "test").unwrap();
        for _ in 0..2 {
            let error =
                pull_request_diagnostics_from_port(&client, port, REQUEST_DIAGNOSTICS_POLL_TIMEOUT)
                    .await
                    .unwrap_err();
            assert!(error.contains("2 MiB"));
        }
        server.await.unwrap();
    }

    #[tokio::test]
    async fn request_diagnostics_pull_limits_event_count_and_does_not_echo_invalid_body() {
        let mut malformed = FakeDiagnosticResponse::json(json!({"events":[]}));
        malformed.body = b"{\"events\":[{\"firstResponseMs\":\"private-secret\"}]}".to_vec();
        let (port,server) = fake_diagnostic_server(vec![
            FakeDiagnosticResponse::json(json!({"events":(0..9).map(|index|fake_detail(&index.to_string())).collect::<Vec<_>>()})),
            malformed,
        ]).await;
        let client = build_localhost_http_client(REQUEST_DIAGNOSTICS_POLL_TIMEOUT, "test").unwrap();
        assert!(pull_request_diagnostics_from_port(
            &client,
            port,
            REQUEST_DIAGNOSTICS_POLL_TIMEOUT
        )
        .await
        .unwrap_err()
        .contains("8 条"));
        let error =
            pull_request_diagnostics_from_port(&client, port, REQUEST_DIAGNOSTICS_POLL_TIMEOUT)
                .await
                .unwrap_err();
        assert!(error.contains("格式无效"));
        assert!(!error.contains("private-secret"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn request_diagnostics_pull_bounds_total_body_time_and_cancels_closed_endpoint() {
        let mut delayed = FakeDiagnosticResponse::json(json!({"events":[fake_detail("late")]}));
        delayed.chunked = true;
        delayed.body_delay = Duration::from_millis(150);
        let (port, server) = fake_diagnostic_server(vec![delayed]).await;
        let client = build_localhost_http_client(REQUEST_DIAGNOSTICS_POLL_TIMEOUT, "test").unwrap();
        let started = Instant::now();
        let error = pull_request_diagnostics_from_port(&client, port, Duration::from_millis(40))
            .await
            .unwrap_err();
        assert!(error.contains("超时"));
        assert!(started.elapsed() < Duration::from_millis(500));
        server.await.unwrap();

        let mut delayed =
            FakeDiagnosticResponse::json(json!({"events":[fake_detail("cancelled")]}));
        delayed.body_delay = Duration::from_millis(150);
        let (port, server) = fake_diagnostic_server(vec![delayed]).await;
        let result = pull_request_diagnostics_with_cancel(
            &client,
            port,
            REQUEST_DIAGNOSTICS_POLL_TIMEOUT,
            tokio::time::sleep(Duration::from_millis(40)),
        )
        .await
        .unwrap();
        assert!(result.is_none());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn request_diagnostics_pull_http_errors_exclude_response_body() {
        let mut rejected = FakeDiagnosticResponse::json(json!({"error":"private-secret"}));
        rejected.status = 403;
        let (port, server) = fake_diagnostic_server(vec![rejected]).await;
        let client = build_localhost_http_client(REQUEST_DIAGNOSTICS_POLL_TIMEOUT, "test").unwrap();
        let error =
            pull_request_diagnostics_from_port(&client, port, REQUEST_DIAGNOSTICS_POLL_TIMEOUT)
                .await
                .unwrap_err();
        assert!(error.contains("403"));
        assert!(!error.contains("private-secret"));
        server.await.unwrap();
    }

    #[test]
    fn request_diagnostics_pull_stale_ready_payloads_and_disabled_capture_are_discarded() {
        let mut detail: CodexLocalAccessRequestDetail =
            serde_json::from_value(fake_detail("stale-ready")).unwrap();
        let captured_at = detail.captured_at_ms;
        prepare_request_diagnostic_payloads(&mut detail, true, captured_at);
        assert!(detail.payloads.is_empty());
        assert_eq!(detail.attempts.len(), 2);
        assert_eq!(detail.first_response_ms, Some(17));
        let mut detail: CodexLocalAccessRequestDetail =
            serde_json::from_value(fake_detail("disabled")).unwrap();
        prepare_request_diagnostic_payloads(&mut detail, false, 0);
        assert!(detail.payloads.is_empty());
    }

    #[test]
    fn request_diagnostics_pull_failed_endpoints_back_off_and_new_identity_recovers() {
        let now = Instant::now();
        let mut failure = RequestDiagnosticPollFailure::default();
        assert!(failure.should_poll(now));
        assert!(failure.fail(now));
        assert!(!failure.should_poll(now + Duration::from_millis(500)));
        assert!(failure.should_poll(now + Duration::from_secs(1)));
        for _ in 0..40 {
            failure.fail(now);
        }
        assert!(failure.should_poll(now + Duration::from_secs(30)));
        let before = RequestDiagnosticEndpoint {
            profile_runtime_key: Some("profile".into()),
            port: 1111,
            pid: 1,
            generation: None,
        };
        let after = RequestDiagnosticEndpoint {
            port: 2222,
            pid: 2,
            ..before.clone()
        };
        let failures = HashMap::from([(before, failure)]);
        assert!(!failures.contains_key(&after));
    }

    #[tokio::test]
    async fn request_diagnostics_stdout_compatibility_never_copies_body_into_startup_log() {
        let diagnostics = Arc::new(Mutex::new(SidecarStartupDiagnostics {
            last_stdout: Some("previous event".into()),
            ..Default::default()
        }));
        let mut detail = fake_detail("");
        detail["type"] = json!("request_diagnostics");
        detail["payloads"][0]["body"] = json!("private-secret");
        handle_sidecar_stdout_line(
            &serde_json::to_string(&detail).unwrap(),
            &mut None,
            &diagnostics,
        )
        .await;
        assert_eq!(
            diagnostics.lock().unwrap().last_stdout.as_deref(),
            Some("previous event")
        );
    }
}
