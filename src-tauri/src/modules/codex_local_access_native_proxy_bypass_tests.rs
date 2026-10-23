// Isolate environment and production config OnceLocks in a fresh process.
fn native_proxy_bypass_case(test_name: &str, scenario: &str) {
    const SCENARIO: &str = "COCKPIT_TEST_NATIVE_PROXY_BYPASS";
    if let Ok(child) = std::env::var(SCENARIO) {
        assert_eq!(child, scenario);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(native_proxy_bypass_network_case(scenario));
        std::fs::write(
            std::env::var("COCKPIT_TEST_NATIVE_PROXY_RESULT").unwrap(),
            scenario,
        )
        .unwrap();
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let status = command
        .env_clear()
        .env(SCENARIO, scenario)
        .env(
            "COCKPIT_TEST_NATIVE_PROXY_RESULT",
            directory.path().join("verified"),
        )
        .env("COCKPIT_TOOLS_DATA_DIR", directory.path().join("data"))
        .env("COCKPIT_TOOLS_TEST_DATA_DIR", directory.path().join("data"))
        .args([
            "--exact",
            &format!("modules::codex_local_access::native_transport_identity_tests::{test_name}"),
            "--nocapture",
            "--test-threads=1",
        ])
        .status()
        .unwrap();
    assert!(status.success(), "proxy scenario failed: {scenario}");
    assert_eq!(
        std::fs::read_to_string(directory.path().join("verified")).unwrap(),
        scenario,
        "the exact child test must execute"
    );
}

#[test]
fn native_environment_proxy_honors_no_proxy_and_updates_cached_clients() {
    native_proxy_bypass_case(
        "native_environment_proxy_honors_no_proxy_and_updates_cached_clients",
        "uppercase",
    );
}

#[test]
fn native_environment_proxy_honors_lowercase_no_proxy() {
    native_proxy_bypass_case(
        "native_environment_proxy_honors_lowercase_no_proxy",
        "lowercase",
    );
}

#[test]
fn native_explicit_service_proxy_is_not_bypassed_by_environment() {
    native_proxy_bypass_case(
        "native_explicit_service_proxy_is_not_bypassed_by_environment",
        "explicit",
    );
}

async fn native_proxy_bypass_network_case(scenario: &str) {
    use axum::{extract::ws::WebSocketUpgrade, routing::get, Router};
    use futures_util::{SinkExt, StreamExt};

    let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_address = upstream.local_addr().unwrap();
    let proxy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_address = proxy.local_addr().unwrap();
    let app = Router::new()
        .route("/probe", get(|| async { "direct" }))
        .route(
            "/socket",
            get(|socket: WebSocketUpgrade| async {
                socket.on_upgrade(|mut socket| async move {
                    if let Some(Ok(message)) = socket.recv().await {
                        socket.send(message).await.unwrap();
                    }
                })
            }),
        );
    let upstream_task = tokio::spawn(async move { axum::serve(upstream, app).await.unwrap() });
    let proxy_task = tokio::spawn(async move {
        axum::serve(proxy, Router::new().fallback(|| async { "proxy" }))
            .await
            .unwrap()
    });
    let proxy_url = format!("http://{proxy_address}");
    std::env::set_var("HTTPS_PROXY", &proxy_url);
    if scenario == "uppercase" {
        std::env::set_var("no_proxy", "*");
    }
    let no_proxy_key = if scenario == "lowercase" {
        "no_proxy"
    } else {
        "NO_PROXY"
    };
    std::env::set_var(
        no_proxy_key,
        match scenario {
            "explicit" => "*",
            "lowercase" => "127.0.0.0/8",
            _ => "127.0.0.1",
        },
    );
    let mut input = Inputs::new();
    input.collection.upstream_proxy_url = (scenario == "explicit").then_some(proxy_url);
    input.base = format!("http://{upstream_address}/v1");
    let host = NativeHostBridge::default();
    let (client, _, _, first_key) =
        native_http_client(&host, &input.collection, &input.account, &input.context())
            .await
            .unwrap();
    let probe = format!("http://{upstream_address}/probe");
    let body = client
        .get(&probe)
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    if scenario == "explicit" {
        for source in [
            "account",
            "service",
            "global",
            "system",
            "direct",
            "account_direct",
        ] {
            assert!(native_inherited_proxy_bypass(source).is_empty());
        }
        assert_eq!(
            body, "proxy",
            "an explicit service proxy remains authoritative"
        );
        std::env::set_var(no_proxy_key, "unrelated.invalid");
        let (_, _, _, second_key) =
            native_http_client(&host, &input.collection, &input.account, &input.context())
                .await
                .unwrap();
        assert_eq!(first_key, second_key);
        assert_eq!(host.clients.lock().unwrap().len(), 1);
    } else {
        assert_eq!(
            body, "direct",
            "inherited environment proxies must honor NO_PROXY"
        );
        // The production WebSocket connector upgrades this same reqwest client.
        let response = client
            .get(format!("http://{upstream_address}/socket"))
            .header("connection", "Upgrade")
            .header("upgrade", "websocket")
            .header("sec-websocket-version", "13")
            .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::SWITCHING_PROTOCOLS
        );
        let mut socket = tokio_tungstenite::WebSocketStream::from_raw_socket(
            response.upgrade().await.unwrap(),
            tokio_tungstenite::tungstenite::protocol::Role::Client,
            None,
        )
        .await;
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                "fixture".into(),
            ))
            .await
            .unwrap();
        assert_eq!(
            timeout(Duration::from_secs(2), socket.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap()
                .to_text()
                .unwrap(),
            "fixture"
        );
        drop(socket);
        std::env::set_var(no_proxy_key, "unrelated.invalid");
        let (proxied, _, _, second_key) =
            native_http_client(&host, &input.collection, &input.account, &input.context())
                .await
                .unwrap();
        assert_ne!(
            first_key, second_key,
            "NO_PROXY changes must replace cached policy"
        );
        assert_eq!(
            proxied
                .get(&probe)
                .timeout(Duration::from_secs(2))
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "proxy"
        );
        // Existing clients retain their captured policy for active requests.
        assert_eq!(
            client
                .get(&probe)
                .timeout(Duration::from_secs(2))
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "direct"
        );
        std::env::set_var(no_proxy_key, "*");
        let (wildcard, _, _, third_key) =
            native_http_client(&host, &input.collection, &input.account, &input.context())
                .await
                .unwrap();
        assert_ne!(second_key, third_key);
        assert_eq!(
            wildcard
                .get(&probe)
                .timeout(Duration::from_secs(2))
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "direct"
        );
        assert_eq!(host.clients.lock().unwrap().len(), 3);
    }
    upstream_task.abort();
    proxy_task.abort();
}
