use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct TestData {
    dir: PathBuf,
    previous: Vec<(&'static str, Option<std::ffi::OsString>)>,
}
impl TestData {
    fn new() -> Self {
        let dir =
            std::env::temp_dir().join(format!("pelican-provider-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let previous = [
            "COCKPIT_TOOLS_TEST_DATA_DIR",
            "COCKPIT_TOOLS_DATA_DIR",
            "CODEX_HOME",
        ]
        .into_iter()
        .map(|key| {
            let old = std::env::var_os(key);
            std::env::set_var(key, &dir);
            (key, old)
        })
        .collect();
        Self { dir, previous }
    }
}
impl Drop for TestData {
    fn drop(&mut self) {
        for (key, value) in &self.previous {
            if let Some(value) = value {
                std::env::set_var(key, value);
            } else {
                std::env::remove_var(key);
            }
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn mock_provider(wire: &str) -> (String, tokio::task::JoinHandle<Value>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let wire = wire.to_string();
    let task = tokio::spawn(async move {
        let (mut stream, _) = timeout(Duration::from_secs(30), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let mut raw = Vec::new();
        let header_end = loop {
            let mut chunk = [0; 4096];
            let size = stream.read(&mut chunk).await.unwrap();
            assert!(size > 0);
            raw.extend_from_slice(&chunk[..size]);
            if let Some(index) = raw.windows(4).position(|part| part == b"\r\n\r\n") {
                break index + 4;
            }
        };
        let headers = String::from_utf8_lossy(&raw[..header_end]).to_lowercase();
        let length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .unwrap()
            .trim()
            .parse::<usize>()
            .unwrap();
        while raw.len() < header_end + length {
            let mut chunk = [0; 4096];
            let size = stream.read(&mut chunk).await.unwrap();
            assert!(size > 0);
            raw.extend_from_slice(&chunk[..size]);
        }
        assert!(headers.contains("authorization: bearer fixture-key"));
        assert!(headers.contains(if wire == "responses" {
            "post /v1/responses "
        } else {
            "post /v1/chat/completions "
        }));
        let body: Value = serde_json::from_slice(&raw[header_end..header_end + length]).unwrap();
        assert_eq!(body["model"], "provider-model");
        assert_eq!(body["stream"], true);
        assert!(!body.to_string().contains("max_output_tokens\":256"));
        let html = "<!doctype html><html><body>pelican fixture</body></html>";
        let events = if wire == "responses" {
            format!(
                "data: {}\n\ndata: {}\n\n",
                json!({"type":"response.output_text.delta","delta":html}),
                json!({"type":"response.completed","response":{"id":"resp-fixture","status":"completed","model":"provider-model","output":[{"type":"message","content":[{"type":"output_text","text":html}]}],"usage":{"input_tokens":1,"output_tokens":2,"total_tokens":3}}})
            )
        } else {
            format!(
                "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                json!({"id":"chat-fixture","object":"chat.completion.chunk","model":"provider-model","choices":[{"index":0,"delta":{"role":"assistant","content":html},"finish_reason":null}]}),
                json!({"id":"chat-fixture","object":"chat.completion.chunk","model":"provider-model","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}})
            )
        };
        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", events.len(), events);
        stream.write_all(response.as_bytes()).await.unwrap();
        body
    });
    (format!("http://{address}/v1"), task)
}

#[tokio::test]
async fn pelican_provider_protocols_stream_and_clean_up_without_enrolling_accounts() {
    let _lock = crate::modules::test_support::env_lock().lock().unwrap();
    let data = TestData::new();
    for wire in ["responses", "chat_completions"] {
        let (base_url, upstream) = mock_provider(wire).await;
        let request = CodexModelProviderGatewayChatTestRequest {
            run_id: uuid::Uuid::new_v4().to_string(),
            provider_id: "fixture-provider".into(),
            provider_name: "Fixture".into(),
            base_url,
            api_key_id: Some("fixture-key-id".into()),
            api_key_name: Some("Fixture key".into()),
            api_key: "fixture-key".into(),
            wire_api: wire.into(),
            model_catalog: vec!["provider-model".into()],
            model_id: "provider-model".into(),
            prompt: "Create HTML".into(),
        };
        let (_cancel, receiver) = watch::channel(false);
        let result = timeout(
            Duration::from_secs(45),
            run_pelican_provider_chat(
                request,
                "medium".into(),
                receiver,
                |_| {},
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(result.reply.contains("pelican fixture"));
        let sent = upstream.await.unwrap().to_string();
        assert!(sent.contains("Create HTML"));
        assert!(sent.contains("medium"));
        let dir = provider_gateway_sidecars_dir().unwrap();
        assert!(std::fs::read_dir(dir).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("pelican-provider-")));
    }
    assert!(!data.dir.join("codex_accounts.json").exists());
}

#[tokio::test]
async fn pelican_provider_cancel_closes_stream_and_removes_temporary_gateway() {
    let _lock = crate::modules::test_support::env_lock().lock().unwrap();
    let _data = TestData::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (started, ready) = tokio::sync::oneshot::channel();
    let upstream = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 8192];
        assert!(stream.read(&mut bytes).await.unwrap() > 0);
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
        let _ = started.send(());
        let mut remaining = Vec::new();
        timeout(Duration::from_secs(10), stream.read_to_end(&mut remaining))
            .await
            .unwrap()
            .unwrap();
    });
    let request = CodexModelProviderGatewayChatTestRequest {
        run_id: uuid::Uuid::new_v4().to_string(),
        provider_id: "cancel-fixture".into(),
        provider_name: "Fixture".into(),
        base_url: format!("http://{address}/v1"),
        api_key_id: Some("key".into()),
        api_key_name: None,
        api_key: "fixture-key".into(),
        wire_api: "responses".into(),
        model_catalog: vec!["provider-model".into()],
        model_id: "provider-model".into(),
        prompt: "Create HTML".into(),
    };
    let (cancel, receiver) = watch::channel(false);
    let worker = tokio::spawn(run_pelican_provider_chat(
        request,
        "medium".into(),
        receiver,
        |_| {},
    ));
    timeout(Duration::from_secs(30), ready)
        .await
        .unwrap()
        .unwrap();
    cancel.send_replace(true);
    assert_eq!(
        timeout(Duration::from_secs(10), worker)
            .await
            .unwrap()
            .unwrap()
            .err()
            .as_deref(),
        Some("PELICAN_CANCELLED")
    );
    upstream.await.unwrap();
    assert!(std::fs::read_dir(provider_gateway_sidecars_dir().unwrap())
        .unwrap()
        .all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("pelican-provider-")));
}
