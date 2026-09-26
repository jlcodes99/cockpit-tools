//! Separate SQLite index and immutable per-item artifacts; all callers run blocking I/O off runtime threads.
use super::{Artifact, Batch, History, IO_TIMEOUT};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, Weak};

static ROOT: LazyLock<Mutex<Option<PathBuf>>> = LazyLock::new(|| Mutex::new(None));
static BATCH_GATES: LazyLock<Mutex<HashMap<String, Weak<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
const MAX_ARTIFACT_BYTES: u64 = 32 * 1024 * 1024;
const DEFAULT_RETENTION_DAYS: u32 = 7;

fn with_batch<T>(id: &str, operation: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    validate_id(id)?;
    let gate = {
        let mut gates = BATCH_GATES.lock().map_err(storage_error)?;
        gates.retain(|_, gate| gate.strong_count() > 0);
        if let Some(gate) = gates.get(id).and_then(Weak::upgrade) {
            gate
        } else {
            let gate = Arc::new(Mutex::new(()));
            gates.insert(id.to_string(), Arc::downgrade(&gate));
            gate
        }
    };
    let _guard = gate.lock().map_err(storage_error)?;
    operation()
}

fn storage_error(error: impl std::fmt::Display) -> String {
    format!("pelican.error.storage: {error}")
}

fn root() -> Result<PathBuf, String> {
    // This maintenance-only lock is never held on an async runtime thread or by account reads.
    // A timed-out caller cannot cause duplicate recovery: the blocking initialization retains it.
    let mut root = ROOT.lock().map_err(storage_error)?;
    if let Some(path) = root.as_ref() {
        return Ok(path.clone());
    }
    let path = crate::modules::account::get_data_dir()?.join("codex_pelican");
    initialize(&path)?;
    let path = path.canonicalize().map_err(storage_error)?;
    *root = Some(path.clone());
    Ok(path)
}

fn safe_dir(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err("pelican.error.unsafePath".into())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(storage_error)
        }
        Err(error) => Err(storage_error(error)),
    }
}

fn open(path: &Path) -> Result<Connection, String> {
    let database = path.join("history.sqlite3");
    if fs::symlink_metadata(&database).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err("pelican.error.unsafePath".into());
    }
    let connection = Connection::open(database).map_err(storage_error)?;
    connection
        .busy_timeout(std::time::Duration::from_secs(5))
        .map_err(storage_error)?;
    Ok(connection)
}

fn initialize(path: &Path) -> Result<(), String> {
    safe_dir(path)?;
    safe_dir(&path.join("artifacts"))?;
    let connection = open(path)?;
    connection
        .execute_batch(
            "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS batches (
             id TEXT PRIMARY KEY, created_at INTEGER NOT NULL, finished_at INTEGER,
             revision INTEGER NOT NULL, status TEXT NOT NULL, snapshot TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS batches_created ON batches(created_at DESC, id DESC);
         CREATE INDEX IF NOT EXISTS batches_status ON batches(status);
         CREATE TABLE IF NOT EXISTS settings (
             key TEXT PRIMARY KEY, value TEXT NOT NULL
         );
         INSERT OR IGNORE INTO settings(key,value) VALUES('retention_days','7');",
        )
        .map_err(storage_error)?;
    // Only mark unfinished rows. Read-time projection updates item states without rewriting a
    // potentially large history, and repeated recovery never touches completed data/artifacts.
    connection
        .execute(
            "UPDATE batches SET status='interrupted', finished_at=?1, revision=revision+1
         WHERE status IN ('running','cancelling')",
            [super::now()],
        )
        .map_err(storage_error)?;
    Ok(())
}

async fn blocking<T: Send + 'static>(
    operation: impl FnOnce(PathBuf) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::time::timeout(
        IO_TIMEOUT,
        tokio::task::spawn_blocking(move || operation(root()?)),
    )
    .await
    .map_err(|_| "pelican.error.storageTimeout".to_string())?
    .map_err(storage_error)?
}

pub(super) async fn save(batch: Batch) -> Result<(), String> {
    blocking(move |path| with_batch(&batch.id, || save_at(&path, &batch))).await
}

fn save_at(path: &Path, batch: &Batch) -> Result<(), String> {
    let snapshot = serde_json::to_string(batch).map_err(storage_error)?;
    open(path)?.execute(
        "INSERT INTO batches(id,created_at,finished_at,revision,status,snapshot) VALUES(?1,?2,?3,?4,?5,?6)
         ON CONFLICT(id) DO UPDATE SET finished_at=excluded.finished_at,
             revision=excluded.revision,status=excluded.status,snapshot=excluded.snapshot
         WHERE batches.revision < excluded.revision AND batches.status != 'deleted'",
        params![batch.id, batch.created_at, batch.finished_at, batch.revision, batch.status, snapshot],
    ).map_err(storage_error)?;
    Ok(())
}

fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<(String, String, Option<i64>, u64)> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
}

fn project(
    (snapshot, status, finished_at, revision): (String, String, Option<i64>, u64),
) -> Result<Batch, String> {
    let mut batch: Batch = serde_json::from_str(&snapshot).map_err(storage_error)?;
    batch.status = status;
    batch.finished_at = finished_at;
    batch.revision = revision;
    if batch.status == "interrupted" {
        for item in &mut batch.items {
            if matches!(item.status.as_str(), "running" | "queued") {
                item.status = "interrupted".into();
                item.finished_at = finished_at;
            }
        }
    }
    Ok(batch)
}

pub(super) async fn read(id: String) -> Result<Batch, String> {
    blocking(move |path| read_at(&path, &id)).await
}

fn read_at(path: &Path, id: &str) -> Result<Batch, String> {
    validate_id(id)?;
    let row = open(path)?
        .query_row(
            "SELECT snapshot,status,finished_at,revision FROM batches WHERE id=?1 AND status != 'deleted'",
            [id],
            decode,
        )
        .optional()
        .map_err(storage_error)?
        .ok_or_else(|| "pelican.error.historyMissing".to_string())?;
    project(row)
}

pub(super) async fn history(offset: usize, limit: usize) -> Result<History, String> {
    blocking(move |path| {
        let connection = open(&path)?;
        let mut statement = connection.prepare(
            "SELECT snapshot,status,finished_at,revision FROM batches WHERE status != 'deleted' ORDER BY created_at DESC,id DESC LIMIT ?1 OFFSET ?2"
        ).map_err(storage_error)?;
        let rows = statement.query_map(params![(limit + 1) as i64, offset.min(i64::MAX as usize) as i64], decode).map_err(storage_error)?;
        let mut items = Vec::new();
        for row in rows { items.push(project(row.map_err(storage_error)?)?); }
        let has_more = items.len() > limit;
        items.truncate(limit);
        Ok(History { items, has_more })
    }).await
}

pub(super) async fn retention_days() -> Result<u32, String> {
    blocking(move |path| {
        let value: String = open(&path)?
            .query_row(
                "SELECT value FROM settings WHERE key='retention_days'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(storage_error)?
            .unwrap_or_else(|| DEFAULT_RETENTION_DAYS.to_string());
        Ok(value
            .parse::<u32>()
            .ok()
            .filter(|days| (1..=3650).contains(days))
            .unwrap_or(DEFAULT_RETENTION_DAYS))
    })
    .await
}

pub(super) async fn set_retention_days(days: u32) -> Result<(), String> {
    blocking(move |path| {
        open(&path)?.execute(
            "INSERT INTO settings(key,value) VALUES('retention_days',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [days.to_string()],
        ).map_err(storage_error)?;
        Ok(())
    }).await
}

pub(super) async fn cleanup_expired(
    days: u32,
    current_time: i64,
    active_id: Option<String>,
) -> Result<Vec<String>, String> {
    blocking(move |root| cleanup_expired_at(&root, days, current_time, active_id.as_deref())).await
}

pub(super) async fn clear_all() -> Result<usize, String> {
    blocking(move |root| clear_all_at(&root)).await
}

fn cleanup_expired_at(
    root: &Path,
    days: u32,
    current_time: i64,
    active_id: Option<&str>,
) -> Result<Vec<String>, String> {
    let cutoff = current_time.saturating_sub(i64::from(days).saturating_mul(86_400_000));
    let batches = {
        let connection = open(root)?;
        let mut statement = connection.prepare(
            "SELECT id,snapshot FROM batches WHERE status NOT IN ('running','cancelling','deleted') AND COALESCE(finished_at,created_at) < ?1 AND (?2 IS NULL OR id != ?2)"
        ).map_err(storage_error)?;
        let rows = statement
            .query_map(params![cutoff, active_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(storage_error)?;
        let mut batches = Vec::new();
        for row in rows {
            batches.push(row.map_err(storage_error)?);
        }
        batches
    };
    let mut deleted = Vec::new();
    for (id, snapshot) in batches {
        let batch: Batch = serde_json::from_str(&snapshot).map_err(storage_error)?;
        with_batch(&id, || {
            delete_batch_artifacts(root, &batch)?;
            open(root)?
                .execute("DELETE FROM batches WHERE id=?1", [&id])
                .map_err(storage_error)?;
            Ok(())
        })?;
        deleted.push(id);
    }
    Ok(deleted)
}

fn clear_all_at(root: &Path) -> Result<usize, String> {
    let count: usize = open(root)?
        .query_row(
            "SELECT COUNT(*) FROM batches WHERE status != 'deleted'",
            [],
            |row| row.get(0),
        )
        .map_err(storage_error)?;
    let artifacts = root.join("artifacts");
    safe_dir(&artifacts)?;
    for entry in fs::read_dir(&artifacts).map_err(storage_error)? {
        let entry = entry.map_err(storage_error)?;
        let metadata = entry.file_type().map_err(storage_error)?;
        if metadata.is_symlink() || !metadata.is_file() {
            return Err("pelican.error.unsafePath".into());
        }
        fs::remove_file(entry.path()).map_err(storage_error)?;
    }
    let connection = open(root)?;
    connection
        .execute("DELETE FROM batches", [])
        .map_err(storage_error)?;
    connection.execute_batch("VACUUM").map_err(storage_error)?;
    Ok(count)
}

fn validate_id(id: &str) -> Result<(), String> {
    let parsed = uuid::Uuid::parse_str(id).map_err(|_| "pelican.error.invalidId".to_string())?;
    if parsed.to_string() != id {
        return Err("pelican.error.invalidId".into());
    }
    Ok(())
}

fn artifact_path(root: &Path, batch_id: &str, item_id: &str) -> Result<PathBuf, String> {
    validate_id(batch_id)?;
    validate_id(item_id)?;
    let artifacts = root.join("artifacts");
    safe_dir(&artifacts)?;
    let canonical_root = root.canonicalize().map_err(storage_error)?;
    let canonical_artifacts = artifacts.canonicalize().map_err(storage_error)?;
    if canonical_artifacts.parent() != Some(canonical_root.as_path()) {
        return Err("pelican.error.unsafePath".into());
    }
    let path = canonical_artifacts.join(format!("{batch_id}_{item_id}.json"));
    if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err("pelican.error.unsafePath".into());
    }
    Ok(path)
}

fn delete_artifact_file(path: PathBuf) -> Result<(), String> {
    for candidate in [
        path.clone(),
        path.with_file_name(format!(
            "{}.bak",
            path.file_name()
                .and_then(|name| name.to_str())
                .ok_or("pelican.error.unsafePath")?
        )),
    ] {
        match fs::remove_file(candidate) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(storage_error(error)),
        }
    }
    Ok(())
}

fn delete_batch_artifacts(root: &Path, batch: &Batch) -> Result<(), String> {
    for item in &batch.items {
        delete_artifact_file(artifact_path(root, &batch.id, &item.id)?)?;
    }
    Ok(())
}

pub(super) async fn save_artifact(
    batch_id: String,
    item_id: String,
    artifact: Artifact,
) -> Result<(), String> {
    blocking(move |root| {
        with_batch(&batch_id, || {
            let exists: bool = open(&root)?
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM batches WHERE id=?1 AND status != 'deleted')",
                    [&batch_id],
                    |row| row.get(0),
                )
                .map_err(storage_error)?;
            if !exists {
                return Err("pelican.error.historyMissing".into());
            }
            let path = artifact_path(&root, &batch_id, &item_id)?;
            let content = serde_json::to_string(&artifact).map_err(storage_error)?;
            if content.len() as u64 > MAX_ARTIFACT_BYTES {
                return Err("PELICAN_RESPONSE_TOO_LARGE".into());
            }
            // Only this batch's maintenance gate is held; never a SQLite transaction or UI state lock.
            crate::modules::atomic_write::write_string_atomic(&path, &content)
                .map_err(storage_error)?;
            Ok(())
        })
    })
    .await
}

pub(super) async fn artifact(batch_id: String, item_id: String) -> Result<Artifact, String> {
    blocking(move |root| {
        with_batch(&batch_id, || {
            let path = artifact_path(&root, &batch_id, &item_id)?;
            let metadata = fs::symlink_metadata(&path).map_err(storage_error)?;
            if !metadata.is_file() || metadata.len() > MAX_ARTIFACT_BYTES {
                return Err("pelican.error.unsafePath".into());
            }
            let mut content = String::new();
            fs::File::open(path)
                .map_err(storage_error)?
                .take(MAX_ARTIFACT_BYTES + 1)
                .read_to_string(&mut content)
                .map_err(storage_error)?;
            if content.len() as u64 > MAX_ARTIFACT_BYTES {
                return Err("PELICAN_RESPONSE_TOO_LARGE".into());
            }
            serde_json::from_str(&content).map_err(storage_error)
        })
    })
    .await
}

pub(super) async fn delete(batch_id: String) -> Result<(), String> {
    blocking(move |root| {
        with_batch(&batch_id, || {
            let batch = read_at(&root, &batch_id)?;
            // Only exact UUID-derived files belonging to the selected historical batch are removed.
            // Keep metadata if any deletion fails, so users can retry cleanup instead of losing the index.
            delete_batch_artifacts(&root, &batch)?;
            // Retain only an anonymous ID tombstone so late blocking writers cannot resurrect data.
            open(&root)?
            .execute(
                "UPDATE batches SET status='deleted',snapshot='',revision=revision+1 WHERE id=?1",
                [&batch_id],
            )
            .map_err(storage_error)?;
            Ok(())
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDir(PathBuf);
    impl TestDir {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("codex-pelican-test-{}", uuid::Uuid::new_v4()));
            initialize(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn batch() -> Batch {
        Batch {
            id: uuid::Uuid::new_v4().to_string(),
            revision: 1,
            created_at: 1,
            finished_at: None,
            status: "running".into(),
            prompt: "prompt".into(),
            model: "model".into(),
            effort: "medium".into(),
            concurrency: 3,
            transport: "direct-chat".into(),
            error: None,
            delivery_instructions: "standalone HTML".into(),
            items: vec![super::super::Item {
                id: uuid::Uuid::new_v4().to_string(),
                account_id: "account".into(),
                account_email: "email".into(),
                status: "running".into(),
                started_at: Some(1),
                finished_at: None,
                has_html: false,
                error: None,
                reply_preview: None,
                usage: None,
                response_id: None,
                response_model: None,
                quota: None,
            }],
        }
    }

    #[test]
    fn recovery_is_idempotent_and_keeps_completed_results() {
        let dir = TestDir::new();
        let mut batch = batch();
        save_at(&dir.0, &batch).unwrap();
        initialize(&dir.0).unwrap();
        let interrupted = read_at(&dir.0, &batch.id).unwrap();
        assert_eq!(interrupted.status, "interrupted");
        assert_eq!(interrupted.items[0].status, "interrupted");
        initialize(&dir.0).unwrap();
        assert_eq!(
            read_at(&dir.0, &batch.id).unwrap().revision,
            interrupted.revision
        );
        batch.revision = 3;
        batch.status = "completed".into();
        batch.items[0].status = "completed".into();
        save_at(&dir.0, &batch).unwrap();
        initialize(&dir.0).unwrap();
        assert_eq!(read_at(&dir.0, &batch.id).unwrap().status, "completed");
    }

    #[test]
    fn stale_snapshots_cannot_overwrite_newer_metadata() {
        let dir = TestDir::new();
        let mut batch = batch();
        let stale = batch.clone();
        batch.revision = 5;
        batch.status = "completed".into();
        save_at(&dir.0, &batch).unwrap();
        save_at(&dir.0, &stale).unwrap();
        assert_eq!(read_at(&dir.0, &batch.id).unwrap().status, "completed");
    }

    #[test]
    fn late_snapshots_cannot_resurrect_deleted_history() {
        let dir = TestDir::new();
        let mut batch = batch();
        save_at(&dir.0, &batch).unwrap();
        open(&dir.0)
            .unwrap()
            .execute(
                "UPDATE batches SET status='deleted',snapshot='' WHERE id=?1",
                [&batch.id],
            )
            .unwrap();
        batch.revision = 100;
        save_at(&dir.0, &batch).unwrap();
        assert!(read_at(&dir.0, &batch.id).is_err());
        let snapshot: String = open(&dir.0)
            .unwrap()
            .query_row(
                "SELECT snapshot FROM batches WHERE id=?1",
                [&batch.id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(snapshot.is_empty());
    }

    #[test]
    fn gates_are_per_batch_and_release_after_operations() {
        let first = uuid::Uuid::new_v4().to_string();
        let second = uuid::Uuid::new_v4().to_string();
        with_batch(&first, || with_batch(&second, || Ok(()))).unwrap();
        with_batch(&first, || Ok(())).unwrap();
    }

    #[test]
    fn artifact_paths_reject_traversal_and_round_trip_unicode() {
        let dir = TestDir::new();
        let batch = batch();
        assert!(artifact_path(&dir.0, "../escape", &batch.items[0].id).is_err());
        assert!(artifact_path(&dir.0, &batch.id, "/tmp/escape").is_err());
        let path = artifact_path(&dir.0, &batch.id, &batch.items[0].id).unwrap();
        assert!(path.starts_with(dir.0.canonicalize().unwrap().join("artifacts")));
        let original = Artifact {
            raw_reply: "鹈鹕\n```html\n<html></html>\n```".into(),
            html: Some("<html></html>".into()),
        };
        crate::modules::atomic_write::write_string_atomic(
            &path,
            &serde_json::to_string(&original).unwrap(),
        )
        .unwrap();
        let restored: Artifact = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(restored.raw_reply, original.raw_reply);
        assert_eq!(restored.html, original.html);
    }

    #[cfg(unix)]
    #[test]
    fn artifact_symlinks_are_rejected() {
        let dir = TestDir::new();
        let batch = batch();
        let path = artifact_path(&dir.0, &batch.id, &batch.items[0].id).unwrap();
        std::os::unix::fs::symlink(dir.0.join("history.sqlite3"), &path).unwrap();
        assert!(artifact_path(&dir.0, &batch.id, &batch.items[0].id).is_err());
    }

    #[test]
    fn corrupt_metadata_is_an_error_not_an_empty_record() {
        let dir = TestDir::new();
        let batch = batch();
        save_at(&dir.0, &batch).unwrap();
        open(&dir.0)
            .unwrap()
            .execute(
                "UPDATE batches SET snapshot='broken' WHERE id=?1",
                [&batch.id],
            )
            .unwrap();
        assert!(read_at(&dir.0, &batch.id).is_err());
    }

    #[test]
    fn retention_cleanup_removes_expired_records_and_artifacts_but_keeps_active_id() {
        let dir = TestDir::new();
        let mut expired = batch();
        expired.status = "completed".into();
        expired.finished_at = Some(1);
        expired.items[0].status = "completed".into();
        let kept = Batch {
            id: uuid::Uuid::new_v4().to_string(),
            items: vec![super::super::Item {
                id: uuid::Uuid::new_v4().to_string(),
                ..expired.items[0].clone()
            }],
            ..expired.clone()
        };
        save_at(&dir.0, &expired).unwrap();
        save_at(&dir.0, &kept).unwrap();
        for item in [&expired.items[0], &kept.items[0]] {
            let owner = if item.id == expired.items[0].id {
                &expired.id
            } else {
                &kept.id
            };
            fs::write(artifact_path(&dir.0, owner, &item.id).unwrap(), "{}").unwrap();
        }
        let deleted = cleanup_expired_at(&dir.0, 7, 8 * 86_400_000, Some(&kept.id)).unwrap();
        assert_eq!(deleted, vec![expired.id.clone()]);
        assert!(read_at(&dir.0, &expired.id).is_err());
        assert!(read_at(&dir.0, &kept.id).is_ok());
        assert!(!artifact_path(&dir.0, &expired.id, &expired.items[0].id)
            .unwrap()
            .exists());
    }

    #[test]
    fn clear_all_removes_records_artifacts_and_atomic_backups() {
        let dir = TestDir::new();
        let batch = batch();
        save_at(&dir.0, &batch).unwrap();
        let artifact = artifact_path(&dir.0, &batch.id, &batch.items[0].id).unwrap();
        fs::write(&artifact, "{}").unwrap();
        fs::write(
            artifact.with_file_name(format!(
                "{}.bak",
                artifact.file_name().unwrap().to_string_lossy()
            )),
            "{}",
        )
        .unwrap();
        assert_eq!(clear_all_at(&dir.0).unwrap(), 1);
        assert!(read_at(&dir.0, &batch.id).is_err());
        assert_eq!(fs::read_dir(dir.0.join("artifacts")).unwrap().count(), 0);
    }
}
