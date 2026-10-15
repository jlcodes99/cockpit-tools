//! Exercise the real runtime/entry lifecycle with an isolated child test process.
//! The fake engine only accepts loopback sockets and never resolves destinations.
use super::*;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;

const CHILD_TEST: &str =
    "modules::codex_proxy_desktop_router::tests::unified_runtime::fake_engine_child";

fn shell_word(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn install_fake_engine(fixture: &LaunchFixture) -> PathBuf {
    let root = fixture.temp.0.join("proxy-engine");
    let directory = format!("release-{}", uuid::Uuid::new_v4());
    let release = root.join(&directory);
    fs::create_dir_all(&release).unwrap();
    let starts = fixture.temp.0.join("engine-starts");
    let executable = std::env::current_exe().unwrap();
    // Reuse this Rust test binary, avoiding a dependency on Python/Node/a real engine.
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = -v ]; then\n printf 'Mihomo Meta v{}\\n'\n exit 0\nfi\nexport COCKPIT_UNIFIED_TEST_ENGINE={}\nexec {} --exact {} --nocapture\n",
        crate::modules::codex_proxy_engine::ENGINE_VERSION,
        shell_word(starts.to_str().unwrap()),
        shell_word(executable.to_str().unwrap()),
        shell_word(CHILD_TEST),
    );
    let binary = release.join("mihomo");
    fs::write(&binary, &script).unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    let target = format!(
        "{}-{}",
        std::env::consts::ARCH,
        if cfg!(target_os = "macos") {
            "apple-darwin"
        } else {
            "unknown-linux-gnu"
        }
    );
    let manifest: serde_json::Value = serde_json::from_str(include_str!(
        "../../../sidecars/mihomo/upstream-assets.json"
    ))
    .unwrap();
    let record = serde_json::json!({
        "directory": directory,
        "version": crate::modules::codex_proxy_engine::ENGINE_VERSION,
        "target": target,
        "archive_sha256": manifest["assets"][&target]["sha256"],
        "files": {"mihomo": format!("{:x}", Sha256::digest(script.as_bytes()))},
    });
    let record = serde_json::to_vec(&record).unwrap();
    fs::write(release.join("installed.json"), &record).unwrap();
    fs::write(release.join(".lease"), "").unwrap();
    fs::write(root.join("active.json"), record).unwrap();
    starts
}

#[test]
fn fake_engine_child() {
    let Some(starts) = std::env::var_os("COCKPIT_UNIFIED_TEST_ENGINE") else {
        return;
    };
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    let config: serde_json::Value = serde_json::from_str(&input).unwrap();
    let marker = config["proxies"][0]["password"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(starts)
        .unwrap();
    writeln!(log, "{marker}").unwrap();
    drop(log);
    if marker == "fail" {
        std::process::exit(9);
    }
    let port = config["mixed-port"].as_u64().unwrap() as u16;
    let credentials = config["authentication"][0].as_str().unwrap().to_owned();
    let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).unwrap();
    for connection in listener.incoming() {
        let mut socket = connection.unwrap();
        let credentials = credentials.clone();
        let marker = marker.clone();
        std::thread::spawn(move || {
            // Startup verification only greets/authenticates; real test requests CONNECT.
            let _ = (|| -> std::io::Result<()> {
                socket.set_read_timeout(Some(Duration::from_secs(10)))?;
                let mut greeting = [0; 3];
                socket.read_exact(&mut greeting)?;
                assert_eq!(greeting, [5, 1, 2]);
                socket.write_all(&[5, 2])?;
                let mut header = [0; 2];
                socket.read_exact(&mut header)?;
                assert_eq!(header[0], 1);
                let mut username = vec![0; header[1] as usize];
                socket.read_exact(&mut username)?;
                let mut length = [0];
                socket.read_exact(&mut length)?;
                let mut password = vec![0; length[0] as usize];
                socket.read_exact(&mut password)?;
                assert_eq!(
                    format!(
                        "{}:{}",
                        String::from_utf8(username).unwrap(),
                        String::from_utf8(password).unwrap()
                    ),
                    credentials
                );
                socket.write_all(&[1, 0])?;
                let mut connect = [0; 5];
                socket.read_exact(&mut connect)?;
                assert_eq!(
                    &connect[..4],
                    &[5, 1, 0, 3],
                    "domain must reach upstream without local DNS"
                );
                let mut host = vec![0; connect[4] as usize];
                socket.read_exact(&mut host)?;
                assert_eq!(&host, b"fixture.invalid");
                let mut port = [0; 2];
                socket.read_exact(&mut port)?;
                assert_eq!(u16::from_be_bytes(port), 443);
                socket.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])?;
                loop {
                    let mut ping = [0; 4];
                    socket.read_exact(&mut ping)?;
                    assert_eq!(&ping, b"ping");
                    socket.write_all(marker.as_bytes())?;
                }
            })();
        });
    }
}

struct RuntimeCleanup(String);
impl Drop for RuntimeCleanup {
    fn drop(&mut self) {
        codex_proxy_runtime::release_deleted_account(&self.0);
    }
}

async fn connect_entry(proxy: &str) -> TcpStream {
    let port = url::Url::parse(proxy).unwrap().port().unwrap();
    let mut socket = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    socket.write_all(&[5, 1, 0]).await.unwrap();
    let mut greeting = [0; 2];
    socket.read_exact(&mut greeting).await.unwrap();
    assert_eq!(greeting, [5, 0]);
    let host = b"fixture.invalid";
    let mut request = vec![5, 1, 0, 3, host.len() as u8];
    request.extend_from_slice(host);
    request.extend_from_slice(&443u16.to_be_bytes());
    socket.write_all(&request).await.unwrap();
    let mut reply = [0; 10];
    socket.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply[1], 0);
    socket
}

async fn response(socket: &mut TcpStream, expected: &[u8; 4]) {
    socket.write_all(b"ping").await.unwrap();
    let mut value = [0; 4];
    socket.read_exact(&mut value).await.unwrap();
    assert_eq!(&value, expected);
}

async fn wait_closed(proxy: &str) {
    let port = url::Url::parse(proxy).unwrap().port().unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while TcpStream::connect((Ipv4Addr::LOCALHOST, port))
            .await
            .is_ok()
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("unleased engine process must exit");
}

#[tokio::test]
async fn concurrent_consumers_share_entry_engine_and_binding_switch_preserves_old_streams() {
    let fixture = LaunchFixture::new("single-runtime");
    let starts = install_fake_engine(&fixture);
    let mut account = eligible_account("single-runtime");
    account.egress_proxy_url = Some("trojan://old1@fixture.invalid:443".into());
    fixture.save(&account);
    let cleanup = RuntimeCleanup(account.id.clone());
    assert_eq!(
        codex_proxy_runtime::prepared_url(&account).unwrap_err(),
        "PROXY_RUNTIME_NOT_READY"
    );
    tokio::time::timeout(Duration::from_secs(20), async {
        let (first, sidecar, second, desktop) = tokio::join!(
            codex_proxy_runtime::ensure(&account.id),
            codex_proxy_runtime::ensure_sidecar(&account.id),
            codex_proxy_runtime::ensure(&account.id),
            ensure(&account.id),
        );
        let entry = first.unwrap().unwrap();
        for url in [sidecar, second, desktop] {
            assert_eq!(url.unwrap(), Some(entry.clone()));
        }
        assert_eq!(
            fs::read_to_string(&starts).unwrap(),
            "old1\n",
            "concurrent consumers start exactly one engine"
        );
        assert_eq!(
            codex_proxy_runtime::prepared_url(&account).unwrap(),
            Some(entry.clone())
        );
        assert_eq!(
            codex_proxy_runtime::prepared_sidecar_url(&account).unwrap(),
            Some(entry.clone())
        );
        let status = codex_proxy_runtime::status(&account.id).await.unwrap();
        assert!(status.shared_entry);
        assert_eq!(
            [status.account, status.desktop, status.sidecar],
            ["running"; 3]
        );
        assert_eq!(
            [
                status.account_port,
                status.desktop_port,
                status.sidecar_port
            ],
            [url::Url::parse(&entry).unwrap().port(); 3]
        );
        let controllers = codex_proxy_runtime::active_controllers(&account.id).await;
        assert_eq!(controllers.len(), 1);
        let (old_upstream, old_lease) = codex_proxy_runtime::desktop_target(&account.id)
            .await
            .unwrap()
            .unwrap();
        assert_ne!(
            old_upstream, entry,
            "router upstream must not recurse into its own entry"
        );
        let old_lease = old_lease.unwrap();
        assert_eq!(old_lease.controller().tunnel_id, controllers[0].1.tunnel_id);
        let mut old_stream = connect_entry(&entry).await;
        response(&mut old_stream, b"old1").await;

        let updated = codex_proxy_runtime::save_binding(
            account.id.clone(),
            Some("trojan://new2@fixture.invalid:443".into()),
        )
        .await
        .unwrap();
        assert_eq!(
            codex_proxy_runtime::ensure_sidecar(&account.id)
                .await
                .unwrap(),
            Some(entry.clone())
        );
        assert_eq!(
            codex_proxy_runtime::prepared_url(&updated).unwrap(),
            Some(entry.clone())
        );
        let (new_upstream, new_lease) = codex_proxy_runtime::desktop_target(&account.id)
            .await
            .unwrap()
            .unwrap();
        assert_ne!(old_upstream, new_upstream);
        assert!(
            old_lease.is_running(),
            "active requests retain the previous engine lease"
        );
        drop(old_lease);
        response(&mut old_stream, b"old1").await;
        let mut new_stream = connect_entry(&entry).await;
        response(&mut new_stream, b"new2").await;
        assert_eq!(fs::read_to_string(&starts).unwrap(), "old1\nnew2\n");

        let error = codex_proxy_runtime::save_binding(
            account.id.clone(),
            Some("trojan://fail@fixture.invalid:443".into()),
        )
        .await
        .err()
        .unwrap();
        assert_eq!(error, "PROXY_ENGINE_START_FAILED");
        let persisted = codex_proxy_runtime::load(&account.id).await.unwrap();
        assert_eq!(
            persisted.egress_proxy_url, updated.egress_proxy_url,
            "failed candidate must not overwrite durable binding"
        );
        let (retained, lease) = codex_proxy_runtime::desktop_target(&account.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retained, new_upstream);
        assert_eq!(
            codex_proxy_runtime::prepared_url(&persisted).unwrap(),
            Some(entry.clone())
        );
        response(&mut old_stream, b"old1").await;
        response(&mut new_stream, b"new2").await;
        let mut after_failure = connect_entry(&entry).await;
        response(&mut after_failure, b"new2").await;
        drop(after_failure);
        drop(old_stream);
        wait_closed(&old_upstream).await;
        drop(new_stream);
        drop(new_lease);
        drop(lease);
        drop(cleanup);
        wait_closed(&new_upstream).await;
    })
    .await
    .expect("single-engine lifecycle completes within a bounded deadline");
}

#[tokio::test]
async fn direct_proxy_manifest_requires_preparation_and_all_consumers_share_native_entry() {
    let fixture = LaunchFixture::new("direct-native-entry");
    for (index, binding) in [
        "http://127.0.0.1:8080",
        "http://user:secret@127.0.0.1:8080",
        "https://user:secret@127.0.0.1:8443",
        "socks5://user:secret@127.0.0.1:1080",
        "socks5h://127.0.0.1:1080",
    ]
    .into_iter()
    .enumerate()
    {
        let mut account = eligible_account(&format!("direct-native-entry-{index}"));
        account.egress_proxy_url = Some(binding.into());
        fixture.save(&account);
        assert_eq!(
            codex_proxy_runtime::prepared_sidecar_url(&account).unwrap_err(),
            "PROXY_RUNTIME_NOT_READY"
        );
        let entry = codex_proxy_runtime::ensure(&account.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            codex_proxy_runtime::ensure_sidecar(&account.id)
                .await
                .unwrap(),
            Some(entry.clone())
        );
        assert_eq!(ensure(&account.id).await.unwrap(), Some(entry.clone()));
        assert_eq!(
            codex_proxy_runtime::prepared_sidecar_url(&account).unwrap(),
            Some(entry.clone())
        );
        let (upstream, lease) = codex_proxy_runtime::desktop_target(&account.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            upstream,
            codex_account_proxy::normalize_direct_proxy(binding).unwrap()
        );
        assert!(lease.is_none());
        assert!(codex_proxy_runtime::active_controllers(&account.id)
            .await
            .is_empty());
        let status = codex_proxy_runtime::status(&account.id).await.unwrap();
        assert_eq!(
            [status.account, status.desktop, status.sidecar],
            ["direct"; 3]
        );
        assert_eq!(
            [
                status.account_port,
                status.desktop_port,
                status.sidecar_port
            ],
            [url::Url::parse(&entry).unwrap().port(); 3]
        );
    }
    assert!(
        !fixture.temp.0.join("proxy-engine").exists(),
        "native direct proxy must not install or start an engine"
    );
}

#[tokio::test]
async fn concurrent_entries_keep_both_port_records_and_slow_storage_does_not_block_ready_entry() {
    let fixture = LaunchFixture::new("entry-concurrent-persistence");
    let path = fixture.temp.ports_file();
    let ready_id = "entry-concurrent-ready";
    let ready = ensure_route(ready_id, Some(&path)).await.unwrap();
    // Simulate a slow registry write only, without blocking an async runtime worker.
    let storage_guard = REGISTRY_WRITES
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let pending_path = path.clone();
    let mut pending =
        tokio::spawn(
            async move { ensure_route("entry-concurrent-pending", Some(&pending_path)).await },
        );
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut pending)
            .await
            .is_err()
    );
    assert_eq!(
        tokio::time::timeout(
            Duration::from_millis(250),
            ensure_route(ready_id, Some(&path))
        )
        .await
        .expect("ready account entry is independent of another account's storage")
        .unwrap(),
        ready,
    );
    drop(storage_guard);
    let pending_entry = pending.await.unwrap().unwrap();
    let (first, second) = tokio::join!(
        ensure_route("entry-concurrent-first", Some(&path)),
        ensure_route("entry-concurrent-second", Some(&path)),
    );
    let registry = read_registry(&path);
    for (id, entry) in [
        (ready_id, ready),
        ("entry-concurrent-pending", pending_entry),
        ("entry-concurrent-first", first.unwrap()),
        ("entry-concurrent-second", second.unwrap()),
    ] {
        assert_eq!(
            registry.recorded(id),
            url::Url::parse(&entry).unwrap().port()
        );
    }
}
