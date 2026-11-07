#[test]
fn http2_recovery_counts_generations_bounds_state_and_expires() {
    let now = Instant::now();
    let client = Client::builder().no_proxy().build().unwrap();
    let mut cache = NativeHttpClientCache { capacity: 2, ..Default::default() };
    let first = cache.insert("route-a".into(), "a".into(), client.clone());
    assert!(cache.invalidate_http2(&first, now));
    for _ in 0..10 { assert!(!cache.invalidate_http2(&first, now)); }
    assert!(!cache.http1_fallback("route-a", now));
    let second = cache.insert("route-a".into(), "a".into(), client.clone());
    assert!(!cache.invalidate_http2(&first, now));
    assert!(cache.invalidate_http2(&second, now + Duration::from_secs(1)));
    assert!(cache.http1_fallback("route-a", now + Duration::from_secs(1)));
    let fallback = cache.insert("route-a".into(), "a".into(), client.clone());
    assert!(cache.entries["route-a"].http1_fallback);
    assert!(!cache.invalidate_http2(&second, now));
    assert_eq!(cache.get("route-a").unwrap().1, fallback);
    // Expiration removes the old HTTP/1-only client at the next preparation.
    cache.recovery.get_mut("route-a").unwrap().fallback_until = Some(now);
    assert!(cache.get("route-a").is_none());
    assert!(!cache.http1_fallback("route-a", now + Duration::from_secs(302)));
    for key in ["a", "b", "c", "d"] {
        let identity = cache.insert(key.into(), key.into(), client.clone());
        assert!(cache.invalidate_http2(&identity, now));
    }
    assert!(cache.recovery.len() <= 2);
}

#[test]
fn http2_failures_outside_window_do_not_force_http1() {
    let now = Instant::now();
    let client = Client::builder().no_proxy().build().unwrap();
    let mut cache = NativeHttpClientCache::default();
    let first = cache.insert("route".into(), "a".into(), client.clone());
    assert!(cache.invalidate_http2(&first, now));
    let second = cache.insert("route".into(), "a".into(), client);
    assert!(cache.invalidate_http2(&second, now + Duration::from_secs(61)));
    assert!(!cache.http1_fallback("route", now + Duration::from_secs(61)));
}

#[tokio::test]
async fn native_http2_recovery_is_scoped_and_reports_fallback() {
    let host = NativeHostBridge::default();
    let mut input = Inputs::new();
    input.account.egress_proxy_disabled = true;
    let mut other = input.clone();
    other.account.id = "other-account".into();
    let mut websocket = input.clone();
    websocket.http1_only = true;
    let mut changed_proxy = input.clone();
    changed_proxy.account.egress_proxy_disabled = false;
    changed_proxy.collection.upstream_proxy_url = Some("http://127.0.0.1:1".into());
    let (_, _, _, other_id) = native_http_client(&host, &other.collection, &other.account, &other.context()).await.unwrap();
    let (_, _, _, ws_id) = native_http_client(&host, &websocket.collection, &websocket.account, &websocket.context()).await.unwrap();
    for _ in 0..2 {
        let (_, _, _, id) = native_http_client(&host, &input.collection, &input.account, &input.context()).await.unwrap();
        assert!(native_contract::Host::invalidate_http2_transport(&host, &id));
    }
    let (_, proxy, diagnostic, fallback_id) = native_http_client(&host, &input.collection, &input.account, &input.context()).await.unwrap();
    assert!(proxy.is_none());
    assert_eq!(diagnostic.unwrap()["transportHttp1Fallback"], true);
    for (unchanged, expected) in [(&other, other_id), (&websocket, ws_id)] {
        let (_, _, diagnostic, id) = native_http_client(&host, &unchanged.collection, &unchanged.account, &unchanged.context()).await.unwrap();
        assert_eq!(id, expected);
        assert!(diagnostic.unwrap().get("transportHttp1Fallback").is_none());
    }
    let (_, proxy, diagnostic, _) = native_http_client(&host, &changed_proxy.collection, &changed_proxy.account, &changed_proxy.context()).await.unwrap();
    assert_eq!(proxy.as_deref(), Some("http://127.0.0.1:1"));
    assert!(diagnostic.unwrap().get("transportHttp1Fallback").is_none());
    // Even a failure on the temporary H1 client must not extend the fallback.
    let until = host.clients.lock().unwrap().recovery.values().find_map(|e| e.fallback_until).unwrap();
    assert!(native_contract::Host::invalidate_transport(&host, &fallback_id));
    assert_eq!(host.clients.lock().unwrap().recovery.values().find_map(|e| e.fallback_until), Some(until));
}

#[tokio::test]
async fn http2_recovery_policy_changes_real_tls_alpn_then_expires() {
    let now = Instant::now();
    let mut cache = NativeHttpClientCache::default();
    let dummy = Client::builder().no_proxy().build().unwrap();
    for _ in 0..2 {
        let id = cache.insert("route".into(), "a".into(), dummy.clone());
        cache.invalidate_http2(&id, now);
    }
    for (at, expected) in [(now, b"http/1.1".as_slice()), (now + Duration::from_secs(301), b"h2".as_slice())] {
        let (address, root, server) = native_alpn_fixture(vec![b"h2".to_vec(), b"http/1.1".to_vec()]).await;
        let client = native_http_client_builder(true, cache.http1_fallback("route", at))
            .no_proxy().tls_built_in_root_certs(false).add_root_certificate(root)
            .timeout(NATIVE_ALPN_DEADLINE).build().unwrap();
        let _ = client.get(format!("https://{address}/responses")).send().await;
        assert_eq!(server.await.unwrap().unwrap().as_deref(), Some(expected));
    }
}
