//! Bounded, local-only subscription conversion. Never log input, outbounds or parser errors.
//! Format references: https://wiki.metacubex.one/en/config/proxies/ and /proxy-groups/;
//! Clash parameters stay native; only URI/legacy snapshots use the compatibility converter.
use super::codex_proxy_node_parser::parse_node_link;
use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use url::Url;

#[path = "codex_proxy_subscription_diagnostics.rs"]
mod diagnostics;
pub(super) use diagnostics::{group_issues, reachable_nodes, GroupIssue};

const INVALID: &str = "SUBSCRIPTION_INVALID";
const UNSUPPORTED: &str = "PROXY_UNSUPPORTED_OPTION";
const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_ITEMS: usize = 4096;
type Result<T> = std::result::Result<T, String>;

// These types contain secrets and must only be stored through encrypted storage.
// Deliberately no Debug; IPC callers must return their own redacted DTOs.
#[derive(Clone, Serialize, Deserialize)]
pub struct ParsedCatalog {
    pub nodes: Vec<ParsedNode>,
    pub groups: Vec<ParsedGroup>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct ParsedNode {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub outbound: Option<Value>,
    pub error: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct ParsedGroup {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub members: Vec<String>,
    pub url: Option<String>,
    pub interval: Option<u64>,
    pub tolerance: Option<u16>,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native: Option<Value>,
}
fn id(kind: &str, value: &str) -> String {
    format!("{:x}", Sha256::digest(format!("{kind}:{value}")))[..24].into()
}
fn b64(s: &str) -> Result<Vec<u8>> {
    [
        general_purpose::STANDARD,
        general_purpose::STANDARD_NO_PAD,
        general_purpose::URL_SAFE,
        general_purpose::URL_SAFE_NO_PAD,
    ]
    .iter()
    .find_map(|e| e.decode(s).ok())
    .ok_or_else(|| INVALID.into())
}
fn name(s: &str) -> Result<String> {
    let s = s.trim();
    if s.is_empty()
        || s.len() > 256
        || s.chars().any(char::is_control)
        || s.contains("://")
        || s.contains("token=")
        || s.contains("password=")
    {
        return Err(INVALID.into());
    }
    Ok(s.into())
}
fn text<'a>(m: &'a Map<String, Value>, k: &str) -> Result<&'a str> {
    let s = m.get(k).and_then(Value::as_str).ok_or(INVALID)?;
    if s.len() > 8192 || s.chars().any(char::is_control) {
        return Err(INVALID.into());
    }
    Ok(s)
}
fn optional<'a>(m: &'a Map<String, Value>, k: &str) -> Result<Option<&'a str>> {
    if m.contains_key(k) {
        text(m, k).map(Some)
    } else {
        Ok(None)
    }
}
fn flag(m: &Map<String, Value>, k: &str, default: bool) -> Result<bool> {
    m.get(k)
        .map(|v| v.as_bool().ok_or_else(|| INVALID.into()))
        .unwrap_or(Ok(default))
}
fn decoded(s: &str) -> Result<String> {
    let mut bytes = Vec::new();
    let mut chars = s.as_bytes().iter().copied();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let high = chars
                .next()
                .and_then(|b| (b as char).to_digit(16))
                .ok_or(INVALID)?;
            let low = chars
                .next()
                .and_then(|b| (b as char).to_digit(16))
                .ok_or(INVALID)?;
            bytes.push((high * 16 + low) as u8);
        } else {
            bytes.push(b);
        }
    }
    let s = String::from_utf8(bytes).map_err(|_| INVALID)?;
    if s.chars().any(char::is_control) {
        return Err(INVALID.into());
    }
    Ok(s)
}
fn direct(input: &str) -> Result<Value> {
    let normalized = super::codex_account_proxy::normalize_direct_proxy(input)?;
    let u = Url::parse(&normalized).map_err(|_| INVALID)?;
    let mut out = json!({"type":if u.scheme().starts_with("socks") {"socks"} else {"http"},
        "server":u.host_str().ok_or(INVALID)?.trim_matches(['[',']']),
        "server_port":u.port_or_known_default().ok_or(INVALID)?, "connect_timeout":"10s"});
    if u.scheme().starts_with("socks") {
        out["version"] = json!("5");
    }
    if u.scheme() == "https" {
        out["tls"] = json!({"enabled":true});
    }
    if !u.username().is_empty() {
        out["username"] = json!(decoded(u.username())?);
    }
    if let Some(p) = u.password() {
        out["password"] = json!(decoded(p)?);
    }
    Ok(out)
}
fn link_outbound(link: &str) -> Result<Value> {
    if ["http://", "https://", "socks5://", "socks5h://"]
        .iter()
        .any(|s| link.starts_with(s))
    {
        direct(link)
    } else {
        parse_node_link(link)
    }
}
fn link_name(link: &str, index: usize) -> String {
    let candidate = if let Some(s) = link.strip_prefix("vmess://") {
        b64(s)
            .ok()
            .and_then(|v| serde_json::from_slice::<Value>(&v).ok())
            .and_then(|v| v.get("ps").and_then(Value::as_str).map(str::to_owned))
    } else {
        link.split_once('#').and_then(|(_, s)| decoded(s).ok())
    };
    candidate
        .and_then(|s| name(&s).ok())
        .unwrap_or_else(|| format!("#{}", index + 1))
}
fn links(input: &str) -> Result<ParsedCatalog> {
    let lines: Vec<_> = input
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    if lines.is_empty() || lines.len() > MAX_ITEMS {
        return Err(INVALID.into());
    }
    let mut nodes = Vec::new();
    for (index, link) in lines.iter().enumerate() {
        if !link.contains("://") || link.len() > 8192 {
            return Err(INVALID.into());
        }
        let result = link_outbound(link);
        let node_name = link_name(link, index);
        let protocol = result
            .as_ref()
            .ok()
            .and_then(|o| o["type"].as_str())
            .unwrap_or("unsupported")
            .to_owned();
        let stable_name = if node_name != format!("#{}", index + 1) {
            node_name.clone()
        } else {
            (*link).into()
        };
        nodes.push(ParsedNode {
            id: id("node", &stable_name),
            name: node_name,
            protocol,
            error: result.as_ref().err().map(|e| safe_node_error(e)),
            outbound: result.ok(),
        });
    }
    let mut catalog = ParsedCatalog {
        nodes,
        groups: vec![],
    };
    validate(&mut catalog)?;
    Ok(catalog)
}

// serde_yaml discards unknown `!!domain` tags before Value visitation. Reject
// explicit tags lexically too, while preserving quoted credentials containing !.
fn reject_yaml_tags(input: &str) -> Result<()> {
    let mut quote = None;
    let mut comment = false;
    let mut escaped = false;
    let mut previous = ' ';
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if comment {
            if c == '\n' {
                comment = false;
            }
            previous = c;
            continue;
        }
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if q == '"' && c == '\\' {
                escaped = true;
            } else if c == q {
                if q == '\'' && chars.peek() == Some(&'\'') {
                    chars.next();
                } else {
                    quote = None;
                }
            }
        } else {
            let boundary = previous.is_whitespace() || "[{,:".contains(previous);
            if c == '!' && boundary {
                return Err(INVALID.into());
            }
            if c == '#' && (previous.is_whitespace() || previous == '\0') {
                comment = true;
            }
            if matches!(c, '\'' | '"') && boundary {
                quote = Some(c);
            }
        }
        previous = c;
    }
    Ok(())
}

fn bounded_yaml(value: &serde_yaml::Value, depth: usize, budget: &mut usize) -> Result<()> {
    if depth > 32 || *budget == 0 {
        return Err(INVALID.into());
    }
    *budget -= 1;
    match value {
        serde_yaml::Value::Tagged(_) => return Err(INVALID.into()),
        serde_yaml::Value::Sequence(items) => {
            for item in items {
                bounded_yaml(item, depth + 1, budget)?;
            }
        }
        serde_yaml::Value::Mapping(items) => {
            for (key, value) in items {
                if !key.is_string() {
                    return Err(INVALID.into());
                }
                bounded_yaml(key, depth + 1, budget)?;
                bounded_yaml(value, depth + 1, budget)?;
            }
        }
        serde_yaml::Value::String(s) if s.len() > 16384 => return Err(INVALID.into()),
        _ => {}
    }
    Ok(())
}

/// Only imports proxies and groups. Global DNS/rules/scripts/providers are never executed.
pub fn parse(input: &str) -> Result<ParsedCatalog> {
    if input.len() > MAX_BYTES {
        return Err("SUBSCRIPTION_TOO_LARGE".into());
    }
    let input = input.trim_start_matches('\u{feff}').trim();
    if input.is_empty() {
        return Err(INVALID.into());
    }
    let first = input
        .lines()
        .find(|l| !l.trim().is_empty() && !l.trim().starts_with('#'))
        .unwrap_or("")
        .trim();
    if first
        .split_once("://")
        .is_some_and(|(s, _)| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric()))
    {
        return links(input);
    }
    if input
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"+/=_-\r\n \t".contains(&b))
    {
        let compact: String = input.chars().filter(|c| !c.is_ascii_whitespace()).collect();
        let decoded = String::from_utf8(b64(&compact)?).map_err(|_| INVALID)?;
        return links(&decoded);
    }
    // Value rejects duplicate YAML keys; serde_yaml also bounds alias expansion/recursion.
    reject_yaml_tags(input)?;
    let yaml: serde_yaml::Value = serde_yaml::from_str(input).map_err(|_| INVALID)?;
    bounded_yaml(&yaml, 0, &mut 100_000)?;
    let value = serde_json::to_value(yaml).map_err(|_| INVALID)?;
    let root = value.as_object().ok_or(INVALID)?;
    let empty = Vec::new();
    let proxies = match root.get("proxies") {
        None => &empty,
        Some(v) => v.as_array().ok_or(INVALID)?,
    };
    let groups = match root.get("proxy-groups") {
        None => &empty,
        Some(v) => v.as_array().ok_or(INVALID)?,
    };
    if proxies.len() + groups.len() > MAX_ITEMS || proxies.is_empty() {
        return Err(INVALID.into());
    }
    let mut nodes = Vec::new();
    for proxy in proxies {
        let m = proxy.as_object().ok_or(INVALID)?;
        let node_name = name(text(m, "name")?)?;
        let protocol = text(m, "type")?;
        let result = clash_node(m);
        let insecure = result
            .as_ref()
            .is_ok_and(super::codex_proxy_catalog_binding::is_insecure);
        let error = result
            .as_ref()
            .err()
            .map(|e| safe_node_error(e))
            .or_else(|| insecure.then(|| "PROXY_TLS_INSECURE".into()));
        nodes.push(ParsedNode {
            id: id("node", &node_name),
            name: node_name,
            protocol: if super::codex_proxy_catalog_binding::native_protocol(protocol) {
                protocol.into()
            } else {
                "unsupported".into()
            },
            error,
            outbound: result.ok(),
        });
    }
    let node_names: Vec<_> = nodes.iter().map(|n| n.name.clone()).collect();
    let has_providers = root
        .get("proxy-providers")
        .is_some_and(|p| p.as_object().is_none_or(|m| !m.is_empty()));
    let parsed_groups = groups
        .iter()
        .map(|g| clash_group(g, &node_names, has_providers))
        .collect::<Result<_>>()?;
    let mut catalog = ParsedCatalog {
        nodes,
        groups: parsed_groups,
    };
    validate(&mut catalog)?;
    Ok(catalog)
}

// Never pass through arbitrary lower-level errors or subscription values to IPC.
fn safe_node_error(error: &str) -> String {
    match error {
        INVALID
        | "PROXY_TLS_INSECURE"
        | "PROXY_TRANSPORT_UNSUPPORTED"
        | "PROXY_ECH_UNSUPPORTED" => error.into(),
        _ => UNSUPPORTED.into(),
    }
}

fn clash_node(m: &Map<String, Value>) -> Result<Value> {
    let mut proxy = Value::Object(m.clone());
    let kind = proxy["type"].as_str().unwrap_or("");
    let alias = match kind {
        "vmess" | "vless" => Some(("sni", "servername")),
        "trojan" => Some(("servername", "sni")),
        _ => None,
    };
    if let Some((from, to)) = alias {
        if let Some(value) = proxy.get(from).cloned() {
            if proxy.get(to).is_some_and(|existing| existing != &value) {
                return Err(INVALID.into());
            }
            proxy[to] = value;
            proxy.as_object_mut().ok_or(INVALID)?.remove(from);
        }
    }
    // mport is a subscription alias. Conflicts are rejected rather than hidden.
    if let Some(mport) = proxy.get("mport").cloned() {
        if proxy.get("ports").is_some_and(|ports| ports != &mport) {
            return Err(UNSUPPORTED.into());
        }
        proxy["ports"] = mport;
        proxy.as_object_mut().ok_or(INVALID)?.remove("mport");
    }
    super::codex_proxy_catalog_binding::validate_native_proxy(&proxy)?;
    Ok(json!({"type":"mihomo", "proxy":proxy}))
}

/// Recheck retained definitions against current capabilities without changing
/// user permissions. A missing rejected definition requires an explicit refresh.
pub(super) fn revalidate_retained_nodes(catalog: &mut ParsedCatalog) {
    for node in &mut catalog.nodes {
        let Some(outbound) = node.outbound.as_ref() else {
            continue;
        };
        let approved = node.error.is_none();
        let result = if outbound["type"] == "mihomo" {
            outbound["proxy"]
                .as_object()
                .ok_or_else(|| INVALID.to_owned())
                .and_then(clash_node)
        } else {
            super::codex_proxy_catalog_binding::runtime_parts(outbound.clone()).and_then(
                |(nodes, groups)| {
                    if nodes.len() != 1 || !groups.is_empty() {
                        Err(UNSUPPORTED.into())
                    } else {
                        Ok(outbound.clone())
                    }
                },
            )
        };
        match result {
            Ok(current) => {
                node.error = (!approved
                    && super::codex_proxy_catalog_binding::is_insecure(&current))
                .then(|| "PROXY_TLS_INSECURE".into());
                if current["type"] == "mihomo" {
                    node.protocol = current["proxy"]["type"]
                        .as_str()
                        .unwrap_or("unsupported")
                        .to_owned();
                }
                node.outbound = Some(current);
            }
            Err(error) => node.error = Some(safe_node_error(&error)),
        }
    }
}

fn clash_group(value: &Value, node_names: &[String], has_providers: bool) -> Result<ParsedGroup> {
    let m = value.as_object().ok_or(INVALID)?;
    let group_name = name(text(m, "name")?)?;
    let kind = text(m, "type")?;
    let mut group = ParsedGroup {
        id: id("group", &group_name),
        name: group_name,
        kind: kind.into(),
        members: vec![],
        url: None,
        interval: None,
        tolerance: None,
        error: None,
        native: None,
    };
    let result = (|| -> Result<()> {
        // Keep members visible even when the strategy/options cannot be faithfully executed.
        if let Some(members) = m.get("proxies") {
            let members = members.as_array().ok_or(INVALID)?;
            if members.len() > MAX_ITEMS {
                return Err(INVALID.into());
            }
            group.members = members
                .iter()
                .map(|m| name(m.as_str().ok_or(INVALID)?))
                .collect::<Result<_>>()?;
        }
        if !matches!(kind, "select" | "url-test" | "fallback" | "load-balance") {
            return Err("SUBSCRIPTION_GROUP_STRATEGY".into());
        }
        if m.contains_key("use") || flag(m, "include-all-providers", false)? {
            return Err("SUBSCRIPTION_PROVIDER_UNSUPPORTED".into());
        }
        super::codex_proxy_catalog_binding::validate_native_group(value, false)
            .map_err(|_| "SUBSCRIPTION_GROUP_OPTIONS")?;
        let include_all = flag(m, "include-all", false)?;
        let include_proxies = flag(m, "include-all-proxies", false)?;
        if include_all && has_providers {
            return Err("SUBSCRIPTION_PROVIDER_UNSUPPORTED".into());
        }
        if include_all || include_proxies {
            let mut all = node_names.to_vec();
            all.sort();
            for name in all {
                if !group.members.contains(&name) {
                    group.members.push(name);
                }
            }
        }
        if group.members.is_empty() {
            return Err(UNSUPPORTED.into());
        }
        flag(m, "hidden", false)?;
        if let Some(raw) = optional(m, "url")? {
            let url = Url::parse(raw).map_err(|_| INVALID)?;
            if !matches!(url.scheme(), "http" | "https")
                || !url.username().is_empty()
                || url.password().is_some()
                || url.fragment().is_some()
                || url.host_str().is_none()
            {
                return Err(UNSUPPORTED.into());
            }
            group.url = Some(url.into());
        }
        if let Some(interval) = m.get("interval") {
            group.interval = Some(
                interval
                    .as_u64()
                    .filter(|v| (0..=86400).contains(v))
                    .ok_or(UNSUPPORTED)?,
            );
        }
        if let Some(tolerance) = m.get("tolerance") {
            group.tolerance =
                Some(u16::try_from(tolerance.as_u64().ok_or(INVALID)?).map_err(|_| INVALID)?);
        }
        let mut native = value.clone();
        for key in [
            "include-all",
            "include-all-proxies",
            "include-all-providers",
            "interrupt-exist-connections",
        ] {
            native.as_object_mut().ok_or(INVALID)?.remove(key);
        }
        native["proxies"] = json!(group.members);
        group.native = Some(native);
        Ok(())
    })();
    group.error = result.err();
    // Avoid displaying arbitrary attacker-supplied protocol text.
    if !["select", "url-test", "fallback", "load-balance", "relay"].contains(&kind) {
        group.kind = "unsupported".into();
    }
    Ok(group)
}

pub(super) fn validate(catalog: &mut ParsedCatalog) -> Result<()> {
    // Availability is derived from the current capabilities and permissions.
    // Recompute it for persisted catalogs; keep actual parse/option failures.
    for group in &mut catalog.groups {
        if group
            .error
            .as_deref()
            .is_some_and(diagnostics::derived_group_error)
        {
            group.error = None;
        }
        if super::codex_proxy_catalog_binding::is_builtin_name(&group.name) {
            group.error = Some("SUBSCRIPTION_GROUP_OPTIONS".into());
        }
    }
    for node in &mut catalog.nodes {
        // Keep legacy entries visible instead of invalidating the whole store.
        // A subscription entry cannot shadow an engine built-in.
        if super::codex_proxy_catalog_binding::is_builtin_name(&node.name) {
            node.error = Some(UNSUPPORTED.into());
        }
    }
    let mut names = HashSet::new();
    for name in catalog
        .nodes
        .iter()
        .map(|n| &n.name)
        .chain(catalog.groups.iter().map(|g| &g.name))
    {
        if !names.insert(name.clone()) {
            return Err("SUBSCRIPTION_DUPLICATE_NAME".into());
        }
    }
    let nodes: HashMap<_, _> = catalog
        .nodes
        .iter()
        .map(|n| (n.name.as_str(), n.error.is_none() && n.outbound.is_some()))
        .collect();
    let groups: HashMap<_, _> = catalog
        .groups
        .iter()
        .enumerate()
        .map(|(i, g)| (g.name.as_str(), i))
        .collect();
    // Least fixed point handles nested selectors with usable exits without
    // letting cycles alone become valid. OR for manual choices; AND for
    // URLTest, which must never silently lose an unsupported candidate.
    let mut available = vec![false; catalog.groups.len()];
    for _ in 0..32 {
        let previous = available.clone();
        for (i, group) in catalog.groups.iter().enumerate() {
            if previous[i] || group.error.is_some() || group.members.is_empty() {
                continue;
            }
            let supported = |member: &String| {
                if super::codex_proxy_catalog_binding::is_builtin_name(member) {
                    return super::codex_proxy_catalog_binding::is_blocking_builtin(member);
                }
                nodes.get(member.as_str()).copied().unwrap_or_else(|| {
                    groups
                        .get(member.as_str())
                        .is_some_and(|index| previous[*index])
                })
            };
            available[i] = if group.kind == "select" {
                group.members.iter().any(supported)
            } else {
                group.members.iter().all(supported)
            };
        }
        if available == previous {
            break;
        }
    }
    let issues = group_issues(catalog);
    let invalid = available
        .iter()
        .enumerate()
        .filter_map(|(i, valid)| (!valid).then_some(i));
    for i in invalid {
        if catalog.groups[i].error.is_none() {
            let reasons = &issues[i];
            let error = if reasons
                .iter()
                .any(|issue| issue.error == "SUBSCRIPTION_GROUP_CYCLE")
            {
                "SUBSCRIPTION_GROUP_CYCLE"
            } else if reasons
                .iter()
                .any(|issue| issue.error == "SUBSCRIPTION_GROUP_MEMBER_MISSING")
            {
                "SUBSCRIPTION_GROUP_MEMBER_MISSING"
            } else if !reasons.is_empty()
                && reasons
                    .iter()
                    .all(|issue| issue.error == "PROXY_TLS_INSECURE")
            {
                "PROXY_TLS_INSECURE"
            } else {
                "SUBSCRIPTION_GROUP_MEMBER_UNSUPPORTED"
            };
            catalog.groups[i].error = Some(error.into());
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "codex_proxy_subscription_parser_tests.rs"]
mod tests;
