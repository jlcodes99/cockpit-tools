use super::*;
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("catalog-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn fixture() -> Source {
    Source {
        id: "source-id".into(),
        name: "Demo".into(),
        kind: "subscription".into(),
        url: Some("https://subscription.example/list?token=private-subscription-secret".into()),
        catalog: parser::parse("trojan://private-node-secret@node.example:443#Demo").unwrap(),
        updated_at: 1,
        last_attempt_at: None,
        auto_update: false,
        error: None,
        revision: "v1".into(),
        network: Default::default(),
        default: None,
        default_invalidated: false,
        strategy_members: None,
        usage: None,
        parser_version: PARSER_VERSION,
    }
}

#[test]
fn node_view_exposes_only_endpoint_fields_for_search() {
    for input in [
        "trojan://private-node-secret@node.example:443#Demo",
        "proxies:\n  - {name: Demo, type: trojan, server: node.example, port: 443, password: private-node-secret}",
        "http://private-user:private-password@node.example:443",
    ] {
        let mut source = fixture();
        source.catalog = parser::parse(input).unwrap();
        let public = serde_json::to_value(view(&Store { sources: vec![source], ..Store::default() })).unwrap();
        let node = &public["sources"][0]["nodes"][0];
        assert_eq!(node["server"], "node.example");
        assert_eq!(node["port"], 443);
        let serialized = public.to_string();
        for secret in ["private-node-secret", "private-password", "private-user", "private-subscription-secret"] {
            assert!(!serialized.contains(secret));
        }
        assert!(node.get("outbound").is_none());
    }
}
fn restore_env(key: &str, previous: Option<std::ffi::OsString>) {
    match previous {
        Some(value) => std::env::set_var(key, value),
        None => std::env::remove_var(key),
    }
}
/// 指向临时目录的数据目录覆盖：两套数据目录解析都改到临时目录，测试结束后恢复原值。
struct DataDir {
    test: Option<std::ffi::OsString>,
    data: Option<std::ffi::OsString>,
}
impl DataDir {
    fn set(path: &Path) -> Self {
        let test = std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR");
        let data = std::env::var_os("COCKPIT_TOOLS_DATA_DIR");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", path);
        std::env::set_var("COCKPIT_TOOLS_DATA_DIR", path);
        Self { test, data }
    }
}
impl Drop for DataDir {
    fn drop(&mut self) {
        restore_env("COCKPIT_TOOLS_TEST_DATA_DIR", self.test.take());
        restore_env("COCKPIT_TOOLS_DATA_DIR", self.data.take());
    }
}
/// 策略测试用的订阅来源：Alpha / Beta / Gamma 可用，TLS 因跳过证书校验未获授权而不可用。
fn subscription_fixture() -> Source {
    Source {
        id: "subscription-id".into(),
        name: "Demo".into(),
        kind: "subscription".into(),
        url: Some("https://subscription.example/list?token=private-subscription-secret".into()),
        catalog: parser::parse(
            "proxies:\n  - {name: Alpha, type: trojan, server: alpha.example, port: 443, password: private-alpha}\n  - {name: Beta, type: trojan, server: beta.example, port: 443, password: private-beta}\n  - {name: Gamma, type: trojan, server: gamma.example, port: 443, password: private-gamma}\n  - {name: TLS, type: hysteria2, server: tls.example, port: 443, password: private-tls, skip-cert-verify: true}\n",
        )
        .unwrap(),
        updated_at: 1,
        last_attempt_at: None,
        auto_update: true,
        error: None,
        revision: "v1".into(),
        network: Default::default(),
        default: None,
        default_invalidated: false,
        strategy_members: None,
        usage: None,
        parser_version: PARSER_VERSION,
    }
}
fn item(source: &Source, name: &str) -> StrategyMember {
    StrategyMember {
        source_id: source.id.clone(),
        item_id: source
            .catalog
            .nodes
            .iter()
            .find(|node| node.name == name)
            .unwrap()
            .id
            .clone(),
    }
}
fn stored() -> Store {
    read(&path().unwrap()).unwrap()
}
/// 手选组成员用组 ID 记录：与前端草稿的 selections 结构一致。
fn choices(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(group, member)| ((*group).to_owned(), (*member).to_owned()))
        .collect()
}
fn selectable_catalog() -> ParsedCatalog {
    parser::parse(
        "proxies:\n  - {name: Alpha, type: trojan, server: alpha.example, port: 443, password: private-alpha}\n  - {name: Beta, type: trojan, server: beta.example, port: 443, password: private-beta}\n  - {name: TLS, type: hysteria2, server: tls.example, port: 443, password: private-tls, skip-cert-verify: true}\nproxy-groups:\n  - {name: Manual, type: select, proxies: [TLS, Beta, Alpha]}\n  - {name: Auto, type: url-test, proxies: [Alpha, Beta]}\n  - {name: Broken, type: url-test, proxies: [TLS]}\n  - {name: Nested, type: select, proxies: [Alpha]}\n  - {name: Outer, type: url-test, proxies: [Nested]}",
    )
    .unwrap()
}
/// 旧文件没有默认项字段时按“无默认项”处理，且不破坏已有绑定解析。
#[test]
fn stored_sources_without_default_fields_stay_readable() {
    let source = fixture();
    let node = source.catalog.nodes[0].id.clone();
    let mut legacy = serde_json::to_value(&source).unwrap();
    let fields = legacy.as_object_mut().unwrap();
    fields.remove("default");
    fields.remove("default_invalidated");
    let loaded: Source = serde_json::from_value(legacy).unwrap();
    assert!(loaded.default.is_none());
    assert!(!loaded.default_invalidated);
    let store = Store {
        version: 1,
        sources: vec![loaded],
    };
    let public = serde_json::to_string(&view(&store)).unwrap();
    assert!(public.contains("\"default\":null"), "{public}");
    assert!(public.contains("\"defaultInvalidated\":false"), "{public}");
    let stored = &store.sources[0];
    let snapshot = binding::encode(
        &stored.id,
        &stored.name,
        &node,
        &stored.catalog,
        &BTreeMap::new(),
    )
    .unwrap();
    assert!(binding::decode(&snapshot).is_ok());
}
/// 默认项必须与选择器一致：资源存在且受支持，手选组必须给出合法成员。
#[test]
fn default_items_require_a_reachable_supported_choice() {
    let catalog = selectable_catalog();
    let id = |name: &str| {
        catalog
            .nodes
            .iter()
            .find(|node| node.name == name)
            .map(|node| node.id.clone())
            .or_else(|| {
                catalog
                    .groups
                    .iter()
                    .find(|group| group.name == name)
                    .map(|group| group.id.clone())
            })
            .unwrap()
    };
    let alpha = id("Alpha");
    let manual = id("Manual");
    let nested = id("Nested");
    assert!(default_error("subscription", &catalog, &alpha, &BTreeMap::new()).is_none());
    assert_eq!(
        default_error("subscription", &catalog, &id("TLS"), &BTreeMap::new()),
        Some("CATALOG_INVALID"),
        "不受支持的节点不能设为默认项"
    );
    assert_eq!(
        default_error("subscription", &catalog, "missing", &BTreeMap::new()),
        Some("CATALOG_CHANGED"),
        "陈旧的 itemId 必须提示重新选择，而不是写入失效引用"
    );
    assert_eq!(
        default_error("subscription", &catalog, &manual, &BTreeMap::new()),
        Some("CATALOG_INVALID"),
        "手选组必须先明确成员"
    );
    assert!(default_error(
        "subscription",
        &catalog,
        &manual,
        &choices(&[(&manual, "Beta")])
    )
    .is_none());
    assert_eq!(
        default_error(
            "subscription",
            &catalog,
            &manual,
            &choices(&[(&manual, "TLS")])
        ),
        Some("CATALOG_INVALID")
    );
    assert_eq!(
        default_error(
            "subscription",
            &catalog,
            &manual,
            &choices(&[(&manual, "Gamma")])
        ),
        Some("CATALOG_INVALID")
    );
    // 自动分组保留策略，只需成员可解析；无关的手选组不得阻止验证。
    assert!(default_error("subscription", &catalog, &id("Auto"), &BTreeMap::new()).is_none());
    assert_eq!(
        default_error("subscription", &catalog, &id("Broken"), &BTreeMap::new()),
        Some("CATALOG_INVALID")
    );
    assert_eq!(
        default_error("subscription", &catalog, &id("Outer"), &BTreeMap::new()),
        Some("CATALOG_INVALID")
    );
    assert!(default_error(
        "subscription",
        &catalog,
        &id("Outer"),
        &choices(&[(&nested, "Alpha")])
    )
    .is_none());
    assert!(group_context_error(&catalog, &alpha, None).is_none());
    assert!(group_context_error(&catalog, &alpha, Some("")).is_none());
    assert!(group_context_error(&catalog, &manual, Some(&manual)).is_none());
    assert!(group_context_error(&catalog, &alpha, Some(&manual)).is_none());
    assert_eq!(
        group_context_error(&catalog, &alpha, Some(&id("Broken"))),
        Some("CATALOG_INVALID")
    );
    assert_eq!(
        group_context_error(&catalog, &alpha, Some("missing")),
        Some("CATALOG_INVALID")
    );
}
/// 自建策略的四种分组类型都能作为来源默认项；订阅来源维持原有 select/url-test 范围。
#[test]
fn strategy_defaults_accept_every_strategy_group_kind() {
    let source = subscription_fixture();
    for kind in ["select", "fallback", "url-test", "load-balance"] {
        let copies = source
            .catalog
            .nodes
            .iter()
            .filter(|node| node.error.is_none())
            .map(|node| strategy::StrategyNode {
                name: node.name.clone(),
                protocol: node.protocol.clone(),
                outbound: node.outbound.clone().unwrap(),
            })
            .collect();
        let catalog = strategy::build(
            "Strategy",
            kind,
            &StrategyOptions::default(),
            strategy::group_id("source-id"),
            copies,
        )
        .unwrap();
        let group = catalog.groups[0].id.clone();
        let selections = choices(&[(&group, "Alpha")]);
        assert!(
            default_error(strategy::KIND, &catalog, &group, &selections).is_none(),
            "{kind}"
        );
        assert!(
            default_error(
                strategy::KIND,
                &catalog,
                &catalog.nodes[0].id,
                &BTreeMap::new()
            )
            .is_none(),
            "{kind}"
        );
        assert!(strategy::group_supported(strategy::KIND, kind), "{kind}");
        if !matches!(kind, "select" | "url-test") {
            assert_eq!(
                default_error("subscription", &catalog, &group, &selections),
                Some("CATALOG_INVALID"),
                "订阅来源不得因为策略来源放宽而接受 {kind}"
            );
        }
    }
}
/// 刷新或权限变化后默认项失效只清除并标记，绝不静默换成别的节点。
#[test]
fn invalidated_defaults_are_cleared_instead_of_substituted() {
    let mut source = fixture();
    source.default = Some(SourceDefault {
        item_id: source.catalog.nodes[0].id.clone(),
        group_id: None,
        selections: BTreeMap::new(),
    });
    assert!(!drop_invalid_default(&mut source));
    assert!(source.default.is_some());
    source.catalog = parser::parse("trojan://another@other.example:443#Replaced").unwrap();
    source.default_invalidated = false;
    assert!(drop_invalid_default(&mut source));
    assert!(source.default.is_none(), "不得把默认项指向替代节点");
    assert!(source.default_invalidated);
    assert!(!drop_invalid_default(&mut source));
}
#[test]
fn summaries_and_binding_snapshots_do_not_expose_source_credentials() {
    let source = fixture();
    let id = source.catalog.nodes[0].id.clone();
    let snapshot = binding::encode(
        &source.id,
        &source.name,
        &id,
        &source.catalog,
        &BTreeMap::new(),
    )
    .unwrap();
    let summary = serde_json::to_string(&binding::summary(&snapshot)).unwrap();
    let store = Store {
        version: 1,
        sources: vec![source],
    };
    let mut public = serde_json::to_value(view(&store)).unwrap();
    assert_eq!(public["sources"][0]["nodes"][0]["server"], "node.example");
    // Endpoint metadata is public for node search. It must not escape anywhere
    // else, and neither that metadata nor a binding summary exposes credentials.
    public["sources"][0]["nodes"][0]
        .as_object_mut()
        .unwrap()
        .remove("server");
    let public = public.to_string();
    for text in [summary, public] {
        for secret in [
            "private-node-secret",
            "private-subscription-secret",
            "subscription.example",
            "node.example",
            "outbound",
        ] {
            assert!(!text.contains(secret));
        }
    }
    let old = binding::outbounds(&snapshot).unwrap();
    let mut source = fixture();
    source.catalog = parser::parse("trojan://replacement@other.example:443#Demo").unwrap();
    let updated = binding::encode(
        &source.id,
        &source.name,
        &id,
        &source.catalog,
        &BTreeMap::new(),
    )
    .unwrap();
    assert_ne!(old, binding::outbounds(&updated).unwrap());
    drop(source);
    assert_eq!(
        old,
        binding::outbounds(&snapshot).unwrap(),
        "refresh cannot reroute the stored binding"
    );
}
#[test]
fn removal_dependencies_report_bound_accounts_without_credentials() {
    use crate::models::codex::{CodexAccount, CodexTokens};
    let source = fixture();
    let node = source.catalog.nodes[0].id.clone();
    let bound = binding::encode(
        &source.id,
        &source.name,
        &node,
        &source.catalog,
        &BTreeMap::new(),
    )
    .unwrap();
    let mut neighbour = fixture();
    neighbour.id = "neighbour-id".into();
    let neighbour_bound = binding::encode(
        &neighbour.id,
        &neighbour.name,
        &node,
        &neighbour.catalog,
        &BTreeMap::new(),
    )
    .unwrap();
    let account = |id: &str, email: &str, name: Option<&str>| {
        let mut account = CodexAccount::new(
            id.into(),
            email.into(),
            CodexTokens {
                access_token: "fake-access".into(),
                id_token: "fake-id".into(),
                refresh_token: None,
            },
        );
        account.account_name = name.map(str::to_owned);
        account
    };
    let mut named = account("named", "named@example.com", Some(" Work account "));
    named.egress_proxy_url = Some(bound.clone());
    let mut unnamed = account("unnamed", "unnamed@example.com", Some("   "));
    unnamed.egress_proxy_url = Some(bound);
    let mut elsewhere = account("elsewhere", "elsewhere@example.com", None);
    elsewhere.egress_proxy_url = Some(neighbour_bound);
    let mut direct = account("direct", "direct@example.com", None);
    direct.egress_proxy_url = Some("http://127.0.0.1:1080".into());
    let unbound = account("unbound", "unbound@example.com", Some("Unbound"));

    let listed = dependency_accounts(vec![named, unnamed, elsewhere, direct, unbound], &source.id);
    assert_eq!(
        listed
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        ["named", "unnamed"],
        "只要直接绑定该来源的账号，且不含其它来源或直连代理"
    );
    assert_eq!(listed[0].name, "Work account");
    assert_eq!(listed[0].email, "named@example.com");
    assert_eq!(listed[1].name, "unnamed@example.com");
    assert_eq!(listed[1].email, "unnamed@example.com");
    let preview = serde_json::to_string(&listed).unwrap();
    for secret in [
        "private-node-secret",
        "private-subscription-secret",
        "subscription.example",
        "node.example",
        "cockpit-proxy://",
        "fake-access",
        "fake-id",
    ] {
        assert!(
            !preview.contains(secret),
            "{secret} leaked into the preview"
        );
    }
}
#[test]
fn catalog_storage_encrypts_and_keeps_old_data_on_failed_transaction() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let temp = Temp::new();
    let _restore = DataDir::set(&temp.0);
    let path = temp.0.join("catalog.json");
    mutate(&path, |s| {
        s.sources.push(fixture());
        Ok(())
    })
    .unwrap();
    let content = fs::read_to_string(&path).unwrap();
    assert!(content.contains("AES-256-GCM"));
    assert!(!content.contains("private-"));
    assert_eq!(read(&path).unwrap().sources.len(), 1);
    let result: Result<(), String> = mutate(&path, |s| {
        s.sources.clear();
        Err("CATALOG_CHANGED".into())
    });
    assert_eq!(result.unwrap_err(), "CATALOG_CHANGED");
    assert_eq!(fs::read_to_string(&path).unwrap(), content);
    fs::write(&path, "corrupt").unwrap();
    assert!(read(&path).is_err());
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "corrupt",
        "reads never replace data with an empty catalog"
    );
}

#[test]
fn catalog_read_recovers_derived_group_availability_without_rewriting_saved_data() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let temp = Temp::new();
    let _restore = DataDir::set(&temp.0);
    let path = temp.0.join("catalog.json");
    let mut source = fixture();
    source.catalog = parser::parse("proxies:\n  - {name: Alpha, type: http, server: localhost, port: 8080}\nproxy-groups:\n  - {name: Auto, type: url-test, proxies: [REJECT, Alpha]}\n  - {name: Pick, type: select, proxies: [Auto]}\n  - {name: Unsafe, type: url-test, proxies: [Alpha, DIRECT]}\n").unwrap();
    for group in &mut source.catalog.groups {
        group.error = Some("SUBSCRIPTION_GROUP_UNAVAILABLE".into());
    }
    mutate(&path, |store| {
        store.sources.push(source);
        Ok(())
    })
    .unwrap();
    let before = fs::read(&path).unwrap();
    let store = read(&path).unwrap();
    assert_eq!(fs::read(&path).unwrap(), before);
    let source = &store.sources[0];
    let auto = &source.catalog.groups[0];
    let pick = &source.catalog.groups[1];
    let unsafe_group = &source.catalog.groups[2];
    assert!(auto.error.is_none());
    assert!(pick.error.is_none());
    assert!(unsafe_group.error.is_some());
    let selected = BTreeMap::from([(pick.id.clone(), auto.name.clone())]);
    assert!(default_error("subscription", &source.catalog, &pick.id, &selected).is_none());
    assert!(default_error(
        "subscription",
        &source.catalog,
        &unsafe_group.id,
        &BTreeMap::new()
    )
    .is_some());
    assert!(default_error("subscription", &source.catalog, "REJECT", &BTreeMap::new()).is_some());
    let public = view(&store);
    assert!(public.sources[0].groups[0].supported);
    assert!(public.sources[0].groups[1].supported);
    assert!(!public.sources[0].groups[2].supported);
}

#[test]
fn retained_legacy_nodes_revalidate_without_granting_tls_permission_or_writing() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let temp = Temp::new();
    let _restore = DataDir::set(&temp.0);
    let file = temp.0.join("catalog.json");
    let mut source = subscription_fixture();
    source.parser_version = 0;
    let tls = source
        .catalog
        .nodes
        .iter_mut()
        .find(|node| node.name == "TLS")
        .unwrap();
    tls.error = Some("PROXY_UNSUPPORTED_OPTION".into());
    source.catalog.nodes[0].error = Some("PROXY_UNSUPPORTED_OPTION".into());
    // A separate source contains an existing explicit permission.
    let mut approved = source.clone();
    approved.id = "approved".into();
    approved
        .catalog
        .nodes
        .iter_mut()
        .find(|node| node.name == "TLS")
        .unwrap()
        .error = None;
    mutate(&file, |store| {
        store.sources = vec![source, approved];
        Ok(())
    })
    .unwrap();
    let before = fs::read(&file).unwrap();
    let loaded = read(&file).unwrap();
    assert_eq!(
        fs::read(&file).unwrap(),
        before,
        "reading does not migrate or rewrite storage"
    );
    assert!(loaded.sources[0].catalog.nodes[0].error.is_none());
    assert_eq!(
        loaded.sources[0].catalog.nodes[3].error.as_deref(),
        Some("PROXY_TLS_INSECURE")
    );
    assert!(
        loaded.sources[1].catalog.nodes[3].error.is_none(),
        "keep an existing explicit permission"
    );
    assert!(view(&loaded)
        .sources
        .iter()
        .all(|source| !source.needs_refresh));
}

#[test]
fn legacy_missing_definitions_request_explicit_refresh_but_current_rejections_do_not() {
    let mut legacy = subscription_fixture();
    legacy.parser_version = 0;
    legacy.catalog.nodes[3].outbound = None;
    legacy.catalog.nodes[3].error = Some("PROXY_UNSUPPORTED_OPTION".into());
    let mut current = legacy.clone();
    current.parser_version = PARSER_VERSION;
    current.id = "current".into();
    let mut serialized = serde_json::to_value(&legacy).unwrap();
    serialized.as_object_mut().unwrap().remove("parser_version");
    let legacy: Source = serde_json::from_value(serialized).unwrap();
    let store = Store {
        sources: vec![legacy, current],
        ..Store::default()
    };
    let public = view(&store);
    assert!(public.sources[0].needs_refresh);
    assert!(!public.sources[1].needs_refresh);
    assert!(!public.sources[0].nodes[3].supported);
    let json = serde_json::to_string(&public).unwrap();
    assert!(!json.contains("private-"));
    assert!(!json.contains("parser_version"));
}

fn group_permission_fixture() -> Source {
    let mut source = fixture();
    source.catalog = parser::parse("proxies:\n  - {name: Stable, type: http, server: stable.example, port: 8080}\n  - {name: AT&T, type: hysteria2, server: one.example, port: 443, password: private-one, skip-cert-verify: true}\n  - {name: Starlink, type: hysteria2, server: two.example, port: 443, password: private-two, skip-cert-verify: true}\n  - {name: Outside, type: hysteria2, server: other.example, port: 443, password: private-other, skip-cert-verify: true}\nproxy-groups:\n  - {name: Child, type: url-test, proxies: [Starlink]}\n  - {name: US, type: url-test, proxies: [Stable, AT&T, Child, Starlink], url: https://check.example/group, interval: 300, tolerance: 80}\n  - {name: Other, type: url-test, proxies: [Outside]}\n").unwrap();
    source
}

#[test]
fn group_diagnostics_identify_tls_members_without_claiming_a_cycle() {
    let source = group_permission_fixture();
    let public = view(&Store {
        sources: vec![source],
        ..Store::default()
    });
    let group = &public.sources[0].groups[1];
    assert!(!group.supported);
    assert_eq!(group.error.as_deref(), Some("PROXY_TLS_INSECURE"));
    assert_eq!(
        group
            .issues
            .iter()
            .map(|issue| (issue.name.as_str(), issue.error.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("AT&T", "PROXY_TLS_INSECURE"),
            ("Starlink", "PROXY_TLS_INSECURE")
        ]
    );
    assert_eq!(
        group.insecure_node_ids.len(),
        2,
        "shared descendants are not repeated"
    );
    assert_eq!(
        group.test_url.as_deref(),
        Some("https://check.example/group")
    );
    assert_eq!(public.sources[0].nodes[0].udp, Some(false));
    assert_eq!(public.sources[0].nodes[1].udp, Some(true));
}

#[test]
fn group_tls_permission_is_atomic_revision_checked_and_scoped_to_descendants() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let temp = Temp::new();
    let _restore = DataDir::set(&temp.0);
    let source = group_permission_fixture();
    let source_id = source.id.clone();
    let group_id = source.catalog.groups[1].id.clone();
    let old_binding = binding::encode(
        &source.id,
        &source.name,
        &source.catalog.nodes[0].id,
        &source.catalog,
        &BTreeMap::new(),
    )
    .unwrap();
    mutate(&path().unwrap(), |store| {
        store.sources.push(source);
        Ok(())
    })
    .unwrap();
    let account_file = temp.0.join("unrelated-account.json");
    fs::write(&account_file, b"existing account and binding").unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let before = fs::read(path().unwrap()).unwrap();
            assert_eq!(
                set_group_insecure(source_id.clone(), group_id.clone(), "stale".into(), true)
                    .await
                    .err()
                    .unwrap(),
                "CATALOG_CHANGED"
            );
            assert_eq!(fs::read(path().unwrap()).unwrap(), before);
            let allowed =
                set_group_insecure(source_id.clone(), group_id.clone(), "v1".into(), true)
                    .await
                    .unwrap();
            assert!(allowed.sources[0].groups[0].supported);
            assert!(allowed.sources[0].groups[1].supported);
            assert!(!allowed.sources[0].groups[2].supported);
            assert!(allowed.sources[0].nodes[1].supported && allowed.sources[0].nodes[2].supported);
            assert!(
                !allowed.sources[0].nodes[3].supported,
                "other groups are untouched"
            );
            assert!(allowed.sources[0].groups[1].insecure_node_ids.is_empty());
            let binding = snapshot(source_id.clone(), group_id.clone(), BTreeMap::new())
                .await
                .unwrap();
            let groups =
                binding::runtime_parts(serde_json::json!(binding::outbounds(&binding).unwrap()))
                    .unwrap()
                    .1;
            let us = groups
                .iter()
                .find(|group| group["url"] == "https://check.example/group")
                .unwrap();
            assert_eq!(
                us["proxies"].as_array().unwrap().len(),
                4,
                "all members and policy are retained"
            );
            assert_eq!(us["tolerance"], 80);
            let revoked = set_group_insecure(
                source_id,
                group_id,
                allowed.sources[0].revision.clone(),
                false,
            )
            .await
            .unwrap();
            assert!(!revoked.sources[0].groups[1].supported);
            assert_eq!(revoked.sources[0].groups[1].insecure_node_ids.len(), 2);
        });
    assert!(
        binding::decode(&old_binding).is_ok(),
        "existing account snapshots are unchanged"
    );
    assert_eq!(
        fs::read(&account_file).unwrap(),
        b"existing account and binding"
    );
}

#[test]
fn group_tls_permission_rejects_cycles_without_partial_writes() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let temp = Temp::new();
    let _restore = DataDir::set(&temp.0);
    let mut source = group_permission_fixture();
    source.catalog.groups[0].members.push("US".into());
    let source_id = source.id.clone();
    let group_id = source.catalog.groups[1].id.clone();
    mutate(&path().unwrap(), |store| {
        store.sources.push(source);
        Ok(())
    })
    .unwrap();
    let before = fs::read(path().unwrap()).unwrap();
    let error = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(set_group_insecure(source_id, group_id, "v1".into(), true))
        .err()
        .unwrap();
    assert_eq!(error, "SUBSCRIPTION_GROUP_CYCLE");
    assert_eq!(fs::read(path().unwrap()).unwrap(), before);
}

#[test]
fn latency_uses_selected_group_url_only_for_its_own_subtree() {
    let source = group_permission_fixture();
    let mut catalog = source.catalog;
    catalog.nodes[2].error = None;
    let us = &catalog.groups[1];
    assert_eq!(
        latency_target_url(&catalog, &catalog.nodes[0].id, Some(&us.id)).unwrap(),
        "https://check.example/group"
    );
    assert_eq!(
        latency_target_url(&catalog, &catalog.nodes[2].id, Some(&us.id)).unwrap(),
        "https://check.example/group",
        "parent URL applies to descendants measured under that group"
    );
    assert_eq!(
        latency_target_url(&catalog, &catalog.nodes[0].id, None).unwrap(),
        DEFAULT_LATENCY_URL
    );
    assert_eq!(
        latency_target_url(&catalog, &catalog.nodes[0].id, Some(&catalog.groups[2].id))
            .unwrap_err(),
        "CATALOG_INVALID"
    );
    assert!(
        latency_target_url(&catalog, &catalog.nodes[1].id, Some(&us.id)).is_err(),
        "pending permission cannot start a probe"
    );
    assert!(latency_target_url(&catalog, &catalog.nodes[0].id, Some("missing")).is_err());
    for url in [
        "file:///tmp/test",
        "https://user:private-password@check.example/",
        "https://check.example/#secret",
    ] {
        catalog.groups[1].url = Some(url.into());
        assert!(
            latency_target_url(&catalog, &catalog.nodes[0].id, Some(&catalog.groups[1].id))
                .is_err()
        );
        assert!(public_latency_url(&catalog.groups[1]).is_none());
    }
    catalog.groups[1].url = Some("https://check.example/?token=private-secret".into());
    assert!(
        public_latency_url(&catalog.groups[1]).is_none(),
        "query secrets stay out of IPC"
    );
}

#[test]
fn latency_checks_only_target_reachability_while_group_permissions_remain_strict() {
    let catalog = parser::parse("proxies:\n  - {name: Alpha, type: http, server: one.example, port: 8080}\n  - {name: Outside, type: http, server: other.example, port: 8080}\n  - {name: Pending, type: hysteria2, server: pending.example, port: 443, password: private-password, skip-cert-verify: true}\nproxy-groups:\n  - {name: MissingSibling, type: select, proxies: [Missing, Alpha, Pending], url: https://check.example/missing}\n  - {name: CycleSibling, type: select, proxies: [Loop, Child], url: https://check.example/cycle}\n  - {name: Loop, type: select, proxies: [Loop]}\n  - {name: Child, type: select, proxies: [Alpha, Pending]}\n").unwrap();
    let alpha = &catalog.nodes[0].id;
    let outside = &catalog.nodes[1].id;
    let pending = &catalog.nodes[2].id;
    for (group, expected_url, structural_error) in [
        (
            &catalog.groups[0],
            "https://check.example/missing",
            "SUBSCRIPTION_GROUP_MEMBER_MISSING",
        ),
        (
            &catalog.groups[1],
            "https://check.example/cycle",
            "SUBSCRIPTION_GROUP_CYCLE",
        ),
    ] {
        assert!(
            group.error.is_none(),
            "manual groups retain usable alternatives"
        );
        assert_eq!(
            latency_target_url(&catalog, alpha, Some(&group.id)).unwrap(),
            expected_url
        );
        assert_eq!(
            latency_target_url(&catalog, outside, Some(&group.id)).unwrap_err(),
            "CATALOG_INVALID"
        );
        assert_eq!(
            latency_target_url(&catalog, pending, Some(&group.id)).unwrap_err(),
            "CATALOG_INVALID"
        );
        assert_eq!(
            parser::reachable_nodes(&catalog, &group.id).err().unwrap(),
            structural_error,
            "bulk certificate permissions still require the complete valid subtree"
        );
    }
    assert_eq!(
        latency_target_url(&catalog, alpha, Some(&catalog.groups[2].id)).unwrap_err(),
        "CATALOG_INVALID",
        "a cycle without any route to the target terminates and stays out of scope"
    );
}
#[test]
fn urls_and_names_fail_with_safe_errors() {
    assert!(subscription_url("https://example.com/s?token=secret").is_ok());
    for input in [
        "http://example.com/s?token=secret",
        "file:///private/secret",
        "https://user:secret@example.com/s",
        "https://example.com/s#secret",
    ] {
        assert_eq!(subscription_url(input).unwrap_err(), "CATALOG_URL");
    }
    assert!(valid_name("美国节点").is_ok());
    assert!(valid_name("https://secret.example/").is_err());
}
#[tokio::test]
async fn cancellation_before_and_during_fetch_prevents_commit() {
    let id = uuid::Uuid::new_v4().to_string();
    cancel(id.clone()).unwrap();
    assert_eq!(Operation::begin(id).err().unwrap(), "CATALOG_CANCELLED");
    let id = uuid::Uuid::new_v4().to_string();
    let op = Operation::begin(id.clone()).unwrap();
    cancel(id).unwrap();
    let result: Result<(), String> = cancellable(&op, async { std::future::pending().await }).await;
    assert_eq!(result.unwrap_err(), "CATALOG_CANCELLED");
    assert!(commit(&op.state).is_err());
    let id = uuid::Uuid::new_v4().to_string();
    let op = Operation::begin(id.clone()).unwrap();
    commit(&op.state).unwrap();
    assert_eq!(cancel(id).unwrap_err(), "CATALOG_FINISHING");
}
#[test]
fn per_source_singleflight_and_cross_process_store_lock_are_bounded() {
    let id = uuid::Uuid::new_v4().to_string();
    let guard = SourceGuard::new(id.clone()).unwrap();
    assert!(SourceGuard::new(id.clone()).is_err());
    drop(guard);
    assert!(SourceGuard::new(id).is_ok());
    let temp = Temp::new();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(temp.0.join("codex-proxy-sources.lock"))
        .unwrap();
    lock.try_lock_exclusive().unwrap();
    let result = mutate(&temp.0.join("catalog.json"), |_| Ok(()));
    assert_eq!(result.unwrap_err(), "CATALOG_BUSY");
    assert!(!temp.0.join("catalog.json").exists());
}

#[tokio::test]
async fn response_size_cap_applies_to_stream_without_content_length() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = server.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut socket, _) = server.accept().await.unwrap();
        let mut buf = [0; 2048];
        let n = socket.read(&mut buf).await.unwrap();
        let request = String::from_utf8_lossy(&buf[..n]).to_lowercase();
        assert!(!request.contains("authorization:"));
        assert!(!request.contains("cookie:"));
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let oversized = vec![b'a'; MAX_BODY + 1];
        let _ = socket.write_all(&oversized).await;
    });
    let response = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap()
        .get(format!("http://{address}"))
        .send()
        .await
        .unwrap();
    assert_eq!(read_response(response).await.unwrap_err(), "CATALOG_LIMIT");
    task.await.unwrap();
}

#[tokio::test]
async fn timed_out_disk_worker_cannot_publish_after_operation_returns() {
    let operation = Operation::begin(uuid::Uuid::new_v4().to_string()).unwrap();
    let state = operation.state.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let (finished, done) = tokio::sync::oneshot::channel();
    let result = blocking_with_timeout(Duration::from_millis(20), move || {
        rx.recv().unwrap();
        let publish = commit(&state);
        finished.send(publish.is_err()).unwrap();
        publish
    })
    .await;
    assert_eq!(result.unwrap_err(), "CATALOG_TIMEOUT");
    drop(operation);
    tx.send(()).unwrap();
    assert!(done.await.unwrap());
}

#[test]
fn account_export_masks_the_entire_resource_snapshot() {
    use crate::models::codex::{CodexAccount, CodexTokens};
    let source = fixture();
    let raw = binding::encode(
        &source.id,
        &source.name,
        &source.catalog.nodes[0].id,
        &source.catalog,
        &BTreeMap::new(),
    )
    .unwrap();
    let mut account = CodexAccount::new(
        "fixture".into(),
        "test@example.com".into(),
        CodexTokens {
            access_token: "fake-access".into(),
            id_token: "fake-id".into(),
            refresh_token: None,
        },
    );
    account.egress_proxy_url = Some(raw.clone());
    let response = serde_json::to_value(&account).unwrap();
    assert!(response.get("egress_proxy_url").is_none());
    assert_eq!(response["egress_proxy"]["name"], "Demo");
    let text = response.to_string();
    assert!(!text.contains(&raw));
    assert!(!text.contains("private-node-secret"));
    let restored: CodexAccount = serde_json::from_value(response).unwrap();
    assert!(restored.egress_proxy_url.is_none());
    let persisted = super::super::codex_account_proxy::storage_value(&account).unwrap();
    assert_eq!(persisted["egress_proxy_url"], raw);
}

#[tokio::test]
async fn subscription_negotiation_retains_modern_nodes_without_sending_account_credentials() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = server.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut socket, _) = server.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0; 1];
            socket.read_exact(&mut byte).await.unwrap();
            request.push(byte[0]);
            assert!(request.len() < 8192);
        }
        let headers = String::from_utf8(request).unwrap().to_lowercase();
        assert!(!headers.contains("authorization:"));
        assert!(!headers.contains("cookie:"));
        let modern = headers.contains("user-agent: clash.meta/");
        let mut body = String::from("proxies:\n  - {name: Legacy, type: ss, server: example.com, port: 443, cipher: aes-128-gcm, password: demo}\n");
        if modern {
            body.push_str("  - {name: Modern, type: vless, server: example.org, port: 443, uuid: 00000000-0000-0000-0000-000000000001, tls: true}\n");
        }
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    });
    let client = subscription_client_builder()
        .https_only(false)
        .no_proxy()
        .build()
        .unwrap();
    let url = url::Url::parse(&format!("http://{address}/subscription")).unwrap();
    let body = fetch_with_client(client, url.clone(), url.origin())
        .await
        .unwrap();
    let catalog = parser::parse(&body.body).unwrap();
    assert_eq!(
        catalog.nodes.len(),
        2,
        "modern protocols must not be filtered by the provider"
    );
    assert!(catalog.nodes.iter().all(|n| n.error.is_none()));
    task.await.unwrap();
}

#[tokio::test]
#[ignore = "requires an explicitly supplied private URL file; downloads only, never changes accounts"]
async fn real_subscription_download_reports_only_counts() {
    let path =
        std::env::var("COCKPIT_TEST_SUBSCRIPTION_URL_FILE").expect("private URL file required");
    let input = fs::read_to_string(path).expect("cannot read private URL file");
    let body = fetch(input.trim())
        .await
        .expect("subscription download failed");
    let catalog = parser::parse(&body.body).expect("subscription parsing failed");
    let supported_nodes = catalog.nodes.iter().filter(|n| n.error.is_none()).count();
    let supported_groups = catalog.groups.iter().filter(|g| g.error.is_none()).count();
    println!(
        "nodes={}, supported_nodes={}, groups={}, supported_groups={}",
        catalog.nodes.len(),
        supported_nodes,
        catalog.groups.len(),
        supported_groups
    );
    assert!(supported_nodes > 0, "no supported nodes");
    assert!(supported_groups > 0, "no supported groups");
}

#[test]
fn subscription_titles_decode_official_metadata_and_reject_paths() {
    use base64::Engine as _;
    let mut headers = reqwest::header::HeaderMap::new();
    let title = base64::engine::general_purpose::STANDARD.encode("示例订阅");
    headers.insert("profile-title", format!("base64:{title}").parse().unwrap());
    assert_eq!(subscription_title(&headers).as_deref(), Some("示例订阅"));
    headers.remove("profile-title");
    headers.insert(
        reqwest::header::CONTENT_DISPOSITION,
        "attachment; filename*=UTF-8''%E7%A4%BA%E4%BE%8B.yaml"
            .parse()
            .unwrap(),
    );
    assert_eq!(subscription_title(&headers).as_deref(), Some("示例"));
    headers.insert(
        reqwest::header::CONTENT_DISPOSITION,
        "attachment; filename=\"/private/token=secret.yaml\""
            .parse()
            .unwrap(),
    );
    assert!(subscription_title(&headers).is_none());
    for title in [
        "https://example.test/?token=secret",
        "u:pass@server",
        "secret\nvalue",
    ] {
        assert!(safe_generated_name(title).is_none());
    }
}

#[test]
fn unnamed_import_partial_opt_in_duplicates_and_rename_preserve_bindings() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let temp = Temp::new();
    struct Restore(Option<std::ffi::OsString>, Option<std::ffi::OsString>);
    impl Drop for Restore {
        fn drop(&mut self) {
            if let Some(v) = self.1.take() {
                std::env::set_var("COCKPIT_TOOLS_DATA_DIR", v)
            } else {
                std::env::remove_var("COCKPIT_TOOLS_DATA_DIR")
            }
            if let Some(v) = self.0.take() {
                std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", v)
            } else {
                std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR")
            }
        }
    }
    let _restore = Restore(
        std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR"),
        std::env::var_os("COCKPIT_TOOLS_DATA_DIR"),
    );
    std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &temp.0);
    std::env::set_var("COCKPIT_TOOLS_DATA_DIR", &temp.0);
    assert!(path().unwrap().starts_with(&temp.0));
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let text = "proxy.example:1080:private-user:private-password\nbad line";
            let attempt = import(
                uuid::Uuid::new_v4().to_string(),
                String::new(),
                text.into(),
                "manual".into(),
                manual::ImportOptions::default(),
            )
            .await;
            assert_eq!(attempt.err().unwrap(), "IMPORT_INVALID");
            assert!(list().await.unwrap().sources.is_empty());
            let imported = import(
                uuid::Uuid::new_v4().to_string(),
                String::new(),
                text.into(),
                "manual".into(),
                manual::ImportOptions {
                    skip_invalid: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            assert_eq!(imported.sources[0].name, "SOCKS5 · proxy.example:1080");
            let id = imported.sources[0].id.clone();
            let stored = source(&id).await.unwrap();
            let frozen = binding::encode(
                &stored.id,
                &stored.name,
                &stored.catalog.nodes[0].id,
                &stored.catalog,
                &BTreeMap::new(),
            )
            .unwrap();
            let before = binding::outbounds(&frozen).unwrap();
            let renamed = rename(id.clone(), "New name".into()).await.unwrap();
            assert_eq!(renamed.sources[0].name, "New name");
            assert_ne!(renamed.sources[0].revision, imported.sources[0].revision);
            assert_eq!(source(&id).await.unwrap().catalog.nodes.len(), 1);
            assert_eq!(binding::outbounds(&frozen).unwrap(), before);
            let repeated = import(
                uuid::Uuid::new_v4().to_string(),
                String::new(),
                text.lines().next().unwrap().into(),
                "manual".into(),
                manual::ImportOptions::default(),
            )
            .await;
            assert_eq!(repeated.err().unwrap(), "IMPORT_EMPTY");
            let disk = fs::read_to_string(path().unwrap()).unwrap();
            assert!(!disk.contains("private-password"));
            assert!(!disk.contains("private-user"));
            assert_eq!(list().await.unwrap().sources.len(), 1);
            let network=super::super::codex_proxy_network::NetworkOptions{doh:true,interface:"en0".into()};
            let saved=set_network(id.clone(),renamed.sources[0].revision.clone(),network.clone()).await.unwrap();
            assert!(saved.sources[0].network.interface.is_empty());
            let snap=snapshot(id.clone(),stored.catalog.nodes[0].id.clone(),BTreeMap::new()).await.unwrap();
            assert!(binding::decode(&snap).unwrap().network.doh);
            assert!(binding::decode(&snap).unwrap().network.interface.is_empty());
            assert!(!binding::decode(&frozen).unwrap().network.doh);
            assert!(set_network(id.clone(),renamed.sources[0].revision.clone(),network).await.is_err());
            let config="proxies:\n  - {name: TLS, type: hysteria2, server: example.test, port: 443, password: private-pass, skip-cert-verify: true}\nproxy-groups:\n  - {name: Group, type: select, proxies: [TLS]}";
            let pending=parser::parse(config).unwrap();let node_id=pending.nodes[0].id.clone();
            mutate(&path().unwrap(),|store|{store.sources[0].catalog=pending.clone();Ok(())}).unwrap();
            assert!(snapshot(id.clone(),node_id.clone(),BTreeMap::new()).await.is_err());
            let allowed=set_insecure(id.clone(),node_id.clone(),saved.sources[0].revision.clone(),true).await.unwrap();
            assert!(allowed.sources[0].nodes[0].supported);assert!(allowed.sources[0].groups[0].supported);
            let approved=snapshot(id.clone(),node_id.clone(),BTreeMap::new()).await.unwrap();
            assert_eq!(binding::outbounds(&approved).unwrap()[0]["proxy"]["skip-cert-verify"],true);
            let old=source(&id).await.unwrap().catalog;
            let mut same=pending.clone();preserve_permissions(&old,&mut same).unwrap();assert!(same.nodes[0].error.is_none());
            let mut changed=pending.clone();changed.nodes[0].outbound.as_mut().unwrap()["proxy"]["password"]=serde_json::json!("different");preserve_permissions(&old,&mut changed).unwrap();assert_eq!(changed.nodes[0].error.as_deref(),Some("PROXY_TLS_INSECURE"));
            let revoked=set_insecure(id.clone(),node_id.clone(),allowed.sources[0].revision.clone(),false).await.unwrap();
            assert!(!revoked.sources[0].nodes[0].supported);assert!(snapshot(id,node_id,BTreeMap::new()).await.is_err());
            assert_eq!(binding::outbounds(&approved).unwrap()[0]["proxy"]["skip-cert-verify"],true);

        });
}
#[tokio::test]
async fn default_commands_store_only_validated_ids_and_members() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let temp = Temp::new();
    struct Restore(Option<std::ffi::OsString>, Option<std::ffi::OsString>);
    impl Drop for Restore {
        fn drop(&mut self) {
            if let Some(v) = self.0.take() {
                std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", v)
            } else {
                std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR")
            }
            if let Some(v) = self.1.take() {
                std::env::set_var("COCKPIT_TOOLS_DATA_DIR", v)
            } else {
                std::env::remove_var("COCKPIT_TOOLS_DATA_DIR")
            }
        }
    }
    let _restore = Restore(
        std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR"),
        std::env::var_os("COCKPIT_TOOLS_DATA_DIR"),
    );
    std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &temp.0);
    std::env::set_var("COCKPIT_TOOLS_DATA_DIR", &temp.0);
    let mut source = fixture();
    source.catalog = selectable_catalog();
    let source_id = source.id.clone();
    let node = source
        .catalog
        .nodes
        .iter()
        .find(|node| node.name == "Alpha")
        .unwrap()
        .id
        .clone();
    let manual = source
        .catalog
        .groups
        .iter()
        .find(|group| group.name == "Manual")
        .unwrap()
        .id
        .clone();
    let broken = source
        .catalog
        .groups
        .iter()
        .find(|group| group.name == "Broken")
        .unwrap()
        .id
        .clone();
    mutate(&path().unwrap(), |store| {
        store.sources.push(source);
        Ok(())
    })
    .unwrap();

    let saved = set_default(source_id.clone(), node.clone(), BTreeMap::new(), None)
        .await
        .unwrap();
    let stored = saved.sources[0].default.as_ref().unwrap();
    assert_eq!(stored.item_id, node);
    assert!(stored.group_id.is_none());
    assert!(!saved.sources[0].default_invalidated);
    let public = serde_json::to_string(&saved).unwrap();
    assert!(public.contains("\"default\":{\"itemId\":"), "{public}");
    assert!(public.contains("\"defaultInvalidated\":false"), "{public}");
    for secret in [
        "private-alpha",
        "private-tls",
        "private-subscription-secret",
        "subscription.example",
        "outbound",
    ] {
        assert!(!public.contains(secret), "{secret} 泄漏到默认项视图");
    }
    let mut projected = serde_json::to_value(&saved).unwrap();
    let nodes = projected["sources"][0]["nodes"].as_array_mut().unwrap();
    assert!(nodes.iter().any(|node| node["server"] == "alpha.example"));
    for node in nodes {
        node.as_object_mut().unwrap().remove("server");
    }
    assert!(
        !projected.to_string().contains("alpha.example"),
        "endpoint belongs only in node metadata"
    );

    assert_eq!(
        set_default(source_id.clone(), "missing".into(), BTreeMap::new(), None)
            .await
            .err()
            .unwrap(),
        "CATALOG_CHANGED"
    );
    assert_eq!(
        set_default(source_id.clone(), manual.clone(), BTreeMap::new(), None)
            .await
            .err()
            .unwrap(),
        "CATALOG_INVALID"
    );
    assert_eq!(
        set_default(
            source_id.clone(),
            manual.clone(),
            choices(&[(&manual, "TLS")]),
            None
        )
        .await
        .err()
        .unwrap(),
        "CATALOG_INVALID"
    );
    assert_eq!(
        set_default(
            source_id.clone(),
            node.clone(),
            BTreeMap::new(),
            Some(broken)
        )
        .await
        .err()
        .unwrap(),
        "CATALOG_INVALID"
    );
    assert_eq!(
        set_default("other".into(), node.clone(), BTreeMap::new(), None)
            .await
            .err()
            .unwrap(),
        "CATALOG_NOT_FOUND"
    );
    let grouped = set_default(
        source_id.clone(),
        manual.clone(),
        choices(&[(&manual, "Beta")]),
        Some(manual.clone()),
    )
    .await
    .unwrap();
    let stored = grouped.sources[0].default.as_ref().unwrap();
    assert_eq!(stored.group_id.as_deref(), Some(manual.as_str()));
    assert_eq!(
        stored.selections.get(&manual).map(String::as_str),
        Some("Beta")
    );

    // 模拟刷新后的新目录：引用节点消失时清空默认项并标记，而不是改绑到替代节点。
    mutate(&path().unwrap(), |store| {
        store.sources[0].catalog =
            parser::parse("trojan://other@other.example:443#Replaced").unwrap();
        Ok(())
    })
    .unwrap();
    let marked = blocking(|| {
        mutate(&path()?, |store| {
            drop_invalid_default(&mut store.sources[0]);
            Ok(view(store))
        })
    })
    .await
    .unwrap();
    assert!(marked.sources[0].default.is_none());
    assert!(marked.sources[0].default_invalidated);
    assert!(marked.sources[0].nodes[0].supported, "替代节点信息保持可用");
    let reloaded = list().await.unwrap();
    assert!(reloaded.sources[0].default.is_none());
    assert!(
        reloaded.sources[0].default_invalidated,
        "失效标记随加密文件落盘"
    );

    let cleared = clear_default(source_id.clone()).await.unwrap();
    assert!(cleared.sources[0].default.is_none());
    assert!(!cleared.sources[0].default_invalidated);
    assert!(clear_default(source_id).await.is_ok(), "重复清除必须幂等");
    assert_eq!(
        clear_default("other".into()).await.err().unwrap(),
        "CATALOG_NOT_FOUND"
    );
}

/// 自建策略保存后必须能被现有编码链路落成 Mihomo 组，且成员顺序就是主备顺序。
#[tokio::test]
async fn strategy_save_copies_members_and_encodes_the_group() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let temp = Temp::new();
    let _dir = DataDir::set(&temp.0);
    let source = subscription_fixture();
    mutate(&path().unwrap(), |store| {
        store.sources.push(subscription_fixture());
        Ok(())
    })
    .unwrap();

    let saved = save_strategy(
        None,
        " My Strategy ".into(),
        "fallback".into(),
        vec![item(&source, "Gamma"), item(&source, "Alpha")],
        StrategyOptions {
            url: Some("https://health.example/generate_204".into()),
            interval: Some(60),
            timeout: Some(5),
            tolerance: Some(80),
            lazy: Some(true),
        },
    )
    .await
    .unwrap();

    assert_eq!(saved.sources.len(), 2, "策略来源必须出现在来源列表接口里");
    let listed = saved
        .sources
        .iter()
        .find(|s| s.kind == strategy::KIND)
        .unwrap();
    assert_eq!(listed.name, "My Strategy", "名称两端空白必须收敛");
    assert!(!listed.auto_update, "策略来源没有 URL，不允许自动刷新");
    assert!(listed.default.is_none());
    assert_eq!(
        listed
            .nodes
            .iter()
            .map(|node| node.name.as_str())
            .collect::<Vec<_>>(),
        ["Gamma", "Alpha"],
        "复制顺序必须与勾选顺序一致"
    );
    let group = &listed.groups[0];
    assert_eq!(group.id, strategy::group_id(&listed.id));
    assert_eq!(group.kind, "fallback");
    assert_eq!(group.members, ["Gamma", "Alpha"]);
    assert!(group.supported, "自建策略的 fallback 分组必须可用");

    // 节点确实是副本：outbound 与原来源一致，来源本身没有 URL、没有默认项。
    let store = stored();
    let strategy_source = store
        .sources
        .iter()
        .find(|s| s.kind == strategy::KIND)
        .unwrap();
    assert!(strategy_source.url.is_none());
    assert!(!strategy_source.auto_update);
    assert!(strategy_source.last_attempt_at.is_none());
    assert!(strategy_source.error.is_none());
    for name in ["Gamma", "Alpha"] {
        let copy = strategy_source
            .catalog
            .nodes
            .iter()
            .find(|node| node.name == name)
            .unwrap();
        let original = source
            .catalog
            .nodes
            .iter()
            .find(|node| node.name == name)
            .unwrap();
        assert_eq!(copy.id, original.id, "{name} 沿用解析器的稳定节点 ID");
        assert_eq!(copy.protocol, original.protocol);
        assert_eq!(copy.outbound, original.outbound, "{name}");
        assert!(copy.error.is_none());
    }

    let snapshot = snapshot_with_group(listed.id.clone(), group.id.clone(), BTreeMap::new(), None)
        .await
        .unwrap();
    let outbounds = binding::outbounds(&snapshot).unwrap();
    let encoded = outbounds
        .iter()
        .find(|outbound| outbound["type"] == "mihomo-group")
        .unwrap();
    let names = binding::names(&snapshot).unwrap();
    assert_eq!(encoded["group"]["type"], "fallback");
    assert_eq!(
        encoded["group"]["url"],
        "https://health.example/generate_204"
    );
    assert_eq!(encoded["group"]["interval"], 60);
    assert_eq!(
        encoded["group"]["timeout"], 5000,
        "超时按秒保存、按毫秒编码"
    );
    assert_eq!(encoded["group"]["lazy"], true);
    assert!(
        encoded["group"].get("tolerance").is_none(),
        "fallback 不写 url-test 专有的 tolerance"
    );
    assert_eq!(
        encoded["group"]["proxies"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tag| names[tag.as_str().unwrap()].clone())
            .collect::<Vec<_>>(),
        ["Gamma", "Alpha"]
    );

    // 单个成员副本同样能按现有链路绑定，不依赖分组选择。
    let gamma = strategy_source
        .catalog
        .nodes
        .iter()
        .find(|node| node.name == "Gamma")
        .unwrap()
        .id
        .clone();
    let node_snapshot = snapshot_with_group(listed.id.clone(), gamma, BTreeMap::new(), None)
        .await
        .unwrap();
    let node_outbounds = binding::outbounds(&node_snapshot).unwrap();
    assert_eq!(node_outbounds.len(), 1);
    assert_eq!(node_outbounds[0]["type"], "mihomo");
    assert_eq!(node_outbounds[0]["proxy"]["server"], "gamma.example");
}

/// 旧文件缺少成员身份字段时仍能读取，策略来源按“无记录”处理。
#[test]
fn stored_sources_without_strategy_members_stay_readable() {
    let source = subscription_fixture();
    let mut legacy = serde_json::to_value(&source).unwrap();
    legacy.as_object_mut().unwrap().remove("strategy_members");
    let loaded: Source = serde_json::from_value(legacy).unwrap();
    assert!(loaded.strategy_members.is_none());
    let store = Store {
        version: 1,
        sources: vec![loaded],
    };
    let public = serde_json::to_string(&view(&store)).unwrap();
    assert!(public.contains("\"strategyMembers\":null"), "{public}");
}

/// 保存策略后必须按勾选顺序回读成员的原来源身份，而不是只有副本名可猜。
#[tokio::test]
async fn strategy_save_records_member_identity_in_selection_order() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let temp = Temp::new();
    let _dir = DataDir::set(&temp.0);
    let source = subscription_fixture();
    mutate(&path().unwrap(), |store| {
        store.sources.push(subscription_fixture());
        Ok(())
    })
    .unwrap();

    let gamma = item(&source, "Gamma");
    let alpha = item(&source, "Alpha");
    let saved = save_strategy(
        None,
        "Strategy".into(),
        "fallback".into(),
        vec![gamma.clone(), alpha.clone()],
        StrategyOptions::default(),
    )
    .await
    .unwrap();
    let listed = saved
        .sources
        .iter()
        .find(|s| s.kind == strategy::KIND)
        .unwrap();
    let records = listed.strategy_members.as_ref().unwrap();
    assert_eq!(
        records
            .iter()
            .map(|record| (
                record.source_id.as_str(),
                record.item_id.as_str(),
                record.name.as_str(),
                record.source_name.as_str(),
            ))
            .collect::<Vec<_>>(),
        [
            (source.id.as_str(), gamma.item_id.as_str(), "Gamma", "Demo"),
            (source.id.as_str(), alpha.item_id.as_str(), "Alpha", "Demo"),
        ],
        "记录必须与勾选顺序一致，并带上原来源与节点 ID"
    );
    // IPC 输出必须是 camelCase：前端按 strategyMembers 精确回读每个成员。
    let public = serde_json::to_value(&saved).unwrap();
    let sources = public["sources"].as_array().unwrap();
    let strategy = sources
        .iter()
        .find(|entry| entry["kind"] == "strategy")
        .unwrap();
    let first = &strategy["strategyMembers"][0];
    assert_eq!(first["sourceId"].as_str(), Some(source.id.as_str()));
    assert_eq!(first["itemId"].as_str(), Some(gamma.item_id.as_str()));
    assert_eq!(first["name"].as_str(), Some("Gamma"));
    assert_eq!(first["sourceName"].as_str(), Some("Demo"));
    // 非策略来源不返回成员身份：订阅来源的 JSON 里该字段保持 null。
    let subscription = sources
        .iter()
        .find(|entry| entry["kind"] == "subscription")
        .unwrap();
    assert!(subscription["strategyMembers"].is_null(), "{subscription}");
}

/// 两个来源出现同名节点时，内核按名称只保留首次勾选的副本，身份记录也必须只有一条。
#[tokio::test]
async fn strategy_save_records_only_the_first_member_of_a_duplicated_name() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let temp = Temp::new();
    let _dir = DataDir::set(&temp.0);
    let mut first = subscription_fixture();
    first.name = "First".into();
    let mut second = subscription_fixture();
    second.id = "second-id".into();
    second.name = "Second".into();
    mutate(&path().unwrap(), |store| {
        store.sources.push(first.clone());
        store.sources.push(second.clone());
        Ok(())
    })
    .unwrap();

    let saved = save_strategy(
        None,
        "Strategy".into(),
        "fallback".into(),
        vec![item(&first, "Alpha"), item(&second, "Alpha")],
        StrategyOptions::default(),
    )
    .await
    .unwrap();
    let listed = saved
        .sources
        .iter()
        .find(|s| s.kind == strategy::KIND)
        .unwrap();
    assert_eq!(
        listed
            .nodes
            .iter()
            .map(|node| node.name.as_str())
            .collect::<Vec<_>>(),
        ["Alpha"]
    );
    assert_eq!(listed.groups[0].members, ["Alpha"]);
    let records = listed.strategy_members.as_ref().unwrap();
    assert_eq!(
        records.len(),
        1,
        "同名去重是有意行为：回读记录不能包含并未进入 catalog 的成员"
    );
    assert_eq!(records[0].source_id, first.id);
    assert_eq!(records[0].item_id, item(&first, "Alpha").item_id);
    assert_eq!(records[0].source_name, "First");
}

/// 编辑保存整体替换身份记录：旧成员不追加，原来源改名后再次保存即刷新。
#[tokio::test]
async fn strategy_update_replaces_member_identity_records() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let temp = Temp::new();
    let _dir = DataDir::set(&temp.0);
    let source = subscription_fixture();
    mutate(&path().unwrap(), |store| {
        store.sources.push(subscription_fixture());
        Ok(())
    })
    .unwrap();

    let created = save_strategy(
        None,
        "Strategy".into(),
        "fallback".into(),
        vec![item(&source, "Alpha"), item(&source, "Beta")],
        StrategyOptions::default(),
    )
    .await
    .unwrap();
    let id = created
        .sources
        .iter()
        .find(|s| s.kind == strategy::KIND)
        .unwrap()
        .id
        .clone();
    // 原来源改名后再次保存策略：记录里的来源名必须是当前名称。
    mutate(&path().unwrap(), |store| {
        let entry = store
            .sources
            .iter_mut()
            .find(|entry| entry.id == source.id)
            .unwrap();
        entry.name = "Renamed".into();
        Ok(())
    })
    .unwrap();
    let updated = save_strategy(
        Some(id.clone()),
        "Strategy".into(),
        "fallback".into(),
        vec![item(&source, "Gamma")],
        StrategyOptions::default(),
    )
    .await
    .unwrap();
    let listed = updated.sources.iter().find(|s| s.id == id).unwrap();
    let records = listed.strategy_members.as_ref().unwrap();
    assert_eq!(records.len(), 1, "编辑保存必须替换而不是追加历史成员");
    assert_eq!(records[0].name, "Gamma");
    assert_eq!(records[0].source_name, "Renamed");
}

/// 原来源被删除后再次编辑保存：策略继续使用自己保存的副本，成员不会被静默丢掉。
#[tokio::test]
async fn strategy_update_keeps_saved_copies_after_the_original_source_is_removed() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let temp = Temp::new();
    let _dir = DataDir::set(&temp.0);
    let source = subscription_fixture();
    mutate(&path().unwrap(), |store| {
        store.sources.push(subscription_fixture());
        Ok(())
    })
    .unwrap();

    let alpha = item(&source, "Alpha");
    let beta = item(&source, "Beta");
    let original_beta = source
        .catalog
        .nodes
        .iter()
        .find(|node| node.name == "Beta")
        .unwrap()
        .outbound
        .clone();
    let created = save_strategy(
        None,
        "Strategy".into(),
        "fallback".into(),
        vec![alpha.clone(), beta.clone()],
        StrategyOptions::default(),
    )
    .await
    .unwrap();
    let id = created
        .sources
        .iter()
        .find(|s| s.kind == strategy::KIND)
        .unwrap()
        .id
        .clone();

    // 用户删除了原来源：策略里的副本与身份记录都留下，只是原来源不再存在。
    mutate(&path().unwrap(), |store| {
        store.sources.retain(|entry| entry.id != source.id);
        Ok(())
    })
    .unwrap();

    // 改名 + 换类型 + 调换顺序：副本必须按本次提交重建，成员一个都不能少。
    let updated = save_strategy(
        Some(id.clone()),
        "Renamed".into(),
        "url-test".into(),
        vec![beta.clone(), alpha.clone()],
        StrategyOptions::default(),
    )
    .await
    .unwrap();
    let listed = updated.sources.iter().find(|s| s.id == id).unwrap();
    assert_eq!(listed.name, "Renamed");
    assert_eq!(listed.groups[0].kind, "url-test");
    assert_eq!(listed.groups[0].members, ["Beta", "Alpha"]);
    assert_eq!(
        listed
            .nodes
            .iter()
            .map(|node| node.name.as_str())
            .collect::<Vec<_>>(),
        ["Beta", "Alpha"],
        "原来源删除后成员不得被丢掉或改写"
    );
    let records = listed.strategy_members.as_ref().unwrap();
    assert_eq!(
        records
            .iter()
            .map(|record| (
                record.source_id.as_str(),
                record.item_id.as_str(),
                record.name.as_str(),
                record.source_name.as_str(),
            ))
            .collect::<Vec<_>>(),
        [
            (source.id.as_str(), beta.item_id.as_str(), "Beta", "Demo"),
            (source.id.as_str(), alpha.item_id.as_str(), "Alpha", "Demo"),
        ],
        "来源已删除时沿用上次保存的身份与来源名"
    );
    let store = stored();
    let strategy_source = store.sources.iter().find(|s| s.id == id).unwrap();
    assert_eq!(strategy_source.catalog.nodes.len(), 2, "节点数量不变");
    assert_eq!(
        strategy_source
            .catalog
            .nodes
            .iter()
            .find(|node| node.name == "Beta")
            .unwrap()
            .outbound,
        original_beta,
        "副本仍是原来源节点的 outbound"
    );

    // 再次编辑仍然认得这些副本，顺序按新一次提交。
    let again = save_strategy(
        Some(id.clone()),
        "Renamed".into(),
        "fallback".into(),
        vec![alpha, beta],
        StrategyOptions::default(),
    )
    .await
    .unwrap();
    let listed = again.sources.iter().find(|s| s.id == id).unwrap();
    assert_eq!(
        listed
            .nodes
            .iter()
            .map(|node| node.name.as_str())
            .collect::<Vec<_>>(),
        ["Alpha", "Beta"]
    );
    // IPC 视图继续回读身份记录：前端据此提示“继续使用已保存副本”。
    let public = serde_json::to_value(&again).unwrap();
    let strategy = public["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == id.as_str())
        .unwrap();
    assert_eq!(
        strategy["strategyMembers"][0]["sourceId"].as_str(),
        Some(source.id.as_str())
    );
    assert_eq!(
        strategy["strategyMembers"][0]["name"].as_str(),
        Some("Alpha")
    );
}

/// 原来源仍在时节点被移除或不可用：保存必须按原有错误码失败，绝不用副本顶替。
#[tokio::test]
async fn strategy_update_never_replaces_a_vanished_source_node_with_its_copy() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let temp = Temp::new();
    let _dir = DataDir::set(&temp.0);
    let source = subscription_fixture();
    mutate(&path().unwrap(), |store| {
        store.sources.push(subscription_fixture());
        Ok(())
    })
    .unwrap();

    let alpha = item(&source, "Alpha");
    let beta = item(&source, "Beta");
    let created = save_strategy(
        None,
        "Strategy".into(),
        "fallback".into(),
        vec![alpha, beta.clone()],
        StrategyOptions::default(),
    )
    .await
    .unwrap();
    let id = created
        .sources
        .iter()
        .find(|s| s.kind == strategy::KIND)
        .unwrap()
        .id
        .clone();

    // 来源刷新后不再包含 Beta：节点 ID 消失，保持 CATALOG_CHANGED。
    mutate(&path().unwrap(), |store| {
        store
            .sources
            .iter_mut()
            .find(|entry| entry.id == source.id)
            .unwrap()
            .catalog = parser::parse(
            "proxies:\n  - {name: Alpha, type: trojan, server: alpha.example, port: 443, password: private-alpha}\n",
        )
        .unwrap();
        Ok(())
    })
    .unwrap();
    assert_eq!(
        save_strategy(
            Some(id.clone()),
            "Strategy".into(),
            "fallback".into(),
            vec![beta],
            StrategyOptions::default(),
        )
        .await
        .err()
        .unwrap(),
        "CATALOG_CHANGED"
    );

    // 节点仍在但未获授权（跳过证书校验）：保持 CATALOG_INVALID。
    mutate(&path().unwrap(), |store| {
        store
            .sources
            .iter_mut()
            .find(|entry| entry.id == source.id)
            .unwrap()
            .catalog = parser::parse(
            "proxies:\n  - {name: TLS, type: hysteria2, server: tls.example, port: 443, password: private-tls, skip-cert-verify: true}\n",
        )
        .unwrap();
        Ok(())
    })
    .unwrap();
    assert_eq!(
        save_strategy(
            Some(id.clone()),
            "Strategy".into(),
            "fallback".into(),
            vec![item(&source, "TLS")],
            StrategyOptions::default(),
        )
        .await
        .err()
        .unwrap(),
        "CATALOG_INVALID"
    );

    let store = stored();
    let strategy_source = store.sources.iter().find(|s| s.id == id).unwrap();
    assert_eq!(
        strategy_source
            .catalog
            .nodes
            .iter()
            .map(|node| node.name.as_str())
            .collect::<Vec<_>>(),
        ["Alpha", "Beta"],
        "保存失败不得改动已保存的策略"
    );
    assert_eq!(strategy_source.strategy_members.as_ref().unwrap().len(), 2);
}

/// 回退只允许用于被编辑策略自己的记录：新建策略与别的策略都不能引用它的副本。
#[tokio::test]
async fn strategies_never_borrow_another_strategys_saved_copy() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let temp = Temp::new();
    let _dir = DataDir::set(&temp.0);
    let source = subscription_fixture();
    mutate(&path().unwrap(), |store| {
        store.sources.push(subscription_fixture());
        Ok(())
    })
    .unwrap();

    let alpha = item(&source, "Alpha");
    let beta = item(&source, "Beta");
    let created = save_strategy(
        None,
        "First".into(),
        "fallback".into(),
        vec![alpha.clone()],
        StrategyOptions::default(),
    )
    .await
    .unwrap();
    let first = created
        .sources
        .iter()
        .find(|s| s.kind == strategy::KIND)
        .unwrap()
        .id
        .clone();
    let created = save_strategy(
        None,
        "Second".into(),
        "fallback".into(),
        vec![beta],
        StrategyOptions::default(),
    )
    .await
    .unwrap();
    let second = created
        .sources
        .iter()
        .find(|s| s.kind == strategy::KIND && s.id != first)
        .unwrap()
        .id
        .clone();

    mutate(&path().unwrap(), |store| {
        store.sources.retain(|entry| entry.id != source.id);
        Ok(())
    })
    .unwrap();

    assert_eq!(
        save_strategy(
            None,
            "Third".into(),
            "fallback".into(),
            vec![alpha.clone()],
            StrategyOptions::default(),
        )
        .await
        .err()
        .unwrap(),
        "CATALOG_NOT_FOUND",
        "新建策略不能引用别的策略的副本"
    );
    assert_eq!(
        save_strategy(
            Some(second.clone()),
            "Second".into(),
            "fallback".into(),
            vec![alpha.clone()],
            StrategyOptions::default(),
        )
        .await
        .err()
        .unwrap(),
        "CATALOG_NOT_FOUND",
        "只能回退到被编辑策略自己的身份记录"
    );
    // 被编辑策略自己的成员仍然可以原样保存。
    let saved = save_strategy(
        Some(first.clone()),
        "First".into(),
        "fallback".into(),
        vec![alpha],
        StrategyOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        saved
            .sources
            .iter()
            .find(|s| s.id == first)
            .unwrap()
            .nodes
            .len(),
        1
    );
    let store = stored();
    assert_eq!(store.sources.len(), 2, "失败的保存不得新增或改动来源");
    assert_eq!(
        store
            .sources
            .iter()
            .find(|s| s.id == second)
            .unwrap()
            .catalog
            .nodes
            .iter()
            .map(|node| node.name.as_str())
            .collect::<Vec<_>>(),
        ["Beta"]
    );
}

/// 删除来源的影响预览只列出引用该来源成员副本的策略，且不含地址或凭据。
#[test]
fn removal_dependencies_list_only_strategies_that_reference_the_source() {
    let subscription = subscription_fixture();
    let mut referencing = subscription_fixture();
    referencing.id = "strategy-id".into();
    referencing.name = "My Strategy".into();
    referencing.kind = strategy::KIND.into();
    referencing.url = None;
    referencing.strategy_members = Some(vec![StrategyMemberRecord {
        source_id: subscription.id.clone(),
        item_id: "node-id".into(),
        name: "Alpha".into(),
        source_name: subscription.name.clone(),
    }]);
    let mut unrelated = subscription_fixture();
    unrelated.id = "unrelated-id".into();
    unrelated.name = "Unrelated".into();
    unrelated.kind = strategy::KIND.into();
    unrelated.url = None;
    unrelated.strategy_members = Some(vec![StrategyMemberRecord {
        source_id: "elsewhere".into(),
        item_id: "node-id".into(),
        name: "Beta".into(),
        source_name: "Elsewhere".into(),
    }]);
    // 旧策略没有身份记录时无法判定成员来源，不能误报。
    let mut legacy = subscription_fixture();
    legacy.id = "legacy-id".into();
    legacy.name = "Legacy".into();
    legacy.kind = strategy::KIND.into();
    legacy.url = None;

    let sources = vec![subscription.clone(), referencing, unrelated, legacy];
    assert_eq!(
        strategy_dependency_names(&sources, &subscription.id)
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["My Strategy"]
    );
    assert!(strategy_dependency_names(&sources, "missing").is_empty());
}

/// 校验失败必须在落盘前拦住，且不修改任何已有来源。
#[tokio::test]
async fn strategy_save_rejects_invalid_input_without_touching_the_store() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let temp = Temp::new();
    let _dir = DataDir::set(&temp.0);
    let source = subscription_fixture();
    mutate(&path().unwrap(), |store| {
        store.sources.push(subscription_fixture());
        Ok(())
    })
    .unwrap();
    let members = vec![item(&source, "Alpha"), item(&source, "Beta")];
    let options = StrategyOptions::default();
    let cases: Vec<(
        Option<String>,
        String,
        String,
        Vec<StrategyMember>,
        StrategyOptions,
        &str,
    )> = vec![
        (
            None,
            "   ".into(),
            "fallback".into(),
            members.clone(),
            options.clone(),
            "CATALOG_NAME",
        ),
        (
            None,
            "Bad\nName".into(),
            "fallback".into(),
            members.clone(),
            options.clone(),
            "CATALOG_NAME",
        ),
        (
            None,
            "Strategy".into(),
            "relay".into(),
            members.clone(),
            options.clone(),
            "CATALOG_INVALID",
        ),
        (
            None,
            "Strategy".into(),
            "fallback".into(),
            vec![],
            options.clone(),
            "CATALOG_INVALID",
        ),
        (
            None,
            "Strategy".into(),
            "fallback".into(),
            vec![members[0].clone(), members[0].clone()],
            options.clone(),
            "CATALOG_INVALID",
        ),
        (
            None,
            "Strategy".into(),
            "fallback".into(),
            vec![StrategyMember {
                source_id: "missing".into(),
                item_id: "node".into(),
            }],
            options.clone(),
            "CATALOG_NOT_FOUND",
        ),
        (
            None,
            "Strategy".into(),
            "fallback".into(),
            vec![StrategyMember {
                source_id: source.id.clone(),
                item_id: "missing".into(),
            }],
            options.clone(),
            "CATALOG_CHANGED",
        ),
        (
            None,
            "Strategy".into(),
            "fallback".into(),
            vec![item(&source, "TLS")],
            options.clone(),
            "CATALOG_INVALID",
        ),
        (
            None,
            "Strategy".into(),
            "fallback".into(),
            members.clone(),
            StrategyOptions {
                interval: Some(10),
                ..Default::default()
            },
            "CATALOG_INVALID",
        ),
        (
            None,
            "Strategy".into(),
            "fallback".into(),
            members.clone(),
            StrategyOptions {
                timeout: Some(0),
                ..Default::default()
            },
            "CATALOG_INVALID",
        ),
        (
            None,
            "Strategy".into(),
            "fallback".into(),
            members.clone(),
            StrategyOptions {
                tolerance: Some(5000),
                ..Default::default()
            },
            "CATALOG_INVALID",
        ),
        (
            None,
            "Strategy".into(),
            "fallback".into(),
            members.clone(),
            StrategyOptions {
                url: Some("file:///private/secret".into()),
                ..Default::default()
            },
            "CATALOG_INVALID",
        ),
    ];
    for (id, name, kind, list, options, expected) in cases {
        assert_eq!(
            save_strategy(id, name, kind, list, options)
                .await
                .err()
                .unwrap(),
            expected
        );
    }
    let mut overflow = Vec::new();
    for index in 0..=strategy::MAX_MEMBERS {
        overflow.push(StrategyMember {
            source_id: source.id.clone(),
            item_id: format!("node-{index}"),
        });
    }
    assert_eq!(
        save_strategy(
            None,
            "Strategy".into(),
            "fallback".into(),
            overflow,
            StrategyOptions::default()
        )
        .await
        .err()
        .unwrap(),
        "CATALOG_INVALID",
        "超过成员上限必须被拒绝"
    );
    assert_eq!(stored().sources.len(), 1, "校验失败不得写入任何来源");
}

/// 策略来源改名要同步分组名（分组 ID 不变），与成员同名时必须拒绝且不改动任何数据。
#[tokio::test]
async fn strategy_rename_keeps_the_group_id_and_rejects_member_names() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let temp = Temp::new();
    let _dir = DataDir::set(&temp.0);
    let source = subscription_fixture();
    mutate(&path().unwrap(), |store| {
        store.sources.push(subscription_fixture());
        Ok(())
    })
    .unwrap();
    let created = save_strategy(
        None,
        "Strategy".into(),
        "fallback".into(),
        vec![item(&source, "Alpha"), item(&source, "Beta")],
        StrategyOptions::default(),
    )
    .await
    .unwrap();
    let id = created
        .sources
        .iter()
        .find(|s| s.kind == strategy::KIND)
        .unwrap()
        .id
        .clone();
    let group = strategy::group_id(&id);

    let renamed = rename(id.clone(), " Renamed ".into()).await.unwrap();
    let listed = renamed.sources.iter().find(|s| s.id == id).unwrap();
    assert_eq!(listed.name, "Renamed");
    assert_eq!(listed.groups[0].name, "Renamed", "分组名与来源名保持同步");
    assert_eq!(listed.groups[0].id, group, "改名不得更换分组 ID");
    assert!(
        snapshot_with_group(id.clone(), group.clone(), BTreeMap::new(), None)
            .await
            .is_ok(),
        "改名后已有绑定仍按原分组 ID 解析"
    );
    assert_eq!(
        rename(id.clone(), "Alpha".into()).await.err().unwrap(),
        "CATALOG_INVALID",
        "分组名与成员同名时编码会解析到节点，必须拒绝"
    );
    assert_eq!(
        stored()
            .sources
            .iter()
            .find(|s| s.id == id)
            .unwrap()
            .catalog
            .groups[0]
            .name,
        "Renamed"
    );
}
/// 更新保留原 ID（分组 ID 也随之稳定），删除走现有来源删除路径并返回最新 CatalogView。
#[tokio::test]
async fn strategy_update_keeps_the_id_and_removal_drops_it_from_the_view() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let temp = Temp::new();
    let _dir = DataDir::set(&temp.0);
    let source = subscription_fixture();
    mutate(&path().unwrap(), |store| {
        store.sources.push(subscription_fixture());
        Ok(())
    })
    .unwrap();

    let created = save_strategy(
        None,
        "Strategy".into(),
        "fallback".into(),
        vec![item(&source, "Alpha"), item(&source, "Beta")],
        StrategyOptions::default(),
    )
    .await
    .unwrap();
    let id = created
        .sources
        .iter()
        .find(|s| s.kind == strategy::KIND)
        .unwrap()
        .id
        .clone();
    let group = strategy::group_id(&id);
    // 自建策略的自动分组可以直接设为来源默认项。
    let defaulted = set_default(id.clone(), group.clone(), BTreeMap::new(), None)
        .await
        .unwrap();
    assert_eq!(
        defaulted
            .sources
            .iter()
            .find(|s| s.id == id)
            .unwrap()
            .default
            .as_ref()
            .unwrap()
            .item_id,
        group
    );
    // 默认项指向成员节点时，成员被移除后必须失效而不是改绑到别的节点。
    let beta = item(&source, "Beta").item_id;
    assert!(set_default(id.clone(), beta, BTreeMap::new(), None)
        .await
        .unwrap()
        .sources
        .iter()
        .find(|s| s.id == id)
        .unwrap()
        .default
        .is_some());

    let updated = save_strategy(
        Some(id.clone()),
        " Renamed ".into(),
        "url-test".into(),
        vec![item(&source, "Gamma")],
        StrategyOptions::default(),
    )
    .await
    .unwrap();
    let listed = updated
        .sources
        .iter()
        .find(|s| s.kind == strategy::KIND)
        .unwrap();
    assert_eq!(listed.id, id, "更新必须保留原来源 ID");
    assert_eq!(listed.name, "Renamed");
    assert_eq!(listed.groups[0].id, group, "分组 ID 不因改名或换成员变化");
    assert_eq!(listed.groups[0].kind, "url-test");
    assert_eq!(listed.groups[0].members, ["Gamma"]);
    assert!(listed.default.is_none());
    assert!(listed.default_invalidated, "成员消失后默认项必须标记失效");

    // 只允许更新已存在的策略来源：未知 ID 与其他来源类型都必须被拒绝。
    assert_eq!(
        save_strategy(
            Some("missing".into()),
            "Strategy".into(),
            "fallback".into(),
            vec![item(&source, "Alpha")],
            StrategyOptions::default()
        )
        .await
        .err()
        .unwrap(),
        "CATALOG_NOT_FOUND"
    );
    assert_eq!(
        save_strategy(
            Some(source.id.clone()),
            "Strategy".into(),
            "fallback".into(),
            vec![item(&source, "Alpha")],
            StrategyOptions::default()
        )
        .await
        .err()
        .unwrap(),
        "CATALOG_INVALID",
        "订阅来源不能被策略保存改写"
    );

    let removed = crate::commands::codex_proxy_catalog::codex_proxy_strategy_remove(id.clone())
        .await
        .unwrap();
    assert!(removed.sources.iter().all(|s| s.id != id));
    assert_eq!(removed.sources.len(), 1, "订阅来源不受影响");
    assert_eq!(
        ensure_strategy(&id).await.err().unwrap(),
        "CATALOG_NOT_FOUND"
    );
    assert_eq!(
        snapshot_with_group(id, group, BTreeMap::new(), None)
            .await
            .err()
            .unwrap(),
        "CATALOG_NOT_FOUND"
    );
    assert_eq!(
        crate::commands::codex_proxy_catalog::codex_proxy_strategy_remove(source.id.clone())
            .await
            .err()
            .unwrap(),
        "CATALOG_INVALID",
        "非策略来源不能通过策略删除入口删除"
    );
    assert!(stored().sources.iter().any(|s| s.id == source.id));
}

/// 策略来源没有 URL，永远不进入订阅自动刷新队列，也不能打开自动更新。
#[tokio::test]
async fn strategy_sources_never_enter_the_auto_refresh_queue() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let temp = Temp::new();
    let _dir = DataDir::set(&temp.0);
    let source = subscription_fixture();
    mutate(&path().unwrap(), |store| {
        store.sources.push(subscription_fixture());
        Ok(())
    })
    .unwrap();
    let created = save_strategy(
        None,
        "Strategy".into(),
        "fallback".into(),
        vec![item(&source, "Alpha")],
        StrategyOptions::default(),
    )
    .await
    .unwrap();
    let id = created
        .sources
        .iter()
        .find(|s| s.kind == strategy::KIND)
        .unwrap()
        .id
        .clone();

    let now = chrono::Utc::now().timestamp_millis();
    let store = stored();
    assert_eq!(
        due_sources(&store.sources, now),
        [source.id.clone()],
        "只有到期的订阅来源会被自动刷新"
    );
    // 即使手工把策略来源标记成开启，也不能被自动刷新队列选中。
    let mut forced = store.sources.clone();
    for entry in forced
        .iter_mut()
        .filter(|entry| entry.kind == strategy::KIND)
    {
        entry.auto_update = true;
        entry.last_attempt_at = Some(0);
    }
    assert!(due_sources(&forced, now).iter().all(|due| due != &id));
    assert_eq!(
        set_auto_update(id.clone(), true).await.err().unwrap(),
        "CATALOG_INVALID"
    );
    assert!(
        !stored()
            .sources
            .iter()
            .find(|s| s.id == id)
            .unwrap()
            .auto_update
    );
}

/// 订阅用量头：只认已知键与十进制数字，缺失或非法字段一律降级为“没有数据”。
#[test]
fn subscription_usage_header_parses_known_fields_and_ignores_the_rest() {
    let headers = |value: &str| {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("subscription-userinfo", value.parse().unwrap());
        headers
    };
    let full = subscription_usage(&headers(
        "upload=1024; download=2048; total=4096; expire=1759999999; plan=pro",
    ))
    .expect("full header");
    assert_eq!(full.upload, 1024);
    assert_eq!(full.download, 2048);
    assert_eq!(full.total, 4096);
    assert_eq!(full.expire_at, Some(1759999999000));
    assert!(full.at > 0);

    // 缺 total / expire 时仍保留可用字段，不把缺失项编造成 0 / 到期。
    let partial = subscription_usage(&headers("download=10")).expect("partial header");
    assert_eq!(
        (
            partial.upload,
            partial.download,
            partial.total,
            partial.expire_at
        ),
        (0, 10, 0, None)
    );
    // expire=0 表示不限时，不当成有效到期时间。
    assert_eq!(
        subscription_usage(&headers("total=1; expire=0"))
            .expect("expire zero")
            .expire_at,
        None
    );
    // 非法数值按缺失处理；全部不可用时返回 None，界面不会显示假数据。
    let invalid = subscription_usage(&headers("upload=abc; download=-1; total=; expire=x"));
    assert!(invalid.is_none());
    assert!(subscription_usage(&headers("upload=99999999999999999999")).is_none());
    assert!(subscription_usage(&reqwest::header::HeaderMap::new()).is_none());
}

/// 只有订阅来源会把用量带给界面；策略与手动来源即使存了数字也不返回；
/// 旧文件没有 usage 字段时按“没有用量信息”处理，不会把缺失当成 0。
#[test]
fn usage_is_exposed_only_for_subscription_sources() {
    let usage = SourceUsage {
        upload: 1,
        download: 2,
        total: 4,
        expire_at: Some(1759999999000),
        at: 7,
    };
    let mut subscription = fixture();
    subscription.usage = Some(usage.clone());
    let mut legacy = serde_json::to_value(&subscription).unwrap();
    legacy.as_object_mut().unwrap().remove("usage");
    let loaded: Source = serde_json::from_value(legacy).unwrap();
    assert!(loaded.usage.is_none());

    let mut manual = fixture();
    manual.id = "manual-id".into();
    manual.kind = "manual".into();
    manual.url = None;
    manual.usage = Some(usage.clone());
    let mut strategy_source = fixture();
    strategy_source.id = "strategy-id".into();
    strategy_source.kind = strategy::KIND.into();
    strategy_source.url = None;
    strategy_source.usage = Some(usage);
    let store = Store {
        version: 1,
        sources: vec![subscription, manual, strategy_source, loaded],
    };
    let public = serde_json::to_string(&view(&store)).unwrap();
    let view = view(&store);
    let usage = view
        .sources
        .iter()
        .find(|s| s.id == "source-id")
        .and_then(|s| s.usage.clone())
        .expect("subscription usage");
    assert_eq!(
        (usage.upload, usage.download, usage.total, usage.at),
        (1, 2, 4, 7)
    );
    assert_eq!(usage.expire_at, Some(1759999999000));
    for id in ["manual-id", "strategy-id"] {
        assert!(view
            .sources
            .iter()
            .find(|s| s.id == id)
            .unwrap()
            .usage
            .is_none());
    }
    // 序列化后的公开视图只带脱敏数字字段，不包含响应头原文。
    assert!(public.contains("\"usage\":{"), "{public}");
    assert!(!public.contains("subscription-userinfo"), "{public}");
}
