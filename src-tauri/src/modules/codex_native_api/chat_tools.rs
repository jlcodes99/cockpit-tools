//! Request-local protocol aliases. Tool arguments, outputs and opaque IDs are never rewritten.
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

pub(super) fn map(body: &mut Value) -> BTreeMap<String, String> {
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    if let Some(tools) = body["tools"].as_array() {
        for tool in tools {
            collect_tool(tool, &mut names, &mut seen);
        }
    }
    if let Some(choice) = body.get("tool_choice") {
        collect_choice(choice, &mut names, &mut seen);
    }
    if let Some(items) = body["input"].as_array() {
        for item in items {
            if matches!(
                item["type"].as_str(),
                Some("function_call" | "custom_tool_call")
            ) {
                collect(item, &mut names, &mut seen);
            }
        }
    }
    let mut used = HashSet::new();
    let mut forward = BTreeMap::new();
    let mut reverse = BTreeMap::new();
    for original in names {
        let base = alias(&original);
        let mut candidate = base.clone();
        let mut sequence = 1;
        while used.contains(&candidate) {
            let suffix = format!("_{sequence}");
            candidate = format!("{}{}", &base[..base.len().min(64 - suffix.len())], suffix);
            sequence += 1;
        }
        used.insert(candidate.clone());
        forward.insert(original.clone(), candidate.clone());
        reverse.insert(candidate, original);
    }
    if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
        for tool in tools {
            rewrite_tool(tool, &forward);
        }
    }
    if let Some(choice) = body.get_mut("tool_choice") {
        rewrite_choice(choice, &forward);
    }
    if let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) {
        for item in input {
            if matches!(
                item["type"].as_str(),
                Some("function_call" | "custom_tool_call")
            ) {
                rewrite(item, &forward);
            }
        }
    }
    reverse
}

fn alias(name: &str) -> String {
    let normalized: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if normalized.len() <= 64 {
        return normalized;
    }
    if normalized.starts_with("mcp__") {
        if let Some((_, last)) = normalized.rsplit_once("__") {
            let candidate = format!("mcp__{last}");
            return candidate[..candidate.len().min(64)].to_owned();
        }
    }
    normalized[..64].to_owned()
}

fn collect(value: &Value, names: &mut Vec<String>, seen: &mut HashSet<String>) {
    if let Some(name) = value["name"].as_str().filter(|name| !name.is_empty()) {
        if seen.insert(name.to_owned()) {
            names.push(name.to_owned());
        }
    }
}
fn collect_tool(tool: &Value, names: &mut Vec<String>, seen: &mut HashSet<String>) {
    if matches!(tool["type"].as_str(), Some("function" | "custom")) {
        collect(tool, names, seen);
    }
    if tool["type"] == "namespace" {
        if let Some(tools) = tool["tools"].as_array() {
            for tool in tools {
                collect_tool(tool, names, seen);
            }
        }
    }
}
fn collect_choice(choice: &Value, names: &mut Vec<String>, seen: &mut HashSet<String>) {
    if matches!(choice["type"].as_str(), Some("function" | "custom")) {
        collect(choice, names, seen);
    }
    if let Some(tools) = choice["tools"].as_array() {
        for choice in tools {
            collect_choice(choice, names, seen);
        }
    }
    if let Some(nested) = choice.get("allowed_tools") {
        collect_choice(nested, names, seen);
    }
}
fn rewrite(value: &mut Value, forward: &BTreeMap<String, String>) {
    if let Some(name) = value["name"].as_str().and_then(|name| forward.get(name)) {
        value["name"] = json!(name);
    }
}
fn rewrite_tool(tool: &mut Value, forward: &BTreeMap<String, String>) {
    if matches!(tool["type"].as_str(), Some("function" | "custom")) {
        rewrite(tool, forward);
    }
    if tool["type"] == "namespace" {
        if let Some(tools) = tool.get_mut("tools").and_then(Value::as_array_mut) {
            for tool in tools {
                rewrite_tool(tool, forward);
            }
        }
    }
}
fn rewrite_choice(choice: &mut Value, forward: &BTreeMap<String, String>) {
    if matches!(choice["type"].as_str(), Some("function" | "custom")) {
        rewrite(choice, forward);
    }
    if let Some(tools) = choice.get_mut("tools").and_then(Value::as_array_mut) {
        for choice in tools {
            rewrite_choice(choice, forward);
        }
    }
    if let Some(nested) = choice.get_mut("allowed_tools") {
        rewrite_choice(nested, forward);
    }
}
