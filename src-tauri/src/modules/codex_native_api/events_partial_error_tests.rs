use super::*;
use serde_json::json;

fn wire(value: &Value) -> Vec<u8> {
    format!("data: {value}\n\n").into_bytes()
}

#[test]
fn complete_text_and_tool_frames_survive_a_later_invalid_frame_in_the_same_chunk() {
    let text = json!({"type":"response.output_text.delta","output_index":0,"delta":"kept"});
    let tool = json!({"type":"response.output_item.done","output_index":1,"item":{"id":"tool","type":"function_call","call_id":"c","name":"run","arguments":"{}","encrypted_function_args":["preserved-cipher"]}});
    let mut decoder = SseDecoder::new(4096);
    let mut chunk = wire(&text);
    chunk.extend(wire(&tool));
    chunk.extend_from_slice(b"data: {invalid}\n\n");
    let frames = decoder.push_frames(&chunk).unwrap();
    assert_eq!(frames.len(), 3);
    assert_eq!(frames[0].value.as_ref(), Some(&text));
    assert_eq!(frames[1].value.as_ref(), Some(&tool));
    assert_eq!(frames[0].bytes.as_ref(), wire(&text));
    assert_eq!(frames[1].bytes.as_ref(), wire(&tool));
    let failure = frames[2].value.as_ref().unwrap();
    assert_eq!(failure["type"], "error");
    assert_eq!(failure["error"]["code"], "invalid_upstream_event");
    assert!(failure.get("sequence_number").is_none());
    assert!(!decoder.has_partial_event());
    assert!(decoder
        .push_frames(b"data: {\"type\":\"response.completed\"}\n\n")
        .unwrap()
        .is_empty());
    assert!(decoder.finish_frames().unwrap().is_empty());
}

#[test]
fn complete_frame_survives_following_oversized_frame_without_waiting_for_more_bytes() {
    let text = json!({"type":"response.output_text.delta","delta":"kept"});
    let mut decoder = SseDecoder::new(128);
    let mut chunk = wire(&text);
    chunk.extend_from_slice(b"data: ");
    chunk.extend(std::iter::repeat_n(b'x', 256));
    let frames = decoder.push_frames(&chunk).unwrap();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].value.as_ref(), Some(&text));
    assert_eq!(
        frames[1].value.as_ref().unwrap()["error"]["code"],
        "upstream_event_too_large"
    );
    assert!(decoder.finish_frames().unwrap().is_empty());
    assert!(!decoder.has_partial_event());
}

#[test]
fn malformed_or_oversized_first_data_frame_remains_an_error() {
    for chunk in [
        b"data: {invalid}\n\n".to_vec(),
        format!(": keep-alive\n\ndata: {}", "x".repeat(256)).into_bytes(),
    ] {
        let mut decoder = SseDecoder::new(128);
        assert!(decoder.push_frames(&chunk).is_err());
    }
}

#[test]
fn malformed_completion_is_not_promoted_to_success_after_valid_output() {
    let mut decoder = SseDecoder::new(4096);
    let mut chunk = wire(&json!({"type":"response.output_text.delta","delta":"kept"}));
    chunk.extend(wire(
        &json!({"type":"response.completed","response":{"status":"completed","output":[]}}),
    ));
    let frames = decoder.push_frames(&chunk).unwrap();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[1].value.as_ref().unwrap()["type"], "error");
    assert_eq!(
        frames[1].value.as_ref().unwrap()["error"]["code"],
        "invalid_upstream_event"
    );
    assert!(decoder.finish_frames().unwrap().is_empty());
}
