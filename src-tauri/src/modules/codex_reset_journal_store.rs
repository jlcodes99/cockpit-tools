use std::{
    collections::{HashMap, HashSet},
    fs,
    io::ErrorKind,
    path::Path,
    time::Duration,
};

use rusqlite::{params, Connection, OptionalExtension, Row, Transaction, TransactionBehavior};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use super::{
    legacy, message, AccountRule, AttemptContext, Document, Pending, Record, ResetObservation,
    Rule, AUTOMATIC_COOLDOWN_MS, STORAGE,
};
use crate::modules::{atomic_write, background_maintenance as maintenance};

const DATABASE: &str = "codex_reset_journal.sqlite";
const RULES: &str = "codex_reset_rules.json";
const VERSION: u32 = 3;
const HISTORY_LIMIT: usize = 100;

#[derive(Default, Serialize, Deserialize)]
struct Rules {
    #[serde(default)]
    revision: u64,
    public_rule: Rule,
    accounts: HashMap<String, AccountRule>,
}

#[derive(Default)]
struct Progress {
    revision: u64,
    base_revision: u64,
    stamp: Option<String>,
    cursor: legacy::Cursor,
    success_map: bool,
    source_done: bool,
    complete: bool,
}

fn error(reason: impl std::fmt::Display) -> String {
    format!("{}: {reason}", message("storageError"))
}

fn read_file<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    match fs::File::open(path) {
        Ok(file) => serde_json::from_reader(std::io::BufReader::new(file))
            .map(Some)
            .map_err(error),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(error(err)),
    }
}

fn open(path: &Path) -> Result<Connection, String> {
    let connection = Connection::open(path.with_file_name(DATABASE)).map_err(error)?;
    connection
        .busy_timeout(Duration::from_millis(150))
        .map_err(error)?;
    let version: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(error)?;
    if version > VERSION {
        return Err(error("用卡记录数据库版本不受支持"));
    }
    if version == 0 {
        connection.execute_batch(
            "BEGIN IMMEDIATE;
             CREATE TABLE IF NOT EXISTS reset_progress (
               id INTEGER PRIMARY KEY CHECK(id=1), revision INTEGER NOT NULL DEFAULT 0,
               base_revision INTEGER NOT NULL DEFAULT 0, stamp TEXT,
               cursor TEXT NOT NULL DEFAULT '{}', success_map INTEGER NOT NULL DEFAULT 0,
               source_done INTEGER NOT NULL DEFAULT 0, complete INTEGER NOT NULL DEFAULT 0
             );
             INSERT OR IGNORE INTO reset_progress(id) VALUES(1);
             CREATE TABLE IF NOT EXISTS reset_accounts (
               account_id TEXT PRIMARY KEY, pending TEXT, evidence TEXT,
               success_ms INTEGER, modified INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS reset_records (
               sequence INTEGER PRIMARY KEY, id TEXT NOT NULL UNIQUE, account_id TEXT NOT NULL,
               at INTEGER NOT NULL, reason TEXT NOT NULL, status TEXT NOT NULL, detail TEXT,
               legacy_order INTEGER
             );
             CREATE INDEX IF NOT EXISTS reset_record_order ON reset_records(legacy_order, sequence DESC);
             CREATE INDEX IF NOT EXISTS reset_record_account ON reset_records(account_id, at DESC);
             CREATE TABLE IF NOT EXISTS reset_import_rules (id TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS reset_import_success (account_id TEXT PRIMARY KEY, at INTEGER NOT NULL);
             PRAGMA user_version=1;
             COMMIT;"
        ).map_err(error)?;
    }
    if version < 2 {
        super::usage_history::initialize(&connection).map_err(error)?;
    }
    if version < 3 {
        super::usage_history::initialize_context(&connection).map_err(error)?;
    }
    Ok(connection)
}

fn progress(conn: &Connection) -> Result<Progress, String> {
    let (revision, base_revision, stamp, cursor, success_map, source_done, complete) = conn.query_row(
        "SELECT revision,base_revision,stamp,cursor,success_map,source_done,complete FROM reset_progress WHERE id=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get::<_, String>(3)?, row.get(4)?, row.get(5)?, row.get(6)?)),
    ).map_err(error)?;
    Ok(Progress {
        revision,
        base_revision,
        stamp,
        cursor: serde_json::from_str(&cursor).map_err(error)?,
        success_map,
        source_done,
        complete,
    })
}

fn read_rules(path: &Path, original: &Document, complete: bool) -> Result<Rules, String> {
    let path = path.with_file_name(RULES);
    match read_file(&path)? {
        Some(rules) => Ok(rules),
        None if complete => Err(error("用卡规则文件缺失")),
        None => Ok(Rules {
            revision: 0,
            public_rule: original.public_rule.clone(),
            accounts: original.accounts.clone(),
        }),
    }
}

fn read_original(path: &Path, state: &Progress) -> Result<Option<Document>, String> {
    if state.complete {
        return Ok(None);
    }
    let mut original = match read_file::<Document>(path)? {
        Some(document) => document,
        None if state.stamp.is_none() => Document::default(),
        None => return Err(error("旧用卡记录在迁移期间不可用")),
    };
    original.automatic_successes_mut();
    Ok(Some(original))
}

fn snapshot(
    conn: &Connection,
    path: &Path,
    state: &Progress,
    original: Option<Document>,
    account: Option<&str>,
    activities: bool,
) -> Result<(Document, Rules), String> {
    let mut document = if state.complete {
        Document {
            revision: state.base_revision,
            last_automatic_success_ms: Some(HashMap::new()),
            ..Document::default()
        }
    } else {
        original.ok_or_else(|| error("旧用卡记录在迁移期间不可用"))?
    };
    let rules = read_rules(path, &document, state.complete)?;
    document.public_rule = rules.public_rule.clone();
    document.accounts = rules.accounts.clone();
    document.revision = document
        .revision
        .checked_add(state.revision)
        .and_then(|revision| revision.checked_add(rules.revision))
        .ok_or_else(|| error("用卡记录版本溢出"))?;

    if let Some(account) = account {
        document.pending.retain(|key, _| key == account);
        document.last_evidence.retain(|key, _| key == account);
        document
            .automatic_successes_mut()
            .retain(|key, _| key == account);
        let entry = conn
            .query_row(
                "SELECT pending,evidence,success_ms FROM reset_accounts
             WHERE account_id=?1 AND (modified=1 OR ?2)",
                params![account, state.complete],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(error)?;
        if let Some((pending, evidence, success)) = entry {
            document.pending.remove(account);
            document.last_evidence.remove(account);
            document.automatic_successes_mut().remove(account);
            if let Some(pending) = pending {
                document.pending.insert(
                    account.to_string(),
                    serde_json::from_str(&pending).map_err(error)?,
                );
            }
            if let Some(evidence) = evidence {
                document.last_evidence.insert(account.to_string(), evidence);
            }
            if let Some(success) = success {
                document
                    .automatic_successes_mut()
                    .insert(account.to_string(), success);
            }
        }
    } else {
        document.pending.clear();
        document.last_evidence.clear();
        document.automatic_successes_mut().clear();
    }
    if activities {
        let mut records = read_records(conn, state.complete)?;
        if !state.complete {
            let mut ids: HashSet<_> = records.iter().map(|row| row.id.clone()).collect();
            records.extend(
                document
                    .records
                    .into_iter()
                    .filter(|row| ids.insert(row.id.clone())),
            );
            retain_activity_records(&mut records);
        }
        document.records = records;
    } else {
        document.records.clear();
    }
    Ok((document, rules))
}

pub(super) fn retain_activity_records(records: &mut Vec<Record>) {
    if records.len() <= HISTORY_LIMIT {
        return;
    }
    let successes = records.iter().filter(|row| row.status == "success").count();
    let mut successes_left = successes.min(HISTORY_LIMIT);
    let mut others_left = HISTORY_LIMIT - successes_left;
    records.retain(|row| {
        let left = if row.status == "success" {
            &mut successes_left
        } else {
            &mut others_left
        };
        if *left == 0 {
            return false;
        }
        *left -= 1;
        true
    });
}

fn read_records(conn: &Connection, complete: bool) -> Result<Vec<Record>, String> {
    let mut query = conn
        .prepare(
            "SELECT id,account_id,at,reason,status,detail FROM reset_records
         WHERE legacy_order IS NULL OR ?1
         ORDER BY legacy_order,sequence DESC LIMIT ?2",
        )
        .map_err(error)?;
    let rows = query
        .query_map(params![complete, HISTORY_LIMIT], read_record)
        .map_err(error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(error)
}

fn read_record(row: &Row<'_>) -> rusqlite::Result<Record> {
    Ok(Record {
        id: row.get(0)?,
        account_id: row.get(1)?,
        at: row.get(2)?,
        reason: row.get(3)?,
        status: row.get(4)?,
        detail: row.get(5)?,
    })
}

fn store_record(tx: &Transaction<'_>, row: &Record, order: Option<u64>) -> Result<(), String> {
    tx.execute(
        "INSERT INTO reset_records(id,account_id,at,reason,status,detail,legacy_order)
         VALUES (?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(id) DO NOTHING",
        params![
            row.id,
            row.account_id,
            row.at,
            row.reason,
            row.status,
            row.detail,
            order
        ],
    )
    .map_err(error)?;
    Ok(())
}

fn trim_records(tx: &Transaction<'_>) -> Result<(), String> {
    tx.execute(
        "DELETE FROM reset_records WHERE sequence IN (
           SELECT sequence FROM reset_records
           ORDER BY (status = 'success') DESC,legacy_order,sequence DESC LIMIT -1 OFFSET ?1
         )",
        [HISTORY_LIMIT],
    )
    .map_err(error)?;
    Ok(())
}

pub(super) fn read(
    path: &Path,
    account: Option<&str>,
    activities: bool,
) -> Result<Document, String> {
    let mut conn = open(path)?;
    let original = read_original(path, &progress(&conn)?)?;
    let tx = conn.transaction().map_err(error)?;
    let state = progress(&tx)?;
    snapshot(&tx, path, &state, original, account, activities).map(|(document, _)| document)
}

pub(super) fn update_account<T>(
    path: &Path,
    account: &str,
    edit: impl FnOnce(&mut Document) -> Result<T, String>,
) -> Result<T, String> {
    let mut conn = open(path)?;
    let original = read_original(path, &progress(&conn)?)?;
    let _lock = STORAGE.lock().map_err(|_| message("storageError"))?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(error)?;
    let state = progress(&tx)?;
    let (mut document, _) = snapshot(&tx, path, &state, original, Some(account), true)?;
    let existing: HashSet<_> = document.records.iter().map(|row| row.id.clone()).collect();
    document.revision = document
        .revision
        .checked_add(1)
        .ok_or_else(|| error("用卡记录版本溢出"))?;
    let result = edit(&mut document)?;
    let pending = document
        .pending
        .get(account)
        .map(serde_json::to_string)
        .transpose()
        .map_err(error)?;
    let success = document
        .last_automatic_success_ms
        .as_ref()
        .and_then(|values| values.get(account));
    tx.execute(
        "INSERT INTO reset_accounts(account_id,pending,evidence,success_ms,modified) VALUES (?1,?2,?3,?4,1)
         ON CONFLICT(account_id) DO UPDATE SET pending=excluded.pending,evidence=excluded.evidence,
           success_ms=excluded.success_ms,modified=1",
        params![account, pending, document.last_evidence.get(account), success],
    ).map_err(error)?;
    for row in document
        .records
        .iter()
        .rev()
        .filter(|row| !existing.contains(&row.id))
    {
        store_record(&tx, row, None)?;
    }
    trim_records(&tx)?;
    for update in &document.usage_updates {
        super::usage_history::store(&tx, update).map_err(error)?;
    }
    tx.execute(
        "UPDATE reset_progress SET revision=revision+1 WHERE id=1",
        [],
    )
    .map_err(error)?;
    tx.commit().map_err(error)?;
    Ok(result)
}

pub(super) fn usage_history(
    path: &Path,
    account: &str,
    identity: &str,
    cursor: Option<i64>,
) -> Result<super::UsagePage, String> {
    super::usage_history::read(&open(path)?, account, identity, cursor).map_err(error)
}

pub(super) fn recent_no_op(
    path: &Path,
    account: &str,
    identity: &str,
    reason: &str,
    context: &AttemptContext,
    cutoff: i64,
) -> Result<bool, String> {
    super::usage_history::recent_no_op(&open(path)?, account, identity, reason, context, cutoff)
        .map_err(error)
}

pub(super) fn store_after_observation(
    path: &Path,
    account: &str,
    pending: &Pending,
    observation: &ResetObservation,
) -> Result<(), String> {
    super::usage_history::store_after_observation(&open(path)?, account, pending, observation)
        .map_err(error)
}

pub(super) fn update_rules<T>(
    path: &Path,
    edit: impl FnOnce(&mut Document) -> Result<T, String>,
) -> Result<T, String> {
    let mut conn = open(path)?;
    let original = read_original(path, &progress(&conn)?)?;
    let _lock = STORAGE.lock().map_err(|_| message("storageError"))?;
    let tx = conn.transaction().map_err(error)?;
    let (mut document, mut rules) = snapshot(&tx, path, &progress(&tx)?, original, None, true)?;
    tx.commit().map_err(error)?;
    rules.revision = rules
        .revision
        .checked_add(1)
        .ok_or_else(|| error("用卡规则版本溢出"))?;
    document.revision = document
        .revision
        .checked_add(1)
        .ok_or_else(|| error("用卡记录版本溢出"))?;
    let result = edit(&mut document)?;
    rules.public_rule = document.public_rule;
    rules.accounts = document.accounts;
    atomic_write::write_string_atomic(
        &path.with_file_name(RULES),
        &serde_json::to_string_pretty(&rules).map_err(error)?,
    )?;
    Ok(result)
}

fn store_imported(tx: &Transaction<'_>, item: legacy::Item) -> Result<(), String> {
    match item {
        legacy::Item::Revision(revision) => {
            tx.execute(
                "UPDATE reset_progress SET base_revision=?1 WHERE id=1",
                [revision],
            )
            .map_err(error)?;
        }
        legacy::Item::PublicRule(rule) => {
            tx.execute(
                "INSERT OR REPLACE INTO reset_import_rules(id,value) VALUES('',?1)",
                [serde_json::to_string(&rule).map_err(error)?],
            )
            .map_err(error)?;
        }
        legacy::Item::AccountRule(id, rule) => {
            tx.execute(
                "INSERT OR REPLACE INTO reset_import_rules(id,value) VALUES(?1,?2)",
                params![id, serde_json::to_string(&rule).map_err(error)?],
            )
            .map_err(error)?;
        }
        legacy::Item::Pending(id, pending) => {
            tx.execute(
                "INSERT INTO reset_accounts(account_id,pending) VALUES(?1,?2)
                 ON CONFLICT(account_id) DO UPDATE SET pending=excluded.pending WHERE modified=0",
                params![id, serde_json::to_string(&pending).map_err(error)?],
            )
            .map_err(error)?;
        }
        legacy::Item::Evidence(id, evidence) => {
            tx.execute(
                "INSERT INTO reset_accounts(account_id,evidence) VALUES(?1,?2)
                 ON CONFLICT(account_id) DO UPDATE SET evidence=excluded.evidence WHERE modified=0",
                params![id, evidence],
            )
            .map_err(error)?;
        }
        legacy::Item::Success(id, at) => {
            tx.execute(
                "INSERT INTO reset_accounts(account_id,success_ms) VALUES(?1,?2)
                 ON CONFLICT(account_id) DO UPDATE SET success_ms=excluded.success_ms WHERE modified=0",
                params![id, at],
            ).map_err(error)?;
        }
        legacy::Item::SuccessMap => {
            tx.execute("UPDATE reset_progress SET success_map=1 WHERE id=1", [])
                .map_err(error)?;
        }
        legacy::Item::Record(order, row) => {
            if row.status == "success" && super::is_automatic_reason(&row.reason) {
                tx.execute(
                    "INSERT INTO reset_import_success(account_id,at) VALUES(?1,?2)
                     ON CONFLICT(account_id) DO UPDATE SET at=MAX(at,excluded.at)",
                    params![row.account_id, row.at.saturating_mul(1000)],
                )
                .map_err(error)?;
            }
            store_record(tx, &row, Some(order))?;
        }
    }
    Ok(())
}

fn finalize(conn: &mut Connection, path: &Path, state: &Progress) -> Result<bool, String> {
    let _lock = match STORAGE.try_lock() {
        Ok(lock) => lock,
        Err(std::sync::TryLockError::WouldBlock) => return Ok(false),
        Err(_) => return Err(message("storageError")),
    };
    let rules_path = path.with_file_name(RULES);
    if !rules_path.try_exists().map_err(error)? {
        let mut rules = Rules::default();
        let mut query = conn
            .prepare("SELECT id,value FROM reset_import_rules")
            .map_err(error)?;
        let mut rows = query.query([]).map_err(error)?;
        while let Some(row) = rows.next().map_err(error)? {
            let id: String = row.get(0).map_err(error)?;
            let value: String = row.get(1).map_err(error)?;
            if id.is_empty() {
                rules.public_rule = serde_json::from_str(&value).map_err(error)?;
            } else {
                rules
                    .accounts
                    .insert(id, serde_json::from_str(&value).map_err(error)?);
            }
        }
        // Write the independent rule snapshot before committing completion.
        atomic_write::write_string_atomic(
            &rules_path,
            &serde_json::to_string_pretty(&rules).map_err(error)?,
        )?;
    }
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(error)?;
    if !state.success_map {
        let cutoff = chrono::Utc::now().timestamp_millis() - AUTOMATIC_COOLDOWN_MS;
        tx.execute(
            "INSERT INTO reset_accounts(account_id,success_ms)
             SELECT account_id,at FROM reset_import_success WHERE at>?1
             ON CONFLICT(account_id) DO UPDATE SET success_ms=excluded.success_ms WHERE modified=0",
            [cutoff],
        )
        .map_err(error)?;
    }
    tx.execute_batch("DELETE FROM reset_import_rules; DELETE FROM reset_import_success;")
        .map_err(error)?;
    trim_records(&tx)?;
    tx.execute("UPDATE reset_progress SET complete=1 WHERE id=1", [])
        .map_err(error)?;
    tx.commit().map_err(error)?;
    Ok(true)
}

pub(super) fn migrate(path: &Path) -> Result<bool, String> {
    let mut conn = open(path)?;
    let before = progress(&conn)?;
    if before.complete {
        return Ok(true);
    }
    if before.source_done {
        return finalize(&mut conn, path, &before);
    }
    let batch = legacy::read(path, before.stamp.as_deref(), &before.cursor)?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(error)?;
    let latest = progress(&tx)?;
    if latest.complete {
        return Ok(true);
    }
    if latest.source_done || latest.cursor != before.cursor || latest.stamp != before.stamp {
        return Ok(false);
    }
    for item in batch.items {
        store_imported(&tx, item)?;
    }
    tx.execute(
        "UPDATE reset_progress SET stamp=?1,cursor=?2,source_done=?3 WHERE id=1",
        params![
            batch.stamp,
            serde_json::to_string(&batch.cursor).map_err(error)?,
            batch.complete
        ],
    )
    .map_err(error)?;
    tx.commit().map_err(error)?;
    Ok(false)
}

pub(super) fn schedule(path: &Path) {
    let path = path.to_path_buf();
    maintenance::schedule(
        format!("codex-reset-journal:{}", path.display()),
        move || migrate(&path),
    );
}
