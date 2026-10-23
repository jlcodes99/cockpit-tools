#[test]
fn image_main_model_legacy_collection_remains_opt_in() {
    let collection = test_local_access_collection(Vec::new());
    let value = serde_json::to_value(&collection).unwrap();
    assert!(value.get("imageGenerationMainModel").is_none());
    let restored: super::CodexLocalAccessCollection = serde_json::from_value(value).unwrap();
    assert_eq!(restored.image_generation_main_model, None);
    assert_eq!(
        super::effective_image_generation_main_model(None),
        "gpt-5.6-luna"
    );
    assert_eq!(
        restored.image_generation_model,
        super::DEFAULT_CODEX_IMAGE_GENERATION_MODEL
    );
}

#[test]
fn image_main_model_luna_migrates_only_the_old_image_driver_and_preserves_explicit_responses() {
    for legacy in ["gpt-5.5", " GPT-5.5 "] {
        let mut collection = test_local_access_collection(Vec::new());
        collection.image_generation_main_model = Some(legacy.into());
        let image_model = collection.image_generation_model.clone();
        assert_eq!(
            super::native_image_main_model(&collection, None, true),
            "gpt-5.6-luna"
        );
        assert_eq!(
            super::native_image_main_model(&collection, Some("gpt-5.5"), false),
            "gpt-5.5"
        );
        assert!(super::sanitize_collection_structure(&mut collection).unwrap());
        assert_eq!(
            collection.image_generation_main_model.as_deref(),
            Some("gpt-5.6-luna")
        );
        assert_eq!(collection.image_generation_model, image_model);
        let mut profile = test_local_access_collection(Vec::new());
        // Test profiles can copy an unsanitized global template.
        collection.image_generation_main_model = Some(legacy.into());
        super::apply_provider_gateway_template_settings(&mut profile, &collection);
        assert_eq!(
            profile.image_generation_main_model.as_deref(),
            Some("gpt-5.6-luna")
        );
    }
    for custom in [
        "gpt-custom-image-main",
        "gpt-5.5-custom",
        "vendor/gpt-5.5",
        "gpt-5.6-luna",
    ] {
        assert_eq!(
            super::normalize_image_generation_main_model(Some(custom.into()))
                .unwrap()
                .as_deref(),
            Some(custom)
        );
    }
}

#[test]
fn image_main_model_luna_migration_is_durable_and_preserves_newer_custom_and_reset() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("collection.json");
    let mut legacy = test_local_access_collection(Vec::new());
    legacy.image_generation_main_model = Some("gpt-5.5".into());
    fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let read = || -> super::CodexLocalAccessCollection {
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap()
    };
    let mut loaded = read();
    super::sanitize_collection_structure(&mut loaded).unwrap();
    super::save_collection_to_path(&path, &loaded).unwrap();
    assert_eq!(
        read().image_generation_main_model.as_deref(),
        Some("gpt-5.6-luna")
    );
    super::save_collection_to_path(&path, &legacy).unwrap();
    assert_eq!(
        read().image_generation_main_model.as_deref(),
        Some("gpt-5.6-luna")
    );

    for choice in [Some("gpt-custom-image-main".to_owned()), None] {
        let published =
            super::persist_image_model_settings(&path, &legacy, None, Some(choice.clone()), None)
                .unwrap();
        let mut stale = legacy.clone();
        stale.debug_logs = !published.debug_logs;
        super::save_collection_to_path(&path, &stale).unwrap();
        assert_eq!(read().image_generation_main_model, choice);
        assert_eq!(read().debug_logs, stale.debug_logs);
        super::preserve_runtime_collection_fields(&mut stale, &published);
        assert_eq!(stale.image_generation_main_model, choice);
        assert_eq!(stale.image_generation_model, legacy.image_generation_model);
    }
}

#[test]
fn image_main_model_luna_catalog_and_source_transactions_migrate_before_first_load() {
    for source_registration in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("collection.json");
        let mut legacy = test_local_access_collection(Vec::new());
        legacy.image_generation_main_model = Some(" gpt-5.5 ".into());
        let mut document = serde_json::to_value(&legacy).unwrap();
        document["futureSetting"] = serde_json::json!({"keep":true});
        fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        let published = super::persist_image_model_settings_with_sources(
            &path,
            &legacy,
            None,
            None,
            None,
            source_registration.then(|| vec![legacy.image_generation_model.clone()]),
        )
        .unwrap();
        assert_eq!(
            published.image_generation_main_model.as_deref(),
            Some("gpt-5.6-luna")
        );
        let saved: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["imageGenerationMainModel"], "gpt-5.6-luna");
        assert_eq!(saved["futureSetting"], document["futureSetting"]);
        assert_eq!(
            saved["imageGenerationModel"],
            document["imageGenerationModel"]
        );
        let mut runtime = super::GatewayRuntime::default();
        runtime.collection = Some(legacy.clone());
        super::publish_image_model_settings(&mut runtime, &published, false);
        assert_eq!(
            runtime
                .collection
                .as_ref()
                .unwrap()
                .image_generation_main_model
                .as_deref(),
            Some("gpt-5.6-luna")
        );
        let again = super::persist_image_model_settings(&path, &legacy, None, None, None).unwrap();
        assert_eq!(
            again.image_generation_main_model,
            published.image_generation_main_model
        );
        assert_eq!(again.updated_at, published.updated_at);
    }
}

#[test]
fn image_main_model_validates_trims_and_resets_without_touching_tool() {
    assert_eq!(
        super::normalize_image_generation_main_model(Some(" gpt-custom-image-main ".into()))
            .unwrap(),
        Some("gpt-custom-image-main".into())
    );
    for empty in [None, Some(String::new()), Some("  ".into())] {
        assert_eq!(
            super::normalize_image_generation_main_model(empty).unwrap(),
            None
        );
    }
    for invalid in [
        "gpt model".to_string(),
        "gpt\nmodel".into(),
        "x".repeat(201),
        "gpt\0bad".into(),
    ] {
        assert!(super::normalize_image_generation_main_model(Some(invalid)).is_err());
    }
    let mut collection = test_local_access_collection(Vec::new());
    collection.image_generation_main_model = Some(" gpt-custom-image-main ".into());
    super::sanitize_collection_structure(&mut collection).unwrap();
    assert_eq!(
        collection.image_generation_main_model.as_deref(),
        Some("gpt-custom-image-main")
    );
    assert_eq!(
        collection.image_generation_model,
        super::DEFAULT_CODEX_IMAGE_GENERATION_MODEL
    );
    let restored: super::CodexLocalAccessCollection =
        serde_json::from_value(serde_json::to_value(collection).unwrap()).unwrap();
    assert_eq!(
        restored.image_generation_main_model.as_deref(),
        Some("gpt-custom-image-main")
    );
}

#[test]
fn image_main_model_survives_instance_template_and_sidecar_manifest() {
    let mut collection = test_local_access_collection(Vec::new());
    let mut template = collection.clone();
    template.image_generation_main_model = Some("gpt-custom-image-main".into());
    super::apply_provider_gateway_template_settings(&mut collection, &template);
    assert_eq!(
        collection.image_generation_main_model,
        template.image_generation_main_model
    );
    let dir = make_temp_dir("image-main-manifest");
    super::prepare_sidecar_launch_config_in_dir_sync(
        &collection,
        dir.clone(),
        HashMap::new(),
        None,
        HashMap::new(),
        true,
        None,
    )
    .unwrap();
    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(super::sidecar_manifest_path(&dir)).unwrap())
            .unwrap();
    assert_eq!(
        manifest["imageGenerationMainModel"],
        "gpt-custom-image-main"
    );
    assert_eq!(
        manifest["imageGenerationModel"],
        super::DEFAULT_CODEX_IMAGE_GENERATION_MODEL
    );
    fs::remove_dir_all(dir).unwrap();
}
