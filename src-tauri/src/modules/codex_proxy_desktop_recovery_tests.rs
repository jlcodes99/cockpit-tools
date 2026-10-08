use super::super::{entry_status, install, ROUTES};
use super::*;
use crate::models::codex::{CodexAccount, CodexTokens};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

fn running(pid: u32, home: Option<&str>, port: u16) -> processes::RunningEntry {
    processes::RunningEntry {
        pid,
        profile_dir: home.map(str::to_owned),
        proxy_port: port,
    }
}

fn matches(entry: &processes::RunningEntry, home: Option<&str>) -> bool {
    entry.profile_dir.as_deref() == home
}

#[test]
fn recovery_requires_own_record_and_matching_running_default_or_managed_profile() {
    let mut store = InstanceStore::new();
    store.default_settings.bind_account_id = Some("default-account".into());
    store.default_settings.last_pid = Some(1);
    store.instances.push(
        serde_json::from_value(serde_json::json!({
            "id": "instance", "name": "test", "userDataDir": "/own/codex", "extraArgs": "",
            "bindAccountId": "managed-account", "createdAt": 0, "lastLaunchedAt": null, "lastPid": 2
        }))
        .unwrap(),
    );
    let mut registry = PortRegistry::default();
    registry.record("default-account", 45001);
    registry.record("managed-account", 45002);
    let processes = [
        running(1, None, 45001),
        running(2, Some("/own/codex"), 45002),
        running(3, Some("/other-environment/codex"), 45002),
        running(4, None, 45003),
    ];
    assert_eq!(
        select_targets(&registry, &store, &processes, matches),
        vec![
            Target {
                account_id: "default-account".into(),
                port: 45001
            },
            Target {
                account_id: "managed-account".into(),
                port: 45002
            },
        ]
    );
    assert!(select_targets(&registry, &store, &processes[2..], matches).is_empty());
    assert!(select_targets(&PortRegistry::default(), &store, &processes, matches).is_empty());
    assert!(select_targets(&registry, &store, &[], matches).is_empty());
    store.default_settings.last_pid = Some(999); // Another host's default desktop.
    assert_eq!(
        select_targets(&registry, &store, &processes, matches).len(),
        1
    );
    store.instances[0].last_pid = Some(998);
    assert!(select_targets(&registry, &store, &processes, matches).is_empty());
}

#[test]
fn pending_next_launch_binding_does_not_reroute_a_running_desktop() {
    let mut store = InstanceStore::new();
    store.default_settings.last_pid = Some(1);
    store.default_settings.bind_account_id = Some("__api_service__".into());
    store.default_settings.launch_mode = crate::models::InstanceLaunchMode::Cli;
    let mut registry = PortRegistry::default();
    registry.record("original-account", 45001);
    let targets = select_targets(&registry, &store, &[running(1, None, 45001)], matches);
    assert_eq!(
        targets,
        vec![Target {
            account_id: "original-account".into(),
            port: 45001
        }]
    );
}

#[test]
fn duplicate_desktops_are_deduplicated_and_ambiguous_account_ports_are_rejected() {
    let mut store = InstanceStore::new();
    store.default_settings.bind_account_id = Some("a".into());
    store.default_settings.last_pid = Some(1);
    let mut registry = PortRegistry::default();
    registry.record("a", 45001);
    let running = [running(1, None, 45001), running(2, None, 45001)];
    assert_eq!(
        select_targets(&registry, &store, &running, matches).len(),
        1
    );
    store.instances.push(
        serde_json::from_value(serde_json::json!({
            "id": "ambiguous", "name": "test", "userDataDir": "/same", "extraArgs": "",
            "bindAccountId": "b", "createdAt": 0, "lastLaunchedAt": null
        }))
        .unwrap(),
    );
    registry.record("b", 45001);
    assert!(select_targets(&registry, &store, &running, |_, _| true).is_empty());
}

#[tokio::test]
async fn recovery_reclaims_exact_port_and_is_idempotent() {
    let id = format!("restart-{}", uuid::Uuid::new_v4());
    let (old_host, port) = install(&id, None).await.unwrap();
    drop(old_host); // The old host process is gone; no in-memory route survives it.
    restore_exact_port(&id, port).await.unwrap();
    restore_exact_port(&id, port).await.unwrap();
    assert_eq!(ROUTES.lock().await.get(&id), Some(&port));
    assert_eq!(entry_status(&id).unwrap().state, "listening");
    let mut client = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    client.write_all(&[5, 1, 0]).await.unwrap();
    let mut reply = [0; 2];
    tokio::time::timeout(Duration::from_secs(2), client.read_exact(&mut reply))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reply, [5, 0]);
}

#[tokio::test]
async fn occupied_port_is_never_replaced_and_can_be_retried_after_release() {
    let id = format!("restore-conflict-{}", uuid::Uuid::new_v4());
    let (other_process, port) = install(&id, None).await.unwrap();
    assert_eq!(
        restore_exact_port(&id, port).await.unwrap_err(),
        "PROXY_ENTRY_PORT_UNAVAILABLE"
    );
    assert!(!ROUTES.lock().await.contains_key(&id));
    let status = entry_status(&id).unwrap();
    assert_eq!(status.state, "failed");
    assert_eq!(status.port, Some(port));
    drop(other_process);
    restore_exact_port(&id, port).await.unwrap();
    assert_eq!(entry_status(&id).unwrap().port, Some(port));
    assert_eq!(entry_status(&id).unwrap().state, "listening");
}

#[tokio::test]
async fn restoring_a_previous_port_cannot_replace_a_new_active_route() {
    let id = format!("restore-new-route-{}", uuid::Uuid::new_v4());
    let (reserved, port) = install(&id, None).await.unwrap();
    let (another, other_port) = install(&id, None).await.unwrap();
    drop(another);
    restore_exact_port(&id, other_port).await.unwrap();
    drop(reserved);
    assert_eq!(
        restore_exact_port(&id, port).await.unwrap_err(),
        "PROXY_ENTRY_PORT_UNAVAILABLE"
    );
    assert_eq!(entry_status(&id).unwrap().port, Some(other_port));
}

#[tokio::test]
async fn restored_entry_uses_current_direct_mode_and_forwards_without_client_restart() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let temp = std::env::temp_dir().join(format!("proxy-recovery-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&temp).unwrap();
    struct Cleanup {
        path: std::path::PathBuf,
        previous: Option<std::ffi::OsString>,
    }
    impl Drop for Cleanup {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", value),
                None => std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR"),
            }
            let _ = fs::remove_dir_all(&self.path);
        }
    }
    let _cleanup = Cleanup {
        path: temp.clone(),
        previous: std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR"),
    };
    std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &temp);
    let id = format!("restored-direct-{}", uuid::Uuid::new_v4());
    let mut account = CodexAccount::new(
        id.clone(),
        "restore@example.test".into(),
        CodexTokens {
            access_token: "test".into(),
            id_token: "test".into(),
            refresh_token: None,
        },
    );
    account.egress_proxy_disabled = true;
    account.egress_proxy_url = Some("http://127.0.0.1:1".into()); // Retired route must never be used.
    crate::modules::codex_account::save_account(&account).unwrap();
    let (old_host, port) = install(&id, None).await.unwrap();
    drop(old_host);
    let echo = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let target_port = echo.local_addr().unwrap().port();
    let responder = tokio::spawn(async move {
        let (mut socket, _) = echo.accept().await.unwrap();
        let mut buf = [0; 4];
        socket.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"ping");
        socket.write_all(b"pong").await.unwrap();
    });
    restore_target(&Target {
        account_id: id,
        port,
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        let mut client = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
            .await
            .unwrap();
        client.write_all(&[5, 1, 0]).await.unwrap();
        let mut greeting = [0; 2];
        client.read_exact(&mut greeting).await.unwrap();
        let mut request = vec![5, 1, 0, 1, 127, 0, 0, 1];
        request.extend_from_slice(&target_port.to_be_bytes());
        client.write_all(&request).await.unwrap();
        let mut reply = [0; 10];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply[1], 0);
        client.write_all(b"ping").await.unwrap();
        let mut response = [0; 4];
        client.read_exact(&mut response).await.unwrap();
        assert_eq!(&response, b"pong");
        responder.await.unwrap();
    })
    .await
    .unwrap();
}
