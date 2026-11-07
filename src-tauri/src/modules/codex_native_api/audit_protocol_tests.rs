//! Protocol edge cases audited against CPA 90c447d3 and sub2api a3298b46e.
use super::*;
use std::io::Write;

fn compressed(kind: &str, plain: &[u8]) -> Vec<u8> {
    match kind {
        "gzip" => {
            let mut encoder =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            encoder.write_all(plain).unwrap();
            encoder.finish().unwrap()
        }
        "zstd" => zstd::stream::encode_all(plain, 3).unwrap(),
        "deflate" => {
            let mut encoder =
                flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
            encoder.write_all(plain).unwrap();
            encoder.finish().unwrap()
        }
        "zlib" => {
            let mut encoder =
                flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
            encoder.write_all(plain).unwrap();
            encoder.finish().unwrap()
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn audit_unlabelled_compression_and_both_deflate_formats_stream_in_byte_fragments() {
    let plain = b"data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r\",\"status\":\"completed\",\"output\":[]}}\n\n";
    for (kind, encoding) in [
        ("gzip", None),
        ("zstd", None),
        ("deflate", Some("deflate")),
        ("zlib", Some("deflate")),
    ] {
        let wire = compressed(kind, plain);
        let chunks = wire
            .into_iter()
            .map(|byte| Ok::<_, std::io::Error>(axum::body::Bytes::from(vec![byte])));
        let mut response =
            axum::http::Response::builder().header("content-type", "text/event-stream");
        if let Some(encoding) = encoding {
            response = response.header("content-encoding", encoding);
        }
        let response = reqwest::Response::from(
            response
                .body(reqwest::Body::wrap_stream(futures_util::stream::iter(
                    chunks,
                )))
                .unwrap(),
        );
        let decoded = super::super::http_compression::decode_response(response)
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(decoded.as_ref(), plain, "{kind}");
    }
}

#[tokio::test]
async fn audit_plain_first_byte_does_not_wait_for_another_network_chunk() {
    let stream = futures_util::stream::once(async {
        Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"d"))
    })
    .chain(futures_util::stream::pending());
    let response = reqwest::Response::from(
        axum::http::Response::builder()
            .header("content-type", "text/event-stream")
            .body(reqwest::Body::wrap_stream(stream))
            .unwrap(),
    );
    let mut stream = super::super::http_compression::decode_response(response)
        .unwrap()
        .bytes_stream();
    let first = tokio::time::timeout(Duration::from_millis(200), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(first.as_ref(), b"d");
}

#[tokio::test]
async fn audit_uncompressed_upstream_retains_large_transport_chunks() {
    let chunk = axum::body::Bytes::from(format!("data: {}", "x".repeat(256 * 1024)));
    let source = futures_util::stream::iter(vec![Ok::<_, std::io::Error>(chunk.clone())]);
    let response = reqwest::Response::from(
        axum::http::Response::builder()
            .header("content-type", "text/event-stream")
            .body(reqwest::Body::wrap_stream(source))
            .unwrap(),
    );
    let mut stream = super::super::http_compression::decode_response(response)
        .unwrap()
        .bytes_stream();
    assert_eq!(stream.next().await.unwrap().unwrap(), chunk);
    assert!(stream.next().await.is_none());
}

#[test]
fn audit_legacy_messages_preserve_native_tools_templates_and_stream_options() {
    let native_tool = json!({"type":"function","name":"lookup","parameters":{"type":"object","properties":{}},"future_option":true});
    let body = json!({"type":"response.create","event_id":"caller","model":"m","messages":[{"role":"user","content":"hello"}],
        "tools":[native_tool,{"type":"function","function":{"name":"legacy","parameters":{"type":"object"}}}],
        "prompt":{"id":"template","variables":{"name":"value"}},"stream_options":{"reasoning_summary_delivery":"stream"},
        "metadata":{"large_integer":9007199254740993u64},"instructions":"explicit","parallel_tool_calls":false});
    let normalized = super::super::request_compat::ingress(body.clone()).unwrap();
    assert_eq!(normalized["tools"][0], native_tool);
    assert_eq!(normalized["tools"][1]["name"], "legacy");
    for key in [
        "type",
        "event_id",
        "prompt",
        "stream_options",
        "metadata",
        "instructions",
        "parallel_tool_calls",
    ] {
        assert_eq!(normalized[key], body[key], "{key}");
    }
    assert_eq!(normalized["input"][0]["content"][0]["text"], "hello");
}

#[test]
fn audit_legacy_responses_preserve_tool_names_and_native_choice_scope() {
    let name = format!("mcp__lookup.{}", "long_name".repeat(9));
    let native_tool = json!({"type":"function","name":name,"parameters":{"type":"object"}});
    let custom = json!({"type":"custom","name":"custom.tool","format":{"type":"text"}});
    let grouped = json!({"type":"namespace","name":"group","tools":[native_tool.clone()]});
    let choice = json!({"type":"function","name":name,"namespace":"group","future_option":true});
    let body = json!({"model":"m","messages":[
        {"role":"assistant","tool_calls":[{"id":"c1","type":"function","function":{"name":name,"arguments":"{}"}}]},
        {"role":"tool","tool_call_id":"c1","content":"ok"}
    ],"tools":[native_tool,custom,grouped],"tool_choice":choice});
    let normalized = super::super::request_compat::ingress(body.clone()).unwrap();
    assert_eq!(normalized["tools"], body["tools"]);
    assert_eq!(normalized["tool_choice"], choice);
    let call = normalized["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "function_call")
        .unwrap();
    assert_eq!(call["name"], name);
    assert_eq!(call["call_id"], "c1");
}

#[test]
fn audit_public_openai_steering_keeps_correlation_local_only_on_official_destinations() {
    let original = json!({"type":"response.steer","event_id":"local-id","previous_response_id":"r","input":"change"});
    for (endpoint, strict) in [
        ("https://api.openai.com/v1", true),
        ("https://eu.api.openai.com/v1", true),
        ("wss://api.openai.com/v1/responses", true),
        ("wss://eu.api.openai.com/v1/responses", true),
        ("https://api.openai.com.example/v1", false),
        ("wss://api.openai.com.example/v1/responses", false),
        ("https://provider.invalid/v1", false),
    ] {
        let mut body = original.clone();
        super::super::codex_wire::normalize_steering(&mut body, false, endpoint);
        assert_eq!(body.get("event_id").is_none(), strict, "{endpoint}");
        assert_eq!(body["input"], original["input"]);
        assert_eq!(
            body["previous_response_id"],
            original["previous_response_id"]
        );
    }
}

#[tokio::test]
async fn audit_websocket_accepts_string_input_and_reused_empty_create_without_replaying_history() {
    let (addr,task) = mock(Router::new().route("/v1/responses",get(|upgrade:WebSocketUpgrade|async move {
        upgrade.on_upgrade(|mut socket|async move {
            for index in 0..2 {
                let Some(Ok(Message::Text(text))) = socket.recv().await else { panic!("missing create") };
                let body:Value = serde_json::from_str(&text).unwrap();
                assert_eq!(body["input"], if index==0 {json!([{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}])} else {json!([])});
                socket.send(Message::Text(json!({"type":"response.completed","response":{"id":format!("r{index}"),"status":"completed","output":[]}}).to_string().into())).await.unwrap();
            }
            let _=socket.recv().await;
        })
    }))).await;
    let (mut handle, _, path) = native(addr, false, false, Options::default()).await;
    let mut request = format!("ws://{}/v1/responses", handle.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", "Bearer local-test-key".parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    for body in [
        json!({"type":"response.create","model":"m","input":"hello"}),
        json!({"type":"response.create","model":"m"}),
    ] {
        socket
            .send(ClientMessage::Text(body.to_string().into()))
            .await
            .unwrap();
        assert_eq!(ws_json(&mut socket).await["type"], "response.completed");
    }
    socket.close(None).await.unwrap();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
    let _ = std::fs::remove_dir_all(path);
}

#[tokio::test]
async fn audit_binary_attachments_and_explicit_identity_are_not_magic_decoded() {
    let wire = compressed("gzip", b"attachment");
    for (media, encoding) in [
        ("application/octet-stream", None),
        ("application/json", Some("identity")),
    ] {
        let mut response = axum::http::Response::builder().header("content-type", media);
        if let Some(encoding) = encoding {
            response = response.header("content-encoding", encoding);
        }
        let response =
            reqwest::Response::from(response.body(reqwest::Body::from(wire.clone())).unwrap());
        let bytes = super::super::http_compression::decode_response(response)
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(bytes.as_ref(), wire);
    }
}

#[tokio::test]
async fn audit_compression_rejection_inspection_preserves_bodies_and_execution_fences() {
    let cases = [
        (json!({"error":{"message":"invalid encoding"}}).to_string(),true),
        (json!({"error":{"message":"invalid encoding"},"usage":{"output_tokens":1}}).to_string(),false),
        (json!({"body":{"error":{"message":"invalid encoding"},"output":[{"type":"function_call","call_id":"ran"}]}}).to_string(),false),
        ("data: {\"type\":\"response.output_text.delta\",\"delta\":\"zstd\"}\n\n".to_string(),false),
        (json!({"error":{"message":"invalid encoding"},"padding":"x".repeat(65536)}).to_string(),false),
    ];
    for (body, safe) in cases {
        let response = reqwest::Response::from(
            axum::http::Response::builder()
                .status(400)
                .header("content-type", "application/json")
                .header("content-encoding", "gzip")
                .body(reqwest::Body::from(compressed("gzip", body.as_bytes())))
                .unwrap(),
        );
        let (response, actual) =
            super::super::http_compression::rejected_before_execution(response)
                .await
                .unwrap();
        assert_eq!(actual, safe);
        assert_eq!(response.status().as_u16(), 400);
        assert_eq!(response.text().await.unwrap(), body);
    }
}
