//! Startup preparation tests. All sockets target loopback; no desktop UI is launched.
use super::*;
use base64::Engine as _;

async fn local_socks_greeting(url: &str) -> TcpStream {
    let port = url::Url::parse(url).unwrap().port().unwrap();
    let mut client = tokio::time::timeout(
        Duration::from_secs(2),
        TcpStream::connect((Ipv4Addr::LOCALHOST, port)),
    )
    .await
    .expect("local proxy connection deadline")
    .expect("local proxy is listening");
    client.write_all(&[5, 1, 0]).await.unwrap();
    let mut reply = [0; 2];
    tokio::time::timeout(Duration::from_secs(2), client.read_exact(&mut reply))
        .await
        .expect("local SOCKS greeting deadline")
        .unwrap();
    assert_eq!(
        reply,
        [5, 0],
        "desktop entry accepts unauthenticated local SOCKS"
    );
    client
}

async fn local_connect(client: &mut TcpStream) -> u8 {
    let host = b"fixture.invalid";
    let mut request = vec![5, 1, 0, 3, host.len() as u8];
    request.extend_from_slice(host);
    request.extend_from_slice(&443u16.to_be_bytes());
    client.write_all(&request).await.unwrap();
    let mut reply = [0; 10];
    client.read_exact(&mut reply).await.unwrap();
    reply[1]
}

fn verify_launch_injection(proxy: &str) {
    let mut args = vec!["--no-proxy-server".into(), "--proxy-bypass-list=*".into()];
    crate::modules::process::append_electron_proxy_args(&mut args, proxy);
    assert_eq!(
        args,
        [
            format!("--proxy-server={proxy}"),
            "--proxy-bypass-list=localhost;127.0.0.1;[::1]".into(),
        ]
    );
    // Inspect the real command builder without launching a GUI or contacting any server.
    let mut command = std::process::Command::new("unused-test-client");
    crate::modules::process::apply_effective_proxy_env_to_command(&mut command, Some(proxy));
    for key in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        let value = command
            .get_envs()
            .find(|(name, _)| *name == key)
            .and_then(|(_, value)| value);
        assert_eq!(value.and_then(|value| value.to_str()), Some(proxy));
    }
    #[cfg(target_os = "macos")]
    {
        let mut open = std::process::Command::new("/usr/bin/open");
        crate::modules::process::append_effective_proxy_env_to_open_args(&mut open, Some(proxy));
        let args: Vec<_> = open
            .get_args()
            .map(|value| value.to_string_lossy().into_owned())
            .collect();
        for key in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"] {
            assert!(args
                .windows(2)
                .any(|pair| pair[0] == "--env" && pair[1] == format!("{key}={proxy}")));
        }
    }
}

#[tokio::test]
async fn startup_resolver_without_account_or_shared_proxy_creates_no_entry() {
    let fixture = LaunchFixture::new("startup-without-proxy");
    let account = eligible_account("startup-without-proxy");
    fixture.save(&account);
    let proxy =
        crate::modules::codex_instance::resolve_egress_proxy_for_bind_account(Some(&account.id))
            .await
            .expect("startup proxy resolution");
    assert!(proxy.is_none());
    assert!(route_port(&account.id).await.is_none());
    assert!(!fixture.temp.ports_file().exists());
    let status = codex_proxy_runtime::status(&account.id).await.unwrap();
    assert!(status.desktop_entry.is_none());
    assert_eq!(status.proxy_source, "none");
    assert!(status.effective_proxy.is_none());
}

#[tokio::test]
async fn startup_resolver_routes_through_local_proxy_and_injects_client_settings() {
    let fixture = LaunchFixture::new("startup-local-proxy");
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let proxy_port = listener.local_addr().unwrap().port();
    let upstream = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut headers = Vec::new();
        while !headers.ends_with(b"\r\n\r\n") {
            headers.push(socket.read_u8().await.unwrap());
            assert!(headers.len() < 8192);
        }
        assert!(headers.starts_with(b"CONNECT fixture.invalid:443 HTTP/1.1\r\n"));
        socket
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await
            .unwrap();
        let mut request = [0; 4];
        socket.read_exact(&mut request).await.unwrap();
        assert_eq!(&request, b"ping");
        socket.write_all(b"pong").await.unwrap();
    });
    let mut account = eligible_account("startup-local-proxy");
    account.egress_proxy_url = Some(format!("http://127.0.0.1:{proxy_port}"));
    fixture.save(&account);
    let proxy =
        crate::modules::codex_instance::resolve_egress_proxy_for_bind_account(Some(&account.id))
            .await
            .unwrap()
            .expect("bound account must get a desktop proxy entry");
    verify_launch_injection(&proxy);
    let status = codex_proxy_runtime::status(&account.id).await.unwrap();
    let entry = status.desktop_entry.unwrap();
    assert_eq!(entry.state, "listening");
    assert_eq!(entry.port, url::Url::parse(&proxy).unwrap().port());
    assert_eq!(entry.request_count, 0);
    assert_eq!(entry.last_request_state, "none");
    assert_eq!(status.proxy_source, "account");
    let check = async {
        let mut client = local_socks_greeting(&proxy).await;
        assert_eq!(
            entry_status(&account.id).unwrap().request_count,
            0,
            "a greeting alone is not a CONNECT request"
        );
        assert_eq!(
            local_connect(&mut client).await,
            0,
            "local mock accepted CONNECT"
        );
        client.write_all(b"ping").await.unwrap();
        let mut response = [0; 4];
        client.read_exact(&mut response).await.unwrap();
        assert_eq!(&response, b"pong");
        upstream.await.unwrap();
    };
    tokio::time::timeout(Duration::from_secs(5), check)
        .await
        .unwrap();
    let entry = entry_status(&account.id).unwrap();
    assert_eq!(entry.request_count, 1);
    assert_eq!(entry.last_request_state, "forwarded");
    assert!(entry.last_error.is_none());
    assert_eq!(ensure(&account.id).await.unwrap(), Some(proxy));
}

#[tokio::test]
async fn entry_observes_upstream_failure_and_recovery_without_claiming_connectivity() {
    let fixture = LaunchFixture::new("entry-request-recovery");
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let mut account = eligible_account("entry-request-recovery");
    account.egress_proxy_url = Some(format!(
        "http://127.0.0.1:{}",
        listener.local_addr().unwrap().port()
    ));
    fixture.save(&account);
    let upstream = tokio::spawn(async move {
        for success in [false, true] {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") {
                headers.push(socket.read_u8().await.unwrap());
                assert!(headers.len() < 8192);
            }
            socket
                .write_all(if success {
                    b"HTTP/1.1 200 OK\r\n\r\n"
                } else {
                    b"HTTP/1.1 502 Failed\r\n\r\n"
                })
                .await
                .unwrap();
        }
    });
    let proxy = ensure(&account.id).await.unwrap().unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut failed = local_socks_greeting(&proxy).await;
        assert_ne!(local_connect(&mut failed).await, 0);
        let entry = entry_status(&account.id).unwrap();
        assert_eq!(entry.state, "listening");
        assert_eq!(entry.request_count, 1);
        assert_eq!(entry.last_request_state, "failed");
        assert_eq!(entry.last_error, Some("PROXY_CONNECT_FAILED"));
        let mut retry = local_socks_greeting(&proxy).await;
        assert_eq!(local_connect(&mut retry).await, 0);
        upstream.await.unwrap();
        let entry = entry_status(&account.id).unwrap();
        assert_eq!(entry.request_count, 2);
        assert_eq!(entry.last_request_state, "forwarded");
        assert!(entry.last_error.is_none());
    })
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "requires explicit installed engine; whole local startup/request path, no external requests or GUI"]
async fn startup_request_crosses_entry_and_engine_to_loopback_upstream() {
    let engine = PathBuf::from(
        std::env::var_os("COCKPIT_TEST_PROXY_ENGINE_ROOT").expect("explicit engine installation"),
    );
    let fixture = LaunchFixture::new("entry-engine-request");
    install_isolated_engine(&fixture, &engine);
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let mut account = eligible_account("entry-engine-request");
    account.egress_proxy_url = Some(format!(
        "http://test-user:synthetic-secret@127.0.0.1:{}",
        listener.local_addr().unwrap().port()
    ));
    fixture.save(&account);
    let cleanup = RuntimeCleanup(account.id.clone());
    let upstream = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut headers = Vec::new();
        while !headers.ends_with(b"\r\n\r\n") {
            headers.push(socket.read_u8().await.unwrap());
            assert!(headers.len() < 8192);
        }
        let headers = String::from_utf8(headers).unwrap();
        assert!(headers.starts_with("CONNECT fixture.invalid:443 HTTP/1.1\r\n"));
        assert!(headers
            .to_ascii_lowercase()
            .contains("proxy-authorization: basic "));
        socket.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
        let mut ping = [0; 4];
        socket.read_exact(&mut ping).await.unwrap();
        assert_eq!(&ping, b"ping");
        socket.write_all(b"pong").await.unwrap();
    });
    let proxy =
        crate::modules::codex_instance::resolve_egress_proxy_for_bind_account(Some(&account.id))
            .await
            .unwrap()
            .unwrap();
    verify_launch_injection(&proxy);
    let before = codex_proxy_runtime::status(&account.id).await.unwrap();
    assert_eq!(before.desktop, "idle");
    assert_eq!(before.desktop_entry.unwrap().state, "listening");
    tokio::time::timeout(Duration::from_secs(15), async {
        let mut client = local_socks_greeting(&proxy).await;
        assert_eq!(local_connect(&mut client).await, 0);
        client.write_all(b"ping").await.unwrap();
        let mut pong = [0; 4];
        client.read_exact(&mut pong).await.unwrap();
        assert_eq!(&pong, b"pong");
        upstream.await.unwrap();
    })
    .await
    .unwrap();
    let after = codex_proxy_runtime::status(&account.id).await.unwrap();
    assert_eq!(after.desktop, "running");
    let entry = after.desktop_entry.as_ref().unwrap();
    assert_eq!(entry.port, url::Url::parse(&proxy).unwrap().port());
    assert_ne!(
        entry.port, after.desktop_port,
        "entry and engine ports have different roles"
    );
    assert_eq!(entry.request_count, 1);
    assert_eq!(entry.last_request_state, "forwarded");
    let public = serde_json::to_string(&after).unwrap();
    assert!(!public.contains("test-user"));
    assert!(!public.contains("synthetic-secret"));
    assert!(!public.contains("fixture.invalid"));
    let engine_port = after.desktop_port.unwrap();
    drop(cleanup);
    tokio::time::timeout(Duration::from_secs(5), async {
        while TcpStream::connect((Ipv4Addr::LOCALHOST, engine_port))
            .await
            .is_ok()
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("test-owned engine is reclaimed");
}

#[derive(Deserialize)]
struct LocalAccountSnapshot {
    account_id: String,
    binding: String,
}

fn install_isolated_engine(fixture: &LaunchFixture, source: &Path) {
    let record = fs::read(source.join("active.json")).expect("read explicit engine metadata");
    let value: serde_json::Value = serde_json::from_slice(&record).unwrap();
    let directory = value["directory"].as_str().unwrap();
    assert!(directory.starts_with("release-") && !directory.contains(['/', '\\']));
    let destination = fixture.temp.0.join("proxy-engine").join(directory);
    fs::create_dir_all(&destination).unwrap();
    for (name, hash) in value["files"].as_object().unwrap() {
        assert!(!name.contains(['/', '\\']) && name != "." && name != "..");
        let origin = source.join(directory).join(name);
        assert!(fs::symlink_metadata(&origin).unwrap().is_file());
        let contents = fs::read(&origin).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(&contents)),
            hash.as_str().unwrap(),
            "installed engine checksum"
        );
        fs::copy(origin, destination.join(name)).unwrap();
    }
    fs::write(destination.join("installed.json"), &record).unwrap();
    fs::write(destination.join(".lease"), "").unwrap();
    fs::write(fixture.temp.0.join("proxy-engine/active.json"), record).unwrap();
}

struct RuntimeCleanup(String);
impl Drop for RuntimeCleanup {
    fn drop(&mut self) {
        codex_proxy_runtime::release_deleted_account(&self.0);
    }
}

/// Explicit local diagnostic: the supplied snapshot contains only the requested
/// account ID and proxy binding. Real account tokens and profiles are never loaded.
#[tokio::test]
#[ignore = "requires an explicit private proxy snapshot and installed engine; local sockets only"]
async fn local_account_startup_prepares_ports_without_external_requests() {
    let input = PathBuf::from(
        std::env::var_os("COCKPIT_TEST_PROXY_STARTUP_SNAPSHOT").expect("explicit snapshot path"),
    );
    let engine = PathBuf::from(
        std::env::var_os("COCKPIT_TEST_PROXY_ENGINE_ROOT").expect("explicit engine installation"),
    );
    let snapshot: LocalAccountSnapshot = serde_json::from_slice(&fs::read(input).unwrap()).unwrap();
    assert!(!snapshot.account_id.is_empty() && snapshot.binding.starts_with("cockpit-proxy://"));
    let fixture = LaunchFixture::new("real-account-startup-ports");
    install_isolated_engine(&fixture, &engine);
    let mut account = eligible_account(&snapshot.account_id);
    account.egress_proxy_url = Some(snapshot.binding);
    // Refuse automatic health checks: this diagnostic must not send external probes.
    let encoded = account
        .egress_proxy_url
        .as_deref()
        .unwrap()
        .strip_prefix("cockpit-proxy://")
        .unwrap();
    let payload: serde_json::Value = serde_json::from_slice(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(payload["network"]["doh"], false);
    for outbound in payload["outbounds"].as_array().unwrap() {
        if let Some(group) = outbound.get("group") {
            assert_eq!(
                group["type"], "select",
                "automatic health-check groups are outside this offline diagnostic"
            );
            assert!(group.get("url").is_none());
        }
    }
    fixture.save(&account);
    let cleanup = RuntimeCleanup(account.id.clone());

    // Reproduce saving an account proxy: account/API tunnel starts first.
    let account_url = codex_proxy_runtime::ensure(&account.id)
        .await
        .unwrap()
        .unwrap();
    let before = codex_proxy_runtime::status(&account.id).await.unwrap();
    assert_eq!(before.account, "running");
    assert_eq!(before.desktop, "idle");
    let proxy =
        crate::modules::codex_instance::resolve_egress_proxy_for_bind_account(Some(&account.id))
            .await
            .unwrap()
            .expect("startup returns a local proxy entry");
    verify_launch_injection(&proxy);
    drop(local_socks_greeting(&proxy).await);
    let after_launch = codex_proxy_runtime::status(&account.id).await.unwrap();
    assert_eq!(
        after_launch.desktop, "idle",
        "current launch preparation is lazy despite a listening entry"
    );
    assert!(after_launch.desktop_port.is_none());

    // Call the same upstream preparation used by the first desktop connection,
    // but never send a destination/CONNECT request to the real proxy.
    let (desktop_url, lease) = codex_proxy_runtime::desktop_target(&account.id)
        .await
        .unwrap()
        .unwrap();
    drop(local_socks_greeting(&desktop_url).await);
    let ready = codex_proxy_runtime::status(&account.id).await.unwrap();
    assert_eq!(ready.desktop, "running");
    assert!(ready.desktop_port.is_some());
    assert_eq!(ensure(&account.id).await.unwrap(), Some(proxy.clone()));
    let ports = [
        url::Url::parse(&account_url).unwrap().port().unwrap(),
        url::Url::parse(&desktop_url).unwrap().port().unwrap(),
    ];
    drop(lease);
    drop(cleanup);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let mut closed = true;
            for port in ports {
                closed &= TcpStream::connect((Ipv4Addr::LOCALHOST, port))
                    .await
                    .is_err();
            }
            if closed {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("test-owned engine ports are reclaimed");
    let report = serde_json::json!({
        "entryPort": url::Url::parse(&proxy).unwrap().port(),
        "accountPort": before.account_port,
        "desktopAfterLaunchPreparation": after_launch.desktop,
        "desktopAfterDemandPreparation": ready.desktop,
        "desktopPort": ready.desktop_port,
        "launchArgumentsVerified": true,
        "localSocksGreetingVerified": true,
        "externalRequestsTested": false,
        "enginePortsReclaimed": true,
    });
    if let Some(path) = std::env::var_os("COCKPIT_TEST_PROXY_STARTUP_REPORT") {
        fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    println!("LOCAL_PROXY_STARTUP_REPORT {report}");
}
