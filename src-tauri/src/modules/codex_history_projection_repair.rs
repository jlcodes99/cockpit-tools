//! Manual recovery of derived Codex history indexes, including while the client runs.
//! Durable rollouts are read-only; concurrent changes abort the repair.
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior};

#[path = "codex_history_projection_repair/files.rs"]
mod files;
#[path = "codex_history_projection_repair/storage.rs"]
mod storage;

const HISTORY_DB: &str = "thread_history_1.sqlite";
const STATE_DB: &str = "state_5.sqlite";
const MAX_ENTRIES: usize = 100_000;
const MAX_CANDIDATES: usize = 64;

#[derive(Debug)]
pub(crate) struct Plan {
    profile: PathBuf,
    schema: storage::Schema,
    db_identity: files::Identity,
    sources: HashMap<String, files::Evidence>,
    selected_paths: Vec<(String, String)>,
    candidates: Vec<Checkpoint>,
}

impl Plan {
    pub(crate) fn candidate_count(&self) -> usize {
        self.candidates.len()
    }

    pub(crate) fn scanned_files(&self) -> usize {
        self.sources.len()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Checkpoint {
    id: String,
    offset: i64,
    ordinal: i64,
}

struct Budget {
    started: Instant,
    bytes: u64,
}

impl Budget {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            bytes: 0,
        }
    }

    fn check(&self) -> Result<(), String> {
        if self.started.elapsed() > Duration::from_secs(45) {
            return Err(error(
                "LIMIT",
                "history scan exceeded its time budget; select fewer sessions",
            ));
        }
        Ok(())
    }

    fn consume(&mut self, bytes: usize) -> Result<(), String> {
        self.check()?;
        self.bytes = self.bytes.saturating_add(bytes as u64);
        if self.bytes > 2 * 1024 * 1024 * 1024 {
            return Err(error(
                "LIMIT",
                "history scan exceeded its byte budget; select fewer sessions",
            ));
        }
        Ok(())
    }
}

fn error(code: &str, detail: impl std::fmt::Display) -> String {
    format!("CODEX_HISTORY_REPAIR_{code}: {detail}")
}

fn database(profile: &Path, name: &str, writable: bool) -> Result<Connection, String> {
    let path = profile.join(name);
    files::regular_file(&path)?;
    for suffix in ["-wal", "-shm"] {
        let side = profile.join(format!("{name}{suffix}"));
        if side.symlink_metadata().is_ok() {
            files::regular_file(&side)?;
        }
    }
    let flags = if writable {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    };
    let db = Connection::open_with_flags(path, flags).map_err(|e| error("DATABASE", e))?;
    db.busy_timeout(Duration::from_millis(250))
        .map_err(|e| error("DATABASE", e))?;
    db.execute_batch("PRAGMA trusted_schema=OFF;")
        .map_err(|e| error("DATABASE", e))?;
    Ok(db)
}

fn checkpoint(db: &Connection, id: &str) -> Result<Option<Checkpoint>, String> {
    db.query_row(
        "SELECT thread_id,next_rollout_byte_offset,next_rollout_ordinal FROM thread_history_projection_state WHERE thread_id=?1",
        [id],
        |row| Ok(Checkpoint { id: row.get(0)?, offset: row.get(1)?, ordinal: row.get(2)? }),
    ).optional().map_err(|e| error("DATABASE", e))
}

/// Scan on a blocking maintenance worker. Preview is read-only, including while the client runs.
pub(crate) fn scan(
    profile: &Path,
    selected_session_ids: Option<&HashSet<String>>,
    on_progress: &dyn Fn(usize, usize),
) -> Result<Plan, String> {
    let mut budget = Budget::new();
    let profile = files::profile_root(profile)?;
    let history_path = profile.join(HISTORY_DB);
    if matches!(history_path.symlink_metadata(), Err(e) if e.kind()==std::io::ErrorKind::NotFound) {
        return Ok(Plan {
            profile,
            schema: storage::Schema::empty(),
            db_identity: files::Identity::default(),
            sources: HashMap::new(),
            selected_paths: vec![],
            candidates: vec![],
        });
    }
    let db = database(&profile, HISTORY_DB, false)?;
    let schema = storage::schema(&db)?;
    let db_identity = files::identity(&history_path)?;
    let registry = files::registry(&profile, &budget)?;
    let mut roots = Vec::new();
    let mut selected_paths = Vec::new();
    if let Some(selected) = selected_session_ids {
        let state = database(&profile, STATE_DB, false)?;
        let mut selected = selected.iter().collect::<Vec<_>>();
        selected.sort();
        for id in selected {
            budget.check()?;
            let path: String = state
                .query_row(
                    "SELECT rollout_path FROM threads WHERE id=?1",
                    [id],
                    |row| row.get(0),
                )
                .map_err(|_| {
                    error(
                        "SOURCE_MISSING",
                        format!("selected session {id} has no rollout path"),
                    )
                })?;
            let physical = files::selected_source(&profile, &path, &registry)?;
            roots.push(physical);
            selected_paths.push((id.to_owned(), path));
        }
    } else {
        let mut stmt = db.prepare("SELECT thread_id FROM thread_history_projection_state ORDER BY thread_id LIMIT 100001").map_err(|e| error("DATABASE", e))?;
        roots = stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| error("DATABASE", e))?
            .collect::<rusqlite::Result<Vec<String>>>()
            .map_err(|e| error("DATABASE", e))?;
    }
    if roots.len() > MAX_ENTRIES {
        return Err(error("LIMIT", "too many projected sessions"));
    }
    roots.sort();
    roots.dedup();
    let mut sources = HashMap::new();
    for (index, id) in roots.iter().enumerate() {
        files::load_chain(
            id,
            &registry,
            &mut sources,
            &mut HashSet::new(),
            &mut budget,
        )?;
        on_progress(index + 1, roots.len());
        std::thread::yield_now();
    }
    let mut candidates = Vec::new();
    for (id, source) in &sources {
        budget.check()?;
        let Some(saved) = checkpoint(&db, id)? else {
            continue;
        };
        if saved.offset < 0 || saved.ordinal < 0 {
            return Err(error(
                "UNSAFE_HISTORY",
                format!("negative projection cursor for {id}"),
            ));
        }
        if saved.offset as u64 <= source.decoded_bytes {
            continue;
        }
        if !source.has_ordinals
            || saved.ordinal as u64 > source.next_ordinal
            || (saved.ordinal as u64) < source.initial_ordinal
        {
            return Err(error(
                "MISSING_RECORDS",
                format!("durable records do not cover the projection for {id}"),
            ));
        }
        candidates.push(saved);
    }
    candidates.sort_by(|a, b| a.id.cmp(&b.id));
    if candidates.len() > MAX_CANDIDATES {
        return Err(error(
            "LIMIT",
            "too many damaged projections; select fewer sessions",
        ));
    }
    Ok(Plan {
        profile,
        schema,
        db_identity,
        sources,
        selected_paths,
        candidates,
    })
}

/// Persist a typed, bounded backup before atomically invalidating only proven stale projections.
pub(crate) fn apply(profile: &Path, plan: &Plan, backup_dir: &Path) -> Result<usize, String> {
    apply_with_checkpoints(profile, plan, backup_dir, &|| {})
}

// Checkpoints let concurrency tests schedule real writes at transaction boundaries.
fn apply_with_checkpoints(
    profile: &Path,
    plan: &Plan,
    backup_dir: &Path,
    checkpoint: &dyn Fn(),
) -> Result<usize, String> {
    if files::profile_root(profile)? != plan.profile {
        return Err(error("CHANGED", "profile changed since scan"));
    }
    if plan.candidates.is_empty() {
        return Ok(0);
    }
    let mut budget = Budget::new();
    for source in plan.sources.values() {
        source.recheck(&mut budget)?;
    }
    validate_selection(plan)?;
    if files::identity(&plan.profile.join(HISTORY_DB))? != plan.db_identity {
        return Err(error("CHANGED", "history database was replaced"));
    }
    let mut db = database(&plan.profile, HISTORY_DB, true)?;
    if storage::schema(&db)? != plan.schema {
        return Err(error("CHANGED", "history database schema changed"));
    }
    let snapshots = {
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|e| error("DATABASE", e))?;
        validate_cursors(&tx, plan)?;
        let rows = storage::capture(&tx, &plan.schema, &plan.candidates, &mut budget)?;
        tx.commit().map_err(|e| error("DATABASE", e))?;
        rows
    };
    storage::backup(backup_dir, plan, &snapshots, &mut budget)?;
    checkpoint();
    // Expensive decompression and hashing finished before acquiring the write transaction.
    for source in plan.sources.values() {
        source.check_stamp()?;
    }
    validate_selection(plan)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| error("DATABASE", e))?;
    if storage::schema(&tx)? != plan.schema {
        return Err(error("CHANGED", "history database schema changed"));
    }
    validate_cursors(&tx, plan)?;
    if storage::capture(&tx, &plan.schema, &plan.candidates, &mut budget)? != snapshots {
        return Err(error("CHANGED", "derived history changed after backup"));
    }
    for candidate in &plan.candidates {
        for table in storage::TABLES {
            budget.check()?;
            tx.execute(
                &format!("DELETE FROM {table} WHERE thread_id=?1"),
                [&candidate.id],
            )
            .map_err(|e| error("DATABASE", e))?;
        }
    }
    checkpoint();
    for source in plan.sources.values() {
        source.check_stamp()?;
    }
    if files::identity(&plan.profile.join(HISTORY_DB))? != plan.db_identity {
        return Err(error(
            "CHANGED",
            "history database was replaced during repair",
        ));
    }
    budget.check()?;
    tx.commit().map_err(|e| error("DATABASE", e))?;
    Ok(plan.candidates.len())
}

fn validate_cursors(db: &Connection, plan: &Plan) -> Result<(), String> {
    for expected in &plan.candidates {
        if checkpoint(db, &expected.id)?.as_ref() != Some(expected) {
            return Err(error(
                "CHANGED",
                format!("projection changed for {}", expected.id),
            ));
        }
    }
    Ok(())
}

fn validate_selection(plan: &Plan) -> Result<(), String> {
    if plan.selected_paths.is_empty() {
        return Ok(());
    }
    let db = database(&plan.profile, STATE_DB, false)?;
    for (id, path) in &plan.selected_paths {
        let current: Option<String> = db
            .query_row(
                "SELECT rollout_path FROM threads WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| error("DATABASE", e))?;
        if current.as_deref() != Some(path.as_str()) {
            return Err(error(
                "CHANGED",
                format!("selected session changed for {id}"),
            ));
        }
    }
    Ok(())
}
#[cfg(test)]
#[path = "codex_history_projection_repair/tests.rs"]
mod tests;
