#[test]
fn codex_refresh_scope_patch_is_independent_and_validates_types() {
    let mut config = UserConfig::default();
    let original_minutes = config.codex_auto_refresh_minutes;
    for selection in [serde_json::json!(["plus", "pro"]), serde_json::json!([])] {
        let patch = serde_json::json!({"codex_auto_refresh_plan_types": selection});
        apply_general_config_updates(&mut config, patch.as_object().unwrap()).unwrap();
        assert_eq!(serde_json::to_value(&config.codex_auto_refresh_plan_types).unwrap(), selection);
        assert_eq!(config.codex_auto_refresh_minutes, original_minutes);
    }
    let before = serde_json::to_value(&config).unwrap();
    for invalid in [serde_json::json!("plus"), serde_json::json!([42]), serde_json::json!(null)] {
        let patch = serde_json::json!({"codex_auto_refresh_plan_types": invalid});
        assert!(apply_general_config_updates(&mut config, patch.as_object().unwrap()).is_err());
        assert_eq!(serde_json::to_value(&config).unwrap(), before);
    }
}
