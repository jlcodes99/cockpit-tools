//! Synthetic large histories only. No account credentials, proxy or live endpoint.
use super::*;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

fn request_with(bytes: Vec<u8>) -> reqwest::Request {
    let mut request = reqwest::Request::new(
        Method::POST,
        "https://chatgpt.com/backend-api/codex/responses"
            .parse()
            .unwrap(),
    );
    request.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    request
        .headers_mut()
        .insert("x-fixture-route", HeaderValue::from_static("route-a"));
    *request.timeout_mut() = Some(Duration::from_secs(17));
    *request.body_mut() = Some(bytes.into());
    request
}

fn body_bytes(request: &reqwest::Request) -> &[u8] {
    request.body().unwrap().as_bytes().unwrap()
}

// A deterministic base64-like image payload, rather than a string of zeroes.
// Distinct suffixes can be used by callers without sharing real image content.
fn large_history(bytes: usize) -> Vec<u8> {
    let prefix = br#"{"model":"fixture-model","input":[{"role":"user","content":[{"type":"input_image","image_url":"data:image/png;base64,"#;
    let suffix = br#""}]}],"stream":true}"#;
    let mut output = Vec::with_capacity(bytes);
    output.extend_from_slice(prefix);
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut state = 0x1234_5678_u32;
    while output.len() < bytes - suffix.len() {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        output.push(alphabet[(state & 63) as usize]);
    }
    output.extend_from_slice(suffix);
    assert_eq!(output.len(), bytes);
    output
}

fn assert_large_history_roundtrip(mebibytes: usize) {
    let input = large_history(mebibytes * 1024 * 1024);
    let input_hash = Sha256::digest(&input);
    let request = request_with(input);
    let original_address = body_bytes(&request).as_ptr();
    let original_headers = request.headers().clone();
    let original_url = request.url().clone();
    let started = Instant::now();
    let (compressed, plain) = compress_request(request).unwrap();
    let elapsed = started.elapsed();
    let plain = plain.expect("compressible large history needs a safe identity fallback");
    assert_eq!(plain.headers(), &original_headers);
    assert_eq!(plain.url(), &original_url);
    assert_eq!(plain.method(), Method::POST);
    assert_eq!(plain.timeout(), Some(&Duration::from_secs(17)));
    assert_eq!(body_bytes(&plain).as_ptr(), original_address);
    assert_eq!(body_bytes(&plain).len(), mebibytes * 1024 * 1024);
    assert_eq!(Sha256::digest(body_bytes(&plain)), input_hash);
    assert_eq!(compressed.headers()[header::CONTENT_ENCODING], "zstd");
    assert_eq!(compressed.headers()["x-fixture-route"], "route-a");
    assert!(body_bytes(&compressed).len() < body_bytes(&plain).len());
    let decoded = zstd::stream::decode_all(body_bytes(&compressed)).unwrap();
    assert_eq!(decoded.as_slice(), body_bytes(&plain));
    // Both actual outbound forms remain reusable for the existing retry adapter.
    for request in [&plain, &compressed] {
        let cloned = request.try_clone().unwrap();
        assert_eq!(body_bytes(&cloned).as_ptr(), body_bytes(request).as_ptr());
        assert_eq!(body_bytes(&cloned), body_bytes(request));
    }
    eprintln!(
        "large-compression fixture: input={} compressed={} elapsed_ms={} plain_sha256={:x}",
        body_bytes(&plain).len(),
        body_bytes(&compressed).len(),
        elapsed.as_millis(),
        input_hash,
    );
}

#[test]
fn five_mebibyte_image_history_roundtrips_with_original_plain_fallback() {
    assert_large_history_roundtrip(5);
}

#[test]
fn thirty_mebibyte_image_history_roundtrips_with_original_plain_fallback() {
    assert_large_history_roundtrip(30);
}

#[test]
fn one_hundred_ten_mebibyte_image_history_roundtrips_with_original_plain_fallback() {
    assert_large_history_roundtrip(110);
}

#[test]
fn uncompressible_body_is_preserved_without_an_encoding_or_fallback() {
    let mut bytes = vec![0; 64 * 1024];
    let mut state = 0x9abc_def0_u32;
    for byte in &mut bytes {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        *byte = state as u8;
    }
    let hash = Sha256::digest(&bytes);
    let request = request_with(bytes);
    let original_address = body_bytes(&request).as_ptr();
    let (request, fallback) = compress_request(request).unwrap();
    assert!(fallback.is_none());
    assert!(!request.headers().contains_key(header::CONTENT_ENCODING));
    assert_eq!(body_bytes(&request).as_ptr(), original_address);
    assert_eq!(Sha256::digest(body_bytes(&request)), hash);
}

#[tokio::test]
async fn small_missing_and_streaming_bodies_keep_their_original_content() {
    let request = request_with(b"{\"input\":\"small\"}".to_vec());
    let address = body_bytes(&request).as_ptr();
    let (request, fallback) = compress_request(request).unwrap();
    assert!(fallback.is_none());
    assert_eq!(body_bytes(&request).as_ptr(), address);
    assert_eq!(body_bytes(&request), b"{\"input\":\"small\"}");

    let mut missing = request_with(Vec::new());
    *missing.body_mut() = None;
    let (missing, fallback) = compress_request(missing).unwrap();
    assert!(missing.body().is_none());
    assert!(fallback.is_none());

    let stream = futures_util::stream::iter([Ok::<_, std::io::Error>(Bytes::from_static(
        b"streamed identity body",
    ))]);
    let mut streamed = request_with(Vec::new());
    *streamed.body_mut() = Some(reqwest::Body::wrap_stream(stream));
    let (mut streamed, fallback) = compress_request(streamed).unwrap();
    assert!(fallback.is_none());
    assert!(!streamed.headers().contains_key(header::CONTENT_ENCODING));
    use http_body_util::BodyExt;
    let bytes = streamed
        .body_mut()
        .take()
        .unwrap()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(&bytes[..], b"streamed identity body");
}

#[test]
fn duplicate_inline_images_keep_the_existing_compression_exclusion() {
    let image = "data:image/png;base64,QUJDREVG";
    let body = serde_json::json!({"input":[
        {"role":"user","content":[{"type":"input_image","image_url":image}]},
        {"type":"custom_tool_call_output","output":[{"type":"input_image","image_url":image}]}
    ]});
    assert!(has_duplicate_inline_images(&body));
    assert!(!codex_endpoint(
        &"https://example.invalid/backend-api/codex/responses"
            .parse()
            .unwrap()
    ));
    assert!(!codex_endpoint(
        &"http://chatgpt.com/backend-api/codex/responses"
            .parse()
            .unwrap()
    ));
}

#[tokio::test]
async fn cancelled_compression_waiter_drops_its_body_after_worker_completion() {
    let (stop, stop_rx) = tokio::sync::watch::channel(false);
    let request = request_with(large_history(5 * 1024 * 1024));
    let witness = Arc::new(());
    let weak = Arc::downgrade(&witness);
    let (entered, ready) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let (done, finished) = tokio::sync::oneshot::channel();
    let work = tokio::spawn(super::super::blocking::run(
        super::super::lifecycle::StopSignal(stop_rx),
        Duration::from_secs(10),
        move || {
            let _owned = witness;
            let _ = entered.send(());
            released.recv_timeout(Duration::from_secs(5)).unwrap();
            let result = compress_request(request);
            let _ = done.send(());
            result
        },
    ));
    ready.await.unwrap();
    work.abort();
    assert!(work.await.unwrap_err().is_cancelled());
    assert!(weak.upgrade().is_some(), "real worker still owns its input");
    release.send(()).unwrap();
    finished.await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while weak.upgrade().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(stop);
}
