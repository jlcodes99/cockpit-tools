//! Indexed local history with resumable import from immutable legacy arrays.
use std::{
    collections::HashSet,
    fs::File,
    io::{BufRead, BufReader, ErrorKind, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{de::DeserializeOwned, Serialize};

const BATCH_ROWS: usize = 32;
const BATCH_BYTES: u64 = 2 * 1024 * 1024;
const BATCH_TIME: Duration = Duration::from_millis(25);

const DATABASE: &str = "wakeup_history.sqlite";
const MIGRATION_VERSION: u32 = 2;

pub(crate) trait HistoryRow: Serialize + DeserializeOwned + Send + 'static {
    fn id(&self) -> &str;
    fn timestamp(&self) -> i64;
}

type BatchRunner = fn(&HistoryStore) -> Result<bool, String>;
type Scheduler = fn(HistoryStore, BatchRunner);

#[derive(Clone)]
pub(crate) struct HistoryStore {
    source: PathBuf,
    scope: &'static str,
    database: &'static str,
    limit: Option<usize>,
    legacy_first: bool,
    scheduler: Option<Scheduler>,
    fallback: Option<Fallback>,
}

#[derive(Clone)]
struct Fallback {
    source: PathBuf,
    scope: &'static str,
    cutoff: fn() -> i64,
}

impl HistoryStore {
    pub(crate) fn new(source: PathBuf, scope: &'static str, limit: usize) -> Self {
        Self {
            source,
            scope,
            database: DATABASE,
            limit: Some(limit),
            legacy_first: false,
            scheduler: None,
            fallback: None,
        }
    }

    pub(crate) fn daily_checkin(source: PathBuf, scope: &'static str) -> Self {
        Self {
            source,
            scope,
            database: "checkin_history.sqlite",
            limit: None,
            legacy_first: false,
            scheduler: None,
            fallback: None,
        }
    }

    pub(crate) fn switches(source: PathBuf, scope: &'static str, limit: usize) -> Self {
        Self {
            source,
            scope,
            database: "switch_history.sqlite",
            limit: Some(limit),
            legacy_first: true,
            scheduler: None,
            fallback: None,
        }
    }

    pub(crate) fn with_scheduler(mut self, scheduler: Scheduler) -> Self {
        self.scheduler = Some(scheduler);
        self
    }

    pub(crate) fn with_fallback(
        mut self,
        source: PathBuf,
        scope: &'static str,
        cutoff: fn() -> i64,
    ) -> Self {
        self.fallback = Some(Fallback {
            source,
            scope,
            cutoff,
        });
        self
    }

    pub(crate) fn migration_key(&self) -> String {
        format!("local-history:{}:{}", self.scope, self.source.display())
    }

    fn order(&self) -> &'static str {
        if self.legacy_first {
            // The previous switch log kept older records first for equal timestamps.
            "timestamp DESC, legacy_offset IS NULL, legacy_offset, sequence ASC"
        } else {
            "timestamp DESC, legacy_offset, sequence DESC"
        }
    }

    fn open(&self) -> Result<Connection, String> {
        let db = self.source.with_file_name(self.database);
        let mut conn = Connection::open(db).map_err(sql_error)?;
        conn.busy_timeout(Duration::from_millis(100))
            .map_err(sql_error)?;
        let version: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(sql_error)?;
        if version > MIGRATION_VERSION {
            return Err("本地历史数据库版本不受支持".into());
        }
        if version == 0 {
            conn.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE IF NOT EXISTS history (
                    sequence INTEGER PRIMARY KEY,
                    scope TEXT NOT NULL,
                    id TEXT NOT NULL,
                    timestamp INTEGER NOT NULL,
                    legacy_offset INTEGER,
                    payload TEXT NOT NULL,
                    UNIQUE(scope,id)
                 );
                 CREATE INDEX IF NOT EXISTS history_order ON history(scope,timestamp DESC,legacy_offset,sequence DESC);
                 CREATE TABLE IF NOT EXISTS migration (
                    scope TEXT PRIMARY KEY,
                    source_stamp TEXT,
                    offset INTEGER NOT NULL DEFAULT 0,
                    complete INTEGER NOT NULL DEFAULT 0,
                    prune_before INTEGER
                 );
                 PRAGMA user_version=2;
                 COMMIT;"
            ).map_err(sql_error)?;
        } else if version == 1 {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(sql_error)?;
            // Another reader may have opened and upgraded the same database.
            let upgraded: u32 = tx
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .map_err(sql_error)?;
            if upgraded == 1 {
                tx.execute_batch(
                    "ALTER TABLE migration ADD COLUMN prune_before INTEGER; PRAGMA user_version=2;",
                )
                .map_err(sql_error)?;
            }
            tx.commit().map_err(sql_error)?;
        }
        Ok(conn)
    }

    fn migration(&self, conn: &Connection) -> Result<Migration, String> {
        Self::migration_for(conn, self.scope)
    }

    fn migration_for(conn: &Connection, scope: &str) -> Result<Migration, String> {
        conn.query_row(
            "SELECT source_stamp,offset,complete,prune_before FROM migration WHERE scope=?1",
            [scope],
            |row| {
                Ok(Migration {
                    source_stamp: row.get(0)?,
                    offset: row.get(1)?,
                    complete: row.get(2)?,
                    prune_before: row.get(3)?,
                })
            },
        )
        .optional()
        .map(|state| state.unwrap_or_default())
        .map_err(sql_error)
    }

    fn save_migration(&self, tx: &Transaction<'_>, state: &Migration) -> Result<(), String> {
        Self::save_migration_for(tx, self.scope, state)
    }

    fn save_migration_for(
        tx: &Transaction<'_>,
        scope: &str,
        state: &Migration,
    ) -> Result<(), String> {
        tx.execute(
            "INSERT INTO migration(scope,source_stamp,offset,complete,prune_before) VALUES (?1,?2,?3,?4,?5)
             ON CONFLICT(scope) DO UPDATE SET source_stamp=excluded.source_stamp,offset=excluded.offset,
               complete=excluded.complete,prune_before=excluded.prune_before",
            params![scope, state.source_stamp, state.offset, state.complete, state.prune_before],
        ).map_err(sql_error)?;
        Ok(())
    }

    fn store<T: HistoryRow>(
        &self,
        tx: &Transaction<'_>,
        row: &T,
        offset: Option<u64>,
    ) -> Result<(), String> {
        let payload =
            serde_json::to_string(row).map_err(|error| format!("序列化历史失败: {error}"))?;
        tx.execute(
            "INSERT INTO history(scope,id,timestamp,legacy_offset,payload) VALUES (?1,?2,?3,?4,?5)
             ON CONFLICT(scope,id) DO NOTHING",
            params![self.scope, row.id(), row.timestamp(), offset, payload],
        )
        .map_err(sql_error)?;
        Ok(())
    }

    fn trim(&self, tx: &Transaction<'_>) -> Result<(), String> {
        let Some(limit) = self.limit else {
            return Ok(());
        };
        tx.execute(
            &format!(
                "DELETE FROM history WHERE scope=?1 AND sequence IN (
                 SELECT sequence FROM history WHERE scope=?1 ORDER BY {} LIMIT -1 OFFSET ?2
             )",
                self.order()
            ),
            params![self.scope, limit],
        )
        .map_err(sql_error)?;
        Ok(())
    }

    pub(crate) fn schedule<T: HistoryRow>(&self) {
        if let Some(schedule) = self.scheduler {
            schedule(self.clone(), Self::migrate_sources::<T>);
        }
    }

    fn migrate_sources<T: HistoryRow>(&self) -> Result<bool, String> {
        if !self.migrate_batch::<T>()? {
            return Ok(false);
        }
        self.migrate_fallback::<T>()
    }

    fn migrate_batch<T: HistoryRow>(&self) -> Result<bool, String> {
        let mut conn = self.open()?;
        let state = self.migration(&conn)?;
        if state.complete {
            return Ok(true);
        }
        let source = open_source(&self.source)?;
        let (stamp, batch) = match source {
            Some((mut source, stamp)) => {
                let offset = state.resume_offset(&stamp)?;
                let batch = read_batch::<T>(&mut source, offset)?;
                (Some(stamp), batch)
            }
            None if state.offset != 0 => return Err("旧历史文件在迁移期间不可用".into()),
            None => (
                None,
                Batch {
                    rows: Vec::new(),
                    offset: 0,
                    complete: true,
                },
            ),
        };
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        if self.migration(&tx)? != state {
            return Ok(false);
        }
        for (offset, row) in &batch.rows {
            if state
                .prune_before
                .is_none_or(|cutoff| row.timestamp() >= cutoff)
            {
                self.store(&tx, row, Some(*offset))?;
            }
        }
        self.trim(&tx)?;
        self.save_migration(
            &tx,
            &Migration {
                source_stamp: stamp,
                offset: batch.offset,
                complete: batch.complete,
                prune_before: state.prune_before,
            },
        )?;
        tx.commit().map_err(sql_error)?;
        Ok(batch.complete)
    }

    fn migrate_fallback<T: HistoryRow>(&self) -> Result<bool, String> {
        let Some(fallback) = &self.fallback else {
            return Ok(true);
        };
        let mut conn = self.open()?;
        let before = Self::migration_for(&conn, fallback.scope)?;
        if before.complete {
            return Ok(true);
        }
        let Some((mut source, stamp)) = open_source(&fallback.source)? else {
            return if before.offset == 0 {
                // The WebView can hand off its snapshot after native migration.
                Ok(true)
            } else {
                Err("旧 WebView 历史在迁移期间不可用".into())
            };
        };
        let batch = read_batch::<T>(&mut source, before.resume_offset(&stamp)?)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        if Self::migration_for(&tx, fallback.scope)? != before {
            return Ok(false);
        }
        let cutoff = self
            .migration(&tx)?
            .prune_before
            .unwrap_or(i64::MIN)
            .max((fallback.cutoff)());
        for (_, row) in &batch.rows {
            if fallback_in_retention(row, cutoff) {
                self.store(&tx, row, None)?;
            }
        }
        Self::save_migration_for(
            &tx,
            fallback.scope,
            &Migration {
                source_stamp: Some(stamp),
                offset: batch.offset,
                complete: batch.complete,
                prune_before: None,
            },
        )?;
        tx.commit().map_err(sql_error)?;
        Ok(batch.complete)
    }

    /// Read only committed rows. Interactive callers can show these while the
    /// maintenance worker imports the remainder of the legacy source.
    pub(crate) fn load_available<T: HistoryRow>(&self) -> Result<(Vec<T>, bool), String> {
        let mut conn = self.open()?;
        let tx = conn.transaction().map_err(sql_error)?;
        let state = self.migration(&tx)?;
        let fallback = self
            .fallback
            .as_ref()
            .map(|source| Self::migration_for(&tx, source.scope).map(|state| (source, state)))
            .transpose()?;
        let rows = self.read_rows(&tx)?;
        tx.commit().map_err(sql_error)?;
        let pending = state.needs_import(&self.source)
            || fallback.is_some_and(|(source, progress)| progress.needs_import(&source.source));
        Ok((rows.into_iter().map(|(_, row)| row).collect(), pending))
    }

    fn read_rows<T: HistoryRow>(&self, conn: &Connection) -> Result<Vec<(Option<u64>, T)>, String> {
        let mut query = conn
            .prepare(&format!(
                "SELECT legacy_offset,payload FROM history WHERE scope=?1 ORDER BY {} LIMIT ?2",
                self.order()
            ))
            .map_err(sql_error)?;
        let mut result = query
            .query(params![
                self.scope,
                self.limit.map(|limit| limit as i64).unwrap_or(-1)
            ])
            .map_err(sql_error)?;
        let mut rows = Vec::new();
        while let Some(row) = result.next().map_err(sql_error)? {
            let offset = row.get(0).map_err(sql_error)?;
            let payload: String = row.get(1).map_err(sql_error)?;
            rows.push((
                offset,
                serde_json::from_str(&payload).map_err(|_| "历史记录损坏".to_string())?,
            ));
        }
        Ok(rows)
    }

    pub(crate) fn load<T: HistoryRow>(&self) -> Result<Vec<T>, String> {
        let mut conn = self.open()?;
        let tx = conn.transaction().map_err(sql_error)?;
        let state = self.migration(&tx)?;
        let fallback = self
            .fallback
            .as_ref()
            .map(|fallback| Self::migration_for(&tx, fallback.scope).map(|state| (fallback, state)))
            .transpose()?;
        let mut rows = self.read_rows::<T>(&tx)?;
        tx.commit().map_err(sql_error)?;
        if !state.complete || fallback.as_ref().is_some_and(|(_, state)| !state.complete) {
            self.schedule::<T>();
        }
        if !state.complete {
            if let Some((mut source, stamp)) = open_source(&self.source)? {
                let mut offset = state.resume_offset(&stamp)?;
                let mut ids: HashSet<String> = rows.iter().map(|(_, row)| row.id().to_string()).collect();
                loop {
                    let batch = read_batch::<T>(&mut source, offset)?;
                    for (position, row) in batch.rows {
                        if state
                            .prune_before
                            .is_none_or(|cutoff| row.timestamp() >= cutoff)
                            && ids.insert(row.id().to_string())
                        {
                            rows.push((Some(position), row));
                        }
                    }
                    if let Some(limit) = self.limit {
                        self.sort_merged(&mut rows);
                        for (_, row) in rows.drain(limit.min(rows.len())..) {
                            ids.remove(row.id());
                        }
                    }
                    if batch.complete {
                        break;
                    }
                    offset = batch.offset;
                }
                if self.limit.is_none() {
                    self.sort_merged(&mut rows);
                }
            } else if state.offset != 0 {
                return Err("旧历史文件在迁移期间不可用".into());
            }
        }
        let mut rows: Vec<_> = rows.into_iter().map(|(_, row)| row).collect();
        if let Some((fallback, progress)) = fallback {
            if !progress.complete {
                if let Some((mut source, stamp)) = open_source(&fallback.source)? {
                    let cutoff = state
                        .prune_before
                        .unwrap_or(i64::MIN)
                        .max((fallback.cutoff)());
                    let mut ids: HashSet<String> =
                        rows.iter().map(|row| row.id().to_string()).collect();
                    let mut offset = progress.resume_offset(&stamp)?;
                    loop {
                        let batch = read_batch::<T>(&mut source, offset)?;
                        rows.extend(batch.rows.into_iter().map(|(_, row)| row).filter(|row| {
                            fallback_in_retention(row, cutoff) && ids.insert(row.id().to_string())
                        }));
                        if batch.complete {
                            break;
                        }
                        offset = batch.offset;
                    }
                    rows.sort_by_key(|row| std::cmp::Reverse(row.timestamp()));
                } else if progress.offset != 0 {
                    return Err("旧 WebView 历史在迁移期间不可用".into());
                }
            }
        }
        Ok(rows)
    }

    fn sort_merged<T: HistoryRow>(&self, rows: &mut [(Option<u64>, T)]) {
        rows.sort_by_key(|(offset, row)| {
            let order = if self.legacy_first {
                offset.unwrap_or(u64::MAX)
            } else {
                offset.unwrap_or_default()
            };
            (std::cmp::Reverse(row.timestamp()), order)
        });
    }

    pub(crate) fn add<T: HistoryRow>(&self, rows: &[T]) -> Result<(), String> {
        if rows.is_empty() {
            return Ok(());
        }
        let mut conn = self.open()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        let pending = !self.migration(&tx)?.complete;
        for row in rows.iter().rev() {
            self.store(&tx, row, None)?;
        }
        self.trim(&tx)?;
        tx.commit().map_err(sql_error)?;
        if pending {
            self.schedule::<T>();
        }
        Ok(())
    }

    pub(crate) fn replace_item<T: HistoryRow>(&self, row: T) -> Result<(), String> {
        let mut conn = self.open()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        let pending = !self.migration(&tx)?.complete;
        tx.execute(
            "DELETE FROM history WHERE scope=?1 AND id=?2",
            params![self.scope, row.id()],
        )
        .map_err(sql_error)?;
        self.store(&tx, &row, None)?;
        self.trim(&tx)?;
        tx.commit().map_err(sql_error)?;
        if pending {
            self.schedule::<T>();
        }
        Ok(())
    }

    /// Merge a platform's daily row without rewriting its other days. If the
    /// old day has not reached the importer, read that day before taking a write lock.
    pub(crate) fn merge_day<T: HistoryRow>(
        &self,
        incoming: T,
        cutoff: i64,
        merge: impl FnOnce(&mut T, T),
    ) -> Result<(), String> {
        let mut conn = self.open()?;
        let before = self.migration(&conn)?;
        let mut old = None;
        let mut fallback = None;
        if self.read_day::<T>(&conn, incoming.id())?.is_none() {
            if !before.complete {
                old = Self::read_legacy_day::<T>(&self.source, incoming.id(), &before)?;
            }
            if old.is_none() {
                if let Some(source) = &self.fallback {
                    let progress = Self::migration_for(&conn, source.scope)?;
                    if !progress.complete {
                        fallback =
                            Self::read_legacy_day::<T>(&source.source, incoming.id(), &progress)?
                                .filter(|row| fallback_in_retention(row, (source.cutoff)()));
                    }
                }
            }
        }

        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        let mut state = self.migration(&tx)?;
        let fallback_pending = self
            .fallback
            .as_ref()
            .map(|source| Self::migration_for(&tx, source.scope).map(|state| !state.complete))
            .transpose()?
            .unwrap_or(false);
        let old = old
            .filter(|_| !state.complete)
            .or(fallback.filter(|_| fallback_pending));
        let existing = self.read_day(&tx, incoming.id())?.or_else(|| {
            old.filter(|row| {
                state
                    .prune_before
                    .is_none_or(|cutoff| row.timestamp() >= cutoff)
            })
        });
        let row = if let Some(mut existing) = existing {
            merge(&mut existing, incoming);
            existing
        } else {
            incoming
        };
        let payload =
            serde_json::to_string(&row).map_err(|error| format!("序列化签到历史失败: {error}"))?;
        tx.execute(
            "INSERT INTO history(scope,id,timestamp,payload) VALUES (?1,?2,?3,?4)
             ON CONFLICT(scope,id) DO UPDATE SET
                timestamp=excluded.timestamp,legacy_offset=NULL,payload=excluded.payload",
            params![self.scope, row.id(), row.timestamp(), payload],
        )
        .map_err(sql_error)?;
        let prune_before = state.prune_before.map_or(cutoff, |last| cutoff.max(last));
        tx.execute(
            "DELETE FROM history WHERE scope=?1 AND timestamp<?2",
            params![self.scope, prune_before],
        )
        .map_err(sql_error)?;
        state.prune_before = Some(prune_before);
        self.save_migration(&tx, &state)?;
        tx.commit().map_err(sql_error)?;
        if !state.complete || fallback_pending {
            self.schedule::<T>();
        }
        Ok(())
    }

    fn read_day<T: HistoryRow>(&self, conn: &Connection, day: &str) -> Result<Option<T>, String> {
        let payload: Option<String> = conn
            .query_row(
                "SELECT payload FROM history WHERE scope=?1 AND id=?2",
                params![self.scope, day],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        payload
            .map(|payload| {
                serde_json::from_str(&payload).map_err(|_| "签到历史记录损坏".to_string())
            })
            .transpose()
    }

    fn read_legacy_day<T: HistoryRow>(
        path: &Path,
        day: &str,
        state: &Migration,
    ) -> Result<Option<T>, String> {
        let Some((mut source, stamp)) = open_source(path)? else {
            return if state.offset == 0 {
                Ok(None)
            } else {
                Err("旧签到历史在迁移期间不可用".into())
            };
        };
        let mut offset = state.resume_offset(&stamp)?;
        loop {
            let batch = read_batch::<T>(&mut source, offset)?;
            if let Some((_, row)) = batch.rows.into_iter().find(|(_, row)| row.id() == day) {
                return Ok(state
                    .prune_before
                    .is_none_or(|cutoff| row.timestamp() >= cutoff)
                    .then_some(row));
            }
            if batch.complete {
                return Ok(None);
            }
            offset = batch.offset;
        }
    }

    pub(crate) fn clear(&self) -> Result<(), String> {
        let mut conn = self.open()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        tx.execute("DELETE FROM history WHERE scope=?1", [self.scope])
            .map_err(sql_error)?;
        self.save_migration(
            &tx,
            &Migration {
                complete: true,
                ..Migration::default()
            },
        )?;
        if let Some(fallback) = &self.fallback {
            Self::save_migration_for(
                &tx,
                fallback.scope,
                &Migration {
                    complete: true,
                    ..Migration::default()
                },
            )?;
        }
        tx.commit().map_err(sql_error)
    }
}

fn fallback_in_retention(row: &impl HistoryRow, cutoff: i64) -> bool {
    let timestamp = row.timestamp();
    timestamp != i64::MAX && timestamp >= cutoff
}

#[derive(Default, PartialEq)]
struct Migration {
    source_stamp: Option<String>,
    offset: u64,
    complete: bool,
    prune_before: Option<i64>,
}

impl Migration {
    fn needs_import(&self, source: &Path) -> bool {
        // Let the worker report source access errors without hiding committed rows.
        !self.complete && (self.offset != 0 || source.try_exists().unwrap_or(true))
    }

    fn resume_offset(&self, stamp: &str) -> Result<u64, String> {
        if self.source_stamp.as_deref().is_some_and(|saved| saved != stamp) {
            return Err("旧历史文件在迁移期间发生变化，已保留原有数据".into());
        }
        Ok(self.offset)
    }
}

struct Batch<T> {
    rows: Vec<(u64, T)>,
    offset: u64,
    complete: bool,
}

fn open_source(path: &Path) -> Result<Option<(BufReader<File>, String)>, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("读取旧历史失败: {error}")),
    };
    let metadata = file
        .metadata()
        .map_err(|error| format!("读取旧历史失败: {error}"))?;
    let modified = metadata
        .modified()
        .map_err(|error| format!("读取旧历史失败: {error}"))?;
    let stamp = format!("{}:{modified:?}", metadata.len());
    Ok(Some((BufReader::new(file), stamp)))
}

fn read_batch<T: HistoryRow>(
    source: &mut BufReader<File>,
    offset: u64,
) -> Result<Batch<T>, String> {
    let invalid = || "旧历史格式无效，原文件已保留".to_string();
    source
        .seek(SeekFrom::Start(offset))
        .map_err(|error| error.to_string())?;
    let started = Instant::now();
    let mut rows = Vec::new();
    let mut next_offset = offset;
    let mut complete = false;
    loop {
        let delimiter = next_byte(source)?;
        match (next_offset == 0, delimiter) {
            (false, Some(b']')) => {
                if peek_byte(source)?.is_some() {
                    return Err(invalid());
                }
                complete = true;
                break;
            }
            (true, Some(b'[')) => {
                if peek_byte(source)? == Some(b']') {
                    next_byte(source)?;
                    if peek_byte(source)?.is_some() {
                        return Err(invalid());
                    }
                    complete = true;
                    break;
                }
            }
            (false, Some(b',')) => {}
            _ => return Err(invalid()),
        }

        let position = source
            .stream_position()
            .map_err(|error| error.to_string())?;
        let available = BATCH_BYTES.saturating_sub(position.saturating_sub(offset));
        if available == 0 && !rows.is_empty() {
            break;
        }
        let mut items =
            serde_json::Deserializer::from_reader(source.by_ref().take(available)).into_iter::<T>();
        match items.next() {
            Some(Ok(row)) => rows.push((position, row)),
            Some(Err(error)) if error.is_eof() && !rows.is_empty() => break,
            _ => return Err(invalid()),
        }
        next_offset = source
            .stream_position()
            .map_err(|error| error.to_string())?;
        if rows.len() >= BATCH_ROWS
            || next_offset.saturating_sub(offset) >= BATCH_BYTES
            || started.elapsed() >= BATCH_TIME
        {
            break;
        }
    }
    Ok(Batch {
        rows,
        offset: next_offset,
        complete,
    })
}

fn peek_byte(source: &mut BufReader<File>) -> Result<Option<u8>, String> {
    loop {
        let bytes = source.fill_buf().map_err(|error| error.to_string())?;
        let whitespace = bytes
            .iter()
            .take_while(|byte| byte.is_ascii_whitespace())
            .count();
        if whitespace < bytes.len() {
            let value = bytes[whitespace];
            source.consume(whitespace);
            return Ok(Some(value));
        }
        if bytes.is_empty() {
            return Ok(None);
        }
        source.consume(whitespace);
    }
}

fn next_byte(source: &mut BufReader<File>) -> Result<Option<u8>, String> {
    let value = peek_byte(source)?;
    if value.is_some() {
        source.consume(1);
    }
    Ok(value)
}

pub(crate) fn checkin_timestamp(timestamp: &str) -> i64 {
    use chrono::{Local, NaiveDateTime, TimeZone};
    let Ok(naive) = NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%d %H:%M:%S") else {
        return i64::MAX;
    };
    Local
        .from_local_datetime(&naive)
        .single()
        .map_or_else(|| naive.and_utc().timestamp(), |time| time.timestamp())
}

fn sql_error(error: rusqlite::Error) -> String {
    format!("读写本地历史数据库失败: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct Item {
        id: String,
        timestamp: i64,
    }

    impl HistoryRow for Item {
        fn id(&self) -> &str {
            &self.id
        }

        fn timestamp(&self) -> i64 {
            self.timestamp
        }
    }

    fn rows(count: usize) -> Vec<Item> {
        (0..count)
            .map(|index| Item {
                id: format!("legacy-{index}"),
                timestamp: index as i64,
            })
            .collect()
    }

    #[test]
    fn resumes_by_byte_offset_and_keeps_new_history_and_legacy_file() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("legacy.json");
        let legacy = serde_json::to_vec_pretty(&rows(75)).unwrap();
        std::fs::write(&source, &legacy).unwrap();
        let store = HistoryStore::new(source.clone(), "fixture", 100);
        assert!(!store.migrate_batch::<Item>().unwrap());
        let offset = store.migration(&store.open().unwrap()).unwrap().offset;
        assert!(offset > 0);

        let live = Item {
            id: "live".into(),
            timestamp: 100,
        };
        store.add(std::slice::from_ref(&live)).unwrap();
        assert_eq!(store.load::<Item>().unwrap().len(), 76);

        let restarted = HistoryStore::new(source.clone(), "fixture", 100);
        while !restarted.migrate_batch::<Item>().unwrap() {}
        let all = restarted.load::<Item>().unwrap();
        assert_eq!(all.len(), 76);
        assert_eq!(all[0], live);
        assert_eq!(std::fs::read(source).unwrap(), legacy);
    }

    #[test]
    fn partial_history_uses_copied_rows_and_rejects_a_replaced_source() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("history.json");
        let store = HistoryStore::daily_checkin(source.clone(), "fixture");
        let original = rows(75);
        std::fs::write(&source, serde_json::to_vec(&original).unwrap()).unwrap();
        let (before_copy, pending) = store.load_available::<Item>().unwrap();
        assert!(before_copy.is_empty() && pending);
        assert!(!store.migrate_batch::<Item>().unwrap());
        let before = store.migration(&store.open().unwrap()).unwrap();
        let actual = store.load::<Item>().unwrap();
        assert_eq!(actual, original.into_iter().rev().collect::<Vec<_>>());

        std::fs::write(&source, "[]").unwrap();
        let (available, pending) = store.load_available::<Item>().unwrap();
        assert!(pending);
        assert!(!available.is_empty() && available.len() <= BATCH_ROWS);
        assert!(store
            .load::<Item>()
            .unwrap_err()
            .contains("迁移期间发生变化"));
        assert!(store
            .migrate_batch::<Item>()
            .unwrap_err()
            .contains("迁移期间发生变化"));
        assert!(store.migration(&store.open().unwrap()).unwrap() == before);
        assert_eq!(std::fs::read(&source).unwrap(), b"[]");
        store.clear().unwrap();
        assert_eq!(store.load_available::<Item>().unwrap(), (Vec::new(), false));
    }

    #[test]
    fn clearing_mid_import_stops_old_history_from_reappearing() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("legacy.json");
        std::fs::write(&source, serde_json::to_vec(&rows(40)).unwrap()).unwrap();
        let store = HistoryStore::new(source.clone(), "fixture", 100);
        assert!(!store.migrate_batch::<Item>().unwrap());
        store.clear().unwrap();
        assert!(store.migrate_batch::<Item>().unwrap());
        assert!(store.load::<Item>().unwrap().is_empty());
        assert!(source.exists());
    }

    #[test]
    fn checkin_merges_unmigrated_day_without_reviving_pruned_rows() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("legacy.json");
        let mut old = rows(40);
        old.push(Item {
            id: "today".into(),
            timestamp: 50,
        });
        std::fs::write(&source, serde_json::to_vec(&old).unwrap()).unwrap();
        let store = HistoryStore::daily_checkin(source, "fixture");
        assert!(!store.migrate_batch::<Item>().unwrap());
        store
            .merge_day(
                Item {
                    id: "today".into(),
                    timestamp: 60,
                },
                38,
                |existing, incoming| existing.timestamp += incoming.timestamp,
            )
            .unwrap();
        store
            .merge_day(
                Item {
                    id: "legacy-20".into(),
                    timestamp: 60,
                },
                38,
                |_, _| unreachable!("expired records do not merge"),
            )
            .unwrap();
        while !store.migrate_batch::<Item>().unwrap() {}
        let rows = store.load::<Item>().unwrap();
        assert_eq!(
            rows.iter()
                .map(|row| (row.id.as_str(), row.timestamp))
                .collect::<Vec<_>>(),
            vec![
                ("today", 110),
                ("legacy-20", 60),
                ("legacy-39", 39),
                ("legacy-38", 38)
            ]
        );
        store.clear().unwrap();
        assert!(store.migrate_batch::<Item>().unwrap());
        assert!(store.load::<Item>().unwrap().is_empty());
    }

    #[test]
    fn checkin_shows_live_updates_to_days_already_copied_before_cutover() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("legacy.json");
        std::fs::write(&source, serde_json::to_vec(&rows(40)).unwrap()).unwrap();
        let store = HistoryStore::daily_checkin(source, "fixture");
        assert!(!store.migrate_batch::<Item>().unwrap());
        store
            .merge_day(
                Item {
                    id: "legacy-0".into(),
                    timestamp: 50,
                },
                0,
                |existing, incoming| existing.timestamp += incoming.timestamp,
            )
            .unwrap();
        assert_eq!(store.load::<Item>().unwrap()[0].timestamp, 50);
    }

    #[test]
    fn checkin_fallback_fills_missing_days_without_replacing_native_or_live_results() {
        let dir = tempfile::tempdir().unwrap();
        let primary = dir.path().join("native.json");
        let fallback = dir.path().join("webview.json");
        let item = |id: &str, timestamp| Item {
            id: id.into(),
            timestamp,
        };
        std::fs::write(
            &primary,
            serde_json::to_vec(&[item("native", 120)]).unwrap(),
        )
        .unwrap();
        std::fs::write(
            &fallback,
            serde_json::to_vec(&[
                item("native", 100),
                item("today", 100),
                item("fallback", 90),
                item("expired", 20),
            ])
            .unwrap(),
        )
        .unwrap();
        let store = HistoryStore::daily_checkin(primary, "fixture").with_fallback(
            fallback.clone(),
            "fixture-webview",
            || 50,
        );
        let before = store.load::<Item>().unwrap();
        assert_eq!(
            before,
            vec![
                item("native", 120),
                item("today", 100),
                item("fallback", 90)
            ]
        );
        store
            .merge_day(item("today", 30), 70, |existing, incoming| {
                existing.timestamp += incoming.timestamp;
            })
            .unwrap();
        while !store.migrate_sources::<Item>().unwrap() {}
        assert_eq!(
            store.load::<Item>().unwrap(),
            vec![
                item("today", 130),
                item("native", 120),
                item("fallback", 90),
            ]
        );
        assert!(fallback.exists());
    }

    #[test]
    fn checkin_clear_before_webview_handoff_does_not_revive_old_days() {
        let dir = tempfile::tempdir().unwrap();
        let primary = dir.path().join("native.json");
        let fallback = dir.path().join("webview.json");
        let store = HistoryStore::daily_checkin(primary, "fixture").with_fallback(
            fallback.clone(),
            "fixture-webview",
            || 0,
        );
        assert!(store.migrate_sources::<Item>().unwrap());
        store.clear().unwrap();
        std::fs::write(&fallback, serde_json::to_vec(&rows(45)).unwrap()).unwrap();
        assert!(store.migrate_sources::<Item>().unwrap());
        assert!(store.load::<Item>().unwrap().is_empty());
        assert!(fallback.exists());
    }

    #[test]
    fn switch_history_keeps_legacy_order_and_shares_new_entries_across_connections() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("legacy.json");
        let mut old = rows(40);
        old[0].timestamp = 100;
        old[1].timestamp = 100;
        let legacy = serde_json::to_vec(&old).unwrap();
        std::fs::write(&source, &legacy).unwrap();
        let host = HistoryStore::switches(source.clone(), "fixture", 100);
        let cli = HistoryStore::switches(source.clone(), "fixture", 100);
        assert!(!host.migrate_batch::<Item>().unwrap());
        cli.replace_item(Item {
            id: "live".into(),
            timestamp: 100,
        })
        .unwrap();
        cli.replace_item(Item {
            id: "legacy-38".into(),
            timestamp: 101,
        })
        .unwrap();
        let before = host.load::<Item>().unwrap();
        assert_eq!(before.len(), 41);
        assert_eq!(
            before
                .iter()
                .take(4)
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            vec!["legacy-38", "legacy-0", "legacy-1", "live"]
        );
        while !host.migrate_batch::<Item>().unwrap() {}
        assert_eq!(cli.load::<Item>().unwrap(), before);
        assert_eq!(std::fs::read(source).unwrap(), legacy);
    }
}
