use super::*;
use std::path::Path;

fn read(path: &Path) -> Result<Document, String> {
    journal::read(path, Some("a"), true)
}

fn update_at<T>(
    path: &Path,
    edit: impl FnOnce(&mut Document) -> Result<T, String>,
) -> Result<T, String> {
    journal::update_account(path, "a", edit)
}

fn finish_migration(path: &Path) {
    while !journal::migrate(path).unwrap() {}
}
fn cards(count: Option<i64>, expiry: Option<i64>) -> codex_quota::CodexResetCreditsSnapshot {
    codex_quota::CodexResetCreditsSnapshot {
        available_count: count,
        credits: vec![],
        next_expires_at: expiry,
    }
}
fn quota(hourly: i32, weekly: i32) -> CodexQuota {
    CodexQuota {
        hourly_percentage: hourly,
        hourly_reset_time: Some(2000),
        hourly_window_minutes: Some(300),
        hourly_window_present: Some(true),
        weekly_percentage: weekly,
        weekly_reset_time: Some(3000),
        weekly_window_minutes: Some(10080),
        weekly_window_present: Some(true),
        reset_credits_available: Some(3),
        reset_credits: vec![],
        reset_credits_next_expires_at: None,
        raw_data: None,
    }
}
#[test]
fn inherits_public_but_custom_and_off_remain_independent() {
    let mut d = Document::default();
    assert!(effective(&d, "new").is_none());
    let enabled = Rule {
        enabled: true,
        ..Rule::default()
    };
    let custom = Rule {
        before_seconds: 60,
        ..enabled.clone()
    };
    set_account(
        &mut d,
        "custom",
        AccountRule::Custom {
            rule: custom.clone(),
        },
        &AccountRule::Inherit,
    )
    .unwrap();
    set_account(&mut d, "off", AccountRule::Off, &AccountRule::Inherit).unwrap();
    set_public(&mut d, enabled.clone(), &Rule::default()).unwrap();
    assert_eq!(effective(&d, "new"), Some(&enabled));
    assert_eq!(effective(&d, "custom"), Some(&custom));
    assert!(effective(&d, "off").is_none());
    set_public(&mut d, Rule::default(), &enabled).unwrap();
    assert!(effective(&d, "new").is_none());
    assert_eq!(effective(&d, "custom"), Some(&custom));
    set_account(
        &mut d,
        "custom",
        AccountRule::Inherit,
        &AccountRule::Custom { rule: custom },
    )
    .unwrap();
    assert!(effective(&d, "custom").is_none());
}
#[test]
fn scope_conflicts_do_not_overwrite_rules_or_block_other_accounts() {
    let mut d = Document::default();
    set_account(&mut d, "a", AccountRule::Off, &AccountRule::Inherit).unwrap();
    assert!(set_account(&mut d, "a", AccountRule::Inherit, &AccountRule::Inherit).is_err());
    set_account(&mut d, "b", AccountRule::Off, &AccountRule::Inherit).unwrap();
    assert_eq!(d.accounts["a"], AccountRule::Off);
    let old = d.public_rule.clone();
    let changed = Rule {
        before_seconds: 60,
        ..old.clone()
    };
    set_public(&mut d, changed.clone(), &old).unwrap();
    assert!(set_public(&mut d, old.clone(), &old).is_err());
    assert_eq!(d.public_rule, changed);
}
#[test]
fn expiry_is_independent_of_quota_and_never_uses_expired_cards() {
    let rule = Rule {
        enabled: true,
        before_seconds: 60,
        ..Rule::default()
    };
    assert_eq!(
        trigger(
            &rule,
            Some(&quota(100, 100)),
            &cards(Some(1), Some(1060)),
            1000
        ),
        Some("expiry")
    );
    assert_eq!(
        trigger(&rule, None, &cards(Some(1), Some(1060)), 1000),
        Some("expiry")
    );
    assert_eq!(
        trigger(&rule, None, &cards(Some(1), Some(1061)), 1000),
        None
    );
    assert_eq!(
        trigger(&rule, None, &cards(Some(1), Some(1000)), 1000),
        None
    );
    assert_eq!(trigger(&rule, None, &cards(Some(1), None), 1000), None);
    assert_eq!(
        trigger(&rule, Some(&quota(0, 0)), &cards(Some(0), Some(1010)), 1000),
        None
    );
    assert_eq!(
        trigger(&rule, Some(&quota(0, 0)), &cards(None, Some(1010)), 1000),
        None
    );
}
#[test]
fn weekly_threshold_ignores_hourly_and_preserves_unknown_expired_boundaries() {
    let mut rule = Rule {
        enabled: true,
        expiry_enabled: false,
        remaining_percent: 10,
        ..Rule::default()
    };
    let cards = cards(Some(1), None);
    assert_eq!(trigger(&rule, Some(&quota(0, 11)), &cards, 1000), None);
    assert_eq!(
        trigger(&rule, Some(&quota(100, 10)), &cards, 1000),
        Some("quota")
    );
    let mut q = quota(0, 0);
    q.weekly_window_present = Some(false);
    assert_eq!(trigger(&rule, Some(&q), &cards, 1000), None);
    q.weekly_window_present = None;
    q.weekly_reset_time = None;
    assert_eq!(trigger(&rule, Some(&q), &cards, 1000), None);
    q.weekly_window_present = Some(true);
    q.weekly_reset_time = Some(999);
    assert_eq!(trigger(&rule, Some(&q), &cards, 1000), None);
    rule.remaining_percent = 100;
    assert_eq!(
        trigger(&rule, Some(&quota(100, 100)), &cards, 1000),
        Some("quota")
    );
    rule.quota_enabled = false;
    assert_eq!(trigger(&rule, Some(&quota(0, 0)), &cards, 1000), None);
    assert!(effective(
        &Document {
            public_rule: rule,
            ..Document::default()
        },
        "a"
    )
    .is_none());
}
#[test]
fn legacy_windows_keep_switches_and_values_and_serialize_without_window() {
    for window in ["hourly", "any", "weekly"] {
        let raw = serde_json::json!({"public_rule":{"enabled":true,"expiry_enabled":false,"before_seconds":7200,"quota_enabled":true,"remaining_percent":15,"window":window},"accounts":{"a":{"mode":"custom","rule":{"enabled":false,"before_seconds":600,"remaining_percent":20,"window":window}}}});
        let d: Document = serde_json::from_value(raw).unwrap();
        assert_eq!(d.public_rule.before_seconds, 7200);
        assert_eq!(d.public_rule.remaining_percent, 15);
        assert_eq!(
            trigger(
                &d.public_rule,
                Some(&quota(0, 100)),
                &cards(Some(1), None),
                1000
            ),
            None
        );
        assert_eq!(
            trigger(
                &d.public_rule,
                Some(&quota(100, 15)),
                &cards(Some(1), None),
                1000
            ),
            Some("quota")
        );
        let AccountRule::Custom { rule } = &d.accounts["a"] else {
            panic!("custom rule lost")
        };
        assert!(!rule.enabled);
        assert_eq!(rule.before_seconds, 600);
        assert_eq!(rule.remaining_percent, 20);
        let saved = serde_json::to_value(&d).unwrap();
        assert!(saved["public_rule"].get("window").is_none());
        assert!(saved["accounts"]["a"]["rule"].get("window").is_none());
    }
}
#[test]
fn account_lease_excludes_manual_and_automatic_overlap_but_not_other_accounts() {
    let a = acquire("reset-lease-a").unwrap();
    assert!(acquire("reset-lease-a").unwrap().is_none());
    assert!(acquire("reset-lease-b").unwrap().is_some());
    drop(a);
    assert!(acquire("reset-lease-a").unwrap().is_some());
}
#[test]
fn journal_survives_restart_and_malformed_storage_is_never_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(FILE);
    update_at(&file, |d| {
        d.pending.insert(
            "a".into(),
            Pending {
                identity: "owner-a".into(),
                request_id: "stable-id".into(),
                reason: "expiry".into(),
                evidence: Some("old-cards".into()),
                unconfirmed_reported: false,
                context: None,
            },
        );
        Ok(())
    })
    .unwrap();
    let restored = read(&file).unwrap();
    assert_eq!(restored.pending["a"].request_id, "stable-id");
    std::fs::write(&file, "{damaged").unwrap();
    assert!(update_at(&file, |_| Ok(())).is_err());
    assert!(journal::migrate(&file).is_err());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "{damaged");
    std::fs::write(&file, "{}").unwrap();
    finish_migration(&file);
    assert_eq!(read(&file).unwrap().pending["a"].request_id, "stable-id");
}
#[test]
fn same_card_snapshot_is_stable_and_history_is_bounded() {
    let mut d = Document::default();
    let first = evidence(&cards(Some(3), Some(2000))).unwrap();
    d.last_evidence.insert("a".into(), first.clone());
    assert_eq!(evidence(&cards(Some(3), Some(2000))).unwrap(), first);
    assert_ne!(evidence(&cards(Some(2), Some(2000))).unwrap(), first);
    for _ in 0..150 {
        record(&mut d, "a", "expiry", "success", None);
    }
    assert_eq!(d.records.len(), 100);
}

#[test]
fn journal_keeps_success_when_other_accounts_fill_history_with_failures() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(FILE);
    let mut legacy = Document::default();
    record(&mut legacy, "a", "expiry", "success", None);
    for _ in 0..99 {
        record(&mut legacy, "b", "check", "failed", None);
    }
    std::fs::write(&file, serde_json::to_vec(&legacy).unwrap()).unwrap();

    let write_failure = || {
        journal::update_account(&file, "b", |document| {
            record(document, "b", "check", "failed", None);
            Ok(())
        })
        .unwrap();
    };
    let assert_success_retained = || {
        let records = read(&file).unwrap().records;
        assert_eq!(records.len(), 100);
        assert_eq!(records[0].account_id, "b");
        let oldest = records.last().unwrap();
        assert_eq!(oldest.account_id, "a");
        assert_eq!(oldest.status, "success");
    };

    write_failure();
    assert_success_retained();
    finish_migration(&file);
    write_failure();
    assert_success_retained();
}

#[test]
fn automatic_success_cools_both_triggers_for_one_account_even_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(FILE);
    journal::update_rules(&file, |d| {
        d.public_rule.enabled = true;
        Ok(())
    })
    .unwrap();
    let success_at = update_at(&file, |d| {
        let pending = begin_redemption(d, "a", "owner-a", "expiry", Some("two-cards".into()), None);
        finish_redemption(d, "a", &pending, Ok(ResetCreditConsumeOutcome::Reset), None)?;
        let success_at = d.last_automatic_success_ms.as_ref().unwrap()["a"];

        let manual = begin_redemption(d, "a", "owner-a", "manual", None, None);
        assert_eq!(manual.reason, "manual");
        assert_eq!(
            finish_redemption(d, "a", &manual, Ok(ResetCreditConsumeOutcome::Reset), None)?,
            None
        );
        for _ in 0..100 {
            record(d, "b", "manual", "success", None);
        }
        Ok(success_at)
    })
    .unwrap();

    let restored = read(&file).unwrap();
    assert!(restored.records.iter().all(|row| row.account_id == "b"));
    assert_eq!(
        restored.last_automatic_success_ms.as_ref().unwrap()["a"],
        success_at
    );
    let before_end = success_at + AUTOMATIC_COOLDOWN_MS - 1;
    assert!(automatic_rule(&restored, "a", before_end).is_none());
    assert!(automatic_rule(&restored, "b", before_end).is_some());
    assert!(automatic_rule(&restored, "a", before_end + 1).is_some());
}

#[test]
fn existing_journal_seeds_cooldown_once_from_recent_automatic_successes() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(FILE);
    let now = chrono::Utc::now().timestamp();
    let legacy = serde_json::json!({
        "public_rule": { "enabled": true },
        "records": [
            { "id": "1", "account_id": "a", "at": now, "reason": "quota", "status": "success" },
            { "id": "2", "account_id": "b", "at": now, "reason": "manual", "status": "success" },
            { "id": "3", "account_id": "c", "at": now, "reason": "expiry", "status": "failed" },
            { "id": "4", "account_id": "d", "at": now - 3600, "reason": "expiry", "status": "success" }
        ]
    });
    std::fs::write(&file, legacy.to_string()).unwrap();

    finish_migration(&file);
    let restored = read(&file).unwrap();
    assert_eq!(restored.records.len(), 4);
    assert_eq!(
        restored.last_automatic_success_ms.as_ref().unwrap(),
        &HashMap::from([("a".to_owned(), now * 1_000)])
    );
    assert!(automatic_rule(&restored, "a", now * 1_000).is_none());
    for id in ["b", "c", "d"] {
        assert!(automatic_rule(&restored, id, now * 1_000).is_some());
    }
}

#[test]
fn failed_or_interrupted_redemption_reuses_request_id_and_success_rearms_only_on_new_cards() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(FILE);
    let pending = update_at(&file, |d| {
        Ok(begin_redemption(
            d,
            "a",
            "owner-a",
            "expiry",
            Some("cards-3".into()),
            None,
        ))
    })
    .unwrap();
    assert_eq!(
        update_at(&file, |d| {
            finish_redemption(d, "a", &pending, Err("response lost".into()), None)
        })
        .unwrap(),
        Some("codex.resetHistory.unconfirmed")
    );
    assert!(read(&file)
        .unwrap()
        .last_automatic_success_ms
        .unwrap()
        .is_empty());
    let resumed = update_at(&file, |d| {
        Ok(begin_redemption(d, "a", "owner-a", "manual", None, None))
    })
    .unwrap();
    assert_eq!(resumed.request_id, pending.request_id);
    assert_eq!(resumed.reason, "expiry");
    assert!(resumed.unconfirmed_reported);
    assert_eq!(
        update_at(&file, |d| {
            finish_redemption(d, "a", &resumed, Err("response lost".into()), None)
        })
        .unwrap(),
        None
    );
    assert_eq!(
        update_at(&file, |d| {
            finish_redemption(d, "a", &resumed, Ok(ResetCreditConsumeOutcome::Reset), None)
        })
        .unwrap(),
        Some("codex.resetHistory.used")
    );
    let restored = read(&file).unwrap();
    assert!(!restored.pending.contains_key("a"));
    assert_eq!(restored.last_evidence["a"], "cards-3");
    assert_eq!(restored.records[0].status, "success");
    let next = update_at(&file, |d| {
        Ok(begin_redemption(
            d,
            "a",
            "owner-a",
            "quota",
            Some("cards-2".into()),
            None,
        ))
    })
    .unwrap();
    assert_ne!(next.request_id, pending.request_id);
    assert!(update_at(&file, |d| finish_redemption(
        d,
        "a",
        &pending,
        Ok(ResetCreditConsumeOutcome::Reset),
        None,
    ))
    .is_err());
    assert_eq!(
        read(&file).unwrap().pending["a"].request_id,
        next.request_id
    );
}

#[test]
fn pending_request_ids_are_scoped_to_the_actual_account_identity() {
    let mut d = Document::default();
    let old = begin_redemption(&mut d, "same-local-id", "workspace-a", "expiry", None, None);
    let retry = begin_redemption(&mut d, "same-local-id", "workspace-a", "manual", None, None);
    assert_eq!(old.request_id, retry.request_id);
    let other = begin_redemption(&mut d, "same-local-id", "workspace-b", "manual", None, None);
    assert_ne!(old.request_id, other.request_id);
    assert!(finish_redemption(
        &mut d,
        "same-local-id",
        &old,
        Ok(ResetCreditConsumeOutcome::Reset),
        None,
    )
    .is_err());
    assert_eq!(d.pending["same-local-id"].identity, "workspace-b");
}

#[test]
fn bulk_save_overwrites_all_existing_accounts_and_keeps_copied_rules_independent() {
    let mut d = Document::default();
    d.accounts.insert(
        "custom".into(),
        AccountRule::Custom {
            rule: Rule::default(),
        },
    );
    d.accounts.insert("off".into(), AccountRule::Off);
    let expected = d.accounts.clone();
    let ids = ["new", "custom", "off"].map(String::from);
    let rule = Rule {
        enabled: true,
        before_seconds: 600,
        remaining_percent: 0,
        ..Rule::default()
    };
    // Unrelated journal/revision changes do not conflict with a settings save.
    d.revision = 99;
    d.last_evidence
        .insert("off".into(), "existing-evidence".into());
    set_public_for_accounts(&mut d, rule.clone(), &Rule::default(), &ids, &expected).unwrap();
    for id in &ids {
        assert_eq!(d.accounts[id], AccountRule::Custom { rule: rule.clone() });
    }
    assert!(!d.accounts.contains_key("deleted"));
    assert_eq!(d.last_evidence["off"], "existing-evidence");
    set_public(&mut d, Rule::default(), &rule).unwrap();
    assert_eq!(effective(&d, "new"), Some(&rule));
    assert!(effective(&d, "future-account").is_none());
    // A disabled rule is also a valid bulk choice.
    let expected = d.accounts.clone();
    set_public_for_accounts(&mut d, Rule::default(), &Rule::default(), &ids, &expected).unwrap();
    for id in &ids {
        assert!(effective(&d, id).is_none());
    }
}

#[test]
fn bulk_conflict_aborts_default_and_account_changes_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rules.json");
    journal::update_rules(&path, |d| {
        d.accounts.insert("a".into(), AccountRule::Off);
        Ok(())
    })
    .unwrap();
    update_at(&path, |d| {
        d.last_evidence.insert("a".into(), "kept".into());
        Ok(())
    })
    .unwrap();
    let rules_path = dir.path().join("codex_reset_rules.json");
    let before = std::fs::read(&rules_path).unwrap();
    let rule = Rule {
        enabled: true,
        ..Rule::default()
    };
    assert!(journal::update_rules(&path, |d| set_public_for_accounts(
        d,
        rule.clone(),
        &Rule::default(),
        &["a".into(), "b".into()],
        &HashMap::new()
    ))
    .is_err());
    assert_eq!(std::fs::read(&rules_path).unwrap(), before);
    assert_eq!(read(&path).unwrap().last_evidence["a"], "kept");
    let mut d = read(&path).unwrap();
    let expected = d.accounts.clone();
    d.public_rule.before_seconds = 60;
    let before = serde_json::to_value(&d).unwrap();
    assert!(
        set_public_for_accounts(&mut d, rule, &Rule::default(), &["a".into()], &expected).is_err()
    );
    assert_eq!(serde_json::to_value(&d).unwrap(), before);
}

#[test]
fn confirmed_no_op_keeps_card_evidence_available_and_allows_a_new_attempt() {
    for (outcome, notice) in [
        (
            ResetCreditConsumeOutcome::NothingToReset,
            "codex.resetHistory.nothingToReset",
        ),
        (
            ResetCreditConsumeOutcome::NoCredit,
            "codex.resetHistory.noCredit",
        ),
    ] {
        let mut d = Document::default();
        let pending =
            begin_redemption(&mut d, "a", "identity", "expiry", Some("card".into()), None);
        assert_eq!(
            finish_redemption(&mut d, "a", &pending, Ok(outcome), None).unwrap(),
            Some(notice)
        );
        assert_eq!(d.records[0].status, "failed");
        assert!(d.records[0].detail.is_some());
        assert!(!d.last_evidence.contains_key("a"));
        assert!(d.automatic_successes_mut().is_empty());
        let next = begin_redemption(&mut d, "a", "identity", "expiry", None, None);
        assert_ne!(next.request_id, pending.request_id);
        assert_eq!(
            finish_redemption(
                &mut d,
                "a",
                &next,
                Ok(ResetCreditConsumeOutcome::AlreadyRedeemed),
                None,
            )
            .unwrap(),
            Some("codex.resetHistory.alreadyRedeemed")
        );
        assert_eq!(d.records[0].status, "success");
        assert!(!d.pending.contains_key("a"));
        assert!(d.automatic_successes_mut().contains_key("a"));
    }
}

#[test]
fn migration_keeps_live_rules_pending_changes_and_cooldown_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join(FILE);
    let now = chrono::Utc::now().timestamp_millis();
    let mut legacy = Document {
        revision: 41,
        ..Document::default()
    };
    legacy.public_rule.enabled = true;
    let pending = begin_redemption(
        &mut legacy,
        "a",
        "owner-a",
        "expiry",
        Some("cards-3".into()),
        None,
    );
    for index in 0..75 {
        legacy
            .accounts
            .insert(format!("account-{index:02}"), AccountRule::Off);
    }
    let original = serde_json::to_vec_pretty(&legacy).unwrap();
    std::fs::write(&source, &original).unwrap();
    assert!(!journal::migrate(&source).unwrap());

    journal::update_rules(&source, |d| {
        set_account(d, "b", AccountRule::Off, &AccountRule::Inherit)
    })
    .unwrap();
    update_at(&source, |d| {
        finish_redemption(d, "a", &pending, Ok(ResetCreditConsumeOutcome::Reset), None)
    })
    .unwrap();

    let midway = read(&source).unwrap();
    assert_eq!(midway.revision, 43);
    assert!(!midway.pending.contains_key("a"));
    assert!(midway.last_automatic_success_ms.as_ref().unwrap()["a"] >= now);
    finish_migration(&source);
    let restored = read(&source).unwrap();
    assert_eq!(restored.revision, 43);
    assert!(!restored.pending.contains_key("a"));
    assert_eq!(restored.last_evidence["a"], "cards-3");
    assert_eq!(
        restored.last_automatic_success_ms,
        midway.last_automatic_success_ms
    );
    assert_eq!(restored.accounts["b"], AccountRule::Off);
    assert_eq!(restored.accounts.len(), 76);
    assert_eq!(restored.records.len(), 2);
    assert_eq!(std::fs::read(&source).unwrap(), original);
    std::fs::write(&source, "{damaged after cutover").unwrap();
    let restored = read(&source).unwrap();
    assert_eq!(restored.revision, 43);
    assert_eq!(restored.records[0].status, "success");
    let pending = journal::update_account(&source, "b", |document| {
        Ok(begin_redemption(
            document, "b", "owner-b", "quota", None, None,
        ))
    })
    .unwrap();
    let mut restarted = journal::read(&source, Some("b"), true).unwrap();
    assert_eq!(
        begin_redemption(&mut restarted, "b", "owner-b", "quota", None, None).request_id,
        pending.request_id
    );
}
