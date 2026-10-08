fn explicit_long_override() -> super::CodexLocalAccessModelPricing {
    let mut pricing = super::model_pricing(
        "gpt-5.5",
        Some(272_000),
        super::codex_price(5.0, 0.5, 30.0),
        Some(super::codex_price(7.0, 0.7, 40.0)),
        Some(super::codex_price(10.0, 1.0, 60.0)),
        None,
    );
    pricing.standard_long_price_override = true;
    pricing
}

fn long_override_usage(input_tokens: u64) -> super::UsageCapture {
    super::UsageCapture {
        input_tokens,
        ..super::UsageCapture::default()
    }
}

#[test]
fn long_price_override_standard_and_flex_ignore_other_tiers_priority_configuration() {
    let normalized = super::normalize_model_pricings(vec![explicit_long_override()]);
    let stored: super::CodexLocalAccessModelPricing =
        serde_json::from_value(serde_json::to_value(&normalized[0]).unwrap()).unwrap();
    let usage = long_override_usage(272_001);
    for (tier, expected) in [(None, (7.0, 0.7, 40.0)), (Some("flex"), (3.5, 0.35, 20.0))] {
        let actual =
            super::compute_effective_unit_prices(&stored, &stored.model_id, Some(&usage), tier);
        assert!(super::prices_close(
            actual.input_usd_per_million,
            expected.0
        ));
        assert!(super::prices_close(
            actual.cached_input_usd_per_million,
            expected.1
        ));
        assert!(super::prices_close(
            actual.output_usd_per_million,
            expected.2
        ));
    }
    let short = super::compute_effective_unit_prices(
        &stored,
        &stored.model_id,
        Some(&long_override_usage(272_000)),
        None,
    );
    assert_eq!(short.input_usd_per_million, 5.0);
}

#[test]
fn long_price_override_priority_preserves_absolute_rates_or_composes_multiplier() {
    let mut pricing = explicit_long_override();
    let usage = long_override_usage(300_000);
    let priority = super::compute_effective_unit_prices(
        &pricing,
        &pricing.model_id,
        Some(&usage),
        Some("priority"),
    );
    assert_eq!(priority.input_usd_per_million, 20.0);
    assert_eq!(priority.cached_input_usd_per_million, 2.0);
    assert_eq!(priority.output_usd_per_million, 90.0);
    pricing.priority_input_usd_per_million = None;
    pricing.priority_cached_input_usd_per_million = None;
    pricing.priority_output_usd_per_million = None;
    let priority = super::compute_effective_unit_prices(
        &pricing,
        &pricing.model_id,
        Some(&usage),
        Some("priority"),
    );
    assert_eq!(priority.input_usd_per_million, 14.0);
    assert_eq!(priority.cached_input_usd_per_million, 1.4);
    assert_eq!(priority.output_usd_per_million, 80.0);
}

#[test]
fn long_price_override_zero_and_partial_values_are_preserved() {
    let mut pricing = explicit_long_override();
    pricing.standard_long_input_usd_per_million = Some(0.0);
    pricing.standard_long_cached_input_usd_per_million = None;
    pricing.standard_long_output_usd_per_million = Some(0.0);
    let normalized = super::normalize_model_pricings(vec![pricing]);
    assert!(normalized[0].standard_long_price_override);
    let actual = super::compute_effective_unit_prices(
        &normalized[0],
        "gpt-5.5",
        Some(&long_override_usage(300_000)),
        None,
    );
    assert_eq!(actual.input_usd_per_million, 0.0);
    assert_eq!(actual.cached_input_usd_per_million, 1.0);
    assert_eq!(actual.output_usd_per_million, 0.0);
}

#[test]
fn long_price_override_legacy_configs_keep_derivation_when_base_changes() {
    let mut json = serde_json::to_value(explicit_long_override()).unwrap();
    json.as_object_mut()
        .unwrap()
        .remove("standardLongPriceOverride");
    let mut legacy: super::CodexLocalAccessModelPricing = serde_json::from_value(json).unwrap();
    assert!(!legacy.standard_long_price_override);
    legacy.input_usd_per_million = 8.0;
    let normalized = super::normalize_model_pricings(vec![legacy]);
    assert_eq!(
        normalized[0].standard_long_input_usd_per_million,
        Some(16.0)
    );
    assert!(serde_json::to_value(&normalized[0])
        .unwrap()
        .get("standardLongPriceOverride")
        .is_none());
}

#[test]
fn long_price_override_clear_and_migration_preserve_user_intent() {
    let mut pricing = explicit_long_override();
    pricing.model_id = "gpt-5.6-sol".into();
    assert_eq!(
        super::drop_superseded_default_56_model_pricings(vec![pricing.clone()]).len(),
        1
    );
    let mut derived = pricing.clone();
    derived.standard_long_price_override = false;
    assert_eq!(
        super::changed_model_pricing_ids(&[derived], &[pricing.clone()]),
        vec!["gpt-5.6-sol"]
    );
    pricing.standard_long_input_usd_per_million = None;
    pricing.standard_long_cached_input_usd_per_million = None;
    pricing.standard_long_output_usd_per_million = None;
    assert!(!super::normalize_model_pricings(vec![pricing])[0].standard_long_price_override);
}

#[test]
fn long_price_override_custom_model_uses_explicit_threshold() {
    let mut pricing = explicit_long_override();
    pricing.model_id = "custom-model".into();
    let normalized = super::normalize_model_pricings(vec![pricing]);
    assert_eq!(normalized[0].long_context_threshold_tokens, Some(272_000));
    assert_eq!(
        super::compute_effective_unit_prices(
            &normalized[0],
            "custom-model",
            Some(&long_override_usage(272_001)),
            None
        )
        .input_usd_per_million,
        7.0
    );
}
