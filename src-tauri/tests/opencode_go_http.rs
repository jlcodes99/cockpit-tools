#[path = "../src/modules/opencode_go_http.rs"]
mod opencode_go_http;

use opencode_go_http::{OpenCodeGoHttpClient, OpenCodeGoHttpError};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

fn read_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .expect("read timeout");
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let count = stream.read(&mut buffer).expect("read request");
        if count == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..count]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8(request).expect("HTTP request is UTF-8")
}

fn spawn_server(
    response: String,
    delay: Duration,
) -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let address = listener.local_addr().expect("test server address");
    let (sender, receiver) = mpsc::channel();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept request");
        let request = read_request(&mut stream);
        sender.send(request).expect("capture request");
        thread::sleep(delay);
        let _ = stream.write_all(response.as_bytes());
    });
    (
        format!("http://{address}/zen/go/v1/usage"),
        receiver,
        handle,
    )
}

#[test]
fn official_base_url_is_canonicalized_without_accepting_lookalike_hosts() {
    let client =
        OpenCodeGoHttpClient::new("https://opencode.ai/zen/go").expect("official base URL");
    assert_eq!(client.endpoint(), "https://opencode.ai/zen/go/v1/usage");

    let client = OpenCodeGoHttpClient::new("https://opencode.ai/zen/go/v1/usage")
        .expect("canonical usage URL");
    assert_eq!(client.endpoint(), "https://opencode.ai/zen/go/v1/usage");

    for unsafe_url in [
        "http://opencode.ai/zen/go/v1",
        "https://opencode.ai.evil.example/zen/go/v1",
        "https://user@opencode.ai/zen/go/v1",
        "https://opencode.ai:444/zen/go/v1",
        "https://opencode.ai/zen/go/v2",
        "https://opencode.ai/zen/go/v1?key=secret",
    ] {
        assert_eq!(
            OpenCodeGoHttpClient::new(unsafe_url)
                .err()
                .expect("unsafe URL rejected"),
            OpenCodeGoHttpError::InvalidBaseUrl
        );
    }
}

#[tokio::test]
async fn request_uses_bearer_auth_accept_json_and_the_usage_path() {
    let response = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 21\r\nConnection: close\r\n\r\n{\"usage\":{\"ok\":true}}";
    let (endpoint, receiver, server) = spawn_server(response.to_string(), Duration::ZERO);
    let client =
        OpenCodeGoHttpClient::for_test(&endpoint, Duration::from_secs(1)).expect("test client");

    let body = client.fetch_usage("fixture-key").await.expect("usage JSON");
    assert_eq!(body["usage"]["ok"], true);
    let request = receiver.recv().expect("captured request");
    assert!(request.starts_with("GET /zen/go/v1/usage HTTP/1.1\r\n"));
    assert!(request
        .to_ascii_lowercase()
        .contains("authorization: bearer fixture-key\r\n"));
    assert!(request
        .to_ascii_lowercase()
        .contains("accept: application/json\r\n"));
    server.join().expect("test server");
}

#[tokio::test]
async fn rate_limit_returns_retry_after_without_retrying_or_exposing_the_body() {
    let secret_body = "upstream diagnostic includes fixture-key";
    let response = format!(
        "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 17\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        secret_body.len(),
        secret_body
    );
    let (endpoint, receiver, server) = spawn_server(response, Duration::ZERO);
    let client =
        OpenCodeGoHttpClient::for_test(&endpoint, Duration::from_secs(1)).expect("test client");

    let error = client
        .fetch_usage("fixture-key")
        .await
        .expect_err("429 is returned to the caller");
    assert_eq!(
        error,
        OpenCodeGoHttpError::RateLimited {
            retry_after: Some(Duration::from_secs(17))
        }
    );
    let rendered = error.to_string();
    assert!(!rendered.contains("fixture-key"));
    assert!(!rendered.contains(secret_body));
    receiver.recv().expect("exactly one request");
    server.join().expect("test server");
}

#[tokio::test]
async fn configured_timeout_is_reported_without_secret_material() {
    let response = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}";
    let (endpoint, receiver, server) =
        spawn_server(response.to_string(), Duration::from_millis(150));
    let client =
        OpenCodeGoHttpClient::for_test(&endpoint, Duration::from_millis(20)).expect("test client");

    let error = client
        .fetch_usage("timeout-fixture-key")
        .await
        .expect_err("request times out");
    assert_eq!(error, OpenCodeGoHttpError::Timeout);
    assert!(!error.to_string().contains("timeout-fixture-key"));
    receiver.recv().expect("request reached test server");
    server.join().expect("test server");
}

#[tokio::test]
async fn redirects_are_not_followed_with_authorization() {
    let response = "HTTP/1.1 302 Found\r\nLocation: https://example.invalid/collect\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    let (endpoint, receiver, server) = spawn_server(response.to_string(), Duration::ZERO);
    let client =
        OpenCodeGoHttpClient::for_test(&endpoint, Duration::from_secs(1)).expect("test client");

    assert_eq!(
        client.fetch_usage("redirect-fixture-key").await,
        Err(OpenCodeGoHttpError::HttpStatus(302))
    );
    receiver.recv().expect("only original request");
    server.join().expect("test server");
}
