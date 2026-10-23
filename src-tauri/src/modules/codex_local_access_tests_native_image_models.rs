// Synthetic accounts and loopback upstreams only; no live credentials or image calls.

fn image_model_request(model: &str) -> native_contract::ImageRouteRequest {
    native_contract::ImageRouteRequest {
        model: Some(model.into()),
        responses_model: None,
        edit: false,
        target_account_id: None,
        force_refresh: false,
        preferred_account_id: None,
        excluded_account_ids: vec![],
    }
}

fn image_model_api_account(models: &[&str]) -> CodexAccount {
    let mut source = account();
    source.auth_mode = CodexAuthMode::Apikey;
    source.openai_api_key = Some("fixture-image-key".into());
    source.api_base_url = Some("http://127.0.0.1:9/v1".into());
    source.api_model_catalog = models.iter().map(|model| (*model).into()).collect();
    source.egress_proxy_disabled = true;
    source
}

#[test]
fn native_image_bare_aliases_use_configured_model_for_oauth_and_api_sources() {
    for selected in ["gpt-image-2", "gpt-image-2.5"] {
        let mut config = collection();
        config.image_generation_model = selected.into();
        for source in [account(), image_model_api_account(&[selected])] {
            for requested in ["gpt-image-2", "gpt-image-2.5"] {
                assert_eq!(
                    native_image_source_model(&config, &source, requested).as_deref(),
                    Some(selected)
                );
            }
        }
    }
}

#[test]
fn native_image_mapping_keeps_explicit_variants_custom_models_and_catalog_boundaries() {
    let mut config = collection();
    let mut source = image_model_api_account(&[
        "gpt-image-2.5-flare",
        "gpt-image-2.5-sunburst",
        "gpt-image-2-flare",
    ]);
    config
        .image_generation_account_models
        .insert(source.id.clone(), "gpt-image-2.5-flare".into());
    for requested in ["gpt-image-2", "gpt-image-2.5"] {
        assert_eq!(
            native_image_source_model(&config, &source, requested).as_deref(),
            Some("gpt-image-2.5-flare")
        );
    }
    assert_eq!(
        native_image_source_model(&config, &source, "gpt-image-2.5-sunburst").as_deref(),
        Some("gpt-image-2.5-sunburst")
    );
    for invalid in [
        "gpt-image-2-flare",
        "gpt-image-2.50-flare",
        "gpt-image-20",
        "gpt-image-2.5-",
        "unknown-model",
    ] {
        assert!(
            native_image_source_model(&config, &source, invalid).is_none(),
            "{invalid}"
        );
    }
    assert!(native_image_source_model(&config, &account(), "gpt-image-2.5-flare").is_none());
    source
        .api_model_catalog
        .retain(|model| model != "gpt-image-2.5-flare");
    assert!(native_image_source_model(&config, &source, "gpt-image-2").is_none());
    source.api_model_catalog.push("gpt-image-2.5-flare".into());
    config
        .image_generation_deleted_models
        .push("gpt-image-2.5-flare".into());
    assert!(native_image_source_model(&config, &source, "gpt-image-2").is_none());

    config.image_generation_model = "vendor/custom-image".into();
    config.image_generation_account_models.clear();
    source.api_model_catalog = vec!["vendor/custom-image".into(), "vendor/other-image".into()];
    for requested in ["gpt-image-2", "gpt-image-2.5", "vendor/custom-image"] {
        assert_eq!(
            native_image_source_model(&config, &source, requested).as_deref(),
            Some("vendor/custom-image")
        );
    }
    assert!(native_image_source_model(&config, &source, "vendor/other-image").is_none());
    assert!(native_image_source_model(&config, &account(), "gpt-image-2").is_none());
}

#[test]
fn native_image_alias_permissions_check_namespace_request_configured_and_actual_models() {
    let mut config = collection();
    let source = image_model_api_account(&["gpt-image-2.5-flare"]);
    config
        .image_generation_account_models
        .insert(source.id.clone(), "gpt-image-2.5-flare".into());
    let mut key: CodexLocalAccessApiKey = serde_json::from_value(json!({
        "id":"key", "label":"key", "key":"fixture-key", "modelPrefix":"team",
        "allowedModels":["gpt-image-2.5", "gpt-image-2.5-flare"]
    }))
    .unwrap();
    assert_eq!(
        native_image_requested_model(
            &config,
            Some(&key),
            &image_model_request("team/gpt-image-2")
        )
        .unwrap(),
        "gpt-image-2"
    );
    for invalid in ["gpt-image-2", "other/gpt-image-2"] {
        assert_eq!(
            native_image_requested_model(&config, Some(&key), &image_model_request(invalid))
                .unwrap_err()
                .status,
            403
        );
    }
    for denied in ["gpt-image-2", "gpt-image-2.5", "gpt-image-2.5-flare"] {
        key.excluded_models = vec![denied.into()];
        assert!(!native_image_model_allowed(
            &config,
            Some(&key),
            &source,
            "gpt-image-2"
        ));
        key.excluded_models.clear();
        config.excluded_models = vec![denied.into()];
        assert!(!native_image_model_allowed(
            &config,
            Some(&key),
            &source,
            "gpt-image-2"
        ));
        config.excluded_models.clear();
        config.account_model_rules =
            serde_json::from_value(json!([{"accountId":source.id,"excludedModels":[denied]}]))
                .unwrap();
        assert!(!native_image_model_allowed(
            &config,
            Some(&key),
            &source,
            "gpt-image-2"
        ));
        config.account_model_rules.clear();
    }
    key.allowed_models = vec!["text-only".into()];
    assert!(native_image_requested_model(
        &config,
        Some(&key),
        &image_model_request("team/gpt-image-2")
    )
    .is_err());
    key.allowed_models = vec!["gpt-image-2.5".into()];
    assert!(!native_image_model_allowed(
        &config,
        Some(&key),
        &source,
        "gpt-image-2"
    ));
    key.allowed_models.push("gpt-image-2.5-flare".into());
    assert!(native_image_model_allowed(
        &config,
        Some(&key),
        &source,
        "gpt-image-2"
    ));
    let mut nested = image_model_request("gpt-image-2");
    nested.responses_model = Some("team/chat-model".into());
    assert_eq!(
        native_image_requested_model(&config, Some(&key), &nested).unwrap(),
        "gpt-image-2"
    );
}

#[tokio::test]
async fn native_image_alias_routing_preserves_pool_disabled_sources_keys_and_pinning() {
    let _env = crate::modules::test_support::env_lock().lock().unwrap();
    let mut config = collection();
    config.image_generation_account_ids = vec!["image".into()];
    config.max_concurrent_image_requests = 0;
    let mut host = NativeHostBridge::for_collection(config);
    host.account_overrides
        .insert("image".into(), image_model_api_account(&["gpt-image-2.5"]));
    let mut outside = image_model_api_account(&["gpt-image-2.5"]);
    outside.id = "outside".into();
    host.account_overrides.insert(outside.id.clone(), outside);
    let caller = native_contract::Caller {
        id: NATIVE_PRIMARY_CALLER.into(),
        label: "fixture".into(),
    };
    let selected = native_image_route_once(&host, &caller, image_model_request("gpt-image-2"))
        .await
        .unwrap();
    assert_eq!(selected.route.account_id, "image");
    assert_eq!(selected.image_model, "gpt-image-2.5");
    drop(selected);
    let mut pinned = image_model_request("gpt-image-2");
    pinned.preferred_account_id = Some("outside".into());
    assert_eq!(
        native_image_route_once(&host, &caller, pinned)
            .await
            .err()
            .unwrap()
            .code,
        "image_source_unavailable"
    );
    let mut targeted = image_model_request("gpt-image-2");
    targeted.target_account_id = Some("outside".into());
    assert_eq!(
        native_image_route_once(&host, &caller, targeted)
            .await
            .err()
            .unwrap()
            .status,
        403
    );
    let mut excluded = image_model_request("gpt-image-2");
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
    assert_eq!(
        native_image_route_once(&host, &caller, image_model_request("gpt-image-2"))
            .await
            .err()
            .unwrap()
            .code,
        "image_source_unavailable"
    );
    host.collection_override
        .as_mut()
        .unwrap()
        .image_generation_account_policies
        .clear();
    host.collection_override.as_mut().unwrap().api_keys = vec![serde_json::from_value(
        json!({"id":"disabled","label":"fixture","key":"fixture-key","enabled":false}),
    )
    .unwrap()];
    let disabled = native_contract::Caller {
        id: "disabled".into(),
        label: "fixture".into(),
    };
    assert_eq!(
        native_image_route_once(&host, &disabled, image_model_request("gpt-image-2"))
            .await
            .err()
            .unwrap()
            .status,
        401
    );
    host.collection_override.as_mut().unwrap().api_keys.clear();
    host.collection_override
        .as_mut()
        .unwrap()
        .image_generation_mode = CodexLocalAccessImageGenerationMode::ImagesOnly;
    let mut hosted = image_model_request("gpt-image-2");
    hosted.responses_model = Some("chat-model".into());
    assert_eq!(
        native_image_route_once(&host, &caller, hosted)
            .await
            .err()
            .unwrap()
            .code,
        "image_generation_disabled"
    );
    host.collection_override
        .as_mut()
        .unwrap()
        .image_generation_mode = CodexLocalAccessImageGenerationMode::Disabled;
    assert_eq!(
        native_image_route_once(&host, &caller, image_model_request("gpt-image-2"))
            .await
            .err()
            .unwrap()
            .code,
        "image_generation_disabled"
    );
}

struct ImageModelHttpHost {
    bridge: NativeHostBridge,
    upstream: String,
}

impl native_contract::Host for ImageModelHttpHost {
    fn authorize<'a>(
        &'a self,
        bearer: &'a str,
    ) -> futures_util::future::BoxFuture<
        'a,
        Result<native_contract::Caller, native_contract::ServiceError>,
    > {
        native_contract::Host::authorize(&self.bridge, bearer)
    }
    fn route<'a>(
        &'a self,
        _: &'a native_contract::Caller,
        _: native_contract::RouteRequest,
    ) -> futures_util::future::BoxFuture<
        'a,
        Result<native_contract::Route, native_contract::ServiceError>,
    > {
        Box::pin(async { panic!("forced image requests must use their image source") })
    }
    fn models<'a>(
        &'a self,
        _: &'a native_contract::Caller,
    ) -> futures_util::future::BoxFuture<'a, Result<Value, native_contract::ServiceError>> {
        Box::pin(async { Ok(json!({"data":[]})) })
    }
    fn image_route<'a>(
        &'a self,
        caller: &'a native_contract::Caller,
        request: native_contract::ImageRouteRequest,
    ) -> futures_util::future::BoxFuture<
        'a,
        Result<native_contract::ImageRoute, native_contract::ServiceError>,
    > {
        Box::pin(async move {
            let mut selected = native_image_route_once(&self.bridge, caller, request).await?;
            // Route the real OAuth adapter to a local fixture as well.
            selected.route.base_url = self.upstream.clone();
            selected.route.client = reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap();
            Ok(selected)
        })
    }
    fn record(&self, _: native_contract::RequestReport) -> futures_util::future::BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

#[tokio::test]
async fn native_image_bare_alias_http_generation_edit_and_responses_send_configured_model() {
    use axum::{
        body::Body,
        extract::{FromRequest, Multipart, Request, State},
        response::Response,
        routing::post,
        Json, Router,
    };
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";
    type Seen = Arc<Mutex<Vec<String>>>;
    async fn image_json(State(seen): State<Seen>, Json(body): Json<Value>) -> Json<Value> {
        seen.lock()
            .unwrap()
            .push(body["model"].as_str().unwrap().into());
        Json(json!({"data":[{"b64_json":PNG}]}))
    }
    async fn image_edit(State(seen): State<Seen>, request: Request) -> Json<Value> {
        if !request.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("multipart/form-data")
        {
            let body = Json::<Value>::from_request(request, &()).await.unwrap();
            return image_json(State(seen), body).await;
        }
        let mut parts = Multipart::from_request(request, &()).await.unwrap();
        while let Some(part) = parts.next_field().await.unwrap() {
            if part.name() == Some("model") {
                let model = part.text().await.unwrap();
                seen.lock().unwrap().push(model);
            }
        }
        Json(json!({"data":[{"b64_json":PNG}]}))
    }
    async fn oauth(State(seen): State<Seen>, Json(body): Json<Value>) -> Response {
        seen.lock()
            .unwrap()
            .push(body["tools"][0]["model"].as_str().unwrap().into());
        let event = json!({"type":"response.completed","response":{"id":"image-response","status":"completed","output":[{"id":"image-item","type":"image_generation_call","result":PNG,"output_format":"png"}]}});
        Response::builder()
            .header("content-type", "text/event-stream")
            .body(Body::from(format!(
                "event: response.completed\ndata: {event}\n\n"
            )))
            .unwrap()
    }
    let _env = crate::modules::test_support::env_lock().lock().unwrap();
    for (oauth_source, selected, actual) in [
        (true, "gpt-image-2.5", "gpt-image-2.5"),
        (false, "gpt-image-2.5", "gpt-image-2.5-flare"),
        (false, "vendor/custom-image", "vendor/custom-image"),
    ] {
        let seen: Seen = Arc::new(Mutex::new(vec![]));
        let app = Router::new()
            .route("/v1/images/generations", post(image_json))
            .route("/v1/images/edits", post(image_edit))
            .route("/v1/responses", post(oauth))
            .with_state(seen.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let mut config = collection();
        config.image_generation_model = selected.into();
        config.image_generation_account_ids = vec!["image".into()];
        config
            .image_generation_account_models
            .insert("image".into(), actual.into());
        config.image_generation_main_model = Some("fixture-main".into());
        config.max_concurrent_image_requests = 0;
        let mut bridge = NativeHostBridge::for_collection(config);
        let mut source = if oauth_source {
            account()
        } else {
            image_model_api_account(&[actual])
        };
        let explicit = if actual == "gpt-image-2.5-flare" {
            "gpt-image-2.5-sunburst"
        } else {
            actual
        };
        if !oauth_source {
            source.api_model_catalog.push(explicit.into());
        }
        source.tokens.access_token = "at-fixture-image-token".into();
        source.egress_proxy_disabled = true;
        bridge.account_overrides.insert("image".into(), source);
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
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();
        let mut expected = Vec::new();
        let requests = if oauth_source {
            vec!["gpt-image-2", "gpt-image-2.5"]
        } else {
            vec!["gpt-image-2", "gpt-image-2.5", explicit]
        };
        for requested in requests {
            for kind in ["generations", "edits", "multipart", "responses"] {
                expected.push(if requested == explicit {
                    explicit.to_string()
                } else {
                    actual.to_string()
                });
                let (endpoint, mut body) = if kind == "responses" {
                    (
                        "responses".to_string(),
                        json!({"model":"chat-model","input":"draw a cat","tools":[{"type":"image_generation","model":requested}],"tool_choice":{"type":"image_generation"}}),
                    )
                } else {
                    (
                        format!(
                            "images/{}",
                            if kind == "multipart" { "edits" } else { kind }
                        ),
                        json!({"model":requested,"prompt":"draw a cat"}),
                    )
                };
                if kind == "edits" {
                    body["image"] = json!(format!("data:image/png;base64,{PNG}"));
                }
                let outgoing = client
                    .post(format!("http://{}/v1/{endpoint}", service.addr))
                    .bearer_auth("fixture");
                let outgoing = if kind == "multipart" {
                    outgoing.multipart(
                        reqwest::multipart::Form::new()
                            .text("prompt", "draw a cat")
                            .text("model", requested.to_string())
                            .part(
                                "image",
                                reqwest::multipart::Part::bytes(vec![0, 1, 2])
                                    .file_name("source.png")
                                    .mime_str("image/png")
                                    .unwrap(),
                            ),
                    )
                } else {
                    outgoing.json(&body)
                };
                let response = outgoing.send().await.unwrap();
                let status = response.status();
                let text = response.text().await.unwrap();
                assert_eq!(
                    status, 200,
                    "{oauth_source}/{selected}/{kind}/{requested}: {text}"
                );
                assert!(text.contains(PNG), "{text}");
            }
        }
        service.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
        assert_eq!(*seen.lock().unwrap(), expected);
    }
}
