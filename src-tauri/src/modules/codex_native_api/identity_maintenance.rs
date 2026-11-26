//! Bounded, non-blocking storage maintenance and legacy archive slimming for `identity.sqlite`.
//!
//! Active continuation transcripts live exclusively in the self-contained,
//! strictly bounded hot tables `native_rebuild_checkpoints_v1` and
//! `native_partial_history_v1`. Legacy unbounded chunk tables
//! (`native_rebuild_history_v1`, `native_rebuild_items_v1`, `native_archive_*`)
//! are automatically salvaged and removed on open or compaction.
use super::*;
use std::{
    path::Path,
    sync::TryLockError,
    time::Instant,
};

pub(crate) const ARCHIVE_RETENTION_SECS: i64 = 7 * 24 * 60 * 60;
pub(crate) const ACTIVE_CONVERSATION_GRACE_SECS: i64 = 60 * 60;
pub(crate) const MAX_HISTORY_PER_THREAD: usize = 16;
pub(crate) const MAX_PARTIAL_HISTORY_ROWS: usize = 512;
pub(crate) const MAX_ARCHIVE_DB_BYTES: i64 = 64 * 1024 * 1024;
pub(crate) const TARGET_ARCHIVE_DB_BYTES: i64 = 48 * 1024 * 1024;
pub(crate) const VACUUM_FREELIST_THRESHOLD_BYTES: i64 = 16 * 1024 * 1024;
pub(crate) const PRUNE_BATCH_SIZE: usize = 128;
pub(crate) const ITEM_GC_BATCH_SIZE: usize = 512;
const SLIM_REBUILD_THRESHOLD_BYTES: i64 = 16 * 1024 * 1024;
const LEGACY_SALVAGE_LIMIT: usize = 64;

#[derive(Clone, Copy, Debug)]
pub struct MaintenancePolicy {
    pub retention_secs: i64,
    pub max_history_per_thread: usize,
    pub max_partial_rows: usize,
    pub max_archive_bytes: i64,
    pub target_archive_bytes: i64,
    pub vacuum_freelist_threshold_bytes: i64,
    pub batch_size: usize,
    pub item_gc_batch_size: usize,
    /// Cooperative SQLite budget while holding the shared connection.
    pub max_batch_duration: Duration,
}

impl Default for MaintenancePolicy {
    fn default() -> Self {
        Self {
            retention_secs: ARCHIVE_RETENTION_SECS,
            max_history_per_thread: MAX_HISTORY_PER_THREAD,
            max_partial_rows: MAX_PARTIAL_HISTORY_ROWS,
            max_archive_bytes: MAX_ARCHIVE_DB_BYTES,
            target_archive_bytes: TARGET_ARCHIVE_DB_BYTES,
            vacuum_freelist_threshold_bytes: VACUUM_FREELIST_THRESHOLD_BYTES,
            batch_size: PRUNE_BATCH_SIZE,
            item_gc_batch_size: ITEM_GC_BATCH_SIZE,
            max_batch_duration: Duration::from_millis(250),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MaintenanceReport {
    pub deleted_history_rows: usize,
    pub deleted_partial_rows: usize,
    pub deleted_checkpoint_rows: usize,
    pub deleted_item_rows: usize,
    pub vacuumed: bool,
    pub has_more: bool,
}

impl MaintenanceReport {
    pub fn merge(&mut self, other: Self) {
        self.deleted_history_rows += other.deleted_history_rows;
        self.deleted_partial_rows += other.deleted_partial_rows;
        self.deleted_checkpoint_rows += other.deleted_checkpoint_rows;
        self.deleted_item_rows += other.deleted_item_rows;
        self.vacuumed |= other.vacuumed;
        self.has_more = other.has_more;
    }

    pub fn any_deleted(&self) -> bool {
        self.deleted_history_rows > 0
            || self.deleted_partial_rows > 0
            || self.deleted_checkpoint_rows > 0
            || self.deleted_item_rows > 0
    }
}

#[derive(Clone, Copy, Debug)]
struct DbSizeMetrics {
    auto_vacuum: i64,
    total_bytes: i64,
    live_bytes: i64,
    freelist_bytes: i64,
}

fn read_db_size_metrics(connection: &Connection) -> Result<DbSizeMetrics, String> {
    let page_size: i64 = connection
        .query_row("PRAGMA page_size", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    let page_count: i64 = connection
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    let freelist_count: i64 = connection
        .query_row("PRAGMA freelist_count", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    let auto_vacuum: i64 = connection
        .query_row("PRAGMA auto_vacuum", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    let live_pages = page_count.saturating_sub(freelist_count).max(0);
    Ok(DbSizeMetrics {
        auto_vacuum,
        total_bytes: page_count.max(0).saturating_mul(page_size),
        live_bytes: live_pages.saturating_mul(page_size),
        freelist_bytes: freelist_count.max(0).saturating_mul(page_size),
    })
}

pub(super) fn ensure_created_column(connection: &Connection, table: &str) -> rusqlite::Result<()> {
    let has_created = connection
        .prepare(&format!("PRAGMA table_info({table})"))?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .any(|name| name == "created");
    if !has_created {
        connection.execute(
            &format!("ALTER TABLE {table} ADD COLUMN created INTEGER NOT NULL DEFAULT 0"),
            [],
        )?;
    }
    Ok(())
}

pub(super) fn enable_incremental_auto_vacuum_if_empty(
    connection: &Connection,
) -> rusqlite::Result<()> {
    let page_count: i64 = connection.query_row("PRAGMA page_count", [], |row| row.get(0))?;
    if page_count == 0 {
        connection.execute_batch("PRAGMA auto_vacuum = INCREMENTAL;")?;
    }
    Ok(())
}

fn table_exists(connection: &Connection, name: &str) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1)",
        params![name],
        |row| row.get(0),
    )
}

fn has_legacy_archive_objects(connection: &Connection) -> rusqlite::Result<bool> {
    let has_tables: bool = connection.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM sqlite_schema
            WHERE name IN (
                'native_rebuild_history_v1',
                'native_rebuild_items_v1',
                'native_archive_job_v1',
                'native_archive_order_v1',
                'native_archive_threads_v1',
                'native_archive_candidates_v1',
                'native_archive_changed_v1'
            )
        )",
        [],
        |row| row.get(0),
    )?;
    if has_tables {
        return Ok(true);
    }
    if table_exists(connection, "native_partial_history_v1")?
        && !super::partial_history::has_column(connection, "native_partial_history_v1", "transcript")?
    {
        return Ok(true);
    }
    Ok(false)
}

fn drop_legacy_archive_objects(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "DROP TRIGGER IF EXISTS archive_INSERT_0_v1;
         DROP TRIGGER IF EXISTS archive_UPDATE_0_v1;
         DROP TRIGGER IF EXISTS archive_delete_0_v1;
         DROP TRIGGER IF EXISTS archive_INSERT_1_v1;
         DROP TRIGGER IF EXISTS archive_UPDATE_1_v1;
         DROP TRIGGER IF EXISTS archive_delete_1_v1;
         DROP TABLE IF EXISTS native_archive_job_v1;
         DROP TABLE IF EXISTS native_archive_order_v1;
         DROP TABLE IF EXISTS native_archive_threads_v1;
         DROP TABLE IF EXISTS native_archive_candidates_v1;
         DROP TABLE IF EXISTS native_archive_changed_v1;
         DROP TABLE IF EXISTS native_rebuild_history_v1;
         DROP TABLE IF EXISTS native_rebuild_items_v1;",
    )
}

fn decode_legacy_manifest_items(
    source: &Connection,
    caller: &str,
    manifest_raw: &str,
) -> Option<Vec<Value>> {
    let manifest: Vec<String> = serde_json::from_str(manifest_raw).ok()?;
    let mut result = Vec::with_capacity(manifest.len());
    let mut stmt = source
        .prepare_cached("SELECT body FROM native_rebuild_items_v1 WHERE caller=?1 AND digest=?2")
        .ok()?;
    for digest in manifest {
        let compressed: Vec<u8> = stmt
            .query_row(params![caller, digest], |row| row.get(0))
            .ok()?;
        let bytes = zstd::stream::decode_all(compressed.as_slice()).ok()?;
        if format!("{:x}", Sha256::digest(&bytes)) != digest {
            return None;
        }
        result.push(serde_json::from_slice(&bytes).ok()?);
    }
    Some(result)
}

/// Salvage the newest recent turns from legacy `native_rebuild_history_v1` and
/// legacy `native_partial_history_v1` (manifest-based) into self-contained rows
/// on `target` before the legacy tables are dropped or the database is replaced.
fn salvage_legacy_rows(source: &Connection, target: &Connection) -> rusqlite::Result<()> {
    let has_items = table_exists(source, "native_rebuild_items_v1")?;
    if !has_items {
        return Ok(());
    }
    if table_exists(source, "native_rebuild_history_v1")? {
        ensure_created_column(source, "native_rebuild_history_v1").ok();
        let mut stmt = source.prepare(
            "SELECT caller, response, thread, account, generation, manifest, parameters, created
             FROM native_rebuild_history_v1
             ORDER BY rowid DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![LEGACY_SALVAGE_LIMIT as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (caller, response, thread, account, generation, manifest, parameters, created) in rows {
            let exists: bool = target.query_row(
                "SELECT EXISTS(SELECT 1 FROM native_rebuild_checkpoints_v1 WHERE caller=?1 AND response=?2)",
                params![caller, response],
                |r| r.get(0),
            )?;
            if exists {
                continue;
            }
            let Some(items) = decode_legacy_manifest_items(source, &caller, &manifest) else {
                continue;
            };
            let parsed_params = parameters
                .as_deref()
                .and_then(|raw| serde_json::from_str::<Value>(raw).ok());
            let Ok(encoded) =
                super::rebuild::checkpoint_store::Encoded::prepare(&items, &[], parsed_params.as_ref())
            else {
                continue;
            };
            let raw_transcript = serde_json::to_string(&items).unwrap_or_default();
            let raw_bytes = raw_transcript
                .len()
                .saturating_add(parameters.as_ref().map_or(0, String::len));
            let (stored_transcript, stored_params, stored_bytes) =
                if raw_bytes <= 8 * 1024 * 1024 {
                    (raw_transcript, parameters, raw_bytes)
                } else {
                    let Ok(comp_t) =
                        super::rebuild::checkpoint_store::encode_zstd_text(&raw_transcript)
                    else {
                        continue;
                    };
                    let comp_p = match parameters {
                        Some(p) if p.len() > 64 * 1024 => {
                            super::rebuild::checkpoint_store::encode_zstd_text(&p).ok()
                        }
                        other => other,
                    };
                    let b = comp_t
                        .len()
                        .saturating_add(comp_p.as_ref().map_or(0, String::len));
                    if b > 8 * 1024 * 1024 {
                        continue;
                    }
                    (comp_t, comp_p, b)
                };
            let _ = encoded;
            target.execute(
                "INSERT OR IGNORE INTO native_rebuild_checkpoints_v1
                    (caller,response,thread,account,generation,transcript,bytes,created,parameters)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    caller,
                    response,
                    thread,
                    account,
                    generation,
                    stored_transcript,
                    stored_bytes as i64,
                    created,
                    stored_params
                ],
            )?;
        }
    }

    if table_exists(source, "native_partial_history_v1")?
        && !super::partial_history::has_column(source, "native_partial_history_v1", "transcript")?
    {
        ensure_created_column(source, "native_partial_history_v1").ok();
        let mut stmt = source.prepare(
            "SELECT caller, response, thread, session, account, owner, manifest, parameters, bytes, created
             FROM native_partial_history_v1
             ORDER BY rowid DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![LEGACY_SALVAGE_LIMIT as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if std::ptr::eq(source, target) {
            target.execute_batch("DROP TABLE IF EXISTS native_partial_history_v1;")?;
            super::partial_history::initialize(target)?;
        }
        for (caller, response, thread, session, account, owner, manifest, parameters, bytes, created) in rows {
            let Some(items) = decode_legacy_manifest_items(source, &caller, &manifest) else {
                continue;
            };
            let Ok(raw_transcript) = serde_json::to_string(&items) else {
                continue;
            };
            let stored_transcript = if raw_transcript.len() > 4096 {
                let Ok(comp) = super::rebuild::checkpoint_store::encode_zstd_text(&raw_transcript)
                else {
                    continue;
                };
                comp
            } else {
                raw_transcript
            };
            target.execute(
                "INSERT OR IGNORE INTO native_partial_history_v1
                    (caller,response,thread,session,account,owner,transcript,parameters,bytes,created)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    caller,
                    response,
                    thread,
                    session,
                    account,
                    owner,
                    stored_transcript,
                    parameters,
                    bytes,
                    created
                ],
            )?;
        }
    }
    Ok(())
}

fn initialize_clean_schema(connection: &Connection) -> rusqlite::Result<()> {
    enable_incremental_auto_vacuum_if_empty(connection)?;
    connection.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS native_identity_v1 (
            caller TEXT NOT NULL, owner TEXT NOT NULL, original TEXT NOT NULL,
            mapped TEXT NOT NULL, PRIMARY KEY(caller, owner, original));
         CREATE UNIQUE INDEX IF NOT EXISTS native_identity_v1_mapped ON native_identity_v1(mapped);
         CREATE TABLE IF NOT EXISTS native_affinity_v1 (
            caller TEXT NOT NULL, kind TEXT NOT NULL, original TEXT NOT NULL,
            account TEXT NOT NULL, owner TEXT NOT NULL,
            PRIMARY KEY(caller, kind, original));
         CREATE TABLE IF NOT EXISTS native_opaque_v1 (
            caller TEXT NOT NULL, kind TEXT NOT NULL, digest TEXT NOT NULL,
            account TEXT NOT NULL, owner TEXT NOT NULL,
            session TEXT, thread TEXT, turn TEXT, expires INTEGER NOT NULL,
            PRIMARY KEY(caller, kind, digest));
         CREATE INDEX IF NOT EXISTS native_opaque_v1_expiry ON native_opaque_v1(expires);
         CREATE INDEX IF NOT EXISTS native_opaque_v1_reference ON native_opaque_v1(kind,digest);
         CREATE TABLE IF NOT EXISTS native_recovery_commits_v1 (
            id TEXT PRIMARY KEY, caller TEXT NOT NULL, thread TEXT NOT NULL,
            account TEXT NOT NULL, owner TEXT NOT NULL, completed_at INTEGER NOT NULL);",
    )?;
    super::rebuild::initialize(connection)?;
    super::partial_history::initialize(connection)?;
    Ok(())
}

fn copy_live_state_to_slim_db(
    source_path: &Path,
    legacy_conn: &Connection,
    slim_path: &Path,
) -> rusqlite::Result<()> {
    if slim_path.exists() {
        let _ = std::fs::remove_file(slim_path);
    }
    let mut slim = Connection::open(slim_path)?;
    slim.busy_timeout(Duration::from_secs(2))?;
    initialize_clean_schema(&slim)?;

    let tx = slim.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    salvage_legacy_rows(legacy_conn, &tx)?;
    tx.commit()?;

    slim.execute(
        "ATTACH DATABASE ?1 AS legacy",
        params![source_path.to_string_lossy().as_ref()],
    )?;
    let now_secs = timestamp();
    let cutoff = now_secs.saturating_sub(ARCHIVE_RETENTION_SECS);
    let copy_result = (|| -> rusqlite::Result<()> {
        let tx = slim.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for (table, sql) in [
            (
                "native_identity_v1",
                "INSERT OR IGNORE INTO main.native_identity_v1(caller,owner,original,mapped)
                 SELECT caller,owner,original,mapped FROM legacy.native_identity_v1",
            ),
            (
                "native_affinity_v1",
                "INSERT OR IGNORE INTO main.native_affinity_v1(caller,kind,original,account,owner)
                 SELECT caller,kind,original,account,owner FROM legacy.native_affinity_v1",
            ),
            (
                "native_opaque_v1",
                "INSERT OR IGNORE INTO main.native_opaque_v1(caller,kind,digest,account,owner,session,thread,turn,expires)
                 SELECT caller,kind,digest,account,owner,session,thread,turn,expires
                 FROM legacy.native_opaque_v1",
            ),
            (
                "native_handoff_v1",
                "INSERT OR IGNORE INTO main.native_handoff_v1(caller,thread,generation,source_account,source_owner,reason,created_at)
                 SELECT caller,thread,generation,source_account,source_owner,reason,created_at
                 FROM legacy.native_handoff_v1",
            ),
            (
                "native_recovery_commits_v1",
                "INSERT OR IGNORE INTO main.native_recovery_commits_v1(id,caller,thread,account,owner,completed_at)
                 SELECT id,caller,thread,account,owner,completed_at FROM legacy.native_recovery_commits_v1",
            ),
            (
                "native_rebuild_active_v1",
                "INSERT OR IGNORE INTO main.native_rebuild_active_v1(caller,thread,account,owner,generation,revision)
                 SELECT caller,thread,account,owner,generation,revision FROM legacy.native_rebuild_active_v1",
            ),
            (
                "native_rebuild_refs_v1",
                "INSERT OR IGNORE INTO main.native_rebuild_refs_v1(caller,kind,digest,generation)
                 SELECT caller,kind,digest,generation FROM legacy.native_rebuild_refs_v1",
            ),
            (
                "native_rebuild_blobs_v1",
                "INSERT OR IGNORE INTO main.native_rebuild_blobs_v1(caller,thread,generation,digest,account,owner)
                 SELECT caller,thread,generation,digest,account,owner FROM legacy.native_rebuild_blobs_v1",
            ),
            (
                "native_rebuild_portable_v1",
                "INSERT OR IGNORE INTO main.native_rebuild_portable_v1(caller,thread,generation,account,owner)
                 SELECT caller,thread,generation,account,owner FROM legacy.native_rebuild_portable_v1",
            ),
            (
                "native_rebuild_checkpoints_v1",
                "INSERT OR IGNORE INTO main.native_rebuild_checkpoints_v1
                    (caller,response,thread,account,generation,transcript,bytes,created,parameters)
                 SELECT caller,response,thread,account,generation,transcript,bytes,created,parameters
                 FROM legacy.native_rebuild_checkpoints_v1
                 ORDER BY created DESC, rowid DESC LIMIT 128",
            ),
        ] {
            if table_exists(legacy_conn, table)? {
                tx.execute(sql, [])?;
            }
        }
        if table_exists(legacy_conn, "native_partial_history_v1")?
            && super::partial_history::has_column(
                legacy_conn,
                "native_partial_history_v1",
                "transcript",
            )?
        {
            tx.execute(
                "INSERT OR IGNORE INTO main.native_partial_history_v1
                    (caller,response,thread,session,account,owner,transcript,parameters,bytes,created)
                 SELECT caller,response,thread,session,account,owner,transcript,parameters,bytes,created
                 FROM legacy.native_partial_history_v1
                 ORDER BY created DESC, rowid DESC LIMIT 512",
                [],
            )?;
        }
        let _ = prune_checkpoints_batch(
            &tx,
            now_secs,
            cutoff,
            &MaintenancePolicy::default(),
            false,
            usize::MAX,
        );
        if cutoff > 0 {
            let _ = tx.execute(
                "DELETE FROM main.native_partial_history_v1 WHERE created > 0 AND created < ?1",
                params![cutoff],
            );
        }
        let _ = super::partial_history::evict_partial_cache(&tx);
        tx.commit()
    })();
    let _ = slim.execute("DETACH DATABASE legacy", []);
    copy_result?;
    let _ = slim.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;");
    Ok(())
}

pub(super) fn migrate_and_slim_on_open(
    path: &Path,
    mut connection: Connection,
) -> Result<Connection, String> {
    let has_legacy = has_legacy_archive_objects(&connection).map_err(|e| e.to_string())?;
    let metrics = read_db_size_metrics(&connection)?;
    let disk_bytes = std::fs::metadata(path)
        .map(|m| m.len() as i64)
        .unwrap_or(metrics.total_bytes);
    let needs_side_file_rebuild = (has_legacy
        && (disk_bytes > SLIM_REBUILD_THRESHOLD_BYTES
            || metrics.total_bytes > SLIM_REBUILD_THRESHOLD_BYTES))
        || (metrics.freelist_bytes >= VACUUM_FREELIST_THRESHOLD_BYTES && metrics.auto_vacuum != 2);

    if needs_side_file_rebuild && path.is_file() {
        let slim_path = path.with_extension("sqlite.slim.tmp");
        let slim_wal = PathBuf::from(format!("{}-wal", slim_path.display()));
        let slim_shm = PathBuf::from(format!("{}-shm", slim_path.display()));
        match copy_live_state_to_slim_db(path, &connection, &slim_path) {
            Ok(()) => {
                drop(connection);
                let wal_path = PathBuf::from(format!("{}-wal", path.display()));
                let shm_path = PathBuf::from(format!("{}-shm", path.display()));
                let _ = std::fs::remove_file(&wal_path);
                let _ = std::fs::remove_file(&shm_path);
                let _ = std::fs::remove_file(&slim_wal);
                let _ = std::fs::remove_file(&slim_shm);
                if std::fs::rename(&slim_path, path).is_ok() {
                    let reopened = Connection::open(path).map_err(|e| e.to_string())?;
                    reopened
                        .busy_timeout(Duration::from_secs(2))
                        .map_err(|e| e.to_string())?;
                    initialize_clean_schema(&reopened).map_err(|e| e.to_string())?;
                    return Ok(reopened);
                }
                let _ = std::fs::remove_file(&slim_path);
                connection = Connection::open(path).map_err(|e| e.to_string())?;
                connection
                    .busy_timeout(Duration::from_secs(2))
                    .map_err(|e| e.to_string())?;
            }
            Err(_) => {
                let _ = std::fs::remove_file(&slim_path);
                let _ = std::fs::remove_file(&slim_wal);
                let _ = std::fs::remove_file(&slim_shm);
            }
        }
    }

    if has_legacy {
        let _ = salvage_legacy_rows(&connection, &connection);
        drop_legacy_archive_objects(&connection).map_err(|e| e.to_string())?;
    }
    let after = read_db_size_metrics(&connection)?;
    if after.freelist_bytes > 0 && (has_legacy || after.freelist_bytes >= VACUUM_FREELIST_THRESHOLD_BYTES) {
        reclaim_pages_incremental(&connection, after.auto_vacuum);
    }
    super::partial_history::initialize(&connection).map_err(|e| e.to_string())?;
    Ok(connection)
}

// Scope the handler and busy timeout to background work. In particular, remove
// the handler after the transaction has rolled back so foreground queries can
// reuse this connection after an interrupted batch.
fn with_batch_budget(
    connection: &mut Connection,
    budget: Duration,
    operation: impl FnOnce(&mut Connection) -> Result<MaintenanceReport, String>,
) -> Result<MaintenanceReport, String> {
    let busy_timeout: u64 = connection
        .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    connection
        .busy_timeout(Duration::ZERO)
        .map_err(|error| error.to_string())?;
    let started = Instant::now();
    let interrupted = Arc::new(AtomicBool::new(false));
    let did_interrupt = Arc::clone(&interrupted);
    connection.progress_handler(
        1_000,
        Some(move || {
            // Only interrupt once, allowing the transaction's rollback to run.
            if started.elapsed() >= budget && !did_interrupt.swap(true, Ordering::Relaxed) {
                return true;
            }
            false
        }),
    );
    let result = operation(connection);
    connection.progress_handler(0, None::<fn() -> bool>);
    connection
        .busy_timeout(Duration::from_millis(busy_timeout))
        .map_err(|error| error.to_string())?;
    if interrupted.load(Ordering::Relaxed) {
        return Err("Identity archive maintenance batch exceeded its time budget".to_string());
    }
    result
}

fn reclaim_pages_incremental(connection: &Connection, auto_vacuum: i64) {
    if auto_vacuum == 2 {
        if let Ok(mut stmt) = connection.prepare("PRAGMA incremental_vacuum(2048);") {
            if let Ok(mut rows) = stmt.query([]) {
                while let Ok(Some(_)) = rows.next() {}
            }
        }
    }
    let _ = connection.execute_batch("PRAGMA wal_checkpoint(PASSIVE);");
    let _ = connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
}

fn convert_and_vacuum(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "PRAGMA auto_vacuum = INCREMENTAL;
             VACUUM;
             PRAGMA wal_checkpoint(TRUNCATE);",
        )
        .map_err(|error| error.to_string())
}

fn prune_partial_batch(
    transaction: &Transaction<'_>,
    cutoff: i64,
    policy: &MaintenancePolicy,
    over_budget: bool,
    limit: usize,
) -> Result<usize, String> {
    if limit == 0 {
        return Ok(0);
    }
    let mut deleted = 0usize;
    if cutoff > 0 {
        deleted += transaction
            .execute(
                "DELETE FROM native_partial_history_v1
                 WHERE rowid IN (
                    SELECT rowid FROM native_partial_history_v1
                    WHERE created > 0 AND created < ?1
                    ORDER BY created ASC, rowid ASC LIMIT ?2
                 )",
                params![cutoff, limit as i64],
            )
            .map_err(|error| error.to_string())?;
    }
    let remaining = limit.saturating_sub(deleted);
    if remaining == 0 {
        return Ok(deleted);
    }
    let total: usize = transaction
        .query_row(
            "SELECT COUNT(*) FROM (SELECT 1 FROM native_partial_history_v1 LIMIT ?1)",
            params![(policy.max_partial_rows.saturating_add(remaining)) as i64],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let take = if over_budget {
        remaining
    } else {
        total.saturating_sub(policy.max_partial_rows).min(remaining)
    };
    if take > 0 {
        deleted += transaction
            .execute(
                "DELETE FROM native_partial_history_v1
                 WHERE rowid IN (
                    SELECT rowid FROM native_partial_history_v1
                    ORDER BY created ASC, rowid ASC LIMIT ?1
                 )",
                params![take as i64],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(deleted)
}

fn prune_checkpoints_batch(
    transaction: &Transaction<'_>,
    now_secs: i64,
    cutoff: i64,
    policy: &MaintenancePolicy,
    over_budget: bool,
    limit: usize,
) -> Result<usize, String> {
    if limit == 0 {
        return Ok(0);
    }
    let mut deleted = 0usize;
    if cutoff > 0 {
        deleted += transaction
            .execute(
                "DELETE FROM native_rebuild_checkpoints_v1
                 WHERE rowid IN (
                    SELECT rowid FROM native_rebuild_checkpoints_v1
                    WHERE created > 0 AND created < ?1
                    ORDER BY created ASC, rowid ASC LIMIT ?2
                 )",
                params![cutoff, limit as i64],
            )
            .map_err(|error| error.to_string())?;
    }
    let remaining = limit.saturating_sub(deleted);
    if remaining > 0 && policy.max_history_per_thread > 0 {
        deleted += transaction
            .execute(
                "WITH ranked AS (
                    SELECT rowid, created,
                           ROW_NUMBER() OVER (
                             PARTITION BY caller, thread
                             ORDER BY created DESC, response DESC
                           ) AS rn
                    FROM native_rebuild_checkpoints_v1
                 )
                 DELETE FROM native_rebuild_checkpoints_v1
                 WHERE rowid IN (
                    SELECT rowid FROM ranked
                    WHERE rn > ?1
                    ORDER BY created ASC, rowid ASC LIMIT ?2
                 )",
                params![policy.max_history_per_thread as i64, remaining as i64],
            )
            .map_err(|error| error.to_string())?;
    }
    let remaining = limit.saturating_sub(deleted);
    if remaining > 0 && over_budget {
        let recent_guard = now_secs.saturating_sub(600);
        deleted += transaction
            .execute(
                "DELETE FROM native_rebuild_checkpoints_v1
                 WHERE rowid IN (
                    SELECT rowid FROM native_rebuild_checkpoints_v1
                    WHERE created = 0 OR created < ?1
                    ORDER BY created ASC, rowid ASC LIMIT ?2
                 )",
                params![recent_guard, remaining as i64],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(deleted)
}

fn prune_legacy_tables_if_present(
    transaction: &Transaction<'_>,
    limit: usize,
    report: &mut MaintenanceReport,
) -> Result<bool, String> {
    let mut has_more = false;
    if table_exists(transaction, "native_rebuild_history_v1").map_err(|e| e.to_string())? {
        let deleted = transaction
            .execute(
                "DELETE FROM native_rebuild_history_v1
                 WHERE rowid IN (SELECT rowid FROM native_rebuild_history_v1 LIMIT ?1)",
                params![limit.max(1) as i64],
            )
            .map_err(|e| e.to_string())?;
        report.deleted_history_rows += deleted;
        if deleted >= limit.max(1) {
            has_more = true;
        } else {
            transaction
                .execute_batch("DROP TABLE IF EXISTS native_rebuild_history_v1;")
                .map_err(|e| e.to_string())?;
        }
    }
    if !has_more
        && table_exists(transaction, "native_rebuild_items_v1").map_err(|e| e.to_string())?
    {
        let deleted = transaction
            .execute(
                "DELETE FROM native_rebuild_items_v1
                 WHERE rowid IN (SELECT rowid FROM native_rebuild_items_v1 LIMIT ?1)",
                params![limit.max(1) as i64],
            )
            .map_err(|e| e.to_string())?;
        report.deleted_item_rows += deleted;
        if deleted >= limit.max(1) {
            has_more = true;
        } else {
            transaction
                .execute_batch("DROP TABLE IF EXISTS native_rebuild_items_v1;")
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(has_more)
}

impl IdentityStore {
    /// Run one best-effort batch, yielding to foreground connection users.
    /// Long SQL statements are interrupted at the policy's time budget.
    pub fn prune_archive_batch(
        &self,
        now_secs: i64,
        policy: &MaintenancePolicy,
        allow_full_vacuum: bool,
    ) -> Result<MaintenanceReport, String> {
        let mut connection = match self.connection.try_lock() {
            Ok(connection) => connection,
            Err(TryLockError::WouldBlock) => {
                return Ok(MaintenanceReport {
                    has_more: true,
                    ..MaintenanceReport::default()
                });
            }
            Err(TryLockError::Poisoned(_)) => {
                return Err("Identity store lock poisoned".to_string());
            }
        };
        with_batch_budget(&mut connection, policy.max_batch_duration, |connection| {
            let metrics = read_db_size_metrics(connection)?;
            let over_budget = metrics.live_bytes > policy.max_archive_bytes
                || (metrics.live_bytes > policy.target_archive_bytes && metrics.freelist_bytes > 0);
            let cutoff = now_secs.saturating_sub(policy.retention_secs.max(0));
            let mut report = MaintenanceReport::default();

            let transaction = connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|error| error.to_string())?;

            let legacy_has_more =
                prune_legacy_tables_if_present(&transaction, policy.batch_size, &mut report)?;

            report.deleted_partial_rows += prune_partial_batch(
                &transaction,
                cutoff,
                policy,
                over_budget,
                policy.batch_size,
            )?;

            let checkpoint_limit = policy.batch_size.saturating_sub(report.deleted_partial_rows);
            report.deleted_checkpoint_rows += prune_checkpoints_batch(
                &transaction,
                now_secs,
                cutoff,
                policy,
                over_budget,
                checkpoint_limit,
            )?;

            transaction.commit().map_err(|error| error.to_string())?;

            let pruned_count = report.deleted_partial_rows + report.deleted_checkpoint_rows;
            report.has_more =
                (policy.batch_size > 0 && pruned_count >= policy.batch_size) || legacy_has_more;

            let before_reclaim = read_db_size_metrics(connection)?;
            if report.any_deleted() || before_reclaim.freelist_bytes > 0 {
                reclaim_pages_incremental(connection, before_reclaim.auto_vacuum);
            }
            let after = read_db_size_metrics(connection)?;
            if after.auto_vacuum == 2
                && after.freelist_bytes >= policy.vacuum_freelist_threshold_bytes
                && after.freelist_bytes < before_reclaim.freelist_bytes
            {
                report.has_more = true;
            }
            if allow_full_vacuum
                && !report.has_more
                && after.auto_vacuum != 2
                && after.freelist_bytes >= policy.vacuum_freelist_threshold_bytes
            {
                convert_and_vacuum(connection)?;
                report.vacuumed = true;
            }
            Ok(report)
        })
    }

    /// Clear cached conversation transcripts and compact the database on disk.
    /// Keeps identity/affinity/opaque ownership metadata and each thread's most
    /// recent turn from the last hour so active conversations can still resume.
    pub fn clear_archived_history_and_compact(&self) -> Result<MaintenanceReport, String> {
        let now_secs = timestamp();
        self.clear_archived_history_and_compact_at(now_secs, ACTIVE_CONVERSATION_GRACE_SECS)
    }

    pub(crate) fn clear_archived_history_and_compact_at(
        &self,
        now_secs: i64,
        active_grace_secs: i64,
    ) -> Result<MaintenanceReport, String> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "Identity store lock poisoned".to_string())?;
        let grace_cutoff = now_secs.saturating_sub(active_grace_secs.max(0));
        let mut report = MaintenanceReport::default();

        if has_legacy_archive_objects(&connection).map_err(|e| e.to_string())? {
            let _ = salvage_legacy_rows(&connection, &connection);
            if table_exists(&connection, "native_rebuild_history_v1").map_err(|e| e.to_string())? {
                let count: usize = connection
                    .query_row("SELECT COUNT(*) FROM native_rebuild_history_v1", [], |r| {
                        r.get(0)
                    })
                    .unwrap_or(0);
                report.deleted_history_rows += count;
            }
            if table_exists(&connection, "native_rebuild_items_v1").map_err(|e| e.to_string())? {
                let count: usize = connection
                    .query_row("SELECT COUNT(*) FROM native_rebuild_items_v1", [], |r| {
                        r.get(0)
                    })
                    .unwrap_or(0);
                report.deleted_item_rows += count;
            }
            drop_legacy_archive_objects(&connection).map_err(|e| e.to_string())?;
            super::partial_history::initialize(&connection).map_err(|e| e.to_string())?;
        }

        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        // Keep only the single newest checkpoint per (caller, thread) created within grace window.
        report.deleted_checkpoint_rows += transaction
            .execute(
                "WITH ranked AS (
                    SELECT rowid, created,
                           ROW_NUMBER() OVER (
                             PARTITION BY caller, thread
                             ORDER BY created DESC, response DESC
                           ) AS rn
                    FROM native_rebuild_checkpoints_v1
                 )
                 DELETE FROM native_rebuild_checkpoints_v1
                 WHERE rowid IN (
                    SELECT rowid FROM ranked
                    WHERE rn > 1 OR created = 0 OR created < ?1
                 )",
                params![grace_cutoff],
            )
            .map_err(|error| error.to_string())?;
        // Clear partial history older than grace window.
        report.deleted_partial_rows += transaction
            .execute(
                "DELETE FROM native_partial_history_v1 WHERE created = 0 OR created < ?1",
                params![grace_cutoff],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;

        convert_and_vacuum(&connection)?;
        report.vacuumed = true;
        Ok(report)
    }

    /// Compact an on-disk `identity.sqlite` file when no native server handle is active.
    pub fn compact_path_if_exists(
        path: &std::path::Path,
    ) -> Result<Option<MaintenanceReport>, String> {
        if !path.is_file() {
            return Ok(None);
        }
        let store = Self::open(path.to_path_buf())?;
        let report = store.clear_archived_history_and_compact()?;
        Ok(Some(report))
    }

    pub(crate) fn try_acquire_maintenance_guard(self: &Arc<Self>) -> Option<MaintenanceGuard> {
        self.maintenance_running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| MaintenanceGuard {
                store: Arc::clone(self),
            })
    }
}

pub(crate) struct MaintenanceGuard {
    store: Arc<IdentityStore>,
}

impl Drop for MaintenanceGuard {
    fn drop(&mut self) {
        self.store
            .maintenance_running
            .store(false, Ordering::Release);
    }
}

#[cfg(test)]
#[path = "identity_maintenance_tests.rs"]
mod tests;
