//! Routing-mode transitions use loopback sockets only; no desktop is launched.
use super::*;

async fn connect_target(entry: &str, port: u16) -> TcpStream {
    let entry_port = url::Url::parse(entry).unwrap().port().unwrap();
    let mut client = TcpStream::connect((Ipv4Addr::LOCALHOST, entry_port)).await.unwrap();
    client.write_all(&[5, 1, 0]).await.unwrap();
    let mut greeting = [0; 2];
    client.read_exact(&mut greeting).await.unwrap();
    assert_eq!(greeting, [5, 0]);
    let mut request = vec![5, 1, 0, 1, 127, 0, 0, 1];
    request.extend_from_slice(&port.to_be_bytes());
    client.write_all(&request).await.unwrap();
    let mut reply = [0; 10];
    client.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply[1], 0, "SOCKS connection must succeed");
    client
}

#[tokio::test]
async fn disabling_proxy_preserves_entry_and_live_connection_but_new_connections_go_direct() {
    let fixture = LaunchFixture::new("mode-transition");
    tokio::time::timeout(Duration::from_secs(10), async {
        let proxy = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let proxy_address = format!("http://127.0.0.1:{}", proxy.local_addr().unwrap().port());
        let proxy_task = tokio::spawn(async move {
            let (mut stream, _) = proxy.accept().await.unwrap();
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") { headers.push(stream.read_u8().await.unwrap()); }
            stream.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await.unwrap();
            let (mut reader, mut writer) = stream.split();
            tokio::io::copy(&mut reader, &mut writer).await.unwrap();
        });
        let destination = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = destination.local_addr().unwrap().port();
        let direct_task = tokio::spawn(async move {
            let (mut stream, _) = destination.accept().await.unwrap();
            stream.write_all(b"direct").await.unwrap();
        });
        let mut account = eligible_account("mode-transition");
        account.egress_proxy_url = Some(proxy_address.clone());
        fixture.save(&account);
        // A shared proxy exists throughout: disabling must bypass it, not inherit it.
        codex_unified_proxy::enable(codex_unified_proxy::Reference::default(), "cockpit-proxy://disabled-mode-fixture".into()).unwrap();
        let entry = ensure(&account.id).await.unwrap().unwrap();
        let mut existing = connect_target(&entry, port).await;
        existing.write_all(b"before").await.unwrap();
        let mut bytes = [0; 6];
        existing.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"before");

        let saved = codex_proxy_runtime::save_binding_with_mode(account.id.clone(), None, true).await.unwrap();
        assert!(saved.egress_proxy_disabled);
        assert!(saved.egress_proxy_url.is_none());
        assert!(codex_account::load_account(&account.id).unwrap().egress_proxy_disabled);
        assert_eq!(ensure(&account.id).await.unwrap(), Some(entry.clone()));
        assert_eq!(codex_proxy_runtime::status(&account.id).await.unwrap().proxy_source, "disabled");
        assert!(codex_proxy_runtime::desktop_target(&account.id).await.unwrap().is_none());
        existing.write_all(b"after!").await.unwrap();
        existing.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"after!", "existing connection retains its route");
        let mut fresh = connect_target(&entry, port).await;
        fresh.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"direct", "new connection bypasses both account and shared proxies");
        direct_task.await.unwrap();
        drop(existing);
        proxy_task.await.unwrap();

        let followed = codex_proxy_runtime::save_binding(account.id.clone(), None).await.unwrap();
        assert!(!followed.egress_proxy_disabled);
        assert_eq!(codex_account_proxy::configured_url(&followed).unwrap().as_deref(), Some("cockpit-proxy://disabled-mode-fixture"));
        codex_proxy_runtime::save_binding_with_mode(account.id.clone(), None, true).await.unwrap();
        let rebound = codex_proxy_runtime::save_binding(account.id.clone(), Some(proxy_address)).await.unwrap();
        assert!(!rebound.egress_proxy_disabled);
        assert!(rebound.egress_proxy_url.is_some());
    }).await.expect("mode transitions must not hang");
}

#[tokio::test]
async fn disabled_mode_can_be_saved_and_started_with_unreadable_shared_config() {
    let fixture = LaunchFixture::new("disabled-broken-shared");
    let account = eligible_account("disabled-broken-shared");
    fixture.save(&account);
    fs::write(fixture.temp.0.join("codex-unified-proxy.json"), "{broken").unwrap();
    let saved = codex_proxy_runtime::save_binding_with_mode(account.id.clone(), None, true).await.unwrap();
    assert!(saved.egress_proxy_disabled);
    assert!(ensure(&account.id).await.unwrap().is_some());
    assert!(codex_proxy_runtime::save_binding_with_mode(account.id.clone(), Some("http://127.0.0.1:9".into()), true).await.is_err());
    assert!(codex_account::load_account(&account.id).unwrap().egress_proxy_disabled);
}

#[tokio::test]
async fn disabled_request_client_ignores_an_explicit_default_proxy() {
    let fixture = LaunchFixture::new("disabled-request-client");
    let mut account = eligible_account("disabled-request-client");
    account.egress_proxy_disabled = true;
    fixture.save(&account);
    tokio::time::timeout(Duration::from_secs(5), async {
        let destination = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let url = format!("http://127.0.0.1:{}/", destination.local_addr().unwrap().port());
        let direct = tokio::spawn(async move {
            let (mut stream, _) = destination.accept().await.unwrap();
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") { headers.push(stream.read_u8().await.unwrap()); }
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\ndirect").await.unwrap();
        });
        let builder = reqwest::Client::builder().proxy(reqwest::Proxy::all("http://127.0.0.1:9").unwrap());
        let client = codex_proxy_runtime::client_builder(&account, builder).await.unwrap().build().unwrap();
        assert_eq!(client.get(url).send().await.unwrap().text().await.unwrap(), "direct");
        direct.await.unwrap();
    }).await.expect("direct request must bypass the unusable default proxy");
}
