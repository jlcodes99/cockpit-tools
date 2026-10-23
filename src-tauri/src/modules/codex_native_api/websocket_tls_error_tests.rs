//! Preserve typed transport evidence at WebSocket opening and HTTP bridge
//! boundaries; protocol failures and lookalike strings must stay unclassified.
use super::super::{
    contract::{Route, TransportFailureKind, WireApi},
    lifecycle::StopSignal,
};
use super::*;
use futures_util::{stream, StreamExt};
use std::{io, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::{error::TlsError, Error as WsError};

fn tls_io_error() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, rustls::Error::DecryptError)
}

fn assert_tls(error: &ServiceError, code: &str, context: &str) {
    assert_eq!(error.status, 502);
    assert_eq!(error.code, code);
    assert!(error.message.starts_with(context), "{}", error.message);
    assert!(matches!(
        error.transport_failure,
        Some(TransportFailureKind::TlsDecrypt)
    ));
}

#[test]
fn websocket_initial_read_keeps_typed_tls_evidence_and_existing_error_code() {
    for error in [
        WsError::Io(tls_io_error()),
        WsError::Tls(TlsError::Rustls(rustls::Error::DecryptError)),
    ] {
        let converted = initial_frame_error(&error);
        assert_tls(
            &converted,
            "incomplete_upstream_stream",
            "Upstream closed before its first response event",
        );
    }
    for error in [
        WsError::ConnectionClosed,
        WsError::Io(io::Error::other("TLS DecryptError is only text")),
        WsError::Tls(TlsError::Rustls(rustls::Error::General(
            "DecryptError".into(),
        ))),
    ] {
        let converted = initial_frame_error(&error);
        assert_eq!(converted.code, "incomplete_upstream_stream");
        assert_eq!(converted.status, 502);
        assert!(converted.transport_failure.is_none());
    }
}

#[tokio::test]
async fn websocket_write_preserves_tls_type_without_changing_send_status() {
    let (_stop, snapshot) = tokio::sync::watch::channel(false);
    let error = timed_write(StopSignal(snapshot), Duration::from_secs(1), None, async {
        Err::<(), _>(WsError::Io(tls_io_error()))
    })
    .await
    .unwrap_err();
    assert_tls(&error, "websocket_send_failed", "WebSocket write failed");
}

#[tokio::test]
async fn websocket_http_bridge_preserves_tls_type_before_and_after_canonical_conversion() {
    let canonical = super::super::diagnostics::transport_failure(
        502,
        "upstream_stream_error",
        "Canonical HTTP body failed",
        &tls_io_error(),
    );
    for (failure, context) in [
        (tls_io_error(), "Upstream HTTP stream disconnected"),
        (io::Error::other(canonical), "Canonical HTTP body failed"),
    ] {
        let source = stream::iter([Err::<axum::body::Bytes, _>(failure)]).boxed();
        let mut bridge = transport_bridge::sse_messages(source, 4096);
        let error = bridge.next().await.unwrap().unwrap_err();
        assert_tls(
            &error,
            "upstream_stream_error",
            context,
        );
        assert!(
            bridge.next().await.is_none(),
            "the bridge must deliver the failure once"
        );
    }
    let source = stream::iter([Err::<axum::body::Bytes, _>(io::Error::other(
        "DecryptError",
    ))])
    .boxed();
    let error = transport_bridge::sse_messages(source, 4096)
        .next()
        .await
        .unwrap()
        .unwrap_err();
    assert!(error.transport_failure.is_none());
}

fn route(base_url: String) -> Route {
    Route {
        account_id: "synthetic".into(),
        owner_id: "synthetic-owner".into(),
        continuity_account_id: None,
        installation_profile: None,
        official_authorization_revision: None,
        official_registration_id: None,
        official_fallback_registration_id: None,
        official_fallback_authorization_revision: None,
        authorization_scope_fingerprint: None,
                official_fallback_failure: None,
        transport_identity: None,
        base_url,
        model: "synthetic-model".into(),
        headers: HeaderMap::new(),
        client: reqwest::Client::builder().no_proxy().build().unwrap(),
        proxy_url: None,
        proxy_route: None,
        chatgpt: false,
        chatgpt_plan: false,
        wire_api: WireApi::Responses,
        compact_endpoint: None,
        websocket_endpoint: None,
        default_service_tier: None,
        permit: None,
    }
}

#[tokio::test]
async fn websocket_handshake_connection_failure_keeps_code_and_context() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let (_stop, snapshot) = tokio::sync::watch::channel(false);
    let error = super::super::transport::connect_websocket(
        &route(format!("http://{address}/v1")),
        &format!("ws://{address}/v1/responses"),
        HeaderMap::new(),
        4096,
        Duration::from_secs(2),
        StopSignal(snapshot),
    )
    .await
    .err()
    .expect("closed loopback port cannot connect");
    assert_eq!(error.status, 502);
    assert_eq!(error.code, "websocket_connect_failed");
    assert!(error
        .message
        .starts_with("Unable to establish the upstream WebSocket"));
    assert!(error.transport_failure.is_none());
}

#[tokio::test]
async fn websocket_rejection_body_read_failure_keeps_http_status_without_inventing_tls() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let upstream = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 1024];
        while !request.windows(4).any(|part| part == b"\r\n\r\n") {
            let count = socket.read(&mut buffer).await.unwrap();
            assert!(count > 0);
            request.extend_from_slice(&buffer[..count]);
        }
        // Deliberately omit the terminating chunk; the status has already been
        // received, so a body transport error must retain the rejection status.
        socket.write_all(b"HTTP/1.1 403 Forbidden\r\nTransfer-Encoding: chunked\r\nContent-Type: application/json\r\n\r\n3\r\nbad\r\n").await.unwrap();
        socket.shutdown().await.unwrap();
    });
    let (_stop, snapshot) = tokio::sync::watch::channel(false);
    let error = super::super::transport::connect_websocket(
        &route(format!("http://{address}/v1")),
        &format!("ws://{address}/v1/responses"),
        HeaderMap::new(),
        4096,
        Duration::from_secs(2),
        StopSignal(snapshot),
    )
    .await
    .err()
    .expect("truncated rejected body must fail");
    upstream.await.unwrap();
    assert_eq!(error.status, 403);
    assert_eq!(error.code, "websocket_upstream_rejected");
    assert!(error
        .message
        .starts_with("Unable to read the upstream error response"));
    assert!(error.transport_failure.is_none());
}
