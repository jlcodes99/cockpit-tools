//! Live local redemption history, independent of the bounded automation activity log.
use super::codex_quota::ResetCreditReceipt;
use super::{AttemptContext, ResetObservation};
use rusqlite::{params, types::Type, Connection, Transaction};
use serde::Serialize;

#[derive(Clone)]
pub(super) struct UsageUpdate {
    account_id: String,
    identity: String,
    request_id: String,
    at: i64,
    reason: String,
    outcome: String,
    detail: Option<String>,
    context: Option<AttemptContext>,
    pub(super) receipt: Option<ResetCreditReceipt>,
}

impl UsageUpdate {
    pub(super) fn new(
        account_id: &str,
        pending: &super::Pending,
        outcome: &str,
        detail: Option<String>,
    ) -> Self {
        Self {
            account_id: account_id.into(),
            identity: pending.identity.clone(),
            request_id: pending.request_id.clone(),
            at: chrono::Utc::now().timestamp(),
            reason: pending.reason.clone(),
            outcome: outcome.into(),
            detail,
            context: pending.context.clone(),
            receipt: None,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct UsageRecord {
    pub sequence: i64,
    pub request_id: String,
    pub at: i64,
    pub updated_at: i64,
    pub reason: String,
    pub outcome: String,
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<AttemptContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_observation: Option<ResetObservation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt: Option<ResetCreditReceipt>,
}

#[derive(Debug, Serialize)]
pub struct UsagePage {
    pub records: Vec<UsageRecord>,
    pub next_cursor: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub legacy_records: Vec<super::Record>,
}

pub(super) fn initialize(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "BEGIN IMMEDIATE;
         CREATE TABLE IF NOT EXISTS reset_usage_history (
           sequence INTEGER PRIMARY KEY AUTOINCREMENT, request_id TEXT NOT NULL UNIQUE,
           account_id TEXT NOT NULL, identity TEXT NOT NULL, at INTEGER NOT NULL,
           updated_at INTEGER NOT NULL, reason TEXT NOT NULL, outcome TEXT NOT NULL, detail TEXT
         );
         CREATE INDEX IF NOT EXISTS reset_usage_account_identity
           ON reset_usage_history(account_id,identity,sequence DESC);
         PRAGMA user_version=2;
         COMMIT;",
    )
}

pub(super) fn initialize_context(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "BEGIN IMMEDIATE;
         CREATE TABLE IF NOT EXISTS reset_usage_context (
           request_id TEXT PRIMARY KEY, context TEXT, after_observation TEXT, receipt TEXT
         );
         PRAGMA user_version=3;
         COMMIT;",
    )
}

fn decode_json<T: serde::de::DeserializeOwned>(
    row: &rusqlite::Row<'_>,
    column: usize,
) -> rusqlite::Result<Option<T>> {
    row.get::<_, Option<String>>(column)?
        .map(|raw| {
            serde_json::from_str(&raw).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(column, Type::Text, Box::new(error))
            })
        })
        .transpose()
}

fn encode_json<T: Serialize>(value: &T) -> rusqlite::Result<String> {
    serde_json::to_string(value)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
}

pub(super) fn store(tx: &Transaction<'_>, update: &UsageUpdate) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO reset_usage_history(request_id,account_id,identity,at,updated_at,reason,outcome,detail)
         VALUES(?1,?2,?3,?4,?4,?5,?6,?7)
         ON CONFLICT(request_id) DO UPDATE SET updated_at=excluded.updated_at,
           outcome=excluded.outcome,detail=excluded.detail
         WHERE reset_usage_history.account_id=excluded.account_id
           AND reset_usage_history.identity=excluded.identity",
        params![update.request_id, update.account_id, update.identity, update.at,
            update.reason, update.outcome, update.detail],
    )?;
    if update.context.is_some() || update.receipt.is_some() {
        let context = update.context.as_ref().map(encode_json).transpose()?;
        let receipt = update.receipt.as_ref().map(encode_json).transpose()?;
        tx.execute(
            "INSERT INTO reset_usage_context(request_id,context,receipt)
             SELECT request_id,?2,?3 FROM reset_usage_history
             WHERE request_id=?1 AND account_id=?4 AND identity=?5
             ON CONFLICT(request_id) DO UPDATE SET receipt=COALESCE(excluded.receipt,reset_usage_context.receipt)",
            params![update.request_id, context, receipt, update.account_id, update.identity],
        )?;
    }
    Ok(())
}

pub(super) fn read(
    conn: &Connection,
    account: &str,
    identity: &str,
    cursor: Option<i64>,
) -> rusqlite::Result<UsagePage> {
    const PAGE_SIZE: usize = 30;
    let mut query = conn.prepare(
        "SELECT sequence,request_id,at,updated_at,reason,outcome,detail,audit.context,audit.after_observation,audit.receipt
         FROM reset_usage_history LEFT JOIN reset_usage_context AS audit USING (request_id)
         WHERE account_id=?1 AND identity=?2 AND sequence < ?3 ORDER BY sequence DESC LIMIT ?4",
    )?;
    let mut records = query
        .query_map(
            params![account, identity, cursor.unwrap_or(i64::MAX), PAGE_SIZE + 1],
            |row| {
                Ok(UsageRecord {
                    sequence: row.get(0)?,
                    request_id: row.get(1)?,
                    at: row.get(2)?,
                    updated_at: row.get(3)?,
                    reason: row.get(4)?,
                    outcome: row.get(5)?,
                    detail: row.get(6)?,
                    context: decode_json(row, 7)?,
                    after_observation: decode_json(row, 8)?,
                    receipt: decode_json(row, 9)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let has_more = records.len() > PAGE_SIZE;
    records.truncate(PAGE_SIZE);
    let next_cursor = has_more.then(|| records.last().unwrap().sequence);
    Ok(UsagePage {
        records,
        next_cursor,
        legacy_records: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use rusqlite::params;

    #[test]
    fn historical_automatic_context_and_observations_remain_readable() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE);
        // Initialize SQLite without relying on codex_reset_rules.json.
        assert!(journal::usage_history(&file, "a", "owner-a", None)
            .unwrap()
            .records
            .is_empty());
        let conn =
            rusqlite::Connection::open(dir.path().join("codex_reset_journal.sqlite")).unwrap();
        let legacy_context = serde_json::json!({
            "rule": {
                "enabled": true,
                "expiry_enabled": false,
                "before_seconds": 7200,
                "quota_enabled": true,
                "remaining_percent": 15,
                "window": "weekly"
            },
            "before": {
                "available_count": 2,
                "next_expires_at": 1_700_000_090,
                "available_card_ids": ["credit-a", "credit-b"],
                "hourly": null,
                "weekly": { "remaining": 10, "resets_at": 1_700_050_000 }
            }
        });
        let after = ResetObservation::from_quota(None, chrono::Utc::now().timestamp());
        let receipt = ResetCreditReceipt {
            credit_id: Some("credit-b".into()),
            windows_reset: Some(2),
        };
        conn.execute(
            "INSERT INTO reset_usage_history(request_id,account_id,identity,at,updated_at,reason,outcome,detail)
             VALUES('auto-1','a','owner-a',100,101,'quota','reset',NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO reset_usage_context(request_id,context,after_observation,receipt)
             VALUES('auto-1',?1,?2,?3)",
            params![
                legacy_context.to_string(),
                serde_json::to_string(&after).unwrap(),
                serde_json::to_string(&receipt).unwrap()
            ],
        )
        .unwrap();
        drop(conn);

        let page = journal::usage_history(&file, "a", "owner-a", None).unwrap();
        assert_eq!(page.records.len(), 1);
        let record = &page.records[0];
        assert_eq!(record.reason, "quota");
        assert_eq!(record.outcome, "reset");
        let context = record.context.as_ref().unwrap();
        assert!(context.rule.enabled);
        assert_eq!(context.rule.before_seconds, 7200);
        assert_eq!(context.rule.remaining_percent, 15);
        assert!(!context.rule.prefer_usage_credits);
        assert_eq!(context.before.available_count, Some(2));
        assert_eq!(record.after_observation, Some(after));
        assert_eq!(record.receipt.as_ref(), Some(&receipt));
    }

    #[test]
    fn context_schema_upgrade_keeps_existing_usage_without_inventing_observations() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE);
        let pending = journal::update_account(&file, "a", |d| {
            let pending = begin_redemption(d, "a", "owner-a");
            finish_redemption(d, "a", &pending, Ok(ResetCreditConsumeOutcome::Reset), None)?;
            Ok(pending)
        })
        .unwrap();
        let conn =
            rusqlite::Connection::open(dir.path().join("codex_reset_journal.sqlite")).unwrap();
        conn.execute_batch("DROP TABLE reset_usage_context; PRAGMA user_version=2;")
            .unwrap();
        drop(conn);

        let page = journal::usage_history(&file, "a", "owner-a", None).unwrap();
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].request_id, pending.request_id);
        assert_eq!(page.records[0].outcome, "reset");
        assert!(page.records[0].context.is_none());
        assert!(page.records[0].after_observation.is_none());
    }

    #[test]
    fn local_usage_retry_updates_one_durable_record_without_automatic_replay() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE);
        journal::usage_history(&file, "a", "owner-a", None).unwrap();
        let original_context = AttemptContext {
            rule: Rule {
                enabled: true,
                before_seconds: 1800,
                ..Rule::default()
            },
            before: ResetObservation::from_quota(None, 100),
        };
        let legacy_pending = Pending {
            identity: "owner-a".into(),
            request_id: "auto-unconfirmed-1".into(),
            reason: "manual".into(),
            evidence: None,
            unconfirmed_reported: false,
            context: None,
        };
        let conn =
            rusqlite::Connection::open(dir.path().join("codex_reset_journal.sqlite")).unwrap();
        conn.execute(
            "INSERT INTO reset_accounts(account_id,pending,modified) VALUES('a',?1,1)",
            [serde_json::to_string(&legacy_pending).unwrap()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO reset_usage_history(request_id,account_id,identity,at,updated_at,reason,outcome,detail)
             VALUES('auto-unconfirmed-1','a','owner-a',100,100,'expiry','unconfirmed','response lost')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO reset_usage_context(request_id,context) VALUES('auto-unconfirmed-1',?1)",
            [serde_json::to_string(&original_context).unwrap()],
        )
        .unwrap();
        drop(conn);

        journal::update_account(&file, "a", |d| {
            let retry = begin_redemption(d, "a", "owner-a");
            assert_eq!(retry.request_id, "auto-unconfirmed-1");
            finish_redemption(
                d,
                "a",
                &retry,
                Ok(ResetCreditConsumeOutcome::AlreadyRedeemed),
                Some(ResetCreditReceipt {
                    credit_id: Some("card-9".into()),
                    windows_reset: Some(1),
                }),
            )
        })
        .unwrap();
        let page = journal::usage_history(&file, "a", "owner-a", None).unwrap();
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].at, 100);
        // Existing SQLite row retains its original source and context even across retry updates.
        assert_eq!(page.records[0].reason, "expiry");
        assert_eq!(page.records[0].outcome, "already_redeemed");
        assert_eq!(page.records[0].context.as_ref(), Some(&original_context));
        assert_eq!(
            page.records[0].receipt.as_ref().and_then(|r| r.credit_id.as_deref()),
            Some("card-9")
        );
        assert!(page.records[0].detail.is_none());
    }

    #[test]
    fn local_usage_records_manual_outcomes_and_stays_scoped_to_account_identity() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE);
        std::fs::write(
            dir.path().join("codex_reset_rules.json"),
            r#"{"public_rule":{"enabled":true}}"#,
        )
        .unwrap();
        for outcome in [
            ResetCreditConsumeOutcome::Reset,
            ResetCreditConsumeOutcome::NothingToReset,
            ResetCreditConsumeOutcome::NoCredit,
        ] {
            journal::update_account(&file, "a", |d| {
                let pending = begin_redemption(d, "a", "owner-a");
                finish_redemption(d, "a", &pending, Ok(outcome), None)
            })
            .unwrap();
        }
        let page = journal::usage_history(&file, "a", "owner-a", None).unwrap();
        assert_eq!(
            page.records
                .iter()
                .map(|r| (r.reason.as_str(), r.outcome.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("manual", "no_credit"),
                ("manual", "nothing_to_reset"),
                ("manual", "reset")
            ]
        );
        assert!(journal::usage_history(&file, "a", "other-owner", None)
            .unwrap()
            .records
            .is_empty());
        assert!(journal::usage_history(&file, "b", "owner-a", None)
            .unwrap()
            .records
            .is_empty());
    }

    #[test]
    fn local_usage_keeps_all_new_records_with_stable_paging_and_no_legacy_backfill() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE);
        let mut old = Document::default();
        record(&mut old, "a", "manual", "success", None);
        std::fs::write(&file, serde_json::to_vec(&old).unwrap()).unwrap();
        while !journal::migrate(&file).unwrap() {}
        let before_manual = journal::usage_history(&file, "a", "owner-a", None).unwrap();
        assert!(before_manual.records.is_empty());
        assert_eq!(before_manual.legacy_records.len(), 1);
        for _ in 0..115 {
            journal::update_account(&file, "a", |d| {
                let pending = begin_redemption(d, "a", "owner-a");
                finish_redemption(d, "a", &pending, Ok(ResetCreditConsumeOutcome::Reset), None)
            })
            .unwrap();
        }
        let first = journal::usage_history(&file, "a", "owner-a", None).unwrap();
        assert_eq!(first.records.len(), 30);
        assert_eq!(first.legacy_records.len(), 1);
        let newest = journal::update_account(&file, "a", |d| {
            Ok(begin_redemption(d, "a", "owner-a"))
        })
        .unwrap();
        let mut ids: Vec<_> = first.records.into_iter().map(|r| r.request_id).collect();
        let mut cursor = first.next_cursor;
        while let Some(next) = cursor {
            let page = journal::usage_history(&file, "a", "owner-a", Some(next)).unwrap();
            assert!(page.legacy_records.is_empty());
            ids.extend(page.records.into_iter().map(|r| r.request_id));
            cursor = page.next_cursor;
        }
        assert_eq!(ids.len(), 115);
        assert_eq!(ids.iter().collect::<HashSet<_>>().len(), 115);
        assert!(!ids.contains(&newest.request_id));
    }

    #[test]
    fn local_usage_and_pending_state_roll_back_together() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE);
        journal::usage_history(&file, "a", "owner-a", None).unwrap();
        let conn =
            rusqlite::Connection::open(dir.path().join("codex_reset_journal.sqlite")).unwrap();
        conn.execute_batch(
            "CREATE TRIGGER reject_usage BEFORE INSERT ON reset_usage_history
            BEGIN SELECT RAISE(ABORT, 'disk write failed'); END;",
        )
        .unwrap();
        let result: Result<(), String> = journal::update_account(&file, "a", |d| {
            begin_redemption(d, "a", "owner-a");
            Ok(())
        });
        assert!(result.is_err());
        assert!(journal::usage_history(&file, "a", "owner-a", None)
            .unwrap()
            .records
            .is_empty());
        assert!(journal::read(&file, Some("a"), false)
            .unwrap()
            .pending
            .is_empty());
    }
}
