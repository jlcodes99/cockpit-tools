use super::*;

#[tokio::test]
async fn model_list_forbidden_uses_quota_summary_and_keeps_ambiguous_denials_retryable() {
    use axum::{http::StatusCode, routing::post, Json, Router};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    for (model_message, summary_response, expect_summary, expect_forbidden, expect_quota) in [
        (
            "Permission denied",
            (StatusCode::OK, json!({"groups":[{"buckets":[
                {"bucketId":"gemini-5h","remainingFraction":0.0,"resetTime":"2099-01-01T00:00:00Z"},
                {"bucketId":"gemini-weekly","remainingFraction":0.5}
            ]}]})),
            true, false, true,
        ),
        (
            "Permission denied",
            (StatusCode::FORBIDDEN, json!({"error":{"message":"Permission denied"}})),
            true, false, false,
        ),
        (
            "Permission denied",
            (StatusCode::OK, json!({"groups":[]})),
            true, false, false,
        ),
        (
            "Permission denied",
            (StatusCode::FORBIDDEN, json!({"error":{"message":"Verify your account"}})),
            true, true, false,
        ),
        (
            "Subscription required",
            (StatusCode::OK, json!({"groups":[]})),
            false, true, false,
        ),
    ] {
        let summary_requests = Arc::new(AtomicUsize::new(0));
        let counter = summary_requests.clone();
        let app = Router::new()
            .route(&format!("/{FETCH_AVAILABLE_MODELS_PATH}"), post(move || async move {
                (StatusCode::FORBIDDEN, Json(json!({"error":{"message":model_message}})))
            }))
            .route("/v1internal:retrieveUserQuotaSummary", post(move || {
                let response = summary_response.clone();
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    (response.0, Json(response.1))
                }
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let meta = ProjectMetadataResult {
            project_id: None,
            subscription_tier: Some("PRO".to_string()),
            credits: vec![CreditInfo {
                credit_type: "AI".to_string(),
                credit_amount: Some("25".to_string()),
                minimum_credit_amount_for_usage: None,
            }],
            is_gcp_tos: None,
        };
        let result = fetch_quota_with_project_metadata(
            "synthetic-access-token",
            "quota-fixture@example.test",
            true,
            &QuotaCloudCodeContext::default(),
            &meta,
            &base_url,
        )
        .await
        .unwrap();
        server.abort();

        assert_eq!(summary_requests.load(Ordering::SeqCst), usize::from(expect_summary));
        assert_eq!(result.quota.is_forbidden, expect_forbidden);
        assert_eq!(result.quota.subscription_tier.as_deref(), Some("PRO"));
        assert_eq!(result.quota.credits[0].credit_amount.as_deref(), Some("25"));
        if expect_quota {
            assert!(result.error.is_none());
            assert_eq!(result.quota.models[0].percentage, 0);
            assert_eq!(result.quota.models[0].reset_time, "2099-01-01T00:00:00Z");
            assert_eq!(result.quota.models[1].percentage, 50);
        } else {
            assert_eq!(result.error.unwrap().code, Some(403));
            assert!(result.quota.models.is_empty());
            if !expect_forbidden {
                assert!(result.quota.quota_summary_stale);
                assert!(result.quota.quota_summary_updated_at.is_none());
            }
        }
    }
}

fn previous() -> QuotaData {
    let mut quota = QuotaData::new();
    quota.last_updated = 100;
    quota.quota_summary_updated_at = Some(90);
    quota.add_model("gemini-5h".into(), None, 25, "later".into());
    quota.add_model("gemini-argon-wtu-weekly".into(), None, 91, "weekly-reset".into());
    quota.add_model("gemini-3-pro-high".into(), None, 60, String::new());
    quota
}

#[test]
fn failed_summary_keeps_real_windows_but_not_old_model_quotas() {
    let mut next = QuotaData::new();
    next.quota_summary_stale = true;
    next.add_model("gemini-3-pro-high".into(), None, 10, String::new());
    preserve_failed_quota_summary(&mut next, Some(&previous()));
    assert_eq!(next.models.len(), 3);
    assert_eq!(next.models[0].percentage, 10);
    assert_eq!(next.models[1].percentage, 25);
    assert_eq!(next.models[2].name, "gemini-argon-wtu-weekly");
    assert_eq!(next.models[2].percentage, 91);
    assert!(next.quota_summary_stale);
    assert_eq!(next.quota_summary_updated_at, Some(90));
    preserve_failed_quota_summary(&mut next, Some(&previous()));
    assert_eq!(next.models.len(), 3);
}

#[test]
fn cloud_code_user_agent_uses_aidev_client_format_with_cl() {
    let ua = build_cloud_code_user_agent();
    assert!(ua.contains("aidev_client"), "expected aidev_client in UA: {ua}");
    assert!(ua.contains("cl=1003475615"), "expected cl=1003475615 in UA: {ua}");
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
