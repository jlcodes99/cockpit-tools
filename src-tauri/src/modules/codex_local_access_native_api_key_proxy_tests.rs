#[test]
fn native_api_key_proxy_independent_is_shared_and_changes_retire_transport() {
    native_proxy_cold_start_case(
        "native_api_key_proxy_independent_is_shared_and_changes_retire_transport",
        "api-key-independent",
    );
}

#[test]
fn native_api_key_proxy_inherits_unified_before_service_proxy() {
    native_proxy_cold_start_case(
        "native_api_key_proxy_inherits_unified_before_service_proxy",
        "api-key-inherit",
    );
}

#[test]
fn native_api_key_proxy_direct_overrides_service_and_environment() {
    native_proxy_cold_start_case(
        "native_api_key_proxy_direct_overrides_service_and_environment",
        "api-key-direct",
    );
}

#[test]
fn native_api_key_proxy_corruption_fails_closed_and_recovers() {
    native_proxy_cold_start_case(
        "native_api_key_proxy_corruption_fails_closed_and_recovers",
        "api-key-corrupt",
    );
}

async fn native_api_key_proxy_network_case(scenario: &str) {
    use crate::modules::{codex_client_proxy, codex_proxy_desktop_router, codex_unified_proxy};
    use axum::{routing::get, Router};
    use std::collections::BTreeMap;

    let data = crate::modules::account::get_data_dir().unwrap();
    assert_eq!(
        data,
        PathBuf::from(std::env::var("COCKPIT_TOOLS_TEST_DATA_DIR").unwrap())
    );
    codex_unified_proxy::reset_cache();
    let mut input = Inputs::new();
    codex_account::save_account(&input.account).unwrap();
    let before =
        serde_json::to_value(codex_account::load_account(&input.account.id).unwrap()).unwrap();
    let host = NativeHostBridge::default();
    let id = codex_client_proxy::route_id(&input.account.id);
    let unified_store = data.join("codex-unified-proxy.json");
    let settings_store = data.join("codex-client-proxies.json");

    let service = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let service_address = service.local_addr().unwrap();
    let service_hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let hits = service_hits.clone();
    let service_task = tokio::spawn(async move {
        axum::serve(
            service,
            Router::new().fallback(get(move || {
                let hits = hits.clone();
                async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    "service"
                }
            })),
        )
        .await
        .unwrap();
    });
    input.collection.upstream_proxy_url = Some(format!("http://{service_address}"));
    std::env::set_var("HTTPS_PROXY", format!("http://{service_address}"));
    // Explicit account routes must not inherit bypass rules from the environment.
    std::env::set_var("NO_PROXY", "*");

    if scenario == "api-key-corrupt" {
        std::fs::write(&settings_store, "{private-broken-setting").unwrap();
        let requests = (0..8).map(|_| async {
            let error =
                native_http_client(&host, &input.collection, &input.account, &input.context())
                    .await
                    .err()
                    .expect("corrupt settings must not use a fallback");
            assert_eq!(error.code, "proxy_unavailable");
            assert!(!error.message.contains("private-broken-setting"));
        });
        futures::future::join_all(requests).await;
        assert_eq!(host.clients.lock().unwrap().len(), 0);
        assert_eq!(service_hits.load(Ordering::SeqCst), 0);
        assert_eq!(
            std::fs::read_to_string(&settings_store).unwrap(),
            "{private-broken-setting"
        );
        // Repair only the isolated fixture; no cache reset should be necessary.
        std::fs::write(&settings_store, r#"{"version":1,"targets":{}}"#).unwrap();
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
            "service"
        );
    } else if scenario == "api-key-direct" {
        // Neither a broken unified policy nor a service/environment proxy may
        // override the account's explicit direct setting.
        std::fs::write(&unified_store, "{broken-unified").unwrap();
        codex_client_proxy::save(
            input.account.id.clone(),
            codex_client_proxy::Mode::Direct,
            None,
            None,
            None,
            BTreeMap::new(),
            None,
        )
        .await
        .unwrap();
        let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = upstream.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(upstream, Router::new().fallback(|| async { "direct" }))
                .await
                .unwrap();
        });
        let (client, proxy, diagnostic, _) =
            native_http_client(&host, &input.collection, &input.account, &input.context())
                .await
                .unwrap();
        assert!(proxy.is_none());
        assert_eq!(diagnostic.unwrap()["kind"], "direct");
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
            "direct"
        );
        assert_eq!(service_hits.load(Ordering::SeqCst), 0);
        task.abort();
    } else {
        let socks = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_url = format!("socks5h://{}", socks.local_addr().unwrap());
        if scenario == "api-key-inherit" {
            std::fs::write(&unified_store, serde_json::to_vec(&json!({
                "version":1,"mode":"all_accounts","snapshot":proxy_url,
                "reference":{"sourceId":"fixture","sourceName":"Fixture","itemId":"node","name":"Node"}
            })).unwrap()).unwrap();
        } else {
            std::fs::write(&unified_store, "{broken-unified").unwrap();
            codex_client_proxy::save(
                input.account.id.clone(),
                codex_client_proxy::Mode::Independent,
                Some(proxy_url.clone()),
                None,
                None,
                BTreeMap::new(),
                None,
            )
            .await
            .unwrap();
        }
        let peer = tokio::spawn(native_proxy_cold_start_socks_peer(socks));
        // Concurrent cold preparation must share one entry and one HTTP pool.
        let prepared = futures::future::join_all((0..8).map(|_| async {
            native_http_client(&host, &input.collection, &input.account, &input.context())
                .await
                .unwrap()
        }))
        .await;
        let (client, proxy, diagnostic, identity) = &prepared[0];
        assert!(prepared
            .iter()
            .all(|(_, p, _, key)| p == proxy && key == identity));
        assert_eq!(diagnostic.as_ref().unwrap()["kind"], "proxy");
        assert_eq!(host.clients.lock().unwrap().len(), 1);
        assert_eq!(
            codex_client_proxy::resolve(&input.account.id)
                .await
                .unwrap(),
            crate::modules::process::CodexLaunchProxy::Proxy(proxy.clone().unwrap())
        );
        assert_eq!(
            url::Url::parse(proxy.as_deref().unwrap()).unwrap().port(),
            codex_proxy_desktop_router::entry_status(&id).unwrap().port
        );
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
            "account-proxy"
        );
        peer.await.unwrap();
        assert_eq!(service_hits.load(Ordering::SeqCst), 0);
        if scenario == "api-key-independent" {
            let socks = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let next_url = format!("socks5h://{}", socks.local_addr().unwrap());
            codex_client_proxy::save(
                input.account.id.clone(),
                codex_client_proxy::Mode::Independent,
                Some(next_url),
                None,
                None,
                BTreeMap::new(),
                None,
            )
            .await
            .unwrap();
            let peer = tokio::spawn(native_proxy_cold_start_socks_peer(socks));
            let (next_client, next_proxy, _, next_identity) =
                native_http_client(&host, &input.collection, &input.account, &input.context())
                    .await
                    .unwrap();
            assert_eq!(&next_proxy, proxy, "the local entry remains stable");
            assert_ne!(
                &next_identity, identity,
                "a changed binding must select a new pool"
            );
            assert_eq!(
                next_client
                    .get("http://fixture.invalid/probe")
                    .timeout(Duration::from_secs(3))
                    .send()
                    .await
                    .unwrap()
                    .text()
                    .await
                    .unwrap(),
                "account-proxy"
            );
            peer.await.unwrap();
            // The setting is per account, not a new service-wide proxy.
            let mut other = input.account.clone();
            other.id = "other-api-key".into();
            std::fs::write(&unified_store, r#"{"version":1,"mode":"off"}"#).unwrap();
            codex_unified_proxy::reset_cache();
            let (_, other_proxy, _, _) =
                native_http_client(&host, &input.collection, &other, &input.context())
                    .await
                    .unwrap();
            assert_eq!(other_proxy, input.collection.upstream_proxy_url);
        }
    }
    assert_eq!(
        serde_json::to_value(codex_account::load_account(&input.account.id).unwrap()).unwrap(),
        before
    );
    crate::modules::codex_proxy_runtime::release_deleted_account(&id);
    service_task.abort();
}
