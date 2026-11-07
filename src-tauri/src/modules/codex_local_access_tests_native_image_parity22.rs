// Historical Go routing cases; standalone native transport follows CPA v8.0.58.
const PARITY22_IMAGE_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";

#[tokio::test]
async fn parity22_image_outbound_models_match_go_relay() {
    use axum::{body::Body, extract::State, response::{Response, IntoResponse}, routing::post, Json, Router};
    use base64::Engine;
    type Seen = Arc<Mutex<Vec<Value>>>;
    async fn capture(State(seen): State<Seen>, Json(body): Json<Value>) -> Response {
        seen.lock().unwrap().push(body);
        let event = json!({"type":"response.completed","response":{"id":"image-response","status":"completed","output":[{"id":"image-item","type":"image_generation_call","result":PARITY22_IMAGE_PNG,"output_format":"png"}]}});
        Response::builder()
            .header("content-type", "text/event-stream")
            .body(Body::from(format!(
                "event: response.completed\ndata: {event}\n\n"
            )))
            .unwrap()
    }
    async fn capture_native(State(seen): State<Seen>, request: axum::extract::Request) -> Response {
        use axum::extract::FromRequest;
        let multipart = request.headers().get("content-type").and_then(|value|value.to_str().ok()).is_some_and(|value|value.starts_with("multipart/"));
        let body = if multipart {
            let mut form = axum::extract::Multipart::from_request(request,&()).await.unwrap();
            let mut body = json!({});
            while let Some(field) = form.next_field().await.unwrap() {
                let name = field.name().unwrap_or("").to_owned();
                if field.file_name().is_none() { body[&name] = json!(field.text().await.unwrap()); }
            }
            body
        } else { Json::<Value>::from_request(request,&()).await.unwrap().0 };
        seen.lock().unwrap().push(body);
        Json(json!({"data":[{"b64_json":PARITY22_IMAGE_PNG}],"created":1})).into_response()
    }
    let _env = crate::modules::test_support::env_lock().lock().unwrap();
    let cases: Vec<Value> = serde_json::from_str(include_str!(
        "codex_native_api/fixtures/parity22_images_go.json"
    ))
    .unwrap();
    for case in cases {
        let seen: Seen = Arc::new(Mutex::new(vec![]));
        let app = Router::new()
            .route("/v1/responses", post(capture))
            .route("/v1/images/generations", post(capture_native))
            .route("/v1/images/edits", post(capture_native))
            .with_state(seen.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let mut config = collection();
        config.account_ids = vec!["chat".into()];
        config.image_generation_account_ids = vec!["image".into(), "image-api".into()];
        config.image_generation_main_model = Some("gpt-5.5".into());
        config.max_concurrent_image_requests = 0;
        config.model_aliases = serde_json::from_value(
            json!([{"alias":"gpt-5.5","sourceModel":"vendor/private-model","fork":false}]),
        )
        .unwrap();
        config.suppress_oauth_model_alias = true;
        config.api_keys = serde_json::from_value(json!([{"id":"fixture","label":"fixture","key":"fixture","enabled":true,"accountIds":["chat"],"inheritAccountPool":false,
            "providerGateway":{"baseUrl":"http://127.0.0.1:9/v1","apiKey":"fixture-provider","upstreamModel":"vendor/private-model","upstreamModels":["vendor/private-model"],"wireApi":"responses"}}])).unwrap();
        let mut bridge = NativeHostBridge::for_collection(config);
        let mut oauth = account();
        oauth.tokens.access_token = "at-fixture-image-token".into();
        oauth.egress_proxy_disabled = true;
        bridge.account_overrides.insert("image".into(), oauth);
        for (id, models) in [
            ("image-api", vec!["gpt-image-2.5"]),
            ("chat", vec!["vendor/private-model"]),
        ] {
            let mut source = image_model_api_account(&models);
            source.id = id.into();
            bridge.account_overrides.insert(id.into(), source);
        }
        let host = Arc::new(ImageModelHttpHost {
            bridge,
            upstream: base,
        });
        let dir = tempfile::tempdir().unwrap();
        let mut service = native_api::start(
            "127.0.0.1:0".parse().unwrap(),
            host,
            dir.path().join("identity.sqlite"),
            native_api::Options::default(),
        )
        .await
        .unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let endpoint = format!("http://{}{}", service.addr, case["path"].as_str().unwrap());
        let mut request = client.post(endpoint).bearer_auth("fixture");
        if case["name"].as_str().unwrap().ends_with("-multipart") {
            request = request.multipart(
                reqwest::multipart::Form::new()
                    .text("model", "gpt-5.5")
                    .text("prompt", "draw")
                    .part(
                        "image",
                        reqwest::multipart::Part::bytes(
                            base64::engine::general_purpose::STANDARD
                                .decode(PARITY22_IMAGE_PNG)
                                .unwrap(),
                        )
                        .file_name("fixture.png")
                        .mime_str("image/png")
                        .unwrap(),
                    ),
            );
        } else {
            request = request.json(&case["request"]);
        }
        let response = request.send().await.unwrap();
        let status = response.status();
        let body = response.text().await.unwrap();
        let wire = seen.lock().unwrap().clone();
        if case["name"] == "mixed-oauth-main" {
            // The historical mixed route selects Lite; v8 rejects forced hosted
            // images rather than silently switching its selected main model.
            assert_eq!(status, 400, "{}: {body}", case["name"]);
            let error: Value = serde_json::from_str(&body).unwrap();
            assert_eq!(error["error"]["code"], "hosted_image_generation_unsupported");
            assert!(wire.is_empty());
        } else {
            assert_eq!(status, 200, "{}: {body}", case["name"]);
            assert_eq!(wire.len(), 1, "{}", case["name"]);
            assert_eq!(wire[0]["model"],case["image_model"],"{}",case["name"]);
            assert!(wire[0].get("tools").is_none());
        }
        service.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
    }
}

#[tokio::test]
async fn parity22_image_conversation_fallback_keeps_model_permissions_and_pool() {
    let _env = crate::modules::test_support::env_lock().lock().unwrap();
    let mut config = collection();
    config.account_ids = vec!["chat".into()];
    config.image_generation_account_ids = vec!["image".into()];
    config
        .image_generation_models
        .push("vendor/explicit-image".into());
    config
        .image_generation_account_models
        .insert("image".into(), "gpt-image-2.5-flare".into());
    config.max_concurrent_image_requests = 0;
    config.api_keys=serde_json::from_value(json!([{"id":"key","label":"key","key":"fixture-key","enabled":true,"modelPrefix":"team","allowedModels":["gpt-5.6-sol","vendor/chat-model","gpt-image-2.5","gpt-image-2.5-flare","gpt-image-2.5-sunburst","vendor/explicit-image","vendor/unknown"],"accountIds":["chat"],"inheritAccountPool":false}])).unwrap();
    let mut host = NativeHostBridge::for_collection(config);
    host.account_overrides.insert(
        "image".into(),
        image_model_api_account(&[
            "gpt-image-2.5-flare",
            "gpt-image-2.5-sunburst",
            "vendor/explicit-image",
        ]),
    );
    let mut chat = image_model_api_account(&["vendor/chat-model"]);
    chat.id = "chat".into();
    host.account_overrides.insert("chat".into(), chat);
    let caller = native_contract::Caller {
        id: "key".into(),
        label: "fixture".into(),
    };
    for model in [
        "team/gpt-5.6-sol",
        "team/vendor/chat-model",
        "team/gpt-image-2.5-sunburst",
    ] {
        let selected = native_image_route_once(&host, &caller, image_model_request(model))
            .await
            .unwrap();
        assert_eq!(selected.route.account_id, "image");
        assert_eq!(
            selected.image_model,
            if model.ends_with("sunburst") {
                "gpt-image-2.5-sunburst"
            } else {
                "gpt-image-2.5-flare"
            }
        );
    }
    for model in [
        "gpt-5.6-sol",
        "team/vendor/unknown",
        "team/vendor/explicit-image",
        "team/gpt-image-2.50-flare",
    ] {
        assert!(
            native_image_route_once(&host, &caller, image_model_request(model))
                .await
                .is_err(),
            "{model}"
        );
    }
    for denied in ["gpt-5.6-sol", "gpt-image-2.5", "gpt-image-2.5-flare"] {
        host.collection_override.as_mut().unwrap().api_keys[0].excluded_models =
            vec![denied.into()];
        assert!(
            native_image_route_once(&host, &caller, image_model_request("team/gpt-5.6-sol"))
                .await
                .is_err(),
            "{denied}"
        );
        host.collection_override.as_mut().unwrap().api_keys[0]
            .excluded_models
            .clear();
        host.collection_override
            .as_mut()
            .unwrap()
            .account_model_rules =
            serde_json::from_value(json!([{"accountId":"image","excludedModels":[denied]}]))
                .unwrap();
        assert!(
            native_image_route_once(&host, &caller, image_model_request("team/gpt-5.6-sol"))
                .await
                .is_err(),
            "{denied}"
        );
        host.collection_override
            .as_mut()
            .unwrap()
            .account_model_rules
            .clear();
        host.collection_override.as_mut().unwrap().excluded_models = vec![denied.into()];
        assert!(
            native_image_route_once(&host, &caller, image_model_request("team/gpt-5.6-sol"))
                .await
                .is_err(),
            "{denied}"
        );
        host.collection_override
            .as_mut()
            .unwrap()
            .excluded_models
            .clear();
    }
    host.collection_override.as_mut().unwrap().api_keys[0].allowed_models =
        vec!["gpt-5.6-sol".into(), "gpt-image-2.5".into()];
    assert!(
        native_image_route_once(&host, &caller, image_model_request("team/gpt-5.6-sol"))
            .await
            .is_err()
    );
    host.collection_override.as_mut().unwrap().api_keys[0]
        .allowed_models
        .clear();
    host.collection_override.as_mut().unwrap().api_keys[0].model_routing=Some(serde_json::from_value(json!({"defaultRoute":"oauth","failurePolicy":"strict","routes":[{"id":"route","namespace":"provider","providerAccountId":"chat","providerGateway":{"baseUrl":"http://127.0.0.1:9/v1","apiKey":"fixture","upstreamModel":"private-chat","upstreamModels":["private-chat"]}}]})).unwrap());
    let selected = native_image_route_once(
        &host,
        &caller,
        image_model_request("team/provider/private-chat"),
    )
    .await
    .unwrap();
    assert_eq!(selected.image_model, "gpt-image-2.5-flare");
    drop(selected);
    assert!(
        native_image_route_once(&host, &caller, image_model_request("team/provider/unknown"))
            .await
            .is_err()
    );
    host.collection_override.as_mut().unwrap().api_keys[0].model_routing = None;
    host.collection_override.as_mut().unwrap().model_aliases = serde_json::from_value(
        json!([{"alias":"gpt-5.6-sol","sourceModel":"vendor/chat-model","fork":false}]),
    )
    .unwrap();
    host.collection_override
        .as_mut()
        .unwrap()
        .account_model_rules =
        serde_json::from_value(json!([{"accountId":"image","excludedModels":["gpt-5.6-sol"]}]))
            .unwrap();
    assert!(
        native_image_route_once(&host, &caller, image_model_request("team/gpt-5.6-sol"))
            .await
            .is_err()
    );
    host.collection_override
        .as_mut()
        .unwrap()
        .account_model_rules
        .clear();
    let mut pinned = image_model_request("team/gpt-5.6-sol");
    pinned.preferred_account_id = Some("chat".into());
    assert!(native_image_route_once(&host, &caller, pinned)
        .await
        .is_err());
    let mut excluded = image_model_request("team/gpt-5.6-sol");
    excluded.excluded_account_ids = vec!["image".into()];
    assert!(native_image_route_once(&host, &caller, excluded)
        .await
        .is_err());
    host.collection_override
        .as_mut()
        .unwrap()
        .image_generation_account_policies
        .insert(
            "image".into(),
            CodexLocalAccessImageGenerationPolicy::Disabled,
        );
    assert!(
        native_image_route_once(&host, &caller, image_model_request("team/gpt-5.6-sol"))
            .await
            .is_err()
    );
}
