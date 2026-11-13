#[tokio::test]
async fn native_network_deadlines_retire_the_failed_pool_without_refreshing_credentials() {
    for code in [
        "upstream_connect_timeout",
        "upstream_idle_timeout",
        "upstream_timeout",
    ] {
        let (mut tracker, host, activity, _stop) = unsent_fixture();
        select_candidate(&mut tracker, "network-account");
        tracker.begin_attempt("http");
        tracker
            .fail(&ServiceError::new(
                504,
                code,
                "Local transport deadline elapsed",
            ))
            .await;
        let report = recorded(&host).await;
        assert_eq!(
            *host.invalidations.lock().unwrap(),
            ["transport-network-account"]
        );
        assert_eq!(report.status, 504);
        assert!(!report.credential_scope);
        let context = report.diagnostics.unwrap();
        assert_eq!(
            context.attempts.len(),
            1,
            "retiring a pool must not dispatch another turn"
        );
        assert_eq!(
            context.attempts[0].transport_error.as_deref(),
            Some("network")
        );
        assert_eq!(context.attempts[0].transport_pool_retired, Some(true));
        drop(tracker);
        assert_eq!(activity.count.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn native_network_recovery_does_not_retire_pools_for_upstream_rejections_or_cancellation() {
    for (status, code) in [
        (400, "upstream_http_error"),
        (429, "upstream_rate_limited"),
        (503, "upstream_http_error"),
        (499, "client_cancelled"),
        (503, "service_stopping"),
    ] {
        let (mut tracker, host, _activity, _stop) = unsent_fixture();
        select_candidate(&mut tracker, "healthy-account");
        tracker.begin_attempt("http");
        tracker
            .fail(&ServiceError::new(
                status,
                code,
                "network timeout mentioned by remote text",
            ))
            .await;
        let report = recorded(&host).await;
        assert!(host.invalidations.lock().unwrap().is_empty());
        assert!(report.diagnostics.unwrap().attempts[0]
            .transport_pool_retired
            .is_none());
    }
}
