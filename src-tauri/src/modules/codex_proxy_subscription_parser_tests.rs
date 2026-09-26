use super::*;
const ID: &str = "11111111-2222-3333-4444-555555555555";
fn fixture(extra: &str) -> String {
    format!("proxies:\n  - name: Alpha\n    type: vless\n    server: example.com\n    port: 443\n    uuid: {ID}\n    tls: true\n{extra}")
}
#[test]
fn yaml_nodes_groups_and_stable_ids() {
    let input=fixture("proxy-groups:\n  - {name: Auto, type: url-test, proxies: [Alpha], url: https://example.com/check, interval: 300, tolerance: 50}\n  - {name: Pick, type: select, proxies: [Auto, Alpha]}");
    let c = parse(&input).unwrap();
    assert_eq!(c.nodes[0].outbound.as_ref().unwrap()["proxy"]["tls"], true);
    assert!(c.groups.iter().all(|g| g.error.is_none()));
    assert_eq!(c.groups[0].interval, Some(300));
    let changed = parse(&input.replace("example.com", "other.example")).unwrap();
    assert_eq!(c.nodes[0].id, changed.nodes[0].id);
}

#[test]
fn nested_auto_groups_keep_blocking_members_through_binding_and_runtime() {
    use super::super::codex_proxy_catalog_binding as binding;
    use std::collections::BTreeMap;
    let catalog = parse(&fixture("proxy-groups:\n  - {name: Auto, type: url-test, proxies: [REJECT, Alpha, REJECT-DROP], url: https://example.com/check, interval: 300, tolerance: 150}\n  - {name: Region, type: select, proxies: [Auto, Alpha, DIRECT]}\n")).unwrap();
    assert!(catalog.groups.iter().all(|g| g.error.is_none()));
    let auto = &catalog.groups[0];
    let region = &catalog.groups[1];
    // Both a selected member of a manual group and a direct child-group choice
    // execute URLTest, rather than freezing whichever concrete node is fastest.
    for (item, selections) in [
        (
            &region.id,
            BTreeMap::from([(region.id.clone(), auto.name.clone())]),
        ),
        (&auto.id, BTreeMap::new()),
    ] {
        let raw = binding::encode_with_group(
            "source",
            "Source",
            item,
            &catalog,
            &selections,
            Some(&region.id),
        )
        .unwrap();
        let summary = binding::summary(&raw).unwrap();
        assert_eq!(summary["groupId"], region.id);
        assert_eq!(summary["itemId"], *item);
        let (proxies, groups) =
            binding::runtime_parts(json!(binding::outbounds(&raw).unwrap())).unwrap();
        assert_eq!(proxies.len(), 1);
        let runtime_auto = groups.iter().find(|g| g["type"] == "url-test").unwrap();
        assert_eq!(
            runtime_auto["proxies"],
            json!(["REJECT", proxies[0]["name"], "REJECT-DROP"])
        );
        assert_eq!(runtime_auto["url"], "https://example.com/check");
        assert_eq!(runtime_auto["interval"], 300);
        assert_eq!(runtime_auto["tolerance"], 150);
        assert!(groups.iter().all(|g| g["empty-fallback"] == "REJECT"));
    }
}

#[test]
fn group_blocking_leaves_do_not_allow_bypass_or_standalone_builtin_bindings() {
    use super::super::codex_proxy_catalog_binding as binding;
    use std::collections::BTreeMap;
    for member in [
        "REJECT",
        "REJECT-DROP",
        "DIRECT",
        "PASS",
        "PASS-RULE",
        "COMPATIBLE",
        "Missing",
    ] {
        let catalog = parse(&fixture(&format!("proxy-groups:\n  - {{name: Auto, type: url-test, proxies: [Alpha, {member}]}}\n  - {{name: Manual, type: select, proxies: [Alpha, {member}]}}\n"))).unwrap();
        let blocked = binding::is_blocking_builtin(member);
        assert_eq!(catalog.groups[0].error.is_none(), blocked, "{member}");
        assert_eq!(
            binding::encode("s", "s", &catalog.groups[0].id, &catalog, &BTreeMap::new()).is_ok(),
            blocked
        );
        let selected = BTreeMap::from([(catalog.groups[1].id.clone(), member.into())]);
        let manual = binding::encode("s", "s", &catalog.groups[1].id, &catalog, &selected);
        assert_eq!(manual.is_ok(), blocked, "{member}");
        if let Ok(raw) = manual {
            assert_eq!(binding::summary(&raw).unwrap()["selectedName"], member);
            let (proxies, groups) =
                binding::runtime_parts(json!(binding::outbounds(&raw).unwrap())).unwrap();
            assert!(proxies.is_empty());
            assert_eq!(groups[0]["proxies"], json!([member]));
        }
        assert!(binding::encode("s", "s", member, &catalog, &BTreeMap::new()).is_err());
    }
}

#[test]
fn revalidation_recomputes_availability_but_preserves_incompatible_nodes_and_options() {
    let mut catalog = parse(&fixture("proxy-groups:\n  - {name: Auto, type: url-test, proxies: [REJECT, Alpha]}\n  - {name: Invalid, type: url-test, proxies: [Alpha], unsupported: true}\n")).unwrap();
    catalog.groups[0].error = Some("SUBSCRIPTION_GROUP_UNAVAILABLE".into());
    validate(&mut catalog).unwrap();
    assert!(catalog.groups[0].error.is_none());
    assert_eq!(
        catalog.groups[1].error.as_deref(),
        Some("SUBSCRIPTION_GROUP_OPTIONS")
    );
    catalog.nodes[0].error = Some("PROXY_UNSUPPORTED_OPTION".into());
    catalog.nodes[0].outbound = None;
    validate(&mut catalog).unwrap();
    assert_eq!(
        catalog.groups[0].error.as_deref(),
        Some("SUBSCRIPTION_GROUP_MEMBER_UNSUPPORTED")
    );
    assert_eq!(catalog.groups[0].members, vec!["REJECT", "Alpha"]);
    for reserved in [
        "DIRECT",
        "REJECT",
        "REJECT-DROP",
        "PASS",
        "PASS-RULE",
        "COMPATIBLE",
    ] {
        let catalog =
            parse(&fixture("").replace("name: Alpha", &format!("name: {reserved}"))).unwrap();
        assert!(catalog.nodes[0].error.is_some());
        assert!(super::super::codex_proxy_catalog_binding::encode(
            "s",
            "s",
            &catalog.nodes[0].id,
            &catalog,
            &Default::default()
        )
        .is_err());
    }
}
#[test]
fn plain_and_base64_links_preserve_labels() {
    let input=format!("vless://{ID}@example.com:443?security=tls#Tokyo%20One\ntrojan://secret@example.org:443#London");
    for input in [&input, &general_purpose::STANDARD.encode(&input)] {
        let c = parse(input).unwrap();
        assert_eq!(c.nodes.len(), 2);
        assert_eq!(c.nodes[0].name, "Tokyo One");
        assert!(c.nodes[1].outbound.is_some());
    }
}
#[test]
fn unsupported_nodes_and_groups_remain_visible_but_cannot_bind() {
    let input=fixture("    skip-cert-verify: true\nproxy-groups:\n  - {name: Pick, type: select, proxies: [Alpha]}\n  - {name: Failover, type: fallback, proxies: [Alpha]}");
    let c = parse(&input).unwrap();
    assert!(c.nodes[0]
        .outbound
        .as_ref()
        .is_some_and(|o| o["proxy"]["skip-cert-verify"] == true));
    assert_eq!(c.nodes[0].error.as_deref(), Some("PROXY_TLS_INSECURE"));
    assert!(super::super::codex_proxy_catalog_binding::encode(
        "source",
        "source",
        &c.nodes[0].id,
        &c,
        &std::collections::BTreeMap::new()
    )
    .is_err());
    assert!(c.groups.iter().all(|g| g.error.is_some()));
    assert_eq!(c.groups[1].members, vec!["Alpha"]);
}
#[test]
fn unknown_security_and_transport_fields_are_not_dropped() {
    for extra in [
        "    dialer-proxy: Evil\n",
        "    certificate: /private/certificate.pem\n",
        "    unknown-option: secret\n",
    ] {
        assert!(parse(&fixture(extra)).unwrap().nodes[0].outbound.is_none());
    }
}
#[test]
fn cycles_missing_refs_and_remote_providers_disable_groups() {
    for groups in [
        "  - {name: A, type: select, proxies: [B]}\n  - {name: B, type: select, proxies: [A]}",
        "  - {name: A, type: select, proxies: [MISSING]}",
        "  - {name: A, type: select, proxies: [Alpha], use: [remote]}",
    ] {
        let c = parse(&fixture(&format!("proxy-groups:\n{groups}"))).unwrap();
        assert!(c.groups.iter().all(|g| g.error.is_some()));
    }
}

#[test]
fn group_diagnostics_distinguish_missing_members_cycles_and_unsupported_options() {
    let catalog = parse(&fixture("proxy-groups:\n  - {name: A, type: url-test, proxies: [B]}\n  - {name: B, type: url-test, proxies: [A]}\n  - {name: Missing, type: url-test, proxies: [Alpha, Removed]}\n  - {name: Options, type: fallback, proxies: [Alpha], unsupported: true}\n  - {name: Outer, type: url-test, proxies: [Options]}\n")).unwrap();
    assert_eq!(
        catalog.groups[0].error.as_deref(),
        Some("SUBSCRIPTION_GROUP_CYCLE")
    );
    assert_eq!(
        catalog.groups[1].error.as_deref(),
        Some("SUBSCRIPTION_GROUP_CYCLE")
    );
    assert_eq!(
        catalog.groups[2].error.as_deref(),
        Some("SUBSCRIPTION_GROUP_MEMBER_MISSING")
    );
    assert_eq!(
        catalog.groups[4].error.as_deref(),
        Some("SUBSCRIPTION_GROUP_MEMBER_UNSUPPORTED")
    );
    let issues = group_issues(&catalog);
    assert_eq!(issues[2][0].name, "Removed");
    assert_eq!(issues[2][0].error, "SUBSCRIPTION_GROUP_MEMBER_MISSING");
    assert_eq!(issues[4][0].name, "Options");
    assert_eq!(issues[4][0].error, "SUBSCRIPTION_GROUP_OPTIONS");
}

#[test]
fn group_diagnostics_shared_descendants_are_not_misreported_as_cycles() {
    let catalog = parse(&fixture("proxy-groups:\n  - {name: Leaf, type: url-test, proxies: [Alpha]}\n  - {name: Left, type: url-test, proxies: [Leaf]}\n  - {name: Right, type: url-test, proxies: [Leaf]}\n  - {name: All, type: url-test, proxies: [Left, Right, Alpha]}\n")).unwrap();
    assert!(catalog.groups.iter().all(|group| group.error.is_none()));
    assert!(group_issues(&catalog).iter().all(Vec::is_empty));
    let leaves = reachable_nodes(&catalog, &catalog.groups[3].id).unwrap();
    assert_eq!(leaves.len(), 1);
    assert_eq!(leaves[0].name, "Alpha");
}
#[test]
fn ignores_top_level_executable_and_network_configuration() {
    let c=parse(&fixture("script: {code: evil}\nproxy-providers: {evil: {url: 'file:///private'}}\nrules: [MATCH,DIRECT]\nexternal-controller: '0.0.0.0:1'\n")).unwrap();
    assert_eq!(c.nodes.len(), 1);
    let out = c.nodes[0].outbound.as_ref().unwrap().to_string();
    assert!(!out.contains("evil"));
    assert!(!out.contains("controller"));
}
#[test]
fn duplicate_names_keys_and_oversized_inputs_fail_safely() {
    assert!(parse(&fixture("    password: FIRST\n    password: SECOND\n")).is_err());
    assert!(parse(&fixture(
        "proxy-groups:\n  - {name: Alpha, type: select, proxies: [Alpha]}"
    ))
    .is_err());
    assert!(parse(&"a".repeat(MAX_BYTES + 1)).is_err());
    assert!(parse("<html>secret response</html>").is_err());
    assert!(parse("proxies:\n  - {name: a, type: !!SECRET evil}\n").is_err());
}
#[test]
fn direct_proxies_decode_credentials_and_support_ipv6_tls() {
    let c = parse("https://u:p%40ss@[::1]:443#Proxy").unwrap();
    let o = c.nodes[0].outbound.as_ref().unwrap();
    assert_eq!(o["password"], "p@ss");
    assert_eq!(o["server"], "::1");
    assert_eq!(o["tls"]["enabled"], true);
    let c=parse("proxies:\n  - {name: SOCKS, type: socks5, server: localhost, port: 1080, username: a, password: b}").unwrap();
    assert_eq!(
        c.nodes[0].outbound.as_ref().unwrap()["proxy"]["type"],
        "socks5"
    );
}
#[test]
fn vmess_ws_and_grpc_map_only_known_fields() {
    let input=format!("proxies:\n  - {{name: VM, type: vmess, server: example.org, port: 443, uuid: {ID}, alterId: 0, cipher: auto, tls: true, network: ws, ws-opts: {{path: /ws, headers: {{Host: example.org}}}}}}\n  - {{name: GRPC, type: vless, server: example.org, port: 443, uuid: {ID}, tls: true, network: grpc, grpc-opts: {{grpc-service-name: proxy}}}}");
    let c = parse(&input).unwrap();
    assert_eq!(
        c.nodes[0].outbound.as_ref().unwrap()["proxy"]["ws-opts"]["path"],
        "/ws"
    );
    assert_eq!(
        c.nodes[1].outbound.as_ref().unwrap()["proxy"]["grpc-opts"]["grpc-service-name"],
        "proxy"
    );
}
#[test]
fn errors_and_generated_labels_do_not_contain_supplied_credentials() {
    let c = parse(
        "unknown://secret@example.com:443\ntrojan://password@example.com:443#https%3A%2F%2Fsecret",
    )
    .unwrap();
    assert_eq!(c.nodes[0].name, "#1");
    assert_eq!(c.nodes[1].name, "#2");
    assert_eq!(c.nodes[0].error.as_deref(), Some(UNSUPPORTED));
}
#[test]
fn deeply_nested_yaml_and_alias_cycles_fail_without_panicking() {
    assert!(parse(&format!("proxies: {}0{}", "[".repeat(150), "]".repeat(150))).is_err());
    assert!(parse("proxies: &a [*a]").is_err());
}

#[test]
fn include_all_local_nodes_is_supported_without_fetching_providers() {
    let c = parse(&fixture(
        "proxy-groups:\n  - {name: All, type: select, include-all-proxies: true}",
    ))
    .unwrap();
    assert_eq!(c.groups[0].members, vec!["Alpha"]);
    assert!(c.groups[0].error.is_none());
    let c=parse(&fixture("proxy-providers: {remote: {url: 'https://example.com/token'}}\nproxy-groups:\n  - {name: All, type: select, include-all: true}")).unwrap();
    assert!(c.groups[0].error.is_some());
}

#[test]
fn all_supported_yaml_protocols_map_and_reject_unknown_options() {
    let pk = general_purpose::URL_SAFE_NO_PAD.encode([1u8; 32]);
    let input=format!("proxies:\n  - {{name: SS, type: ss, server: example.com, port: 443, cipher: aes-128-gcm, password: secret}}\n  - {{name: Trojan, type: trojan, server: example.com, port: 443, password: secret, sni: example.com}}\n  - {{name: Reality, type: vless, server: example.com, port: 443, uuid: {ID}, tls: true, client-fingerprint: chrome, reality-opts: {{public-key: {pk}, short-id: abcdef01}}}}\n  - {{name: HY, type: hysteria2, server: example.com, port: 443, password: secret, obfs: salamander, obfs-password: secret}}\n  - {{name: TUIC, type: tuic, server: example.com, port: 443, uuid: {ID}, password: secret, congestion-controller: bbr, alpn: [h3]}}");
    let c = parse(&input).unwrap();
    assert!(c.nodes.iter().all(|n| n.outbound.is_some()));
    assert_eq!(
        c.nodes[2].outbound.as_ref().unwrap()["proxy"]["reality-opts"]["public-key"],
        pk
    );
    for n in &c.nodes {
        super::super::codex_proxy_catalog_binding::validate_outbound(n.outbound.as_ref().unwrap())
            .unwrap();
    }
}

#[test]
fn hysteria2_bandwidth_and_duplicate_port_hopping_fields_are_preserved() {
    let input = "proxies:\n  - {name: HY, type: hysteria2, server: example.com, port: 443, password: secret, up: 50, down: 100, ports: '20000-20002', mport: '20000-20002', hop-interval: 30}";
    let catalog = parse(input).unwrap();
    let node = &catalog.nodes[0];
    assert!(node.error.is_none());
    let outbound = node.outbound.as_ref().unwrap();
    assert_eq!(outbound["proxy"]["ports"], "20000-20002");
    assert_eq!(outbound["proxy"]["port"], 443);
    assert_eq!(outbound["proxy"]["up"], 50);
    assert_eq!(outbound["proxy"]["down"], 100);
    assert_eq!(outbound["proxy"]["hop-interval"], 30);
    super::super::codex_proxy_catalog_binding::validate_outbound(outbound).unwrap();
    let insecure = input.replace(
        "hop-interval: 30}",
        "hop-interval: 30, skip-cert-verify: true}",
    );
    let insecure_node = parse(&insecure).unwrap().nodes.remove(0);
    assert_eq!(insecure_node.error.as_deref(), Some("PROXY_TLS_INSECURE"));
    assert_eq!(
        insecure_node.outbound.unwrap()["proxy"]["ports"],
        "20000-20002"
    );

    let different = input.replace("mport: '20000-20002'", "mport: '30000-30002'");
    assert_eq!(
        parse(&different).unwrap().nodes[0].error.as_deref(),
        Some(UNSUPPORTED)
    );
    let invalid = input.replace(
        "ports: '20000-20002', mport: '20000-20002'",
        "ports: '70000-80000'",
    );
    assert_eq!(
        parse(&invalid).unwrap().nodes[0].error.as_deref(),
        Some(UNSUPPORTED)
    );
}

#[test]
fn hysteria2_udp_subscription_flag_keeps_security_checks_and_requires_native_semantics() {
    use super::super::codex_proxy_catalog_binding as binding;
    let input = "proxies:\n  - {name: HY, type: hysteria2, server: localhost, port: 443, password: secret, sni: example.test, up: 50, down: 100, ports: '20000-20002', mport: '20000-20002', udp: true, skip-cert-verify: true}\nproxy-groups:\n  - {name: Auto, type: url-test, proxies: [REJECT, HY]}\n";
    let mut catalog = parse(input).unwrap();
    assert_eq!(
        catalog.nodes[0].error.as_deref(),
        Some("PROXY_TLS_INSECURE")
    );
    assert!(catalog.nodes[0].outbound.is_some());
    assert!(catalog.groups[0].error.is_some());
    assert!(binding::encode(
        "s",
        "s",
        &catalog.groups[0].id,
        &catalog,
        &Default::default()
    )
    .is_err());
    // This is the explicit per-node certificate permission path, not an import default.
    catalog.nodes[0].error = None;
    validate(&mut catalog).unwrap();
    let raw = binding::encode(
        "s",
        "s",
        &catalog.groups[0].id,
        &catalog,
        &Default::default(),
    )
    .unwrap();
    let (proxies, groups) =
        binding::runtime_parts(json!(binding::outbounds(&raw).unwrap())).unwrap();
    assert_eq!(proxies[0]["udp"], true);
    assert_eq!(proxies[0]["skip-cert-verify"], true);
    assert_eq!(proxies[0]["up"], 50);
    assert_eq!(proxies[0]["down"], 100);
    assert_eq!(proxies[0]["ports"], "20000-20002");
    assert!(proxies[0].get("mport").is_none());
    assert_eq!(groups[0]["proxies"][0], "REJECT");
    for unsupported in ["false", "'true'", "1", "null"] {
        let catalog = parse(&input.replace("udp: true", &format!("udp: {unsupported}"))).unwrap();
        assert_eq!(
            catalog.nodes[0].error.as_deref(),
            Some("PROXY_UNSUPPORTED_OPTION")
        );
        assert!(catalog.nodes[0].outbound.is_none());
        assert!(catalog.groups[0].error.is_some());
    }
}

#[test]
fn quoted_password_tags_are_data_and_explicit_yaml_tags_are_rejected() {
    let c=parse("proxies:\n  - {name: T, type: trojan, server: example.com, port: 443, password: '!!secret'}").unwrap();
    assert_eq!(
        c.nodes[0].outbound.as_ref().unwrap()["proxy"]["password"],
        "!!secret"
    );
    assert!(parse("proxies:\n  - {name: T, type: trojan, server: example.com, port: 443, password: !!str secret}").is_err());
    assert!(parse("proxies:\n  - {name: T, type: trojan, server: example.com, port: 443, password: !tag secret}").is_err());
}

#[test]
fn eager_health_checks_remain_native() {
    let c = parse(&fixture(
        "proxy-groups:\n  - {name: Auto, type: url-test, proxies: [Alpha], lazy: false}",
    ))
    .unwrap();
    assert!(c.groups[0].error.is_none());
    assert_eq!(c.groups[0].native.as_ref().unwrap()["lazy"], false);
    assert_eq!(c.groups[0].members, vec!["Alpha"]);
    let c = parse(&fixture(
        "proxy-groups:\n  - {name: Auto, type: url-test, proxies: [Alpha], lazy: true}",
    ))
    .unwrap();
    assert!(c.groups[0].error.is_none());
}

#[test]
fn untrusted_protocol_and_group_kind_never_escape_as_display_metadata() {
    let input = "proxies:\n  - {name: Proxy, type: 'https://user:SECRET@example.com/token', server: example.com, port: 443}\nproxy-groups:\n  - {name: Group, type: 'password=SECRET', proxies: [Proxy]}";
    let c = parse(input).unwrap();
    assert_eq!(c.nodes[0].protocol, "unsupported");
    assert_eq!(c.groups[0].kind, "unsupported");
    assert!(c.nodes[0].outbound.is_none());
    assert!(c.nodes[0].error.is_some());
    assert!(c.groups[0].error.is_some());
    // Unlike a supported catalog this fixture has no outbound secrets; its
    // serialised representation proves raw kind/type values were discarded.
    assert!(!serde_json::to_string(&c).unwrap().contains("SECRET"));
}

#[test]
fn selectors_keep_usable_branches_while_urltest_preserves_all_members() {
    let c = parse(&fixture("proxy-groups:\n  - {name: Pick, type: select, proxies: [Failover, DIRECT, Alpha], timeout: 12000, interrupt-exist-connections: false}\n  - {name: Failover, type: fallback, proxies: [Alpha]}\n  - {name: Auto, type: url-test, proxies: [Alpha, DIRECT]}\n")).unwrap();
    assert!(c.groups[0].error.is_none());
    assert_eq!(c.groups[0].members.len(), 3);
    assert!(c.groups[1].error.is_none());
    assert_eq!(
        c.groups[2].error.as_deref(),
        Some("SUBSCRIPTION_GROUP_MEMBER_UNSUPPORTED")
    );
    let selection = std::collections::BTreeMap::from([(c.groups[0].id.clone(), "Alpha".into())]);
    let encoded = super::super::codex_proxy_catalog_binding::encode(
        "source",
        "Example",
        &c.groups[0].id,
        &c,
        &selection,
    )
    .unwrap();
    assert_eq!(
        super::super::codex_proxy_catalog_binding::outbounds(&encoded)
            .unwrap()
            .len(),
        2
    );
    for member in ["DIRECT"] {
        let selection = std::collections::BTreeMap::from([(c.groups[0].id.clone(), member.into())]);
        assert!(super::super::codex_proxy_catalog_binding::encode(
            "source",
            "Example",
            &c.groups[0].id,
            &c,
            &selection
        )
        .is_err());
    }
}

#[test]
fn nested_selectors_with_exits_do_not_depend_on_traversal_order() {
    let c = parse(&fixture("proxy-groups:\n  - {name: A, type: select, proxies: [B, Alpha]}\n  - {name: B, type: select, proxies: [A]}\n  - {name: C, type: select, proxies: [C]}\n")).unwrap();
    assert!(c.groups[0].error.is_none());
    assert!(c.groups[1].error.is_none());
    assert!(c.groups[2].error.is_some());
    let selection = std::collections::BTreeMap::from([
        (c.groups[0].id.clone(), "B".into()),
        (c.groups[1].id.clone(), "A".into()),
    ]);
    assert!(super::super::codex_proxy_catalog_binding::encode(
        "source",
        "Example",
        &c.groups[0].id,
        &c,
        &selection
    )
    .is_err());
}

#[test]
fn enabled_ech_dns_override_maps_without_removing_handshake_security() {
    let c = parse(&fixture("    network: ws\n    ws-opts: {path: /ws, headers: {Host: example.org}}\n    ech-opts: {enable: true, query-server-name: ech.example.org}\n")).unwrap();
    let out = c.nodes[0].outbound.as_ref().unwrap();
    assert_eq!(
        out["proxy"]["ech-opts"],
        json!({"enable":true,"query-server-name":"ech.example.org"})
    );
    super::super::codex_proxy_catalog_binding::validate_outbound(out).unwrap();
    for extra in [
        "    ech-opts: {enable: true, query-server-name: 'https://secret.example/token'}\n",
        "    ech-opts: {enable: true, config_path: /private/key}\n",
    ] {
        assert!(parse(&fixture(extra)).unwrap().nodes[0].outbound.is_none());
    }
    let mut injected = out.clone();
    injected["proxy"]["ech-opts"]["config_path"] = json!("/private/key");
    assert!(super::super::codex_proxy_catalog_binding::validate_outbound(&injected).is_err());
}

#[test]
fn compatibility_failures_have_safe_specific_codes() {
    for (extra, code) in [
        ("    skip-cert-verify: true\n", "PROXY_TLS_INSECURE"),
        ("    dialer-proxy: DIRECT\n", UNSUPPORTED),
        ("    private-key: /private/SECRET\n", UNSUPPORTED),
    ] {
        let c = parse(&fixture(extra)).unwrap();
        assert_eq!(c.nodes[0].error.as_deref(), Some(code));
        if code == "PROXY_TLS_INSECURE" {
            assert_eq!(
                c.nodes[0].outbound.as_ref().unwrap()["proxy"]["skip-cert-verify"],
                true
            );
            assert!(super::super::codex_proxy_catalog_binding::encode(
                "source",
                "source",
                &c.nodes[0].id,
                &c,
                &std::collections::BTreeMap::new()
            )
            .is_err());
        } else {
            assert!(c.nodes[0].outbound.is_none());
        }
    }
    let c = parse(&fixture(
        "proxy-groups:\n  - {name: Auto, type: url-test, proxies: [Alpha], timeout: 12000}\n",
    ))
    .unwrap();
    assert!(c.groups[0].error.is_none());
    assert_eq!(c.groups[0].native.as_ref().unwrap()["timeout"], 12000);
}

#[test]
#[ignore = "explicit private local fixture; outputs only counts and fixed reason codes"]
fn private_subscription_fixture_reports_only_counts() {
    let path =
        std::env::var("COCKPIT_TEST_SUBSCRIPTION_FILE").expect("private fixture path required");
    let input = std::fs::read_to_string(path).unwrap_or_else(|_| panic!("fixture unavailable"));
    let c = parse(&input).unwrap_or_else(|_| panic!("fixture parse failed"));
    let mut errors = std::collections::BTreeMap::<String, usize>::new();
    for node in &c.nodes {
        if let Some(error) = &node.error {
            *errors.entry(error.clone()).or_default() += 1;
        }
        if let Some(outbound) = &node.outbound {
            super::super::codex_proxy_catalog_binding::validate_outbound(outbound)
                .unwrap_or_else(|_| panic!("outbound validation failed"));
        }
    }
    println!(
        "nodes={} supported_nodes={} groups={} supported_groups={} reasons={:?}",
        c.nodes.len(),
        c.nodes.iter().filter(|n| n.outbound.is_some()).count(),
        c.groups.len(),
        c.groups.iter().filter(|g| g.error.is_none()).count(),
        errors
    );
    assert!(!c.nodes.is_empty());
}

#[test]
fn native_protocol_parameters_and_group_strategies_survive_to_runtime() {
    let input = fixture("    network: xhttp\n    xhttp-opts: {path: /x, mode: auto, headers: {Authorization: secret}}\n    smux: {enabled: true, protocol: h2mux}\n    ech-opts: {enable: true, config: STATIC_ECH}\nproxy-groups:\n  - {name: Failover, type: fallback, proxies: [Alpha], timeout: 8000, lazy: false}\n  - {name: Balanced, type: load-balance, proxies: [Failover], strategy: round-robin}");
    let catalog = parse(&input).unwrap();
    assert!(catalog.nodes.iter().all(|n| n.error.is_none()));
    assert!(catalog.groups.iter().all(|g| g.error.is_none()));
    let binding = super::super::codex_proxy_catalog_binding::encode(
        "s",
        "s",
        &catalog.groups[1].id,
        &catalog,
        &Default::default(),
    )
    .unwrap();
    let outbounds = super::super::codex_proxy_catalog_binding::outbounds(&binding).unwrap();
    let (proxies, groups) =
        super::super::codex_proxy_catalog_binding::runtime_parts(json!(outbounds)).unwrap();
    assert_eq!(proxies[0]["network"], "xhttp");
    assert_eq!(
        proxies[0]["xhttp-opts"]["headers"]["Authorization"],
        "secret"
    );
    assert_eq!(proxies[0]["ech-opts"]["config"], "STATIC_ECH");
    assert_eq!(proxies[0]["smux"]["enabled"], true);
    assert_eq!(groups[0]["strategy"], "round-robin");
    assert_eq!(groups[1]["timeout"], 8000);
    assert_eq!(groups[1]["lazy"], false);
    assert!(groups.iter().all(|g| g["empty-fallback"] == "REJECT"));
}

#[test]
fn native_catalog_never_enables_local_files_system_routes_or_direct_fallback() {
    for extra in [
        "    private-key: /private/secret\n",
        "    certificate: cert.pem\n",
        "    dialer-proxy: DIRECT\n",
        "    interface-name: eth0\n",
        "    routing-mark: 123\n",
        "    ws-opts: {config_path: /private/secret}\n",
    ] {
        let catalog = parse(&fixture(extra)).unwrap();
        assert!(catalog.nodes[0].outbound.is_none());
    }
    let catalog = parse(&fixture("proxy-groups:\n  - {name: Fall, type: fallback, proxies: [Alpha], empty-fallback: DIRECT}\n  - {name: Load, type: load-balance, proxies: [Alpha, DIRECT]}")).unwrap();
    assert!(catalog.groups.iter().all(|g| g.error.is_some()));
}

#[test]
fn nested_tls_bypass_is_rejected_until_explicitly_approved() {
    for key in ["skip-cert-verify", "skip_cert_verify", "insecure"] {
        let input = fixture(&format!(
            "    network: xhttp\n    xhttp-opts: {{download-settings: {{{key}: true}}}}\n"
        ));
        let mut catalog = parse(&input).unwrap();
        assert_eq!(
            catalog.nodes[0].error.as_deref(),
            Some("PROXY_TLS_INSECURE")
        );
        assert!(super::super::codex_proxy_catalog_binding::encode(
            "s",
            "s",
            &catalog.nodes[0].id,
            &catalog,
            &Default::default()
        )
        .is_err());
        catalog.nodes[0].error = None;
        let binding = super::super::codex_proxy_catalog_binding::encode(
            "s",
            "s",
            &catalog.nodes[0].id,
            &catalog,
            &Default::default(),
        )
        .unwrap();
        assert!(super::super::codex_proxy_catalog_binding::decode(&binding).is_ok());
        for bypass in ["1", "'true'"] {
            let invalid = input.replace(&format!("{key}: true"), &format!("{key}: {bypass}"));
            assert!(parse(&invalid).unwrap().nodes[0].outbound.is_none());
        }
    }
}

#[test]
fn native_ssh_requires_host_identity_and_tls_aliases_preserve_sni() {
    let missing = parse("proxies:\n  - {name: SSH, type: ssh, server: example.org, port: 22, username: user, password: secret}").unwrap();
    assert!(missing.nodes[0].outbound.is_none());
    let pinned = parse("proxies:\n  - {name: SSH, type: ssh, server: example.org, port: 22, username: user, password: secret, host-key: ['ssh-ed25519 AAAA']}").unwrap();
    assert!(pinned.nodes[0].outbound.is_some());
    let catalog = parse(&fixture("    sni: tls.example.org\n")).unwrap();
    assert_eq!(
        catalog.nodes[0].outbound.as_ref().unwrap()["proxy"]["servername"],
        "tls.example.org"
    );
    let conflict = parse(&fixture(
        "    sni: tls.example.org\n    servername: other.example.org\n",
    ))
    .unwrap();
    assert!(conflict.nodes[0].outbound.is_none());
}
