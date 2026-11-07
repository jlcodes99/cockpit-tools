//! A precise upstream rejection may discard one unusable reasoning cipher.
//! This never decrypts content, drops compaction, or changes stored client data.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(super) fn rejection(event: &Value) -> Option<(u16, Value)> {
    (matches!(
        event["type"].as_str(),
        Some("error" | "response.error" | "response.failed")
    ) && super::diagnostics::error_object(event)["code"] == "invalid_encrypted_content"
        && super::bootstrap::pre_execution(event))
    .then(|| (400, event.clone()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Repair {
    digest: String,
}
impl Repair {
    pub fn rejected(body: &Value, event: &Value) -> Option<Self> {
        let error = super::diagnostics::error_object(event);
        if error["code"] != "invalid_encrypted_content"
            || ["previous_response_id", "conversation", "conversation_id"]
                .iter()
                .any(|key| body.get(key).is_some_and(|v| !v.is_null()))
        {
            return None;
        }
        let input = body.get("input")?.as_array()?;
        if !super::identity::rebuild::replay_items(input)
            || !super::identity::rebuild::paired_tools(input)
        {
            return None;
        }
        let message = error["message"].as_str().unwrap_or_default();
        let id = message
            .strip_prefix("The encrypted content for item ")
            .and_then(|rest| rest.split_once(" could not be verified"))
            .map(|(id, _)| id);
        let index = error["param"].as_str().and_then(|param| {
            let rest = param.strip_prefix("input[")?;
            let (index, suffix) = rest.split_once(']')?;
            matches!(suffix, "" | ".encrypted_content")
                .then(|| index.parse::<usize>().ok())
                .flatten()
        });
        let candidates: Vec<_> = input
            .iter()
            .enumerate()
            .filter(|(i, item)| {
                id.is_some_and(|id| item["id"].as_str() == Some(id)) || index == Some(*i)
            })
            .collect();
        let [(_, item)] = candidates.as_slice() else {
            return None;
        };
        if item["type"] != "reasoning" {
            return None;
        }
        let cipher = item["encrypted_content"]
            .as_str()
            .filter(|s| !s.is_empty())?;
        // If both upstream selectors exist, they must refer to this same item.
        if id.is_some_and(|id| item["id"].as_str() != Some(id))
            || index.is_some_and(|i| input.get(i) != Some(*item))
        {
            return None;
        }
        let repair = Self {
            digest: format!("{:x}", Sha256::digest(cipher.as_bytes())),
        };
        if input.iter().filter(|item| repair.matches(item)).count() != 1 {
            return None;
        }
        // Only known text representations can be preserved without inventing
        // plaintext or silently throwing away an unknown reasoning payload.
        repair.replacement(item)?;
        Some(repair)
    }
    fn matches(&self, item: &Value) -> bool {
        item["type"] == "reasoning"
            && item["encrypted_content"].as_str().is_some_and(|cipher| {
                format!("{:x}", Sha256::digest(cipher.as_bytes())) == self.digest
            })
    }
    fn replacement(&self, item: &Value) -> Option<Option<Value>> {
        let mut parts = Vec::new();
        for (field, kind) in [("summary", "summary_text"), ("content", "reasoning_text")] {
            match item.get(field) {
                None | Some(Value::Null) => {}
                Some(Value::Array(values)) => {
                    for part in values {
                        if part["type"] != kind {
                            return None;
                        }
                        let text = part["text"].as_str()?;
                        if !text.is_empty() {
                            parts.push(json!({"type":"output_text","text":text}));
                        }
                    }
                }
                _ => return None,
            }
        }
        Some(
            (!parts.is_empty())
                .then(|| json!({"type":"message","role":"assistant","content":parts})),
        )
    }
    pub fn apply_items(&self, input: &mut Vec<Value>) -> bool {
        let indexes: Vec<_> = input
            .iter()
            .enumerate()
            .filter_map(|(i, item)| self.matches(item).then_some(i))
            .collect();
        let [index] = indexes.as_slice() else {
            return false;
        };
        let Some(replacement) = self.replacement(&input[*index]) else {
            return false;
        };
        if let Some(replacement) = replacement {
            input[*index] = replacement;
        } else {
            input.remove(*index);
        }
        true
    }
    pub fn apply(&self, body: &mut Value) -> bool {
        body.get_mut("input")
            .and_then(Value::as_array_mut)
            .is_some_and(|items| self.apply_items(items))
    }
}

#[cfg(test)]
#[path = "encrypted_reasoning_tests.rs"]
mod tests;
