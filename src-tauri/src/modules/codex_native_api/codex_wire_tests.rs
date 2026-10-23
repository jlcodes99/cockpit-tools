use super::*;

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
        if case["websocket"].as_bool().unwrap() {
            normalize_websocket(&mut headers, &mut body, chatgpt).unwrap();
        } else {
            normalize(
                &mut headers,
                &mut body,
                chatgpt,
                case["compact"].as_bool().unwrap(),
            )
            .unwrap();
        }
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
