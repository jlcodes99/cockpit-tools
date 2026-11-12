//! Indexed results and batch history. Legacy input is read-only during import.
use super::{
    WakeupVerificationBatchHistoryItem as Batch, WakeupVerificationStateItem as Item,
    MAX_HISTORY_BATCHES,
};
use crate::modules::{
    background_maintenance as maintenance,
    legacy_json_input::{Input, ReadError},
    wakeup_history_store::{schedule_tracked, track_read, HistoryRead},
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::File,
    io::{BufReader, ErrorKind},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Clone)]
pub(super) struct VerificationStore {
    legacy: PathBuf,
}

impl VerificationStore {
    pub(super) fn new(legacy: PathBuf) -> Self {
        Self { legacy }
    }

    fn open(&self) -> Result<Connection, String> {
        let conn = Connection::open(self.legacy.with_extension("sqlite")).map_err(error)?;
        conn.busy_timeout(Duration::from_millis(100))
            .map_err(error)?;
        let version: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(error)?;
        match version {
            1 => {}
            0 => conn
                .execute_batch(
                    "BEGIN IMMEDIATE;
                 CREATE TABLE IF NOT EXISTS account_results (
                    account_id TEXT PRIMARY KEY, verified_at INTEGER NOT NULL,
                    source INTEGER NOT NULL, payload TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS batch_history (
                    batch_id TEXT PRIMARY KEY, verified_at INTEGER NOT NULL,
                    source INTEGER NOT NULL, payload TEXT NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS batch_by_source_time
                    ON batch_history(source, verified_at DESC, batch_id);
                 CREATE INDEX IF NOT EXISTS batch_by_time
                    ON batch_history(verified_at DESC, batch_id);
                 CREATE TABLE IF NOT EXISTS deleted_batches (batch_id TEXT PRIMARY KEY);
                 CREATE TABLE IF NOT EXISTS migration (
                    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
                    progress TEXT NOT NULL
                 );
                 PRAGMA user_version=1;
                 COMMIT;",
                )
                .map_err(error)?,
            _ => return Err("唤醒验证数据库版本不受支持".into()),
        }
        Ok(conn)
    }

    fn migration_key(&self) -> String {
        format!("wakeup-verification:{}", self.legacy.display())
    }

    pub(super) fn schedule(&self) {
        let store = self.clone();
        schedule_tracked(self.migration_key(), move || store.migrate_batch());
    }

    pub(super) fn load_results(&self, retry: bool) -> Result<HistoryRead<Item>, String> {
        self.load_available(retry, |conn, source| {
            read_payloads(
                conn,
                "SELECT payload FROM account_results WHERE source=0 OR source=?1",
                [source],
            )
        })
    }

    pub(super) fn load_history(&self, retry: bool) -> Result<HistoryRead<Batch>, String> {
        self.load_available(retry, |conn, source| {
            read_payloads(
                conn,
                "SELECT payload FROM batch_history WHERE source=0 OR source=?1
                 ORDER BY verified_at DESC, batch_id LIMIT ?2",
                params![source, MAX_HISTORY_BATCHES],
            )
        })
    }

    fn load_available<T>(
        &self,
        retry: bool,
        read: impl FnOnce(&Connection, i64) -> Result<Vec<T>, String>,
    ) -> Result<HistoryRead<T>, String> {
        let mut conn = self.open()?;
        let tx = conn.transaction().map_err(error)?;
        let state = progress(&tx)?;
        let rows = read(&tx, state.source())?;
        tx.commit().map_err(error)?;
        let pending = !state.complete
            && (state.stamp.is_some()
                || state.backup
                || self.legacy.try_exists().unwrap_or(true)
                || self
                    .legacy
                    .with_extension("json.bak")
                    .try_exists()
                    .unwrap_or(true));
        Ok(track_read(
            rows,
            pending,
            &self.migration_key(),
            retry,
            || self.schedule(),
        ))
    }

    pub(super) fn upsert_result(&self, item: &Item) -> Result<bool, String> {
        let conn = self.open()?;
        let applied = put_result(&conn, item, 0)?;
        if !progress(&conn)?.complete {
            self.schedule();
        }
        Ok(applied)
    }

    pub(super) fn append_batch(&self, batch: &Batch) -> Result<(), String> {
        let mut conn = self.open()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(error)?;
        let state = progress(&tx)?;
        put_batch(&tx, batch, 0)?;
        trim_batches(&tx, state.complete.then_some(state.source()), 0)?;
        tx.commit().map_err(error)?;
        if !state.complete {
            self.schedule();
        }
        Ok(())
    }

    pub(super) fn delete_batches(&self, targets: &HashSet<String>) -> Result<usize, String> {
        let mut conn = self.open()?;
        let before = progress(&conn)?;
        let mut unknown = HashSet::new();
        if !before.complete {
            let mut query = conn.prepare(
                "SELECT EXISTS(SELECT 1 FROM batch_history WHERE batch_id=?1 AND (source=0 OR source=?2))
                    OR EXISTS(SELECT 1 FROM deleted_batches WHERE batch_id=?1)"
            ).map_err(error)?;
            for id in targets {
                let known: bool = query
                    .query_row(params![id, before.source()], |row| row.get(0))
                    .map_err(error)?;
                if !known {
                    unknown.insert(id.as_str());
                }
            }
        }
        let legacy: HashSet<String> = if unknown.is_empty() {
            HashSet::new()
        } else {
            self.read_legacy::<LegacyHistory>(&before)?
                .history
                .into_iter()
                .map(|batch| batch.batch_id)
                .filter(|id| unknown.contains(id.as_str()))
                .collect()
        };
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(error)?;
        let state = progress(&tx)?;
        let mut deleted = 0;
        for id in targets {
            let removed = tx
                .execute(
                    "DELETE FROM batch_history WHERE batch_id=?1 AND (source=0 OR source=?2 OR ?3)",
                    params![id, state.visible_source(), !state.complete],
                )
                .map_err(error)?;
            if !state.complete && (removed > 0 || legacy.contains(id)) {
                let added = tx
                    .execute(
                        "INSERT OR IGNORE INTO deleted_batches(batch_id) VALUES (?1)",
                        [id],
                    )
                    .map_err(error)?;
                deleted += usize::from(added > 0);
            } else {
                deleted += removed;
            }
        }
        tx.commit().map_err(error)?;
        Ok(deleted)
    }

    fn read_legacy<T: Default + DeserializeOwned>(&self, state: &Progress) -> Result<T, String> {
        let backup = self.legacy.with_extension("json.bak");
        if state.backup {
            return read_snapshot(&backup, true);
        }
        if !self.legacy.try_exists().map_err(error)? && backup.try_exists().map_err(error)? {
            return read_snapshot(&backup, true);
        }
        match read_snapshot(&self.legacy, state.stamp.is_some()) {
            Ok(value) => Ok(value),
            Err(primary) => read_snapshot(&backup, true).map_err(|_| primary),
        }
    }

    fn migrate_batch(&self) -> Result<bool, String> {
        let mut conn = self.open()?;
        let before = progress(&conn)?;
        if before.complete {
            return Ok(true);
        }
        let mut next = before.clone();
        let mut rows = Vec::new();
        let path = if next.backup {
            self.legacy.with_extension("json.bak")
        } else {
            self.legacy.clone()
        };
        match File::open(&path) {
            Ok(file) => {
                let metadata = file.metadata().map_err(error)?;
                let stamp = format!(
                    "{}:{:?}",
                    metadata.len(),
                    metadata.modified().map_err(error)?
                );
                if next
                    .stamp
                    .as_ref()
                    .is_some_and(|previous| previous != &stamp)
                {
                    return Err("迁移期间旧唤醒验证记录发生变化".into());
                }
                next.stamp = Some(stamp);
                let mut input = Input::new(file, next.cursor.offset, maintenance::BATCH_BYTES)
                    .map_err(error)?;
                match read_batch(&mut input, &mut next) {
                    Ok(batch) => rows = batch,
                    Err(ReadError::Format)
                        if !next.backup
                            && self
                                .legacy
                                .with_extension("json.bak")
                                .try_exists()
                                .map_err(error)? =>
                    {
                        next = Progress {
                            backup: true,
                            ..Progress::default()
                        };
                    }
                    Err(cause) => return Err(legacy_error(cause)),
                }
            }
            Err(cause)
                if cause.kind() == ErrorKind::NotFound
                    && !next.backup
                    && self
                        .legacy
                        .with_extension("json.bak")
                        .try_exists()
                        .map_err(error)? =>
            {
                next = Progress {
                    backup: true,
                    ..Progress::default()
                };
            }
            Err(cause)
                if cause.kind() == ErrorKind::NotFound && !next.backup && next.stamp.is_none() =>
            {
                next.complete = true;
            }
            Err(cause) => return Err(error(cause)),
        }

        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(error)?;
        if progress(&tx)? != before {
            return Ok(false);
        }
        let source = next.source();
        for row in rows {
            match row {
                LegacyRow::Result(item) => {
                    put_result(&tx, &item, source)?;
                }
                LegacyRow::Batch(batch) => put_batch(&tx, &batch, source)?,
            }
        }
        trim_batches(&tx, next.complete.then_some(source), source)?;
        tx.execute(
            "INSERT INTO migration(singleton,progress) VALUES (1,?1)
             ON CONFLICT(singleton) DO UPDATE SET progress=excluded.progress",
            [encode(&next)?],
        )
        .map_err(error)?;
        tx.commit().map_err(error)?;
        Ok(next.complete)
    }
}

fn put_result(conn: &Connection, item: &Item, source: i64) -> Result<bool, String> {
    let updated = conn.execute(
        "INSERT INTO account_results(account_id,verified_at,source,payload) VALUES (?1,?2,?3,?4)
         ON CONFLICT(account_id) DO UPDATE SET verified_at=excluded.verified_at,
            source=excluded.source,payload=excluded.payload
         WHERE (account_results.source<>0 AND account_results.source<>excluded.source)
           OR (account_results.source=excluded.source AND excluded.verified_at>=account_results.verified_at)",
        params![item.account_id, item.last_verify_at.unwrap_or_default(), source, encode(item)?],
    ).map_err(error)?;
    Ok(updated > 0)
}

fn put_batch(conn: &Connection, batch: &Batch, source: i64) -> Result<(), String> {
    conn.execute(
        "INSERT INTO batch_history(batch_id,verified_at,source,payload)
         SELECT ?1,?2,?3,?4 WHERE NOT EXISTS (SELECT 1 FROM deleted_batches WHERE batch_id=?1)
         ON CONFLICT(batch_id) DO UPDATE SET verified_at=excluded.verified_at,
            source=excluded.source,payload=excluded.payload
         WHERE batch_history.source<>0 OR excluded.source=0",
        params![batch.batch_id, batch.verified_at, source, encode(batch)?],
    )
    .map_err(error)?;
    Ok(())
}

fn trim_batches(conn: &Connection, include_live: Option<i64>, scope: i64) -> Result<(), String> {
    if let Some(source) = include_live {
        conn.execute(
            "DELETE FROM batch_history WHERE (source=0 OR source=?1) AND batch_id IN (
                SELECT batch_id FROM batch_history WHERE source=0 OR source=?1
                ORDER BY verified_at DESC,batch_id LIMIT -1 OFFSET ?2
             )",
            params![source, MAX_HISTORY_BATCHES],
        )
        .map_err(error)?;
    } else {
        conn.execute(
            "DELETE FROM batch_history WHERE source=?1 AND batch_id IN (
                SELECT batch_id FROM batch_history WHERE source=?1
                ORDER BY verified_at DESC,batch_id LIMIT -1 OFFSET ?2
             )",
            params![scope, MAX_HISTORY_BATCHES],
        )
        .map_err(error)?;
    }
    Ok(())
}

fn read_payloads<T: DeserializeOwned>(
    conn: &Connection,
    sql: &str,
    parameters: impl rusqlite::Params,
) -> Result<Vec<T>, String> {
    let mut query = conn.prepare(sql).map_err(error)?;
    let rows = query
        .query_map(parameters, |row| row.get::<_, String>(0))
        .map_err(error)?;
    rows.map(|row| serde_json::from_str(&row.map_err(error)?).map_err(error))
        .collect()
}

#[derive(Default, Deserialize)]
struct LegacyHistory {
    #[serde(default)]
    history: Vec<Batch>,
}

fn read_snapshot<T: Default + DeserializeOwned>(path: &Path, required: bool) -> Result<T, String> {
    use std::io::BufRead;
    let file = match File::open(path) {
        Ok(file) => file,
        Err(cause) if cause.kind() == ErrorKind::NotFound && !required => return Ok(T::default()),
        Err(cause) => return Err(error(cause)),
    };
    let mut input = BufReader::new(file);
    loop {
        let buffer = input.fill_buf().map_err(error)?;
        if buffer.is_empty() {
            return Ok(T::default());
        }
        let whitespace = buffer
            .iter()
            .take_while(|byte| byte.is_ascii_whitespace())
            .count();
        let has_content = whitespace < buffer.len();
        input.consume(whitespace);
        if has_content {
            break;
        }
    }
    serde_json::from_reader(input).map_err(error)
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
struct Progress {
    backup: bool,
    stamp: Option<String>,
    cursor: Cursor,
    complete: bool,
}

impl Progress {
    fn source(&self) -> i64 {
        if self.backup {
            2
        } else {
            1
        }
    }
    fn visible_source(&self) -> i64 {
        if self.complete {
            self.source()
        } else {
            0
        }
    }
}

fn progress(conn: &Connection) -> Result<Progress, String> {
    let value: Option<String> = conn
        .query_row(
            "SELECT progress FROM migration WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(error)?;
    value
        .map(|value| serde_json::from_str(&value).map_err(error))
        .transpose()
        .map(Option::unwrap_or_default)
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
struct Cursor {
    offset: u64,
    section: Section,
    next_item: bool,
}

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
enum Section {
    #[default]
    Root,
    Items,
    History,
}

enum LegacyRow {
    Result(Item),
    Batch(Batch),
}

fn read_batch(input: &mut Input, state: &mut Progress) -> Result<Vec<LegacyRow>, ReadError> {
    let started = Instant::now();
    let start = state.cursor.offset;
    let mut rows = Vec::new();
    if start == 0 {
        if input.peek()?.is_none() {
            state.complete = true;
            return Ok(rows);
        }
        input.expect(b'{')?;
    }
    loop {
        match read_item(input, state) {
            Ok(Some(row)) => rows.push(row),
            Ok(None) => {}
            Err(ReadError::Budget) if state.cursor.offset > start => break,
            Err(cause) => return Err(cause),
        }
        state.cursor.offset = input.offset;
        if state.complete
            || rows.len() >= maintenance::BATCH_ROWS
            || input.budget == 0
            || started.elapsed() >= maintenance::BATCH_TIME
        {
            break;
        }
    }
    Ok(rows)
}

fn read_item(input: &mut Input, state: &mut Progress) -> Result<Option<LegacyRow>, ReadError> {
    let cursor = &mut state.cursor;
    let end = if cursor.section == Section::Root {
        b'}'
    } else {
        b']'
    };
    if input.peek()? == Some(end) {
        input.take()?;
        if cursor.section == Section::Root {
            if input.peek()?.is_some() {
                return Err(ReadError::Format);
            }
            state.complete = true;
        } else {
            cursor.section = Section::Root;
            cursor.next_item = true;
        }
        return Ok(None);
    }
    if cursor.next_item {
        input.expect(b',')?;
    }
    let row = match cursor.section {
        Section::Items => Some(LegacyRow::Result(input.value()?)),
        Section::History => Some(LegacyRow::Batch(input.value()?)),
        Section::Root => {
            let key = input.value::<String>()?;
            input.expect(b':')?;
            let section = match key.as_str() {
                "items" => Some(Section::Items),
                "history" => Some(Section::History),
                _ => None,
            };
            if let Some(section) = section {
                input.expect(b'[')?;
                cursor.section = section;
                cursor.next_item = false;
                return Ok(None);
            }
            input.value::<serde::de::IgnoredAny>()?;
            None
        }
    };
    cursor.next_item = true;
    Ok(row)
}

fn encode(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_string(value).map_err(error)
}

fn error(cause: impl std::fmt::Display) -> String {
    format!("读写唤醒验证记录失败: {cause}")
}

fn legacy_error(cause: ReadError) -> String {
    match cause {
        ReadError::Budget => "旧唤醒验证记录单项超出迁移预算".into(),
        ReadError::Format => "旧唤醒验证记录格式无效，原文件已保留".into(),
        ReadError::Io(cause) => error(cause),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn item(id: &str, timestamp: i64, status: &str) -> Item {
        Item {
            account_id: id.into(),
            account_email: format!("{id}@example.test"),
            status: status.into(),
            last_verify_at: Some(timestamp),
            last_model: None,
            last_error_code: None,
            last_message: None,
            validation_url: None,
            appeal_url: None,
            trajectory_id: None,
            duration_ms: None,
        }
    }

    fn batch(id: &str, timestamp: i64) -> Batch {
        Batch {
            batch_id: id.into(),
            verified_at: timestamp,
            model: "model".into(),
            prompt: "hi".into(),
            total: 1,
            completed: 1,
            success_count: 1,
            verification_required_count: 0,
            tos_violation_count: 0,
            failed_count: 0,
            records: vec![item("account", timestamp, "success")],
        }
    }

    fn finish(store: &VerificationStore) {
        for _ in 0..20 {
            if store.migrate_batch().unwrap() {
                return;
            }
        }
        panic!("bounded fixture should finish migrating");
    }

    #[test]
    fn committed_verification_batches_can_be_read_and_deleted_with_unreadable_legacy_data() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("verification.json");
        let store = VerificationStore::new(path.clone());
        let conn = store.open().unwrap();
        put_batch(&conn, &batch("available", 10), 1).unwrap();
        put_result(&conn, &item("account", 10, "success"), 1).unwrap();
        fs::write(&path, "invalid legacy JSON").unwrap();

        let history = store.load_history(false).unwrap();
        assert!(!history.is_complete());
        assert_eq!(history.records[0].batch_id, "available");
        let results = store.load_results(false).unwrap();
        assert_eq!(results.records[0].status, "success");
        let targets = HashSet::from(["available".into()]);
        assert_eq!(store.delete_batches(&targets).unwrap(), 1);
        assert_eq!(store.delete_batches(&targets).unwrap(), 0);
        put_batch(&conn, &batch("available", 10), 1).unwrap();
        assert!(store.load_history(false).unwrap().records.is_empty());
        assert_eq!(fs::read_to_string(path).unwrap(), "invalid legacy JSON");
    }

    #[test]
    fn wakeup_verification_resumes_without_restoring_deleted_or_replacing_live_records() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("verification.json");
        let old_results: Vec<_> = (0..75)
            .map(|i| item(&format!("account-{i}"), i, "failed"))
            .collect();
        let old_history: Vec<_> = (0..105).map(|i| batch(&format!("batch-{i}"), i)).collect();
        let original = serde_json::to_vec(&serde_json::json!({
            "version": 2, "items": old_results, "history": old_history,
        }))
        .unwrap();
        fs::write(&path, &original).unwrap();

        let store = VerificationStore::new(path.clone());
        assert!(!store.migrate_batch().unwrap());
        let first = progress(&store.open().unwrap()).unwrap();
        assert!(first.cursor.offset > 0);
        assert!(store
            .upsert_result(&item("account-74", 150, "success"))
            .unwrap());
        assert!(!store
            .upsert_result(&item("account-74", 149, "failed"))
            .unwrap());
        store.append_batch(&batch("new-batch", 200)).unwrap();
        for _ in 0..20 {
            let copied: bool = store
                .open()
                .unwrap()
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM batch_history WHERE batch_id='batch-30')",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            if copied {
                break;
            }
            assert!(!store.migrate_batch().unwrap());
        }
        let delete = HashSet::from(["batch-30".to_string(), "batch-104".to_string()]);
        assert_eq!(store.delete_batches(&delete).unwrap(), 2);
        assert_eq!(store.delete_batches(&delete).unwrap(), 0);

        let restarted = VerificationStore::new(path.clone());
        finish(&restarted);
        let results = restarted.load_results(false).unwrap().records;
        assert_eq!(results.len(), 75);
        assert_eq!(
            results
                .iter()
                .find(|row| row.account_id == "account-74")
                .unwrap()
                .status,
            "success"
        );
        let history = restarted.load_history(false).unwrap().records;
        assert_eq!(history.len(), MAX_HISTORY_BATCHES);
        assert_eq!(history[0].batch_id, "new-batch");
        assert!(!history.iter().any(|batch| delete.contains(&batch.batch_id)));
        assert_eq!(fs::read(&path).unwrap(), original);
        fs::write(&path, "unreadable after cutover").unwrap();
        assert_eq!(restarted.load_results(false).unwrap().records.len(), 75);
    }

    #[test]
    fn wakeup_verification_switches_from_partially_copied_corrupt_primary_to_backup() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("verification.json");
        let backup = path.with_extension("json.bak");
        let primary_items: Vec<_> = (0..40)
            .map(|i| item(&format!("primary-{i}"), i, "failed"))
            .collect();
        let primary = format!(
            r#"{{"items":{},"history":broken}}"#,
            encode(&primary_items).unwrap()
        );
        fs::write(&path, &primary).unwrap();
        let old_backup = serde_json::to_vec(&serde_json::json!({
            "items":[item("backup-account", 120, "failed")],
            "history":[batch("backup-batch", 123)],
        }))
        .unwrap();
        fs::write(&backup, &old_backup).unwrap();

        let store = VerificationStore::new(path.clone());
        assert!(!store.migrate_batch().unwrap());
        assert!(progress(&store.open().unwrap()).unwrap().cursor.offset > 0);
        store
            .upsert_result(&item("primary-0", 200, "success"))
            .unwrap();
        finish(&store);
        let results = store.load_results(false).unwrap().records;
        assert_eq!(results.len(), 2);
        assert!(results
            .iter()
            .any(|item| item.account_id == "primary-0" && item.status == "success"));
        assert!(results
            .iter()
            .any(|item| item.account_id == "backup-account"));
        assert_eq!(store.load_history(false).unwrap().records[0].batch_id, "backup-batch");
        assert_eq!(fs::read_to_string(path).unwrap(), primary);
        assert_eq!(fs::read(backup).unwrap(), old_backup);
    }

    #[test]
    fn wakeup_verification_does_not_advance_on_write_contention_or_oversized_legacy_record() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("verification.json");
        let mut rows: Vec<_> = (0..32)
            .map(|i| item(&format!("account-{i}"), i, "success"))
            .collect();
        let mut large = item("large", 40, "failed");
        large.last_message = Some("x".repeat(maintenance::BATCH_BYTES as usize));
        rows.push(large);
        fs::write(
            &path,
            encode(&serde_json::json!({ "items": rows })).unwrap(),
        )
        .unwrap();

        let store = VerificationStore::new(path);
        let mut other = store.open().unwrap();
        let lock = other
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert!(store.migrate_batch().is_err());
        assert!(progress(&lock).unwrap().cursor.offset == 0);
        lock.rollback().unwrap();

        assert!(!store.migrate_batch().unwrap());
        let checkpoint = progress(&store.open().unwrap()).unwrap();
        assert!(checkpoint.cursor.offset > 0);
        assert!(store.migrate_batch().unwrap_err().contains("超出迁移预算"));
        assert!(progress(&store.open().unwrap()).unwrap() == checkpoint);
    }
}
