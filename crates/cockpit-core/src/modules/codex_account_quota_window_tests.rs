mod quota_window_threshold_tests {
    use super::super::*;

    fn quota_account(quota: serde_json::Value) -> CodexAccount {
        let tokens = CodexTokens {
            access_token: "access".into(),
            id_token: "identity".into(),
            refresh_token: Some("refresh".into()),
        };
        let mut account =
            CodexAccount::new("quota-test".into(), "quota@example.com".into(), tokens);
        account.quota = Some(serde_json::from_value(quota).unwrap());
        account
    }

    #[test]
    fn primary_weekly_only_uses_weekly_threshold_for_trigger_and_candidate() {
        let mut account = quota_account(serde_json::json!({
            "hourly_percentage": 30,
            "weekly_percentage": 0,
            "hourly_window_minutes": 10080,
            "hourly_window_present": true,
            "weekly_window_present": false
        }));
        account.plan_type = Some("pro".into());
        let metrics = extract_quota_metrics(&account);
        assert_eq!(metrics.len(), 1);
        assert!(metric_crossed_threshold(&metrics[0], 10, 30));
        assert!(build_switch_candidate(&account, 10, 30).is_none());
        let candidate = build_switch_candidate(&account, 80, 20).unwrap();
        assert_eq!(candidate.min_margin, 10);
        assert_eq!(candidate.min_percentage, 30);
    }

    #[test]
    fn secondary_weekly_only_ignores_missing_short_cycle() {
        let account = quota_account(serde_json::json!({
            "hourly_percentage": 0,
            "weekly_percentage": 60,
            "weekly_window_minutes": 10080,
            "hourly_window_present": false,
            "weekly_window_present": true
        }));
        let metrics = extract_quota_metrics(&account);
        assert_eq!(metrics.len(), 1);
        assert!(!metric_crossed_threshold(&metrics[0], 80, 20));
        assert_eq!(
            build_switch_candidate(&account, 80, 20).unwrap().min_margin,
            40
        );
    }

    #[test]
    fn swapped_windows_use_duration_for_both_thresholds_and_ranking() {
        let account = quota_account(serde_json::json!({
            "hourly_percentage": 40,
            "weekly_percentage": 70,
            "hourly_window_minutes": 10080,
            "weekly_window_minutes": 300,
            "hourly_window_present": true,
            "weekly_window_present": true
        }));
        let metrics = extract_quota_metrics(&account);
        assert!(metrics
            .iter()
            .all(|metric| !metric_crossed_threshold(metric, 60, 20)));
        assert_eq!(
            build_switch_candidate(&account, 60, 20).unwrap().min_margin,
            10
        );
        assert!(metric_crossed_threshold(&metrics[0], 10, 40));
        assert!(metric_crossed_threshold(&metrics[1], 70, 20));
    }

    #[test]
    fn normal_dual_windows_require_both_candidate_quotas_above_threshold() {
        let account = quota_account(serde_json::json!({
            "hourly_percentage": 60,
            "weekly_percentage": 20,
            "hourly_window_minutes": 300,
            "weekly_window_minutes": 10080,
            "hourly_window_present": true,
            "weekly_window_present": true
        }));
        let metrics = extract_quota_metrics(&account);
        assert!(!metric_crossed_threshold(&metrics[0], 50, 20));
        assert!(metric_crossed_threshold(&metrics[1], 50, 20));
        assert!(build_switch_candidate(&account, 50, 20).is_none());
        assert_eq!(
            build_switch_candidate(&account, 50, 10).unwrap().min_margin,
            10
        );
    }

    #[test]
    fn candidate_ranking_uses_actual_window_threshold_margin() {
        let mut weekly = quota_account(serde_json::json!({
            "hourly_percentage": 70,
            "weekly_percentage": 0,
            "hourly_window_minutes": 10080,
            "hourly_window_present": true,
            "weekly_window_present": false
        }));
        weekly.id = "weekly".into();
        let mut short = weekly.clone();
        short.id = "short".into();
        short.quota.as_mut().unwrap().hourly_window_minutes = Some(300);
        let candidates = vec![
            build_switch_candidate(&short, 60, 20).unwrap(),
            build_switch_candidate(&weekly, 60, 20).unwrap(),
        ];
        assert_eq!(pick_best_candidate(candidates).unwrap().id, "weekly");
    }

    #[test]
    fn absent_or_invalid_duration_preserves_legacy_field_thresholds() {
        for minutes in [None, Some(0), Some(-1)] {
            let account = quota_account(serde_json::json!({
                "hourly_percentage": 30,
                "weekly_percentage": 60,
                "hourly_window_minutes": minutes,
                "weekly_window_minutes": minutes
            }));
            let metrics = extract_quota_metrics(&account);
            assert_eq!(metrics.len(), 2);
            assert!(metric_crossed_threshold(&metrics[0], 30, 20));
            assert!(!metric_crossed_threshold(&metrics[1], 30, 20));
            assert_eq!(
                build_switch_candidate(&account, 10, 20).unwrap().min_margin,
                20
            );
        }
    }

    #[test]
    fn weekly_duration_boundary_matches_window_labels() {
        for (minutes, expected) in [
            (300, 10),
            (10078, 10),
            (10079, 30),
            (10080, 30),
            (20160, 30),
        ] {
            let metric = CodexQuotaMetric {
                key: "primary_window",
                label: format_codex_quota_metric_label(Some(minutes), "5h"),
                percentage: 20,
                window_minutes: Some(minutes),
            };
            assert_eq!(quota_metric_threshold(&metric, 10, 30), Some(expected));
        }
    }

    #[test]
    fn no_present_windows_cannot_trigger_or_become_candidate() {
        let account = quota_account(serde_json::json!({
            "hourly_percentage": 0,
            "weekly_percentage": 0,
            "hourly_window_present": false,
            "weekly_window_present": false
        }));
        assert!(extract_quota_metrics(&account).is_empty());
        assert!(build_switch_candidate(&account, 20, 20).is_none());
        let mut account = account;
        account.quota = None;
        assert!(extract_quota_metrics(&account).is_empty());
    }
}
