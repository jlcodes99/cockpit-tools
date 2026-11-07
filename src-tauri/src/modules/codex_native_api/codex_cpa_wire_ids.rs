//! CPA input item ID normalization. IDs are counted in Unicode code points.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

const LIMIT: usize = 64;
const OCCUPIED: u8 = 1;
const PRESERVED: u8 = 2;
const SEEN: u8 = 4;
const DUPLICATE: u8 = 8;

fn prefixed(item: &Value, id: &str) -> String {
    let prefix = match item["type"].as_str() {
        Some("message") => "msg",
        Some("reasoning") => "rs",
        Some("function_call") => "fc",
        Some("custom_tool_call") => "ctc",
        Some("custom_tool_call_output") => "ctco",
        Some("tool_search_call") => {
            if let Some(suffix) = id.strip_prefix("ts_").filter(|suffix| !suffix.is_empty()) {
                return format!("tsc_{suffix}");
            }
            "tsc"
        }
        _ => return id.into(),
    };
    if id.is_empty() || id.starts_with(prefix) {
        id.into()
    } else {
        format!("{prefix}_{id}")
    }
}
fn portable(id: &str) -> String {
    id.chars().map(|ch| if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-') { ch } else { '_' }).collect()
}
pub(super) fn normalize(item: &Value, id: &str) -> String {
    let id = prefixed(item, id);
    if id.chars().count() > LIMIT { id } else { portable(&id) }
}
fn drop_encrypted_reasoning(item: &Value) -> bool {
    item["type"] == "reasoning"
        && item["id"]
            .as_str()
            .is_some_and(|id| id.chars().count() > LIMIT)
        && item["encrypted_content"]
            .as_str()
            .is_some_and(|cipher| !cipher.is_empty())
}
pub(super) fn hashed(id: &str, attempt: usize) -> String {
    let input = if attempt == 0 {
        id.into()
    } else {
        format!("{id}\0{attempt}")
    };
    let hash = Sha256::digest(input.as_bytes());
    let suffix = hash[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!(
        "{}_{}",
        portable(&id.chars().take(LIMIT - 17).collect::<String>()),
        suffix
    )
}

pub(super) fn sanitize_input_ids(body: &mut Value) {
    let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return;
    };
    let mut states: HashMap<String, u8> = HashMap::new();
    for item in input.iter() {
        if drop_encrypted_reasoning(item) {
            continue;
        }
        let Some(original) = item["id"].as_str() else {
            continue;
        };
        let normalized = normalize(item, original);
        let state = states.entry(normalized.clone()).or_default();
        if *state & SEEN != 0 {
            *state |= DUPLICATE;
        }
        *state |= SEEN;
        if normalized == original {
            *state |= PRESERVED;
        }
        if normalized.chars().count() <= LIMIT {
            *state |= OCCUPIED;
        }
    }
    let mut short: HashMap<String, String> = HashMap::new();
    let mut collisions: HashMap<String, String> = HashMap::new();
    input.retain_mut(|item| {
        if drop_encrypted_reasoning(item) {
            return false;
        }
        let Some(original) = item["id"].as_str() else {
            return true;
        };
        let mut id = normalize(item, original);
        if id != original && states.get(&id).is_some_and(|state| state & (PRESERVED | DUPLICATE) != 0) {
            let key = format!("{id}\0{original}");
            id = if let Some(mapped) = collisions.get(&key) {
                mapped.clone()
            } else {
                let candidate = if states.get(&id).is_some_and(|state| state & PRESERVED == 0) {
                    id.clone()
                } else {
                    let mut attempt = 0;
                    loop {
                        let candidate = hashed(&prefixed(item, original), attempt);
                        if !states
                            .get(&candidate)
                            .is_some_and(|state| state & OCCUPIED != 0)
                        {
                            break candidate;
                        }
                        attempt += 1;
                    }
                };
                *states.entry(candidate.clone()).or_default() |= OCCUPIED | PRESERVED;
                collisions.insert(key, candidate.clone());
                candidate
            };
        }
        if id.chars().count() > LIMIT {
            id = short
                .entry(id.clone())
                .or_insert_with(|| {
                    let mut attempt = 0;
                    loop {
                        let candidate = hashed(&id, attempt);
                        if !states
                            .get(&candidate)
                            .is_some_and(|state| state & OCCUPIED != 0)
                        {
                            *states.entry(candidate.clone()).or_default() |= OCCUPIED;
                            break candidate;
                        }
                        attempt += 1;
                    }
                })
                .clone();
        }
        if id != original {
            item["id"] = Value::String(id);
        }
        true
    });
}
/// Normalize only complete call/result histories. A lone result referring to
/// a server-side previous_response_id must retain the upstream's actual ID.
pub(super) fn shorten_call_ids(body: &mut Value) {
    use sha2::{Digest, Sha256};
    use std::collections::{HashMap, HashSet};
    let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) else { return; };
    let mut used: HashSet<String> = input.iter().filter_map(|item| item["call_id"].as_str()).map(str::to_owned).collect();
    let mut names = HashMap::new();
    for item in input.iter() {
        if !matches!(item["type"].as_str(), Some("function_call" | "custom_tool_call")) { continue; }
        let Some(id) = item["call_id"].as_str().filter(|id| id.len() > 64) else { continue; };
        if names.contains_key(id) { continue; }
        let mut attempt = 0;
        loop {
            let seed = if attempt == 0 { id.to_owned() } else { format!("{id}_{attempt}") };
            let digest = Sha256::digest(seed.as_bytes());
            let suffix: String = digest[..8].iter().map(|byte| format!("{byte:02x}")).collect();
            let mut end = 47.min(id.len());
            while !id.is_char_boundary(end) { end -= 1; }
            let short = format!("{}_{}", &id[..end], suffix);
            if used.insert(short.clone()) { names.insert(id.to_owned(), short); break; }
            attempt += 1;
        }
    }
    for item in input {
        if matches!(item["type"].as_str(), Some("function_call" | "custom_tool_call" | "function_call_output" | "custom_tool_call_output")) {
            if let Some(short) = item["call_id"].as_str().and_then(|id| names.get(id)) { item["call_id"] = Value::String(short.clone()); }
        }
    }
}
