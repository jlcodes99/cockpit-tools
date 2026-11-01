//! Clash Verge's default client-side repairs, before the native Mihomo decoder.
//! Reference: enhance/mod.rs builtin_hy_alpn, builtin_meta_guard and
//! cleanup_proxy_groups. Provider expressions and protocol decoding stay native.
use serde_json::{json, Value};
use std::collections::HashSet;

pub(super) fn normalize(mut config: Value) -> Value {
    if let Some(map) = config.as_object_mut() {
        let old = std::mem::take(map);
        map.extend(
            old.into_iter()
                .map(|(key, value)| (key.to_ascii_lowercase(), value)),
        );
    }
    if config["mode"] == "script" {
        config["mode"] = json!("rule");
    }
    if let Some(proxies) = config.get_mut("proxies").and_then(Value::as_array_mut) {
        for proxy in proxies {
            if proxy["type"] == "hysteria" {
                if let Some(alpn) = proxy["alpn"].as_str().map(str::to_owned) {
                    proxy["alpn"] = json!([alpn]);
                }
            }
        }
    }
    let providers: HashSet<String> = config["proxy-providers"]
        .as_object()
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default();
    let mut allowed = providers.clone();
    allowed.extend(["DIRECT", "REJECT", "REJECT-DROP", "PASS", "PASS-RULE"].map(str::to_owned));
    for section in ["proxies", "proxy-groups"] {
        if let Some(items) = config[section].as_array() {
            allowed.extend(
                items
                    .iter()
                    .filter_map(|item| item["name"].as_str().or_else(|| item.as_str()))
                    .map(str::to_owned),
            );
        }
    }
    if let Some(groups) = config.get_mut("proxy-groups").and_then(Value::as_array_mut) {
        for group in groups {
            let mut provider_members = false;
            if let Some(uses) = group.get_mut("use").and_then(Value::as_array_mut) {
                uses.retain(|value| value.as_str().is_some_and(|name| providers.contains(name)));
                provider_members = !uses.is_empty();
            }
            if let Some(members) = group.get_mut("proxies").and_then(Value::as_array_mut) {
                members.retain(|value| {
                    value
                        .as_str()
                        .is_none_or(|name| provider_members || allowed.contains(name))
                });
            }
        }
    }
    config
}
