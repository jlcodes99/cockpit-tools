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

#[test]
fn manual_lease_excludes_concurrent_same_account_redemption_but_not_other_accounts() {
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
                reason: "manual".into(),
                evidence: None,
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
    assert!(!dir.path().join("codex_reset_rules.json").exists());
    assert_eq!(read(&file).unwrap().pending["a"].request_id, "stable-id");
}

#[test]
fn activity_history_keeps_latest_hundred_records() {
    let mut d = Document::default();
    for _ in 0..150 {
        record(&mut d, "a", "manual", "success", None);
    }
    assert_eq!(d.records.len(), 100);
}

#[test]
fn legacy_enabled_rules_configs_and_rules_file_states_do_not_block_manual_redemption_or_history() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(FILE);
    let rules_file = dir.path().join("codex_reset_rules.json");

    let mut accounts = serde_json::Map::new();
    for index in 0..75 {
        accounts.insert(
            format!("account-{index:02}"),
            serde_json::json!({
                "mode": "custom",
                "rule": {
                    "enabled": true,
                    "expiry_enabled": true,
                    "before_seconds": 3600,
                    "quota_enabled": true,
                    "remaining_percent": 20,
                    "window": "weekly"
                }
            }),
        );
    }
    let legacy_json = serde_json::json!({
        "revision": 41,
        "public_rule": {
            "enabled": true,
            "expiry_enabled": true,
            "before_seconds": 3600,
            "quota_enabled": true,
            "remaining_percent": 15,
            "window": "both"
        },
        "accounts": accounts,
        "last_evidence": { "a": "cards-3" },
        "last_automatic_success_ms": { "a": 1_700_000_000_000i64 },
        "records": [
            { "id": "rec-a-1", "account_id": "a", "at": 200, "reason": "quota", "status": "success", "detail": null },
            { "id": "rec-b-1", "account_id": "b", "at": 150, "reason": "check", "status": "failed", "detail": "network" }
        ]
    });
    let raw_legacy = serde_json::to_vec_pretty(&legacy_json).unwrap();
    std::fs::write(&file, &raw_legacy).unwrap();
    // Write a damaged/enabled deprecated rules file to prove neither manual operations nor
    // migration read or validate codex_reset_rules.json.
    std::fs::write(&rules_file, "{damaged rules enabled: true").unwrap();

    // Skipping obsolete rule sections must still advance in bounded steps per batch.
    let first_batch = legacy::read(&file, None, &legacy::Cursor::default()).unwrap();
    assert!(!first_batch.complete);
    assert!(first_batch.items.is_empty());

    // Pre-migration read returns account-owned legacy activities without backfilling usage_history.
    let pre_mig = journal::usage_history(&file, "a", "any-identity", None).unwrap();
    assert!(pre_mig.records.is_empty());
    assert_eq!(pre_mig.legacy_records.len(), 1);
    assert_eq!(pre_mig.legacy_records[0].id, "rec-a-1");

    // Multi-batch migration across 75 legacy accounts + interrupted mid-AccountRules resume.
    assert!(!journal::migrate(&file).unwrap());
    let conn = rusqlite::Connection::open(dir.path().join("codex_reset_journal.sqlite")).unwrap();
    let mid_cursor: String = conn
        .query_row("SELECT cursor FROM reset_progress WHERE id=1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(mid_cursor.contains("AccountRules"));
    drop(conn);

    finish_migration(&file);
    assert_eq!(std::fs::read(&file).unwrap(), raw_legacy);

    // Deleting codex_reset_rules.json also must not break reads, manual redemption, or history.
    std::fs::remove_file(&rules_file).unwrap();
    let migrated = read(&file).unwrap();
    assert_eq!(migrated.revision, 41);

    let manual = update_at(&file, |d| {
        let p = begin_redemption(d, "a", "owner-a");
        assert_eq!(p.reason, "manual");
        finish_redemption(d, "a", &p, Ok(ResetCreditConsumeOutcome::Reset), None)?;
        Ok(p)
    })
    .unwrap();
    assert!(!rules_file.exists());
    let history = journal::usage_history(&file, "a", "owner-a", None).unwrap();
    assert_eq!(history.records.len(), 1);
    assert_eq!(history.records[0].request_id, manual.request_id);
    assert_eq!(history.records[0].reason, "manual");
    assert_eq!(history.records[0].outcome, "reset");
    // Legacy read-only activity log remains distinct, intact, and scoped by account_id.
    assert_eq!(history.legacy_records.len(), 1);
    assert_eq!(history.legacy_records[0].id, "rec-a-1");
    assert_eq!(history.legacy_records[0].reason, "quota");
    let other_legacy = journal::usage_history(&file, "a", "different-owner", None).unwrap();
    assert!(other_legacy.records.is_empty());
    assert_eq!(other_legacy.legacy_records.len(), 1);
    assert_eq!(other_legacy.legacy_records[0].id, "rec-a-1");
}

#[test]
fn migration_settled_pending_is_not_resurrected_by_remaining_json_import() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(FILE);
    let mut accounts = serde_json::Map::new();
    for index in 0..45 {
        accounts.insert(
            format!("account-{index:02}"),
            serde_json::json!({ "mode": "off" }),
        );
    }
    let legacy_json = serde_json::json!({
        "accounts": accounts,
        "pending": {
            "a": {
                "identity": "owner-a",
                "request_id": "mid-mig-req",
                "reason": "expiry",
                "evidence": "cards-2",
                "unconfirmed_reported": false,
                "context": {
                    "rule": { "enabled": true, "expiry_enabled": true, "before_seconds": 3600, "quota_enabled": false, "remaining_percent": 5 },
                    "before": { "available_count": 2, "next_expires_at": 2000, "available_card_ids": [], "hourly": null, "weekly": null }
                }
            }
        }
    });
    std::fs::write(&file, serde_json::to_vec(&legacy_json).unwrap()).unwrap();

    // First batch stops inside AccountRules before reaching "pending".
    assert!(!journal::migrate(&file).unwrap());
    let conn = rusqlite::Connection::open(dir.path().join("codex_reset_journal.sqlite")).unwrap();
    let mid_cursor: String = conn
        .query_row("SELECT cursor FROM reset_progress WHERE id=1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(mid_cursor.contains("AccountRules"));
    drop(conn);

    // Manually settle "a" while migration is still incomplete (modified = 1).
    let settled = update_at(&file, |d| {
        let p = begin_redemption(d, "a", "owner-a");
        assert_eq!(p.request_id, "mid-mig-req");
        finish_redemption(
            d,
            "a",
            &p,
            Ok(ResetCreditConsumeOutcome::AlreadyRedeemed),
            None,
        )?;
        Ok(p)
    })
    .unwrap();
    assert_eq!(settled.reason, "expiry");

    // Finishing migration walks past "pending" in the untouched JSON, which must NOT revive "a".
    finish_migration(&file);
    assert!(!read(&file).unwrap().pending.contains_key("a"));
    let history = journal::usage_history(&file, "a", "owner-a", None).unwrap();
    assert_eq!(history.records.len(), 1);
    assert_eq!(history.records[0].request_id, "mid-mig-req");
    assert_eq!(history.records[0].reason, "expiry");
    assert_eq!(history.records[0].outcome, "already_redeemed");
}

#[test]
fn legacy_json_automatic_pending_reuses_request_id_and_preserves_original_source_and_context() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(FILE);
    let legacy_json = serde_json::json!({
        "pending": {
            "a": {
                "identity": "owner-a",
                "request_id": "auto-timeout-id",
                "reason": "expiry",
                "evidence": "cards-3",
                "unconfirmed_reported": true,
                "context": {
                    "rule": {
                        "enabled": true,
                        "expiry_enabled": true,
                        "before_seconds": 3600,
                        "quota_enabled": false,
                        "remaining_percent": 5
                    },
                    "before": {
                        "available_count": 3,
                        "next_expires_at": 2000,
                        "available_card_ids": ["card-1"],
                        "hourly": null,
                        "weekly": null
                    }
                }
            }
        },
        "records": [
            { "id": "rec-old-1", "account_id": "a", "at": 100, "reason": "expiry", "status": "failed", "detail": "timeout" }
        ]
    });
    std::fs::write(&file, serde_json::to_vec(&legacy_json).unwrap()).unwrap();

    // User's manual confirmation reuses the saved request_id and retains the original
    // automatic trigger reason & rule/before context (never rewriting history to "manual").
    let adopted = update_at(&file, |d| Ok(begin_redemption(d, "a", "owner-a"))).unwrap();
    assert_eq!(adopted.request_id, "auto-timeout-id");
    assert_eq!(adopted.reason, "expiry");
    assert!(adopted.context.is_some());

    // Another transient failure keeps pending and original context intact.
    update_at(&file, |d| {
        finish_redemption(d, "a", &adopted, Err("network timeout".into()), None)
    })
    .unwrap();
    let mid_history = journal::usage_history(&file, "a", "owner-a", None).unwrap();
    assert_eq!(mid_history.records.len(), 1);
    assert_eq!(mid_history.records[0].request_id, "auto-timeout-id");
    assert_eq!(mid_history.records[0].reason, "expiry");
    assert_eq!(mid_history.records[0].outcome, "unconfirmed");
    assert!(mid_history.records[0].context.is_some());

    // Server returning AlreadyRedeemed settles that same durable row as expiry + already_redeemed
    // with its original context, without polluting read-only legacy_records.
    let second_retry = update_at(&file, |d| Ok(begin_redemption(d, "a", "owner-a"))).unwrap();
    assert_eq!(second_retry.request_id, "auto-timeout-id");
    update_at(&file, |d| {
        finish_redemption(
            d,
            "a",
            &second_retry,
            Ok(ResetCreditConsumeOutcome::AlreadyRedeemed),
            None,
        )
    })
    .unwrap();
    assert!(!read(&file).unwrap().pending.contains_key("a"));

    let settled_history = journal::usage_history(&file, "a", "owner-a", None).unwrap();
    assert_eq!(settled_history.records.len(), 1);
    assert_eq!(settled_history.records[0].request_id, "auto-timeout-id");
    assert_eq!(settled_history.records[0].reason, "expiry");
    assert_eq!(settled_history.records[0].outcome, "already_redeemed");
    assert_eq!(settled_history.records[0].context, adopted.context);
    assert_eq!(settled_history.legacy_records.len(), 1);
    assert_eq!(settled_history.legacy_records[0].id, "rec-old-1");

    // Next fresh manual click allocates a new manual request_id with no automatic context.
    let next = update_at(&file, |d| Ok(begin_redemption(d, "a", "owner-a"))).unwrap();
    assert_ne!(next.request_id, "auto-timeout-id");
    assert_eq!(next.reason, "manual");
    assert!(next.context.is_none());
    assert!(update_at(&file, |d| finish_redemption(
        d,
        "a",
        &adopted,
        Ok(ResetCreditConsumeOutcome::Reset),
        None,
    ))
    .is_err());
    assert_eq!(read(&file).unwrap().pending["a"].request_id, next.request_id);
}

#[test]
fn pending_request_ids_are_scoped_to_the_actual_account_identity() {
    let mut d = Document {
        pending: HashMap::from([(
            "same-local-id".into(),
            Pending {
                identity: "workspace-a".into(),
                request_id: "legacy-a-req".into(),
                reason: "expiry".into(),
                evidence: Some("cards-1".into()),
                unconfirmed_reported: true,
                context: None,
            },
        )]),
        ..Document::default()
    };
    let retry = begin_redemption(&mut d, "same-local-id", "workspace-a");
    assert_eq!(retry.request_id, "legacy-a-req");
    assert_eq!(retry.reason, "expiry");

    let other = begin_redemption(&mut d, "same-local-id", "workspace-b");
    assert_ne!(other.request_id, "legacy-a-req");
    assert_eq!(other.reason, "manual");
    assert!(finish_redemption(
        &mut d,
        "same-local-id",
        &retry,
        Ok(ResetCreditConsumeOutcome::Reset),
        None,
    )
    .is_err());
    assert_eq!(d.pending["same-local-id"].identity, "workspace-b");
}

#[cfg(unix)]
#[test]
fn schedule_migration_propagates_try_exists_error_instead_of_treating_as_missing() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let missing_file = dir.path().join("missing.json");
    assert_eq!(schedule_migration_at(&missing_file), Ok(false));

    let blocked = dir.path().join("blocked");
    std::fs::create_dir(&blocked).unwrap();
    let probe = blocked.join(FILE);
    std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let cant_probe = std::fs::metadata(&probe).is_err();
    let err = schedule_migration_at(&probe);
    std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o755)).unwrap();
    if cant_probe {
        assert!(err.is_err(), "expected try_exists error, got {err:?}");
    }
}

#[test]
fn historical_rules_keep_legacy_window_compatibility_when_deserializing() {
    for window in ["hourly", "any", "weekly"] {
        let raw = serde_json::json!({
            "enabled": true,
            "expiry_enabled": false,
            "before_seconds": 7200,
            "quota_enabled": true,
            "remaining_percent": 15,
            "window": window
        });
        let rule: Rule = serde_json::from_value(raw).unwrap();
        assert!(rule.enabled);
        assert!(!rule.expiry_enabled);
        assert_eq!(rule.before_seconds, 7200);
        assert!(rule.quota_enabled);
        assert_eq!(rule.remaining_percent, 15);
        assert!(!rule.prefer_usage_credits);
    }
}
