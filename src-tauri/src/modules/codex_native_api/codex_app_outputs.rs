//! Classify the two host-injected Codex app handoff records without inventing a
//! tool call. The same decision is used by replay validation and wire repair.
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Classification {
    Ordinary,
    Standalone,
    Ambiguous,
}

pub(super) struct Classifier {
    called: [bool; 2],
}

fn handoff_name(name: &str) -> Option<usize> {
    match name {
        "create_thread" => Some(0),
        "send_message_to_thread" => Some(1),
        _ => None,
    }
}

fn qualified_call(item: &Value) -> Option<usize> {
    // Wire repair already recognizes this spelling. Treat its normalized forms
    // conservatively as calls too; they must never make an output standalone.
    if !item
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("function_call"))
    {
        return None;
    }
    let name = item.get("name")?.as_str()?.trim();
    if item.get("namespace").and_then(Value::as_str) == Some("codex_app") {
        return handoff_name(name);
    }
    if item.get("namespace").is_none_or(Value::is_null) {
        // Recognize existing flattened tool spellings only as conflicts. They
        // never expand the exact standalone-output envelope accepted below.
        return ["codex_app.", "codex_app__", "mcp__codex_app__"]
            .into_iter()
            .find_map(|prefix| name.strip_prefix(prefix).and_then(handoff_name));
    }
    None
}

impl Classifier {
    pub(super) fn new(items: &[Value]) -> Self {
        let mut called = [false; 2];
        for item in items {
            if let Some(name) = qualified_call(item) {
                called[name] = true;
            }
        }
        Self { called }
    }

    pub(super) fn classify(&self, item: &Value) -> Classification {
        // Only the exact, observed host envelope qualifies. In particular a
        // null/blank/non-string call_id is not an absent call_id, and metadata
        // supplied by a client cannot turn another tool output into this kind.
        if item.get("type").and_then(Value::as_str) != Some("function_call_output")
            || item.get("namespace").and_then(Value::as_str) != Some("codex_app")
            || item.get("call_id").is_some()
            || !item.get("output").is_some_and(Value::is_string)
        {
            return Classification::Ordinary;
        }
        let Some(name) = item
            .get("name")
            .and_then(Value::as_str)
            .and_then(handoff_name)
        else {
            return Classification::Ordinary;
        };
        if self.called[name] {
            Classification::Ambiguous
        } else {
            Classification::Standalone
        }
    }
}

#[cfg(test)]
#[path = "codex_app_outputs_tests.rs"]
mod tests;
