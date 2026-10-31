use super::*;

#[test]
fn remote_service_enables_images_without_a_legacy_image_model_in_selected_catalog() {
    let base = format!("http://192.0.2.10:1455/{}/v1", uuid::Uuid::new_v4());
    let mut account = CodexAccount::new_api_key(
        "remote-cockpit-test".into(),
        "remote@fixture.invalid".into(),
        "fixture-key".into(),
        CodexApiProviderMode::Custom,
        Some(base.clone()),
        Some("remote-cockpit".into()),
        Some("Remote Cockpit".into()),
        vec!["gpt-5.5".into()],
    );
    let remote = serde_json::from_value(serde_json::json!({
        "product":"cockpit-tools","protocolVersion":1,"serviceId":"fixture","serviceName":"Cockpit Tools",
        "keyLabel":"B","status":"active","models":["gpt-5.5","gpt-image-2.5"],"imageModels":["gpt-image-2.5"],
        "capabilities":{"responses":true,"images":true,"imageEdits":true,"hostedImages":true,"usage":true,"websockets":true},
        "tokenLimit":null,"tokenUsed":0,"budget":{},"updatedAt":1
    })).unwrap();
    crate::modules::codex_remote_service::remember_test_connection(&base, "fixture-key", remote);
    let provider = resolve_api_provider_config(
        Some(&base),
        Some(CodexApiProviderMode::Custom),
        Some("remote-cockpit"),
        Some("Remote Cockpit"),
    )
    .unwrap();
    assert!(api_key_provider_should_enable_imagegen(&account, &provider));
    assert!(crate::modules::codex_local_access::account_requires_provider_gateway(&account));
    let dir = tempfile::tempdir().unwrap();
    write_api_key_bearer_provider_override_to_config_toml(
        dir.path(),
        &provider,
        "fixture-key",
        true,
        true,
        false,
        "responses",
    )
    .unwrap();
    let config = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
    assert!(config.contains(CODEX_IMAGEGEN_ACTOR_HEADER));
    assert!(!config.contains(CODEX_DISABLE_HOSTED_IMAGE_GENERATION_HEADER));
    assert!(config.contains("requires_openai_auth = false"));
    account.openai_api_key = Some("different-key".into());
    assert!(!api_key_provider_should_enable_imagegen(
        &account, &provider
    ));
    account.api_model_catalog = vec!["gpt-image-2.5".into()];
    assert!(api_key_account_supports_image_generation(&account));
}
