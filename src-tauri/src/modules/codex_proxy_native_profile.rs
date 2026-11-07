//! Structured subscriptions execute in the same Mihomo decoder as Clash Verge.
//! No host-side protocol, regexp, provider or override-expression translation.
use super::{ParsedCatalog, ParsedGroup, ParsedNode};
use crate::modules::codex_proxy_engine as engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

#[path = "codex_proxy_native_assets.rs"]
mod assets;
#[path = "codex_proxy_verge_compat.rs"]
mod verge;

pub(crate) const KIND: &str = "mihomo-profile";
const ERROR: &str = "PROXY_ENGINE_CONFIG_INVALID";
const MAX_CONFIG: usize = 4 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
pub struct NativeProfile {
    #[serde(default)]
    pub snapshot_id: String,
    pub config: Value,
    pub files: BTreeMap<String, String>,
    #[serde(default)]
    pub resource_directory: String,
    #[serde(default)]
    pub rules_name: String,
    #[serde(default)]
    pub names: BTreeMap<String, String>,
    #[serde(default)]
    pub members: BTreeMap<String, Vec<String>>,
}

impl NativeProfile {
    pub fn validate(&self) -> Result<(), String> {
        if !self.config.is_object()
            || self.config.to_string().len() > MAX_CONFIG
            || self.files.len() > 512
            || self.files.iter().any(|(p, id)| {
                !assets::relative(p) || id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit())
            })
        {
            return Err(ERROR.into());
        }
        Ok(())
    }
    pub fn entry(&self) -> String {
        let mut name = "account-node".to_owned();
        while ["proxies", "proxy-groups"].iter().any(|key| {
            self.config[*key]
                .as_array()
                .is_some_and(|list| list.iter().any(|v| v["name"] == name))
        }) {
            name.push('_');
        }
        name
    }
}

pub fn document(input: &str) -> Result<Option<Value>, String> {
    if input.len() > MAX_CONFIG {
        return Err("CATALOG_LIMIT".into());
    }
    let decoded;
    let input = if input
        .trim()
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"+/=_-\r\n ".contains(&b))
    {
        let compact: String = input.chars().filter(|c| !c.is_whitespace()).collect();
        decoded = super::b64(&compact)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok());
        decoded.as_deref().unwrap_or(input)
    } else {
        input
    };
    let mut yaml: serde_yaml::Value =
        match serde_yaml::from_str(input.trim_start_matches('\u{feff}')) {
            Ok(v) => v,
            Err(_) => return Ok(None),
        };
    yaml.apply_merge().map_err(|_| ERROR)?;
    let serde_yaml::Value::Mapping(mapping) = yaml else {
        return Ok(None);
    };
    let mut object = serde_json::Map::new();
    for (key, value) in mapping {
        if let Some(key) = key.as_str() {
            object.insert(
                key.to_ascii_lowercase(),
                serde_json::to_value(value).map_err(|_| ERROR)?,
            );
        }
    }
    let value = Value::Object(object);
    Ok(value
        .as_object()
        .is_some_and(|m| m.contains_key("proxies") || m.contains_key("proxy-providers"))
        .then_some(value))
}

pub fn envelope(
    profile: &NativeProfile,
    target: Value,
    selections: BTreeMap<String, String>,
) -> Value {
    if !profile.snapshot_id.is_empty() {
        return json!({"type":KIND,"tag":"account-node","profileId":profile.snapshot_id,"target":target,"selections":selections});
    }
    json!({"type":KIND, "tag":"account-node", "profile":profile,"target":target,"selections":selections})
}

pub fn valid_reference(value: &Value) -> bool {
    value["type"] == KIND
        && value["profileId"]
            .as_str()
            .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        && value["target"].is_object()
        && value["selections"].is_object()
}

pub async fn resolve(mut value: Value) -> Result<Value, String> {
    let item = if let Some(items) = value.as_array_mut() {
        if items.len() != 1 {
            return Ok(value);
        }
        &mut items[0]
    } else {
        &mut value
    };
    if item["type"] != KIND || item.get("profileId").is_none() {
        return Ok(value);
    }
    if !valid_reference(item) {
        return Err(ERROR.into());
    }
    let id = item["profileId"].as_str().ok_or(ERROR)?.to_owned();
    let profile: NativeProfile = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::task::spawn_blocking(move || {
            serde_json::from_slice(&assets::get(&id)?).map_err(|_| ERROR.to_owned())
        }),
    )
    .await
    .map_err(|_| "PROXY_ENGINE_TIMEOUT")?
    .map_err(|_| ERROR)??;
    profile.validate()?;
    item["profile"] = serde_json::to_value(profile).map_err(|_| ERROR)?;
    item.as_object_mut().ok_or(ERROR)?.remove("profileId");
    Ok(value)
}

pub fn from_outbound(
    value: &Value,
) -> Result<Option<(NativeProfile, Value, BTreeMap<String, String>)>, String> {
    let value = if let Some(items) = value.as_array() {
        if items.len() != 1 {
            return Ok(None);
        }
        &items[0]
    } else {
        value
    };
    if value["type"] != KIND {
        return Ok(None);
    }
    let profile: NativeProfile =
        serde_json::from_value(value["profile"].clone()).map_err(|_| ERROR)?;
    profile.validate()?;
    let target = value["target"].clone();
    if !target.is_object() {
        return Err(ERROR.into());
    }
    let selections = serde_json::from_value(value["selections"].clone()).map_err(|_| ERROR)?;
    Ok(Some((profile, target, selections)))
}

/// Preserve native routing semantics; host owns ingress and process control only.
pub fn configuration(
    profile: &NativeProfile,
    target: &Value,
    port: u16,
    username: &str,
    password: &str,
) -> Result<Value, String> {
    profile.validate()?;
    let mut config = scoped_config(&profile.config, target);
    let map = config.as_object_mut().ok_or(ERROR)?;
    for key in [
        "port",
        "socks-port",
        "redir-port",
        "tproxy-port",
        "listeners",
        "tunnels",
        "ss-config",
        "vmess-config",
        "tuic-server",
        "external-controller",
        "external-controller-tls",
        "external-controller-unix",
        "external-controller-pipe",
        "external-ui",
        "external-ui-url",
        "external-ui-name",
        "secret",
        "authentication",
        "skip-auth-prefixes",
        "iptables",
    ] {
        map.remove(key);
    }
    config["mixed-port"] = json!(port);
    config["bind-address"] = json!("127.0.0.1");
    config["allow-lan"] = json!(false);
    config["authentication"] = json!([format!("{username}:{password}")]);
    config["skip-auth-prefixes"] = json!([]);
    config["tun"] = json!({"enable":false});
    config["profile"] = json!({"store-selected":false,"store-fake-ip":false});
    config["log-level"] = json!("warning");
    if config.get("proxy-groups").is_none() {
        config["proxy-groups"] = json!([]);
    }
    if let Some(dns) = config.get_mut("dns").and_then(Value::as_object_mut) {
        dns.remove("listen");
    }
    if let Some(ntp) = config.get_mut("ntp").and_then(Value::as_object_mut) {
        ntp.insert("write-to-system".into(), json!(false));
    }
    if target["inventory"] == true {
        if !inventory_uses_routing(&profile.config) {
            config["mode"] = json!("rule");
            config["rules"] = json!(["MATCH,DIRECT"]);
        }
        return Ok(config);
    }
    if target["rules"] != true {
        if target["name"] == "GLOBAL" {
            config["mode"] = json!("global");
            return Ok(config);
        }
        let entry = profile.entry();
        let mut group = json!({"name":entry,"type":"select","empty-fallback":"REJECT"});
        if let Some(context) = target
            .get("context")
            .and_then(Value::as_str)
            .filter(|s| *s != "GLOBAL")
        {
            // Provider-local duplicate names resolve in the source group's order,
            // exactly as they do when selected inside Clash Verge.
            group = profile.config["proxy-groups"]
                .as_array()
                .and_then(|groups| groups.iter().find(|g| g["name"] == context))
                .cloned()
                .ok_or(ERROR)?;
            let name = target["name"].as_str().ok_or(ERROR)?;
            let members: Vec<Value> = group["proxies"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter(|v| v.as_str() == Some(name))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            group["name"] = json!(entry);
            group["type"] = json!("select");
            group["empty-fallback"] = json!("REJECT");
            group["proxies"] = json!(members);
            group["filter"] = json!(format!("^{}$", regex::escape(name)));
            group["default-selected"] = json!(name);
        } else if let Some(provider) = target.get("provider").and_then(Value::as_str) {
            let name = target["name"].as_str().ok_or(ERROR)?;
            group["use"] = json!([provider]);
            group["filter"] = json!(format!("^{}$", regex::escape(name)));
        } else {
            group["proxies"] = json!([target
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("REJECT")]);
        }
        let mut groups = config
            .get("proxy-groups")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        groups.push(group);
        config["proxy-groups"] = json!(groups);
        config["rules"] = json!([format!("MATCH,{entry}")]);
        config["mode"] = json!("rule");
    }
    Ok(config)
}

/// Forced-node/group connections do not evaluate the subscription's routing
/// table. Keep rule sets referenced by DNS or other retained native settings;
/// unrelated routing downloads must not block a working proxy node.
fn scoped_config(original: &Value, target: &Value) -> Value {
    let mut config = original.clone();
    if (target["rules"] == true && target["inventory"] != true)
        || (target["inventory"] == true && inventory_uses_routing(original))
    {
        return config;
    }
    let map = config
        .as_object_mut()
        .expect("validated native configuration");
    map.remove("rules");
    map.remove("sub-rules");
    let providers = map.remove("rule-providers");
    let mut required = std::collections::BTreeSet::new();
    fn reference(text: &str, required: &mut std::collections::BTreeSet<String>) {
        let lower = text.to_ascii_lowercase();
        if lower.starts_with("rule-set:") {
            required.extend(text[9..].split(',').map(|v| v.trim().to_owned()));
        } else if lower.starts_with("rule-set,") {
            if let Some(name) = text.split(',').nth(1) {
                required.insert(name.trim().to_owned());
            }
        }
    }
    fn scan(value: &Value, required: &mut std::collections::BTreeSet<String>) {
        match value {
            Value::String(text) => reference(text, required),
            Value::Array(values) => {
                for value in values {
                    scan(value, required);
                }
            }
            Value::Object(values) => {
                for (key, value) in values {
                    reference(key, required);
                    scan(value, required);
                }
            }
            _ => {}
        }
    }
    scan(&config, &mut required);
    if let Some(Value::Object(mut providers)) = providers {
        providers.retain(|name, _| required.contains(name));
        if !providers.is_empty() {
            config["rule-providers"] = Value::Object(providers);
        }
    }
    config
}

fn inventory_uses_routing(config: &Value) -> bool {
    // An unspecified download proxy uses the native routing table. Preserve
    // that bootstrap path while discovering remote proxy providers.
    config["proxy-providers"]
        .as_object()
        .is_some_and(|providers| {
            providers.values().any(|provider| {
                provider["type"] == "http" && provider["proxy"].as_str().is_none_or(str::is_empty)
            })
        })
}

fn relocate_files(value: &mut Value, directory: Option<&Path>) -> Result<(), String> {
    match value {
        Value::Object(map) => {
            let kind = map
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            for (key, value) in map {
                let file_field = (key == "path" && kind == "file")
                    || (!kind.is_empty()
                        && [
                            "certificate",
                            "private-key",
                            "client-cert",
                            "client-key",
                            "ca",
                            "ca-str",
                            "config-path",
                        ]
                        .contains(&key.as_str())
                        && !(key == "private-key"
                            && matches!(kind.as_str(), "wireguard" | "masque")));
                if file_field {
                    if let Some(text) = value.as_str().filter(|s| Path::new(s).is_absolute()) {
                        let root = directory.ok_or("PROXY_PROFILE_DIRECTORY")?;
                        let path = Path::new(text)
                            .strip_prefix(root)
                            .map_err(|_| "PROXY_PROFILE_DIRECTORY")?;
                        *value = json!(path
                            .to_str()
                            .ok_or("PROXY_PROFILE_RESOURCE")?
                            .replace('\\', "/"));
                    }
                }
                relocate_files(value, directory)?;
            }
        }
        Value::Array(items) => {
            for item in items {
                relocate_files(item, directory)?;
            }
        }
        _ => {}
    }
    Ok(())
}

async fn prepare(
    mut config: Value,
    directory: String,
    rules_name: String,
) -> Result<NativeProfile, String> {
    let cancel = assets::CancelOnDrop::new();
    let cancelled = cancel.0.clone();
    tokio::task::spawn_blocking(move || {
        config = verge::normalize(config);
        let directory = if directory.trim().is_empty() {
            None
        } else {
            Some(
                PathBuf::from(&directory)
                    .canonicalize()
                    .map_err(|_| "PROXY_PROFILE_DIRECTORY")?,
            )
        };
        let files = directory
            .as_ref()
            .map(|p| assets::capture(p, &cancelled))
            .transpose()?
            .unwrap_or_default();
        for section in ["proxy-providers", "rule-providers"] {
            if let Some(providers) = config.get_mut(section).and_then(Value::as_object_mut) {
                for (index, (_, provider)) in providers.iter_mut().enumerate() {
                    if provider["type"] == "http" {
                        // Cache writes belong to the private engine, never to Clash's files.
                        provider["path"] = json!(format!("cockpit-{section}-{index}.cache"));
                    }
                }
            }
        }
        relocate_files(&mut config, directory.as_deref())?;
        for section in ["proxy-providers", "rule-providers"] {
            if let Some(entries) = config[section].as_object() {
                for provider in entries.values() {
                    if provider["type"] != "file"
                        || provider["payload"]
                            .as_array()
                            .is_some_and(|p| !p.is_empty())
                    {
                        continue;
                    }
                    if directory.is_none() {
                        return Err("PROXY_PROFILE_DIRECTORY".into());
                    }
                    let path = provider["path"].as_str().ok_or("PROXY_PROFILE_RESOURCE")?;
                    if !files.contains_key(path) {
                        return Err("PROXY_PROFILE_RESOURCE".into());
                    }
                }
            }
        }
        let profile = NativeProfile {
            snapshot_id: String::new(),
            config,
            files,
            resource_directory: directory
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            rules_name,
            names: BTreeMap::new(),
            members: BTreeMap::new(),
        };
        profile.validate()?;
        Ok(profile)
    })
    .await
    .map_err(|_| ERROR)?
}

pub async fn restore(
    profile: &NativeProfile,
    directory: std::sync::Arc<engine::RuntimeDirectory>,
) -> Result<(), String> {
    assets::restore(profile.files.clone(), directory).await
}

pub async fn inspect(
    config: Value,
    directory: String,
    rules_name: String,
) -> Result<ParsedCatalog, String> {
    inspect_mode(config, directory, rules_name, false).await
}

pub async fn inspect_mode(
    config: Value,
    directory: String,
    rules_name: String,
    refreshing: bool,
) -> Result<ParsedCatalog, String> {
    let binary = engine::engine_path_async().await?;
    inspect_with_binary_mode(&binary, config, directory, rules_name, refreshing).await
}

#[cfg(test)]
pub(crate) async fn inspect_with_binary(
    binary: &Path,
    config: Value,
    directory: String,
    rules_name: String,
) -> Result<ParsedCatalog, String> {
    inspect_with_binary_mode(binary, config, directory, rules_name, false).await
}

async fn inspect_with_binary_mode(
    binary: &Path,
    config: Value,
    directory: String,
    rules_name: String,
    refreshing: bool,
) -> Result<ParsedCatalog, String> {
    let profile = prepare(config, directory, rules_name).await?;
    let target = json!({"rules":true,"inventory":true});
    let readiness_config = scoped_config(&profile.config, &target);
    let outbound = envelope(&profile, target, BTreeMap::new());
    let tunnel = engine::start_native_inspection(binary, outbound).await?;
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let mut profile = profile;
        let controller = tunnel.controller();
        wait_providers(&tunnel, &readiness_config, refreshing).await?;
        let providers = engine::native_request(&controller, "providers/proxies", None).await?;
        let proxies = engine::native_request(&controller, "proxies", None).await?;
        profile.files = assets::snapshot(tunnel.native_directory_lease()).await?;
        // Keep providers native, including their refresh interval and download
        // proxy. Captured files are bootstrap caches, as in Clash Verge.
        tokio::task::spawn_blocking(move || catalog(profile, proxies, providers))
            .await
            .map_err(|_| ERROR)?
    })
    .await
    .map_err(|_| "CATALOG_TIMEOUT".to_owned())
    .and_then(|v| v);
    let stopped = tunnel.stop().await;
    result.and_then(|value| stopped.map(|_| value))
}

/// Listening sockets precede provider initialization in Mihomo. Both import and
/// account startup wait for native readiness under their caller's deadline.
pub(crate) async fn wait_providers(
    tunnel: &engine::NodeTunnel,
    config: &Value,
    refreshing: bool,
) -> Result<(), String> {
    if ["proxy-providers", "rule-providers"].iter().all(|key| {
        config[*key]
            .as_object()
            .is_none_or(|entries| entries.is_empty())
    }) {
        return Ok(());
    }
    let controller = tunnel.controller();
    loop {
        let response = match engine::native_request(&controller, "providers/proxies", None).await {
            Ok(value) => value,
            Err(error) => {
                if !tunnel.is_running() {
                    return Err(error);
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let ready = config
            .get("proxy-providers")
            .and_then(Value::as_object)
            .is_none_or(|providers| {
                providers.keys().all(|name| {
                    response["providers"][name]["proxies"]
                        .as_array()
                        .is_some_and(|nodes| !nodes.is_empty())
                })
            });
        let mut files_ready = true;
        let rule_providers = if config["rule-providers"]
            .as_object()
            .is_some_and(|v| !v.is_empty())
        {
            engine::native_request(&controller, "providers/rules", None).await?
        } else {
            Value::Null
        };
        for section in ["proxy-providers", "rule-providers"] {
            if let Some(entries) = config[section].as_object() {
                for (name, provider) in entries {
                    let kind = if section == "proxy-providers" {
                        "proxy"
                    } else {
                        "rule"
                    };
                    let failed = tunnel.native_provider_failed(kind, name);
                    let payload = provider["payload"]
                        .as_array()
                        .is_some_and(|a| !a.is_empty());
                    if failed && (refreshing || !payload) {
                        return Err(if provider["type"] == "http" {
                            "CATALOG_DOWNLOAD"
                        } else {
                            "PROXY_PROFILE_RESOURCE"
                        }
                        .into());
                    }
                    if !failed && matches!(provider["type"].as_str(), Some("http" | "file")) {
                        let metadata = if kind == "proxy" {
                            &response
                        } else {
                            &rule_providers
                        };
                        files_ready &= metadata["providers"][name]["updatedAt"]
                            .as_str()
                            .is_some_and(|time| !time.starts_with("0001-") && !time.is_empty());
                    }
                    if provider["type"] == "http" && !failed {
                        let path = provider["path"].as_str().ok_or(ERROR)?;
                        files_ready &= tokio::fs::try_exists(tunnel.native_directory().join(path))
                            .await
                            .unwrap_or(false);
                    }
                }
            }
        }
        if ready && files_ready {
            return Ok(());
        }
        if !tunnel.is_running() {
            return Err(ERROR.into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn catalog(
    mut profile: NativeProfile,
    proxies: Value,
    providers: Value,
) -> Result<ParsedCatalog, String> {
    let mut nodes = Vec::new();
    let mut groups = Vec::new();
    let entry = profile.entry();
    for (name, proxy) in proxies["proxies"].as_object().ok_or(ERROR)? {
        if name == &entry {
            continue;
        }
        let name = label(name)?;
        profile.names.insert(name.clone(), name.clone());
        if let Some(members) = proxy["all"].as_array() {
            profile.members.insert(
                name.clone(),
                members
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect(),
            );
            let kind = match proxy["type"].as_str() {
                Some("Selector") => "select",
                Some("URLTest") => "url-test",
                Some("Fallback") => "fallback",
                Some("LoadBalance") => "load-balance",
                Some("Relay") => "relay",
                _ => "native",
            };
            groups.push(ParsedGroup {
                id: super::id("group", &name),
                name: name.clone(),
                kind: kind.into(),
                members: members
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect(),
                url: proxy["testUrl"].as_str().map(str::to_owned),
                interval: None,
                tolerance: None,
                error: None,
                native: Some(json!({"default-selected":proxy["now"]})),
            });
        } else {
            nodes.push(node(&name, name.clone(), None, proxy));
        }
    }
    for (provider, data) in providers["providers"].as_object().ok_or(ERROR)? {
        if profile.config["proxy-providers"].get(provider).is_none() {
            continue;
        }
        for proxy in data["proxies"].as_array().ok_or(ERROR)? {
            let raw = label(proxy["name"].as_str().ok_or(ERROR)?)?;
            let label = if nodes.iter().any(|n| n.name == raw) {
                label(&format!("{raw} ({provider})"))?
            } else {
                raw.clone()
            };
            profile
                .names
                .entry(raw.clone())
                .or_insert_with(|| label.clone());
            nodes.push(node(&label, raw, Some(provider.clone()), proxy));
        }
    }
    let mut name = if profile.rules_name.is_empty() {
        "Mihomo".to_owned()
    } else {
        profile.rules_name.clone()
    };
    while nodes.iter().any(|n| n.name == name) || groups.iter().any(|g| g.name == name) {
        name.push_str(" · Mihomo");
    }
    nodes.push(ParsedNode {
        id: super::id("native", "rules"),
        name,
        protocol: "profile".into(),
        outbound: Some(json!({"type":"mihomo-profile-ref","rules":true})),
        error: None,
    });
    profile.snapshot_id = assets::put(&serde_json::to_vec(&profile).map_err(|_| ERROR)?)?;
    Ok(ParsedCatalog {
        nodes,
        groups,
        native: Some(profile),
    })
}

fn label(value: &str) -> Result<String, String> {
    if value.is_empty() || value.len() > 8192 {
        return Err(ERROR.into());
    }
    Ok(value.to_owned())
}

fn node(label: &str, name: String, provider: Option<String>, metadata: &Value) -> ParsedNode {
    ParsedNode {
        id: super::id("node", label),
        name: label.into(),
        protocol: metadata["type"]
            .as_str()
            .unwrap_or("native")
            .to_ascii_lowercase(),
        outbound: Some(json!({"type":"mihomo-profile-ref","name":name,"provider":provider})),
        error: None,
    }
}

#[cfg(test)]
#[path = "codex_proxy_native_profile_tests.rs"]
mod tests;

pub fn binding_target(
    catalog: &ParsedCatalog,
    item_id: &str,
    choices: &BTreeMap<String, String>,
) -> Result<(Value, BTreeMap<String, String>), String> {
    let target = if let Some(node) = catalog.nodes.iter().find(|n| n.id == item_id) {
        node.outbound.clone().ok_or(ERROR)?
    } else {
        json!({"name":catalog.groups.iter().find(|g| g.id == item_id).ok_or(ERROR)?.name})
    };
    let mut selections = BTreeMap::new();
    for group in &catalog.groups {
        if group.kind != "select" {
            continue;
        }
        if let Some(choice) = choices.get(&group.id) {
            if !group.members.contains(choice) {
                return Err("PROXY_RESOURCE_SELECTION_REQUIRED".into());
            }
            selections.insert(group.name.clone(), choice.clone());
        } else if let Some(choice) = group
            .native
            .as_ref()
            .and_then(|g| g["default-selected"].as_str())
        {
            selections.insert(group.name.clone(), choice.into());
        }
    }
    Ok((target, selections))
}
