#[test]
fn floating_appearance_patch_preserves_defaults_and_other_settings() {
    let mut config = UserConfig::default();
    assert!(!config.floating_card_minimal);
    assert_eq!(config.floating_card_background_opacity, 1.0);
    config.floating_card_always_on_top = true;
    config.floating_card_position_x = Some(125);
    let patch = serde_json::json!({"floating_card_minimal": true});
    apply_general_config_updates(&mut config, patch.as_object().unwrap()).unwrap();
    assert!(config.floating_card_minimal);
    assert_eq!(config.floating_card_background_opacity, 1.0);
    let patch = serde_json::json!({"floating_card_background_opacity": 0.4});
    apply_general_config_updates(&mut config, patch.as_object().unwrap()).unwrap();
    assert!(config.floating_card_minimal);
    assert_eq!(config.floating_card_background_opacity, 0.4);
    assert!(config.floating_card_always_on_top);
    assert_eq!(config.floating_card_position_x, Some(125));
    let restored: UserConfig = serde_json::from_value(serde_json::to_value(config).unwrap()).unwrap();
    assert!(restored.floating_card_minimal);
    assert_eq!(restored.floating_card_background_opacity, 0.4);
}

#[test]
fn floating_appearance_old_config_and_opacity_bounds_are_safe() {
    let mut value = serde_json::to_value(UserConfig::default()).unwrap();
    value.as_object_mut().unwrap().remove("floating_card_minimal");
    value.as_object_mut().unwrap().remove("floating_card_background_opacity");
    let mut config: UserConfig = serde_json::from_value(value).unwrap();
    assert!(!config.floating_card_minimal);
    assert_eq!(config.floating_card_background_opacity, 1.0);
    for (input, expected) in [(-1.0, 0.0), (2.0, 1.0), (0.0, 0.0)] {
        let patch = serde_json::json!({"floating_card_background_opacity": input});
        apply_general_config_updates(&mut config, patch.as_object().unwrap()).unwrap();
        assert_eq!(config.floating_card_background_opacity, expected);
    }
    let before = config.clone();
    let patch = serde_json::json!({"floating_card_minimal": "invalid"});
    assert!(apply_general_config_updates(&mut config, patch.as_object().unwrap()).is_err());
    assert_eq!(config.floating_card_minimal, before.floating_card_minimal);
}
