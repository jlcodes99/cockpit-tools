use super::super::{prepare, restore_event};
use super::*;

#[test]
fn reasoning_stream_uses_summary_events_and_preserves_indices_and_extensions() {
    let (_, mut context) = prepare(json!({"input":[]})).unwrap();
    let added = restore_event(&mut context, json!({"type":"response.content_part.added","item_id":"rs_1","output_index":3,"content_index":2,"part":{"type":"reasoning_text","text":"","vendor":true}})).unwrap();
    assert_eq!(added[0]["type"], "response.reasoning_summary_part.added");
    assert_eq!(
        added[0]["part"],
        json!({"type":"summary_text","text":"","vendor":true})
    );
    assert_eq!(added[0]["summary_index"], 2);
    let delta = restore_event(&mut context, json!({"type":"response.reasoning_text.delta","item_id":"rs_1","content_index":2,"delta":"think","vendor":5})).unwrap();
    assert_eq!(delta[0]["type"], "response.reasoning_summary_text.delta");
    assert_eq!(delta[0]["summary_index"], 2);
    assert_eq!(delta[0]["delta"], "think");
    assert_eq!(delta[0]["vendor"], 5);
    let done = restore_event(&mut context, json!({"type":"response.reasoning_text.done","content_index":2,"text":"think","item_id":"rs_1","output_index":3})).unwrap();
    assert_eq!(done.len(), 2);
    assert_eq!(done[0]["type"], "response.reasoning_summary_text.done");
    assert_eq!(done[0]["text"], "think");
    assert_eq!(done[1]["type"], "response.reasoning_summary_part.done");
    assert_eq!(
        done[1]["part"],
        json!({"type":"summary_text","text":"think"})
    );
    for event in done {
        assert_eq!(event["summary_index"], 2);
        assert!(event.get("content_index").is_none());
    }
}

#[test]
fn terminal_reasoning_summary_keeps_opaque_state_and_unknown_items() {
    let (_, mut context) = prepare(json!({})).unwrap();
    let reasoning = json!({"type":"reasoning","id":"rs_1","content":[{"type":"reasoning_text","text":"think","extension":"kept"}],"encrypted_content":"opaque","vendor":{"content":"keep"}});
    for event in [
        json!({"type":"response.output_item.done","item":reasoning}),
        json!({"output":[reasoning]}),
        json!({"type":"response.completed","response":{"output":[reasoning]}}),
    ] {
        let out = restore_event(&mut context, event).unwrap().remove(0);
        let item = out
            .get("item")
            .or_else(|| out.pointer("/output/0"))
            .or_else(|| out.pointer("/response/output/0"))
            .unwrap();
        assert_eq!(
            item["summary"],
            json!([{"type":"summary_text","text":"think","extension":"kept"}])
        );
        assert!(item.get("content").is_none());
        assert_eq!(item["encrypted_content"], "opaque");
        assert_eq!(item["vendor"]["content"], "keep");
    }
    let original = json!({"type":"response.content_part.added","content_index":0,"part":{"type":"output_text","text":"answer"}});
    assert_eq!(
        restore_event(&mut context, original.clone()).unwrap(),
        vec![original]
    );
}

#[test]
fn reasoning_conversion_does_not_discard_unknown_or_encrypted_parts() {
    let mut event = json!({"output":[{"type":"reasoning","summary":[{"type":"summary_text","text":"brief"}],"content":[{"type":"reasoning_text","text":"detail"},{"type":"encrypted_content","encrypted_content":"opaque-part"},{"type":"future","vendor":true}]}]});
    normalize(&mut event);
    assert_eq!(
        event["output"][0]["summary"],
        json!([{"type":"summary_text","text":"brief"},{"type":"summary_text","text":"detail"}])
    );
    assert_eq!(
        event["output"][0]["content"],
        json!([{"type":"encrypted_content","encrypted_content":"opaque-part"},{"type":"future","vendor":true}])
    );
    let mut done =
        json!({"type":"response.reasoning_text.done","text":"done","part":{"vendor":true}});
    normalize(&mut done);
    assert_eq!(
        done["part"],
        json!({"type":"summary_text","text":"done","vendor":true})
    );
}
