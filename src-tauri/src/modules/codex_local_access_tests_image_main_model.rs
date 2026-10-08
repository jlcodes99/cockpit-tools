#[test]
fn image_main_model_legacy_collection_remains_opt_in() {
    let collection = test_local_access_collection(Vec::new());
    let value = serde_json::to_value(&collection).unwrap();
    assert!(value.get("imageGenerationMainModel").is_none());
    let restored: super::CodexLocalAccessCollection = serde_json::from_value(value).unwrap();
    assert_eq!(restored.image_generation_main_model, None);
    assert_eq!(restored.image_generation_model, super::DEFAULT_CODEX_IMAGE_GENERATION_MODEL);
}

#[test]
fn image_main_model_validates_trims_and_resets_without_touching_tool() {
    assert_eq!(super::normalize_image_generation_main_model(Some(" gpt-custom-image-main ".into())).unwrap(), Some("gpt-custom-image-main".into()));
    for empty in [None, Some(String::new()), Some("  ".into())] {
        assert_eq!(super::normalize_image_generation_main_model(empty).unwrap(), None);
    }
    for invalid in ["gpt model".to_string(), "gpt\nmodel".into(), "x".repeat(201), "gpt\0bad".into()] {
        assert!(super::normalize_image_generation_main_model(Some(invalid)).is_err());
    }
    let mut collection = test_local_access_collection(Vec::new());
    collection.image_generation_main_model = Some(" gpt-custom-image-main ".into());
    super::sanitize_collection_structure(&mut collection).unwrap();
    assert_eq!(collection.image_generation_main_model.as_deref(), Some("gpt-custom-image-main"));
    assert_eq!(collection.image_generation_model, super::DEFAULT_CODEX_IMAGE_GENERATION_MODEL);
    let restored: super::CodexLocalAccessCollection = serde_json::from_value(serde_json::to_value(collection).unwrap()).unwrap();
    assert_eq!(restored.image_generation_main_model.as_deref(), Some("gpt-custom-image-main"));
}

#[test]
fn image_main_model_survives_instance_template_and_sidecar_manifest() {
    let mut collection = test_local_access_collection(Vec::new());
    let mut template = collection.clone();
    template.image_generation_main_model = Some("gpt-custom-image-main".into());
    super::apply_provider_gateway_template_settings(&mut collection, &template);
    assert_eq!(collection.image_generation_main_model, template.image_generation_main_model);
    let dir = make_temp_dir("image-main-manifest");
    super::prepare_sidecar_launch_config_in_dir_sync(&collection, dir.clone(), HashMap::new(), None, HashMap::new(), true, None).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&fs::read_to_string(super::sidecar_manifest_path(&dir)).unwrap()).unwrap();
    assert_eq!(manifest["imageGenerationMainModel"], "gpt-custom-image-main");
    assert_eq!(manifest["imageGenerationModel"], super::DEFAULT_CODEX_IMAGE_GENERATION_MODEL);
    fs::remove_dir_all(dir).unwrap();
}
