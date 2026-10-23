use super::*;
use axum::http::HeaderValue;

fn prepared(mut body: Value, headers: &mut HeaderMap) -> Value {
    prepare(headers, &mut body, false).unwrap();
    body
}

#[test]
fn plan_function_schemas_receive_cpa_cleanup_before_additional_tools_projection() {
    let body = prepared(
        json!({"model":"fixture","input":[],"tools":[{"type":"function","name":"inspect","parameters":{"type":"object","properties":{
            "path":{"type":"string","pattern":r"\0"},
            "count":{"type":"integer","maximum":18446744073709551615u64},
            "user_data":{"default":{"maximum":18446744073709551615u64}}
        }}}]}),
        &mut HeaderMap::new(),
    );
    let tool = &body["input"][0]["tools"][0];
    assert!(tool["parameters"]["properties"]["path"]
        .get("pattern")
        .is_none());
    assert!(tool["parameters"]["properties"]["count"]
        .get("maximum")
        .is_none());
    assert_eq!(
        tool["parameters"]["properties"]["user_data"]["default"]["maximum"],
        json!(18446744073709551615u64)
    );
}

#[test]
fn plan_parallel_tool_calls_uses_http_rules_without_inserting_backend_defaults() {
    for tools in [None, Some(json!([]))] {
        let mut input = json!({"input":[],"parallel_tool_calls":true});
        if let Some(tools) = tools {
            input["tools"] = tools;
        }
        let body = prepared(input, &mut HeaderMap::new());
        assert!(body.get("parallel_tool_calls").is_none());
    }
    for parallel in [None, Some(json!(false)), Some(json!(true))] {
        let mut body = json!({"input":[],"tools":[{"type":"function","name":"read","parameters":{"type":"object"}}]});
        if let Some(value) = &parallel {
            body["parallel_tool_calls"] = value.clone();
        }
        let body = prepared(body, &mut HeaderMap::new());
        assert_eq!(body.get("parallel_tool_calls"), parallel.as_ref());
    }
}

#[test]
fn plan_lite_parallel_gate_is_read_before_public_header_and_metadata_sanitization() {
    for metadata in [true, false] {
        let mut headers = HeaderMap::new();
        let mut body = json!({"input":[],"tools":[],"parallel_tool_calls":true});
        if metadata {
            body["client_metadata"] =
                json!({"ws_request_header_x_openai_internal_codex_responses_lite":" True "});
        } else {
            headers.insert(
                super::super::codex_wire::LITE_HEADER,
                HeaderValue::from_static("true"),
            );
        }
        let body = prepared(body, &mut headers);
        assert_eq!(body["parallel_tool_calls"], false);
        assert!(body.get("client_metadata").is_none());
        assert!(!headers.contains_key(super::super::codex_wire::LITE_HEADER));
    }
}

#[test]
fn plan_lite_never_removes_unsupported_hosted_tools_to_hide_validation_failures() {
    let mut headers = HeaderMap::from_iter([(
        super::super::codex_wire::LITE_HEADER.parse().unwrap(),
        HeaderValue::from_static("true"),
    )]);
    let mut body = json!({"input":[],"tools":[{"type":"code_interpreter"}]});
    let original = body.clone();
    assert_eq!(
        prepare(&mut headers, &mut body, false).unwrap_err().code,
        "subscription_sharing_unsupported_capability"
    );
    assert_eq!(body, original);
}
