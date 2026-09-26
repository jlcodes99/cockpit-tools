use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn headers(socket: &mut tokio::net::TcpStream) -> String {
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        bytes.push(socket.read_u8().await.unwrap());
        assert!(bytes.len() < 8192);
    }
    String::from_utf8(bytes).unwrap()
}

async fn mock_response(
    status: &str,
    body: String,
    extra_headers: &str,
) -> (EngineController, tokio::task::JoinHandle<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{extra_headers}\r\n{body}",
        body.len(),
    );
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = headers(&mut socket).await;
        socket.write_all(response.as_bytes()).await.unwrap();
        request
    });
    (
        EngineController {
            endpoint,
            secret: "local-controller-secret".into(),
            tunnel_id: uuid::Uuid::new_v4(),
        },
        server,
    )
}

#[tokio::test]
async fn native_delay_returns_engine_value_with_encoded_url_timeout_and_local_auth() {
    let (controller, server) = mock_response(
        "200 OK",
        r#"{"delay":1573,"password":"RESPONSE_SECRET"}"#.into(),
        "",
    )
    .await;
    let test_url = "https://latency.invalid/generate_204?a=one&token=query-secret%23value";
    let actual = controller.measure_delay(test_url).await.unwrap();
    assert_eq!(actual, 1573); // Not elapsed time of this immediate local response.
    let request = server.await.unwrap();
    let path = request
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    let request_url = url::Url::parse(&format!("http://localhost{path}")).unwrap();
    assert_eq!(request_url.path(), "/proxies/account-node/delay");
    let query: std::collections::BTreeMap<_, _> = request_url.query_pairs().into_owned().collect();
    assert_eq!(query.get("url").map(String::as_str), Some(test_url));
    assert_eq!(query.get("timeout").map(String::as_str), Some("8000"));
    assert_eq!(query.len(), 2);
    let lower = request.to_lowercase();
    assert!(lower.contains("authorization: bearer local-controller-secret\r\n"));
    assert!(
        !lower.contains("cookie:")
            && !lower.contains("chatgpt-account-id:")
            && !lower.contains("proxy-authorization:")
    );
}

#[tokio::test]
async fn native_delay_maps_failures_to_fixed_codes_without_remote_details() {
    for (status, expected) in [
        ("504 Gateway Timeout", "PROXY_PROBE_TIMEOUT"),
        ("408 Request Timeout", "PROXY_PROBE_TIMEOUT"),
        ("503 Service Unavailable", "PROXY_PROBE_FAILED"),
        ("401 Unauthorized", "PROXY_PROBE_FAILED"),
        ("200 OK", "PROXY_PROBE_RESPONSE"),
    ] {
        let (controller, server) = mock_response(
            status,
            r#"{"message":"RESPONSE_SECRET","url":"https://private.invalid/?secret=123"}"#.into(),
            "",
        )
        .await;
        let error = controller
            .measure_delay("http://latency.invalid")
            .await
            .unwrap_err();
        assert_eq!(error, expected);
        assert!(!error.contains("secret") && !error.contains("private.invalid"));
        server.await.unwrap();
    }
}

#[tokio::test]
async fn native_delay_rejects_invalid_or_unbounded_success_responses() {
    for body in [
        "not-json".into(),
        "null".into(),
        r#"{"delay":0}"#.into(),
        r#"{"delay":-1}"#.into(),
        r#"{"delay":1.5}"#.into(),
        r#"{"delay":"216"}"#.into(),
        r#"{"delay":65536}"#.into(),
        " ".repeat(RESPONSE_LIMIT + 1),
    ] {
        let (controller, server) = mock_response("200 OK", body, "").await;
        assert_eq!(
            controller
                .measure_delay("http://latency.invalid")
                .await
                .unwrap_err(),
            "PROXY_PROBE_RESPONSE"
        );
        server.await.unwrap();
    }
}

#[tokio::test]
async fn native_delay_does_not_follow_controller_redirects_or_leak_auth() {
    let redirect = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (controller, server) = mock_response(
        "302 Found",
        String::new(),
        &format!(
            "Location: http://{}/capture\r\n",
            redirect.local_addr().unwrap()
        ),
    )
    .await;
    assert_eq!(
        controller
            .measure_delay("http://latency.invalid")
            .await
            .unwrap_err(),
        "PROXY_PROBE_FAILED"
    );
    server.await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(100), redirect.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn native_delay_has_a_bounded_controller_deadline() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let controller = EngineController {
        endpoint: format!("http://{}", listener.local_addr().unwrap()),
        secret: "timeout-secret".into(),
        tunnel_id: uuid::Uuid::new_v4(),
    };
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = headers(&mut socket).await;
        assert!(request.contains("timeout=20"));
        tokio::time::sleep(Duration::from_secs(2)).await;
        drop(socket);
    });
    let started = std::time::Instant::now();
    assert_eq!(
        query_delay(
            &controller,
            "http://latency.invalid",
            Duration::from_millis(20)
        )
        .await
        .unwrap_err(),
        "PROXY_PROBE_TIMEOUT"
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    server.abort();
}

#[tokio::test]
async fn native_delay_rejects_credential_and_non_http_urls_before_controller_io() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let controller = EngineController {
        endpoint: format!("http://{}", listener.local_addr().unwrap()),
        secret: "must-not-be-sent".into(),
        tunnel_id: uuid::Uuid::new_v4(),
    };
    for value in [
        "https://user:password@latency.invalid".into(),
        "https://user@latency.invalid".into(),
        "https://:password@latency.invalid".into(),
        "ftp://latency.invalid".into(),
        "file:///etc/passwd".into(),
        "invalid".into(),
        format!("http://latency.invalid/{}", "x".repeat(4096)),
    ] {
        assert_eq!(
            controller.measure_delay(&value).await.unwrap_err(),
            "PROXY_INVALID_URL"
        );
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(100), listener.accept())
            .await
            .is_err()
    );
}

fn test_engine() -> std::path::PathBuf {
    std::env::var_os("COCKPIT_TEST_MIHOMO")
        .map(std::path::PathBuf::from)
        .expect("Set COCKPIT_TEST_MIHOMO to a checksum-verified Mihomo 1.19.31 executable")
}

#[tokio::test]
#[ignore = "requires explicitly provided, checksum-verified Mihomo; local mock proxy only"]
async fn real_engine_delay_uses_unified_head_checks_without_account_credentials() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let connect = headers(&mut socket).await.to_lowercase();
        assert!(connect.starts_with("connect latency.invalid:80 "));
        assert!(!connect.contains("authorization:") && !connect.contains("cookie:"));
        socket
            .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
            .await
            .unwrap();
        // Unified delay performs a second HEAD on this connection. The result
        // must be the engine's second measurement, not total host request time.
        for delay in [150, 30] {
            let request = headers(&mut socket).await.to_lowercase();
            assert!(request.starts_with("head /generate_204 "));
            for secret_header in ["authorization:", "cookie:", "chatgpt-account-id:"] {
                assert!(!request.contains(secret_header));
            }
            tokio::time::sleep(Duration::from_millis(delay)).await;
            socket.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: keep-alive\r\n\r\n").await.unwrap();
        }
    });
    let tunnel = super::super::start_with_binary_options(
        &test_engine(),
        serde_json::json!({"type":"http","server":"127.0.0.1","server_port":port}),
        false,
        None,
        Default::default(),
        true,
    )
    .await
    .unwrap();
    let controller = tunnel.controller();
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(1))
        .build()
        .unwrap();
    let endpoint = format!("{}/configs", controller.endpoint);
    assert_eq!(
        client.get(&endpoint).send().await.unwrap().status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let configuration: serde_json::Value = client
        .get(&endpoint)
        .bearer_auth(&controller.secret)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(configuration["unified-delay"], true);
    let started = std::time::Instant::now();
    let delay = controller
        .measure_delay("http://latency.invalid/generate_204")
        .await
        .unwrap();
    assert!(delay >= 25, "must preserve the native second HEAD delay");
    assert!(
        started.elapsed().as_millis() as u64 >= delay + 100,
        "must exclude the first request from the unified result"
    );
    server.await.unwrap();
    tunnel.stop().await.unwrap();
}

#[tokio::test]
#[ignore = "requires explicitly provided, checksum-verified Mihomo; local mock proxy only"]
async fn real_engine_delay_cancellation_drops_only_its_temporary_process() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (started, entered) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        headers(&mut socket).await;
        socket
            .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
            .await
            .unwrap();
        headers(&mut socket).await;
        let _ = started.send(());
        let mut byte = [0; 1];
        match socket.read(&mut byte).await {
            Ok(0) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
                ) => {}
            other => panic!("cancelled engine must close its mock proxy connection: {other:?}"),
        }
    });
    let outbound = serde_json::json!({"type":"http","server":"127.0.0.1","server_port":port});
    let idle_tunnel = super::super::start_with_binary_options(
        &test_engine(),
        outbound.clone(),
        false,
        None,
        Default::default(),
        false,
    )
    .await
    .unwrap();
    let probe_tunnel = super::super::start_with_binary_options(
        &test_engine(),
        outbound,
        false,
        None,
        Default::default(),
        true,
    )
    .await
    .unwrap();
    let child = probe_tunnel.child.clone();
    let id = probe_tunnel.id;
    let operation = tokio::spawn(async move {
        let result = probe_tunnel
            .controller()
            .measure_delay("http://latency.invalid/generate_204")
            .await;
        drop(probe_tunnel);
        result
    });
    tokio::time::timeout(Duration::from_secs(3), entered)
        .await
        .unwrap()
        .unwrap();
    operation.abort();
    assert!(operation.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(2), async {
        while child.lock().unwrap().try_wait().unwrap().is_none() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(!super::super::CHILDREN.lock().unwrap().contains_key(&id));
    assert!(
        idle_tunnel.is_running(),
        "cancelling a latency check must not stop another tunnel"
    );
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
    idle_tunnel.stop().await.unwrap();
}
