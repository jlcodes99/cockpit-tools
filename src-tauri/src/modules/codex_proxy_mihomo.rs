//! Native Mihomo node boundary and lossless conversion of previously saved sing-box nodes.
//! Native parameter semantics follow the pinned Mihomo decoder; the host enforces
//! local-file, system-routing, certificate approval and resource safety boundaries.
use super::{INVALID, UNSUPPORTED};
use serde_json::{json, Value};

/// Type names remain safe display identifiers; the pinned engine decides whether
/// a normal outbound protocol exists. Control adapters can bypass the selected
/// proxy, while mesh adapters create state files/listeners even without explicit
/// paths, so those stay outside this account-proxy boundary.
pub fn native_protocol(kind: &str) -> bool {
    !kind.is_empty()
        && kind.len() <= 64
        && kind.as_bytes()[0].is_ascii_lowercase()
        && kind.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !matches!(
            kind,
            "direct" | "reject" | "dns" | "rematch"
                | "pass" | "pass-rule" | "compatible"
                | "tailscale" | "zerotier" | "easytier"
        )
}

pub fn is_insecure(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(key, value)| {
            (["skip-cert-verify", "insecure"]
                .contains(&key.to_ascii_lowercase().replace('_', "-").as_str())
                && value == true)
                || is_insecure(value)
        }),
        Value::Array(items) => items.iter().any(is_insecure),
        _ => false,
    }
}

fn bounded_parameters(value: &Value, depth: usize) -> Result<(), String> {
    if depth > 24 {
        return Err(INVALID.into());
    }
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                let key = key.to_ascii_lowercase().replace('_', "-");
                if matches!(
                    key.as_str(),
                    "dialer-proxy"
                        | "interface-name"
                        | "routing-mark"
                        | "detour"
                        | "bind-interface"
                        | "config-path"
                        | "client-cert"
                        | "client-key"
                ) {
                    return Err(UNSUPPORTED.into());
                }
                if ["skip-cert-verify", "insecure"].contains(&key.as_str()) && !value.is_boolean() {
                    return Err(INVALID.into());
                }
                // TLS/SSH certificate material may be inline PEM or local paths.
                // OpenVPN's CA is inline PEM; mesh adapters with file-backed state
                // are blocked by native_protocol. Encoded WireGuard/MASQUE keys
                // are consumed directly by those adapters, never as file names.
                if matches!(key.as_str(), "certificate" | "private-key" | "ca" | "ca-str") {
                    let raw = value.as_str().ok_or(INVALID)?;
                    let encoded_key = key == "private-key"
                        && matches!(map.get("type").and_then(Value::as_str), Some("wireguard" | "masque"));
                    if !raw.is_empty() && !raw.starts_with("-----BEGIN ") && !encoded_key
                    {
                        return Err(UNSUPPORTED.into());
                    }
                }
                bounded_parameters(value, depth + 1)?;
            }
        }
        Value::Array(items) => {
            if items.len() > 4096 {
                return Err(INVALID.into());
            }
            for item in items {
                bounded_parameters(item, depth + 1)?;
            }
        }
        Value::String(s) if s.len() > 65536 || s.contains('\0') => return Err(INVALID.into()),
        _ => {}
    }
    Ok(())
}

pub fn validate_native_proxy(proxy: &Value) -> Result<(), String> {
    proxy.as_object().ok_or(INVALID)?;
    let kind = proxy["type"].as_str().ok_or(INVALID)?;
    if !native_protocol(kind) {
        return Err(UNSUPPORTED.into());
    }
    bounded_parameters(proxy, 0)?;
    // The pinned engine decodes native protocol parameters and ignores unknown
    // subscription metadata. Keeping a second option whitelist here rejects
    // valid configurations as upstream adapters evolve. Preserve all parameters
    // after the recursive host safety boundary above; let Mihomo validate their
    // protocol semantics when its configuration is prepared.
    if proxy
        .get("skip-cert-verify")
        .is_some_and(|v| !v.is_boolean())
    {
        return Err(INVALID.into());
    }
    if kind == "ssh"
        && !proxy["host-key"].as_array().is_some_and(|keys| {
            !keys.is_empty()
                && keys
                    .iter()
                    .all(|key| key.as_str().is_some_and(|v| !v.is_empty()))
        })
    {
        // Upstream defaults to InsecureIgnoreHostKey: require explicit server identity.
        return Err(UNSUPPORTED.into());
    }
    if let Some(query) = proxy
        .get("ech-opts")
        .and_then(|e| e.get("query-server-name"))
    {
        if !query.as_str().is_some_and(super::valid_dns_name) {
            return Err(INVALID.into());
        }
    }
    Ok(())
}

pub fn validate_native_group(group: &Value, frozen: bool) -> Result<(), String> {
    let map = group.as_object().ok_or(INVALID)?;
    if !matches!(
        group["type"].as_str(),
        Some("select" | "url-test" | "fallback" | "load-balance")
    ) {
        return Err(UNSUPPORTED.into());
    }
    let allowed = [
        "name",
        "type",
        "proxies",
        "url",
        "interval",
        "timeout",
        "max-failed-times",
        "lazy",
        "disable-udp",
        "expected-status",
        "hidden",
        "icon",
        "tolerance",
        "strategy",
        "include-all",
        "include-all-proxies",
        "include-all-providers",
        "empty-fallback",
        "interrupt-exist-connections",
        "default-selected",
    ];
    if map.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err(UNSUPPORTED.into());
    }
    if group
        .get("interrupt-exist-connections")
        .is_some_and(|v| v != false)
    {
        return Err(UNSUPPORTED.into());
    }
    if let Some(default) = group.get("default-selected") {
        if !default.as_str().is_some_and(super::short_text)
            || (frozen
                && !group["proxies"]
                    .as_array()
                    .is_some_and(|members| members.contains(default)))
        {
            return Err(INVALID.into());
        }
    }
    if group.get("empty-fallback").is_some_and(|v| v != "REJECT") {
        return Err(UNSUPPORTED.into());
    }
    if frozen
        && [
            "include-all",
            "include-all-proxies",
            "include-all-providers",
        ]
        .iter()
        .any(|k| group.get(k).is_some())
    {
        return Err(UNSUPPORTED.into());
    }
    if let Some(url) = group.get("url") {
        super::test_url(url.as_str().ok_or(INVALID)?)?;
    }
    for key in ["interval", "timeout", "max-failed-times", "tolerance"] {
        if group
            .get(key)
            .is_some_and(|v| !v.as_u64().is_some_and(|v| v <= 120_000))
        {
            return Err(INVALID.into());
        }
    }
    for key in [
        "lazy",
        "hidden",
        "disable-udp",
        "include-all",
        "include-all-proxies",
        "include-all-providers",
    ] {
        if group.get(key).is_some_and(|v| !v.is_boolean()) {
            return Err(INVALID.into());
        }
    }
    if let Some(members) = group.get("proxies") {
        let members = members.as_array().ok_or(INVALID)?;
        if members.len() > 4096
            || members
                .iter()
                .any(|v| !v.as_str().is_some_and(super::short_text))
        {
            return Err(INVALID.into());
        }
    } else if frozen {
        return Err(INVALID.into());
    }
    Ok(())
}

/// Input is a single legacy outbound, or a frozen array containing legacy and native nodes.
/// Root name remains account-node; configuration must route MATCH to it with no direct fallback.
pub fn runtime_parts(input: Value) -> Result<(Vec<Value>, Vec<Value>), String> {
    let items = if let Value::Array(items) = input {
        items
    } else {
        vec![input]
    };
    let mut proxies = Vec::new();
    let mut groups = Vec::new();
    for item in items {
        let tag = item["tag"].as_str().unwrap_or("account-node");
        match item["type"].as_str().ok_or(INVALID)? {
            "mihomo" => {
                super::validate_outbound_capabilities(&item)?;
                let mut proxy = item["proxy"].clone();
                proxy["name"] = json!(tag);
                proxies.push(proxy);
            }
            "mihomo-group" => {
                let mut group = item["group"].clone();
                validate_native_group(&group, true)?;
                group["name"] = json!(tag);
                group["empty-fallback"] = json!("REJECT");
                if group["type"] != "select" {
                    // A saved `lazy: false` no longer disables demand-driven probes.
                    group["lazy"] = json!(true);
                }
                groups.push(group);
            }
            "selector" | "urltest" => {
                let select = item["type"] == "selector";
                let mut members = item["outbounds"].as_array().ok_or(INVALID)?.clone();
                if let Some(selected) = item.get("default") {
                    let index = members.iter().position(|v| v == selected).ok_or(INVALID)?;
                    let selected = members.remove(index);
                    members.insert(0, selected);
                }
                let mut group = json!({"name":tag,"type":if select {"select"} else {"url-test"},"proxies":members,"empty-fallback":"REJECT"});
                if !select {
                    group["url"] = item["url"].clone();
                    group["interval"] = json!(seconds(&item["interval"])?);
                    group["tolerance"] = item["tolerance"].clone();
                    group["lazy"] = json!(true);
                }
                validate_native_group(&group, true)?;
                groups.push(group);
            }
            _ => proxies.push(legacy_proxy(&item, tag)?),
        }
    }
    // Even a caller supplying raw JSON cannot introduce direct fallback, orphaned
    // entries, duplicate identifiers, or a cycle outside the binding decoder.
    let mut graph = std::collections::BTreeMap::<String, Vec<String>>::new();
    for proxy in &proxies {
        let name = proxy["name"].as_str().ok_or(INVALID)?;
        if !super::short_text(name)
            || super::is_builtin_name(name)
            || graph.insert(name.into(), Vec::new()).is_some()
        {
            return Err(INVALID.into());
        }
    }
    for group in &groups {
        let name = group["name"].as_str().ok_or(INVALID)?;
        let members = group["proxies"]
            .as_array()
            .ok_or(INVALID)?
            .iter()
            .map(|v| v.as_str().map(str::to_owned).ok_or(INVALID))
            .collect::<Result<Vec<_>, _>>()?;
        if !super::short_text(name)
            || super::is_builtin_name(name)
            || members.is_empty()
            || graph.insert(name.into(), members).is_some()
        {
            return Err(INVALID.into());
        }
    }
    fn visit(
        name: &str,
        graph: &std::collections::BTreeMap<String, Vec<String>>,
        path: &mut std::collections::BTreeSet<String>,
        visited: &mut std::collections::BTreeSet<String>,
    ) -> Result<(), String> {
        if super::is_blocking_builtin(name) {
            return Ok(());
        }
        if visited.contains(name) {
            return Ok(());
        }
        if path.len() > 16 || !path.insert(name.into()) {
            return Err(INVALID.into());
        }
        for child in graph.get(name).ok_or(INVALID)? {
            visit(child, graph, path, visited)?;
        }
        path.remove(name);
        visited.insert(name.into());
        Ok(())
    }
    let mut visited = std::collections::BTreeSet::new();
    visit(
        "account-node",
        &graph,
        &mut std::collections::BTreeSet::new(),
        &mut visited,
    )?;
    if visited.len() != graph.len() {
        return Err(INVALID.into());
    }
    Ok((proxies, groups))
}
fn seconds(value: &Value) -> Result<u64, String> {
    value
        .as_str()
        .and_then(|s| s.strip_suffix('s'))
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| INVALID.into())
}
fn legacy_proxy(old: &Value, tag: &str) -> Result<Value, String> {
    super::validate_outbound_capabilities(old)?;
    let kind = old["type"].as_str().ok_or(INVALID)?;
    let mut proxy = json!({"name":tag,"type":match kind {"shadowsocks"=>"ss", "socks"=>"socks5", other=>other},"server":old["server"]});
    for (from, to) in [
        ("server_port", "port"),
        ("username", "username"),
        ("password", "password"),
        ("method", "cipher"),
        ("uuid", "uuid"),
        ("security", "cipher"),
        ("alter_id", "alterId"),
        ("flow", "flow"),
        ("packet_encoding", "packet-encoding"),
        ("congestion_control", "congestion-controller"),
        ("udp_relay_mode", "udp-relay-mode"),
        ("up_mbps", "up"),
        ("down_mbps", "down"),
    ] {
        if let Some(value) = old.get(from) {
            proxy[to] = value.clone();
        }
    }
    if let Some(tls) = old.get("tls") {
        if matches!(kind, "http" | "vmess" | "vless") {
            proxy["tls"] = json!(true);
        }
        if let Some(sni) = tls.get("server_name") {
            proxy[if matches!(kind, "vmess" | "vless") {
                "servername"
            } else {
                "sni"
            }] = sni.clone();
        }
        for (from, to) in [("alpn", "alpn"), ("insecure", "skip-cert-verify")] {
            if let Some(value) = tls.get(from) {
                proxy[to] = value.clone();
            }
        }
        if let Some(utls) = tls.get("utls") {
            proxy["client-fingerprint"] = utls["fingerprint"].clone();
        }
        if let Some(reality) = tls.get("reality") {
            proxy["reality-opts"] =
                json!({"public-key":reality["public_key"],"short-id":reality["short_id"]});
        }
        if let Some(ech) = tls.get("ech") {
            let mut opts = json!({"enable":true});
            if let Some(name) = ech.get("query_server_name") {
                opts["query-server-name"] = name.clone();
            }
            proxy["ech-opts"] = opts;
        }
    }
    if let Some(transport) = old.get("transport") {
        match transport["type"].as_str().ok_or(INVALID)? {
            "ws" | "httpupgrade" => {
                proxy["network"] = json!("ws");
                let mut opts = json!({});
                if let Some(path) = transport.get("path") {
                    opts["path"] = path.clone();
                }
                if let Some(headers) = transport.get("headers") {
                    opts["headers"] = headers.clone();
                }
                if let Some(host) = transport.get("host") {
                    opts["headers"] = json!({"Host":host});
                }
                if transport["type"] == "httpupgrade" {
                    opts["v2ray-http-upgrade"] = json!(true);
                }
                proxy["ws-opts"] = opts;
            }
            "grpc" => {
                proxy["network"] = json!("grpc");
                proxy["grpc-opts"] = json!({"grpc-service-name":transport["service_name"]});
            }
            _ => return Err(UNSUPPORTED.into()),
        }
    }
    if let Some(obfs) = old.get("obfs") {
        proxy["obfs"] = obfs["type"].clone();
        proxy["obfs-password"] = obfs["password"].clone();
    }
    if let Some(ports) = old.get("server_ports") {
        let ports = ports
            .as_array()
            .ok_or(INVALID)?
            .iter()
            .map(|v| v.as_str().ok_or(INVALID).map(|s| s.replace(':', "-")))
            .collect::<Result<Vec<_>, _>>()?;
        proxy["ports"] = json!(ports.join(","));
        proxy["port"] = json!(ports
            .first()
            .and_then(|s| s.split('-').next())
            .and_then(|s| s.parse::<u16>().ok())
            .ok_or(INVALID)?);
    }
    if let Some(interval) = old.get("hop_interval") {
        let min = seconds(interval)?;
        proxy["hop-interval"] = if let Some(max) = old.get("hop_interval_max") {
            json!(format!("{min}-{}", seconds(max)?))
        } else {
            json!(min)
        };
    }
    validate_native_proxy(&proxy)?;
    Ok(proxy)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_type_boundary_allows_core_protocols_and_rejects_control_and_mesh_adapters() {
        for kind in ["vless", "shadowquic", "gost-relay", "sudoku", "trusttunnel", "masque", "openvpn", "future-proxy"] {
            assert!(native_protocol(kind));
            assert!(validate_native_proxy(&json!({"type":kind,"server":"example.org","port":443})).is_ok());
        }
        for kind in ["direct", "reject", "dns", "rematch", "pass", "pass-rule", "compatible", "tailscale", "zerotier", "easytier", "", "VLESS", "foo/bar", "foo\nbar"] {
            assert!(!native_protocol(kind));
            assert!(validate_native_proxy(&json!({"type":kind,"server":"example.org","port":443})).is_err());
        }
    }

    #[test]
    fn native_parameters_preserve_metadata_and_keep_recursive_file_routing_and_tls_boundaries() {
        let proxy = json!({"type":"vless","server":"example.org","port":443,
            "unknown-option":{"future-value":true,"subscription-path":"/tunnel","description-file":"template"},"cipher":"auto","alterId":0,
            "plugin":"future-plugin","ports":"core-owned-syntax"});
        assert!(validate_native_proxy(&proxy).is_ok());
        for extra in [
            json!({"Dialer_Proxy":"DIRECT"}),
            json!({"Interface_Name":"eth0"}),
            json!({"routing-mark":123}),
            json!({"certificate":"/private/cert.pem"}),
            json!({"CA":"/private/ca.pem"}),
            json!({"ca_str":"/private/ca.pem"}),
            json!({"private-key":"/private/key.pem"}),
            json!({"config_path":"/private/config"}),
            json!({"insecure":"true"}),
            json!({"skip_cert_verify":1}),
        ] {
            let mut candidate = proxy.clone();
            candidate["transport-options"] = extra;
            assert!(validate_native_proxy(&candidate).is_err());
        }
        let mut approved = proxy;
        approved["transport-options"] = json!({"skip_cert_verify":true});
        assert!(validate_native_proxy(&approved).is_ok());
        assert!(is_insecure(&approved));
    }

    #[test]
    fn native_inline_ca_and_encoded_keys_never_become_local_paths() {
        let mut proxy = json!({"type":"openvpn","server":"example.org","port":443,
            "ca":"-----BEGIN CERTIFICATE-----\ninline\n-----END CERTIFICATE-----"});
        assert!(validate_native_proxy(&proxy).is_ok());
        proxy["ca"] = json!("ca.pem");
        assert!(validate_native_proxy(&proxy).is_err());
        proxy["ca"] = json!("-----BEGIN CERTIFICATE-----\ninline\n-----END CERTIFICATE-----");
        proxy["ca-str"] = proxy["ca"].clone();
        assert!(validate_native_proxy(&proxy).is_ok());
        for (kind, key, accepted) in [
            ("wireguard", "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", true),
            ("wireguard", "AQ==", true),
            ("masque", "AQ==", true),
            ("masque", "", true),
            ("masque", "/private/key.pem", true),
        ] {
            let candidate = json!({"type":kind,"server":"example.org","port":443,"private-key":key});
            // These adapters only decode inline key bytes; malformed encodings
            // are engine configuration errors, never requests to read a path.
            assert_eq!(validate_native_proxy(&candidate).is_ok(), accepted);
        }
        let ssh = json!({"type":"ssh","server":"example.org","port":22,"password":"secret"});
        assert!(validate_native_proxy(&ssh).is_err());
    }

    #[test]
    fn legacy_credentials_tls_transport_and_port_hopping_survive_conversion() {
        let (proxies, _) = runtime_parts(json!({"type":"socks","server":"localhost","server_port":1080,"version":"5","username":"user","password":"secret"})).unwrap();
        assert_eq!(proxies[0]["type"], "socks5");
        assert_eq!(proxies[0]["password"], "secret");
        let (proxies, _) = runtime_parts(json!({"type":"vless","server":"example.org","server_port":443,"uuid":"11111111-2222-3333-4444-555555555555","tls":{"enabled":true,"server_name":"secure.example","insecure":true,"ech":{"enabled":true,"query_server_name":"ech.example"}},"transport":{"type":"httpupgrade","path":"/ws","host":"cdn.example"}})).unwrap();
        assert_eq!(proxies[0]["skip-cert-verify"], true);
        assert_eq!(proxies[0]["servername"], "secure.example");
        assert_eq!(proxies[0]["ech-opts"]["query-server-name"], "ech.example");
        assert_eq!(proxies[0]["ws-opts"]["v2ray-http-upgrade"], true);
        assert_eq!(proxies[0]["ws-opts"]["headers"]["Host"], "cdn.example");
        let (proxies, _) = runtime_parts(json!({"type":"hysteria2","server":"example.org","server_ports":["20000:20002","30000"],"password":"secret","tls":{"enabled":true},"hop_interval":"30s","hop_interval_max":"60s","up_mbps":50,"obfs":{"type":"salamander","password":"obfs"}})).unwrap();
        assert_eq!(proxies[0]["ports"], "20000-20002,30000");
        assert_eq!(proxies[0]["hop-interval"], "30-60");
        assert_eq!(proxies[0]["up"], 50);
        assert_eq!(proxies[0]["obfs-password"], "obfs");
    }
    #[test]
    fn legacy_groups_preserve_selection_and_health_strategy() {
        let (proxies, groups) = runtime_parts(json!([
            {"type":"selector","tag":"account-node","outbounds":["slow","fast"],"default":"fast"},
            {"type":"http","tag":"slow","server":"example.org","server_port":8080},
            {"type":"urltest","tag":"fast","outbounds":["leaf"],"url":"https://example.org/ping","interval":"180s","tolerance":50},
            {"type":"http","tag":"leaf","server":"example.org","server_port":8081}
        ])).unwrap();
        assert_eq!(proxies.len(), 2);
        assert_eq!(groups[0]["proxies"][0], "fast");
        assert_eq!(groups[1]["interval"], 180);
        assert_eq!(groups[1]["lazy"], true);
        assert!(groups.iter().all(|g| g["empty-fallback"] == "REJECT"));
        assert!(runtime_parts(json!({"type":"direct"})).is_err());
        assert!(runtime_parts(
            json!({"type":"selector","outbounds":["DIRECT"],"default":"DIRECT"})
        )
        .is_err());
    }
    #[test]
    fn ssh_private_key_never_becomes_local_path() {
        let proxy = json!({"type":"ssh","server":"example.org","port":22,"username":"u","private-key":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="});
        assert!(validate_native_proxy(&proxy).is_err());
    }
}
