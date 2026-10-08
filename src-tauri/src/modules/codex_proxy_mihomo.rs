//! Native Mihomo node boundary and lossless conversion of previously saved sing-box nodes.
//! Option names follow MetaCubeX/mihomo v1.19.31 adapter/outbound sources.
use super::{INVALID, UNSUPPORTED};
use serde_json::{json, Value};

pub fn native_protocol(kind: &str) -> bool {
    matches!(
        kind,
        "ss" | "ssr"
            | "socks5"
            | "http"
            | "vmess"
            | "vless"
            | "trojan"
            | "hysteria"
            | "hysteria2"
            | "tuic"
            | "wireguard"
            | "ssh"
            | "snell"
            | "anytls"
            | "mieru"
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
                        | "ca"
                        | "ca-str"
                        | "client-cert"
                        | "client-key"
                ) || key.ends_with("-path")
                    || key.ends_with("-file")
                {
                    return Err(UNSUPPORTED.into());
                }
                if ["skip-cert-verify", "insecure"].contains(&key.as_str()) && !value.is_boolean() {
                    return Err(INVALID.into());
                }
                // Mihomo treats certificate/private-key as either inline PEM or local paths.
                // WireGuard's base64 private key is also inline, never read from a file.
                if matches!(key.as_str(), "certificate" | "private-key") {
                    let raw = value.as_str().ok_or(INVALID)?;
                    use base64::Engine as _;
                    if !raw.starts_with("-----BEGIN ")
                        && !(key == "private-key"
                            && map.get("type").is_some_and(|v| v == "wireguard")
                            && base64::engine::general_purpose::STANDARD
                                .decode(raw)
                                .is_ok_and(|v| v.len() == 32))
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
    let map = proxy.as_object().ok_or(INVALID)?;
    let kind = proxy["type"].as_str().ok_or(INVALID)?;
    if !native_protocol(kind) {
        return Err(UNSUPPORTED.into());
    }
    bounded_parameters(proxy, 0)?;
    let options: &[&str] = match kind {
        "ss" => &[
            "name",
            "server",
            "port",
            "password",
            "cipher",
            "udp",
            "plugin",
            "plugin-opts",
            "udp-over-tcp",
            "udp-over-tcp-version",
            "client-fingerprint",
        ],
        "ssr" => &[
            "name",
            "server",
            "port",
            "password",
            "cipher",
            "obfs",
            "obfs-param",
            "protocol",
            "protocol-param",
            "udp",
        ],
        "socks5" => &[
            "name",
            "server",
            "port",
            "username",
            "password",
            "tls",
            "udp",
            "skip-cert-verify",
            "name-cert-verify",
            "fingerprint",
            "certificate",
            "private-key",
        ],
        "http" => &[
            "name",
            "server",
            "port",
            "username",
            "password",
            "tls",
            "sni",
            "skip-cert-verify",
            "name-cert-verify",
            "fingerprint",
            "certificate",
            "private-key",
            "headers",
        ],
        "vmess" => &[
            "name",
            "server",
            "port",
            "uuid",
            "alterId",
            "cipher",
            "udp",
            "network",
            "tls",
            "alpn",
            "skip-cert-verify",
            "name-cert-verify",
            "fingerprint",
            "certificate",
            "private-key",
            "servername",
            "ech-opts",
            "shadow-tls-opts",
            "restls-opts",
            "jls-opts",
            "reality-opts",
            "tlsmirror-opts",
            "mekya-opts",
            "mkcp-opts",
            "http-opts",
            "h2-opts",
            "grpc-opts",
            "ws-opts",
            "packet-addr",
            "xudp",
            "packet-encoding",
            "global-padding",
            "authenticated-length",
            "client-fingerprint",
        ],
        "vless" => &[
            "name",
            "server",
            "port",
            "uuid",
            "flow",
            "tls",
            "alpn",
            "udp",
            "packet-addr",
            "xudp",
            "packet-encoding",
            "encryption",
            "network",
            "ech-opts",
            "shadow-tls-opts",
            "restls-opts",
            "jls-opts",
            "reality-opts",
            "http-opts",
            "h2-opts",
            "grpc-opts",
            "ws-opts",
            "xhttp-opts",
            "ws-headers",
            "skip-cert-verify",
            "name-cert-verify",
            "fingerprint",
            "certificate",
            "private-key",
            "servername",
            "client-fingerprint",
        ],
        "trojan" => &[
            "name",
            "server",
            "port",
            "password",
            "alpn",
            "sni",
            "skip-cert-verify",
            "name-cert-verify",
            "fingerprint",
            "certificate",
            "private-key",
            "udp",
            "network",
            "ech-opts",
            "shadow-tls-opts",
            "restls-opts",
            "jls-opts",
            "reality-opts",
            "grpc-opts",
            "ws-opts",
            "ss-opts",
            "client-fingerprint",
        ],
        "hysteria" => &[
            "name",
            "server",
            "port",
            "ports",
            "protocol",
            "obfs-protocol",
            "up",
            "up-speed",
            "down",
            "down-speed",
            "auth",
            "auth-str",
            "obfs",
            "sni",
            "ech-opts",
            "skip-cert-verify",
            "name-cert-verify",
            "fingerprint",
            "certificate",
            "private-key",
            "alpn",
            "recv-window-conn",
            "recv-window",
            "disable-mtu-discovery",
            "fast-open",
            "hop-interval",
        ],
        "hysteria2" => &[
            "name",
            "server",
            "port",
            "udp",
            "ports",
            "hop-interval",
            "up",
            "down",
            "password",
            "obfs",
            "obfs-password",
            "obfs-min-packet-size",
            "obfs-max-packet-size",
            "sni",
            "ech-opts",
            "skip-cert-verify",
            "name-cert-verify",
            "fingerprint",
            "certificate",
            "private-key",
            "alpn",
            "cwnd",
            "bbr-profile",
            "udp-mtu",
            "handshake-timeout",
            "realm-opts",
            "initial-stream-receive-window",
            "max-stream-receive-window",
            "initial-connection-receive-window",
            "max-connection-receive-window",
        ],
        "tuic" => &[
            "name",
            "server",
            "port",
            "token",
            "uuid",
            "password",
            "ip",
            "heartbeat-interval",
            "alpn",
            "reduce-rtt",
            "request-timeout",
            "udp-relay-mode",
            "congestion-controller",
            "disable-sni",
            "max-udp-relay-packet-size",
            "fast-open",
            "max-open-streams",
            "cwnd",
            "bbr-profile",
            "skip-cert-verify",
            "name-cert-verify",
            "fingerprint",
            "certificate",
            "private-key",
            "recv-window-conn",
            "recv-window",
            "disable-mtu-discovery",
            "max-datagram-frame-size",
            "sni",
            "ech-opts",
            "udp-over-stream",
            "udp-over-stream-version",
        ],
        "wireguard" => &[
            "name",
            "ip",
            "ipv6",
            "private-key",
            "workers",
            "mtu",
            "udp",
            "persistent-keepalive",
            "ip-stack",
            "amnezia-wg-option",
            "peers",
            "remote-dns-resolve",
            "dns",
            "refresh-server-ip-interval",
            "server",
            "port",
            "public-key",
            "pre-shared-key",
            "reserved",
            "allowed-ips",
        ],
        "ssh" => &[
            "name",
            "server",
            "port",
            "username",
            "password",
            "private-key",
            "private-key-passphrase",
            "host-key",
            "host-key-algorithms",
        ],
        "snell" => &[
            "name",
            "server",
            "port",
            "psk",
            "udp",
            "version",
            "reuse",
            "obfs-opts",
            "client-fingerprint",
        ],
        "anytls" => &[
            "name",
            "server",
            "port",
            "password",
            "alpn",
            "sni",
            "ech-opts",
            "shadow-tls-opts",
            "restls-opts",
            "jls-opts",
            "client-fingerprint",
            "skip-cert-verify",
            "name-cert-verify",
            "fingerprint",
            "certificate",
            "private-key",
            "udp",
            "client-metadata",
            "idle-session-check-interval",
            "idle-session-timeout",
            "min-idle-session",
            "disable-reuse",
        ],
        "mieru" => &[
            "name",
            "server",
            "port",
            "port-range",
            "transport",
            "udp",
            "username",
            "password",
            "multiplexing",
            "handshake-mode",
            "traffic-pattern",
        ],
        _ => return Err(UNSUPPORTED.into()),
    };
    if map.keys().any(|k| {
        !options.contains(&k.as_str())
            && !["type", "tfo", "mptcp", "ip-version", "smux"].contains(&k.as_str())
    }) {
        return Err(UNSUPPORTED.into());
    }
    // Mihomo 1.19.31 always enables Hysteria2 UDP and ignores this common
    // subscription flag. Accept the matching `true`, never pretend `false`
    // disables UDP (Hysteria2Option has no UDP switch).
    if kind == "hysteria2" && proxy.get("udp").is_some_and(|value| value != true) {
        return Err(UNSUPPORTED.into());
    }
    if kind != "wireguard" || proxy.get("peers").is_none() {
        let host = proxy["server"].as_str().ok_or(INVALID)?;
        if host.is_empty()
            || host.len() > 253
            || host
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || "/@?#".contains(c))
        {
            return Err(INVALID.into());
        }
        if !proxy["port"]
            .as_u64()
            .is_some_and(|v| (1..=65535).contains(&v))
            && !(kind == "mieru" && proxy["port-range"].is_string())
        {
            return Err(INVALID.into());
        }
    }
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
    if let Some(plugin) = proxy.get("plugin") {
        if !plugin
            .as_str()
            .is_some_and(|p| ["obfs", "v2ray-plugin", "shadow-tls", "restls"].contains(&p))
        {
            return Err(UNSUPPORTED.into());
        }
    }
    if let Some(ports) = proxy.get("ports") {
        let ports = ports.as_str().ok_or(INVALID)?;
        if ports.split(',').any(|s| {
            let parts: Vec<_> = s.trim().split('-').collect();
            parts.is_empty()
                || parts.len() > 2
                || parts
                    .iter()
                    .any(|p| p.parse::<u16>().ok().is_none_or(|v| v == 0))
                || (parts.len() == 2
                    && parts[0].parse::<u16>().unwrap() > parts[1].parse::<u16>().unwrap())
        }) {
            return Err(UNSUPPORTED.into());
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
