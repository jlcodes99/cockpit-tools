use super::*;

struct Fixture {
    root: tempfile::TempDir,
    previous: [Option<std::ffi::OsString>; 2],
    _env: std::sync::MutexGuard<'static, ()>,
}
impl Fixture {
    fn new() -> Self {
        let env = super::super::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let root = tempfile::tempdir().unwrap();
        let keys = ["COCKPIT_TOOLS_TEST_DATA_DIR", "COCKPIT_TOOLS_DATA_DIR"];
        let previous = keys.map(std::env::var_os);
        for key in keys {
            std::env::set_var(key, root.path());
        }
        Self {
            root,
            previous,
            _env: env,
        }
    }
    fn account(&self, name: &str) -> Account {
        account::upsert_account(
            format!("{name}@fixture.invalid"),
            Some(name.into()),
            crate::models::token::TokenData::new(
                format!("fixture-access-{name}"),
                format!("fixture-refresh-{name}"),
                3600,
                None,
                None,
                None,
            ),
        )
        .unwrap()
    }
    fn saved(&self, id: &str, source: &str) -> String {
        let catalog = super::super::codex_proxy_subscription_parser::parse(
            "http://fixture-user:PRIVATE_PASSWORD@node.invalid:8080#Fixture",
        )
        .unwrap();
        let raw = binding::encode(
            source,
            "Fixture source",
            &catalog.nodes[0].id,
            &catalog,
            &BTreeMap::new(),
        )
        .unwrap();
        let decoded = binding::decode(&raw).unwrap();
        let setting = Setting {
            snapshot: Some(raw.clone()),
            reference: Some(super::super::codex_unified_proxy::Reference {
                source_id: decoded.source_id,
                source_name: decoded.source_name,
                item_id: decoded.item_id,
                name: decoded.name,
                group_id: None,
                selections: BTreeMap::new(),
            }),
            error: None,
            revision: 1,
        };
        mutate(|store| {
            store.targets.insert(id.into(), setting);
            Ok(())
        })
        .unwrap();
        raw
    }
    fn catalog(&self, source: &str, item: &str, password: &str) {
        let mut parsed = super::super::codex_proxy_subscription_parser::parse(&format!(
            "http://fixture-user:{password}@node.invalid:8080#Fixture"
        ))
        .unwrap();
        parsed.nodes[0].id = item.into();
        let value = serde_json::json!({"version":1,"sources":[{
            "id":source,"name":"Fixture source","kind":"manual","url":null,
            "catalog":parsed,"updated_at":1,"last_attempt_at":null,"auto_update":false,
            "error":null,"revision":"v1","parser_version":4
        }]});
        let encrypted =
            secure_account_storage::serialize_account_file("codex-proxy-sources", &value).unwrap();
        atomic_write::write_secret_string_atomic(
            &self.root.path().join("codex-proxy-sources.json"),
            &encrypted,
        )
        .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for (key, old) in ["COCKPIT_TOOLS_TEST_DATA_DIR", "COCKPIT_TOOLS_DATA_DIR"]
            .into_iter()
            .zip(&self.previous)
        {
            match old {
                Some(old) => std::env::set_var(key, old),
                None => std::env::remove_var(key),
            }
        }
    }
}

#[tokio::test]
async fn missing_store_preserves_legacy_then_clear_tombstone_prevents_resurrection() {
    let fixture = Fixture::new();
    let mut account = fixture.account("legacy");
    assert_eq!(launch_url(&account.id).await.unwrap(), None);
    assert!(!path().unwrap().exists());
    account.egress_proxy_url = Some("http://user:PRIVATE_LEGACY@127.0.0.1:8080".into());
    account::save_account(&account).unwrap();
    assert!(get(account.id.clone()).await.unwrap().legacy_bound);
    assert!(launch_url(&account.id)
        .await
        .unwrap()
        .unwrap()
        .contains("PRIVATE_LEGACY"));
    let public = serde_json::to_string(&get(account.id.clone()).await.unwrap()).unwrap();
    assert!(!public.contains("PRIVATE_LEGACY"));
    clear(account.id.clone()).await.unwrap();
    // Even an old credential-refresh snapshot cannot resurrect the discarded URL.
    account::save_account(&account).unwrap();
    assert_eq!(launch_url(&account.id).await.unwrap(), None);
    assert!(!get(account.id.clone()).await.unwrap().legacy_bound);
    assert!(
        super::super::codex_proxy_runtime::load(&route_id(&account.id))
            .await
            .unwrap()
            .egress_proxy_disabled
    );
}

#[tokio::test]
async fn encrypted_snapshot_is_redacted_and_stale_reference_never_falls_back_to_legacy() {
    let fixture = Fixture::new();
    let mut account = fixture.account("secret");
    account.egress_proxy_url = Some("http://127.0.0.1:9".into());
    account::save_account(&account).unwrap();
    let raw = fixture.saved(&account.id, "missing-source");
    let encrypted = std::fs::read_to_string(path().unwrap()).unwrap();
    for secret in [&raw, "PRIVATE_PASSWORD", "node.invalid", "cockpit-proxy://"] {
        assert!(!encrypted.contains(secret));
    }
    assert_eq!(
        read().unwrap().targets[&account.id].snapshot.as_deref(),
        Some(raw.as_str())
    );
    let response = get(account.id.clone()).await.unwrap();
    assert_eq!(response.error.as_deref(), Some(STALE));
    assert!(!response.legacy_bound);
    let response = serde_json::to_string(&response).unwrap();
    for secret in [&raw, "PRIVATE_PASSWORD", "node.invalid", "snapshot"] {
        assert!(!response.contains(secret));
    }
    assert_eq!(launch_url(&account.id).await.unwrap_err(), STALE);
    assert!(
        super::super::codex_proxy_runtime::load(&route_id(&account.id))
            .await
            .is_err()
    );
    assert!(
        fixture
            .root
            .path()
            .join("codex_accounts")
            .try_exists()
            .unwrap_or(false)
            == false
    );
}

#[tokio::test]
async fn invalid_selection_preserves_previous_snapshot_and_other_account() {
    let fixture = Fixture::new();
    let a = fixture.account("first");
    let b = fixture.account("second");
    assert_ne!(a.id, b.id, "isolation fixtures must be distinct accounts");
    fixture.saved(&a.id, "saved-source");
    fixture.saved(&b.id, "other-source");
    let before = std::fs::read(path().unwrap()).unwrap();
    assert!(bind(
        a.id.clone(),
        "missing-source".into(),
        "missing-node".into(),
        BTreeMap::new(),
        None
    )
    .await
    .is_err());
    assert_eq!(std::fs::read(path().unwrap()).unwrap(), before);
    clear(a.id.clone()).await.unwrap();
    let store = read().unwrap();
    assert!(store.targets[&a.id].snapshot.is_none());
    assert!(store.targets[&b.id].snapshot.is_some());
}

#[tokio::test]
async fn refresh_retains_saved_parameters_and_missing_item_marks_stale() {
    let fixture = Fixture::new();
    let a = fixture.account("refresh");
    let raw = fixture.saved(&a.id, "source");
    let item = binding::decode(&raw).unwrap().item_id;
    fixture.catalog("source", &item, "CHANGED_PASSWORD");
    assert!(catalog::reference_exists("source", &item).await.unwrap());
    resync_source("source", false).await.unwrap();
    let store = read().unwrap();
    assert_eq!(store.targets[&a.id].snapshot.as_deref(), Some(raw.as_str()));
    assert!(store.targets[&a.id].error.is_none());
    assert!(get(a.id.clone()).await.unwrap().error.is_none());
    assert!(!store.targets[&a.id]
        .snapshot
        .as_ref()
        .unwrap()
        .contains("CHANGED_PASSWORD"));
    let route = load_route(&a.id).await.unwrap();
    assert_eq!(route.egress_proxy_url.as_deref(), Some(raw.as_str()));
    fixture.catalog("source", "replacement-item", "CHANGED_PASSWORD");
    resync_source("source", false).await.unwrap();
    assert_eq!(read().unwrap().targets[&a.id].error.as_deref(), Some(STALE));
    assert_eq!(launch_url(&a.id).await.unwrap_err(), STALE);
}

#[tokio::test]
async fn removal_keeps_failed_closed_reference_and_does_not_rebuild_snapshot() {
    let fixture = Fixture::new();
    let a = fixture.account("removed");
    let raw = fixture.saved(&a.id, "source");
    let deps = dependencies("source").await.unwrap();
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0].target_id, route_id(&a.id));
    assert!(deps[0].name.starts_with("Antigravity"));
    resync_source("source", true).await.unwrap();
    let store = read().unwrap();
    assert_eq!(store.targets[&a.id].snapshot.as_deref(), Some(raw.as_str()));
    assert_eq!(store.targets[&a.id].error.as_deref(), Some(STALE));
    assert_eq!(launch_url(&a.id).await.unwrap_err(), STALE);
    assert!(get(a.id.clone()).await.unwrap().binding.is_some());
}

#[tokio::test]
async fn account_deletion_removes_secret_and_rejects_orphan_detail_files() {
    let fixture = Fixture::new();
    let a = fixture.account("delete");
    let b = fixture.account("keep");
    assert_ne!(a.id, b.id, "deletion fixtures must be distinct accounts");
    fixture.saved(&a.id, "source");
    fixture.saved(&b.id, "source");
    account::delete_account(&a.id).unwrap();
    assert!(!read().unwrap().targets.contains_key(&a.id));
    assert!(read().unwrap().targets.contains_key(&b.id));
    // Restoring a stale detail file is not enough: the active index owns membership.
    account::save_account(&a).unwrap();
    assert!(get(a.id.clone()).await.is_err());
    assert!(launch_url(&a.id).await.is_err());
    assert!(load_route(&a.id).await.is_err());
}

#[tokio::test]
async fn corrupt_store_is_not_mistaken_for_unconfigured_or_legacy() {
    let fixture = Fixture::new();
    let a = fixture.account("corrupt");
    std::fs::write(path().unwrap(), b"{broken").unwrap();
    assert_eq!(get(a.id.clone()).await.err().unwrap(), STORAGE);
    assert_eq!(launch_url(&a.id).await.unwrap_err(), STORAGE);
    assert!(clear(a.id).await.is_err());
    assert_eq!(std::fs::read(path().unwrap()).unwrap(), b"{broken");
}

#[tokio::test]
async fn stable_entry_rejects_stale_cleared_and_deleted_routes_without_direct_fallback() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let fixture = Fixture::new();
    let a = fixture.account("stable-closed");
    fixture.saved(&a.id, "missing-source");
    super::super::codex_unified_proxy::enable(
        super::super::codex_unified_proxy::Reference::default(),
        "cockpit-proxy://must-not-inherit".into(),
    )
    .unwrap();
    let route = route_id(&a.id);
    tokio::time::timeout(Duration::from_secs(10), async {
        let entry = super::super::codex_proxy_desktop_router::ensure_account_entry(&route)
            .await
            .unwrap();
        assert_eq!(
            entry,
            super::super::codex_proxy_desktop_router::ensure_account_entry(&route)
                .await
                .unwrap()
        );
        let port = reqwest::Url::parse(&entry).unwrap().port().unwrap();
        let target = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let target_port = target.local_addr().unwrap().port();
        for stage in 0..3 {
            let mut client = tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
                .await
                .unwrap();
            let mut response = [0; 12];
            let rejected = async {
                client
                    .write_all(
                        format!(
                            "CONNECT 127.0.0.1:{target_port} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n"
                        )
                        .as_bytes(),
                    )
                    .await?;
                client.read_exact(&mut response).await
            }
            .await;
            match rejected {
                Ok(_) => assert_eq!(&response, b"HTTP/1.1 502", "stage {stage} must fail closed"),
                // Ownership/staleness is checked before reading the handshake:
                // those routes may close without sending an HTTP response.
                Err(error) if stage != 1 => assert!(
                    matches!(
                        error.kind(),
                        std::io::ErrorKind::ConnectionReset
                            | std::io::ErrorKind::BrokenPipe
                            | std::io::ErrorKind::UnexpectedEof
                    ),
                    "stage {stage}: {error}"
                ),
                Err(error) => panic!("cleared route must return HTTP 502: {error}"),
            }
            assert!(
                tokio::time::timeout(Duration::from_millis(30), target.accept())
                    .await
                    .is_err()
            );
            match stage {
                0 => {
                    clear(a.id.clone()).await.unwrap();
                }
                1 => {
                    account::delete_account(&a.id).unwrap();
                }
                _ => {}
            }
        }
    })
    .await
    .expect("stable-entry failure paths must be bounded");
    super::super::codex_proxy_runtime::release_deleted_account(&route);
}

#[tokio::test]
async fn binding_cannot_race_a_no_restart_account_switch() {
    let fixture = Fixture::new();
    let a = fixture.account("switch-gate");
    let before = std::fs::read(path().unwrap()).ok();
    let permit = account::try_begin_antigravity_switch(&account::ANTIGRAVITY_SWITCH_GATE).unwrap();
    let result = bind(a.id, "source".into(), "node".into(), BTreeMap::new(), None).await;
    assert_eq!(result.err().as_deref(), Some("ANTIGRAVITY_SWITCH_BUSY"));
    assert_eq!(std::fs::read(path().unwrap()).ok(), before);
    drop(permit);
}

#[tokio::test]
async fn concurrent_clear_operations_preserve_both_platform_account_entries() {
    let fixture = Fixture::new();
    let a = fixture.account("parallel-a");
    let b = fixture.account("parallel-b");
    assert_ne!(a.id, b.id, "concurrency fixtures must be distinct accounts");
    let (a_result, b_result) = tokio::join!(clear(a.id.clone()), clear(b.id.clone()));
    a_result.unwrap();
    b_result.unwrap();
    let store = read().unwrap();
    assert!(store.targets.contains_key(&a.id));
    assert!(store.targets.contains_key(&b.id));
}
