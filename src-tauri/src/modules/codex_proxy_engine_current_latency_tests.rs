use super::*;

async fn mock(
    steps: Vec<(&'static str, &'static str, Value)>,
) -> (
    SelectionReader,
    EngineController,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let controller = EngineController {
        endpoint: endpoint.clone(),
        secret: "test-secret".into(),
        tunnel_id: uuid::Uuid::new_v4(),
    };
    let reader = SelectionReader {
        endpoint,
        secret: "test-secret".into(),
        names: BTreeMap::from([
            ("leaf-a".into(), "US".into()),
            ("leaf-b".into(), "JP".into()),
        ]),
        groups: BTreeMap::from([(
            "account-node".into(),
            vec!["leaf-a".into(), "leaf-b".into()],
        )]),
        measured: Default::default(),
    };
    let server = tokio::spawn(async move {
        for (path, status, body) in steps {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            while !bytes.ends_with(b"\r\n\r\n") {
                bytes.push(socket.read_u8().await.unwrap());
                assert!(bytes.len() < 8192);
            }
            let request = String::from_utf8(bytes).unwrap();
            assert!(request.starts_with(&format!("GET {path}")), "{request}");
            assert!(request
                .to_lowercase()
                .contains("authorization: bearer test-secret"));
            let body = body.to_string();
            socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
    });
    (reader, controller, server)
}

fn group(node: &str) -> Value {
    json!({"type":"URLTest", "now":node,"testUrl":"https://latency.invalid/check"})
}

#[tokio::test]
async fn active_latency_checks_leaf_and_refreshes_shared_status_cache() {
    let (reader, controller, server) = mock(vec![
        ("/proxies/account-node ", "200 OK", group("leaf-a")),
        ("/proxies/leaf-a/delay?", "200 OK", json!({"delay":213})),
        ("/proxies/account-node ", "200 OK", group("leaf-a")),
        ("/proxies/account-node ", "200 OK", group("leaf-a")),
        (
            "/proxies/leaf-a ",
            "200 OK",
            json!({"alive":true,"history":[{"time":"2020-01-01T00:00:00Z", "delay":900}]}),
        ),
    ])
    .await;
    let other_consumer = reader.clone();
    let (target, result) = reader.measure_current_delay(&controller).await.unwrap();
    assert_eq!(result, Ok(213));
    reader.record_latency(target, result.ok());
    let status = other_consumer.selected_info().await.unwrap().unwrap();
    assert_eq!(status.name, "US");
    assert_eq!(status.delay_ms, Some(213));
    assert!(status.checked_at.unwrap() > 1_600_000_000_000);
    server.await.unwrap();
}

#[tokio::test]
async fn active_latency_rejects_changed_leaf_instead_of_publishing_stale_result() {
    let (reader, controller, server) = mock(vec![
        ("/proxies/account-node ", "200 OK", group("leaf-a")),
        ("/proxies/leaf-a/delay?", "200 OK", json!({"delay":213})),
        ("/proxies/account-node ", "200 OK", group("leaf-b")),
    ])
    .await;
    assert_eq!(
        reader
            .measure_current_delay(&controller)
            .await
            .err()
            .as_deref(),
        Some("PROXY_BINDING_CHANGED")
    );
    assert!(reader.measured.lock().unwrap().is_empty());
    server.await.unwrap();
}

#[tokio::test]
async fn failed_active_latency_suppresses_old_success_and_keeps_check_time() {
    let (reader, controller, server) = mock(vec![
        ("/proxies/account-node ", "200 OK", group("leaf-a")),
        (
            "/proxies/leaf-a/delay?",
            "504 Gateway Timeout",
            json!({"message":"private"}),
        ),
        ("/proxies/account-node ", "200 OK", group("leaf-a")),
        ("/proxies/account-node ", "200 OK", group("leaf-a")),
        (
            "/proxies/leaf-a ",
            "200 OK",
            json!({"alive":true,"history":[{"time":"2020-01-01T00:00:00Z", "delay":90}]}),
        ),
    ])
    .await;
    let (target, result) = reader.measure_current_delay(&controller).await.unwrap();
    assert_eq!(result, Err("PROXY_PROBE_TIMEOUT".into()));
    reader.record_latency(target, result.ok());
    let status = reader.selected_info().await.unwrap().unwrap();
    assert_eq!(status.delay_ms, None);
    assert!(status.checked_at.is_some());
    server.await.unwrap();
}

#[tokio::test]
async fn active_latency_does_not_probe_a_load_balancer_without_unique_exit() {
    let (reader, controller, server) = mock(vec![(
        "/proxies/account-node ",
        "200 OK",
        json!({"type":"LoadBalance", "now":"leaf-a"}),
    )])
    .await;
    assert_eq!(
        reader
            .measure_current_delay(&controller)
            .await
            .err()
            .as_deref(),
        Some("PROXY_LATENCY_NOT_RUNNING")
    );
    server.await.unwrap();
}

#[test]
fn current_latency_cache_is_scoped_to_reader_leaf_and_test_url_and_latest_observation() {
    let reader = SelectionReader {
        endpoint: String::new(),
        secret: String::new(),
        names: BTreeMap::from([("leaf-a".into(), "US".into())]),
        groups: BTreeMap::new(),
        measured: Default::default(),
    };
    let target = ("leaf-a".into(), "https://latency.invalid/check".into());
    reader.record_latency(target.clone(), Some(200));
    let history = ProxySelection {
        name: "US".into(),
        delay_ms: Some(900),
        checked_at: Some(1),
    };
    assert_eq!(
        reader.latest_latency(&target, history.clone()).delay_ms,
        Some(200)
    );
    assert_eq!(
        reader
            .latest_latency(&("leaf-b".into(), target.1.clone()), history.clone())
            .delay_ms,
        Some(900)
    );
    assert_eq!(
        reader
            .latest_latency(
                &(target.0.clone(), "https://another.invalid".into()),
                history.clone()
            )
            .delay_ms,
        Some(900)
    );
    let restarted = SelectionReader {
        measured: Default::default(),
        ..reader.clone()
    };
    assert_eq!(
        restarted.latest_latency(&target, history.clone()).delay_ms,
        Some(900)
    );
    let newer = ProxySelection {
        checked_at: Some(i64::MAX),
        ..history
    };
    assert_eq!(reader.latest_latency(&target, newer).delay_ms, Some(900));
    reader.record_latency(target.clone(), None);
    assert_eq!(
        reader
            .latest_latency(
                &target,
                ProxySelection {
                    name: "US".into(),
                    delay_ms: Some(900),
                    checked_at: Some(1)
                }
            )
            .delay_ms,
        None
    );
}
