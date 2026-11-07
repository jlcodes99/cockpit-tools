use super::super::{compat, events::SseDecoder, request_compat};
use super::*;
use std::io::{Read, Write};

#[test]
fn full_compat_long_call_ids_keep_pairs_and_avoid_existing_aliases() {
    use sha2::{Digest, Sha256};
    let long = "call_".to_owned() + &"x".repeat(120);
    let hash = Sha256::digest(long.as_bytes());
    let alias = format!(
        "{}_{}",
        &long[..47],
        hash[..8]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    let mut body = json!({"input":[{"type":"function_call","call_id":long,"name":"f","arguments":"{}"},{"type":"function_call_output","call_id":long,"output":"result"},{"type":"function_call","call_id":alias,"name":"g","arguments":"{}"}]});
    super::super::codex_cpa_wire::sanitize_input_ids(&mut body);
    assert!(body["input"][0]["call_id"].as_str().unwrap().len() <= 64);
    assert_eq!(body["input"][0]["call_id"], body["input"][1]["call_id"]);
    assert_ne!(body["input"][0]["call_id"], body["input"][2]["call_id"]);
    let once = body.clone();
    super::super::codex_cpa_wire::sanitize_input_ids(&mut body);
    assert_eq!(once, body);
    let mut lone = json!({"previous_response_id":"upstream","input":[{"type":"function_call_output","call_id":long,"output":"result"}]});
    super::super::codex_cpa_wire::sanitize_input_ids(&mut lone);
    assert_eq!(lone["input"][0]["call_id"], long);
}

#[test]
fn full_compat_legacy_ingress_preserves_native_input_templates_and_tool_pairs() {
    let legacy = request_compat::ingress(json!({"model":"m","messages":[
        {"role":"assistant","tool_calls":[{"id":"c","type":"function","function":{"name":"f","arguments":"{}"}}]},
        {"role":"tool","tool_call_id":"c","content":"done"}],"previous_response_id":"stale","parallel_tool_calls":false})).unwrap();
    assert_eq!(legacy["input"][1]["call_id"], "c");
    assert_eq!(legacy["parallel_tool_calls"], false);
    assert!(legacy.get("previous_response_id").is_none());
    let native = request_compat::ingress(json!({"model":"m","input":[],"messages":["invalid legacy"],"prompt":{"id":"template"},"commands":[],"metadata":{"integer":9007199254740993u64}})).unwrap();
    assert_eq!(native["input"], json!([]));
    assert_eq!(native["prompt"]["id"], "template");
    assert_eq!(native["metadata"]["integer"], 9007199254740993u64);
    assert!(native.get("messages").is_none());
    assert_eq!(
        request_compat::ingress(json!({"model":"m","prompt":"legacy"})).unwrap()["input"],
        "legacy"
    );
}

#[test]
fn full_compat_concatenated_events_keep_order_and_reject_partial_documents() {
    let text = concat!("id: event-1\r\nevent: response.created\r\ndata: ",
        "{\"type\":\"response.created\",\"response\":{\"id\":\"r\"}}",
        "{\"type\":\"response.output_text.delta\",\"delta\":\"ok\"}",
        "{\"type\":\"response.completed\",\"response\":{\"id\":\"r\",\"status\":\"completed\",\"output\":[]}}\r\n\r\n");
    for split in 1..text.len() {
        let mut decoder = SseDecoder::new(4096);
        let mut frames = decoder.push_frames(&text.as_bytes()[..split]).unwrap();
        frames.extend(decoder.push_frames(&text.as_bytes()[split..]).unwrap());
        frames.extend(decoder.finish_frames().unwrap());
        let values: Vec<_> = frames
            .iter()
            .filter_map(|frame| frame.value.as_ref())
            .collect();
        assert_eq!(values.len(), 3, "split={split}");
        assert_eq!(values[1]["delta"], "ok");
        assert!(String::from_utf8_lossy(&frames[0].bytes).contains("id: event-1"));
    }
    assert!(super::super::event_documents::split(
        br#"{"type":"response.created"}{"type":"response.failed""#
    )
    .is_none());
    assert!(super::super::event_documents::split(br#"{"type":"x"}[]"#).is_none());
}

#[test]
fn full_compat_commentary_roundtrips_separately_from_signed_reasoning() {
    let req = compat::prepare_request(
        json!({"model":"m","messages":[{"role":"user","content":"hi"}]}),
        compat::CompatFlavor::Claude,
        None,
        None,
    )
    .unwrap();
    let commentary = json!({"type":"message","id":"comment","role":"assistant","phase":"commentary","content":[{"type":"output_text","text":"progress"}]});
    let reasoning = json!({"type":"reasoning","id":"reason","summary":[{"type":"summary_text","text":"reasoning"}],"encrypted_content":"actual-cipher"});
    let final_item = json!({"type":"message","id":"final","role":"assistant","phase":"final_answer","content":[{"type":"output_text","text":"answer"}]});
    let response = json!({"id":"r","status":"completed","output":[commentary,reasoning,final_item],"usage":{"input_tokens":100,"output_tokens":5,"input_tokens_details":{"cached_tokens":20,"cache_write_tokens":30}}});
    let out = compat::response_from_responses(&response, &req.context).unwrap();
    assert_eq!(out["content"][0]["signature"], "cpa:commentary:v1");
    assert_eq!(out["content"][1]["signature"], "actual-cipher");
    assert_eq!(out["content"][2]["text"], "answer");
    assert_eq!(out["usage"]["input_tokens"], 50);
    let replay = compat::prepare_request(
        json!({"model":"m","messages":[{"role":"assistant","content":out["content"]}]}),
        compat::CompatFlavor::Claude,
        None,
        None,
    )
    .unwrap();
    assert_eq!(replay.responses["input"][0]["phase"], "commentary");
    assert!(!replay.responses.to_string().contains("cpa:commentary"));
    assert!(replay.responses.to_string().contains("actual-cipher"));
    for terminal_only in [false, true] {
        let mut adapter = compat::CompatStreamAdapter::new(req.context.clone());
        let mut frames = Vec::new();
        if !terminal_only {
            frames.extend(
                adapter
                    .push(&json!({"type":"response.created","response":{"id":"r","created_at":1}}))
                    .unwrap(),
            );
            frames.extend(adapter.push(&json!({"type":"response.output_item.added","output_index":0,"item":{"id":"comment","type":"message","phase":"commentary","role":"assistant","content":[]}})).unwrap());
            frames.extend(adapter.push(&json!({"type":"response.output_text.delta","item_id":"comment","output_index":0,"content_index":0,"delta":"progress"})).unwrap());
            frames.extend(adapter.push(&json!({"type":"response.output_item.done","output_index":0,"item":commentary})).unwrap());
        }
        frames.extend(
            adapter
                .push(&json!({"type":"response.completed","response":response}))
                .unwrap(),
        );
        let values: Vec<_> = frames.iter().map(|frame| &frame.data).collect();
        assert_eq!(
            values
                .iter()
                .filter(|v| v.pointer("/delta/signature") == Some(&json!("cpa:commentary:v1")))
                .count(),
            1
        );
        assert_eq!(
            values
                .iter()
                .filter_map(|v| v.pointer("/delta/thinking").and_then(Value::as_str))
                .collect::<String>(),
            "progressreasoning"
        );
        assert!(values
            .iter()
            .any(|v| v.pointer("/delta/text") == Some(&json!("answer"))));
    }
}

#[tokio::test]
async fn full_compat_http_response_compression_is_negotiated_and_flushes_sse() {
    let release = Arc::new(tokio::sync::Notify::new());
    let notify = release.clone();
    let (addr, task) = mock(Router::new().route(
        "/v1/responses",
        post(move || {
            let release = notify.clone();
            async move {
                let stream = futures_util::stream::iter(vec![Ok::<_, std::io::Error>(
                    Bytes::from(created()),
                )])
                .chain(futures_util::stream::once(async move {
                    release.notified().await;
                    Ok(Bytes::from(completed()))
                }));
                Response::builder()
                    .header("content-type", "text/event-stream")
                    .body(Body::from_stream(stream))
                    .unwrap()
            }
        }),
    ))
    .await;
    let (mut handle, _host, path) = native(addr, false, false, Options::default()).await;
    let raw = reqwest::Client::builder()
        .no_proxy()
        .no_gzip()
        .no_zstd()
        .no_brotli()
        .no_deflate()
        .build()
        .unwrap();
    for (accept, encoding) in [
        ("gzip;q=0.2,zstd;q=0.8", "zstd"),
        ("GZIP;q=1,zstd;q=0", "gzip"),
    ] {
        let response = raw
            .post(format!("http://{}/v1/responses", handle.addr))
            .bearer_auth("local-test-key")
            .header("accept-encoding", accept)
            .json(&body(true))
            .send()
            .await
            .unwrap();
        assert_eq!(response.headers()["content-encoding"], encoding);
        assert!(response.headers().get("content-length").is_none());
        assert!(response
            .headers()
            .get_all("vary")
            .iter()
            .any(|h| h.to_str().unwrap().contains("Accept-Encoding")));
        let mut stream = response.bytes_stream();
        let first = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        release.notify_one();
        let mut bytes = first.to_vec();
        while let Some(chunk) = stream.next().await {
            bytes.extend_from_slice(&chunk.unwrap());
        }
        let mut decoded = String::new();
        if encoding == "gzip" {
            flate2::read::GzDecoder::new(bytes.as_slice())
                .read_to_string(&mut decoded)
                .unwrap();
        } else {
            decoded =
                String::from_utf8(zstd::stream::decode_all(bytes.as_slice()).unwrap()).unwrap();
        }
        assert!(
            decoded.contains("response.created") && decoded.contains("response.completed"),
            "{decoded}"
        );
    }
    release.notify_one();
    let response = raw
        .post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("local-test-key")
        .header("accept-encoding", "gzip;q=0,zstd;q=0")
        .json(&body(false))
        .send()
        .await
        .unwrap();
    assert!(response.headers().get("content-encoding").is_none());
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
    let _ = std::fs::remove_dir_all(path);
}

#[tokio::test]
async fn full_compat_brotli_deflate_and_chained_requests_enforce_decoded_limit() {
    let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
    let (addr, task) = mock(Router::new().route(
        "/v1/responses",
        post({
            let seen = seen.clone();
            move |axum::Json(value): axum::Json<Value>| {
                seen.lock().unwrap().push(value);
                async { axum::Json(json!({"id":"r","status":"completed","output":[]})) }
            }
        }),
    ))
    .await;
    let (mut handle, _, path) = native(
        addr,
        false,
        false,
        Options {
            max_body_bytes: 1024,
            ..Default::default()
        },
    )
    .await;
    for large in [false, true] {
        let raw = json!({"model":"m","input":"x".repeat(if large {4096}else{64})})
            .to_string()
            .into_bytes();
        let mut deflate = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        deflate.write_all(&raw).unwrap();
        let deflate = deflate.finish().unwrap();
        let mut brotli = Vec::new();
        {
            let mut encoder = brotli::CompressorWriter::new(&mut brotli, 4096, 3, 20);
            encoder.write_all(&raw).unwrap();
        }
        let chain = zstd::stream::encode_all(brotli.as_slice(), 1).unwrap();
        for (encoding, bytes) in [("deflate", deflate), ("br", brotli), ("br, zstd", chain)] {
            let response = client()
                .post(format!("http://{}/v1/responses", handle.addr))
                .bearer_auth("local-test-key")
                .header("content-type", "application/json")
                .header("content-encoding", encoding)
                .body(bytes)
                .send()
                .await
                .unwrap();
            assert_eq!(
                response.status().as_u16(),
                if large { 413 } else { 200 },
                "{encoding}"
            );
        }
    }
    assert_eq!(seen.lock().unwrap().len(), 3);
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
    let _ = std::fs::remove_dir_all(path);
}

#[tokio::test]
async fn full_compat_explicit_field_repair_retries_same_route_without_changing_input() {
    let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
    let (addr,task)=mock(Router::new().route("/v1/responses",post({let seen=seen.clone();move |axum::Json(value):axum::Json<Value>|{seen.lock().unwrap().push(value.clone());async move {
        if value["input"][0].get("status").is_some() {(StatusCode::BAD_REQUEST,axum::Json(json!({"error":{"code":"unknown_parameter","param":"input[0].status","message":"Unknown parameter input[0].status"}})))}
        else {(StatusCode::OK,axum::Json(json!({"id":"r","status":"completed","output":[]})))}
    }}}))).await;
    let (mut handle, host, path) = native(addr, false, false, Options::default()).await;
    let response=client().post(format!("http://{}/v1/responses",handle.addr)).bearer_auth("local-test-key").json(&json!({"model":"m","input":[{"type":"message","role":"user","status":"completed","content":"first"},{"type":"message","role":"user","status":"completed","content":"second"}]})).send().await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "{}",
        response.text().await.unwrap()
    );
    let requests = seen.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1]["input"]
        .as_array()
        .unwrap()
        .iter()
        .all(|i| i.get("status").is_none()));
    assert_eq!(requests[1]["input"][1]["content"], "second");
    drop(requests);
    assert_eq!(host.selections.lock().unwrap().len(), 1);
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
    let _ = std::fs::remove_dir_all(path);
}
