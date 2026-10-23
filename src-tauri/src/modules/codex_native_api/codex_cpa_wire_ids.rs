//! CPA input item ID normalization. IDs are counted in Unicode code points.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

const LIMIT: usize = 64;
const OCCUPIED: u8 = 1;
const PRESERVED: u8 = 2;

pub(super) fn normalize(item: &Value, id: &str) -> String {
    let prefix = match item["type"].as_str() {
        Some("message") => "msg",
        Some("reasoning") => "rs",
        Some("function_call") => "fc",
        Some("custom_tool_call") => "ctc",
        Some("custom_tool_call_output") => "ctco",
        _ => return id.into(),
    };
    if id.is_empty() || id.starts_with(prefix) {
        id.into()
    } else {
        format!("{prefix}_{id}")
    }
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
        id.chars().take(LIMIT - 17).collect::<String>(),
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
        if id != original && states.get(&id).is_some_and(|state| state & PRESERVED != 0) {
            id = collisions
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
