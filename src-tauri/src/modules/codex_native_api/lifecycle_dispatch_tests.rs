fn dispatch_route(account_id: &str) -> Route {
    Route {
        account_id: account_id.into(),
        continuity_account_id: None,
        owner_id: account_id.into(),
        installation_profile: None,
        transport_identity: Some(format!("transport-{account_id}")),
        official_authorization_revision: None,
        official_registration_id: None,
        official_fallback_registration_id: None,
        official_fallback_authorization_revision: None,
        authorization_scope_fingerprint: None,
        official_fallback_failure: None,
        base_url: "http://127.0.0.1:1/v1".into(),
        model: format!("model-{account_id}"),
        headers: axum::http::HeaderMap::new(),
        client: reqwest::Client::new(),
        proxy_url: None,
        proxy_route: None,
        chatgpt: true,
        chatgpt_plan: false,
        wire_api: super::super::contract::WireApi::Responses,
        compact_endpoint: None,
        websocket_endpoint: None,
        default_service_tier: None,
        permit: None,
    }
}

#[test]
fn native_lifecycle_tracker_stack_state_stays_bounded() {
    let tracker = std::mem::size_of::<RequestTracker>();
    let report = std::mem::size_of::<RequestReport>();
    eprintln!("RequestTracker={tracker} bytes; RequestReport={report} bytes");
    assert!(tracker <= 4096,
        "candidate/checkpoint state must stay off nested async frames ({tracker} bytes)");
}

fn unsent_fixture() -> (RequestTracker, Arc<Recorder>, Arc<Activity>, watch::Sender<bool>) {
    let activity = Arc::new(Activity::default());
    let host = Arc::new(Recorder::default());
    let (stop, stopped) = watch::channel(false);
    let mut tracker = RequestTracker::new(
        activity.clone(), host.clone(), StopSignal(stopped), None,
    );
    tracker.context("POST", "/v1/responses", &axum::http::HeaderMap::new(), true);
    tracker.authorized(&Caller { id: "caller".into(), label: "test".into() }, "requested");
    (tracker, host, activity, stop)
}

fn select_candidate(tracker: &mut RequestTracker, account_id: &str) {
    let caller = Caller { id: "caller".into(), label: "test".into() };
    tracker.routed(&caller, &dispatch_route(account_id), "requested");
}

async fn assert_no_dispatch_records(host: &Recorder, activity: &Activity) {
    tokio::task::yield_now().await;
    assert!(host.reports.lock().unwrap().is_empty());
    assert!(host.contexts.lock().unwrap().is_empty());
    assert!(host.quotas.lock().unwrap().is_empty());
    assert!(host.invalidations.lock().unwrap().is_empty());
    assert_eq!(activity.count.load(Ordering::SeqCst), 0);
    assert_eq!(activity.tasks.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn native_lifecycle_unsent_preparation_failures_have_no_upstream_outcome() {
    for code in ["request_rebuild_failed", "authorization_changed", "upstream_timeout"] {
        let (mut tracker, host, activity, _stop) = unsent_fixture();
        select_candidate(&mut tracker, "not-sent");
        assert_eq!(tracker.dispatch_count, 0);
        assert!(tracker.diagnostics.as_ref().unwrap().attempts.is_empty());
        let mut error = ServiceError::new(503, code, "local preparation rejected")
            .with_retry_after(Some(Duration::from_secs(90)));
        error.credential_scope = true;
        error.transport_failure = Some(super::super::contract::TransportFailureKind::TlsDecrypt);
        tracker.fail(&error).await;
        drop(tracker);
        assert_no_dispatch_records(&host, &activity).await;
    }
}

#[tokio::test]
async fn native_lifecycle_dispatch_evidence_does_not_depend_on_diagnostic_capture() {
    let (mut tracker, host, _activity, _stop) = unsent_fixture();
    tracker.diagnostics = None;
    select_candidate(&mut tracker, "sent-without-diagnostics");
    tracker.begin_attempt("http");
    tracker.complete(200, true, None, None).await;
    let report = recorded(&host).await;
    assert_eq!(report.account_id, "sent-without-diagnostics");
    assert_eq!(report.status, 200);
    assert!(report.diagnostics.is_none());
}

#[tokio::test]
async fn native_lifecycle_cancelled_unsent_candidate_releases_permits_without_records() {
    for stopping in [false, true] {
        let (mut tracker, host, activity, stop) = unsent_fixture();
        let gate = Arc::new(Semaphore::new(1));
        let mut candidate = dispatch_route("not-sent");
        candidate.permit = Some(Arc::new(gate.clone().acquire_owned().await.unwrap()));
        tracker.routed(&Caller { id: "caller".into(), label: "test".into() }, &candidate, "requested");
        drop(candidate);
        if stopping { stop.send(true).unwrap(); }
        drop(tracker);
        assert_eq!(gate.available_permits(), 1);
        assert_no_dispatch_records(&host, &activity).await;
    }
}

#[tokio::test]
async fn native_lifecycle_first_dispatch_commits_only_the_selected_candidate() {
    let (mut tracker, host, _activity, _stop) = unsent_fixture();
    select_candidate(&mut tracker, "discarded");
    select_candidate(&mut tracker, "sent");
    tracker.request_metadata(&json!({"service_tier":"priority","reasoning":{"effort":"high"}}));
    tracker.begin_attempt("http");
    // Reinforcing the current dispatch cannot create a duplicate attempt.
    tracker.begin_attempt("http");
    assert_eq!(tracker.dispatch_count, 1);
    assert_eq!(tracker.quota_account.as_deref(), Some("sent"));
    tracker.complete(200, true, Some(usage()), None).await;
    let report = recorded(&host).await;
    assert_eq!(report.account_id, "sent");
    assert_eq!(report.requested_service_tier.as_deref(), Some("priority"));
    assert_eq!(report.reasoning_effort.as_deref(), Some("high"));
    let context = report.diagnostics.unwrap();
    assert_eq!(context.attempts.len(), 1);
    assert_eq!(context.attempts[0].account_id, "sent");
    assert_eq!(context.attempts[0].transport, "http");
    assert_eq!(context.attempts[0].status, Some(200));
}

#[tokio::test]
async fn native_lifecycle_real_429_survives_rejected_or_cancelled_next_candidate() {
    for cancelled in [false, true] {
        let (mut tracker, host, _activity, _stop) = unsent_fixture();
        select_candidate(&mut tracker, "sent");
        tracker.begin_attempt("http");
        let real_error = ServiceError::new(429, "upstream_rate_limited", "actual upstream rejection")
            .with_retry_after(Some(Duration::from_secs(30)));
        tracker.attempt_error(&real_error);
        tracker.set_retry_after(real_error.retry_after);
        select_candidate(&mut tracker, "never-sent");
        tracker.request_metadata(&json!({"service_tier":"not-sent"}));
        if cancelled {
            drop(tracker);
        } else {
            tracker.fail(&ServiceError::new(503, "authorization_changed", "candidate rejected")
                .with_retry_after(Some(Duration::from_secs(90)))).await;
            drop(tracker);
        }
        let report = recorded(&host).await;
        assert_eq!(report.account_id, "sent");
        assert_eq!(report.upstream_model, "model-sent");
        assert_eq!(report.status, 429);
        assert_eq!(report.error.as_deref(), Some("upstream_rate_limited"));
        assert_eq!(report.error_message.as_deref(), Some("actual upstream rejection"));
        assert_eq!(report.retry_after, Some(Duration::from_secs(30)));
        assert_eq!(report.requested_service_tier, None);
        let context = report.diagnostics.unwrap();
        assert_eq!(context.attempts.len(), 1);
        assert_eq!(context.attempts[0].account_id, "sent");
        assert_eq!(context.attempts[0].status, Some(429));
        assert!(!context.attempts[0].success);
    }
}

#[tokio::test]
async fn native_lifecycle_local_fallback_and_handshake_do_not_create_attempts() {
    let (mut tracker, host, activity, _stop) = unsent_fixture();
    let error = ServiceError::new(426, "websocket_unavailable", "negotiate HTTP locally");
    tracker.authorization_fallback_unavailable("not-sent", "model",
        &super::super::contract::OfficialFallbackFailure {
            trigger_stage: "preflight".into(), status: 403, code: "authorization_changed".into(),
            message: None, retry_after: None,
        }, &error);
    assert!(tracker.diagnostics.as_ref().unwrap().attempts.is_empty());
    tracker.transport_fallback(&error).await;
    assert_eq!(tracker.dispatch_count, 0);
    drop(tracker);
    assert_no_dispatch_records(&host, &activity).await;
}

#[tokio::test]
async fn native_lifecycle_new_turn_resets_upstream_dispatch_evidence() {
    let (mut tracker, host, activity, _stop) = unsent_fixture();
    select_candidate(&mut tracker, "sent");
    tracker.begin_attempt("http");
    tracker.complete(200, true, None, None).await;
    let report = recorded(&host).await;
    assert_eq!(report.status, 200);
    tokio::time::timeout(Duration::from_secs(1), async {
        while activity.tasks.load(Ordering::SeqCst) > 1 { tokio::task::yield_now().await; }
    }).await.expect("previous turn's persistence finished");
    host.contexts.lock().unwrap().clear();
    tracker.start_turn().unwrap();
    tracker.context("GET", "/v1/responses", &axum::http::HeaderMap::new(), false);
    select_candidate(&mut tracker, "new-unsent");
    assert_eq!(tracker.dispatch_count, 0);
    tracker.fail(&ServiceError::new(503, "request_rebuild_failed", "new turn failed locally")).await;
    drop(tracker);
    assert_no_dispatch_records(&host, &activity).await;
}

#[tokio::test]
async fn native_lifecycle_unsent_connect_rollback_preserves_previous_real_429() {
    let (mut tracker, host, _activity, _stop) = unsent_fixture();
    select_candidate(&mut tracker, "sent");
    tracker.begin_attempt("http");
    tracker.payload("upstream_request", "http", &axum::http::HeaderMap::new(), "{\"real\":true}");
    let throttled = ServiceError::new(429, "upstream_rate_limited", "real throttle")
        .with_retry_after(Some(Duration::from_secs(30)));
    tracker.attempt_error(&throttled);
    tracker.set_retry_after(throttled.retry_after);
    let quota_started = tracker.quota_started;
    assert!(!tracker.discard_unsent_attempt(), "a settled upstream rejection cannot be discarded");
    select_candidate(&mut tracker, "connect-failed");
    tracker.begin_attempt("http");
    tracker.payload("upstream_request", "http", &axum::http::HeaderMap::new(), "{\"unsent\":true}");
    assert_eq!(tracker.upstream_attempt_count(), 2);
    assert!(tracker.discard_unsent_attempt());
    assert!(!tracker.discard_unsent_attempt(), "rollback is idempotent");
    assert!(tracker.has_upstream_attempt());
    assert_eq!(tracker.upstream_attempt_count(), 1);
    assert_eq!(tracker.quota_account.as_deref(), Some("sent"));
    assert_eq!(tracker.quota_started, quota_started);
    assert_eq!(tracker.transport_identity.as_deref(), Some("transport-sent"));
    let error = ServiceError::new(502, "upstream_unavailable", "connection refused before request send");
    tracker.fail(&error).await;
    let report = recorded(&host).await;
    assert_eq!(report.account_id, "sent");
    assert_eq!(report.status, 429);
    assert_eq!(report.error.as_deref(), Some("upstream_rate_limited"));
    assert_eq!(report.error_message.as_deref(), Some("real throttle"));
    assert_eq!(report.retry_after, Some(Duration::from_secs(30)));
    let context = report.diagnostics.unwrap();
    assert_eq!(context.attempts.len(), 1);
    assert_eq!(context.attempts[0].account_id, "sent");
    assert_eq!(context.attempts[0].status, Some(429));
    assert_eq!(context.payloads.len(), 1);
    assert_eq!(context.payloads[0].body, "{\"real\":true}");
}

#[tokio::test]
async fn native_lifecycle_first_connect_failure_can_retry_without_a_phantom_attempt() {
    for replace_candidate in [false, true] {
        let (mut tracker, host, _activity, _stop) = unsent_fixture();
        tracker.payload("client_request", "http", &axum::http::HeaderMap::new(), "{\"client\":true}");
        select_candidate(&mut tracker, "first");
        tracker.begin_attempt("http");
        tracker.payload("upstream_request", "http", &axum::http::HeaderMap::new(), "{\"unsent\":true}");
        assert!(tracker.discard_unsent_attempt());
        assert!(!tracker.has_upstream_attempt());
        assert_eq!(tracker.upstream_attempt_count(), 0);
        assert!(tracker.diagnostics.as_ref().unwrap().attempts.is_empty());
        assert_eq!(tracker.diagnostics.as_ref().unwrap().payloads.len(), 1);
        assert!(host.reports.lock().unwrap().is_empty());
        assert!(host.contexts.lock().unwrap().is_empty());
        if replace_candidate { select_candidate(&mut tracker, "second"); }
        // Without replacing the route, rollback retains the selected candidate
        // as pending and restores all metadata at the next actual dispatch.
        tracker.begin_attempt("http");
        assert_eq!(tracker.upstream_attempt_count(), 1);
        tracker.complete(200, true, None, None).await;
        let report = recorded(&host).await;
        let expected = if replace_candidate { "second" } else { "first" };
        assert_eq!(report.account_id, expected);
        assert_eq!(report.status, 200);
        let context = report.diagnostics.unwrap();
        assert_eq!(context.attempts.len(), 1);
        assert_eq!(context.attempts[0].account_id, expected);
        assert_eq!(context.attempts[0].sequence, 1);
        assert_eq!(context.attempts[0].status, Some(200));
        assert_eq!(context.payloads.len(), 1);
        assert_eq!(context.payloads[0].stage, "client_request");
    }
}

#[tokio::test]
async fn native_lifecycle_first_connect_failure_then_termination_is_unrecorded() {
    let (mut tracker, host, activity, _stop) = unsent_fixture();
    select_candidate(&mut tracker, "never-sent");
    tracker.begin_attempt("http");
    assert!(tracker.discard_unsent_attempt());
    tracker.fail(&ServiceError::new(502, "upstream_unavailable", "all connections refused")).await;
    drop(tracker);
    assert_no_dispatch_records(&host, &activity).await;
}

#[tokio::test]
async fn quota_protection_fences_the_pending_candidate_without_committing_a_send() {
    let (mut tracker, host, _activity, _stop) = unsent_fixture();
    select_candidate(&mut tracker, "discarded");
    select_candidate(&mut tracker, "selected");
    host.quota_denied.lock().unwrap().push("selected".into());
    let error = tracker.check_quota_before_send().unwrap_err();
    assert_eq!(error.code, "quota_reserved");
    assert!(error.upstream_not_sent);
    assert_eq!(*host.quota_checks.lock().unwrap(), ["selected"]);
    assert_eq!(tracker.dispatch_count, 0);
    assert!(host.reports.lock().unwrap().is_empty());

    host.quota_denied.lock().unwrap().clear();
    tracker.check_quota_before_send().unwrap();
    tracker.begin_attempt("http");
    let mut official = dispatch_route("official-selected");
    official.chatgpt_plan = true;
    tracker.routed(&Caller { id: "caller".into(), label: "test".into() }, &official, "requested");
    tracker.check_quota_before_send().unwrap();
    assert_eq!(*host.quota_checks.lock().unwrap(), ["selected", "selected"],
        "an independent official route must not inherit the prior local quota fence");

    select_candidate(&mut tracker, "retry-selected");
    host.quota_denied.lock().unwrap().push("retry-selected".into());
    assert_eq!(tracker.check_quota_before_send().unwrap_err().code, "quota_reserved");
    assert_eq!(host.quota_checks.lock().unwrap().last().unwrap(), "retry-selected");
}
