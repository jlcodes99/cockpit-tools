use super::*;

fn quota(hourly: i32, weekly: i32) -> CodexQuota {
    serde_json::from_value(
        serde_json::json!({"hourly_percentage": hourly, "weekly_percentage": weekly}),
    )
    .unwrap()
}

#[test]
fn active_refresh_accelerates_at_low_quota_and_ignores_absent_windows() {
    assert_eq!(dynamic_interval(None), MIN_INTERVAL);
    assert_eq!(
        dynamic_interval(Some(&quota(100, 100))),
        Duration::from_secs(30)
    );
    assert_eq!(
        dynamic_interval(Some(&quota(50, 90))),
        Duration::from_secs(15)
    );
    assert_eq!(dynamic_interval(Some(&quota(100, 10))), MIN_INTERVAL);
    let mut missing = quota(0, 100);
    missing.hourly_window_present = Some(false);
    assert_eq!(dynamic_interval(Some(&missing)), Duration::from_secs(30));
}

#[test]
fn group_disabled_wins_and_custom_group_survives_disabled_platform() {
    let policy = (
        false,
        HashSet::from(["custom".into(), "both".into()]),
        HashSet::from(["disabled".into(), "both".into()]),
    );
    assert!(!policy_allows("inherited", &policy));
    assert!(policy_allows("custom", &policy));
    assert!(!policy_allows("both", &policy));
    assert!(!policy_allows("disabled", &policy));
    assert!(policy_allows("inherited", &(true, policy.1, policy.2)));
}

#[test]
fn settings_handshake_account_exclusions_and_plan_scope_are_preserved() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    struct Restore(Option<HashSet<String>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            *DISABLED_EMAILS.lock().unwrap() = self.0.take();
        }
    }
    let _restore = Restore(DISABLED_EMAILS.lock().unwrap().take());
    let mut account = CodexAccount::new(
        "sync-policy".into(),
        "policy@example.invalid".into(),
        crate::models::codex::CodexTokens {
            access_token: "local-fixture".into(),
            id_token: "local-fixture".into(),
            refresh_token: None,
        },
    );
    account.plan_type = Some("plus".into());
    let mut settings = config::UserConfig::default();
    let inherited = (true, HashSet::new(), HashSet::new());
    assert!(!eligible(&account, &settings, &inherited)); // No policy handshake yet.
    set_codex_quota_sync_disabled_emails(vec![]).unwrap();
    assert!(eligible(&account, &settings, &inherited));
    // A retired opt-out cannot disable automatic synchronization.
    let legacy: config::UserConfig = serde_json::from_value(serde_json::json!({
        "codex_quota_sync_enabled": false
    })).unwrap();
    assert!(eligible(&account, &legacy, &inherited));
    set_codex_quota_sync_disabled_emails(vec![account.email.clone()]).unwrap();
    assert!(!eligible(&account, &settings, &inherited));
    let custom = (false, HashSet::from([account.id.clone()]), HashSet::new());
    assert!(eligible(&account, &settings, &custom)); // Existing group priority.
    settings.codex_auto_refresh_plan_types = vec![];
    assert!(!eligible(&account, &settings, &custom));
    settings.codex_auto_refresh_plan_types = vec!["plus".into()];
    account.auth_mode = crate::models::codex::CodexAuthMode::Apikey;
    assert!(!eligible(&account, &settings, &custom));
}

#[test]
fn plan_filter_matches_frontend_compound_and_unknown_plans() {
    for (plan, key) in [
        ("PRO-LITE", "pro"),
        ("ChatGPT Business", "business"),
        ("education", "edu_k12"),
        ("proPlus", "unknown"),
        ("", "unknown"),
        ("Free", "free"),
    ] {
        assert_eq!(plan_key(Some(plan)), key);
    }
}

#[test]
fn activity_is_coalesced_single_flight_and_does_not_bypass_backoff() {
    let now = Instant::now();
    let mut state = RefreshState::default();
    assert!(state.due(now));
    state.in_flight = true;
    state.last_started = Some(now);
    state.activity_pending = true;
    assert!(!state.due(now + Duration::from_secs(10)));
    state.finish(
        now + Duration::from_secs(1),
        &Err("429 [retry_after_seconds:60]".into()),
    );
    assert!(!state.due(now + Duration::from_secs(59)));
    assert!(state.due(now + Duration::from_secs(61)));
    state.finish(now + Duration::from_secs(62), &Ok(quota(100, 100)));
    assert!(state.due(now + Duration::from_secs(63)));
    state.last_started = Some(now + Duration::from_secs(63));
    assert!(!state.due(now + Duration::from_secs(64)));
}

#[test]
fn manual_quota_observation_defers_periodic_poll_without_replacing_retry_with_old_evidence() {
    let now = Instant::now();
    let mut state = RefreshState::default();
    state.observe(now, Duration::from_secs(30));
    assert!(!state.due(now));
    state.finish(
        now + Duration::from_secs(2),
        &Err("429 [retry_after_seconds:60]".into()),
    );
    state.observe(now, MIN_INTERVAL);
    assert!(!state.due(now + Duration::from_secs(59)));
    state.observe(now + Duration::from_secs(60), Duration::from_secs(10));
    assert_eq!(state.failures, 0);
    assert!(!state.due(now + Duration::from_secs(61)));
}

#[test]
fn idle_accounts_leave_fast_polling_and_activity_does_not_immediately_repeat_manual_get() {
    let now = Instant::now();
    let mut state = RefreshState {
        last_activity: Some(now),
        ..Default::default()
    };
    assert!(state.due(now));
    assert!(!state.due(now + ACTIVE_WINDOW + Duration::from_secs(1)));
    state.last_activity = Some(now + ACTIVE_WINDOW);
    state.activity_pending = true;
    state.observe(now + ACTIVE_WINDOW, Duration::from_secs(30));
    assert!(!state.due(now + ACTIVE_WINDOW + Duration::from_secs(4)));
    assert!(state.due(now + ACTIVE_WINDOW + Duration::from_secs(5)));
}

#[test]
fn retry_after_and_backoff_are_bounded() {
    assert_eq!(failure_backoff(1, "network"), Duration::from_secs(10));
    assert_eq!(failure_backoff(20, "network"), Duration::from_secs(300));
    assert_eq!(
        failure_backoff(1, "429 [retry_after_seconds:90]"),
        Duration::from_secs(90)
    );
    assert_eq!(
        failure_backoff(1, "429 [retry_after_seconds:9999999]"),
        Duration::from_secs(3600)
    );
    assert_eq!(
        failure_backoff(1, "bad [retry_after_seconds:invalid]"),
        Duration::from_secs(10)
    );
}

#[test]
fn session_probe_ignores_cold_history_and_detects_append_rotation_and_truncation() {
    let dir = std::env::temp_dir().join(format!("quota-sync-probe-{}", uuid::Uuid::new_v4()));
    let now = chrono::Utc::now();
    let sessions = dir
        .join("sessions")
        .join(now.format("%Y/%m/%d").to_string());
    std::fs::create_dir_all(&sessions).unwrap();
    let file = sessions.join("rollout-fixture.jsonl");
    std::fs::write(&file, "old quota, not parsed\n").unwrap();
    let mut probe = ActivityProbe::default();
    assert!(!probe.scan(&dir, now));
    assert!(!probe.scan(&dir, now));
    std::fs::write(&file, "old quota, not parsed\nnew activity\n").unwrap();
    assert!(probe.scan(&dir, now));
    assert!(!probe.scan(&dir, now));
    std::fs::write(&file, "truncated\n").unwrap();
    assert!(probe.scan(&dir, now));
    std::fs::write(sessions.join("rollout-new.jsonl"), "new activity\n").unwrap();
    assert!(probe.scan(&dir, now));
    std::fs::remove_file(&file).unwrap();
    assert!(!probe.scan(&dir, now));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn resumed_old_session_is_located_with_read_only_index_and_busy_db_can_fall_back() {
    let home = std::env::temp_dir().join(format!("quota-sync-old-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(home.join("sessions/2020/01/01")).unwrap();
    let path = home.join("sessions/2020/01/01/rollout-resumed.jsonl");
    std::fs::write(&path, "baseline\n").unwrap();
    let db = rusqlite::Connection::open(home.join("state_5.sqlite")).unwrap();
    db.execute_batch("CREATE TABLE threads(id TEXT, rollout_path TEXT, updated_at INTEGER); CREATE INDEX idx_threads_updated_at ON threads(updated_at DESC, id DESC);").unwrap();
    db.execute(
        "INSERT INTO threads VALUES ('thread', 'sessions/2020/01/01/rollout-resumed.jsonl', 1)",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO threads VALUES ('invalid', '../outside.jsonl', 2)",
        [],
    )
    .unwrap();
    let mut probe = ActivityProbe::default();
    assert_eq!(recent_rollout_paths(&home), vec![path.clone()]);
    assert!(!probe.scan(&home, chrono::Utc::now()));
    std::fs::write(&path, "baseline\nresumed activity\n").unwrap();
    assert!(probe.scan(&home, chrono::Utc::now()));
    db.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let started = Instant::now();
    assert!(recent_rollout_paths(&home).is_empty());
    assert!(started.elapsed() < Duration::from_secs(1));
    db.execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM threads", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    drop(db);
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn quota_event_serialization_contains_no_credentials() {
    let payload = QuotaUpdated {
        account_id: "local-id".into(),
        remote_account_id: Some("workspace".into()),
        organization_id: None,
        token_generation: 4,
        quota: quota(30, 50),
        plan_type: Some("plus".into()),
        usage_updated_at: 123,
        sequence: 1,
    };
    let json = serde_json::to_value(payload).unwrap();
    assert!(json.get("tokens").is_none());
    assert!(json.get("access_token").is_none());
    assert_eq!(json["quota"]["hourly_percentage"], 30);
    assert!(serde_json::to_value(config::UserConfig::default()).unwrap().get("codex_quota_sync_enabled").is_none());
}
