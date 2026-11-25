//! Bounded, non-blocking storage maintenance for `identity.sqlite`.
//!
//! Bulk conversation transcripts live in `native_rebuild_history_v1`,
//! `native_partial_history_v1`, `native_rebuild_checkpoints_v1`, and the
//! content-addressed `native_rebuild_items_v1` table. Identity, affinity, and
//! ciphertext provenance metadata (`native_identity_v1`, `native_affinity_v1`,
//! `native_opaque_v1`, `native_rebuild_active_v1`, `native_rebuild_refs_v1`,
//! `native_rebuild_blobs_v1`, `native_rebuild_portable_v1`) are preserved.
use super::*;
use std::collections::{HashMap, HashSet};

pub(crate) const ARCHIVE_RETENTION_SECS: i64 = 7 * 24 * 60 * 60;
pub(crate) const ACTIVE_CONVERSATION_GRACE_SECS: i64 = 60 * 60;
pub(crate) const MAX_HISTORY_PER_THREAD: usize = 16;
pub(crate) const MAX_PARTIAL_HISTORY_ROWS: usize = 512;
pub(crate) const MAX_ARCHIVE_DB_BYTES: i64 = 256 * 1024 * 1024;
pub(crate) const TARGET_ARCHIVE_DB_BYTES: i64 = 192 * 1024 * 1024;
pub(crate) const VACUUM_FREELIST_THRESHOLD_BYTES: i64 = 64 * 1024 * 1024;
pub(crate) const PRUNE_BATCH_SIZE: usize = 128;
pub(crate) const ITEM_GC_BATCH_SIZE: usize = 512;
const INLINE_ITEM_GC_HISTORY_LIMIT: i64 = 2_000;

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
        live_bytes: live_pages.saturating_mul(page_size),
        freelist_bytes: freelist_count.max(0).saturating_mul(page_size),
    })
}

pub(super) fn ensure_created_column(
    connection: &Connection,
    table: &str,
) -> rusqlite::Result<()> {
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

fn collect_manifest_digests(manifest_json: &str, target: &mut HashSet<String>) {
    if let Ok(digests) = serde_json::from_str::<Vec<String>>(manifest_json) {
        target.extend(digests);
    }
}

fn collect_referenced_digests_for_caller(
    transaction: &Transaction<'_>,
    caller: &str,
) -> Result<HashSet<String>, String> {
    let mut live = HashSet::new();
    for sql in [
        "SELECT manifest FROM native_rebuild_history_v1 WHERE caller=?1",
        "SELECT manifest FROM native_partial_history_v1 WHERE caller=?1",
    ] {
        let mut stmt = transaction
            .prepare_cached(sql)
            .map_err(|error| error.to_string())?;
        let mut rows = stmt
            .query(params![caller])
            .map_err(|error| error.to_string())?;
        while let Some(row) = rows.next().map_err(|error| error.to_string())? {
            let manifest: String = row.get(0).map_err(|error| error.to_string())?;
            collect_manifest_digests(&manifest, &mut live);
        }
    }
    Ok(live)
}

fn delete_unreferenced_candidates(
    transaction: &Transaction<'_>,
    candidates: HashMap<String, HashSet<String>>,
) -> Result<usize, String> {
    let mut deleted = 0usize;
    let mut delete_item = transaction
        .prepare_cached("DELETE FROM native_rebuild_items_v1 WHERE caller=?1 AND digest=?2")
        .map_err(|error| error.to_string())?;
    for (caller, digests) in candidates {
        if digests.is_empty() {
            continue;
        }
        let remaining_rows: i64 = transaction
            .query_row(
                "SELECT (SELECT COUNT(*) FROM native_rebuild_history_v1 WHERE caller=?1)
                      + (SELECT COUNT(*) FROM native_partial_history_v1 WHERE caller=?1)",
                params![caller],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if remaining_rows > INLINE_ITEM_GC_HISTORY_LIMIT {
            // Defer item scan until history row catch-up shrinks the caller's manifest count.
            continue;
        }
        let live = if remaining_rows == 0 {
            HashSet::new()
        } else {
            collect_referenced_digests_for_caller(transaction, &caller)?
        };
        for digest in digests {
            if !live.contains(&digest) {
                deleted += delete_item
                    .execute(params![caller, digest])
                    .map_err(|error| error.to_string())?;
            }
        }
    }
    Ok(deleted)
}

fn select_partial_prune_candidates(
    transaction: &Transaction<'_>,
    cutoff: i64,
    policy: &MaintenancePolicy,
    over_budget: bool,
    limit: usize,
) -> Result<Vec<(i64, String, String)>, String> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let total: i64 = transaction
        .query_row("SELECT COUNT(*) FROM native_partial_history_v1", [], |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())?;
    if total == 0 {
        return Ok(Vec::new());
    }
    let excess = (total as usize).saturating_sub(policy.max_partial_rows);
    let take = if over_budget {
        limit
    } else if excess > 0 {
        excess.min(limit)
    } else {
        0
    };
    let mut rows_out = Vec::new();
    let mut seen = HashSet::new();
    if take > 0 {
        let mut stmt = transaction
            .prepare_cached(
                "SELECT rowid, caller, manifest FROM native_partial_history_v1
                 ORDER BY created ASC, rowid ASC LIMIT ?1",
            )
            .map_err(|error| error.to_string())?;
        let mut rows = stmt
            .query(params![take as i64])
            .map_err(|error| error.to_string())?;
        while let Some(row) = rows.next().map_err(|error| error.to_string())? {
            let rowid: i64 = row.get(0).map_err(|error| error.to_string())?;
            if seen.insert(rowid) {
                rows_out.push((
                    rowid,
                    row.get(1).map_err(|error| error.to_string())?,
                    row.get(2).map_err(|error| error.to_string())?,
                ));
            }
        }
    }
    let remaining = limit.saturating_sub(rows_out.len());
    if remaining > 0 {
        let mut stmt = transaction
            .prepare_cached(
                "SELECT rowid, caller, manifest FROM native_partial_history_v1
                 WHERE created > 0 AND created < ?1
                 ORDER BY created ASC, rowid ASC LIMIT ?2",
            )
            .map_err(|error| error.to_string())?;
        let mut rows = stmt
            .query(params![cutoff, remaining as i64])
            .map_err(|error| error.to_string())?;
        while let Some(row) = rows.next().map_err(|error| error.to_string())? {
            let rowid: i64 = row.get(0).map_err(|error| error.to_string())?;
            if seen.insert(rowid) {
                rows_out.push((
                    rowid,
                    row.get(1).map_err(|error| error.to_string())?,
                    row.get(2).map_err(|error| error.to_string())?,
                ));
            }
        }
    }
    Ok(rows_out)
}

fn select_history_prune_candidates(
    transaction: &Transaction<'_>,
    now_secs: i64,
    cutoff: i64,
    policy: &MaintenancePolicy,
    over_budget: bool,
    limit: usize,
) -> Result<Vec<(i64, String, String, String)>, String> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut rows_out = Vec::new();
    let mut seen = HashSet::new();
    // 1. Delete rows older than the retention window first.
    {
        let mut stmt = transaction
            .prepare_cached(
                "SELECT rowid, caller, response, manifest FROM native_rebuild_history_v1
                 WHERE created > 0 AND created < ?1
                 ORDER BY created ASC, rowid ASC LIMIT ?2",
            )
            .map_err(|error| error.to_string())?;
        let mut rows = stmt
            .query(params![cutoff, limit as i64])
            .map_err(|error| error.to_string())?;
        while let Some(row) = rows.next().map_err(|error| error.to_string())? {
            let rowid: i64 = row.get(0).map_err(|error| error.to_string())?;
            if seen.insert(rowid) {
                rows_out.push((
                    rowid,
                    row.get(1).map_err(|error| error.to_string())?,
                    row.get(2).map_err(|error| error.to_string())?,
                    row.get(3).map_err(|error| error.to_string())?,
                ));
            }
        }
    }
    // 2. Compact superseded turns within each (caller, thread) beyond max_history_per_thread.
    let remaining = limit.saturating_sub(rows_out.len());
    if remaining > 0 && policy.max_history_per_thread > 0 {
        let mut stmt = transaction
            .prepare_cached(
                "WITH ranked AS (
                    SELECT rowid, caller, response, manifest, created,
                           ROW_NUMBER() OVER (PARTITION BY caller, thread ORDER BY created DESC, rowid DESC) AS rn
                    FROM native_rebuild_history_v1
                 )
                 SELECT rowid, caller, response, manifest FROM ranked
                 WHERE rn > ?1
                 ORDER BY created ASC, rowid ASC LIMIT ?2",
            )
            .map_err(|error| error.to_string())?;
        let mut rows = stmt
            .query(params![policy.max_history_per_thread as i64, remaining as i64])
            .map_err(|error| error.to_string())?;
        while let Some(row) = rows.next().map_err(|error| error.to_string())? {
            let rowid: i64 = row.get(0).map_err(|error| error.to_string())?;
            if seen.insert(rowid) {
                rows_out.push((
                    rowid,
                    row.get(1).map_err(|error| error.to_string())?,
                    row.get(2).map_err(|error| error.to_string())?,
                    row.get(3).map_err(|error| error.to_string())?,
                ));
            }
        }
    }
    // 3. If live DB size still exceeds the archive cap, evict oldest history rows FIFO.
    // Keep very recent rows (last 10 minutes) unless the DB is over 2x the budget.
    let remaining = limit.saturating_sub(rows_out.len());
    if remaining > 0 && over_budget {
        let recent_guard = now_secs.saturating_sub(600);
        let mut stmt = transaction
            .prepare_cached(
                "SELECT rowid, caller, response, manifest FROM native_rebuild_history_v1
                 WHERE created = 0 OR created < ?1
                 ORDER BY created ASC, rowid ASC LIMIT ?2",
            )
            .map_err(|error| error.to_string())?;
        let mut rows = stmt
            .query(params![recent_guard, remaining as i64])
            .map_err(|error| error.to_string())?;
        while let Some(row) = rows.next().map_err(|error| error.to_string())? {
            let rowid: i64 = row.get(0).map_err(|error| error.to_string())?;
            if seen.insert(rowid) {
                rows_out.push((
                    rowid,
                    row.get(1).map_err(|error| error.to_string())?,
                    row.get(2).map_err(|error| error.to_string())?,
                    row.get(3).map_err(|error| error.to_string())?,
                ));
            }
        }
    }
    Ok(rows_out)
}

fn gc_orphan_items_batch(
    transaction: &Transaction<'_>,
    limit: usize,
) -> Result<(usize, bool), String> {
    if limit == 0 {
        return Ok((0, false));
    }
    let total_items: i64 = transaction
        .query_row("SELECT COUNT(*) FROM native_rebuild_items_v1", [], |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())?;
    if total_items == 0 {
        return Ok((0, false));
    }
    let total_manifests: i64 = transaction
        .query_row(
            "SELECT (SELECT COUNT(*) FROM native_rebuild_history_v1)
                  + (SELECT COUNT(*) FROM native_partial_history_v1)",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if total_manifests == 0 {
        let deleted = transaction
            .execute(
                "DELETE FROM native_rebuild_items_v1
                 WHERE rowid IN (SELECT rowid FROM native_rebuild_items_v1 LIMIT ?1)",
                params![limit as i64],
            )
            .map_err(|error| error.to_string())?;
        return Ok((deleted, deleted >= limit));
    }
    if total_manifests > INLINE_ITEM_GC_HISTORY_LIMIT * 4 {
        return Ok((0, false));
    }
    let mut live_by_caller: HashMap<String, HashSet<String>> = HashMap::new();
    for sql in [
        "SELECT caller, manifest FROM native_rebuild_history_v1",
        "SELECT caller, manifest FROM native_partial_history_v1",
    ] {
        let mut stmt = transaction
            .prepare_cached(sql)
            .map_err(|error| error.to_string())?;
        let mut rows = stmt.query([]).map_err(|error| error.to_string())?;
        while let Some(row) = rows.next().map_err(|error| error.to_string())? {
            let caller: String = row.get(0).map_err(|error| error.to_string())?;
            let manifest: String = row.get(1).map_err(|error| error.to_string())?;
            collect_manifest_digests(&manifest, live_by_caller.entry(caller).or_default());
        }
    }
    let mut orphan_rowids = Vec::new();
    let mut has_more = false;
    {
        let mut stmt = transaction
            .prepare_cached("SELECT rowid, caller, digest FROM native_rebuild_items_v1")
            .map_err(|error| error.to_string())?;
        let mut rows = stmt.query([]).map_err(|error| error.to_string())?;
        while let Some(row) = rows.next().map_err(|error| error.to_string())? {
            let rowid: i64 = row.get(0).map_err(|error| error.to_string())?;
            let caller: String = row.get(1).map_err(|error| error.to_string())?;
            let digest: String = row.get(2).map_err(|error| error.to_string())?;
            let referenced = live_by_caller
                .get(&caller)
                .is_some_and(|set| set.contains(&digest));
            if !referenced {
                if orphan_rowids.len() >= limit {
                    has_more = true;
                    break;
                }
                orphan_rowids.push(rowid);
            }
        }
    }
    let mut deleted = 0usize;
    let mut delete_stmt = transaction
        .prepare_cached("DELETE FROM native_rebuild_items_v1 WHERE rowid=?1")
        .map_err(|error| error.to_string())?;
    for rowid in orphan_rowids {
        deleted += delete_stmt
            .execute(params![rowid])
            .map_err(|error| error.to_string())?;
    }
    Ok((deleted, has_more))
}

fn reclaim_pages_incremental(connection: &Connection, auto_vacuum: i64) {
    if auto_vacuum == 2 {
        let _ = connection.execute_batch("PRAGMA incremental_vacuum(2048);");
    }
    let _ = connection.execute_batch("PRAGMA wal_checkpoint(PASSIVE);");
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

impl IdentityStore {
    /// Run a single bounded maintenance batch. Safe to call repeatedly from a
    /// background task with yields between batches so live requests never stall.
    pub fn prune_archive_batch(
        &self,
        now_secs: i64,
        policy: &MaintenancePolicy,
        allow_full_vacuum: bool,
    ) -> Result<MaintenanceReport, String> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "Identity store lock poisoned".to_string())?;
        let metrics = read_db_size_metrics(&connection)?;
        let over_budget = metrics.live_bytes > policy.max_archive_bytes
            || (metrics.live_bytes > policy.target_archive_bytes && metrics.freelist_bytes > 0);
        let cutoff = now_secs.saturating_sub(policy.retention_secs.max(0));
        let mut report = MaintenanceReport::default();

        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let mut candidate_digests: HashMap<String, HashSet<String>> = HashMap::new();

        let partial_rows = select_partial_prune_candidates(
            &transaction,
            cutoff,
            policy,
            over_budget,
            policy.batch_size,
        )?;
        if !partial_rows.is_empty() {
            let mut del_partial = transaction
                .prepare_cached("DELETE FROM native_partial_history_v1 WHERE rowid=?1")
                .map_err(|error| error.to_string())?;
            for (rowid, caller, manifest) in &partial_rows {
                collect_manifest_digests(manifest, candidate_digests.entry(caller.clone()).or_default());
                report.deleted_partial_rows += del_partial
                    .execute(params![rowid])
                    .map_err(|error| error.to_string())?;
            }
        }

        let history_limit = policy.batch_size.saturating_sub(partial_rows.len());
        let history_rows = select_history_prune_candidates(
            &transaction,
            now_secs,
            cutoff,
            policy,
            over_budget,
            history_limit,
        )?;
        if !history_rows.is_empty() {
            let mut del_history = transaction
                .prepare_cached("DELETE FROM native_rebuild_history_v1 WHERE rowid=?1")
                .map_err(|error| error.to_string())?;
            let mut del_hot = transaction
                .prepare_cached(
                    "DELETE FROM native_rebuild_checkpoints_v1 WHERE caller=?1 AND response=?2",
                )
                .map_err(|error| error.to_string())?;
            for (rowid, caller, response, manifest) in &history_rows {
                collect_manifest_digests(manifest, candidate_digests.entry(caller.clone()).or_default());
                report.deleted_history_rows += del_history
                    .execute(params![rowid])
                    .map_err(|error| error.to_string())?;
                report.deleted_checkpoint_rows += del_hot
                    .execute(params![caller, response])
                    .map_err(|error| error.to_string())?;
            }
        }

        if !candidate_digests.is_empty() {
            report.deleted_item_rows += delete_unreferenced_candidates(&transaction, candidate_digests)?;
        }

        let pruned_manifest_count = report.deleted_partial_rows + report.deleted_history_rows;
        let mut more_orphans = false;
        if pruned_manifest_count < policy.batch_size {
            let (gc_deleted, has_more_items) =
                gc_orphan_items_batch(&transaction, policy.item_gc_batch_size)?;
            report.deleted_item_rows += gc_deleted;
            more_orphans = has_more_items;
        }
        transaction.commit().map_err(|error| error.to_string())?;

        report.has_more = pruned_manifest_count >= policy.batch_size || more_orphans;
        let after = read_db_size_metrics(&connection)?;
        if report.any_deleted() || after.freelist_bytes > 0 {
            reclaim_pages_incremental(&connection, after.auto_vacuum);
        }
        if allow_full_vacuum
            && !report.has_more
            && after.auto_vacuum != 2
            && after.freelist_bytes >= policy.vacuum_freelist_threshold_bytes
        {
            convert_and_vacuum(&connection)?;
            report.vacuumed = true;
        }
        Ok(report)
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

        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        // Always clear hot cache rows that are already backed by history or older than grace.
        report.deleted_checkpoint_rows += transaction
            .execute(
                "DELETE FROM native_rebuild_checkpoints_v1
                 WHERE created < ?1
                    OR EXISTS(
                        SELECT 1 FROM native_rebuild_history_v1 h
                        WHERE h.caller = native_rebuild_checkpoints_v1.caller
                          AND h.response = native_rebuild_checkpoints_v1.response
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
        // Keep only the single newest response per (caller, thread) created within grace window.
        report.deleted_history_rows += transaction
            .execute(
                "WITH ranked AS (
                    SELECT rowid, created,
                           ROW_NUMBER() OVER (PARTITION BY caller, thread ORDER BY created DESC, rowid DESC) AS rn
                    FROM native_rebuild_history_v1
                 )
                 DELETE FROM native_rebuild_history_v1
                 WHERE rowid IN (
                    SELECT rowid FROM ranked
                    WHERE rn > 1 OR created = 0 OR created < ?1
                 )",
                params![grace_cutoff],
            )
            .map_err(|error| error.to_string())?;
        loop {
            let (deleted, has_more) = gc_orphan_items_batch(&transaction, 4096)?;
            report.deleted_item_rows += deleted;
            if !has_more || deleted == 0 {
                break;
            }
        }
        transaction.commit().map_err(|error| error.to_string())?;

        convert_and_vacuum(&connection)?;
        report.vacuumed = true;
        Ok(report)
    }

    /// Compact an on-disk `identity.sqlite` file when no native server handle is active.
    pub fn compact_path_if_exists(path: &std::path::Path) -> Result<Option<MaintenanceReport>, String> {
        if !path.is_file() {
            return Ok(None);
        }
        let store = Self::open(path.to_path_buf())?;
        let report = store.clear_archived_history_and_compact()?;
        Ok(Some(report))
    }

    pub(crate) fn try_acquire_maintenance_guard(
        self: &Arc<Self>,
    ) -> Option<MaintenanceGuard> {
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
