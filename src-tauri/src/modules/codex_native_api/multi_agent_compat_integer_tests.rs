use super::*;
use axum::http::HeaderValue;
use serde_json::json;

fn headers(ua: &str) -> HeaderMap {
    HeaderMap::from_iter([(
        "user-agent".parse().unwrap(),
        HeaderValue::from_str(ua).unwrap(),
    )])
}

fn schema(path: &str) -> Value {
    let mut value = json!({"type":"number","minimum":0});
    for key in path.split('.').rev() {
        value = if key == "0" {
            json!([value])
        } else {
            json!({key:value})
        };
    }
    json!({"type":"object","properties":value})
}

#[test]
fn cpa_non_codex_target_integer_map_covers_every_upstream_tool_and_field() {
    let cases: &[(&str, &[&str])] = &[
        (
            "exec_command",
            &["yield_time_ms", "max_output_tokens", "timeout_ms"],
        ),
        (
            "write_stdin",
            &["session_id", "yield_time_ms", "max_output_tokens"],
        ),
        ("sleep", &["duration_ms"]),
        ("wait_agent", &["timeout_ms"]),
        ("wait", &["yield_time_ms", "max_tokens"]),
        ("tool_search", &["limit"]),
        (
            "test_sync_tool",
            &[
                "sleep_before_ms",
                "sleep_after_ms",
                "participants",
                "timeout_ms",
                "barrier.properties.participants",
                "barrier.properties.timeout_ms",
            ],
        ),
        ("create_goal", &["token_budget"]),
        ("get_channels", &["limit"]),
        ("list_threads", &["limit", "max_chars_per_post"]),
        ("search_posts", &["limit", "max_chars_per_post"]),
        ("read_thread", &["limit", "max_chars_per_post"]),
        ("read_post", &["offset_chars", "limit_chars"]),
        ("memories__list", &["max_results"]),
        ("memories__read", &["line_offset", "max_lines"]),
        ("memories__search", &["context_lines", "max_results"]),
        ("history__list_windows", &["limit"]),
        ("history__search_contents", &["limit"]),
        ("history__list_items", &["limit", "max_chars_per_item"]),
        ("history__read_item", &["offset_chars", "limit_chars"]),
        ("notes__list_files_by_prefix", &["max_results"]),
        (
            "notes__read_file",
            &[
                "start_line",
                "stop_line",
                "start_line.anyOf.0",
                "stop_line.anyOf.0",
            ],
        ),
        (
            "notes__search_contents",
            &["max_matches_per_file", "max_files"],
        ),
        ("image_gen__imagegen", &["num_last_images_to_include"]),
        (
            "web__run",
            &[
                "search_query.items.properties.recency",
                "image_query.items.properties.recency",
                "open.items.properties.lineno",
                "click.items.properties.id",
                "screenshot.items.properties.pageno",
                "weather.items.properties.duration",
                "sports.items.properties.num_games",
            ],
        ),
    ];
    for (name, paths) in cases {
        for path in *paths {
            let mut body =
                json!({"tools":[{"type":"function","name":name,"parameters":schema(path)}]});
            normalize_request(&headers("codex-desktop/0.159.0"), &mut body);
            let pointer = format!(
                "/tools/0/parameters/properties/{}/type",
                path.replace('.', "/")
            );
            assert_eq!(
                body.pointer(&pointer),
                Some(&json!("integer")),
                "{name} {path}"
            );
            assert_eq!(
                body.pointer(&pointer.replace("/type", "/minimum")),
                Some(&json!(0))
            );
        }
    }
}

#[test]
fn cpa_integer_ua_gate_is_broader_than_optimization_and_remains_case_insensitive() {
    let original = json!({"tools":[{"type":"function","name":"exec_command","parameters":schema("timeout_ms")}]});
    for ua in [
        "Codex Desktop/1",
        "custom CODEX bridge/1",
        "codex-desktop/1",
    ] {
        let mut body = original.clone();
        normalize_request(&headers(ua), &mut body);
        assert_eq!(
            body["tools"][0]["parameters"]["properties"]["timeout_ms"]["type"],
            "integer"
        );
    }
    for ua in ["", "curl/8", "OpenAI-Python/1"] {
        let mut body = original.clone();
        normalize_request(&headers(ua), &mut body);
        assert_eq!(body, original);
    }
}

#[test]
fn cpa_integer_all_supported_tool_shapes_and_additional_tools_are_normalized() {
    let mut body = json!({"tools":[
        {"type":"function","function":{"name":"exec_command","parameters":schema("yield_time_ms")}},
        {"name":"exec_command","input_schema":schema("yield_time_ms")},
        {"name":"exec_command","parametersJsonSchema":schema("yield_time_ms")},
        {"function_declarations":[{"name":"exec_command","parameters":schema("yield_time_ms")}]},
        {"functionDeclarations":[{"name":"exec_command","parametersJsonSchema":schema("yield_time_ms")}]},
        {"type":"namespace","name":"functions","tools":[{"type":"function","name":"exec_command","parameters":schema("yield_time_ms")}]}
    ],"input":[{"type":"additional_tools","tools":[{"type":"namespace","name":"collaboration","tools":[{"type":"function","name":"wait_agent","parameters":schema("timeout_ms")}]}]}]});
    normalize_request(&headers("Codex Desktop/1"), &mut body);
    for prefix in [
        "/tools/0/function/parameters",
        "/tools/1/input_schema",
        "/tools/2/parametersJsonSchema",
        "/tools/3/function_declarations/0/parameters",
        "/tools/4/functionDeclarations/0/parametersJsonSchema",
        "/tools/5/tools/0/parameters",
    ] {
        assert_eq!(
            body.pointer(&format!("{prefix}/properties/yield_time_ms/type")),
            Some(&json!("integer"))
        );
    }
    assert_eq!(
        body.pointer("/input/0/tools/0/tools/0/parameters/properties/timeout_ms/type"),
        Some(&json!("integer"))
    );
}

#[test]
fn cpa_integer_namespace_aliases_are_exact_and_unrelated_schema_numbers_stay_unchanged() {
    for (name, expected) in [
        ("functions__wait_agent", true),
        ("collab__wait_agent", true),
        ("collaboration__wait_agent", true),
        ("multi_agent_v1__wait_agent", true),
        ("mcp__server__wait_agent", false),
        ("custom__wait_agent", false),
        ("collaboration__exec_command", false),
    ] {
        let path = "timeout_ms";
        let mut body = json!({"tools":[{"type":"function","name":name,"parameters":schema(path)}]});
        normalize_request(&headers("codex_cli_rs"), &mut body);
        assert_eq!(
            body["tools"][0]["parameters"]["properties"][path]["type"],
            if expected { "integer" } else { "number" },
            "{name}"
        );
    }
    let mut nested = json!({"tools":[{"type":"namespace","name":"outer","tools":[{"type":"namespace","name":"functions","tools":[{"name":"wait_agent","parameters":schema("timeout_ms")}]}]}]});
    let original = nested.clone();
    normalize_request(&headers("codex_cli_rs"), &mut nested);
    assert_eq!(nested, original);
}

#[test]
fn cpa_integer_nullable_arrays_deduplicate_only_after_number_replacement() {
    let mut body = json!({"tools":[{"name":"wait_agent","parameters":{"properties":{"timeout_ms":{"type":["number","null","integer","null"]}}}}]});
    normalize_request(&headers("codex_cli_rs"), &mut body);
    assert_eq!(
        body["tools"][0]["parameters"]["properties"]["timeout_ms"]["type"],
        json!(["integer", "null"])
    );
    body["tools"][0]["parameters"]["properties"]["timeout_ms"]["type"] =
        json!(["integer", "integer", "null"]);
    let original = body.clone();
    normalize_request(&headers("codex_cli_rs"), &mut body);
    assert_eq!(body, original);
}

#[test]
fn cpa_integer_normalizer_does_not_traverse_argument_or_result_payloads() {
    let nested = json!({"tools":[{"name":"wait_agent","parameters":schema("timeout_ms")}]});
    let mut body = json!({"input":[{"type":"function_call","arguments":nested.clone()},{"type":"function_call_output","output":nested.clone()}],"tool_choice":nested});
    let original = body.clone();
    normalize_request(&headers("codex_cli_rs"), &mut body);
    assert_eq!(body, original);
}
