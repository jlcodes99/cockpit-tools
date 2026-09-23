// Codex Local Access 测试：上游 x-codex-turn-state 观测、账号风控状态推导与请求日志落库。
// 与其它测试分片共享 tests 作用域，父模块项目统一用 `super::` 前缀访问。

fn turn_state_observation(
    observed_at: i64,
    class: &str,
    length: Option<i64>,
    status: Option<u16>,
    reason: Option<&str>,
) -> super::CodexTurnStateObservation {
    super::CodexTurnStateObservation {
        observed_at,
        source: "manual".to_string(),
        class: class.to_string(),
        length,
        http_status: status,
        reason: reason.map(str::to_string),
    }
}

#[test]
fn turn_state_header_classification_matches_relay_baseline() {
    assert_eq!(
        super::observe_turn_state_header_value(None),
        (None, super::CODEX_TURN_STATE_CLASS_MISSING)
    );
    assert_eq!(
        super::observe_turn_state_header_value(Some("   ")),
        (Some(0), super::CODEX_TURN_STATE_CLASS_ABNORMAL)
    );
    for length in [292_usize, 332, 780] {
        let value = format!("{}{}", "gAAAAA", "x".repeat(length - 6));
        assert_eq!(
            super::observe_turn_state_header_value(Some(value.as_str())),
            (Some(length as i64), super::CODEX_TURN_STATE_CLASS_NORMAL)
        );
    }
    // 312 直接就是疑似风控信号。
    let suspected = format!("{}{}", "gAAAAA", "x".repeat(306));
    assert_eq!(
        super::observe_turn_state_header_value(Some(suspected.as_str())),
        (Some(312), super::CODEX_TURN_STATE_CLASS_SUSPECTED)
    );
    // 长度正确但缺少 `gAAAAA` 前缀：按异常处理，避免把非 state 值当正常。
    let mismatched_prefix = format!("{}{}", "bAAAAA", "x".repeat(286));
    assert_eq!(
        super::observe_turn_state_header_value(Some(mismatched_prefix.as_str())),
        (Some(292), super::CODEX_TURN_STATE_CLASS_ABNORMAL)
    );
    let short = format!("{}{}", "gAAAAA", "x".repeat(100));
    assert_eq!(
        super::observe_turn_state_header_value(Some(short.as_str())),
        (Some(106), super::CODEX_TURN_STATE_CLASS_ABNORMAL)
    );
}

#[test]
fn turn_state_observation_reason_only_uses_state_length() {
    assert_eq!(
        super::turn_state_observation_reason(super::CODEX_TURN_STATE_CLASS_SUSPECTED).as_deref(),
        Some(super::CODEX_TURN_STATE_REASON_SUSPECTED)
    );
    assert_eq!(
        super::turn_state_observation_reason(super::CODEX_TURN_STATE_CLASS_MISSING).as_deref(),
        Some(super::CODEX_TURN_STATE_REASON_MISSING)
    );
    assert_eq!(
        super::turn_state_observation_reason(super::CODEX_TURN_STATE_CLASS_ABNORMAL).as_deref(),
        Some(super::CODEX_TURN_STATE_REASON_ABNORMAL)
    );
    assert_eq!(
        super::turn_state_observation_reason(super::CODEX_TURN_STATE_CLASS_NORMAL),
        None
    );
}

#[test]
fn single_suspected_state_marks_account_suspected() {
    let suspected = super::derive_codex_account_turn_state_status(
        "acc-1",
        &[turn_state_observation(
            1_700_000_000_000,
            super::CODEX_TURN_STATE_CLASS_SUSPECTED,
            Some(312),
            Some(200),
            Some(super::CODEX_TURN_STATE_REASON_SUSPECTED),
        )],
    );
    assert_eq!(suspected.status, super::CODEX_TURN_STATE_STATUS_SUSPECTED);
    assert!(suspected.suspected);
    assert_eq!(
        suspected.reason.as_deref(),
        Some(super::CODEX_TURN_STATE_REASON_SUSPECTED)
    );

    // 任何一次 292/332/780 都会摘除疑似标记。
    let recovered = super::derive_codex_account_turn_state_status(
        "acc-1",
        &[
            turn_state_observation(
                1_700_000_000_000,
                super::CODEX_TURN_STATE_CLASS_SUSPECTED,
                Some(312),
                Some(200),
                Some(super::CODEX_TURN_STATE_REASON_SUSPECTED),
            ),
            turn_state_observation(
                1_700_000_060_000,
                super::CODEX_TURN_STATE_CLASS_NORMAL,
                Some(332),
                Some(200),
                None,
            ),
        ],
    );
    assert_eq!(recovered.status, super::CODEX_TURN_STATE_STATUS_NORMAL);
    assert!(!recovered.suspected);
}

#[test]
fn abnormal_or_missing_state_never_escalates_to_suspected() {
    let abnormal = super::derive_codex_account_turn_state_status(
        "acc-4",
        &[
            turn_state_observation(
                1_700_000_000_000,
                super::CODEX_TURN_STATE_CLASS_ABNORMAL,
                Some(200),
                Some(200),
                Some(super::CODEX_TURN_STATE_REASON_ABNORMAL),
            ),
            turn_state_observation(
                1_700_000_060_000,
                super::CODEX_TURN_STATE_CLASS_MISSING,
                None,
                Some(200),
                Some(super::CODEX_TURN_STATE_REASON_MISSING),
            ),
        ],
    );
    assert_eq!(abnormal.status, super::CODEX_TURN_STATE_STATUS_ABNORMAL);
    assert!(!abnormal.suspected);
    assert_eq!(
        abnormal.reason.as_deref(),
        Some(super::CODEX_TURN_STATE_REASON_MISSING)
    );
}

#[test]
fn legacy_renew_class_reads_as_suspected() {
    assert_eq!(
        super::normalize_turn_state_class(Some("renew")),
        Some(super::CODEX_TURN_STATE_CLASS_SUSPECTED)
    );
    assert_eq!(
        super::normalize_turn_state_class(Some("Suspected ")),
        Some(super::CODEX_TURN_STATE_CLASS_SUSPECTED)
    );
}

#[test]
fn turn_state_status_unknown_without_observations() {
    let status = super::derive_codex_account_turn_state_status("acc-3", &[]);
    assert_eq!(status.status, super::CODEX_TURN_STATE_STATUS_UNKNOWN);
    assert!(!status.suspected);
    assert!(status.observations.is_empty());
}

#[test]
fn request_logs_persist_turn_state_length_and_class() {
    let dir = make_temp_dir("codex-turn-state-log");
    let db_path = dir.join("request_logs.sqlite");
    let conn = super::open_local_access_logs_db_once(&db_path, true).expect("open logs db");
    let mut events = Vec::new();
    let persisted = super::append_usage_event_with_turn_state(
        &mut events,
        1_700_000_000_000,
        Some("req-turn-state"),
        Some("acc-1"),
        Some("user@example.com"),
        None,
        None,
        None,
        Some("gpt-5.5"),
        Some(super::CodexLocalAccessGatewayMode::Sidecar),
        super::CodexLocalAccessRequestKind::Text,
        None,
        None,
        None,
        None,
        true,
        Some(200),
        None,
        None,
        120,
        None,
        None,
        1,
        0.0,
        Some(312),
        Some(super::CODEX_TURN_STATE_CLASS_SUSPECTED),
    );
    super::insert_local_access_usage_event(&conn, &persisted).expect("insert request log");

    let loaded = conn
        .query_row(
            "SELECT * FROM request_logs WHERE request_id = ?1",
            ["req-turn-state"],
            super::usage_event_from_row,
        )
        .expect("read request log");
    assert_eq!(loaded.turn_state_length, Some(312));
    assert_eq!(
        loaded.turn_state_class.as_deref(),
        Some(super::CODEX_TURN_STATE_CLASS_SUSPECTED)
    );

    drop(conn);
    let _ = super::fs::remove_dir_all(dir);
}
