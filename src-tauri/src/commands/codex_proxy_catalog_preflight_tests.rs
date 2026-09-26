use super::*;
use std::{ffi::OsString, fs, path::PathBuf};

struct DataDir {
    path: PathBuf,
    previous: Vec<(&'static str, Option<OsString>)>,
}
impl DataDir {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("proxy-prerequisite-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(path.join("proxy-engine")).unwrap();
        let previous = ["COCKPIT_TOOLS_TEST_DATA_DIR", "COCKPIT_TOOLS_DATA_DIR"]
            .into_iter()
            .map(|key| {
                let old = std::env::var_os(key);
                std::env::set_var(key, &path);
                (key, old)
            })
            .collect();
        Self { path, previous }
    }
}
impl Drop for DataDir {
    fn drop(&mut self) {
        for (key, old) in self.previous.drain(..) {
            if let Some(old) = old {
                std::env::set_var(key, old);
            } else {
                std::env::remove_var(key);
            }
        }
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn latency_precancel_is_processed_before_engine_preflight_or_catalog_reads() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = DataDir::new();
    fs::write(dir.path.join("proxy-engine/active.json"), b"broken install record").unwrap();
    let request_id = uuid::Uuid::new_v4().to_string();
    catalog::cancel(request_id.clone()).unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let error = codex_proxy_catalog_latency(
                request_id,
                "must-not-load-source".into(),
                "must-not-test-node".into(),
                "revision".into(),
                None,
            )
            .await
            .err()
            .unwrap();
            assert_eq!(error, "CATALOG_CANCELLED");
        });
    assert!(!dir.path.join("codex-proxy-sources.json").exists());
    assert_eq!(fs::read(dir.path.join("proxy-engine/active.json")).unwrap(), b"broken install record");
}

#[test]
fn damaged_engine_blocks_import_before_network_or_existing_catalog_write() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = DataDir::new();
    let catalog = dir.path.join("codex-proxy-sources.json");
    let account = dir.path.join("retained-account.json");
    fs::write(&catalog, b"existing encrypted catalog").unwrap();
    fs::write(&account, b"existing account data").unwrap();
    let active = dir.path.join("proxy-engine/active.json");
    fs::write(&active, b"broken install record").unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let input = format!("https://{}/subscription", listener.local_addr().unwrap());
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let error = codex_proxy_catalog_import(
                uuid::Uuid::new_v4().to_string(),
                "New source".into(),
                input,
                "subscription".into(),
                None,
            )
            .await
            .err()
            .unwrap();
            assert_eq!(error, "ENGINE_INSTALL_VERIFY");
            assert!(crate::modules::codex_proxy_engine_preflight::for_url(
                "http://user:secret@127.0.0.1:8080",
                crate::modules::codex_proxy_engine_preflight::Usage::AccountRequest,
            )
            .await
            .is_ok());
        });
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert_eq!(fs::read(&catalog).unwrap(), b"existing encrypted catalog");
    assert_eq!(fs::read(&account).unwrap(), b"existing account data");
    assert_eq!(fs::read(&active).unwrap(), b"broken install record");
}

#[test]
fn invalid_bindings_preserve_account_but_network_failure_does_not_block_saving() {
    use crate::models::codex::{CodexAccount, CodexTokens};
    use crate::modules::{
        codex_account, codex_proxy_engine_preflight as preflight, codex_proxy_runtime as runtime,
    };
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = DataDir::new();
    assert_eq!(
        crate::modules::account::resolve_data_dir().unwrap(),
        dir.path
    );
    fs::write(
        dir.path.join("proxy-engine/active.json"),
        b"broken install record",
    )
    .unwrap();
    let mut account = CodexAccount::new(
        uuid::Uuid::new_v4().to_string(),
        "test@example.com".into(),
        CodexTokens {
            access_token: "access".into(),
            id_token: "id".into(),
            refresh_token: Some("refresh".into()),
        },
    );
    account.egress_proxy_url = Some("http://127.0.0.1:8080".into());
    account.requires_reauth = true;
    account.reauth_reason = Some("existing authorization state".into());
    codex_account::save_account(&account).unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let error = runtime::save_binding(
                account.id.clone(),
                Some("trojan://secret@node.example:443".into()),
            )
            .await
            .err()
            .unwrap();
            assert_eq!(error, "ENGINE_INSTALL_VERIFY");
            assert_eq!(
                codex_account::format_account_switch_error(&account.id, error.clone()),
                error
            );
            let preserved = codex_account::load_account(&account.id).unwrap();
            assert_eq!(preserved.egress_proxy_url, account.egress_proxy_url);
            assert!(preserved.requires_reauth);
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = format!("http://user:secret@{}/", listener.local_addr().unwrap());
            let saved = tokio::time::timeout(
                std::time::Duration::from_secs(3),
                runtime::save_binding(account.id.clone(), Some(address.clone())),
            )
            .await.expect("saving must not wait for an external connectivity check")
            .expect("a valid configuration can be saved without proving connectivity");
            assert_eq!(saved.egress_proxy_url.as_deref(), Some(address.as_str()));
            assert_eq!(listener.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock,
                "saving must not send a request through the configured proxy");
            assert_eq!(codex_account::load_account(&account.id).unwrap().egress_proxy_url, saved.egress_proxy_url);
            assert!(saved.requires_reauth);
            assert_eq!(saved.reauth_reason, account.reauth_reason);

            let invalid = runtime::save_binding(account.id.clone(), Some("http://127.0.0.1:0".into()))
                .await.err().expect("invalid configuration must still be rejected");
            assert_eq!(invalid, "PROXY_INVALID_URL");
            assert_eq!(codex_account::load_account(&account.id).unwrap().egress_proxy_url, saved.egress_proxy_url);

            // Only the explicit test contacts the IP service. A failed test must
            // report an error without removing the independently saved binding.
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            let proxy = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let read = socket.read(&mut request).await.unwrap();
                assert!(String::from_utf8_lossy(&request[..read]).starts_with("CONNECT api64.ipify.org:443"));
                socket.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
            });
            let raw_error = crate::modules::codex_proxy_probe::probe(
                account.id.clone(), uuid::Uuid::new_v4().to_string(), None,
            ).await.err().expect("the explicit connectivity check must still report failure");
            proxy.await.unwrap();
            assert!(!preflight::is_prerequisite_error(&raw_error));
            assert_eq!(codex_account::load_account(&account.id).unwrap().egress_proxy_url, saved.egress_proxy_url);
            assert!(codex_account::load_account(&account.id).unwrap().requires_reauth);
            // The explicit raw test reached the proxy without an engine. Desktop forwarding
            // of the same authenticated address still requires the private bridge.
            assert_eq!(preflight::for_url(&address, preflight::Usage::Desktop).await.unwrap_err(), "ENGINE_INSTALL_VERIFY");
            let cleared = runtime::save_binding(account.id.clone(), None)
                .await
                .unwrap();
            assert!(cleared.egress_proxy_url.is_none());
            runtime::release_deleted_account(&account.id);
            tokio::task::yield_now().await;
        });
}

#[test]
fn default_and_managed_instance_preflight_requires_engine_only_for_desktop_mode() {
    use crate::models::{
        codex::{CodexAccount, CodexTokens},
        InstanceLaunchMode,
    };
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = DataDir::new();
    fs::write(
        dir.path.join("proxy-engine/active.json"),
        b"broken install record",
    )
    .unwrap();
    let mut account = CodexAccount::new(
        uuid::Uuid::new_v4().to_string(),
        "test@example.com".into(),
        CodexTokens {
            access_token: "access".into(),
            id_token: "id".into(),
            refresh_token: Some("refresh".into()),
        },
    );
    // API/CLI do not need a desktop credential bridge for this raw HTTP address.
    account.egress_proxy_url = Some("http://user:secret@127.0.0.1:8080".into());
    crate::modules::codex_account::save_account(&account).unwrap();
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        for (mode, expected_mode) in [("cli", InstanceLaunchMode::Cli), ("app", InstanceLaunchMode::App)] {
            let store = serde_json::json!({
                "defaultSettings": {"bindAccountId": account.id, "followLocalAccount": false, "launchMode": mode},
                "instances": [{
                    "id": "managed-prerequisite", "name": "Test", "userDataDir": dir.path.join("managed"),
                    "extraArgs": "", "bindAccountId": account.id, "launchMode": mode,
                    "createdAt": 0, "lastLaunchedAt": null
                }]
            });
            let store_path = dir.path.join("codex_instances.json");
            let bytes = serde_json::to_vec(&store).unwrap();
            fs::write(&store_path, &bytes).unwrap();
            for id in ["__default__", "managed-prerequisite"] {
                let target = crate::commands::codex_instance::resolve_codex_instance_start_target(id).unwrap();
                assert_eq!(target.launch_mode, expected_mode);
                assert_eq!(target.is_default, id == "__default__");
                let result = crate::commands::codex_proxy_engine::codex_proxy_instance_preflight(id.into()).await;
                if expected_mode == InstanceLaunchMode::Cli {
                    result.expect("CLI must not require a desktop proxy bridge");
                } else {
                    assert_eq!(result.unwrap_err(), "ENGINE_INSTALL_VERIFY");
                }
                assert_eq!(fs::read(&store_path).unwrap(), bytes, "preflight must not rewrite settings");
            }
        }
    });
}
