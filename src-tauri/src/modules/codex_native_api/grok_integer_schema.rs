//! Official Codex integer fields, matching CPA's tool-schema compatibility map.
use serde_json::{json, Value};

pub(crate) fn normalize(schema: &mut Value, name: &str, namespace: Option<&str>) {
    let qualified = namespace
        .filter(|ns| !matches!(*ns, "functions" | "collab"))
        .map_or_else(|| name.to_owned(), |ns| format!("{ns}__{name}"));
    let mut name = qualified
        .strip_prefix("functions__")
        .or_else(|| qualified.strip_prefix("collab__"))
        .unwrap_or(&qualified);
    if let Some(short) = name
        .strip_prefix("collaboration__")
        .or_else(|| name.strip_prefix("multi_agent_v1__"))
    {
        name = short;
    }
    normalize_fields(schema, name, false);
}

/// CPA's strict target map is narrower than the legacy Grok adapter's aliases.
/// Keep the latter intact while applying upstream matching at shared boundaries.
pub(super) fn normalize_cpa(schema: &mut Value, name: &str) {
    let name = name.trim();
    let name = name
        .strip_prefix("functions__")
        .or_else(|| name.strip_prefix("collab__"))
        .unwrap_or(name);
    let name = match name {
        "multi_agent_v1__wait_agent" | "collaboration__wait_agent" => "wait_agent",
        "collaboration__get_channels"
        | "collaboration__list_threads"
        | "collaboration__search_posts"
        | "collaboration__read_thread"
        | "collaboration__read_post" => name.strip_prefix("collaboration__").unwrap(),
        _ => name,
    };
    normalize_fields(schema, name, true);
}

fn normalize_fields(schema: &mut Value, name: &str, exact: bool) {
    let paths: &[&str] = match name {
        "exec_command" => &["yield_time_ms", "max_output_tokens", "timeout_ms"],
        "write_stdin" => &["session_id", "yield_time_ms", "max_output_tokens"],
        "sleep" => &["duration_ms"],
        "wait_agent" => &["timeout_ms"],
        "wait" => &["yield_time_ms", "max_tokens"],
        "tool_search" => &["limit"],
        "test_sync_tool" => &[
            "sleep_before_ms",
            "sleep_after_ms",
            "participants",
            "timeout_ms",
            "barrier.properties.participants",
            "barrier.properties.timeout_ms",
        ],
        "create_goal" => &["token_budget"],
        "get_channels" => &["limit"],
        "list_threads" | "search_posts" | "read_thread" => &["limit", "max_chars_per_post"],
        "read_post" => &["offset_chars", "limit_chars"],
        "memories__list" => &["max_results"],
        "memories__read" => &["line_offset", "max_lines"],
        "memories__search" => &["context_lines", "max_results"],
        "history__list_windows" | "history__search_contents" => &["limit"],
        "history__list_items" => &["limit", "max_chars_per_item"],
        "history__read_item" => &["offset_chars", "limit_chars"],
        "notes__list_files_by_prefix" => &["max_results"],
        "notes__read_file" => &[
            "start_line",
            "stop_line",
            "start_line.anyOf.0",
            "stop_line.anyOf.0",
        ],
        "notes__search_contents" => &["max_matches_per_file", "max_files"],
        "image_gen__imagegen" => &["num_last_images_to_include"],
        "web__run" => &[
            "search_query.items.properties.recency",
            "image_query.items.properties.recency",
            "open.items.properties.lineno",
            "click.items.properties.id",
            "screenshot.items.properties.pageno",
            "weather.items.properties.duration",
            "sports.items.properties.num_games",
        ],
        _ => &[],
    };
    for path in paths {
        let pointer = format!("/properties/{}/type", path.replace('.', "/"));
        match schema.pointer_mut(&pointer) {
            Some(value @ Value::String(_)) if value == "number" => *value = json!("integer"),
            Some(Value::Array(values)) => {
                if exact && !values.iter().any(|value| value == "number") {
                    continue;
                }
                for value in values.iter_mut() {
                    if value == "number" {
                        *value = json!("integer");
                    }
                }
                let mut seen = Vec::new();
                values.retain(|value| {
                    if seen.contains(value) {
                        false
                    } else {
                        seen.push(value.clone());
                        true
                    }
                });
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grok_official_integer_fields_are_targeted_and_preserve_unrelated_numbers() {
        let mut schema = json!({"type":"object","properties":{"yield_time_ms":{"type":"number","minimum":0},"user_value":{"type":"number"},"nested":{"properties":{"yield_time_ms":{"type":"number"}}},"max_output_tokens":{"type":["number","null","integer"]}}});
        normalize(&mut schema, "exec_command", Some("functions"));
        assert_eq!(
            schema["properties"]["yield_time_ms"],
            json!({"type":"integer","minimum":0})
        );
        assert_eq!(
            schema["properties"]["max_output_tokens"]["type"],
            json!(["integer", "null"])
        );
        assert_eq!(schema["properties"]["user_value"]["type"], "number");
        assert_eq!(
            schema["properties"]["nested"]["properties"]["yield_time_ms"]["type"],
            "number"
        );
        let mut custom = json!({"properties":{"yield_time_ms":{"type":"number"}}});
        normalize(&mut custom, "exec_command", Some("custom"));
        assert_eq!(custom["properties"]["yield_time_ms"]["type"], "number");
    }

    #[test]
    fn grok_nested_tool_integer_paths_match_namespaces() {
        let mut schema = json!({"properties":{"open":{"items":{"properties":{"lineno":{"type":"number"},"other":{"type":"number"}}}}}});
        normalize(&mut schema, "run", Some("web"));
        assert_eq!(
            schema.pointer("/properties/open/items/properties/lineno/type"),
            Some(&json!("integer"))
        );
        assert_eq!(
            schema.pointer("/properties/open/items/properties/other/type"),
            Some(&json!("number"))
        );
    }
}
