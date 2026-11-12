//! Live local redemption history, independent of the bounded automation activity log.
use super::codex_quota::ResetCreditReceipt;
use super::{AttemptContext, Pending, ResetObservation};
use rusqlite::{params, types::Type, Connection, OptionalExtension, Transaction};
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

pub(super) fn store_after_observation(
    conn: &Connection,
    account: &str,
    pending: &Pending,
    observation: &ResetObservation,
) -> rusqlite::Result<()> {
    let raw = encode_json(observation)?;
    conn.execute(
        "UPDATE reset_usage_context SET after_observation=?4
         WHERE request_id=(SELECT request_id FROM reset_usage_history
           WHERE request_id=?1 AND account_id=?2 AND identity=?3
           AND outcome IN ('reset','already_redeemed')) AND after_observation IS NULL",
        params![pending.request_id, account, pending.identity, raw],
    )?;
    Ok(())
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

pub(super) fn recent_no_op(
    conn: &Connection,
    account: &str,
    identity: &str,
    reason: &str,
    context: &AttemptContext,
    cutoff: i64,
) -> rusqlite::Result<bool> {
    let last = conn
        .query_row(
            "SELECT history.reason,history.outcome,history.updated_at,audit.context
         FROM reset_usage_history AS history
         LEFT JOIN reset_usage_context AS audit USING (request_id)
         WHERE history.account_id=?1 AND history.identity=?2
         ORDER BY history.sequence DESC LIMIT 1",
            params![account, identity],
            |row| {
                let previous_reason: String = row.get(0)?;
                let outcome: String = row.get(1)?;
                let updated_at: i64 = row.get(2)?;
                Ok(previous_reason == reason
                    && matches!(outcome.as_str(), "nothing_to_reset" | "no_credit")
                    && updated_at > cutoff
                    && decode_json::<AttemptContext>(row, 3)?.as_ref() == Some(context))
            },
        )
        .optional()?;
    Ok(last.unwrap_or(false))
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
    })
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn recent_known_no_op_backs_off_only_same_account_and_trigger_conditions() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE);
        let now = chrono::Utc::now().timestamp();
        let mut before = ResetObservation::from_quota(None, now);
        before.available_count = Some(2);
        before.next_expires_at = Some(now + 90);
        before.available_card_ids = vec!["credit-a".into(), "credit-b".into()];
        let context = AttemptContext {
            rule: Rule::default(),
            before,
        };
        assert_eq!(
            context.retry_cutoff("expiry", now),
            Some(now - NO_OP_RETRY_SECONDS)
        );
        assert_eq!(context.retry_cutoff("expiry", now + 30), None);

        let pending = journal::update_account(&file, "a", |d| {
            Ok(begin_redemption(
                d,
                "a",
                "owner-a",
                "expiry",
                None,
                Some(context.clone()),
            ))
        })
        .unwrap();
        journal::update_account(&file, "a", |d| {
            finish_redemption(
                d,
                "a",
                &pending,
                Ok(ResetCreditConsumeOutcome::NothingToReset),
                None,
            )
        })
        .unwrap();
        let record = journal::usage_history(&file, "a", "owner-a", None)
            .unwrap()
            .records
            .remove(0);
        assert_eq!(record.context.as_ref(), Some(&context));
        let backed_off = |account, identity, reason, context: &AttemptContext, cutoff| {
            journal::recent_no_op(&file, account, identity, reason, context, cutoff).unwrap()
        };
        assert!(backed_off("a", "owner-a", "expiry", &context, now - 120));
        assert!(!backed_off("b", "owner-a", "expiry", &context, now - 120));
        assert!(!backed_off("a", "owner-b", "expiry", &context, now - 120));
        assert!(!backed_off("a", "owner-a", "quota", &context, now - 120));
        assert!(!backed_off(
            "a",
            "owner-a",
            "expiry",
            &context,
            record.updated_at
        ));
        let mut changed = context.clone();
        changed.rule.before_seconds += 60;
        assert!(!backed_off("a", "owner-a", "expiry", &changed, now - 120));
        changed = context.clone();
        changed.before.available_count = Some(1);
        assert!(!backed_off("a", "owner-a", "expiry", &changed, now - 120));

        let next = journal::update_account(&file, "a", |d| {
            let next = begin_redemption(d, "a", "owner-a", "expiry", None, Some(changed));
            let receipt = ResetCreditReceipt {
                credit_id: Some("credit-b".into()),
                windows_reset: Some(2),
            };
            finish_redemption(
                d,
                "a",
                &next,
                Ok(ResetCreditConsumeOutcome::Reset),
                Some(receipt),
            )?;
            Ok(next)
        })
        .unwrap();
        let mut after = ResetObservation::from_quota(None, now);
        after.available_count = Some(0);
        journal::store_after_observation(&file, "a", &next, &after).unwrap();
        let page = journal::usage_history(&file, "a", "owner-a", None).unwrap();
        assert_eq!(page.records.len(), 2);
        let receipt = page.records[0].receipt.as_ref().unwrap();
        assert_eq!(receipt.credit_id.as_deref(), Some("credit-b"));
        assert_eq!(receipt.windows_reset, Some(2));
        assert_eq!(page.records[0].after_observation, Some(after));
        assert!(page.records[1].after_observation.is_none());
    }

    #[test]
    fn context_schema_upgrade_keeps_existing_usage_without_inventing_observations() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE);
        let pending = journal::update_account(&file, "a", |d| {
            let pending = begin_redemption(d, "a", "owner-a", "manual", None, None);
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
    fn local_usage_retry_updates_one_durable_record() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE);
        let pending = journal::update_account(&file, "a", |d| {
            Ok(begin_redemption(d, "a", "owner-a", "expiry", None, None))
        })
        .unwrap();
        journal::update_account(&file, "a", |d| {
            finish_redemption(d, "a", &pending, Err("response lost".into()), None)
        })
        .unwrap();
        let page = journal::usage_history(&file, "a", "owner-a", None).unwrap();
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].outcome, "unconfirmed");
        assert_eq!(page.records[0].detail.as_deref(), Some("response lost"));
        let first_at = page.records[0].at;
        journal::update_account(&file, "a", |d| {
            let retry = begin_redemption(d, "a", "owner-a", "manual", None, None);
            assert_eq!(retry.request_id, pending.request_id);
            finish_redemption(
                d,
                "a",
                &retry,
                Ok(ResetCreditConsumeOutcome::AlreadyRedeemed),
                None,
            )
        })
        .unwrap();
        let page = journal::usage_history(&file, "a", "owner-a", None).unwrap();
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].at, first_at);
        assert_eq!(page.records[0].reason, "expiry");
        assert_eq!(page.records[0].outcome, "already_redeemed");
        assert!(page.records[0].detail.is_none());
    }

    #[test]
    fn local_usage_records_manual_and_automatic_noops_without_rule_gates() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE);
        for (reason, outcome) in [
            ("manual", ResetCreditConsumeOutcome::Reset),
            ("expiry", ResetCreditConsumeOutcome::NothingToReset),
            ("quota", ResetCreditConsumeOutcome::NoCredit),
        ] {
            journal::update_account(&file, "a", |d| {
                d.accounts.insert("a".into(), AccountRule::Off);
                let pending = begin_redemption(d, "a", "owner-a", reason, None, None);
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
                ("quota", "no_credit"),
                ("expiry", "nothing_to_reset"),
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
        assert!(journal::usage_history(&file, "a", "owner-a", None)
            .unwrap()
            .records
            .is_empty());
        for _ in 0..115 {
            journal::update_account(&file, "a", |d| {
                let pending = begin_redemption(d, "a", "owner-a", "manual", None, None);
                finish_redemption(d, "a", &pending, Ok(ResetCreditConsumeOutcome::Reset), None)
            })
            .unwrap();
        }
        let first = journal::usage_history(&file, "a", "owner-a", None).unwrap();
        assert_eq!(first.records.len(), 30);
        let newest = journal::update_account(&file, "a", |d| {
            Ok(begin_redemption(d, "a", "owner-a", "manual", None, None))
        })
        .unwrap();
        let mut ids: Vec<_> = first.records.into_iter().map(|r| r.request_id).collect();
        let mut cursor = first.next_cursor;
        while let Some(next) = cursor {
            let page = journal::usage_history(&file, "a", "owner-a", Some(next)).unwrap();
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
            begin_redemption(d, "a", "owner-a", "manual", None, None);
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
