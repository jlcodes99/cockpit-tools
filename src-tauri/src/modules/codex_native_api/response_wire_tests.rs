use super::super::{bootstrap, codex_cpa_wire, codex_wire};
use super::*;
use axum::http::HeaderMap;

#[test]
fn explicit_parallel_false_survives_http_and_websocket() {
    for websocket in [false, true] {
        for (value, expected) in [
            (json!(false), false),
            (json!(true), true),
            (Value::Null, true),
        ] {
            let mut body = json!({"model":"m","input":[],"tools":[{"type":"function","name":"f"}],"parallel_tool_calls":value});
            if websocket {
                codex_wire::normalize_websocket(&mut HeaderMap::new(), &mut body, true).unwrap();
            } else {
                codex_wire::normalize(&mut HeaderMap::new(), &mut body, true, false).unwrap();
            }
            assert_eq!(body["parallel_tool_calls"], expected);
        }
    }
}

#[test]
fn tool_search_ids_are_portable_collision_free_and_idempotent() {
    let mut body = json!({"input":[
        {"type":"tool_search_call","id":"ts_x","call_id":"call-a"},
        {"type":"tool_search_call","id":"x","call_id":"call-b"},
        {"type":"tool_search_call","id":"tsc_x","call_id":"call-c"},
        {"type":"tool_search_call","id":"ts_x","call_id":"call-a"},
        {"type":"reasoning","id":"rs_original","encrypted_content":"cipher"}]});
    codex_cpa_wire::sanitize_input_ids(&mut body);
    assert_eq!(body["input"][0]["id"], body["input"][3]["id"]);
    for (a, b) in [(0, 1), (0, 2), (1, 2)] {
        assert_ne!(body["input"][a]["id"], body["input"][b]["id"]);
    }
    for i in 0..4 {
        assert!(body["input"][i]["id"].as_str().unwrap().starts_with("tsc_"));
    }
    assert_eq!(body["input"][0]["call_id"], "call-a");
    assert_eq!(body["input"][4]["encrypted_content"], "cipher");
    let again = body.clone();
    codex_cpa_wire::sanitize_input_ids(&mut body);
    assert_eq!(body, again);
}

#[test]
fn queued_and_empty_frames_keep_recovery_but_generated_content_disarms_it() {
    let frames = [
        json!({"type":"response.queued"}),
        json!({"type":"response.output_text.delta","delta":""}),
        json!({"type":"response.content_part.done","part":{"type":"output_text","text":""}}),
        json!({"type":"response.output_item.added","item":{"type":"function_call","arguments":""}}),
    ];
    for frame in frames {
        let mut fence = bootstrap::Fence::new(true);
        assert!(fence
            .observe(Some(&frame), frame.to_string().len(), false)
            .is_none());
        assert!(fence.active(), "{frame}");
        assert!(fence.observe(Some(&json!({"type":"response.failed","response":{"error":{"code":"server_is_overloaded"},"output":[]}})),100,false).is_some());
    }
    for frame in [
        json!({"type":"response.output_item.added","item":{"type":"function_call","arguments":"","output":[{"type":"message","content":"generated"}]}}),
        json!({"type":"response.content_part.added","part":{"type":"output_text","text":"","encrypted_content":"cipher"}}),
        json!({"type":"response.output_text.delta","delta":"generated"}),
        json!({"type":"response.output_text.delta","delta":"","usage":{"output_tokens":1}}),
        json!({"type":"response.output_item.added","item":{"type":"function_call","arguments":"{}"}}),
    ] {
        assert!(!bootstrap::handshake(&frame), "{frame}");
    }
}

#[test]
fn failure_uses_turn_identity_timestamp_and_sequence() {
    let mut wire = ResponseWire::default();
    let mut created = json!({"type":"response.created","sequence_number":12,"response":{"id":"resp_original","model":"m","created_at":123}});
    assert!(!wire.stamp_event(&mut created));
    let mut complete =
        json!({"type":"response.completed","response":{"id":"resp_original","output":[]}});
    assert!(wire.stamp_event(&mut complete));
    assert_eq!(complete["response"]["created_at"], 123);
    assert_eq!(complete["sequence_number"], 13);
    let failed = wire.failure(
        &super::super::contract::ServiceError::new(502, "upstream_error", "failure"),
        true,
        14,
    );
    assert_eq!(failed["response"]["id"], "resp_original");
    assert_eq!(failed["response"]["created_at"], 123);
    assert_eq!(failed["response"]["object"], "response");
    assert_eq!(failed["response"]["output"], json!([]));
    assert_eq!(failed["response"]["model"], "m");
    assert_eq!(failed["sequence_number"], 14);
}

#[test]
fn private_events_are_filtered_for_each_client_without_losing_errors() {
    for codex in [false, true] {
        for kind in [
            "responsesapi.timing",
            "response.subscription_usage",
            "codex.rate_limits",
        ] {
            assert!(!visible(&json!({"type":kind}), codex));
        }
        assert!(visible(&json!({"type":"response.failed"}), codex));
        assert_eq!(
            visible(&json!({"type":"codex.response.metadata"}), codex),
            codex
        );
        assert_eq!(visible(&json!({"type":"response.metadata"}), codex), codex);
        assert!(!visible_frame(
            b"event: responsesapi.timing\ndata: {\"type\":\"response.created\"}\n\n",
            codex
        ));
    }
}

#[test]
fn transient_admission_codes_do_not_override_explicit_quota_errors() {
    use super::super::transient_rate_limit::applies;
    for code in [
        "turn_rate_limit_exceeded",
        "model_at_capacity",
        "model_is_at_capacity",
    ] {
        for event in [
            json!({"error":{"code":code}}),
            json!({"body":{"error":{"code":code}}}),
            json!({"response":{"error":{"code":code}}}),
        ] {
            assert!(applies(&event));
        }
    }
    assert!(applies(
        &json!({"error":{"type":"rate_limit_error","message":"Service is at capacity"}})
    ));
    for code in [
        "usage_limit_reached",
        "rate_limit_exceeded",
        "invalid_token",
    ] {
        assert!(!applies(
            &json!({"error":{"code":code,"type":"rate_limit_error","message":"Service is at capacity"}})
        ));
    }
    let error = super::super::upstream_errors::chat_failure(
        &json!({"type":"error","error":{"code":"turn_rate_limit_exceeded"}}),
        502,
    )
    .unwrap();
    assert_eq!(error.status, 429);
    assert!(error.transient_rate_limit);
    assert!(!error.credential_scope);
}

#[test]
fn repairs_keep_sse_framing_and_do_not_rewrite_metadata() {
    let event = json!({"type":"response.created","response":{"id":"r","created_at":123}});
    let data = super::rewrite_frame(b"data: {}\n\n", &event);
    assert!(data.starts_with(b"data: "));
    assert!(!data.windows(6).any(|bytes| bytes == b"event:"));
    let framed = super::rewrite_frame(
        b": comment\r\nid: 8\r\nevent: response.created\r\ndata: {}\r\n\r\n",
        &event,
    );
    assert!(framed.starts_with(b": comment\r\nid: 8\r\nevent: response.created\r\n"));
    let mut wire = ResponseWire::default();
    let mut metadata = json!({"type":"codex.response.metadata","response":{"opaque":"unchanged"},"sequence_number":9});
    let original = metadata.clone();
    assert!(!wire.stamp_event(&mut metadata));
    assert_eq!(metadata, original);
    let mut delta =
        json!({"type":"response.output_text.delta","sequence_number":20,"delta":"text"});
    assert!(!wire.stamp_event(&mut delta));
    let mut terminal =
        json!({"type":"response.completed","response":{"id":"r","status":"completed","output":[]}});
    wire.stamp_event(&mut terminal);
    assert_eq!(terminal["sequence_number"], 21);
}
