//! Shared desktop and CLI account summaries. Credential files keep their own storage.
use super::legacy_json_input::{Input, ReadError};
use crate::models::account::{AccountIndex, AccountSummary};
use fs2::FileExt;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, ErrorKind},
    marker::PhantomData,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(crate) const LEGACY_FILE: &str = "accounts.json";
pub(super) const BATCH_ROWS: usize = 32;
pub(super) const BATCH_BYTES: u64 = 2 * 1024 * 1024;
pub(super) const BATCH_TIME: Duration = Duration::from_millis(25);
const LOCK_WAIT: Duration = Duration::from_millis(300);

pub(crate) type AccountIndexStore = IndexStore<AccountIndex>;

pub(crate) trait IndexDocument: serde::de::DeserializeOwned {
    const LEGACY_FILE: &'static str;
    const DATABASE: &'static str;
    const KEY: &'static str;
    const MIN_DETAIL_SCHEMA: u32 = 0;
    const CHECK_REVISION: bool = false;
    const IMPORT_EMPTY: bool = false;
    const RECOVER_DETAILS: bool = false;
    const DEFAULT_LEGACY_VERSION: Option<&'static str> = None;

    fn from_storage(state: State, rows: Vec<StoredSummary>) -> Result<Self, String>;
    fn storage_state(&self) -> State;
    fn storage_rows(&self) -> impl Iterator<Item = Result<StoredSummary, String>>;
    fn mark_legacy(&mut self) {}
    fn mark_saved(&self, _revision: Option<u64>) {}
    fn read_legacy_summary(input: &mut Input) -> Result<StoredSummary, ReadError> {
        input.value()
    }
}

#[derive(Clone)]
pub(crate) struct IndexStore<T> {
    root: PathBuf,
    old_root: Option<PathBuf>,
    legacy_override: Option<PathBuf>,
    document: PhantomData<T>,
}

pub(crate) enum IndexRead<T = AccountIndex> {
    Active(T),
    Legacy(T),
    Missing,
    InvalidLegacy(String),
}

#[derive(Default, Deserialize)]
pub(crate) struct StoredSummary {
    pub id: String,
    pub email: String,
    pub name: Option<String>,
    pub plan_type: Option<String>,
    pub subscription_active_until: Option<String>,
    pub provider_login: Option<String>,
    pub provider_user_id: Option<String>,
    pub payload: Option<String>,
    pub created_at: i64,
    pub last_used: i64,
}

fn summary_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredSummary> {
    Ok(StoredSummary {
        id: row.get(0)?,
        email: row.get(1)?,
        name: row.get(2)?,
        created_at: row.get(3)?,
        last_used: row.get(4)?,
        plan_type: row.get(5)?,
        subscription_active_until: row.get(6)?,
        provider_login: row.get(7)?,
        provider_user_id: row.get(8)?,
        payload: row.get(9)?,
    })
}

impl IndexDocument for AccountIndex {
    const LEGACY_FILE: &'static str = LEGACY_FILE;
    const DATABASE: &'static str = "antigravity_accounts.sqlite";
    const KEY: &'static str = "antigravity-account-index";

    fn from_storage(state: State, rows: Vec<StoredSummary>) -> Result<Self, String> {
        Ok(Self {
            version: state.version.ok_or("账号索引缺少版本")?,
            current_account_id: state.current_id,
            accounts: rows
                .into_iter()
                .map(|row| AccountSummary {
                    id: row.id,
                    email: row.email,
                    name: row.name,
                    created_at: row.created_at,
                    last_used: row.last_used,
                })
                .collect(),
        })
    }

    fn storage_state(&self) -> State {
        State {
            version: Some(self.version.clone()),
            current_id: self.current_account_id.clone(),
            ..State::default()
        }
    }

    fn storage_rows(&self) -> impl Iterator<Item = Result<StoredSummary, String>> {
        self.accounts.iter().map(|row| {
            Ok(StoredSummary {
                id: row.id.clone(),
                email: row.email.clone(),
                name: row.name.clone(),
                plan_type: None,
                subscription_active_until: None,
                created_at: row.created_at,
                last_used: row.last_used,
                ..StoredSummary::default()
            })
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct State {
    source: u8,
    stamp: Option<String>,
    offset: u64,
    position: usize,
    pub version: Option<String>,
    pub current_id: Option<String>,
    #[serde(default)]
    pub detail_schema: Option<u32>,
    #[serde(default)]
    pub revision: Option<u64>,
    #[serde(default)]
    pub identity_revision: u64,
    #[serde(default)]
    pub identity_maintenance_version: u32,
    #[serde(default)]
    pub detail_recovery_complete: bool,
    #[serde(default)]
    pub detail_recovery_requires_account: bool,
    in_accounts: bool,
    next_item: bool,
    saw_accounts: bool,
    #[serde(default)]
    saw_current_id: bool,
    complete: bool,
}

impl State {
    pub(crate) fn for_document(
        version: String,
        current_id: Option<String>,
        detail_schema: u32,
        revision: Option<u64>,
    ) -> Self {
        Self {
            version: Some(version),
            current_id,
            detail_schema: Some(detail_schema),
            revision,
            ..Self::default()
        }
    }
}

impl<T: IndexDocument> IndexStore<T> {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            root,
            old_root: None,
            legacy_override: None,
            document: PhantomData,
        }
    }

    pub(crate) fn with_old_root(mut self, path: PathBuf) -> Self {
        if self.root != path {
            self.old_root = Some(path);
        }
        self
    }

    pub(crate) fn with_legacy_path(mut self, path: &Path) -> Self {
        if path.file_name().is_some_and(|name| name != T::LEGACY_FILE) {
            self.legacy_override = Some(path.to_path_buf());
        }
        self
    }

    pub(crate) fn legacy_path(&self) -> PathBuf {
        self.legacy_override
            .clone()
            .unwrap_or_else(|| self.root.join(T::LEGACY_FILE))
    }

    pub(crate) fn database_path(&self) -> PathBuf {
        match &self.legacy_override {
            Some(path) => path.with_extension("sqlite"),
            None => self.root.join(T::DATABASE),
        }
    }

    pub(crate) fn has_legacy_source(&self) -> Result<bool, String> {
        for source in [1, 2] {
            if self.source_path(source).try_exists().map_err(error)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(crate) fn active_path(&self) -> Result<PathBuf, String> {
        let path = self.database_path();
        if path.try_exists().map_err(error)? && read_state(&self.open()?)?.complete {
            return Ok(path);
        }
        Ok(self.legacy_path())
    }

    pub(crate) fn migration_key(&self) -> String {
        format!("{}:{}", T::KEY, self.legacy_path().display())
    }

    pub(crate) fn lock_writes(&self) -> Result<File, String> {
        self.lock_writes_with_wait(LOCK_WAIT)?
            .ok_or_else(|| "账号索引正在由另一个进程更新，请重试".into())
    }

    pub(crate) fn try_lock_writes(&self) -> Result<Option<File>, String> {
        self.lock_writes_with_wait(Duration::ZERO)
    }

    fn lock_writes_with_wait(&self, wait: Duration) -> Result<Option<File>, String> {
        let lock_path = if T::LEGACY_FILE == LEGACY_FILE {
            self.root.join("antigravity_account_index.lock")
        } else {
            self.database_path().with_extension("lock")
        };
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)
            .map_err(error)?;
        let started = Instant::now();
        loop {
            match FileExt::try_lock_exclusive(&file) {
                Ok(()) => return Ok(Some(file)),
                Err(cause)
                    if cause.kind() == ErrorKind::WouldBlock
                        || cause.raw_os_error() == fs2::lock_contended_error().raw_os_error() =>
                {
                    if started.elapsed() >= wait {
                        return Ok(None);
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(cause) => return Err(error(cause)),
            }
        }
    }

    fn source_path(&self, source: u8) -> PathBuf {
        let primary = match source {
            3 | 4 => self
                .old_root
                .as_deref()
                .unwrap_or(&self.root)
                .join(T::LEGACY_FILE),
            _ => self.legacy_path(),
        };
        if source == 2 || source == 4 {
            primary.with_extension("json.bak")
        } else {
            primary
        }
    }

    fn alternate_source(&self, source: u8) -> Result<Option<u8>, String> {
        let last = if self.old_root.is_some() { 4 } else { 2 };
        for candidate in source + 1..=last {
            if self.source_path(candidate).try_exists().map_err(error)? {
                return Ok(Some(candidate));
            }
        }
        Ok(None)
    }

    fn open(&self) -> Result<Connection, String> {
        let path = self.database_path();
        #[cfg(unix)]
        if !path.try_exists().map_err(error)? {
            use std::os::unix::fs::OpenOptionsExt;
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
            {
                Ok(_) => {}
                Err(cause) if cause.kind() == ErrorKind::AlreadyExists => {}
                Err(cause) => return Err(error(cause)),
            }
        }
        let mut conn = Connection::open(path).map_err(error)?;
        conn.busy_timeout(Duration::from_millis(100))
            .map_err(error)?;
        let version: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(error)?;
        match version {
            4 => {}
            0..=3 => {
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(error)?;
                let version: u32 = tx
                    .pragma_query_value(None, "user_version", |row| row.get(0))
                    .map_err(error)?;
                if version == 0 {
                    tx.execute_batch(
                        "CREATE TABLE summaries (
                            source INTEGER NOT NULL, position INTEGER NOT NULL,
                            account_id TEXT NOT NULL, email TEXT NOT NULL, name TEXT,
                            created_at INTEGER NOT NULL, last_used INTEGER NOT NULL,
                            plan_type TEXT, subscription_active_until TEXT,
                            provider_login TEXT, provider_user_id TEXT, payload TEXT,
                            PRIMARY KEY(source,position)
                         );
                         CREATE INDEX summary_by_id ON summaries(source,account_id);
                         CREATE TABLE account_index_state (
                            singleton INTEGER PRIMARY KEY CHECK(singleton=1),
                            state TEXT NOT NULL
                         );",
                    )
                    .map_err(error)?;
                } else if version == 1 {
                    tx.execute_batch(
                        "ALTER TABLE summaries ADD COLUMN plan_type TEXT;
                         ALTER TABLE summaries ADD COLUMN subscription_active_until TEXT;",
                    )
                    .map_err(error)?;
                }
                if version == 1 || version == 2 {
                    tx.execute_batch(
                        "ALTER TABLE summaries ADD COLUMN provider_login TEXT;
                         ALTER TABLE summaries ADD COLUMN provider_user_id TEXT;
                         ALTER TABLE summaries ADD COLUMN payload TEXT;",
                    )
                    .map_err(error)?;
                }
                if version < 3 {
                    if T::RECOVER_DETAILS {
                        tx.execute_batch(
                            "CREATE TABLE IF NOT EXISTS recovery_removals (account_id TEXT PRIMARY KEY);",
                        ).map_err(error)?;
                    }
                    tx.execute_batch(
                        "CREATE INDEX summary_by_email ON summaries(source,email);
                         CREATE INDEX summary_by_provider_login ON summaries(source,provider_login);
                         CREATE INDEX summary_by_provider_user ON summaries(source,provider_user_id);
                         PRAGMA user_version=3;",
                    ).map_err(error)?;
                }
                if version < 4 {
                    tx.execute_batch(
                        "CREATE INDEX summary_missing_identity ON summaries(source,position)
                         WHERE payload IS NOT NULL
                           AND json_extract(payload,'$.query_identity.version') IS NOT 1;
                         PRAGMA user_version=4;",
                    )
                    .map_err(error)?;
                }
                tx.commit().map_err(error)?;
            }
            _ => return Err("账号索引数据库版本不受支持".into()),
        }
        Ok(conn)
    }

    pub(crate) fn load(&self) -> Result<IndexRead<T>, String> {
        // Reads before cutover also work when the data directory cannot create
        // a database. Once a database exists, errors must not reveal stale JSON.
        if !self.database_path().try_exists().map_err(error)? {
            return self.read_legacy(&State::default());
        }
        let mut conn = self.open()?;
        let tx = conn.transaction().map_err(error)?;
        let state = read_state(&tx)?;
        if state.complete {
            let accounts = {
                let mut query = tx
                    .prepare(
                        "SELECT account_id,email,name,created_at,last_used,plan_type,subscription_active_until,
                            provider_login,provider_user_id,payload
                     FROM summaries WHERE source=?1 ORDER BY position",
                    )
                    .map_err(error)?;
                let rows = query
                    .query_map([state.source], summary_row)
                    .map_err(error)?;
                rows.collect::<Result<Vec<_>, _>>().map_err(error)?
            };
            tx.commit().map_err(error)?;
            return T::from_storage(state, accounts).map(IndexRead::Active);
        }
        tx.commit().map_err(error)?;
        self.read_legacy(&state)
    }

    pub(crate) fn active_ids_by_provider_user(
        &self,
        user_id: &str,
    ) -> Result<Option<Vec<String>>, String> {
        if !self.database_path().try_exists().map_err(error)? {
            return Ok(None);
        }
        let mut conn = self.open()?;
        let tx = conn.transaction().map_err(error)?;
        let state = read_state(&tx)?;
        if !state.complete {
            return Ok(None);
        }
        let mut query = tx
            .prepare("SELECT account_id FROM summaries WHERE source=?1 AND provider_user_id=?2")
            .map_err(error)?;
        let rows = query
            .query_map(params![state.source, user_id], |row| row.get(0))
            .map_err(error)?;
        rows.collect::<Result<Vec<_>, _>>().map(Some).map_err(error)
    }

    pub(crate) fn active_revision(&self) -> Result<Option<u64>, String> {
        if !self.database_path().try_exists().map_err(error)? {
            return Ok(None);
        }
        let state = read_state(&self.open()?)?;
        Ok(if state.complete { state.revision } else { None })
    }

    pub(crate) fn missing_identity_batch(
        &self,
        after: i64,
    ) -> Result<Option<(u64, u32, Vec<(i64, String)>)>, String> {
        if !self.database_path().try_exists().map_err(error)? {
            return Ok(None);
        }
        let mut conn = self.open()?;
        let tx = conn.transaction().map_err(error)?;
        let state = read_state(&tx)?;
        let Some(revision) = state.revision.filter(|_| state.complete) else {
            return Ok(None);
        };
        let mut query = tx
            .prepare(
                "SELECT position,account_id FROM summaries
             WHERE source=?1 AND payload IS NOT NULL
               AND json_extract(payload,'$.query_identity.version') IS NOT 1
             AND position>?2 ORDER BY position LIMIT ?3",
            )
            .map_err(error)?;
        let rows = query
            .query_map(params![state.source, after, BATCH_ROWS], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .map_err(error)?;
        let ids = rows.collect::<Result<Vec<_>, _>>().map_err(error)?;
        Ok(Some((revision, state.identity_maintenance_version, ids)))
    }

    pub(crate) fn commit_identity_batch(
        &self,
        revision: u64,
        rows: &[StoredSummary],
    ) -> Result<bool, String> {
        let mut conn = self.open()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(error)?;
        let mut state = read_state(&tx)?;
        if !state.complete || state.revision != Some(revision) {
            return Ok(false);
        }
        {
            let mut position = tx
                .prepare(
                    "SELECT position FROM summaries WHERE source=?1 AND account_id=?2
                   AND json_extract(payload,'$.query_identity.version') IS NOT 1",
                )
                .map_err(error)?;
            for row in rows {
                let result = position
                    .query_row(params![state.source, row.id], |row| row.get(0))
                    .optional()
                    .map_err(error)?;
                if let Some(position) = result {
                    put_summary(&tx, state.source, position, row)?;
                }
            }
        }
        // Newly restored identities can reveal duplicates that the old pass
        // could not see, even when the legacy metadata claimed it was complete.
        state.identity_maintenance_version = 0;
        state.revision = Some(revision.checked_add(1).ok_or("账号索引版本超出范围")?);
        write_state(&tx, &state)?;
        tx.commit().map_err(error)?;
        Ok(true)
    }

    pub(crate) fn active_current(&self) -> Result<Option<Option<String>>, String> {
        let state = read_state(&self.open()?)?;
        Ok(state.complete.then_some(state.current_id))
    }

    pub(crate) fn replace(&self, index: &T) -> Result<(), String> {
        self.replace_with_removals(index, &[])
    }

    pub(crate) fn replace_with_removals(
        &self,
        index: &T,
        removed: &[String],
    ) -> Result<(), String> {
        let mut conn = self.open()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(error)?;
        let before = read_state(&tx)?;
        let mut state = index.storage_state();
        if T::CHECK_REVISION {
            let freshly_empty = T::RECOVER_DETAILS
                && before.revision == Some(0)
                && before.source == 0
                && before.position == 0;
            if before.complete && !freshly_empty && state.revision != before.revision {
                return Err("账号索引已更新，请重试".into());
            }
            state.revision = Some(
                before
                    .revision
                    .unwrap_or(0)
                    .checked_add(1)
                    .ok_or("账号索引版本超出范围")?,
            );
        }
        state.detail_recovery_complete = before.detail_recovery_complete;
        state.detail_recovery_requires_account = before.detail_recovery_requires_account;
        if T::RECOVER_DETAILS && !state.detail_recovery_complete {
            let mut query = tx
                .prepare("INSERT OR IGNORE INTO recovery_removals(account_id) VALUES(?1)")
                .map_err(error)?;
            for id in removed {
                query.execute([id]).map_err(error)?;
            }
        }
        tx.execute("DELETE FROM summaries WHERE source<>0", [])
            .map_err(error)?;
        let mut count = 0;
        for summary in index.storage_rows() {
            put_summary(&tx, 0, count, &summary?)?;
            count += 1;
        }
        tx.execute(
            "DELETE FROM summaries WHERE source=0 AND position>=?1",
            [count],
        )
        .map_err(error)?;
        state.complete = true;
        write_state(&tx, &state)?;
        tx.commit().map_err(error)?;
        index.mark_saved(state.revision);
        Ok(())
    }

    pub(crate) fn detail_recovery_complete(&self) -> Result<bool, String> {
        if !self.database_path().try_exists().map_err(error)? {
            return Ok(false);
        }
        let state = read_state(&self.open()?)?;
        Ok(state.complete && state.detail_recovery_complete)
    }

    pub(crate) fn missing_recovery_ids(&self, ids: &[String]) -> Result<Vec<String>, String> {
        let mut conn = self.open()?;
        let tx = conn.transaction().map_err(error)?;
        let state = read_state(&tx)?;
        if !state.complete || state.detail_recovery_complete {
            return Ok(Vec::new());
        }
        let mut query = tx.prepare(RECOVERABLE_ID).map_err(error)?;
        let mut missing = Vec::new();
        for id in ids {
            if query
                .query_row(params![state.source, id], |row| row.get(0))
                .map_err(error)?
            {
                missing.push(id.clone());
            }
        }
        Ok(missing)
    }

    pub(crate) fn recover_details_batch(
        &self,
        rows: &[StoredSummary],
        finished: bool,
    ) -> Result<(), String> {
        let mut conn = self.open()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(error)?;
        let mut state = read_state(&tx)?;
        if !state.complete || state.detail_recovery_complete {
            return Ok(());
        }
        let mut position: usize = tx
            .query_row(
                "SELECT COALESCE(MAX(position)+1,0) FROM summaries WHERE source=?1",
                [state.source],
                |row| row.get(0),
            )
            .map_err(error)?;
        let start = position;
        {
            let mut query = tx.prepare(RECOVERABLE_ID).map_err(error)?;
            for summary in rows {
                if query
                    .query_row(params![state.source, summary.id], |row| row.get(0))
                    .map_err(error)?
                {
                    put_summary(&tx, state.source, position, summary)?;
                    position += 1;
                }
            }
        }
        if finished && state.detail_recovery_requires_account && position == 0 {
            let removed: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM recovery_removals)",
                    [],
                    |row| row.get(0),
                )
                .map_err(error)?;
            if !removed {
                return Err("旧 Cursor 索引损坏，未找到可恢复的账号详情".into());
            }
        }
        if finished || start != position {
            state.detail_recovery_complete = finished;
            if finished {
                state.detail_recovery_requires_account = false;
            }
            state.revision = Some(
                state
                    .revision
                    .unwrap_or(0)
                    .checked_add(1)
                    .ok_or("账号索引版本超出范围")?,
            );
            write_state(&tx, &state)?;
            if finished {
                tx.execute("DELETE FROM recovery_removals", [])
                    .map_err(error)?;
            }
        }
        tx.commit().map_err(error)
    }

    /// The caller holds the cross-process write lock. Before cutover the
    /// normal snapshot writer is used so the old reference cannot win later.
    pub(crate) fn set_active_current(&self, id: &str) -> Result<bool, String> {
        let mut conn = self.open()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(error)?;
        let mut state = read_state(&tx)?;
        if !state.complete {
            return Ok(false);
        }
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM summaries WHERE source=?1 AND account_id=?2)",
                params![state.source, id],
                |row| row.get(0),
            )
            .map_err(error)?;
        if !exists {
            return Err(format!("找不到账号 ID: {id}"));
        }
        state.current_id = Some(id.to_string());
        write_state(&tx, &state)?;
        tx.commit().map_err(error)?;
        Ok(true)
    }

    fn read_legacy(&self, state: &State) -> Result<IndexRead<T>, String> {
        let first = state.source.max(1);
        let last = if self.old_root.is_some() { 4 } else { 2 };
        let mut invalid = None;
        for source in first..=last {
            match read_document::<T>(&self.source_path(source)) {
                Ok(Some(mut index)) => {
                    index.mark_legacy();
                    return Ok(IndexRead::Legacy(index));
                }
                Ok(None) => {}
                Err(LegacyFailure::Format(cause)) => {
                    invalid.get_or_insert(cause);
                }
                Err(LegacyFailure::Io(cause)) => return Err(error(cause)),
            }
        }
        if let Some(cause) = invalid {
            Ok(IndexRead::InvalidLegacy(cause))
        } else if state.stamp.is_some() || first > 1 {
            Err("迁移期间旧账号索引不可用".into())
        } else {
            Ok(IndexRead::Missing)
        }
    }

    pub(crate) fn migrate_batch(&self) -> Result<bool, String> {
        self.migrate_with_detail_fallback(false)
    }

    pub(crate) fn migrate_with_detail_fallback(&self, has_details: bool) -> Result<bool, String> {
        let mut conn = self.open()?;
        let before = read_state(&conn)?;
        if before.complete {
            return Ok(true);
        }
        let mut next = before.clone();
        if next.source == 0 {
            next.source = 1;
        }
        let mut rows = Vec::new();
        match File::open(self.source_path(next.source)) {
            Ok(file) => {
                let metadata = file.metadata().map_err(error)?;
                let stamp = format!(
                    "{}:{:?}",
                    metadata.len(),
                    metadata.modified().map_err(error)?
                );
                if next.stamp.as_ref().is_some_and(|saved| saved != &stamp) {
                    return Err("迁移期间旧账号索引发生变化".into());
                }
                next.stamp = Some(stamp);
                let mut input = Input::new(file, next.offset, BATCH_BYTES).map_err(error)?;
                match read_batch::<T>(&mut input, &mut next) {
                    Ok(batch) => {
                        rows = batch;
                        if next.complete && !next.saw_accounts {
                            if let Some(source) = self.alternate_source(next.source)? {
                                next = State {
                                    source,
                                    ..State::default()
                                };
                            }
                        }
                    }
                    Err(ReadError::Format) => match self.alternate_source(next.source)? {
                        Some(source) => {
                            next = State {
                                source,
                                ..State::default()
                            }
                        }
                        None if T::RECOVER_DETAILS && has_details => {
                            next = empty_recovery_state::<T>();
                            next.detail_recovery_requires_account = true;
                        }
                        None => return Err(legacy_error(ReadError::Format)),
                    },
                    Err(cause) => return Err(legacy_error(cause)),
                }
            }
            Err(cause) if cause.kind() == ErrorKind::NotFound && next.stamp.is_none() => {
                match self.alternate_source(next.source)? {
                    Some(source) => {
                        next = State {
                            source,
                            ..State::default()
                        }
                    }
                    None if T::RECOVER_DETAILS => next = empty_recovery_state::<T>(),
                    None => return Ok(true),
                }
            }
            Err(cause) => return Err(error(cause)),
        }
        if T::RECOVER_DETAILS && next.complete && !next.saw_accounts {
            next = empty_recovery_state::<T>();
        }
        // Empty and older legacy indexes retain the existing repair-from-details behavior.
        if next.complete
            && ((!next.saw_accounts || (!T::IMPORT_EMPTY && next.position + rows.len() == 0))
                || next.detail_schema.unwrap_or(0) < T::MIN_DETAIL_SCHEMA)
        {
            return Ok(true);
        }
        if next.complete && T::CHECK_REVISION {
            next.revision = Some(0);
        }

        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(error)?;
        if read_state(&tx)? != before {
            return Ok(false);
        }
        if T::RECOVER_DETAILS && next.complete && next.source == 0 {
            tx.execute("DELETE FROM summaries", []).map_err(error)?;
        }
        for summary in rows {
            put_summary(&tx, next.source, next.position, &summary)?;
            next.position += 1;
        }
        write_state(&tx, &next)?;
        tx.commit().map_err(error)?;
        Ok(next.complete)
    }
}

const RECOVERABLE_ID: &str =
    "SELECT NOT EXISTS(SELECT 1 FROM summaries WHERE source=?1 AND account_id=?2)
     AND NOT EXISTS(SELECT 1 FROM recovery_removals WHERE account_id=?2)";

fn empty_recovery_state<T: IndexDocument>() -> State {
    State {
        complete: true,
        saw_accounts: true,
        version: T::DEFAULT_LEGACY_VERSION.map(str::to_owned),
        ..State::default()
    }
}

enum LegacyFailure {
    Format(String),
    Io(std::io::Error),
}

fn read_document<T: IndexDocument>(path: &Path) -> Result<Option<T>, LegacyFailure> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(cause) if cause.kind() == ErrorKind::NotFound => return Ok(None),
        Err(cause) => return Err(LegacyFailure::Io(cause)),
    };
    let mut input = BufReader::new(file);
    loop {
        let buffer = input.fill_buf().map_err(LegacyFailure::Io)?;
        if buffer.is_empty() {
            return Ok(None);
        }
        let spaces = buffer
            .iter()
            .take_while(|byte| byte.is_ascii_whitespace())
            .count();
        let has_content = spaces < buffer.len();
        input.consume(spaces);
        if has_content {
            break;
        }
    }
    serde_json::from_reader(input).map(Some).map_err(|cause| {
        if cause.is_io() {
            LegacyFailure::Io(std::io::Error::new(
                cause.io_error_kind().unwrap_or(ErrorKind::Other),
                cause,
            ))
        } else {
            LegacyFailure::Format(cause.to_string())
        }
    })
}

fn read_state(conn: &Connection) -> Result<State, String> {
    let value: Option<String> = conn
        .query_row(
            "SELECT state FROM account_index_state WHERE singleton=1",
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

fn write_state(conn: &Connection, state: &State) -> Result<(), String> {
    conn.execute(
        "INSERT INTO account_index_state(singleton,state) VALUES(1,?1)
         ON CONFLICT(singleton) DO UPDATE SET state=excluded.state",
        [serde_json::to_string(state).map_err(error)?],
    )
    .map_err(error)?;
    Ok(())
}

fn put_summary(
    conn: &Connection,
    source: u8,
    position: usize,
    summary: &StoredSummary,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO summaries(source,position,account_id,email,name,created_at,last_used,plan_type,subscription_active_until,provider_login,provider_user_id,payload)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
         ON CONFLICT(source,position) DO UPDATE SET
            account_id=excluded.account_id,email=excluded.email,name=excluded.name,
            created_at=excluded.created_at,last_used=excluded.last_used,plan_type=excluded.plan_type,
            subscription_active_until=excluded.subscription_active_until,
            provider_login=excluded.provider_login,provider_user_id=excluded.provider_user_id,payload=excluded.payload
         WHERE summaries.account_id IS NOT excluded.account_id OR summaries.email IS NOT excluded.email
            OR summaries.name IS NOT excluded.name OR summaries.created_at IS NOT excluded.created_at
            OR summaries.last_used IS NOT excluded.last_used OR summaries.plan_type IS NOT excluded.plan_type
            OR summaries.subscription_active_until IS NOT excluded.subscription_active_until
            OR summaries.provider_login IS NOT excluded.provider_login
            OR summaries.provider_user_id IS NOT excluded.provider_user_id OR summaries.payload IS NOT excluded.payload",
        params![
            source,
            position,
            summary.id,
            summary.email,
            summary.name,
            summary.created_at,
            summary.last_used,
            summary.plan_type,
            summary.subscription_active_until,
            summary.provider_login,
            summary.provider_user_id,
            summary.payload
        ],
    )
    .map_err(error)?;
    Ok(())
}

fn read_batch<T: IndexDocument>(
    input: &mut Input,
    state: &mut State,
) -> Result<Vec<StoredSummary>, ReadError> {
    let start = state.offset;
    let started = Instant::now();
    let mut rows = Vec::new();
    if start == 0 {
        if input.peek()?.is_none() {
            state.complete = true;
            return Ok(rows);
        }
        input.expect(b'{')?;
    }
    loop {
        match read_item::<T>(input, state) {
            Ok(Some(summary)) => rows.push(summary),
            Ok(None) => {}
            Err(ReadError::Budget) if state.offset > start => break,
            Err(cause) => return Err(cause),
        }
        state.offset = input.offset;
        if state.complete
            || rows.len() >= BATCH_ROWS
            || input.budget == 0
            || started.elapsed() >= BATCH_TIME
        {
            break;
        }
    }
    Ok(rows)
}

fn read_item<T: IndexDocument>(
    input: &mut Input,
    state: &mut State,
) -> Result<Option<StoredSummary>, ReadError> {
    if input.peek()? == Some(if state.in_accounts { b']' } else { b'}' }) {
        input.take()?;
        if state.in_accounts {
            state.in_accounts = false;
            state.next_item = true;
        } else {
            if state.version.is_none() {
                state.version = T::DEFAULT_LEGACY_VERSION.map(str::to_owned);
            }
            if input.peek()?.is_some() || !state.saw_accounts || state.version.is_none() {
                return Err(ReadError::Format);
            }
            state.complete = true;
        }
        return Ok(None);
    }
    if state.next_item {
        input.expect(b',')?;
    }
    if state.in_accounts {
        let row = T::read_legacy_summary(input)?;
        state.next_item = true;
        return Ok(Some(row));
    }
    let key = input.value::<String>()?;
    input.expect(b':')?;
    match key.as_str() {
        "accounts" if !state.saw_accounts => {
            input.expect(b'[')?;
            state.saw_accounts = true;
            state.in_accounts = true;
            state.next_item = false;
            return Ok(None);
        }
        "accounts" => return Err(ReadError::Format),
        "version" if state.version.is_none() => state.version = Some(input.value()?),
        "current_account_id" if !state.saw_current_id => {
            state.current_id = input.value()?;
            state.saw_current_id = true;
        }
        "detail_schema_version" if state.detail_schema.is_none() => {
            state.detail_schema = Some(input.value()?)
        }
        "identity_revision" => state.identity_revision = input.value()?,
        "identity_maintenance_version" => state.identity_maintenance_version = input.value()?,
        "version" | "current_account_id" | "detail_schema_version" => {
            return Err(ReadError::Format)
        }
        _ => {
            input.value::<serde::de::IgnoredAny>()?;
        }
    }
    state.next_item = true;
    Ok(None)
}

fn error(cause: impl std::fmt::Display) -> String {
    format!("读写账号索引失败: {cause}")
}
fn legacy_error(cause: ReadError) -> String {
    match cause {
        ReadError::Budget => "旧账号索引单项超出迁移预算，原文件已保留".into(),
        ReadError::Format => "旧账号索引格式无效，原文件已保留".into(),
        ReadError::Io(cause) => error(cause),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture(count: usize) -> AccountIndex {
        AccountIndex {
            version: "2.0".into(),
            current_account_id: Some(format!("legacy-{}", count - 1)),
            accounts: (0..count)
                .map(|i| AccountSummary {
                    id: format!("legacy-{i}"),
                    email: format!("account-{i}@example.test"),
                    name: Some(format!("Account {i}")),
                    created_at: i as i64,
                    last_used: i as i64 * 2,
                })
                .collect(),
        }
    }

    fn active(store: &AccountIndexStore) -> AccountIndex {
        match store.load().unwrap() {
            IndexRead::Active(index) => index,
            _ => panic!("index should be active"),
        }
    }

    fn finish(store: &AccountIndexStore) {
        for _ in 0..20 {
            if store.migrate_batch().unwrap() {
                return;
            }
        }
        panic!("fixture import did not finish");
    }

    #[test]
    fn antigravity_index_resume_preserves_account_order_and_reference() {
        let dir = tempfile::tempdir().unwrap();
        let first = AccountIndexStore::new(dir.path().into());
        let old = fixture(75);
        let bytes = serde_json::to_vec_pretty(&old).unwrap();
        fs::write(first.legacy_path(), &bytes).unwrap();
        assert!(!first.migrate_batch().unwrap());
        let checkpoint = read_state(&first.open().unwrap()).unwrap();
        assert!(checkpoint.offset > 0);
        assert!(checkpoint.position < 75);

        let reopened = AccountIndexStore::new(dir.path().into());
        assert_eq!(read_state(&reopened.open().unwrap()).unwrap(), checkpoint);
        finish(&reopened);
        assert_eq!(
            serde_json::to_vec(&active(&reopened)).unwrap(),
            serde_json::to_vec(&old).unwrap()
        );
        assert_eq!(
            reopened.active_current().unwrap(),
            Some(Some("legacy-74".into()))
        );
        assert_eq!(fs::read(first.legacy_path()).unwrap(), bytes);
        fs::write(first.legacy_path(), "unreadable after cutover").unwrap();
        assert_eq!(active(&reopened).accounts.len(), 75);
    }

    #[test]
    fn antigravity_index_mutation_during_import_keeps_deleted_ids_retired() {
        let dir = tempfile::tempdir().unwrap();
        let first = AccountIndexStore::new(dir.path().into());
        fs::write(
            first.legacy_path(),
            serde_json::to_vec(&fixture(75)).unwrap(),
        )
        .unwrap();
        assert!(!first.migrate_batch().unwrap());
        let _guard = first.lock_writes().unwrap();
        let IndexRead::Legacy(mut edited) = first.load().unwrap() else {
            panic!("legacy snapshot expected")
        };
        edited.accounts.retain(|account| account.id != "legacy-74");
        edited.current_account_id = Some("legacy-1".into());
        first.replace(&edited).unwrap();
        assert!(first.migrate_batch().unwrap());
        let cli = AccountIndexStore::new(dir.path().into());
        assert_eq!(active(&cli).accounts.len(), 74);
        assert!(active(&cli)
            .accounts
            .iter()
            .all(|row| row.id != "legacy-74"));
        assert_eq!(cli.active_current().unwrap(), Some(Some("legacy-1".into())));
        assert!(cli.set_active_current("legacy-74").is_err());
        assert!(cli.set_active_current("legacy-2").unwrap());
        assert_eq!(
            first.active_current().unwrap(),
            Some(Some("legacy-2".into()))
        );
    }

    #[test]
    fn antigravity_index_reads_backup_without_overwriting_blank_primary() {
        let dir = tempfile::tempdir().unwrap();
        let store = AccountIndexStore::new(dir.path().into());
        fs::write(store.legacy_path(), "  \n").unwrap();
        fs::write(
            store.source_path(2),
            serde_json::to_vec(&fixture(2)).unwrap(),
        )
        .unwrap();
        assert!(matches!(store.load().unwrap(), IndexRead::Legacy(_)));
        finish(&store);
        assert_eq!(active(&store).accounts.len(), 2);
        assert_eq!(fs::read_to_string(store.legacy_path()).unwrap(), "  \n");
    }

    #[test]
    fn antigravity_index_os_lock_serializes_separate_processes() {
        const CHILD_ROOT: &str = "COCKPIT_ACCOUNT_INDEX_LOCK_TEST_ROOT";
        if let Some(root) = std::env::var_os(CHILD_ROOT) {
            let root = PathBuf::from(root);
            let store = AccountIndexStore::new(root.clone());
            fs::write(root.join("child-ready"), "").unwrap();
            let _lock = store.lock_writes().unwrap();
            let index = active(&store);
            assert!(index
                .accounts
                .iter()
                .all(|account| account.id != "legacy-1"));
            assert!(store.set_active_current("legacy-0").unwrap());
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let store = AccountIndexStore::new(dir.path().into());
        let lock = store.lock_writes().unwrap();
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "modules::antigravity_account_index_store::tests::antigravity_index_os_lock_serializes_separate_processes"])
            .env(CHILD_ROOT, dir.path())
            .spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !dir.path().join("child-ready").exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(dir.path().join("child-ready").exists());
        let mut index = fixture(2);
        index.accounts.pop();
        index.current_account_id = None;
        store.replace(&index).unwrap();
        drop(lock);
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        assert_eq!(
            store.active_current().unwrap(),
            Some(Some("legacy-0".into()))
        );
    }
}
