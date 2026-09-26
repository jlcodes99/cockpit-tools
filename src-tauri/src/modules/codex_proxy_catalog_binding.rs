//! Encrypted-account binding payload. Base64 is an envelope, never encryption.
//! Revalidate even internally generated snapshots: legacy setters accept strings.
use super::codex_proxy_subscription_parser::ParsedCatalog;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[path = "codex_proxy_mihomo.rs"]
mod mihomo;
pub use mihomo::{
    is_insecure, native_protocol, runtime_parts, validate_native_group, validate_native_proxy,
};

pub const PREFIX: &str = "cockpit-proxy://";
const INVALID: &str = "PROXY_RESOURCE_INVALID";
const UNSUPPORTED: &str = "PROXY_UNSUPPORTED_OPTION";
const MAX_BYTES: usize = 512 * 1024;
const MAX_ITEMS: usize = 512;

/// Native blocking leaves can remain inside a group without becoming an exit.
/// The other built-ins can bypass the configured proxy and remain unsupported.
pub fn is_blocking_builtin(name: &str) -> bool {
    matches!(name, "REJECT" | "REJECT-DROP")
}

pub(super) fn is_builtin_name(name: &str) -> bool {
    matches!(
        name,
        "DIRECT" | "REJECT" | "REJECT-DROP" | "PASS" | "PASS-RULE" | "COMPATIBLE"
    )
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Binding {
    version: u8,
    #[serde(default)]
    pub network: super::codex_proxy_network::NetworkOptions,
    #[serde(default)]
    insecure_tags: BTreeSet<String>,
    pub source_id: String,
    pub source_name: String,
    pub item_id: String,
    /// Picker context only; never changes the selected outbound or group policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    pub name: String,
    outbounds: Vec<Value>,
    /// Only display names; no server addresses or credentials returned by telemetry.
    names: BTreeMap<String, String>,
}

fn short_text(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 256
        && !s.chars().any(char::is_control)
        && !s.contains("://")
        && !s.contains("token=")
        && !s.contains("password=")
}
fn keys(value: &Value, allowed: &[&str]) -> Result<(), String> {
    let o = value.as_object().ok_or(INVALID)?;
    if o.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err(UNSUPPORTED.into());
    }
    Ok(())
}
fn string(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|s| s.len() <= 8192 && !s.chars().any(char::is_control))
}
fn optional_strings(v: &Value, fields: &[&str]) -> Result<(), String> {
    if fields.iter().any(|f| v.get(f).is_some_and(|x| !string(x))) {
        return Err(INVALID.into());
    }
    Ok(())
}
pub(super) fn valid_dns_name(name: &str) -> bool {
    let name = name.strip_suffix('.').unwrap_or(name);
    !name.is_empty()
        && name.len() <= 253
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}
fn validate_tls(v: &Value) -> Result<(), String> {
    keys(
        v,
        &[
            "enabled",
            "server_name",
            "alpn",
            "utls",
            "reality",
            "ech",
            "insecure",
        ],
    )?;
    if v["enabled"] != true {
        return Err(INVALID.into());
    }
    if v.get("insecure").is_some_and(|v| !v.is_boolean()) {
        return Err(INVALID.into());
    }
    optional_strings(v, &["server_name"])?;
    if let Some(a) = v.get("alpn") {
        let a = a.as_array().ok_or(INVALID)?;
        if a.len() > 32 || a.iter().any(|s| !string(s)) {
            return Err(INVALID.into());
        }
    }
    if let Some(u) = v.get("utls") {
        keys(u, &["enabled", "fingerprint"])?;
        if u["enabled"] != true
            || !u["fingerprint"].as_str().is_some_and(|s| {
                [
                    "chrome",
                    "firefox",
                    "safari",
                    "ios",
                    "android",
                    "edge",
                    "360",
                    "qq",
                    "random",
                    "randomized",
                ]
                .contains(&s)
            })
        {
            return Err(INVALID.into());
        }
    }
    if let Some(ech) = v.get("ech") {
        keys(ech, &["enabled", "query_server_name"])?;
        if ech["enabled"] != true
            || v.get("reality").is_some()
            || ech
                .get("query_server_name")
                .is_some_and(|q| !q.as_str().is_some_and(valid_dns_name))
        {
            return Err(INVALID.into());
        }
    }
    if let Some(r) = v.get("reality") {
        keys(r, &["enabled", "public_key", "short_id"])?;
        if r["enabled"] != true
            || !r["public_key"]
                .as_str()
                .is_some_and(|s| URL_SAFE_NO_PAD.decode(s).is_ok_and(|b| b.len() == 32))
            || !r["short_id"].as_str().is_some_and(|s| {
                s.len() <= 16 && s.len() % 2 == 0 && s.bytes().all(|b| b.is_ascii_hexdigit())
            })
        {
            return Err(INVALID.into());
        }
    }
    Ok(())
}

/// Strict capability allowlist: no files, detours, sockets, direct routes, plugins,
/// arbitrary transport headers can enter a running engine. TLS bypass is an explicit node opt-in.
pub fn validate_outbound(v: &Value) -> Result<(), String> {
    if is_insecure(v) {
        return Err(UNSUPPORTED.into());
    }
    validate_outbound_capabilities(v)
}
fn validate_outbound_capabilities(v: &Value) -> Result<(), String> {
    if v["type"] == "mihomo" {
        keys(v, &["type", "tag", "proxy"])?;
        return validate_native_proxy(&v["proxy"]);
    }
    let kind = v["type"].as_str().ok_or(INVALID)?;
    if ![
        "http",
        "socks",
        "shadowsocks",
        "vmess",
        "vless",
        "trojan",
        "hysteria2",
        "tuic",
    ]
    .contains(&kind)
    {
        return Err(UNSUPPORTED.into());
    }
    let mut permitted = vec!["type", "tag", "server", "server_port", "connect_timeout"];
    permitted.extend(match kind {
        "http" => vec!["username", "password", "tls"],
        "socks" => vec!["username", "password", "version"],
        "shadowsocks" => vec!["method", "password"],
        "vmess" => vec![
            "uuid",
            "security",
            "alter_id",
            "tls",
            "transport",
            "packet_encoding",
        ],
        "vless" => vec!["uuid", "flow", "tls", "transport", "packet_encoding"],
        "trojan" => vec!["password", "tls", "transport"],
        "hysteria2" => vec![
            "password",
            "tls",
            "obfs",
            "server_ports",
            "hop_interval",
            "hop_interval_max",
            "up_mbps",
            "down_mbps",
        ],
        "tuic" => vec![
            "uuid",
            "password",
            "tls",
            "congestion_control",
            "udp_relay_mode",
        ],
        _ => return Err(UNSUPPORTED.into()),
    });
    keys(v, &permitted)?;
    if matches!(kind, "vmess" | "vless" | "tuic")
        && !v["uuid"]
            .as_str()
            .is_some_and(|s| uuid::Uuid::parse_str(s).is_ok())
    {
        return Err(INVALID.into());
    }
    if matches!(kind, "shadowsocks" | "trojan" | "hysteria2" | "tuic")
        && !v["password"].as_str().is_some_and(|s| !s.is_empty())
    {
        return Err(INVALID.into());
    }
    if kind == "shadowsocks" && !v["method"].is_string() {
        return Err(INVALID.into());
    }
    if matches!(kind, "trojan" | "hysteria2" | "tuic") && v.get("tls").is_none() {
        return Err(INVALID.into());
    }
    if kind == "socks" && v["version"] != "5" {
        return Err(INVALID.into());
    }
    for (field, choices) in [
        ("version", &["5"][..]),
        (
            "method",
            &[
                "aes-128-gcm",
                "aes-192-gcm",
                "aes-256-gcm",
                "chacha20-ietf-poly1305",
                "xchacha20-ietf-poly1305",
                "2022-blake3-aes-128-gcm",
                "2022-blake3-aes-256-gcm",
                "2022-blake3-chacha20-poly1305",
            ][..],
        ),
        (
            "security",
            &["auto", "aes-128-gcm", "chacha20-poly1305", "zero", "none"][..],
        ),
        ("flow", &["xtls-rprx-vision"][..]),
        ("congestion_control", &["cubic", "new_reno", "bbr"][..]),
        ("udp_relay_mode", &["native", "quic"][..]),
        ("packet_encoding", &["xudp", "packetaddr"][..]),
    ] {
        if v.get(field)
            .is_some_and(|x| !x.as_str().is_some_and(|s| choices.contains(&s)))
        {
            return Err(UNSUPPORTED.into());
        }
    }
    let host = v["server"].as_str().ok_or(INVALID)?;
    if host.is_empty()
        || host.len() > 253
        || host
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || "/@?#".contains(c))
    {
        return Err(INVALID.into());
    }
    if let Some(ports) = v.get("server_ports") {
        let ports = ports.as_array().ok_or(INVALID)?;
        if kind != "hysteria2"
            || v.get("server_port").is_some()
            || ports.is_empty()
            || ports.len() > 128
            || ports
                .iter()
                .any(|item| !item.as_str().is_some_and(valid_hy2_server_port))
        {
            return Err(INVALID.into());
        }
    } else if !v["server_port"]
        .as_u64()
        .is_some_and(|n| (1..=65535).contains(&n))
    {
        return Err(INVALID.into());
    }
    if kind == "hysteria2" {
        for field in ["up_mbps", "down_mbps"] {
            if v.get(field)
                .is_some_and(|value| !value.as_u64().is_some_and(|n| n <= 1_000_000))
            {
                return Err(INVALID.into());
            }
        }
        let start = v.get("hop_interval").map(valid_hy2_interval).transpose()?;
        let max = v
            .get("hop_interval_max")
            .map(valid_hy2_interval)
            .transpose()?;
        if max.is_some() && (v.get("server_ports").is_none() || start.is_none() || max < start) {
            return Err(INVALID.into());
        }
        if start.is_some() && v.get("server_ports").is_none() {
            return Err(INVALID.into());
        }
    }
    optional_strings(
        v,
        &[
            "tag",
            "username",
            "password",
            "version",
            "method",
            "uuid",
            "security",
            "flow",
            "congestion_control",
            "udp_relay_mode",
            "packet_encoding",
        ],
    )?;
    if v.get("connect_timeout").is_some_and(|x| x != "10s")
        || v.get("alter_id").is_some_and(|x| x != 0)
    {
        return Err(INVALID.into());
    }
    if let Some(t) = v.get("tls") {
        validate_tls(t)?;
    }
    if let Some(t) = v.get("transport") {
        keys(t, &["type", "path", "host", "headers", "service_name"])?;
        if !matches!(t["type"].as_str(), Some("ws" | "httpupgrade" | "grpc")) {
            return Err(UNSUPPORTED.into());
        }
        optional_strings(t, &["path", "host", "service_name"])?;
        if let Some(h) = t.get("headers") {
            keys(h, &["Host"])?;
            if !string(&h["Host"]) {
                return Err(INVALID.into());
            }
        }
    }
    if let Some(o) = v.get("obfs") {
        keys(o, &["type", "password"])?;
        if o["type"] != "salamander" || !string(&o["password"]) {
            return Err(INVALID.into());
        }
    }
    Ok(())
}

fn valid_hy2_server_port(raw: &str) -> bool {
    let ports = raw.split(':').collect::<Vec<_>>();
    if ports.is_empty() || ports.len() > 2 {
        return false;
    }
    let Some(start) = ports[0].parse::<u16>().ok().filter(|n| *n > 0) else {
        return false;
    };
    ports.len() == 1 || ports[1].parse::<u16>().is_ok_and(|end| end >= start)
}
fn valid_hy2_interval(value: &Value) -> Result<u64, String> {
    let raw = value
        .as_str()
        .ok_or(INVALID)?
        .strip_suffix('s')
        .ok_or(INVALID)?;
    raw.parse::<u64>()
        .ok()
        .filter(|n| (1..=3600).contains(n))
        .ok_or_else(|| INVALID.into())
}

fn test_url(s: &str) -> Result<(), String> {
    let url = url::Url::parse(s).map_err(|_| INVALID)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || s.len() > 2048
    {
        return Err(INVALID.into());
    }
    Ok(())
}

pub fn encode(
    source_id: &str,
    source_name: &str,
    item_id: &str,
    catalog: &ParsedCatalog,
    selections: &BTreeMap<String, String>,
) -> Result<String, String> {
    encode_with_group(source_id, source_name, item_id, catalog, selections, None)
}

pub fn encode_with_group(
    source_id: &str,
    source_name: &str,
    item_id: &str,
    catalog: &ParsedCatalog,
    selections: &BTreeMap<String, String>,
    group_id: Option<&str>,
) -> Result<String, String> {
    let group_id = group_id.filter(|id| !id.is_empty());
    if let Some(id) = group_id {
        let group = catalog
            .groups
            .iter()
            .find(|group| group.id == id)
            .ok_or(INVALID)?;
        let member = catalog
            .nodes
            .iter()
            .find(|node| node.id == item_id)
            .map(|node| &node.name)
            .or_else(|| {
                catalog
                    .groups
                    .iter()
                    .find(|child| child.id == item_id)
                    .map(|child| &child.name)
            });
        if !short_text(id)
            || (id != item_id && !member.is_some_and(|name| group.members.contains(name)))
        {
            return Err(INVALID.into());
        }
    }
    let name = catalog
        .nodes
        .iter()
        .find(|n| n.id == item_id)
        .map(|n| n.name.clone())
        .or_else(|| {
            catalog
                .groups
                .iter()
                .find(|g| g.id == item_id)
                .map(|g| g.name.clone())
        })
        .ok_or(INVALID)?;
    if is_builtin_name(&name) {
        return Err(UNSUPPORTED.into());
    }
    let mut result = Binding {
        version: 2,
        network: Default::default(),
        insecure_tags: BTreeSet::new(),
        source_id: source_id.into(),
        source_name: source_name.into(),
        item_id: item_id.into(),
        group_id: group_id.map(str::to_owned),
        name: name.clone(),
        outbounds: Vec::new(),
        names: BTreeMap::new(),
    };
    fn visit(
        name: &str,
        tag: &str,
        catalog: &ParsedCatalog,
        selections: &BTreeMap<String, String>,
        result: &mut Binding,
        path: &mut BTreeSet<String>,
        depth: usize,
    ) -> Result<(), String> {
        if depth > 16 || result.outbounds.len() >= MAX_ITEMS || !path.insert(name.into()) {
            return Err(INVALID.into());
        }
        if let Some(n) = catalog.nodes.iter().find(|n| n.name == name) {
            if n.error.is_some() {
                return Err(UNSUPPORTED.into());
            }
            let mut outbound = n.outbound.clone().ok_or(UNSUPPORTED)?;
            validate_outbound_capabilities(&outbound)?;
            if is_insecure(&outbound) {
                result.insecure_tags.insert(tag.into());
            }
            outbound["tag"] = json!(tag);
            result.outbounds.push(outbound);
            result.names.insert(tag.into(), n.name.clone());
        } else {
            let g = catalog
                .groups
                .iter()
                .find(|g| g.name == name)
                .ok_or(INVALID)?;
            if g.error.is_some() {
                return Err(UNSUPPORTED.into());
            }
            let members = match g.kind.as_str() {
                "select" => {
                    let selected = selections
                        .get(&g.id)
                        .ok_or("PROXY_RESOURCE_SELECTION_REQUIRED")?;
                    if !g.members.contains(selected) {
                        return Err(INVALID.into());
                    }
                    vec![selected.clone()]
                }
                "url-test" | "fallback" | "load-balance" => g.members.clone(),
                _ => return Err(UNSUPPORTED.into()),
            };
            if members.is_empty() || members.len() > MAX_ITEMS {
                return Err(INVALID.into());
            }
            // Parent first, deterministic opaque tags; no user text is a config identifier.
            let index = result.outbounds.len();
            result.outbounds.push(Value::Null);
            result.names.insert(tag.into(), g.name.clone());
            let mut tags = Vec::new();
            for member in members {
                if is_blocking_builtin(&member) {
                    tags.push(member);
                    continue;
                }
                let child_tag = format!("resource-{}", result.outbounds.len());
                visit(
                    &member,
                    &child_tag,
                    catalog,
                    selections,
                    result,
                    path,
                    depth + 1,
                )?;
                tags.push(child_tag);
            }
            let mut native = g.native.clone().unwrap_or_else(|| {
                let mut value = json!({"type":g.kind});
                if g.kind != "select" {
                    value["url"] = json!(g
                        .url
                        .as_deref()
                        .unwrap_or("https://www.gstatic.com/generate_204"));
                    value["interval"] = json!(g.interval.unwrap_or(180));
                    if g.kind == "url-test" {
                        value["tolerance"] = json!(g.tolerance.unwrap_or(50));
                    }
                }
                value
            });
            native["name"] = json!(tag);
            if native.get("default-selected").is_some() {
                native["default-selected"] = json!(tags[0]);
            }
            native["proxies"] = json!(tags);
            native["empty-fallback"] = json!("REJECT");
            validate_native_group(&native, true)?;
            result.outbounds[index] = json!({"type":"mihomo-group", "tag":tag, "group":native});
        }
        path.remove(name);
        Ok(())
    }
    visit(
        &name,
        "account-node",
        catalog,
        selections,
        &mut result,
        &mut BTreeSet::new(),
        0,
    )?;
    validate(&result)?;
    let bytes = serde_json::to_vec(&result).map_err(|_| INVALID)?;
    if bytes.len() > MAX_BYTES {
        return Err(INVALID.into());
    }
    Ok(format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes)))
}

fn validate(binding: &Binding) -> Result<(), String> {
    if !matches!(binding.version, 1 | 2)
        || [
            &binding.source_id,
            &binding.source_name,
            &binding.item_id,
            &binding.name,
        ]
        .iter()
        .any(|s| !short_text(s))
        || binding.group_id.as_ref().is_some_and(|id| !short_text(id))
        || binding.outbounds.is_empty()
        || binding.outbounds.len() > MAX_ITEMS
        || binding.names.len() != binding.outbounds.len()
    {
        return Err(INVALID.into());
    }
    let mut graph = BTreeMap::<String, Vec<String>>::new();
    for v in &binding.outbounds {
        let tag = v["tag"].as_str().ok_or(INVALID)?;
        if !short_text(tag)
            || is_builtin_name(tag)
            || !binding.names.get(tag).is_some_and(|n| short_text(n))
            || graph.contains_key(tag)
        {
            return Err(INVALID.into());
        }
        let kind = v["type"].as_str().ok_or(INVALID)?;
        let mut edges: Vec<String> = Vec::new();
        if kind == "mihomo-group" {
            keys(v, &["type", "tag", "group"])?;
            validate_native_group(&v["group"], true)?;
            let members = v["group"]["proxies"].as_array().ok_or(INVALID)?;
            if members.is_empty()
                || members.len() > MAX_ITEMS
                || (v["group"]["type"] == "select" && members.len() != 1)
            {
                return Err(INVALID.into());
            }
            for member in members {
                edges.push(member.as_str().ok_or(INVALID)?.into());
            }
        } else if matches!(kind, "selector" | "urltest") {
            keys(
                v,
                if kind == "selector" {
                    &["type", "tag", "outbounds", "default"]
                } else {
                    &["type", "tag", "outbounds", "url", "interval", "tolerance"]
                },
            )?;
            let members = v["outbounds"].as_array().ok_or(INVALID)?;
            if members.is_empty() || members.len() > MAX_ITEMS {
                return Err(INVALID.into());
            }
            for m in members {
                edges.push(m.as_str().ok_or(INVALID)?.into());
            }
            if kind == "selector" {
                if edges.len() != 1 || v["default"].as_str() != Some(edges[0].as_str()) {
                    return Err(INVALID.into());
                }
            } else {
                test_url(v["url"].as_str().ok_or(INVALID)?)?;
                let interval = v["interval"]
                    .as_str()
                    .and_then(|s| s.strip_suffix('s'))
                    .and_then(|s| s.parse::<u64>().ok())
                    .ok_or(INVALID)?;
                if !(30..=86400).contains(&interval)
                    || !v["tolerance"].as_u64().is_some_and(|x| x <= 65535)
                {
                    return Err(INVALID.into());
                }
            }
        } else {
            if is_insecure(v) != binding.insecure_tags.contains(tag) {
                return Err(UNSUPPORTED.into());
            }
            validate_outbound_capabilities(v)?;
        }
        graph.insert(tag.into(), edges);
    }
    fn visit(
        tag: &str,
        graph: &BTreeMap<String, Vec<String>>,
        visiting: &mut BTreeSet<String>,
        visited: &mut BTreeSet<String>,
        depth: usize,
    ) -> Result<(), String> {
        if is_blocking_builtin(tag) {
            return Ok(());
        }
        if visited.contains(tag) {
            return Ok(());
        }
        if depth > 16 || !visiting.insert(tag.into()) {
            return Err(INVALID.into());
        }
        for member in graph.get(tag).ok_or(INVALID)? {
            visit(member, graph, visiting, visited, depth + 1)?;
        }
        visiting.remove(tag);
        visited.insert(tag.into());
        Ok(())
    }
    let mut visited = BTreeSet::new();
    visit(
        "account-node",
        &graph,
        &mut BTreeSet::new(),
        &mut visited,
        0,
    )?;
    if binding
        .insecure_tags
        .iter()
        .any(|tag| !graph.contains_key(tag))
    {
        return Err(INVALID.into());
    }
    if visited.len() != graph.len() {
        return Err(INVALID.into());
    }
    Ok(())
}

pub fn with_network(
    raw: &str,
    network: super::codex_proxy_network::NetworkOptions,
) -> Result<String, String> {
    network.validate()?;
    let mut b = decode(raw)?;
    b.network = network;
    Ok(format!(
        "{PREFIX}{}",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&b).map_err(|_| INVALID)?)
    ))
}
pub fn decode(raw: &str) -> Result<Binding, String> {
    if raw.len() > MAX_BYTES * 2 {
        return Err(INVALID.into());
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(raw.strip_prefix(PREFIX).ok_or(INVALID)?)
        .map_err(|_| INVALID)?;
    if bytes.len() > MAX_BYTES {
        return Err(INVALID.into());
    }
    let binding: Binding = serde_json::from_slice(&bytes).map_err(|_| INVALID)?;
    binding.network.validate()?;
    validate(&binding)?;
    Ok(binding)
}
pub fn belongs_to_source(raw: &str, source_id: &str) -> bool {
    raw.starts_with(PREFIX) && decode(raw).is_ok_and(|binding| binding.source_id == source_id)
}
pub fn outbounds(raw: &str) -> Result<Vec<Value>, String> {
    Ok(decode(raw)?.outbounds)
}
pub fn names(raw: &str) -> Result<BTreeMap<String, String>, String> {
    Ok(decode(raw)?.names)
}
pub fn summary(raw: &str) -> Option<Value> {
    let b = decode(raw).ok()?;
    let selected = b
        .outbounds
        .first()
        .and_then(|v| {
            if v["type"] == "selector" {
                v["default"].as_str()
            } else if v["type"] == "mihomo-group" && v["group"]["type"] == "select" {
                v["group"]["proxies"][0].as_str()
            } else {
                None
            }
        })
        .and_then(|tag| {
            b.names
                .get(tag)
                .map(String::as_str)
                .or_else(|| is_blocking_builtin(tag).then_some(tag))
        });
    Some(
        json!({"protocol":"RESOURCE", "name":b.name, "sourceName":b.source_name, "sourceId":b.source_id, "itemId":b.item_id, "groupId":b.group_id, "selectedName":selected}),
    )
}

#[cfg(test)]
mod tests {
    use super::super::codex_proxy_subscription_parser::{ParsedGroup, ParsedNode};
    use super::*;
    fn catalog() -> ParsedCatalog {
        ParsedCatalog {
            nodes: ["Alpha", "Beta"].into_iter().map(|n| ParsedNode {
                id: n.into(), name: n.into(), protocol: "HTTP".into(),
                outbound: Some(json!({"type":"http","server":"proxy.example","server_port":8080,"username":"user","password":"TOPSECRET"})), error: None,
            }).collect(),
            groups: vec![ParsedGroup { id: "auto".into(), name: "Auto".into(), kind: "url-test".into(), members: vec!["Alpha".into(), "Beta".into()], url: None, interval: Some(180), tolerance: Some(50), error: None, native: None },
                ParsedGroup { id: "choose".into(), name: "Choose".into(), kind: "select".into(), members: vec!["Auto".into(), "Alpha".into()], url: None, interval: None, tolerance: None, error: None, native: None }],
        }
    }
    fn raw(b: &Binding) -> String {
        format!(
            "{PREFIX}{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(b).unwrap())
        )
    }
    #[test]
    fn display_group_roundtrips_without_changing_outbounds() {
        let c = catalog();
        let legacy = encode("source", "Source", "Alpha", &c, &BTreeMap::new()).unwrap();
        assert!(decode(&legacy).unwrap().group_id.is_none());
        assert!(summary(&legacy).unwrap()["groupId"].is_null());
        for group in ["auto", "choose"] {
            let encoded = encode_with_group(
                "source",
                "Source",
                "Alpha",
                &c,
                &BTreeMap::new(),
                Some(group),
            )
            .unwrap();
            assert_eq!(decode(&encoded).unwrap().group_id.as_deref(), Some(group));
            assert_eq!(summary(&encoded).unwrap()["groupId"], group);
            assert_eq!(outbounds(&legacy).unwrap(), outbounds(&encoded).unwrap());
            let network = with_network(&encoded, Default::default()).unwrap();
            assert_eq!(summary(&network).unwrap()["groupId"], group);
        }
        assert!(encode_with_group(
            "source",
            "Source",
            "Beta",
            &c,
            &BTreeMap::new(),
            Some("choose")
        )
        .is_err());
        assert!(encode_with_group(
            "source",
            "Source",
            "Alpha",
            &c,
            &BTreeMap::new(),
            Some("missing")
        )
        .is_err());
        let automatic = encode_with_group(
            "source",
            "Source",
            "auto",
            &c,
            &BTreeMap::new(),
            Some("auto"),
        )
        .unwrap();
        assert_eq!(summary(&automatic).unwrap()["groupId"], "auto");
    }

    #[test]
    fn explicit_select_retains_native_urltest_and_only_reachable_nodes() {
        let c = catalog();
        assert_eq!(
            encode("source", "My source", "choose", &c, &BTreeMap::new()).unwrap_err(),
            "PROXY_RESOURCE_SELECTION_REQUIRED"
        );
        let mut selected = BTreeMap::new();
        selected.insert("choose".into(), "Auto".into());
        let value = encode("source", "My source", "choose", &c, &selected).unwrap();
        let b = decode(&value).unwrap();
        assert_eq!(b.outbounds.len(), 4);
        assert_eq!(b.outbounds[0]["group"]["type"], "select");
        assert_eq!(b.outbounds[1]["group"]["type"], "url-test");
        assert_eq!(b.outbounds[1]["group"]["interval"], 180);
        assert!(!summary(&value).unwrap().to_string().contains("TOPSECRET"));
        assert!(!summary(&value)
            .unwrap()
            .to_string()
            .contains("proxy.example"));
        selected.insert("choose".into(), "Alpha".into());
        assert_eq!(
            outbounds(&encode("source", "My source", "choose", &c, &selected).unwrap())
                .unwrap()
                .len(),
            2
        );
    }
    #[test]
    fn frozen_snapshot_does_not_follow_updated_source() {
        let mut c = catalog();
        let value = encode("source", "My source", "Alpha", &c, &BTreeMap::new()).unwrap();
        c.nodes[0].outbound.as_mut().unwrap()["server"] = json!("changed.example");
        assert_eq!(outbounds(&value).unwrap()[0]["server"], "proxy.example");
    }
    #[test]
    fn source_match_excludes_other_sources_and_direct_proxies() {
        let value = encode("source", "My source", "Alpha", &catalog(), &BTreeMap::new()).unwrap();
        assert!(belongs_to_source(&value, "source"));
        assert!(!belongs_to_source(&value, "another-source"));
        assert!(!belongs_to_source("http://127.0.0.1:8080", "source"));
        assert!(!belongs_to_source("cockpit-proxy://invalid", "source"));
    }
    #[test]
    fn rejects_injected_capabilities_cycles_and_orphan_nodes() {
        let value = encode("source", "My source", "Alpha", &catalog(), &BTreeMap::new()).unwrap();
        for (key, injected) in [
            ("detour", json!("direct")),
            ("bind_interface", json!("en0")),
            (
                "tls",
                json!({"enabled":true,"certificate_path":"/private/secret"}),
            ),
            ("tls", json!({"enabled":true,"insecure":true})),
            (
                "transport",
                json!({"type":"ws","headers":{"Authorization":"secret"}}),
            ),
            ("type", json!("direct")),
        ] {
            let mut b = decode(&value).unwrap();
            b.outbounds[0][key] = injected;
            assert!(decode(&raw(&b)).is_err());
        }
        let mut b = decode(&value).unwrap();
        b.outbounds[0] = json!({"tag":"account-node","type":"selector","outbounds":["account-node"],"default":"account-node"});
        assert!(decode(&raw(&b)).is_err());
        let mut b = decode(&value).unwrap();
        let mut orphan = b.outbounds[0].clone();
        orphan["tag"] = json!("orphan");
        b.outbounds.push(orphan);
        b.names.insert("orphan".into(), "orphan".into());
        assert!(decode(&raw(&b)).is_err());
    }
    #[test]
    fn blocking_leaves_are_revalidated_without_accepting_builtin_shadowing_or_bypass() {
        let mut c = catalog();
        c.groups[0].members = vec!["Alpha".into(), "REJECT".into()];
        let value = encode("s", "s", "auto", &c, &BTreeMap::new()).unwrap();
        for forbidden in ["DIRECT", "COMPATIBLE", "PASS", "PASS-RULE", "Missing"] {
            let mut b = decode(&value).unwrap();
            b.outbounds[0]["group"]["proxies"][1] = json!(forbidden);
            assert!(decode(&raw(&b)).is_err());
            assert!(runtime_parts(json!(b.outbounds)).is_err());
        }
        for builtin in ["REJECT", "REJECT-DROP"] {
            let mut b = decode(&value).unwrap();
            b.outbounds[0]["group"]["proxies"] = json!([builtin]);
            b.outbounds[1]["tag"] = json!(builtin);
            b.names.remove("resource-1");
            b.names.insert(builtin.into(), "Alpha".into());
            assert!(decode(&raw(&b)).is_err());
            assert!(runtime_parts(json!(b.outbounds)).is_err());
        }
    }
    #[test]
    fn unsupported_group_does_not_silently_change_strategy() {
        let mut c = catalog();
        c.groups[0].kind = "relay".into();
        assert_eq!(
            encode("source", "My source", "auto", &c, &BTreeMap::new()).unwrap_err(),
            UNSUPPORTED
        );
    }
    #[test]
    fn version_one_snapshot_keeps_credentials_and_explicit_tls_approval() {
        let value = encode("source", "Source", "Alpha", &catalog(), &BTreeMap::new()).unwrap();
        let mut binding = decode(&value).unwrap();
        binding.version = 1;
        binding.outbounds[0]["tls"] = json!({"enabled":true,"insecure":true});
        assert!(decode(&raw(&binding)).is_err());
        binding.insecure_tags.insert("account-node".into());
        let saved = raw(&binding);
        let decoded = decode(&saved).unwrap();
        assert_eq!(decoded.version, 1);
        let (proxies, groups) = runtime_parts(json!(outbounds(&saved).unwrap())).unwrap();
        assert!(groups.is_empty());
        assert_eq!(proxies[0]["password"], "TOPSECRET");
        assert_eq!(proxies[0]["skip-cert-verify"], true);
        assert_eq!(summary(&saved).unwrap()["sourceName"], "Source");
    }
}
