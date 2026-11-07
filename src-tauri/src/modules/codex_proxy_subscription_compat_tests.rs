use super::*;
use crate::modules::codex_proxy_catalog_binding as binding;
use std::collections::BTreeMap;

fn node(name: &str) -> Value {
    json!({"name":name,"type":"http","server":"127.0.0.1","port":8080})
}

#[test]
fn chain_roundtrip_keeps_dependencies_and_native_connection_options() {
    let mut exit = node("Exit");
    exit["dialer-proxy"] = json!("Relay");
    exit["interface-name"] = json!("en0");
    exit["routing-mark"] = json!(123);
    let catalog = parse(&json!({"proxies":[exit, node("Relay")]}).to_string()).unwrap();
    assert!(catalog.nodes.iter().all(|n| n.error.is_none()));
    let raw = binding::encode("s", "s", &catalog.nodes[0].id, &catalog, &BTreeMap::new()).unwrap();
    let (nodes, groups) = binding::runtime_parts(json!(binding::outbounds(&raw).unwrap())).unwrap();
    assert!(groups.is_empty());
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0]["name"], "account-node");
    assert_eq!(nodes[0]["dialer-proxy"], nodes[1]["name"]);
    assert_eq!(nodes[0]["interface-name"], "en0");
    assert_eq!(nodes[0]["routing-mark"], 123);
}

#[test]
fn direct_dialer_is_transport_for_the_selected_node_and_never_a_direct_exit() {
    let mut exit = node("Exit");
    exit["dialer-proxy"] = json!("DIRECT");
    let catalog = parse(&json!({"proxies":[exit]}).to_string()).unwrap();
    let raw = binding::encode("s", "s", &catalog.nodes[0].id, &catalog, &BTreeMap::new()).unwrap();
    let (nodes, _) = binding::runtime_parts(json!(binding::outbounds(&raw).unwrap())).unwrap();
    assert_eq!(nodes[0]["dialer-proxy"], "DIRECT");
    assert_eq!(nodes[0]["type"], "http");
}

#[test]
fn chained_manual_group_requires_and_preserves_the_actual_selection() {
    let mut exit = node("Exit");
    exit["dialer-proxy"] = json!("Choose");
    let catalog = parse(
        &json!({"proxies":[exit,node("Relay")],"proxy-groups":[
            {"name":"Choose","type":"select","proxies":["Missing","Relay","Exit"]}
        ]})
        .to_string(),
    )
    .unwrap();
    assert!(catalog.nodes[0].error.is_none());
    assert!(binding::encode("s", "s", &catalog.nodes[0].id, &catalog, &BTreeMap::new()).is_err());
    let choices = BTreeMap::from([(catalog.groups[0].id.clone(), "Relay".into())]);
    let raw = binding::encode("s", "s", &catalog.nodes[0].id, &catalog, &choices).unwrap();
    let (nodes, groups) = binding::runtime_parts(json!(binding::outbounds(&raw).unwrap())).unwrap();
    assert_eq!(nodes[0]["dialer-proxy"], groups[0]["name"]);
    assert_eq!(groups[0]["proxies"][0], nodes[1]["name"]);
    let cycle = BTreeMap::from([(catalog.groups[0].id.clone(), "Exit".into())]);
    assert!(binding::encode("s", "s", &catalog.nodes[0].id, &catalog, &cycle).is_err());
}

#[test]
fn missing_and_cyclic_chains_are_diagnosed_without_losing_definitions() {
    for (dependency, error) in [
        ("Missing", "SUBSCRIPTION_GROUP_MEMBER_MISSING"),
        ("Exit", "SUBSCRIPTION_GROUP_CYCLE"),
    ] {
        let mut exit = node("Exit");
        exit["dialer-proxy"] = json!(dependency);
        let mut catalog = parse(&json!({"proxies":[exit],"proxy-groups":[{"name":"Auto","type":"url-test","proxies":["Exit"]}]}).to_string()).unwrap();
        assert_eq!(catalog.nodes[0].error.as_deref(), Some(error));
        assert!(catalog.nodes[0].outbound.is_some());
        assert!(catalog.groups[0].error.is_some());
        catalog.nodes[0].outbound.as_mut().unwrap()["proxy"]
            .as_object_mut()
            .unwrap()
            .remove("dialer-proxy");
        revalidate_retained_nodes(&mut catalog);
        validate(&mut catalog).unwrap();
        assert!(catalog.nodes[0].error.is_none());
        assert!(catalog.groups[0].error.is_none());
    }
    for dialer in ["absent", "account-node"] {
        assert!(binding::runtime_parts(json!({"type":"mihomo","proxy":{
            "type":"http","server":"127.0.0.1","port":8080,"dialer-proxy":dialer
        }}))
        .is_err());
    }
}

#[test]
fn strategy_copy_keeps_complete_chains_from_each_source() {
    use crate::modules::codex_proxy_catalog::strategy;
    let mut exit = node("Exit");
    exit["dialer-proxy"] = json!("Relay");
    let catalog = parse(&json!({"proxies":[exit,node("Relay")]}).to_string()).unwrap();
    let node = &catalog.nodes[0];
    let copied = strategy::build(
        "Strategy",
        "fallback",
        &Default::default(),
        "group".into(),
        vec![strategy::copy_node(node, &catalog, true).unwrap()],
    )
    .unwrap();
    let raw = binding::encode("s", "s", &copied.groups[0].id, &copied, &BTreeMap::new()).unwrap();
    let (nodes, _) = binding::runtime_parts(json!(binding::outbounds(&raw).unwrap())).unwrap();
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0]["dialer-proxy"], nodes[1]["name"]);
    let recovered = strategy::copy_node(&copied.nodes[0], &copied, false).unwrap();
    assert_eq!(recovered.outbound, copied.nodes[0].outbound.clone().unwrap());
    assert_eq!(recovered.dependencies.unwrap().nodes[0].name, copied.nodes[1].name);
}
