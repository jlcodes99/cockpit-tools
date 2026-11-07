use super::sidecar_quota_reserve_snapshot_value;
// Included in the sidecar gateway test module; fixtures use no real accounts.

    #[test]
    fn sidecar_fingerprint_ignores_dynamic_quota_snapshot_changes() {
        let config = r#"{"host":"127.0.0.1","port":58393}"#;
        let manifest_available_high = r#"{
          "accounts": [{
            "id": "account-a",
            "quotaReserve": {
              "hourlyThresholdPercent": 10,
              "weeklyThresholdPercent": 20,
              "snapshotUpdatedAtUnixSeconds": 100,
              "hourlyRemainingPercent": 50,
              "weeklyRemainingPercent": 60,
              "hourlyWindowPresent": true,
              "weeklyWindowPresent": true
            }
          }]
        }"#;
        let manifest_available_low = r#"{
          "accounts": [{
            "id": "account-a",
            "quotaReserve": {
              "hourlyThresholdPercent": 10,
              "weeklyThresholdPercent": 20,
              "snapshotUpdatedAtUnixSeconds": 100,
              "hourlyRemainingPercent": 11,
              "weeklyRemainingPercent": 21,
              "hourlyWindowPresent": true,
              "weeklyWindowPresent": true
            }
          }]
        }"#;
        let manifest_new_snapshot = r#"{
          "accounts": [{
            "id": "account-a",
            "quotaReserve": {
              "hourlyThresholdPercent": 10,
              "weeklyThresholdPercent": 20,
              "snapshotUpdatedAtUnixSeconds": 200,
              "hourlyRemainingPercent": 11,
              "weeklyRemainingPercent": 21,
              "hourlyWindowPresent": true,
              "weeklyWindowPresent": true
            }
          }]
        }"#;
        let manifest_blocked = r#"{
          "accounts": [{
            "id": "account-a",
            "quotaReserve": {
              "hourlyThresholdPercent": 10,
              "weeklyThresholdPercent": 20,
              "snapshotUpdatedAtUnixSeconds": 100,
              "hourlyRemainingPercent": 10,
              "weeklyRemainingPercent": 21,
              "hourlyWindowPresent": true,
              "weeklyWindowPresent": true
            }
          }]
        }"#;
        let manifest_threshold_changed = r#"{
          "accounts": [{
            "id": "account-a",
            "quotaReserve": {
              "hourlyThresholdPercent": 15,
              "weeklyThresholdPercent": 20,
              "snapshotUpdatedAtUnixSeconds": 100,
              "hourlyRemainingPercent": 50,
              "weeklyRemainingPercent": 60,
              "hourlyWindowPresent": true,
              "weeklyWindowPresent": true
            }
          }]
        }"#;

        assert_eq!(
            sidecar_config_fingerprint(config, manifest_available_high),
            sidecar_config_fingerprint(config, manifest_available_low)
        );
        assert_eq!(
            sidecar_config_fingerprint(config, manifest_available_low),
            sidecar_config_fingerprint(config, manifest_new_snapshot)
        );
        assert_eq!(
            sidecar_config_fingerprint(config, manifest_available_low),
            sidecar_config_fingerprint(config, manifest_blocked)
        );
        assert_ne!(
            sidecar_config_fingerprint(config, manifest_available_high),
            sidecar_config_fingerprint(config, manifest_threshold_changed)
        );
    }

    #[test]
    fn bound_oauth_quota_reserve_blocks_at_threshold_and_fails_closed() {
        let reserve = CodexLocalAccessQuotaReserve {
            hourly_percent: 20,
            weekly_percent: 10,
            strict: false, safety_margin_percent: 3,
};
        let blocked =
            test_oauth_account_with_quota("account-bound", 20, 90, Some(true), Some(true));
        assert!(bound_oauth_quota_reserve_blocks_account(
            &reserve,
            Some(&blocked)
        ));

        let available =
            test_oauth_account_with_quota("reserve-confirmed-bound", 24, 14, Some(true), Some(true));
        {
            super::quota_reserve_coordinator().lock().unwrap().accounts.entry(available.id.clone()).or_default();
        }
        super::confirm_gateway_quota_refresh(&available.id, available.quota.as_ref().unwrap(), std::time::Instant::now());
        assert!(!bound_oauth_quota_reserve_blocks_account(&reserve, Some(&available)));

        let ignored_hourly =
            test_oauth_account_with_quota("reserve-confirmed-bound", 0, 14, Some(false), Some(true));
        super::confirm_gateway_quota_refresh(&ignored_hourly.id, ignored_hourly.quota.as_ref().unwrap(), std::time::Instant::now());
        assert!(!bound_oauth_quota_reserve_blocks_account(&reserve, Some(&ignored_hourly)));
        assert!(bound_oauth_quota_reserve_blocks_account(&reserve, None));

        let mut quota_error = available;
        quota_error.quota_error = Some(CodexQuotaErrorInfo {
            code: Some("quota_refresh_failed".to_string()),
            message: "refresh failed".to_string(),
            timestamp: 1,
        });
        assert!(bound_oauth_quota_reserve_blocks_account(
            &reserve,
            Some(&quota_error)
        ));

        let mut stale =
            test_oauth_account_with_quota("account-stale", 80, 80, Some(true), Some(true));
        stale.usage_updated_at = Some(
            chrono::Utc::now().timestamp() - BOUND_OAUTH_QUOTA_RESERVE_MAX_SNAPSHOT_AGE_SECONDS - 1,
        );
        assert!(bound_oauth_quota_reserve_blocks_account(
            &reserve,
            Some(&stale)
        ));

        let mut missing_timestamp =
            test_oauth_account_with_quota("account-missing", 80, 80, Some(true), Some(true));
        missing_timestamp.usage_updated_at = None;
        assert!(bound_oauth_quota_reserve_blocks_account(
            &reserve,
            Some(&missing_timestamp)
        ));

        let mut future_timestamp =
            test_oauth_account_with_quota("account-future", 80, 80, Some(true), Some(true));
        future_timestamp.usage_updated_at = Some(chrono::Utc::now().timestamp() + 60);
        assert!(bound_oauth_quota_reserve_blocks_account(
            &reserve,
            Some(&future_timestamp)
        ));

        let transient =
            test_oauth_account_with_quota("account-transient", 80, 80, Some(true), Some(true));
        bound_oauth_quota_refresh_failures()
            .lock()
            .unwrap()
            .insert(transient.id.clone());
        assert!(bound_oauth_quota_reserve_blocks_account(
            &reserve,
            Some(&transient)
        ));
        bound_oauth_quota_refresh_failures()
            .lock()
            .unwrap()
            .remove(&transient.id);
    }

    #[test]
    fn bound_oauth_quota_reserve_filters_only_the_bound_account() {
        let reserve = CodexLocalAccessQuotaReserve {
            hourly_percent: 20,
            weekly_percent: 10,
            strict: false, safety_margin_percent: 3,
};
        let blocked =
            test_oauth_account_with_quota("account-bound", 20, 90, Some(true), Some(true));
        let scoped = vec!["account-bound".to_string(), "account-other".to_string()];

        assert_eq!(
            filter_bound_oauth_quota_reserve_account(
                scoped,
                "account-bound",
                &reserve,
                Some(&blocked),
            ),
            vec!["account-other"]
        );
    }

    #[test]
    fn sidecar_manifest_keeps_thresholds_and_snapshot_state_is_separate() {
        let mut collection = test_local_access_collection(vec!["account-bound".to_string()]);
        collection.bound_oauth_account_id = Some("account-bound".to_string());
        collection.bound_oauth_quota_reserve = Some(CodexLocalAccessQuotaReserve {
            hourly_percent: 20,
            weekly_percent: 10,
            strict: false, safety_margin_percent: 3,
});
        let account =
            test_oauth_account_with_quota("account-bound", 75, 40, Some(true), Some(false));

        let manifest = sidecar_account_manifest_value(&account, Some("auth.json"), &collection);
        let reserve = manifest
            .get("quotaReserve")
            .expect("quota reserve should exist");
        assert_eq!(reserve["hourlyThresholdPercent"], json!(23));
        assert_eq!(reserve["weeklyThresholdPercent"], json!(13));
        assert!(reserve.get("snapshotUpdatedAtUnixSeconds").is_none());
        assert!(reserve.get("hourlyRemainingPercent").is_none());

        let snapshot = sidecar_quota_reserve_snapshot_value(&collection, &account)
            .expect("quota reserve snapshot should exist");
        assert_eq!(
            snapshot["snapshotUpdatedAtUnixSeconds"],
            json!(account.usage_updated_at)
        );
        assert_eq!(snapshot["hourlyRemainingPercent"], json!(75));
        assert_eq!(snapshot["weeklyRemainingPercent"], json!(40));
        assert_eq!(snapshot["hourlyWindowPresent"], json!(true));
        assert_eq!(snapshot["weeklyWindowPresent"], json!(false));
    }

    #[test]
    fn sidecar_fingerprint_ignores_all_dynamic_reserve_states() {
        let config = r#"{"host":"127.0.0.1","port":58393}"#;
        let manifest = |remaining: Value| {
            json!({
                "accounts": [{
                    "id": "account-a",
                    "quotaReserve": {
                        "hourlyThresholdPercent": 20,
                        "weeklyThresholdPercent": 10,
                        "hourlyRemainingPercent": remaining,
                        "weeklyRemainingPercent": 80,
                        "hourlyWindowPresent": true,
                        "weeklyWindowPresent": true
                    }
                }]
            })
            .to_string()
        };
        let available_80 = manifest(json!(80));
        let available_70 = manifest(json!(70));
        let blocked_20 = manifest(json!(20));
        let unknown = manifest(Value::Null);

        assert_eq!(
            sidecar_config_fingerprint(config, &available_80),
            sidecar_config_fingerprint(config, &available_70)
        );
        assert_eq!(
            sidecar_config_fingerprint(config, &available_70),
            sidecar_config_fingerprint(config, &blocked_20)
        );
        assert_eq!(
            sidecar_config_fingerprint(config, &blocked_20),
            sidecar_config_fingerprint(config, &unknown)
        );
    }
