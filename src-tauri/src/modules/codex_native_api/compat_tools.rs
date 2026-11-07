//! Reversible legal protocol names only; tool arguments, results and opaque history are
//! never searched/replaced. IDs keep matching a result to its original call.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn map(body: &mut Value) -> BTreeMap<String, String> {
    let mut originals = BTreeSet::new();
    if let Some(tools) = body["tools"].as_array() {
        for tool in tools {
            if tool["type"] == "function" {
                collect_name(tool, &mut originals);
            }
        }
    }
    if let Some(input) = body["input"].as_array() {
        for item in input {
            if item["type"] == "function_call" {
                collect_name(item, &mut originals);
            }
        }
    }
    collect_choice(body.get("tool_choice"), &mut originals);
    let mut used = originals
        .iter()
        .filter(|name| name.len() <= 64 && legal_name(name))
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut forward = BTreeMap::new();
    let mut reverse = BTreeMap::new();
    for original in originals {
        let sanitized = sanitize_name(&original);
        let alias = if original.len() <= 64 && legal_name(&original) {
            original.clone()
        } else if sanitized.len() <= 64 && !used.contains(&sanitized) {
            sanitized
        } else {
            let hash = format!("{:x}", Sha256::digest(original.as_bytes()));
            let mut sequence = 0;
            loop {
                let suffix = if sequence == 0 {
                    format!("_{}", &hash[..12])
                } else {
                    format!("_{}_{}", &hash[..12], sequence)
                };
                let candidate = format!("{}{}", prefix(&sanitized, 64 - suffix.len()), suffix);
                if !used.contains(&candidate) {
                    break candidate;
                }
                sequence += 1;
            }
        };
        used.insert(alias.clone());
        forward.insert(original.clone(), alias.clone());
        reverse.insert(alias, original);
    }
    if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
        for tool in tools {
            if tool["type"] == "function" {
                rewrite_name(tool, &forward);
            }
        }
    }
    if let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) {
        for item in input {
            if item["type"] == "function_call" {
                rewrite_name(item, &forward);
            }
        }
    }
    if let Some(choice) = body.get_mut("tool_choice") {
        rewrite_choice(choice, &forward);
    }
    reverse
}
fn legal_name(name: &str) -> bool {
    name.bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn prefix(value: &str, limit: usize) -> &str {
    let mut end = value.len().min(limit);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}
fn collect_name(value: &Value, names: &mut BTreeSet<String>) {
    if let Some(name) = value["name"].as_str() {
        names.insert(name.to_owned());
    }
}
fn collect_choice(value: Option<&Value>, names: &mut BTreeSet<String>) {
    if let Some(value) = value {
        if value["type"] == "function" {
            collect_name(value, names);
        }
        if let Some(tools) = value["tools"].as_array() {
            for tool in tools {
                collect_choice(Some(tool), names);
            }
        }
    }
}
fn rewrite_name(value: &mut Value, names: &BTreeMap<String, String>) {
    if let Some(alias) = value["name"].as_str().and_then(|name| names.get(name)) {
        value["name"] = json!(alias);
    }
}
fn rewrite_choice(value: &mut Value, names: &BTreeMap<String, String>) {
    if value["type"] == "function" {
        rewrite_name(value, names);
    }
    if let Some(tools) = value.get_mut("tools").and_then(Value::as_array_mut) {
        for tool in tools {
            rewrite_choice(tool, names);
        }
    }
}

#[cfg(test)]
#[path = "compat_tools_tests.rs"]
mod tests;
