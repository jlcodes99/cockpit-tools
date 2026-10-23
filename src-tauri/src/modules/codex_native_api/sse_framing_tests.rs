use super::*;
use serde_json::json;

fn event(value: &Value) -> Vec<u8> {
    format!("data: {value}\n\n").into_bytes()
}
fn completion() -> Value {
    json!({"type":"response.completed","response":{"id":"r","status":"completed","output":[],"extension":{"keep":true}}})
}

#[test]
fn complete_wire_fields_and_unknown_values_are_preserved() {
    let value = json!({"type":"response.created","response":{"id":"r","status":"in_progress","extension":17}});
    let bytes = format!(": comment\r\nid: 9\r\nretry: 50\r\ncustom: value\r\nevent: response.created\r\ndata: {value}\r\n\r\n");
    let mut decoder = SseDecoder::new(4096);
    let frames = decoder.push_frames(bytes.as_bytes()).unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].value, Some(value));
    assert_eq!(frames[0].bytes.as_ref(), bytes.as_bytes());
}

#[test]
fn split_json_and_missing_field_newline_are_repaired_without_replay() {
    let mut decoder = SseDecoder::new(4096);
    assert!(decoder
        .push_frames(b"event: response.completed")
        .unwrap()
        .is_empty());
    assert!(decoder
        .push_frames(b"data: {\"response\":{\"id\":\"r\",")
        .unwrap()
        .is_empty());
    let frames = decoder
        .push_frames(b"\"status\":\"completed\",\"output\":[]},\"extension\":true}")
        .unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(
        frames[0].value.as_ref().unwrap()["type"],
        "response.completed"
    );
    assert_eq!(frames[0].value.as_ref().unwrap()["extension"], true);
    let mut reader = SseDecoder::new(4096);
    assert_eq!(
        reader.push(&frames[0].bytes).unwrap()[0],
        *frames[0].value.as_ref().unwrap()
    );
    assert!(decoder.push_frames(b"\n\n").unwrap().is_empty());
}

#[test]
fn data_only_valid_eof_tail_is_flushed_and_truncated_json_is_rejected() {
    let bytes = format!("data: {}", completion());
    let mut decoder = SseDecoder::new(4096);
    assert!(decoder.push_frames(bytes.as_bytes()).unwrap().is_empty());
    let frames = decoder.finish_frames().unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].value, Some(completion()));
    assert!(frames[0].bytes.ends_with(b"\n\n"));
    assert!(!decoder.has_partial_event());
    assert!(decoder.finish_frames().unwrap().is_empty());
    let mut decoder = SseDecoder::new(4096);
    decoder
        .push_frames(b"data: {\"type\":\"response.completed\"")
        .unwrap();
    assert_eq!(
        decoder.finish_frames().unwrap_err().code,
        "incomplete_upstream_stream"
    );
}

#[test]
fn consecutive_data_chunks_missing_delimiters_remain_distinct_events() {
    let mut decoder = SseDecoder::new(4096);
    let first = json!({"type":"response.created","response":{"id":"r"}});
    assert!(decoder
        .push_frames(format!("data: {first}").as_bytes())
        .unwrap()
        .is_empty());
    let frames = decoder
        .push_frames(format!("data: {}", completion()).as_bytes())
        .unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].value, Some(first));
    assert_eq!(decoder.finish().unwrap(), vec![completion()]);
}

#[test]
fn data_only_object_lines_missing_blank_separators_are_chunk_independent() {
    for newline in ["\n", "\r\n"] {
        let expected = vec![
            json!({"type":"response.created","response":{"id":"r","status":"in_progress"}}),
            json!({"type":"response.output_text.delta","delta":"中文 data: kept"}),
            completion(),
        ];
        let wire = expected
            .iter()
            .map(|value| format!("data: {value}{newline}"))
            .collect::<String>()
            .into_bytes();
        // Cover every possible TCP split, including field prefixes, UTF-8,
        // escaped JSON and CRLF, plus a fully coalesced transport chunk.
        for split in 0..=wire.len() {
            let mut decoder = SseDecoder::new(4096);
            let mut frames = decoder.push_frames(&wire[..split]).unwrap_or_else(|error| {
                panic!("newline={newline:?} split={split} first chunk: {error:?}")
            });
            frames.extend(decoder.push_frames(&wire[split..]).unwrap_or_else(|error| {
                panic!("newline={newline:?} split={split} second chunk: {error:?}")
            }));
            frames.extend(decoder.finish_frames().unwrap_or_else(|error| {
                panic!("newline={newline:?} split={split} finish: {error:?}")
            }));
            assert_eq!(
                frames
                    .iter()
                    .filter_map(|frame| frame.value.clone())
                    .collect::<Vec<_>>(),
                expected,
                "split={split}"
            );
            let delivered = frames
                .into_iter()
                .flat_map(|frame| frame.bytes.to_vec())
                .collect::<Vec<_>>();
            let mut standard_reader = SseDecoder::new(4096);
            assert_eq!(standard_reader.push(&delivered).unwrap(), expected);
        }
        let mut decoder = SseDecoder::new(4096);
        let mut output = Vec::new();
        for byte in &wire {
            output.extend(decoder.push(std::slice::from_ref(byte)).unwrap());
        }
        output.extend(decoder.finish().unwrap());
        assert_eq!(output, expected);
    }
}

#[test]
fn early_event_header_flush_resets_independent_data_scanner_for_following_events() {
    let mut decoder = SseDecoder::new(4096);
    let created = json!({"type":"response.created","response":{"id":"r","status":"in_progress"}});
    let first = decoder
        .push_frames(format!("event: response.created\ndata: {created}").as_bytes())
        .unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].value, Some(created));
    assert!(!decoder.data_lines.disabled);
    assert_eq!(decoder.data_lines.scan_from, 0);
    let delta = json!({"type":"response.output_text.delta","delta":"kept after header"});
    let following = format!("data: {delta}\ndata: {}\n", completion());
    let mut frames = decoder.push_frames(following.as_bytes()).unwrap();
    frames.extend(decoder.finish_frames().unwrap());
    assert_eq!(
        frames
            .iter()
            .filter_map(|frame| frame.value.clone())
            .collect::<Vec<_>>(),
        vec![delta, completion()]
    );
    assert!(decoder.terminal);
    assert!(decoder.finish_frames().unwrap().is_empty());
}

#[test]
fn explicit_done_data_line_is_chunk_independent_and_never_invents_completion() {
    for newline in ["\n", "\r\n"] {
        for event in [
            completion(),
            json!({"type":"response.output_text.delta","delta":"partial only"}),
        ] {
            let wire = format!("data: {event}{newline}data: [DONE]{newline}{newline}").into_bytes();
            for split in 0..=wire.len() {
                let mut decoder = SseDecoder::new(4096);
                let mut frames = decoder.push_frames(&wire[..split]).unwrap();
                frames.extend(decoder.push_frames(&wire[split..]).unwrap());
                frames.extend(decoder.finish_frames().unwrap());
                let values = frames
                    .iter()
                    .filter_map(|frame| frame.value.clone())
                    .collect::<Vec<_>>();
                assert_eq!(values, vec![event.clone()], "split={split}");
                let mut observation = ResponseObservation::default();
                for value in values {
                    observation.observe(&value);
                }
                assert_eq!(observation.succeeded, event["type"] == "response.completed");
                if event["type"] != "response.completed" {
                    assert!(decoder.done);
                    assert!(!observation.terminal);
                    assert!(observation.response.is_none());
                }
            }
        }
    }
}

#[test]
fn multiline_json_and_event_metadata_are_not_split_into_fake_events() {
    let wire = concat!(
        ": comment\nid: 9\nretry: 50\nevent: response.created\n",
        "data: {\"type\":\"response.created\",\n",
        ": inside data field\n",
        "data: \"response\":{\"id\":\"r\",\"status\":\"in_progress\"}}\n\n",
    );
    let expected = json!({"type":"response.created","response":{"id":"r","status":"in_progress"}});
    for split in 0..=wire.len() {
        let mut decoder = SseDecoder::new(4096);
        let mut frames = decoder
            .push_frames(&wire.as_bytes()[..split])
            .unwrap_or_else(|error| panic!("split={split} first chunk: {error:?}"));
        frames.extend(
            decoder
                .push_frames(&wire.as_bytes()[split..])
                .unwrap_or_else(|error| panic!("split={split} second chunk: {error:?}")),
        );
        assert_eq!(
            frames
                .iter()
                .filter_map(|frame| frame.value.clone())
                .collect::<Vec<_>>(),
            vec![expected.clone()],
            "split={split}"
        );
        let wire_bytes = frames
            .iter()
            .flat_map(|frame| frame.bytes.iter().copied())
            .collect::<Vec<_>>();
        let delivered = std::str::from_utf8(&wire_bytes).unwrap();
        for preserved in [
            ": comment\n",
            "id: 9\n",
            "retry: 50\n",
            ": inside data field\n",
        ] {
            assert!(delivered.contains(preserved), "split={split}");
        }
    }
}

#[test]
fn independent_data_scanner_visits_large_fragmented_lines_once() {
    let large = json!({"type":"response.output_text.delta","delta":"x".repeat(256*1024)});
    let wire = format!("data: {large}\ndata: {}\n", completion()).into_bytes();
    let mut scanner = IndependentDataLines::default();
    let mut expected = None;
    for end in (31..wire.len())
        .step_by(31)
        .chain(std::iter::once(wire.len()))
    {
        expected = scanner.boundary(&wire[..end], false);
    }
    assert!(expected.is_some());
    assert_eq!(scanner.scanned_bytes, wire.len());
    assert_eq!(scanner.parsed_lines, 2);
}

#[test]
fn independent_data_events_keep_individual_limits_with_bounded_lookahead() {
    let values = [
        json!({"type":"response.output_text.delta","delta":"a".repeat(200)}),
        json!({"type":"response.output_text.delta","delta":"b".repeat(200)}),
        completion(),
    ];
    let wire = values
        .iter()
        .map(|value| format!("data: {value}\n"))
        .collect::<String>();
    let mut decoder = SseDecoder::new(300);
    let mut output = decoder.push(wire.as_bytes()).unwrap();
    output.extend(decoder.finish().unwrap());
    assert_eq!(output, values);
}

#[test]
fn independent_data_repair_never_hides_invalid_or_oversized_data_lines() {
    for wire in [
        format!("data: {{invalid}}\ndata: {}\n\n", completion()),
        format!("data: []\ndata: {}\n\n", completion()),
        format!(
            "data: {{\"extension\":1}}\ndata: {{\"large\":\"{}\"}}\n\n",
            "x".repeat(1024)
        ),
    ] {
        let mut decoder = SseDecoder::new(512);
        let error = decoder.push_frames(wire.as_bytes()).unwrap_err();
        assert!(matches!(
            error.code,
            "invalid_upstream_event" | "upstream_event_too_large"
        ));
        assert!(!decoder.terminal);
    }
}

#[test]
fn many_small_events_in_one_transport_chunk_respect_per_event_limit() {
    let bytes = b"data: {\"extension\":1}\n\n".repeat(20);
    let mut decoder = SseDecoder::new(32);
    let frames = decoder.push_frames(&bytes).unwrap();
    assert_eq!(frames.len(), 20);
    let output: Vec<u8> = frames
        .into_iter()
        .flat_map(|frame| frame.bytes.to_vec())
        .collect();
    assert_eq!(output, bytes);
    let mut decoder = SseDecoder::new(32);
    assert_eq!(
        decoder
            .push_frames(b"data: {\"extension\":\"abcdefghijklmnopqrstuvwxyz\"}\n\n")
            .unwrap_err()
            .code,
        "upstream_event_too_large"
    );
}

#[test]
fn output_items_restore_completed_output_in_index_order_and_preserve_existing_output() {
    for existing in [false, true] {
        let mut decoder = SseDecoder::new(4096);
        for (index, text) in [(2, "later"), (0, "earlier")] {
            decoder.push(&event(&json!({"type":"response.output_item.done","output_index":index,"item":{"type":"message","text":text,"extension":index}}))).unwrap();
        }
        let mut completed = completion();
        if existing {
            completed["response"]["output"] = json!([{"type":"message","text":"canonical"}]);
        }
        let frames = decoder.push_frames(&event(&completed)).unwrap();
        let response = &frames[0].value.as_ref().unwrap()["response"];
        if existing {
            assert_eq!(response["output"][0]["text"], "canonical");
        } else {
            assert_eq!(response["output"][0]["text"], "earlier");
            assert_eq!(response["output"][1]["text"], "later");
            assert_eq!(response["output"][1]["extension"], 2);
        }
        assert_eq!(response["extension"]["keep"], true);
        let mut reader = SseDecoder::new(4096);
        assert_eq!(
            reader.push(&frames[0].bytes).unwrap()[0],
            *frames[0].value.as_ref().unwrap()
        );
    }
}

#[test]
fn output_cache_and_repaired_terminal_follow_go_without_an_aggregate_cap() {
    let mut decoder = SseDecoder::new(160);
    for index in 0..4 {
        let item = json!({"type":"response.output_item.done","output_index":index,"item":{"type":"message","content":"abcdefghijklmnopqrstuvwxyz"}});
        assert_eq!(decoder.push(&event(&item)).unwrap().len(), 1);
    }
    let frames = decoder.push_frames(&event(&completion())).unwrap();
    let terminal = frames.last().unwrap();
    assert_eq!(terminal.value.as_ref().unwrap()["response"]["output"].as_array().unwrap().len(), 4);
    assert!(terminal.bytes.len() > 160);
}

#[test]
fn explicit_failure_wins_over_success_event_and_later_completion() {
    for bytes in [
        format!("event: response.failed\ndata: {}\n\n", completion()),
        "event: response.completed\ndata: {\"type\":\"error\",\"error\":{\"code\":\"invalid_token\"}}\n\n".into(),
        "event: response.completed\ndata: {\"response\":{\"id\":\"r\",\"status\":\"completed\",\"error\":{\"code\":\"server_error\"}}}\n\n".into(),
    ] {
        let mut decoder = SseDecoder::new(4096);
        let mut payload = bytes.into_bytes(); payload.extend(event(&completion()));
        let frames = decoder.push_frames(&payload).unwrap();
        assert_eq!(frames.len(), 1);
        let mut observation = ResponseObservation::default(); observation.observe(frames[0].value.as_ref().unwrap());
        assert!(observation.terminal);
        assert!(!observation.succeeded);
        assert!(decoder.push(&event(&completion())).unwrap().is_empty());
    }
}

#[test]
fn completed_frames_require_real_id_and_completed_status() {
    for response in [
        json!({"status":"completed"}),
        json!({"id":"r"}),
        json!({"id":" ","status":"completed"}),
    ] {
        let value = json!({"type":"response.completed","response":response});
        let mut decoder = SseDecoder::new(4096);
        assert_eq!(
            decoder.push(&event(&value)).unwrap_err().code,
            "invalid_upstream_event"
        );
        let mut observation = ResponseObservation::default();
        observation.observe(&value);
        assert!(!observation.succeeded);
    }
    // A failure status is a valid terminal frame even when its event type says
    // completed. It must stop framing and checkpoint publication, never promote a checkpoint.
    for (status, expected_kind) in [
        ("failed", "response.failed"),
        ("incomplete", "response.incomplete"),
        ("cancelled", "response.cancelled"),
    ] {
        let value = json!({"type":"response.completed","response":{"id":"failed-response","status":status,"output":[]}});
        let mut payload = event(&value);
        payload.extend(event(&completion()));
        let mut decoder = SseDecoder::new(4096);
        let frames = decoder.push_frames(&payload).unwrap();
        assert_eq!(frames.len(), 1, "first failure must fence later completion");
        let failure = frames[0].value.as_ref().unwrap();
        assert_eq!(failure["type"], expected_kind);
        let mut observation = ResponseObservation::default();
        observation.observe(failure);
        observation.observe(&completion());
        assert!(observation.terminal);
        assert!(!observation.succeeded);
        assert!(decoder.push(&event(&completion())).unwrap().is_empty());

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("identity.sqlite");
        let store = super::super::identity::IdentityStore::open(path.clone()).unwrap();

        let mut headers = axum::http::HeaderMap::new();
        let mut body = json!({"model":"test","client_metadata":{"thread_id":"thread"},"input":[{"type":"reasoning","encrypted_content":"unaccepted-cipher"}]});
        let mut context = store
            .inspect_for_rebuild("caller", &headers, &body)
            .unwrap();
        store
            .configure_rebuild(&mut context, &headers, &body, true)
            .unwrap();
        let binding = store
            .rewrite(&context, "account", "owner", &mut headers, &mut body)
            .unwrap();
        store.observe_event(&binding, failure).unwrap();
        // A later transport-level signal cannot authorize an aborted history.
        store.observe_event(&binding, &completion()).unwrap();
        let connection = rusqlite::Connection::open(&path).unwrap();
        for table in ["native_rebuild_active_v1", "native_rebuild_checkpoints_v1"] {
            let rows: i64 = connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(
                rows, 0,
                "{status}: an aborted history must not publish {table}"
            );
        }
    }
}
