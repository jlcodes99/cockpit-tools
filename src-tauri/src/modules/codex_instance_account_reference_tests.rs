use super::*;

fn routing(account: &str) -> CodexInstanceModelRouting {
    CodexInstanceModelRouting {
        enabled: false,
        routes: vec![crate::models::CodexInstanceApiRoute {
            id: "route".into(),
            namespace: "relay".into(),
            provider_account_id: account.into(),
            enabled: true,
            selected_models: None,
            extra_models: None,
        }],
        ..Default::default()
    }
}

#[test]
fn key_rotation_migrates_default_and_managed_gateway_bindings_and_routes() {
    let old = "codex_apikey_old";
    let new = "codex_apikey_new";
    let mut store = InstanceStore::new();
    store.default_settings.bind_account_id = provider_gateway_bind_account_id(old);
    store.default_settings.model_routing = Some(routing(old));
    store.instances.push(InstanceProfile {
        id: "instance".into(),
        name: "test".into(),
        user_data_dir: "synthetic".into(),
        working_dir: None,
        extra_args: String::new(),
        bind_account_id: Some(old.into()),
        model_routing: Some(routing(old)),
        launch_mode: InstanceLaunchMode::App,
        app_speed: CodexAppSpeed::default(),
        created_at: 0,
        last_launched_at: None,
        last_pid: None,
    });
    assert!(replace_instance_store_account_references(
        &mut store, old, new
    ));
    assert_eq!(
        store.default_settings.bind_account_id,
        provider_gateway_bind_account_id(new)
    );
    assert_eq!(store.instances[0].bind_account_id.as_deref(), Some(new));
    assert_eq!(
        store
            .default_settings
            .model_routing
            .as_ref()
            .unwrap()
            .routes[0]
            .provider_account_id,
        new
    );
    assert_eq!(
        store.instances[0].model_routing.as_ref().unwrap().routes[0].provider_account_id,
        new
    );
    assert!(
        !store
            .default_settings
            .model_routing
            .as_ref()
            .unwrap()
            .enabled
    );
    assert!(!replace_instance_store_account_references(
        &mut store, old, new
    ));
    store.instances[0].bind_account_id = provider_gateway_bind_account_id(old);
    assert!(replace_instance_store_account_references(
        &mut store, old, new
    ));
    assert_eq!(
        store.instances[0].bind_account_id,
        provider_gateway_bind_account_id(new)
    );
}

#[test]
fn key_rotation_preserves_unrelated_and_api_service_bindings() {
    let mut store = InstanceStore::new();
    store.default_settings.bind_account_id = Some(CODEX_API_SERVICE_BIND_ACCOUNT_ID.into());
    store.default_settings.model_routing = Some(routing("unrelated"));
    assert!(!replace_instance_store_account_references(
        &mut store, "old", "new"
    ));
    assert_eq!(
        store.default_settings.bind_account_id.as_deref(),
        Some(CODEX_API_SERVICE_BIND_ACCOUNT_ID)
    );
}
