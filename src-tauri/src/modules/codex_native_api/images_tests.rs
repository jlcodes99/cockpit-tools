use super::*;

#[test]
fn json_edit_preserves_references_mask_and_exact_model() {
    let request = ImageRequest::from_json(
        ImageOperation::Edit,
        json!({
            "model":"gpt-image-2.5-flare", "prompt":"Replace the background", "n":2,
            "images":[{"file_id":"file-1"},{"image_url":"https://images.example/source.png"}],
            "mask":{"file_id":"mask-1"}, "size":"1536x1024", "quality":"high",
            "output_format":"webp", "output_compression":80, "stream":true,
            "partial_images":2, "vendor_extension":{"keep":true}
        }),
    )
    .unwrap();
    assert_eq!(request.n, 2);
    assert!(request.stream);
    let native = request.native_json("provider/gpt-image-2.5-flare");
    assert_eq!(native["model"], "provider/gpt-image-2.5-flare");
    assert_eq!(native["vendor_extension"], json!({"keep":true}));
    assert_eq!(native["images"][0]["file_id"], "file-1");
    let responses = request
        .to_responses("text-main", "gpt-image-2.5-flare")
        .unwrap();
    assert_eq!(responses["model"], "text-main");
    assert_eq!(
        responses["input"][0]["content"][0]["text"],
        "Replace the background"
    );
    assert_eq!(
        responses["input"][0]["content"][1],
        json!({"type":"input_image","file_id":"file-1"})
    );
    assert_eq!(
        responses["input"][0]["content"][2]["image_url"],
        "https://images.example/source.png"
    );
    assert_eq!(responses["tools"][0]["action"], "edit");
    assert_eq!(
        responses["tools"][0]["input_image_mask"],
        json!({"file_id":"mask-1"})
    );
    assert_eq!(responses["tools"][0]["output_compression"], 80);
    assert_eq!(responses["tools"][0]["partial_images"], 2);
    assert!(responses.get("n").is_none());
    assert!(responses["tools"][0].get("n").is_none());
}

#[test]
fn multipart_retains_binary_native_parts_and_encodes_oauth_inputs() {
    let parts = vec![
        ImagePart {
            name: "prompt".into(),
            filename: None,
            content_type: None,
            bytes: b"edit".to_vec(),
        },
        ImagePart {
            name: "n".into(),
            filename: None,
            content_type: None,
            bytes: b"2".to_vec(),
        },
        ImagePart {
            name: "image[]".into(),
            filename: Some("source.png".into()),
            content_type: Some("image/png".into()),
            bytes: vec![0, 1, 2, 255],
        },
        ImagePart {
            name: "image[]".into(),
            filename: Some("second.webp".into()),
            content_type: Some("image/webp".into()),
            bytes: b"second".to_vec(),
        },
        ImagePart {
            name: "mask".into(),
            filename: Some("mask.png".into()),
            content_type: Some("image/png".into()),
            bytes: b"mask".to_vec(),
        },
    ];
    let request = ImageRequest::from_multipart(ImageOperation::Edit, parts).unwrap();
    let native = request.native_parts("full-model-variant").unwrap();
    assert_eq!(native[2].bytes, vec![0, 1, 2, 255]);
    assert_eq!(native[2].filename.as_deref(), Some("source.png"));
    assert_eq!(native[3].content_type.as_deref(), Some("image/webp"));
    assert_eq!(native.last().unwrap().name, "model");
    assert_eq!(native.last().unwrap().bytes, b"full-model-variant");
    let responses = request.to_responses("main", "image").unwrap();
    assert_eq!(
        responses["input"][0]["content"][1]["image_url"],
        "data:image/png;base64,AAEC/w=="
    );
    assert_eq!(
        responses["tools"][0]["input_image_mask"]["image_url"],
        "data:image/png;base64,bWFzaw=="
    );
}

#[test]
fn input_validation_rejects_ambiguous_or_missing_inputs_before_dispatch() {
    for body in [
        json!({"prompt":""}),
        json!({"prompt":"p","n":0}),
        json!({"prompt":"p","n":11}),
        json!({"prompt":"p","stream":"true"}),
        json!({"prompt":"p","output_compression":101}),
        json!({"prompt":"p","partial_images":4}),
        json!({"prompt":"p","response_format":"unknown"}),
    ] {
        assert!(ImageRequest::from_json(ImageOperation::Generation, body).is_err());
    }
    for image in [
        json!("file:///private/secret.png"),
        json!({"file_id":"file-1","image_url":"https://example.com/x"}),
        json!("https://user:secret@example.com/x"),
    ] {
        assert!(ImageRequest::from_json(
            ImageOperation::Edit,
            json!({"prompt":"edit","image":image})
        )
        .is_err());
    }
    assert!(ImageRequest::from_json(ImageOperation::Edit, json!({"prompt":"edit"})).is_err());
    assert!(ImageRequest::from_json(
        ImageOperation::Generation,
        json!({"prompt":"draw","mask":{"file_id":"mask"}})
    )
    .is_err());
}

#[test]
fn duplicate_multipart_controls_cannot_override_validated_values() {
    let parts = vec![
        ImagePart {
            name: "prompt".into(),
            filename: None,
            content_type: None,
            bytes: b"p".to_vec(),
        },
        ImagePart {
            name: "n".into(),
            filename: None,
            content_type: None,
            bytes: b"1".to_vec(),
        },
        ImagePart {
            name: "n".into(),
            filename: None,
            content_type: None,
            bytes: b"999".to_vec(),
        },
    ];
    assert!(ImageRequest::from_multipart(ImageOperation::Generation, parts).is_err());
}

#[test]
fn oauth_stream_outputs_partial_then_completed_with_usage_once() {
    let mut accumulator =
        ImageAccumulator::new(ImageOperation::Generation, ImageResponseFormat::Base64);
    assert!(accumulator
        .observe(&json!({"type":"response.created","response":{"created_at":100}}))
        .unwrap()
        .is_empty());
    let partial = accumulator.observe(&json!({"type":"response.image_generation_call.partial_image","partial_image_b64":"AA==","partial_image_index":1})).unwrap();
    assert_eq!(partial[0]["type"], "image_generation.partial_image");
    assert_eq!(partial[0]["b64_json"], "AA==");
    assert_eq!(partial[0]["partial_image_index"], 1);
    assert!(accumulator.observe(&json!({"type":"response.output_item.done","output_index":0,"item":{"type":"image_generation_call","id":"ig-1","result":"AQ=="}})).unwrap().is_empty());
    assert!(accumulator.finish().is_err());
    let complete = json!({"type":"response.completed","response":{"created_at":100,"usage":{"input_tokens":5,"output_tokens":10,"total_tokens":15},"output":[{"type":"image_generation_call","id":"ig-1","result":"AQ==","revised_prompt":"refined","output_format":"png"}]}});
    let events = accumulator.observe(&complete).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["type"], "image_generation.completed");
    assert_eq!(events[0]["usage"]["total_tokens"], 15);
    assert!(accumulator.observe(&complete).unwrap().is_empty());
    let result = accumulator.finish().unwrap();
    assert_eq!(result["created"], 100);
    assert_eq!(result["data"][0]["revised_prompt"], "refined");
    assert_eq!(result["data"].as_array().unwrap().len(), 1);
}

#[test]
fn completed_output_is_authoritative_and_image_order_is_preserved() {
    let mut accumulator = ImageAccumulator::new(ImageOperation::Edit, ImageResponseFormat::Url)
        .with_output_format("webp");
    accumulator.observe(&json!({"type":"response.output_item.done","output_index":9,"item":{"type":"image_generation_call","id":"stale","result":"c3RhbGU="}})).unwrap();
    let events = accumulator
        .observe(&json!({"type":"response.completed","response":{"output":[
            {"type":"image_generation_call","id":"z","result":"Zmlyc3Q=","output_format":"webp"},
            {"type":"message","content":[{"type":"output_text","text":"not an image"}]},
            {"type":"image_generation_call","id":"a","result":"c2Vjb25k","output_format":"jpeg"}
        ]}}))
        .unwrap();
    assert_eq!(events[0]["type"], "image_edit.completed");
    assert_eq!(events[0]["output_format"], "webp");
    assert_eq!(events[1]["output_format"], "jpeg");
    let result = accumulator.finish().unwrap();
    assert_eq!(result["data"][0]["url"], "data:image/webp;base64,Zmlyc3Q=");
    assert_eq!(result["data"][1]["url"], "data:image/jpeg;base64,c2Vjb25k");
    assert_eq!(result["data"].as_array().unwrap().len(), 2);
}

#[test]
fn collected_items_survive_missing_completed_output_but_not_failure() {
    let mut accumulator =
        ImageAccumulator::new(ImageOperation::Generation, ImageResponseFormat::Base64);
    accumulator.observe(&json!({"type":"response.output_item.done","output_index":1,"item":{"type":"image_generation_call","result":"AA=="}})).unwrap();
    accumulator
        .observe(&json!({"type":"response.completed","response":{"output":[]}}))
        .unwrap();
    assert_eq!(accumulator.finish().unwrap()["data"][0]["b64_json"], "AA==");
    let mut failed = ImageAccumulator::new(ImageOperation::Generation, ImageResponseFormat::Base64);
    failed.observe(&json!({"type":"response.output_item.done","item":{"type":"image_generation_call","result":"AA=="}})).unwrap();
    assert!(failed.observe(&json!({"type":"response.failed","response":{"usage":{"total_tokens":9},"error":{"message":"generation failed"}}})).is_err());
    assert!(failed.finish().is_err());
    assert_eq!(failed.usage.unwrap()["total_tokens"], 9);
}

#[test]
fn oversized_image_results_are_rejected_and_usage_is_preserved() {
    let mut accumulator =
        ImageAccumulator::new(ImageOperation::Generation, ImageResponseFormat::Base64)
            .with_limit(3);
    assert!(accumulator.observe(&json!({"type":"response.completed","response":{"usage":{"total_tokens":7},"output":[{"type":"image_generation_call","result":"AAAA"}]}})).is_err());
    assert_eq!(accumulator.usage.unwrap()["total_tokens"], 7);
}

#[test]
fn multiple_requested_operations_merge_nested_usage_and_data() {
    let result = merge_responses(&[
        json!({"created":1,"data":[{"b64_json":"AA=="}],"usage":{"input_tokens":2,"output_tokens":3,"total_tokens":5,"input_tokens_details":{"text_tokens":1,"image_tokens":1}}}),
        json!({"created":2,"data":[{"b64_json":"AQ=="}],"usage":{"input_tokens":4,"output_tokens":5,"total_tokens":9,"input_tokens_details":{"text_tokens":3,"image_tokens":1}}}),
    ]).unwrap();
    assert_eq!(result["created"], 1);
    assert_eq!(result["data"].as_array().unwrap().len(), 2);
    assert_eq!(result["usage"]["total_tokens"], 14);
    assert_eq!(result["usage"]["input_tokens_details"]["text_tokens"], 4);
}

#[test]
fn native_json_and_sse_conversion_preserves_data_and_avoids_duplicate_usage() {
    let response = json!({"created":12,"data":[{"b64_json":"AA=="},{"url":"https://images.example/two.png"}],"usage":{"total_tokens":10},"quality":"high"});
    let events = native_to_events(&response, ImageOperation::Edit).unwrap();
    assert_eq!(events[0]["type"], "image_edit.completed");
    assert!(events[0].get("usage").is_none());
    assert_eq!(events[1]["usage"]["total_tokens"], 10);
    let restored = native_from_events(&events).unwrap();
    assert_eq!(restored["data"], response["data"]);
    assert_eq!(restored["usage"], response["usage"]);
    assert_eq!(restored["quality"], "high");
}

#[test]
fn corrupt_oauth_image_is_not_reported_as_successful_generation() {
    let mut accumulator =
        ImageAccumulator::new(ImageOperation::Generation, ImageResponseFormat::Url);
    assert!(accumulator.observe(&json!({"type":"response.completed","response":{"usage":{"total_tokens":3},"output":[{"type":"image_generation_call","result":"not valid base64!"}]}})).is_err());
    assert!(accumulator.finish().is_err());
    assert_eq!(accumulator.usage.unwrap()["total_tokens"], 3);
}

#[test]
fn oauth_images_prefer_image_tool_usage_and_first_image_metadata() {
    let image_usage = json!({"input_tokens":4,"output_tokens":10,"total_tokens":14,"input_tokens_details":{"image_tokens":3,"text_tokens":1}});
    let mut accumulator =
        ImageAccumulator::new(ImageOperation::Generation, ImageResponseFormat::Base64);
    let events = accumulator.observe(&json!({"type":"response.completed","response":{
        "usage":{"total_tokens":999},"tool_usage":{"image_gen":image_usage},
        "output":[
            {"type":"image_generation_call","result":"AA==","quality":"high","output_format":"png"},
            {"type":"image_generation_call","result":"AQ==","quality":"low","output_format":"webp"}
        ]
    }})).unwrap();
    assert_eq!(events[0]["usage"], image_usage);
    assert_eq!(events[1]["usage"], image_usage);
    assert_eq!(events[1]["quality"], "low");
    let result = accumulator.finish().unwrap();
    assert_eq!(result["usage"], image_usage);
    assert_eq!(result["quality"], "high");
    assert_eq!(result["output_format"], "png");
}

#[test]
fn image_tool_usage_without_text_usage_survives_generation_failure() {
    let mut accumulator =
        ImageAccumulator::new(ImageOperation::Generation, ImageResponseFormat::Base64);
    assert!(accumulator
        .observe(&json!({"type":"response.failed","response":{
            "tool_usage":{"image_gen":{"total_tokens":14}},"error":{"message":"generation failed"}
        }}))
        .is_err());
    assert_eq!(accumulator.usage.unwrap()["total_tokens"], 14);
}

#[test]
fn mixed_response_output_keeps_image_identity_indexes_stable() {
    let mut accumulator =
        ImageAccumulator::new(ImageOperation::Generation, ImageResponseFormat::Base64);
    accumulator.observe(&json!({"type":"response.output_item.added","output_index":0,"item":{"id":"text","type":"message"}})).unwrap();
    accumulator.observe(&json!({"type":"response.output_item.added","output_index":1,"item":{"id":"image-a","type":"image_generation_call"}})).unwrap();
    let first=accumulator.observe(&json!({"type":"response.image_generation_call.partial_image","item_id":"image-a","partial_image_b64":"AA=="})).unwrap();
    let second=accumulator.observe(&json!({"type":"response.image_generation_call.partial_image","output_index":3,"partial_image_b64":"AQ=="})).unwrap();
    let completed = accumulator
        .observe(&json!({"type":"response.completed","response":{"output":[
            {"type":"message","id":"text","content":[]},
            {"type":"image_generation_call","id":"image-a","result":"AA=="},
            {"type":"function_call","id":"tool"},
            {"type":"image_generation_call","id":"image-b","result":"AQ=="}
        ]}}))
        .unwrap();
    assert_eq!(first[0]["output_index"], 0);
    assert_eq!(second[0]["output_index"], 1);
    assert_eq!(completed[0]["output_index"], first[0]["output_index"]);
    assert_eq!(completed[1]["output_index"], second[0]["output_index"]);
    let mut unknown =
        ImageAccumulator::new(ImageOperation::Generation, ImageResponseFormat::Base64);
    let partial=unknown.observe(&json!({"type":"response.image_generation_call.partial_image","partial_image_b64":"AA=="})).unwrap();
    assert!(partial[0].get("output_index").is_none());
}
