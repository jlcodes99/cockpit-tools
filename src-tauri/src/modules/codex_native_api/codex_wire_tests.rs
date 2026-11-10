use super::*;

#[test]
fn agent_browser_host_results_never_consume_pending_tool_ids_on_http_or_websocket_wire() {
    for websocket in [false, true] {
        for empty_id in [false, true] {
            let mut open = json!({"type":"function_call_output","namespace":"agent",
                "name":"codex_apps_open_page","output":"Synthetic page\n  preserve whitespace  "});
            let mut selected = json!({"type":"function_call_output","namespace":"agent",
                "name":"codex_apps_selected_page_part_1_of_1","output":"Synthetic selection"});
            if empty_id {
                open["call_id"] = json!("");
                selected["call_id"] = json!("");
            }
            let input = json!([
                {"type":"function_call","name":"read_file","call_id":"real-function","arguments":"{}"},
                open,
                {"type":"custom_tool_call","name":"custom_read","call_id":"real-custom","input":"fixture"},
                selected,
                {"type":"function_call_output","call_id":"real-function","output":"Original function result"},
                {"type":"custom_tool_call_output","call_id":"real-custom","output":"Original custom result"}
            ]);
            let mut body = json!({"model":"fixture-model","input":input.clone()});
            let mut headers = HeaderMap::new();
            if websocket {
                normalize_websocket(&mut headers, &mut body, true).unwrap();
            } else {
                normalize(&mut headers, &mut body, true, false).unwrap();
            }
            assert_eq!(
                body["input"], input,
                "websocket={websocket}, empty_id={empty_id}"
            );
        }
    }
}

#[test]
fn agent_browser_host_malformed_ids_remain_untouched_for_identity_validation() {
    for name in [
        "codex_apps_open_page",
        "codex_apps_selected_page_part_1_of_1",
    ] {
        for id in [Value::Null, json!(42), json!(" ")] {
            let input = json!([
                {"type":"function_call","name":"read_file","call_id":"real-call","arguments":"{}"},
                {"type":"function_call_output","namespace":"agent","name":name,"call_id":id,"output":"Synthetic host observation"},
                {"type":"function_call_output","call_id":"real-call","output":"Original ordinary result"}
            ]);
            let mut body = json!({"model":"fixture-model","input":input.clone()});
            normalize(&mut HeaderMap::new(), &mut body, true, false).unwrap();
            assert_eq!(
                body["input"], input,
                "The wire layer must not turn a malformed host ID into an unrelated real ID"
            );
            assert!(
                !crate::modules::codex_native_api::identity::rebuild::paired_tools(
                    body["input"].as_array().unwrap()
                )
            );
        }
    }
}

#[test]
fn matches_shared_go_golden_requests_and_headers() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/codex_wire_go.json")).unwrap();
    for case in cases {
        let mut headers = HeaderMap::new();
        for (name, values) in case["headers"].as_object().unwrap() {
            for value in values.as_array().unwrap() {
                headers.append(
                    name.parse::<axum::http::HeaderName>().unwrap(),
                    value.as_str().unwrap().parse().unwrap(),
                );
            }
        }
        let mut body = case["input"].clone();
        let chatgpt = case["chatgpt"].as_bool().unwrap();
        let outcome = if case["websocket"].as_bool().unwrap() {
            normalize_websocket(&mut headers, &mut body, chatgpt)
        } else {
            normalize(&mut headers,&mut body,chatgpt,case["compact"].as_bool().unwrap())
        };
        if let Some(status) = case["expected_error"].as_u64() {
            assert_eq!(outcome.unwrap_err().status,u16::try_from(status).unwrap(),"{}",case["name"]);
            assert_eq!(body,case["input"],"Rejected native requests retain the caller body");
            continue;
        }
        outcome.unwrap();
        assert_eq!(body, case["expected"], "{}", case["name"]);
        let mut expected = HeaderMap::new();
        for (name, values) in case["expected_headers"].as_object().unwrap() {
            for value in values.as_array().unwrap() {
                expected.append(
                    name.parse::<axum::http::HeaderName>().unwrap(),
                    value.as_str().unwrap().parse().unwrap(),
                );
            }
        }
        assert_eq!(headers, expected, "{} headers", case["name"]);
    }
}

#[test]
fn invalid_body_is_rejected_without_rewriting_headers() {
    let mut headers = HeaderMap::new();
    headers.insert(LITE_HEADER, "true".parse().unwrap());
    let mut body = json!([]);
    assert_eq!(
        normalize(&mut headers, &mut body, true, false)
            .unwrap_err()
            .status,
        400
    );
    assert!(headers.contains_key(LITE_HEADER));
}

#[test]
fn normalization_is_idempotent() {
    let mut body = json!({"model":"fixture","input":[{"type":"function_call","name":"f"},{"type":"function_call_output","name":"f","output":"raw"}],"tools":[{"type":"web_search_preview"}]});
    let mut headers = HeaderMap::new();
    normalize(&mut headers, &mut body, true, false).unwrap();
    let once = body.clone();
    normalize(&mut headers, &mut body, true, false).unwrap();
    assert_eq!(body, once);
}
