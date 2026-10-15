use super::*;
use crate::models::codex::CodexApiProviderMode;
use crate::modules::{codex_account, codex_instance, process::CodexLaunchProxy};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn unavailable_api_service_binding_read_times_out_instead_of_waiting_indefinitely() {
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        read_bound_oauth_with_timeout(std::future::pending(), Duration::from_millis(10)),
    )
    .await
    .expect("client settings must remain recoverable when the API service lookup stalls");
    assert_eq!(result.unwrap_err(), "CLIENT_PROXY_TIMEOUT");
}

#[tokio::test]
async fn failed_api_service_binding_read_does_not_pretend_no_oauth_is_bound() {
    let result = read_bound_oauth_with_timeout(
        async { Err("private storage diagnostic".to_string()) },
        Duration::from_secs(1),
    )
    .await;
    assert_eq!(result.unwrap_err(), ERROR);
    assert_eq!(
        read_bound_oauth_with_timeout(async { Ok(None) }, Duration::from_secs(1))
            .await
            .unwrap(),
        None
    );
}

struct Fixture {
    root: PathBuf,
    previous: [Option<std::ffi::OsString>; 4],
    _lock: std::sync::MutexGuard<'static, ()>,
}
impl Fixture {
    fn new() -> Self {
        let lock = super::super::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        super::super::codex_unified_proxy::reset_cache();
        let root = std::env::temp_dir().join(format!("client-proxy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let keys = [
            "COCKPIT_TOOLS_TEST_DATA_DIR",
            "COCKPIT_TOOLS_DATA_DIR",
            "HOME",
            "CODEX_HOME",
        ];
        let previous = keys.map(std::env::var_os);
        for key in keys {
            std::env::set_var(key, &root);
        }
        Self {
            root,
            previous,
            _lock: lock,
        }
    }
    fn provider(&self) -> String {
        let id = format!("provider-{}", uuid::Uuid::new_v4());
        let account = CodexAccount::new_api_key(
            id.clone(),
            "fixture@local.invalid".into(),
            "TEST_KEY".into(),
            CodexApiProviderMode::Custom,
            Some("https://upstream.invalid".into()),
            Some("provider".into()),
            Some("Fixture".into()),
            vec![],
        );
        codex_account::save_account(&account).unwrap();
        id
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        super::super::codex_unified_proxy::reset_cache();
        for (key, value) in [
            "COCKPIT_TOOLS_TEST_DATA_DIR",
            "COCKPIT_TOOLS_DATA_DIR",
            "HOME",
            "CODEX_HOME",
        ]
        .into_iter()
        .zip(&self.previous)
        {
            if let Some(value) = value {
                std::env::set_var(key, value);
            } else {
                std::env::remove_var(key);
            }
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
async fn save_mode(id: &str, mode: Mode, url: Option<&str>) -> Result<View, String> {
    save(
        id.into(),
        mode,
        url.map(str::to_string),
        None,
        None,
        BTreeMap::new(),
        None,
    )
    .await
}

#[tokio::test]
async fn legacy_defaults_modes_encryption_and_manual_credential_retention() {
    let fixture = Fixture::new();
    let id = fixture.provider();
    assert_eq!(get(id.clone()).await.unwrap().mode, Mode::Inherit);
    assert_eq!(resolve(&id).await.unwrap(), CodexLaunchProxy::Inherit);
    assert!(!fixture.root.join(STORE).exists());
    save_mode(&id, Mode::Direct, None).await.unwrap();
    assert_eq!(resolve(&id).await.unwrap(), CodexLaunchProxy::Direct);
    assert_eq!(
        codex_instance::resolve_egress_proxy_for_bind_account(Some(
            &codex_instance::provider_gateway_bind_account_id(&id).unwrap()
        ))
        .await
        .unwrap(),
        CodexLaunchProxy::Direct
    );
    let secret = "http://user:TOPSECRET@127.0.0.1:8080/";
    let response = save_mode(&id, Mode::Independent, Some(secret))
        .await
        .unwrap();
    assert!(!serde_json::to_string(&response)
        .unwrap()
        .contains("TOPSECRET"));
    assert!(!std::fs::read_to_string(fixture.root.join(STORE))
        .unwrap()
        .contains("TOPSECRET"));
    assert_eq!(
        setting(&id).await.unwrap().snapshot.as_deref(),
        Some(secret)
    );
    save_mode(&id, Mode::Independent, None).await.unwrap();
    assert_eq!(
        setting(&id).await.unwrap().snapshot.as_deref(),
        Some(secret)
    );
    let provider = codex_account::load_account(&id).unwrap();
    assert!(!codex_account_proxy::eligible(&provider));
    assert!(codex_account_proxy::configured_url(&provider)
        .unwrap()
        .is_none());
    assert!(provider.egress_proxy_url.is_none());
    save_mode(&id, Mode::Inherit, None).await.unwrap();
    assert_eq!(resolve(&id).await.unwrap(), CodexLaunchProxy::Inherit);
    assert_eq!(setting(&id).await.unwrap().snapshot, None);
}

#[tokio::test]
async fn invalid_save_keeps_old_configuration_and_corruption_fails_closed() {
    let fixture = Fixture::new();
    let id = fixture.provider();
    save_mode(&id, Mode::Direct, None).await.unwrap();
    let before = std::fs::read(fixture.root.join(STORE)).unwrap();
    assert!(save_mode(
        &id,
        Mode::Independent,
        Some("https://user:secret@host/path")
    )
    .await
    .is_err());
    assert_eq!(
        save_mode(&id, Mode::BoundOauth, None).await.unwrap_err(),
        "CLIENT_PROXY_BOUND_OAUTH_MISSING"
    );
    assert_eq!(std::fs::read(fixture.root.join(STORE)).unwrap(), before);
    std::fs::write(fixture.root.join(STORE), "{broken").unwrap();
    assert_eq!(resolve(&id).await.unwrap_err(), ERROR);
    assert_eq!(
        std::fs::read_to_string(fixture.root.join(STORE)).unwrap(),
        "{broken"
    );
}

#[tokio::test]
async fn bound_oauth_tracks_current_exit_and_cannot_fall_back_to_direct() {
    let fixture = Fixture::new();
    let id = fixture.provider();
    let oauth_id = format!("oauth-{}", uuid::Uuid::new_v4());
    let mut oauth = synthetic(&oauth_id, Some("http://127.0.0.1:8081".into()));
    codex_account::save_account(&oauth).unwrap();
    let mut provider = codex_account::load_account(&id).unwrap();
    provider.bound_oauth_account_id = Some(oauth_id);
    codex_account::save_account(&provider).unwrap();
    save_mode(&id, Mode::BoundOauth, None).await.unwrap();
    assert_eq!(
        load_route(&id).await.unwrap().egress_proxy_url,
        oauth.egress_proxy_url
    );
    oauth.egress_proxy_url = Some("socks5h://127.0.0.1:8082".into());
    codex_account::save_account(&oauth).unwrap();
    assert_eq!(
        load_route(&id).await.unwrap().egress_proxy_url,
        oauth.egress_proxy_url
    );
    assert_eq!(
        get(id.clone()).await.unwrap().binding.unwrap()["port"],
        8082
    );
    oauth.egress_proxy_disabled = true;
    codex_account::save_account(&oauth).unwrap();
    assert_eq!(
        resolve(&id).await.unwrap_err(),
        "CLIENT_PROXY_BOUND_PROXY_MISSING"
    );
    assert_eq!(
        get(id.clone()).await.unwrap().stale_error.as_deref(),
        Some("CLIENT_PROXY_BOUND_PROXY_MISSING")
    );
    provider.bound_oauth_account_id = None;
    codex_account::save_account(&provider).unwrap();
    assert_eq!(
        load_route(&id).await.unwrap_err(),
        "CLIENT_PROXY_BOUND_OAUTH_MISSING"
    );
}

#[tokio::test]
async fn api_service_modes_and_actual_entry_route_to_selected_http_proxy() {
    tokio::time::timeout(Duration::from_secs(5), actual_entry_route())
        .await
        .expect("bounded local proxy transport");
}
async fn actual_entry_route() {
    let _fixture = Fixture::new();
    let id = codex_instance::CODEX_API_SERVICE_BIND_ACCOUNT_ID;
    assert_eq!(resolve(id).await.unwrap(), CodexLaunchProxy::Inherit);
    save_mode(id, Mode::Direct, None).await.unwrap();
    assert_eq!(
        codex_instance::resolve_egress_proxy_for_bind_account(Some(id))
            .await
            .unwrap(),
        CodexLaunchProxy::Direct
    );
    let mock = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = mock.local_addr().unwrap();
    save_mode(id, Mode::Independent, Some(&format!("http://{address}")))
        .await
        .unwrap();
    let CodexLaunchProxy::Proxy(entry) =
        codex_instance::resolve_egress_proxy_for_bind_account(Some(id))
            .await
            .unwrap()
    else {
        panic!("missing managed client proxy");
    };
    let parsed = url::Url::parse(&entry).unwrap();
    let mut client = tokio::net::TcpStream::connect(("127.0.0.1", parsed.port().unwrap()))
        .await
        .unwrap();
    let upstream = tokio::spawn(async move {
        let (mut socket, _) = mock.accept().await.unwrap();
        let mut request = Vec::new();
        loop {
            let mut byte = [0];
            socket.read_exact(&mut byte).await.unwrap();
            request.push(byte[0]);
            if request.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        assert!(request.starts_with(b"CONNECT fixture.invalid:443 HTTP/1.1\r\n"));
        socket
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await
            .unwrap();
        let mut ping = [0; 4];
        socket.read_exact(&mut ping).await.unwrap();
        assert_eq!(&ping, b"ping");
        socket.write_all(b"pong").await.unwrap();
    });
    client
        .write_all(b"CONNECT fixture.invalid:443 HTTP/1.1\r\nHost: fixture.invalid:443\r\n\r\n")
        .await
        .unwrap();
    let mut response = Vec::new();
    loop {
        let mut byte = [0];
        client.read_exact(&mut byte).await.unwrap();
        response.push(byte[0]);
        if response.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    assert!(response.starts_with(b"HTTP/1.1 200"));
    client.write_all(b"ping").await.unwrap();
    let mut pong = [0; 4];
    client.read_exact(&mut pong).await.unwrap();
    assert_eq!(&pong, b"pong");
    upstream.await.unwrap();
    // Stable entry survives a mode change; only the next launch's flags change.
    save_mode(id, Mode::Direct, None).await.unwrap();
    assert!(super::super::codex_proxy_desktop_router::entry_status(&route_id(id)).is_some());
}

#[tokio::test]
async fn explicitly_deleted_catalog_source_unbinds_with_ordinary_account_semantics() {
    let fixture = Fixture::new();
    let id = fixture.provider();
    let mut store = read().unwrap();
    store.targets.insert(
        id.clone(),
        Setting {
            mode: Mode::Independent,
            snapshot: Some("http://user:TOPSECRET@127.0.0.1:8080".into()),
            reference: Some(super::super::codex_unified_proxy::Reference {
                source_id: "deleted".into(),
                source_name: "Demo".into(),
                item_id: "node".into(),
                name: "Node".into(),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    write(&store).unwrap();
    assert_eq!(dependencies("deleted").await.unwrap().len(), 1);
    // An automatic refresh failure must never unbind or switch to direct.
    resync_source("deleted", false).await.unwrap();
    assert_eq!(setting(&id).await.unwrap().mode, Mode::Independent);
    assert_eq!(resolve(&id).await.unwrap_err(), "CLIENT_PROXY_STALE");
    // The subsequent explicit user-confirmed deletion clears only that binding.
    resync_source("deleted", true).await.unwrap();
    assert!(get(id.clone()).await.unwrap().stale_error.is_none());
    assert_eq!(resolve(&id).await.unwrap(), CodexLaunchProxy::Inherit);
    assert_eq!(setting(&id).await.unwrap().mode, Mode::Inherit);
    assert!(setting(&id).await.unwrap().snapshot.is_none());
    assert!(setting(&id).await.unwrap().reference.is_none());
    assert!(dependencies("deleted").await.unwrap().is_empty());
}

#[test]
fn target_normalization_rejects_empty_and_nested_route_identifiers() {
    assert_eq!(
        normalize_target(" __provider_gateway__:abc ").unwrap(),
        "abc"
    );
    assert!(normalize_target("").is_err());
    assert!(normalize_target(&route_id("abc")).is_err());
    assert!(normalize_target("abc\n").is_ok()); // surrounding whitespace is normalized
    assert!(normalize_target("a\nb").is_err());
    for target in [
        "../../../other",
        "/absolute/path",
        r"C:\outside",
        r"a\b",
        ".",
        "..",
    ] {
        assert!(normalize_target(target).is_err(), "unexpected path target");
    }
}

#[tokio::test]
async fn removed_source_cannot_use_old_snapshot_when_stale_write_did_not_complete() {
    let fixture = Fixture::new();
    let id = fixture.provider();
    let mut store = read().unwrap();
    store.targets.insert(
        id.clone(),
        Setting {
            mode: Mode::Independent,
            snapshot: Some("http://127.0.0.1:8080".into()),
            reference: Some(super::super::codex_unified_proxy::Reference {
                source_id: "vanished".into(),
                source_name: "Old".into(),
                item_id: "old".into(),
                name: "Old".into(),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    write(&store).unwrap();
    let before = std::fs::read(fixture.root.join(STORE)).unwrap();
    assert_eq!(setting(&id).await.unwrap().stale_error, None);
    assert_eq!(load_route(&id).await.unwrap_err(), "CLIENT_PROXY_STALE");
    assert_eq!(
        get(id).await.unwrap().stale_error.as_deref(),
        Some("CLIENT_PROXY_STALE")
    );
    assert_eq!(std::fs::read(fixture.root.join(STORE)).unwrap(), before);
}

#[test]
fn refreshed_snapshot_is_applied_only_to_the_observed_binding_revision() {
    let mut store = Store {
        version: 1,
        ..Default::default()
    };
    store.targets.insert(
        "card".into(),
        Setting {
            mode: Mode::Independent,
            snapshot: Some("old".into()),
            revision: 7,
            ..Default::default()
        },
    );
    assert!(apply_resync(&mut store, "card", 7, Ok("refreshed".into())));
    assert_eq!(store.targets["card"].snapshot.as_deref(), Some("refreshed"));
    assert_eq!(store.targets["card"].revision, 8);
    assert!(!apply_resync(
        &mut store,
        "card",
        7,
        Err("CATALOG_NOT_FOUND".into())
    ));
    assert_eq!(store.targets["card"].snapshot.as_deref(), Some("refreshed"));
    assert_eq!(store.targets["card"].stale_error, None);
    assert!(apply_resync(
        &mut store,
        "card",
        8,
        Err("CATALOG_NOT_FOUND".into())
    ));
    assert_eq!(store.targets["card"].mode, Mode::Independent);
}

#[tokio::test]
async fn catalog_refresh_rebuilds_encrypted_snapshot_and_preserves_selection_reference() {
    let fixture = Fixture::new();
    let target = fixture.provider();
    let catalog = |port| super::super::codex_proxy_subscription_parser::ParsedCatalog {
        nodes: vec![super::super::codex_proxy_subscription_parser::ParsedNode {
            id: "stable-node".into(),
            name: "Node".into(),
            protocol: "HTTP".into(),
            error: None,
            outbound: Some(
                serde_json::json!({"type":"http","server":"127.0.0.1","server_port":port,"username":"user","password":"TOPSECRET"}),
            ),
        }],
        groups: vec![],
    };
    let old = binding::encode(
        "source",
        "Demo",
        "stable-node",
        &catalog(8081),
        &BTreeMap::new(),
    )
    .unwrap();
    let mut store = read().unwrap();
    store.targets.insert(
        target.clone(),
        Setting {
            mode: Mode::Independent,
            snapshot: Some(old),
            reference: Some(super::super::codex_unified_proxy::Reference {
                source_id: "source".into(),
                source_name: "Demo".into(),
                item_id: "stable-node".into(),
                name: "Node".into(),
                ..Default::default()
            }),
            revision: 1,
            ..Default::default()
        },
    );
    write(&store).unwrap();
    let changed = serde_json::json!({"version":1,"sources":[{
        "id":"source","name":"Renamed","kind":"manual","url":null,"catalog":catalog(8082),
        "updated_at":2,"last_attempt_at":null,"auto_update":false,"error":null,"revision":"2"
    }]});
    std::fs::write(
        fixture.root.join("codex-proxy-sources.json"),
        serde_json::to_string(&changed).unwrap(),
    )
    .unwrap();
    resync_source("source", false).await.unwrap();
    let current = setting(&target).await.unwrap();
    let snapshot = current.snapshot.as_deref().unwrap();
    assert_eq!(
        binding::outbounds(snapshot).unwrap()[0]["server_port"],
        8082
    );
    assert_eq!(binding::decode(snapshot).unwrap().source_name, "Renamed");
    assert_eq!(current.reference.unwrap().item_id, "stable-node");
    assert!(get(target.clone()).await.unwrap().stale_error.is_none());
    assert!(!std::fs::read_to_string(fixture.root.join(STORE))
        .unwrap()
        .contains("TOPSECRET"));
    // Missing selected member fails closed even before a resync mark can be written.
    std::fs::write(
        fixture.root.join("codex-proxy-sources.json"),
        r#"{"version":1,"sources":[]}"#,
    )
    .unwrap();
    assert_eq!(load_route(&target).await.unwrap_err(), "CLIENT_PROXY_STALE");
}

#[tokio::test]
async fn ordinary_proxy_commands_support_api_cards_without_touching_credentials_or_upstream() {
    let fixture = Fixture::new();
    let id = fixture.provider();
    let before = codex_account::load_account(&id).unwrap();
    let proxy = "http://user:PRIVATE_PROXY_SECRET@127.0.0.1:18081";
    let saved = crate::commands::codex::update_codex_account_egress_proxy(
        id.clone(),
        Some(proxy.into()),
        None,
    )
    .await
    .unwrap();
    assert_eq!(saved.id, id);
    assert!(saved.is_api_key_auth());
    assert!(saved.openai_api_key.is_none());
    let json = serde_json::to_string(&saved).unwrap();
    assert!(!json.contains("PRIVATE_PROXY_SECRET"));
    assert!(!json.contains("TEST_KEY"));
    assert!(saved.tokens.access_token.is_empty());
    let runtime_id = super::super::codex_proxy_targets::runtime_id(&id)
        .await
        .unwrap();
    assert_eq!(runtime_id, route_id(&id));
    let status = crate::commands::codex::get_codex_account_proxy_status(id.clone())
        .await
        .unwrap();
    assert_eq!(status.desktop, "direct");
    assert_eq!(status.account, "unbound");
    assert_eq!(status.sidecar, "unbound");
    assert!(status.account_port.is_none() && status.sidecar_port.is_none());
    crate::commands::codex_proxy_engine::codex_proxy_activity_set_enabled(id.clone(), true)
        .await
        .unwrap();
    let capture = crate::commands::codex_proxy_engine::codex_proxy_activity_snapshot(id.clone())
        .await
        .unwrap();
    assert!(capture.enabled);
    crate::commands::codex_proxy_engine::codex_proxy_activity_clear(id.clone())
        .await
        .unwrap();
    crate::commands::codex_proxy_engine::codex_proxy_activity_set_enabled(id.clone(), false)
        .await
        .unwrap();
    // A later credential/name writer must never overwrite the separate proxy store.
    let mut changed = before.clone();
    changed.openai_api_key = Some("UPDATED_PRIVATE_API_KEY".into());
    changed.account_name = Some("Updated name".into());
    codex_account::save_account(&changed).unwrap();
    let projection = crate::commands::codex_client_proxy::codex_proxy_target_get(id.clone())
        .await
        .unwrap();
    assert_eq!(
        projection.account.account_name.as_deref(),
        Some("Updated name")
    );
    assert_eq!(
        projection.account.egress_proxy_url.as_deref(),
        Some("http://user:PRIVATE_PROXY_SECRET@127.0.0.1:18081/")
    );
    assert!(!serde_json::to_string(&projection)
        .unwrap()
        .contains("UPDATED_PRIVATE_API_KEY"));
    let stored = codex_account::load_account(&id).unwrap();
    assert_eq!(
        stored.openai_api_key.as_deref(),
        Some("UPDATED_PRIVATE_API_KEY")
    );
    assert_eq!(stored.egress_proxy_url, before.egress_proxy_url);
    assert!(!codex_account_proxy::eligible(&stored));
    assert!(codex_account_proxy::configured_url(&stored)
        .unwrap()
        .is_none());
    super::super::codex_proxy_runtime::release_deleted_account(&runtime_id);
}

#[tokio::test]
async fn api_service_uses_same_proxy_commands_and_never_creates_an_account_file() {
    let _fixture = Fixture::new();
    let id = codex_instance::CODEX_API_SERVICE_BIND_ACCOUNT_ID;
    let saved = crate::commands::codex::update_codex_account_egress_proxy(
        id.into(),
        Some("socks5h://127.0.0.1:18082".into()),
        None,
    )
    .await
    .unwrap();
    assert_eq!(saved.id, id);
    assert!(saved.is_api_key_auth());
    assert!(saved.email.is_empty());
    assert!(codex_account::load_account(id).is_none());
    assert_eq!(
        crate::commands::codex::get_codex_account_proxy_status(id.into())
            .await
            .unwrap()
            .desktop,
        "direct"
    );
    crate::commands::codex::update_codex_account_egress_proxy(id.into(), None, Some(true))
        .await
        .unwrap();
    assert_eq!(resolve(id).await.unwrap(), CodexLaunchProxy::Direct);
    assert_eq!(
        crate::commands::codex::get_codex_account_proxy_status(id.into())
            .await
            .unwrap()
            .proxy_source,
        "disabled"
    );
    crate::commands::codex::update_codex_account_egress_proxy(id.into(), None, Some(false))
        .await
        .unwrap();
    assert_eq!(resolve(id).await.unwrap(), CodexLaunchProxy::Inherit);
    assert!(codex_account::load_account(id).is_none());
}

#[tokio::test]
async fn follow_unified_changes_effective_route_but_never_changes_api_upstream() {
    let fixture = Fixture::new();
    let id = fixture.provider();
    let api_service = codex_instance::CODEX_API_SERVICE_BIND_ACCOUNT_ID;
    assert_eq!(resolve(&id).await.unwrap(), CodexLaunchProxy::Inherit);
    let catalog = super::super::codex_proxy_subscription_parser::ParsedCatalog {
        nodes: vec![super::super::codex_proxy_subscription_parser::ParsedNode {
            id: "unified-node".into(),
            name: "Node".into(),
            protocol: "HTTP".into(),
            error: None,
            outbound: Some(
                serde_json::json!({"type":"http","server":"127.0.0.1","server_port":18083}),
            ),
        }],
        groups: vec![],
    };
    let snapshot = binding::encode(
        "unified-source",
        "Unified",
        "unified-node",
        &catalog,
        &BTreeMap::new(),
    )
    .unwrap();
    super::super::codex_unified_proxy::enable(
        super::super::codex_unified_proxy::Reference {
            source_id: "unified-source".into(),
            item_id: "unified-node".into(),
            name: "Node".into(),
            ..Default::default()
        },
        snapshot.clone(),
    )
    .unwrap();
    for target in [&id[..], api_service] {
        let route = load_route(target).await.unwrap();
        assert!(!route.egress_proxy_disabled);
        assert_eq!(
            codex_account_proxy::configured_url(&route)
                .unwrap()
                .as_deref(),
            Some(snapshot.as_str())
        );
        // No engine is installed in the fixture: follow must fail explicitly,
        // rather than silently launching inherited/direct and bypassing unified.
        assert!(resolve(target).await.is_err());
        save_account_binding(target.into(), None, true)
            .await
            .unwrap();
        assert_eq!(resolve(target).await.unwrap(), CodexLaunchProxy::Direct);
        assert!(
            codex_account_proxy::configured_url(&load_route(target).await.unwrap())
                .unwrap()
                .is_none()
        );
        save_account_binding(target.into(), None, false)
            .await
            .unwrap();
    }
    let (eligible, independent, disabled) = super::super::codex_proxy_targets::account_totals()
        .await
        .unwrap();
    assert!(eligible.contains(&id) && eligible.contains(&api_service.to_string()));
    assert!(!independent.contains(&id) && !disabled.contains(&id));
    let api = codex_account::load_account(&id).unwrap();
    assert!(codex_account_proxy::configured_url(&api).unwrap().is_none());
    super::super::codex_unified_proxy::disable().unwrap();
    for target in [&id[..], api_service] {
        assert_eq!(resolve(target).await.unwrap(), CodexLaunchProxy::Inherit);
    }
}

#[tokio::test]
async fn legacy_bound_projection_is_explicit_and_can_be_replaced_using_three_mode_commands() {
    let fixture = Fixture::new();
    let id = fixture.provider();
    let oauth_id = format!("legacy-oauth-{}", uuid::Uuid::new_v4());
    let mut oauth = synthetic(&oauth_id, Some("http://127.0.0.1:18084".into()));
    codex_account::save_account(&oauth).unwrap();
    let mut api = codex_account::load_account(&id).unwrap();
    api.bound_oauth_account_id = Some(oauth_id);
    codex_account::save_account(&api).unwrap();
    save_mode(&id, Mode::BoundOauth, None).await.unwrap();
    let view = target_view(&id).await.unwrap();
    assert!(view.egress_proxy_legacy_bound);
    assert_eq!(view.account.egress_proxy_url, oauth.egress_proxy_url);
    oauth.egress_proxy_disabled = true;
    codex_account::save_account(&oauth).unwrap();
    let invalid = target_view(&id).await.unwrap();
    assert!(invalid.egress_proxy_legacy_bound);
    assert_eq!(
        invalid.egress_proxy_error.as_deref(),
        Some("CLIENT_PROXY_BOUND_PROXY_MISSING")
    );
    assert_eq!(setting(&id).await.unwrap().mode, Mode::BoundOauth);
    // Preview is read-only; only this explicit save clears the old setting.
    crate::commands::codex::update_codex_account_egress_proxy(id.clone(), None, Some(false))
        .await
        .unwrap();
    assert!(!target_view(&id).await.unwrap().egress_proxy_legacy_bound);
    assert_eq!(setting(&id).await.unwrap().mode, Mode::Inherit);
}

#[tokio::test]
async fn normal_proxy_getter_returns_no_oauth_credentials_and_keeps_existing_store_unchanged() {
    let _fixture = Fixture::new();
    let id = format!("oauth-safe-{}", uuid::Uuid::new_v4());
    let mut oauth = synthetic(
        &id,
        Some("http://user:PRIVATE_PASSWORD@127.0.0.1:18085".into()),
    );
    oauth.tokens.access_token = "PRIVATE_ACCESS_TOKEN".into();
    oauth.tokens.id_token = "PRIVATE_ID_TOKEN".into();
    oauth.tokens.refresh_token = Some("PRIVATE_REFRESH_TOKEN".into());
    oauth.account_password = Some("PRIVATE_ACCOUNT_PASSWORD".into());
    codex_account::save_account(&oauth).unwrap();
    let view = crate::commands::codex_client_proxy::codex_proxy_target_get(id.clone())
        .await
        .unwrap();
    let serialized = serde_json::to_string(&view).unwrap();
    for private in [
        "PRIVATE_PASSWORD",
        "PRIVATE_ACCESS_TOKEN",
        "PRIVATE_ID_TOKEN",
        "PRIVATE_REFRESH_TOKEN",
        "PRIVATE_ACCOUNT_PASSWORD",
    ] {
        assert!(!serialized.contains(private));
    }
    assert_eq!(
        super::super::codex_proxy_targets::runtime_id(&id)
            .await
            .unwrap(),
        id
    );
    let stored = codex_account::load_account(&id).unwrap();
    assert_eq!(stored.tokens.access_token, oauth.tokens.access_token);
    assert_eq!(stored.egress_proxy_url, oauth.egress_proxy_url);
}

#[test]
fn confirmed_removal_cas_preserves_newer_rebindings_and_other_targets() {
    let mut store = Store {
        version: 1,
        ..Default::default()
    };
    store.targets.insert(
        "card".into(),
        Setting {
            mode: Mode::Independent,
            snapshot: Some("current".into()),
            revision: 9,
            ..Default::default()
        },
    );
    store.targets.insert(
        "other".into(),
        Setting {
            mode: Mode::Independent,
            snapshot: Some("other".into()),
            revision: 2,
            ..Default::default()
        },
    );
    assert!(!apply_removal(&mut store, "card", 8));
    assert_eq!(store.targets["card"].snapshot.as_deref(), Some("current"));
    assert!(apply_removal(&mut store, "card", 9));
    assert_eq!(store.targets["card"].revision, 10);
    assert_eq!(store.targets["card"].mode, Mode::Inherit);
    assert!(store.targets["card"].snapshot.is_none());
    assert_eq!(store.targets["other"].snapshot.as_deref(), Some("other"));
}

#[tokio::test]
async fn ordinary_catalog_binding_preserves_nested_picker_ids_and_failed_preparation_keeps_old_route(
) {
    let fixture = Fixture::new();
    let id = fixture.provider();
    let catalog: super::super::codex_proxy_subscription_parser::ParsedCatalog = serde_json::from_value(serde_json::json!({
        "nodes": [{"id":"node","name":"Selected Node","protocol":"HTTP","error":null,"outbound":{"type":"http","server":"127.0.0.1","server_port":18086}}],
        "groups": [
            {"id":"outer-id","name":"Outer","kind":"select","members":["Inner"],"url":null,"interval":null,"tolerance":null,"error":null},
            {"id":"inner-id","name":"Inner","kind":"select","members":["Selected Node"],"url":null,"interval":null,"tolerance":null,"error":null}
        ]
    })).unwrap();
    let selections = BTreeMap::from([
        ("outer-id".into(), "Inner".into()),
        ("inner-id".into(), "Selected Node".into()),
    ]);
    let snapshot = binding::encode("source", "Demo", "outer-id", &catalog, &selections).unwrap();
    std::fs::write(fixture.root.join("codex-proxy-sources.json"), serde_json::to_string(&serde_json::json!({"version":1,"sources":[{
        "id":"source","name":"Demo","kind":"manual","url":null,"catalog":catalog,"updated_at":1,"last_attempt_at":null,"auto_update":false,"error":null,"revision":"1"
    }]})).unwrap()).unwrap();
    let reference = reference_for_snapshot(&snapshot).await.unwrap().unwrap();
    assert_eq!(reference.selections, selections);
    assert_eq!(reference.item_id, "outer-id");
    save_account_binding(id.clone(), Some("http://127.0.0.1:18087".into()), false)
        .await
        .unwrap();
    let previous = std::fs::read(fixture.root.join(STORE)).unwrap();
    let error = crate::commands::codex_proxy_catalog::codex_proxy_catalog_bind(
        id.clone(),
        "source".into(),
        "outer-id".into(),
        selections,
        None,
    )
    .await
    .unwrap_err();
    assert_ne!(
        error, "PROXY_ACCOUNT_UNSUPPORTED",
        "API cards must reach the shared resource preparation path"
    );
    assert_eq!(std::fs::read(fixture.root.join(STORE)).unwrap(), previous);
    assert_eq!(
        setting(&id).await.unwrap().snapshot.as_deref(),
        Some("http://127.0.0.1:18087/")
    );
    assert!(codex_account::load_account(&id)
        .unwrap()
        .egress_proxy_url
        .is_none());
}
