use super::*;
use crate::models::codex::CodexTokens;
use crate::modules::{codex_account, codex_unified_proxy};

#[path = "codex_proxy_desktop_router_startup_tests.rs"]
mod startup;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "codex-proxy-desktop-router-{tag}-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        Self(path)
    }

    fn ports_file(&self) -> PathBuf {
        self.0.join(PORTS_FILE)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// 构造一个通过 `codex_account_proxy::eligible`、但没有绑定任何代理的账号。
fn eligible_account(id: &str) -> CodexAccount {
    CodexAccount::new(
        id.to_string(),
        "desktop-entry@example.com".to_string(),
        CodexTokens {
            access_token: "access".to_string(),
            refresh_token: Some("refresh".to_string()),
            id_token: "id".to_string(),
        },
    )
}

/// Public `ensure` must exercise fresh account reads, rather than only testing
/// `ensure_for_account`, which never contained the stale-cache shortcut.
struct LaunchFixture {
    _env_lock: std::sync::MutexGuard<'static, ()>,
    temp: TempDir,
    previous_data_dir: Option<std::ffi::OsString>,
    previous_resolved_data_dir: Option<std::ffi::OsString>,
}

impl LaunchFixture {
    fn new(tag: &str) -> Self {
        let env_lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let temp = TempDir::new(tag);
        let previous_data_dir = std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR");
        let previous_resolved_data_dir = std::env::var_os("COCKPIT_TOOLS_DATA_DIR");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &temp.0);
        std::env::set_var("COCKPIT_TOOLS_DATA_DIR", &temp.0);
        let fixture = Self {
            _env_lock: env_lock,
            temp,
            previous_data_dir,
            previous_resolved_data_dir,
        };
        assert_eq!(
            account::get_data_dir().expect("isolated data dir"),
            fixture.temp.0
        );
        // Unified proxy resolves its read-only path without creating directories,
        // while account/key storage uses get_data_dir. Both must be isolated.
        assert_eq!(
            account::resolve_data_dir().expect("isolated unified-proxy data dir"),
            fixture.temp.0
        );
        codex_unified_proxy::reset_cache();
        fixture
    }

    fn save(&self, account: &CodexAccount) {
        codex_account::save_account(account).expect("save fixture account");
    }
}

impl Drop for LaunchFixture {
    fn drop(&mut self) {
        // Clear the process cache before restoring the environment so neither
        // cleanup nor any account read can reach the user's storage directory.
        codex_unified_proxy::reset_cache();
        match &self.previous_data_dir {
            Some(value) => std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", value),
            None => std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR"),
        }
        match &self.previous_resolved_data_dir {
            Some(value) => std::env::set_var("COCKPIT_TOOLS_DATA_DIR", value),
            None => std::env::remove_var("COCKPIT_TOOLS_DATA_DIR"),
        }
    }
}

#[test]
fn candidate_ports_are_deterministic_and_prefer_the_record() {
    let base = preferred_port("entry-order");
    assert_eq!(candidate_ports("entry-order", None).next(), Some(base));
    assert_eq!(
        candidate_ports("entry-order", None).collect::<Vec<_>>(),
        candidate_ports("entry-order", None).collect::<Vec<_>>()
    );
    assert_eq!(
        candidate_ports("entry-order", Some(45_000))
            .take(3)
            .collect::<Vec<_>>(),
        vec![45_000, 45_001, 45_002]
    );
    let last = PORT_RANGE_START + PORT_RANGE_SIZE - 1;
    assert_eq!(
        candidate_ports("entry-order", Some(last))
            .take(2)
            .collect::<Vec<_>>(),
        vec![last, PORT_RANGE_START]
    );
    // 越界记录（例如被外部进程改坏）被忽略，退回确定性起点。
    assert_eq!(
        candidate_ports("entry-order", Some(1234)).next(),
        Some(base)
    );
}

#[tokio::test]
async fn recorded_port_is_persisted_and_reused_after_host_restart() {
    let temp = TempDir::new("reuse");
    let path = temp.ports_file();
    let account_id = "entry-reuse";

    let (listener, port) = install(account_id, Some(&path))
        .await
        .expect("first install");
    assert!(is_managed_port(port));
    drop(listener);

    let text = fs::read_to_string(&path).expect("port record written");
    assert!(text.contains(account_id));
    assert_eq!(read_registry(&path).recorded(account_id), Some(port));

    // 模拟宿主重启：内存路由丢失，但记录文件仍在。
    let (listener, again) = install(account_id, Some(&path))
        .await
        .expect("install after restart");
    assert_eq!(again, port);
    drop(listener);
}

#[tokio::test]
async fn occupied_recorded_port_falls_back_and_rewrites_the_record() {
    let temp = TempDir::new("conflict");
    let path = temp.ports_file();
    let account_id = "entry-conflict";

    let (listener, recorded) = install(account_id, Some(&path))
        .await
        .expect("first install");
    drop(listener);
    // 其他进程占住记录端口。
    let blocker = TcpListener::bind((Ipv4Addr::LOCALHOST, recorded))
        .await
        .expect("occupy recorded port");

    let (listener, port) = install(account_id, Some(&path))
        .await
        .expect("install with occupied record");
    assert_ne!(port, recorded);
    assert_eq!(
        port,
        candidate_ports(account_id, Some(recorded))
            .nth(1)
            .expect("second candidate")
    );
    assert_eq!(read_registry(&path).recorded(account_id), Some(port));
    drop(listener);
    drop(blocker);
}

#[tokio::test]
async fn eligible_account_without_binding_keeps_the_original_launch_path() {
    let temp = TempDir::new("unbound");
    let path = temp.ports_file();
    let account_id = "entry-unbound";
    let account = eligible_account(account_id);
    assert!(codex_account_proxy::eligible(&account));
    assert!(account.egress_proxy_url.is_none());
    // 未绑定且没有统一代理：不安装入口，保留应用全局代理/PAC/系统代理等原有出口行为。
    assert!(!codex_account_proxy::has_effective_proxy(&account, false));
    assert!(ensure_for_account(account_id, &account, false, Some(&path))
        .await
        .expect("unbound account")
        .is_none());
    assert!(route_port(account_id).await.is_none());
    assert!(!path.exists());
}

/// 统一代理属于生效出口：即使账号没有独立绑定，也要能安装入口并热切换。
#[tokio::test]
async fn unified_proxy_installs_an_entry_for_an_account_without_its_own_binding() {
    let temp = TempDir::new("unified");
    let path = temp.ports_file();
    let account_id = "entry-unified";
    let account = eligible_account(account_id);
    assert!(codex_account_proxy::has_effective_proxy(&account, true));
    let url = ensure_for_account(account_id, &account, true, Some(&path))
        .await
        .expect("install entry")
        .expect("entry url");
    let port = route_port(account_id).await.expect("route registered");
    assert_eq!(url, entry_url(port));
}

#[tokio::test]
async fn bound_account_gets_a_stable_entry_reused_within_the_host_process() {
    let temp = TempDir::new("bound");
    let path = temp.ports_file();
    let account_id = "entry-bound";
    let mut account = eligible_account(account_id);
    account.egress_proxy_url = Some("http://127.0.0.1:8080".to_string());
    assert!(codex_account_proxy::has_effective_proxy(&account, false));

    let first = ensure_for_account(account_id, &account, false, Some(&path))
        .await
        .expect("install entry")
        .expect("entry url");
    let second = ensure_for_account(account_id, &account, false, Some(&path))
        .await
        .expect("reuse entry")
        .expect("entry url");
    assert_eq!(first, second);

    let port = route_port(account_id).await.expect("route registered");
    assert_eq!(first, entry_url(port));
    TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .expect("entry is listening");
    assert_eq!(read_registry(&path).recorded(account_id), Some(port));
}

#[tokio::test]
async fn listener_cancellation_is_visible_and_next_launch_reinstalls_entry() {
    let id = "entry-listener-cancelled";
    let (listener, port) = install(id, None).await.unwrap();
    let observation = entry::listening(id, port);
    ROUTES.lock().await.insert(id.into(), port);
    let task = tokio::spawn(serve(listener, id.into(), observation));
    task.abort();
    let _ = task.await;
    assert_eq!(entry_status(id).unwrap().state, "stopped");
    let restarted = ensure_route(id, None).await.unwrap();
    let snapshot = entry_status(id).unwrap();
    assert_eq!(snapshot.state, "listening");
    assert_eq!(Some(restarted), snapshot.port.map(entry_url));
    // Status observations stay immediate even while another launch owns the install lock.
    let _installation = ROUTES.lock().await;
    assert_eq!(entry_status(id).unwrap().state, "listening");
}

#[tokio::test]
async fn occupied_entry_reports_start_failure_and_successful_retry() {
    let id = "entry-listener-occupied";
    let mut blockers = Vec::new();
    for port in candidate_ports(id, None) {
        // A port already held by another test/process is also unavailable.
        if let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await {
            blockers.push(listener);
        }
    }
    assert_eq!(
        ensure_route(id, None).await.unwrap_err(),
        "PROXY_ENTRY_PORT_UNAVAILABLE"
    );
    let failed = entry_status(id).unwrap();
    assert_eq!(failed.state, "failed");
    assert_eq!(failed.port, None);
    drop(blockers);
    ensure_route(id, None).await.unwrap();
    let retry = entry_status(id).unwrap();
    assert_eq!(retry.state, "listening");
    assert!(retry.last_error.is_none());
}

#[tokio::test]
async fn status_shows_effective_account_or_shared_proxy_without_credentials_or_side_effects() {
    use crate::modules::{
        codex_proxy_catalog_binding as binding, codex_proxy_subscription_parser as parser,
    };
    let fixture = LaunchFixture::new("effective-proxy-preview");
    let mut account = eligible_account("effective-proxy-preview");
    fixture.save(&account);
    let catalog = parser::parse("http://shared-user:shared-secret@127.0.0.1:8080#Shared").unwrap();
    let encoded = binding::encode(
        "test-source",
        "Test source",
        &catalog.nodes[0].id,
        &catalog,
        &Default::default(),
    )
    .unwrap();
    codex_unified_proxy::enable(codex_unified_proxy::Reference::default(), encoded).unwrap();
    let shared = codex_proxy_runtime::status(&account.id).await.unwrap();
    assert_eq!(shared.proxy_source, "unified");
    assert!(shared.desktop_entry.is_none());
    let public = serde_json::to_value(shared).unwrap();
    assert_eq!(public["effectiveProxy"]["name"], "Shared");
    assert_eq!(public["effectiveProxy"]["sourceId"], "test-source");
    for secret in [
        "shared-user",
        "shared-secret",
        "cockpit-proxy://",
        "outbounds",
    ] {
        assert!(!public.to_string().contains(secret));
    }
    account.egress_proxy_url = Some("http://own-user:own-secret@127.0.0.1:8081".into());
    fixture.save(&account);
    let own = codex_proxy_runtime::status(&account.id).await.unwrap();
    assert_eq!(own.proxy_source, "account");
    let public = serde_json::to_value(own).unwrap();
    assert_eq!(public["effectiveProxy"]["port"], 8081);
    assert!(!public.to_string().contains("own-secret"));
    assert!(!public.to_string().contains("own-user"));
    account.egress_proxy_url = None;
    fixture.save(&account);
    codex_unified_proxy::disable().unwrap();
    let unbound = codex_proxy_runtime::status(&account.id).await.unwrap();
    assert_eq!(unbound.proxy_source, "none");
    assert!(unbound.effective_proxy.is_none());
    assert!(unbound.desktop_entry.is_none());
    assert!(
        !fixture.temp.ports_file().exists(),
        "reading status never creates a desktop listener"
    );
}

#[tokio::test]
async fn ensure_rechecks_binding_after_a_cached_entry_is_unbound() {
    let fixture = LaunchFixture::new("unbind-cached");
    let mut account = eligible_account("entry-unbind-cached");
    account.egress_proxy_url = Some("http://127.0.0.1:8080".to_string());
    fixture.save(&account);
    let first = ensure(&account.id)
        .await
        .expect("first launch")
        .expect("entry");
    let port = route_port(&account.id).await.expect("cached route");
    assert_eq!(
        ensure(&account.id).await.expect("repeat launch"),
        Some(first.clone())
    );

    account.egress_proxy_url = None;
    fixture.save(&account);
    assert!(ensure(&account.id)
        .await
        .expect("launch after unbind")
        .is_none());
    assert_eq!(route_port(&account.id).await, Some(port));
    TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .expect("old clients can still reach the retained listener");

    account.egress_proxy_url = Some("http://127.0.0.1:8081".to_string());
    fixture.save(&account);
    assert_eq!(
        ensure(&account.id).await.expect("launch after rebind"),
        Some(first)
    );
}

#[tokio::test]
async fn ensure_rechecks_unified_proxy_after_a_cached_entry_is_disabled() {
    let fixture = LaunchFixture::new("unified-disabled-cached");
    let account = eligible_account("entry-unified-disabled-cached");
    fixture.save(&account);
    codex_unified_proxy::enable(
        codex_unified_proxy::Reference::default(),
        "cockpit-proxy://desktop-router-test".to_string(),
    )
    .expect("enable fixture unified proxy");
    assert!(ensure(&account.id).await.expect("unified launch").is_some());
    let port = route_port(&account.id).await.expect("cached route");

    codex_unified_proxy::disable().expect("disable unified proxy");
    assert!(ensure(&account.id)
        .await
        .expect("launch after disable")
        .is_none());
    assert_eq!(route_port(&account.id).await, Some(port));
}

#[tokio::test]
async fn ensure_rechecks_account_eligibility_before_reusing_a_cached_entry() {
    let fixture = LaunchFixture::new("identity-changed-cached");
    let mut account = eligible_account("entry-identity-changed-cached");
    account.egress_proxy_url = Some("http://127.0.0.1:8080".to_string());
    fixture.save(&account);
    assert!(ensure(&account.id).await.expect("oauth launch").is_some());
    let port = route_port(&account.id).await.expect("cached route");

    account.api_provider_id = Some("vendor-x".to_string());
    fixture.save(&account);
    assert!(!codex_account_proxy::eligible(&account));
    assert!(ensure(&account.id)
        .await
        .expect("provider launch")
        .is_none());
    assert_eq!(route_port(&account.id).await, Some(port));
}

#[tokio::test]
async fn unsupported_account_does_not_load_unified_proxy_during_launch() {
    let fixture = LaunchFixture::new("unsupported-unified-error");
    let mut account = eligible_account("entry-unsupported-unified-error");
    account.api_provider_id = Some("vendor-x".to_string());
    fixture.save(&account);
    fs::write(fixture.temp.0.join("codex-unified-proxy.json"), "{broken")
        .expect("write invalid unified state");

    assert!(ensure(&account.id)
        .await
        .expect("unrelated proxy settings must not block provider launch")
        .is_none());
    assert!(route_port(&account.id).await.is_none());
}

#[tokio::test]
async fn non_eligible_account_gets_no_entry() {
    let temp = TempDir::new("ineligible");
    let path = temp.ports_file();
    let account_id = "entry-ineligible";
    let mut account = eligible_account(account_id);
    assert!(codex_account_proxy::eligible(&account));
    account.api_provider_id = Some("vendor-x".to_string());
    assert!(!codex_account_proxy::eligible(&account));

    assert!(ensure_for_account(account_id, &account, false, Some(&path))
        .await
        .expect("ineligible account")
        .is_none());
    assert!(route_port(account_id).await.is_none());
    assert!(!path.exists());
}

#[tokio::test]
async fn unbound_entry_dials_the_target_directly() {
    let origin = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("bind origin");
    let origin_port = origin.local_addr().expect("origin port").port();
    let echo = tokio::spawn(async move {
        let (mut socket, _) = origin.accept().await.expect("accept origin");
        let mut buffer = [0u8; 4];
        socket.read_exact(&mut buffer).await.expect("read payload");
        socket.write_all(&buffer).await.expect("echo payload");
    });

    let destination = Destination {
        host: Ipv4Addr::LOCALHOST.to_string(),
        port: origin_port,
    };
    let (mut upstream, lease) = dial(&destination, None).await.expect("direct dial");
    assert!(lease.is_none());
    upstream.write_all(b"ping").await.expect("write payload");
    let mut buffer = [0u8; 4];
    upstream.read_exact(&mut buffer).await.expect("read echo");
    assert_eq!(&buffer, b"ping");
    echo.await.expect("echo task");
}

#[tokio::test]
async fn socks_connect_keeps_the_requested_host_name() {
    let (mut client, mut server) = tokio::io::duplex(128);
    let peer = tokio::spawn(async move {
        server.write_all(&[5, 1, 0]).await.expect("write greeting");
        let mut reply = [0u8; 2];
        server.read_exact(&mut reply).await.expect("read method");
        assert_eq!(reply, [5, 0]);
        let host = b"edge.example.test";
        let mut request = vec![5, 1, 0, 3, host.len() as u8];
        request.extend_from_slice(host);
        request.extend_from_slice(&443u16.to_be_bytes());
        server.write_all(&request).await.expect("write request");
        tokio::time::sleep(Duration::from_millis(50)).await;
    });

    let destination = read_socks_request(&mut client)
        .await
        .expect("socks request");
    assert_eq!(destination.host, "edge.example.test");
    assert_eq!(destination.port, 443);
    peer.await.expect("socks peer");
}
