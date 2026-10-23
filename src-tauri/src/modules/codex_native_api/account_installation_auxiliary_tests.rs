use super::*;

fn canonical() -> String {
    json!({"installation_id":"client-installation", "thread_id":"raw-thread"}).to_string()
}
fn assert_account_header(record: &WireRecord) {
    let profile = test_profile("a");
    assert_eq!(
        record.headers["x-codex-installation-id"],
        profile.installation_id
    );
    let metadata: Value =
        serde_json::from_str(record.headers["x-codex-turn-metadata"].to_str().unwrap()).unwrap();
    assert_eq!(metadata["installation_id"], profile.installation_id);
}

#[tokio::test]
async fn account_installation_compact_search_oauth_images_and_audio_use_consistent_metadata() {
    let (mut handle, host, source, task, _dir) =
        identity_fixture(WireApi::Responses, false, false).await;
    host.oauth.store(true, Ordering::SeqCst);
    for path in ["responses/compact", "alpha/search", "images/generations"] {
        let request = match path {
            "images/generations" => json!({"model":"image","prompt":"fixture drawing"}),
            "alpha/search" => json!({"model":"m","id":"search-identity","input":"fixture query"}),
            _ => json!({"model":"m","input":[]}),
        };
        let mut request = request;
        request["client_metadata"] = json!({"x-codex-installation-id":"client-installation", "x-codex-turn-metadata":canonical()});
        let response = http_client()
            .post(format!("http://{}/v1/{path}", handle.addr))
            .bearer_auth("key")
            .header("x-codex-installation-id", "client-installation")
            .header("x-codex-turn-metadata", canonical())
            .json(&request)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let text = response.text().await.unwrap();
        assert_eq!(status, 200, "{path}: {text}");
        let seen = source.seen.lock().unwrap();
        let record = seen.last().unwrap();
        assert_account_header(record);
        let profile = test_profile("a");
        assert_eq!(
            record.body["client_metadata"]["x-codex-installation-id"],
            profile.installation_id
        );
        let canonical: Value = serde_json::from_str(
            record.body["client_metadata"]["x-codex-turn-metadata"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(canonical["installation_id"], profile.installation_id);
    }
    let form = reqwest::multipart::Form::new().text("model", "m").part(
        "file",
        reqwest::multipart::Part::bytes(vec![0, 1, 2]).file_name("fixture.wav"),
    );
    let response = http_client()
        .post(format!("http://{}/v1/audio/transcriptions", handle.addr))
        .bearer_auth("key")
        .header("x-codex-installation-id", "client-installation")
        .header("x-codex-turn-metadata", canonical())
        .multipart(form)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert_eq!(status, 200, "{text}");
    let seen = source.seen.lock().unwrap();
    let record = seen.last().unwrap();
    assert_eq!(record.path, "/v1/transcribe");
    assert_account_header(record);
    drop(seen);
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn account_installation_provider_images_and_audio_never_forward_client_device_headers() {
    let (mut handle, _, source, task, _dir) =
        identity_fixture(WireApi::Responses, false, false).await;
    let response = http_client()
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("key")
        .header("x-codex-installation-id", "client-installation")
        .json(&json!({"model":"image","prompt":"fixture drawing"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let _: Value = response.json().await.unwrap();
    let form = reqwest::multipart::Form::new().text("model", "m").part(
        "file",
        reqwest::multipart::Part::bytes(vec![0, 1, 2]).file_name("fixture.wav"),
    );
    let response = http_client()
        .post(format!("http://{}/v1/audio/transcriptions", handle.addr))
        .bearer_auth("key")
        .header("x-codex-installation-id", "client-installation")
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let _: Value = response.json().await.unwrap();
    let seen = source.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    for record in seen.iter() {
        assert!(!record.headers.contains_key("x-codex-installation-id"));
    }
    drop(seen);
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn account_installation_chat_and_grok_keep_their_protocol_and_business_fields() {
    let (mut handle, host, _, task, _dir) =
        identity_fixture(WireApi::Responses, false, false).await;
    let request = RouteRequest {
        model: "m".into(),
        session_id: None,
        thread_id: None,
        parent_thread_id: None,
        preferred_account_id: None,
        soft_preferred_account_id: None,
        target_account_id: None,
        force_refresh: false,
        image_generation_requested: false,
        image_input: false,
        excluded_account_ids: vec![],
        requires_permit: false,
        required_capabilities: Vec::new(),
        capability_fallback_from: None,
        auxiliary: None,
    };
    let caller = Caller {
        id: "fixture".into(),
        label: "fixture".into(),
    };
    let mut route = host.route(&caller, request).await.unwrap();
    for grok in [false, true] {
        let mut headers = HeaderMap::new();
        let mut body = json!({"metadata":{"installation_id":"business-data"},"input":[{"encrypted_content":"cipher"}]});
        let original = body.clone();
        if grok {
            route.wire_api = WireApi::Responses;
            route
                .headers
                .insert("x-xai-token-auth", "fixture".parse().unwrap());
        } else {
            route.wire_api = WireApi::Chat;
        }
        crate::modules::codex_native_api::account_installation::apply(
            &route,
            &mut headers,
            &mut body,
        )
        .unwrap();
        assert!(headers.is_empty());
        assert_eq!(body, original);
    }
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}
