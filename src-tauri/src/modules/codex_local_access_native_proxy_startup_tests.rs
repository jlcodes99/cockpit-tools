// Run each cold-start case in a fresh process: neither cached proxy state nor
// production account/configuration paths may leak in from another test.
fn native_proxy_cold_start_case(test_name: &str, scenario: &str) {
    const SCENARIO: &str = "COCKPIT_TEST_NATIVE_PROXY_STARTUP";
    if let Ok(child) = std::env::var(SCENARIO) {
        assert_eq!(child, scenario);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(native_proxy_cold_start_network_case(scenario));
        std::fs::write(
            std::env::var("COCKPIT_TEST_NATIVE_PROXY_RESULT").unwrap(),
            scenario,
        )
        .unwrap();
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let result_path = directory.path().join("verified");
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let status = command
        .env_clear()
        .env(SCENARIO, scenario)
        .env("COCKPIT_TEST_NATIVE_PROXY_RESULT", &result_path)
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
    assert!(
        status.success(),
        "cold-start proxy scenario failed: {scenario}"
    );
    assert_eq!(std::fs::read_to_string(result_path).unwrap(), scenario);
}

#[test]
fn native_proxy_cold_start_missing_store_accepts_concurrent_first_requests() {
    native_proxy_cold_start_case(
        "native_proxy_cold_start_missing_store_accepts_concurrent_first_requests",
        "missing",
    );
}

#[test]
fn native_proxy_cold_start_loads_saved_disabled_state_before_service_proxy() {
    native_proxy_cold_start_case(
        "native_proxy_cold_start_loads_saved_disabled_state_before_service_proxy",
        "disabled",
    );
}

#[test]
fn native_proxy_cold_start_storage_failure_blocks_fallback_and_can_retry() {
    native_proxy_cold_start_case(
        "native_proxy_cold_start_storage_failure_blocks_fallback_and_can_retry",
        "corrupt",
    );
}

#[test]
fn native_proxy_cold_start_explicit_direct_and_api_keys_skip_shared_storage() {
    native_proxy_cold_start_case(
        "native_proxy_cold_start_explicit_direct_and_api_keys_skip_shared_storage",
        "bypass",
    );
}

#[test]
fn native_proxy_cold_start_independent_binding_creates_missing_entry() {
    native_proxy_cold_start_case(
        "native_proxy_cold_start_independent_binding_creates_missing_entry",
        "independent",
    );
}

async fn native_proxy_cold_start_network_case(scenario: &str) {
    use crate::modules::{codex_proxy_desktop_router, codex_unified_proxy};
    use axum::{routing::get, Router};
    use std::sync::atomic::AtomicUsize;

    let data = crate::modules::account::get_data_dir().unwrap();
    assert_eq!(
        data,
        PathBuf::from(std::env::var("COCKPIT_TOOLS_TEST_DATA_DIR").unwrap())
    );
    codex_unified_proxy::reset_cache();
    let store = data.join("codex-unified-proxy.json");
    const DISABLED: &str = r#"{"version":1,"mode":"off"}"#;
    if scenario == "disabled" {
        std::fs::write(&store, DISABLED).unwrap();
    } else if matches!(scenario, "corrupt" | "bypass" | "independent") {
        std::fs::write(&store, "{private-invalid-state").unwrap();
    }
    assert_eq!(
        codex_unified_proxy::current().unwrap_err(),
        "UNIFIED_PROXY_LOADING"
    );

    let hits = Arc::new(AtomicUsize::new(0));
    let captured = hits.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let proxy_task = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().fallback(get(move || {
                let captured = captured.clone();
                async move {
                    captured.fetch_add(1, Ordering::SeqCst);
                    "service-proxy"
                }
            })),
        )
        .await
        .unwrap();
    });
    let mut input = Inputs::new();
    input.account.auth_mode = CodexAuthMode::OAuth;
    input.account.openai_api_key = None;
    input.collection.upstream_proxy_url = Some(format!("http://{address}"));
    let host = NativeHostBridge::default();

    if scenario == "independent" {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        input.account.egress_proxy_url =
            Some(format!("socks5h://{}", listener.local_addr().unwrap()));
        codex_account::save_account(&input.account).unwrap();
        assert!(codex_proxy_desktop_router::entry_status(&input.account.id).is_none());
        let upstream = tokio::spawn(native_proxy_cold_start_socks_peer(listener));
        let (client, proxy, _, _) =
            native_http_client(&host, &input.collection, &input.account, &input.context())
                .await
                .unwrap();
        let entry = codex_proxy_desktop_router::entry_status(&input.account.id).unwrap();
        assert_eq!(entry.state, "listening");
        assert_eq!(
            url::Url::parse(proxy.as_deref().unwrap()).unwrap().port(),
            entry.port
        );
        let body = client
            .get("http://fixture.invalid/probe")
            .timeout(Duration::from_secs(3))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "account-proxy");
        upstream.await.unwrap();
        assert_eq!(
            hits.load(Ordering::SeqCst),
            0,
            "independent binding must not use service fallback"
        );
        assert_eq!(
            codex_unified_proxy::current().unwrap_err(),
            "UNIFIED_PROXY_LOADING"
        );
    } else if scenario == "bypass" {
        input.account.egress_proxy_disabled = true;
        let (client, proxy, _, _) =
            native_http_client(&host, &input.collection, &input.account, &input.context())
                .await
                .unwrap();
        assert!(proxy.is_none());
        assert_eq!(
            client
                .get(format!("http://{address}/probe"))
                .timeout(Duration::from_secs(3))
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "service-proxy"
        );
        input.account.auth_mode = CodexAuthMode::Apikey;
        input.account.egress_proxy_disabled = false;
        input.account.openai_api_key = Some("fixture-key".into());
        let (client, proxy, _, _) =
            native_http_client(&host, &input.collection, &input.account, &input.context())
                .await
                .unwrap();
        assert_eq!(proxy, input.collection.upstream_proxy_url);
        assert_eq!(
            client
                .get("http://fixture.invalid/probe")
                .timeout(Duration::from_secs(3))
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "service-proxy"
        );
        assert_eq!(
            codex_unified_proxy::current().unwrap_err(),
            "UNIFIED_PROXY_LOADING"
        );
    } else {
        if scenario == "corrupt" {
            let error =
                native_http_client(&host, &input.collection, &input.account, &input.context())
                    .await
                    .err()
                    .expect("unreadable proxy state must fail closed");
            assert_eq!(error.status, 503);
            assert_eq!(error.code, "proxy_unavailable");
            assert!(!error.message.contains("private-invalid-state"));
            assert_eq!(hits.load(Ordering::SeqCst), 0);
            assert_eq!(host.clients.lock().unwrap().len(), 0);
            assert_eq!(
                std::fs::read_to_string(&store).unwrap(),
                "{private-invalid-state"
            );
            std::fs::write(&store, DISABLED).unwrap();
        }
        let requests = (0..8).map(|_| async {
            let (client, proxy, _, _) =
                native_http_client(&host, &input.collection, &input.account, &input.context())
                    .await
                    .expect("first OAuth request must load proxy state without a UI warm-up");
            assert_eq!(proxy, input.collection.upstream_proxy_url);
            client
                .get("http://fixture.invalid/probe")
                .timeout(Duration::from_secs(3))
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap()
        });
        for body in futures::future::join_all(requests).await {
            assert_eq!(body, "service-proxy");
        }
        assert_eq!(hits.load(Ordering::SeqCst), 8);
        assert!(!codex_unified_proxy::current().unwrap().active());
        assert_eq!(host.clients.lock().unwrap().len(), 1);
        assert!(!data.join("secure-account-storage.key").exists());
    }
    if matches!(scenario, "bypass" | "independent") {
        assert_eq!(
            std::fs::read_to_string(&store).unwrap(),
            "{private-invalid-state"
        );
    }
    proxy_task.abort();
}

async fn native_proxy_cold_start_socks_peer(listener: tokio::net::TcpListener) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    timeout(Duration::from_secs(5), async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut greeting = [0; 2];
        stream.read_exact(&mut greeting).await.unwrap();
        assert_eq!(greeting[0], 5);
        let mut methods = vec![0; greeting[1] as usize];
        stream.read_exact(&mut methods).await.unwrap();
        assert!(methods.contains(&0));
        stream.write_all(&[5, 0]).await.unwrap();
        let mut request = [0; 5];
        stream.read_exact(&mut request).await.unwrap();
        assert_eq!(&request[..4], &[5, 1, 0, 3]);
        let mut name = vec![0; request[4] as usize];
        stream.read_exact(&mut name).await.unwrap();
        assert_eq!(name, b"fixture.invalid");
        assert_eq!(stream.read_u16().await.unwrap(), 80);
        stream
            .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
            .await
            .unwrap();
        let mut headers = Vec::new();
        while !headers.ends_with(b"\r\n\r\n") {
            headers.push(stream.read_u8().await.unwrap());
            assert!(headers.len() < 8192);
        }
        assert!(headers.starts_with(b"GET /probe HTTP/1.1\r\n"));
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 13\r\nConnection: close\r\n\r\naccount-proxy",
            )
            .await
            .unwrap();
    })
    .await
    .expect("cold-start entry must forward through the configured account proxy");
}
