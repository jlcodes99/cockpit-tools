use std::{collections::HashSet, fs, io::ErrorKind, path::Path, time::Duration};

use rusqlite::{params, Connection, OptionalExtension, Row, Transaction, TransactionBehavior};
use serde::de::DeserializeOwned;

use super::{legacy, message, Document, Record, STORAGE};
use crate::modules::background_maintenance as maintenance;

const DATABASE: &str = "codex_reset_journal.sqlite";
const VERSION: u32 = 3;
const HISTORY_LIMIT: usize = 100;

#[derive(Default)]
struct Progress {
    revision: u64,
    base_revision: u64,
    stamp: Option<String>,
    cursor: legacy::Cursor,
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
    let (revision, base_revision, stamp, cursor, source_done, complete) = conn.query_row(
        "SELECT revision,base_revision,stamp,cursor,source_done,complete FROM reset_progress WHERE id=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get::<_, String>(3)?, row.get(4)?, row.get(5)?)),
    ).map_err(error)?;
    Ok(Progress {
        revision,
        base_revision,
        stamp,
        cursor: serde_json::from_str(&cursor).map_err(error)?,
        source_done,
        complete,
    })
}

fn read_original(path: &Path, state: &Progress) -> Result<Option<Document>, String> {
    if state.complete {
        return Ok(None);
    }
    let original = match read_file::<Document>(path)? {
        Some(document) => document,
        None if state.stamp.is_none() => Document::default(),
        None => return Err(error("旧用卡记录在迁移期间不可用")),
    };
    Ok(Some(original))
}

fn snapshot(
    conn: &Connection,
    state: &Progress,
    original: Option<Document>,
    account: Option<&str>,
    activities: bool,
) -> Result<Document, String> {
    let mut document = if state.complete {
        Document {
            revision: state.base_revision,
            ..Document::default()
        }
    } else {
        original.ok_or_else(|| error("旧用卡记录在迁移期间不可用"))?
    };
    document.revision = document
        .revision
        .checked_add(state.revision)
        .ok_or_else(|| error("用卡记录版本溢出"))?;

    if let Some(account) = account {
        document.pending.retain(|key, _| key == account);
        let entry = conn
            .query_row(
                "SELECT pending FROM reset_accounts
             WHERE account_id=?1 AND (modified=1 OR ?2)",
                params![account, state.complete],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(error)?;
        if let Some(pending) = entry {
            document.pending.remove(account);
            if let Some(pending) = pending {
                document.pending.insert(
                    account.to_string(),
                    serde_json::from_str(&pending).map_err(error)?,
                );
            }
        }
    } else {
        document.pending.clear();
    }
    if activities {
        #[cfg(test)]
        {
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
        }
        #[cfg(not(test))]
        document.records.clear();
    } else {
        document.records.clear();
    }
    Ok(document)
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

#[cfg(test)]
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

#[cfg(test)]
pub(super) fn read(
    path: &Path,
    account: Option<&str>,
    activities: bool,
) -> Result<Document, String> {
    let mut conn = open(path)?;
    let original = read_original(path, &progress(&conn)?)?;
    let tx = conn.transaction().map_err(error)?;
    let state = progress(&tx)?;
    snapshot(&tx, &state, original, account, activities)
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
    let mut document = snapshot(&tx, &state, original, Some(account), false)?;
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
    tx.execute(
        "INSERT INTO reset_accounts(account_id,pending,modified) VALUES (?1,?2,1)
         ON CONFLICT(account_id) DO UPDATE SET pending=excluded.pending,modified=1",
        params![account, pending],
    )
    .map_err(error)?;
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

fn read_account_records(
    conn: &Connection,
    path: &Path,
    account: &str,
) -> Result<Vec<Record>, String> {
    let state = progress(conn)?;
    let mut records = {
        let mut query = conn
            .prepare(
                "SELECT id,account_id,at,reason,status,detail FROM reset_records
                 WHERE account_id=?1 AND (legacy_order IS NULL OR ?2)
                 ORDER BY legacy_order,sequence DESC LIMIT ?3",
            )
            .map_err(error)?;
        let rows = query
            .query_map(params![account, state.complete, HISTORY_LIMIT], read_record)
            .map_err(error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(error)?
    };
    if !state.complete {
        if let Some(original) = read_original(path, &state)? {
            let mut ids: HashSet<_> = records.iter().map(|row| row.id.clone()).collect();
            records.extend(
                original
                    .records
                    .into_iter()
                    .filter(|row| row.account_id == account && ids.insert(row.id.clone())),
            );
            retain_activity_records(&mut records);
        }
    }
    Ok(records)
}

pub(super) fn usage_history(
    path: &Path,
    account: &str,
    identity: &str,
    cursor: Option<i64>,
) -> Result<super::UsagePage, String> {
    let conn = open(path)?;
    let mut page = super::usage_history::read(&conn, account, identity, cursor).map_err(error)?;
    if cursor.is_none() {
        page.legacy_records = read_account_records(&conn, path, account)?;
    }
    Ok(page)
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
        legacy::Item::Pending(id, pending) => {
            tx.execute(
                "INSERT INTO reset_accounts(account_id,pending) VALUES(?1,?2)
                 ON CONFLICT(account_id) DO UPDATE SET pending=excluded.pending WHERE modified=0",
                params![id, serde_json::to_string(&pending).map_err(error)?],
            )
            .map_err(error)?;
        }
        legacy::Item::Record(order, row) => {
            store_record(tx, &row, Some(order))?;
        }
    }
    Ok(())
}

fn finalize(conn: &mut Connection) -> Result<bool, String> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(error)?;
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
        return finalize(&mut conn);
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
