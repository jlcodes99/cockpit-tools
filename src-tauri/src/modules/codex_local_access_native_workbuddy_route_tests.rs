use super::*;

fn linked_account() -> CodexAccount {
    let mut account = CodexAccount::new_api_key(
        "workbuddy-route-fixture".into(),
        "fixture@example.invalid".into(),
        String::new(),
        crate::models::codex::CodexApiProviderMode::Custom,
        Some(codex_account::WORKBUDDY_ACP_BASE_URL.into()),
        Some("workbuddy_acp".into()),
        Some("WorkBuddy".into()),
        vec!["workbuddy-default".into()],
    );
    account.openai_api_key = None;
    account.api_credential_kind = Some(codex_account::WORKBUDDY_ACP_PROVIDER_KIND.into());
    account.api_credential_ref = Some("dummy-workbuddy-source".into());
    account
}

fn collection() -> CodexLocalAccessCollection {
    serde_json::from_value(json!({
        "enabled": true, "port": 0, "apiKey": "dummy-local-key",
        "accountIds": ["workbuddy-route-fixture"], "createdAt": 1, "updatedAt": 1
    }))
    .unwrap()
}

#[tokio::test]
async fn workbuddy_native_route_bypasses_http_identity_and_proxy_setup() {
    let mut account = linked_account();
    // ACP owns its transport. Generic account proxy preparation must not run.
    account.egress_proxy_url = Some("not-an-http-proxy".into());
    let collection = collection();
    let host = NativeHostBridge::for_collection(collection.clone());
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_static("Bearer dummy-source-token"),
    );
    let permit: Arc<dyn Send + Sync> = Arc::new(());
    let route = native_route_for_prepared_account(
        &host,
        &collection,
        (
            account,
            headers,
            codex_account::WORKBUDDY_ACP_BASE_URL.into(),
            None,
        ),
        "workbuddy-default".into(),
        Some(permit.clone()),
        false,
    )
    .await
    .unwrap();

    // This exact marker is the existing ACP executor's dispatch key. No HTTP
    // normalization, fake key, installation profile, or source token is needed.
    assert_eq!(route.base_url, codex_account::WORKBUDDY_ACP_BASE_URL);
    assert_eq!(route.model, "workbuddy-default");
    assert_eq!(route.account_id, "workbuddy-route-fixture");
    assert_eq!(route.wire_api, native_contract::WireApi::Responses);
    assert!(route.headers.is_empty());
    assert!(route.installation_profile.is_none());
    assert!(route.transport_identity.is_none());
    assert!(route.proxy_url.is_none());
    assert!(route.proxy_route.is_none());
    assert!(route.compact_endpoint.is_none());
    assert!(route.websocket_endpoint.is_none());
    assert!(!route.chatgpt);
    assert!(!route.chatgpt_plan);
    assert!(Arc::ptr_eq(route.permit.as_ref().unwrap(), &permit));
    assert!(!host.identity_fixture.path().exists());
    assert!(host.clients.lock().unwrap().entries.is_empty());
}

#[tokio::test]
async fn workbuddy_native_route_requires_exact_stored_and_prepared_marker() {
    let collection = collection();
    let host = NativeHostBridge::for_collection(collection.clone());
    for marker in [
        "workbuddy-acp://native/",
        "workbuddy-acp://other",
        "workbuddy-acp://native?token=dummy",
        "https://example.invalid/v1",
    ] {
        let mut account = linked_account();
        account.api_base_url = Some(marker.into());
        // Exercise the production entry point: invalid stored references must
        // fail before it can load any real WorkBuddy source from disk.
        let error = native_route_for_reserved_account(
            &host,
            &collection,
            account,
            "workbuddy-default".into(),
            false,
            None,
            false,
        )
        .await
        .err()
        .expect("a linked account cannot replace its internal marker");
        assert_eq!(error.code, "invalid_workbuddy_account", "{marker}");

        let error = native_route_for_prepared_account(
            &host,
            &collection,
            (
                linked_account(),
                axum::http::HeaderMap::new(),
                marker.into(),
                None,
            ),
            "workbuddy-default".into(),
            None,
            false,
        )
        .await
        .err()
        .expect("prepared credentials cannot replace the internal marker");
        assert_eq!(error.code, "invalid_upstream", "{marker}");
    }
    assert!(!host.identity_fixture.path().exists());
}

#[tokio::test]
async fn workbuddy_marker_does_not_relax_ordinary_http_url_security() {
    let collection = collection();
    let host = NativeHostBridge::for_collection(collection.clone());
    for url in [
        codex_account::WORKBUDDY_ACP_BASE_URL,
        "file:///tmp/workbuddy-fixture",
        "https://user@example.invalid/v1",
        "https://user:password@example.invalid/v1",
        "https://example.invalid/v1?credential=dummy",
        "https://example.invalid/v1#fragment",
    ] {
        let mut account = linked_account();
        account.api_credential_kind = None;
        account.api_credential_ref = None;
        account.openai_api_key = Some("dummy-ordinary-api-key".into());
        account.api_base_url = Some(url.into());
        let error = native_route_for_reserved_account(
            &host,
            &collection,
            account,
            "workbuddy-default".into(),
            false,
            None,
            false,
        )
        .await
        .err()
        .expect("ordinary API accounts must retain HTTP URL validation");
        assert_eq!(error.code, "invalid_upstream", "{url}");
    }
    assert!(!host.identity_fixture.path().exists());
    assert!(host.clients.lock().unwrap().entries.is_empty());
}

#[test]
fn workbuddy_native_route_requires_a_nonempty_typed_account_reference() {
    let account = linked_account();
    assert!(native_workbuddy::validate_reference(&account).is_ok());
    for reference in [None, Some(""), Some("   ")] {
        let mut invalid = account.clone();
        invalid.api_credential_ref = reference.map(str::to_string);
        assert!(native_workbuddy::validate_reference(&invalid).is_err());
    }
    let mut invalid = account.clone();
    invalid.api_credential_kind = Some("ordinary-api-key".into());
    assert!(native_workbuddy::validate_reference(&invalid).is_err());
    let mut invalid = account;
    invalid.auth_mode = crate::models::codex::CodexAuthMode::OAuth;
    assert!(native_workbuddy::validate_reference(&invalid).is_err());
}

#[test]
fn workbuddy_native_routes_only_accept_text_responses_not_auxiliary_operations() {
    let account = linked_account();
    assert!(native_auxiliary_eligible(&account, None));
    for operation in [
        "responses.websocket",
        "audio.transcriptions",
        "audio.translations",
        "audio.speech",
        "search",
        "realtime",
        "compact",
    ] {
        assert!(
            !native_auxiliary_eligible(&account, Some(operation)),
            "{operation}"
        );
    }
    let mut ordinary = account;
    ordinary.api_credential_kind = None;
    ordinary.api_credential_ref = None;
    assert!(native_auxiliary_eligible(
        &ordinary,
        Some("responses.websocket")
    ));
    assert!(native_auxiliary_eligible(&ordinary, Some("audio.speech")));
}

#[test]
fn workbuddy_image_inputs_are_not_silently_downgraded_to_text() {
    let account = linked_account();
    let collection = collection();
    let plan = NativeModelRoutePlan {
        model: "workbuddy-default".into(),
        full_model: "workbuddy-default".into(),
        account_ids: vec![account.id.clone()],
        oauth_only: false,
        provider: true,
        automatic: false,
        gateway: None,
        xai: false,
        codex_catalog: NativeRouteCodexCatalog::default(),
    };
    assert_eq!(
        native_operation_model(&collection, &plan, &account, None, false).as_deref(),
        Some("workbuddy-default")
    );
    assert!(native_operation_model(&collection, &plan, &account, None, true).is_none());
}
