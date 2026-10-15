use super::*;

fn previous() -> QuotaData {
    let mut quota = QuotaData::new();
    quota.last_updated = 100;
    quota.quota_summary_updated_at = Some(90);
    quota.add_model("gemini-5h".into(), None, 25, "later".into());
    quota.add_model("gemini-3-pro-high".into(), None, 60, String::new());
    quota
}

#[test]
fn failed_summary_keeps_real_windows_but_not_old_model_quotas() {
    let mut next = QuotaData::new();
    next.quota_summary_stale = true;
    next.add_model("gemini-3-pro-high".into(), None, 10, String::new());
    preserve_failed_quota_summary(&mut next, Some(&previous()));
    assert_eq!(next.models.len(), 2);
    assert_eq!(next.models[0].percentage, 10);
    assert_eq!(next.models[1].percentage, 25);
    assert!(next.quota_summary_stale);
    assert_eq!(next.quota_summary_updated_at, Some(90));
    preserve_failed_quota_summary(&mut next, Some(&previous()));
    assert_eq!(next.models.len(), 2);
}

#[test]
fn failed_summary_never_crosses_project_or_tier_and_never_masks_forbidden() {
    for variant in 0..4 {
        let mut next = QuotaData::new();
        next.quota_summary_stale = true;
        match variant {
            0 => next.project_id = Some("other-project".into()),
            1 => next.subscription_tier = Some("other-tier".into()),
            2 => next.is_forbidden = true,
            _ => next.quota_summary_stale = false,
        }
        preserve_failed_quota_summary(&mut next, Some(&previous()));
        assert!(next.models.is_empty());
        assert_eq!(next.quota_summary_updated_at, None);
    }
}

#[test]
fn summary_failure_is_distinct_from_valid_empty_summary() {
    for (summary, stale) in [(None, true), (Some(json!({"bad":true})), true),
        (Some(json!({"groups":[{"buckets":[{"bucketId":"gemini-5h"}]}]})), true),
        (Some(json!({"groups":[]})), false)] {
        let quota = build_quota_data_from_response(
            QuotaResponse { models: Default::default() }, None, vec![], summary, None, None,
        );
        assert_eq!(quota.quota_summary_stale, stale);
        assert_eq!(quota.quota_summary_updated_at.is_none(), stale);
    }
}

#[test]
fn successful_summary_preserves_actual_percentage_and_reset() {
    let quota = build_quota_data_from_response(QuotaResponse { models: Default::default() },
        None, vec![], Some(json!({"groups":[{"buckets":[{
            "bucketId":"gemini-5h", "remainingFraction":0.25, "resetTime":"2099-01-01T00:00:00Z"
        }]}]})), None, None);
    assert_eq!(quota.models[0].percentage, 25);
    assert_eq!(quota.models[0].reset_time, "2099-01-01T00:00:00Z");
    assert!(!quota.quota_summary_stale);
}
