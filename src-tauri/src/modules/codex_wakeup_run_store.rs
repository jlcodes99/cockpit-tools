//! Durable Codex wakeup claims and results. User-owned task rules are saved separately.
use super::{task_state, CodexWakeupState, CodexWakeupTask, QuotaResetPlan};
use crate::modules::{
    atomic_write, background_maintenance as maintenance,
    legacy_json_input::{read_array_batch, ArrayCursor, Input, ReadError},
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::ErrorKind,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone)]
pub(super) struct RunStore {
    legacy: PathBuf,
}

const DATABASE: &str = "codex_wakeup_state.sqlite";

impl RunStore {
    pub(super) fn new(legacy: PathBuf) -> Self {
        Self { legacy }
    }

    fn rules_path(&self) -> PathBuf {
        self.legacy.with_extension("rules.json")
    }

    fn legacy_backup_path(&self) -> PathBuf {
        self.legacy.with_extension("json.bak")
    }

    fn open(&self) -> Result<Connection, String> {
        let db = self.legacy.with_file_name(DATABASE);
        let conn = Connection::open(db).map_err(error)?;
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
                     CREATE TABLE IF NOT EXISTS task_runs (
                        task_id TEXT PRIMARY KEY, payload TEXT
                     );
                     CREATE TABLE IF NOT EXISTS migration (
                        singleton INTEGER PRIMARY KEY CHECK(singleton=1),
                        stamp TEXT NOT NULL, cursor TEXT NOT NULL, complete INTEGER NOT NULL
                     );
                     PRAGMA user_version=1;
                     COMMIT;",
                )
                .map_err(error)?,
            _ => return Err("Codex 唤醒状态数据库版本不受支持".into()),
        }
        Ok(conn)
    }

    pub(super) fn load(&self) -> Result<CodexWakeupState, String> {
        let mut conn = self.open()?;
        let migration = progress(&conn)?;
        let complete = migration.as_ref().is_some_and(|row| row.complete);
        let (mut state, source) = match read_document_with_backup(&self.rules_path())? {
            Some((_, rules)) => (rules, None),
            None if !complete => match self.read_legacy()? {
                Some((path, legacy)) => (legacy, Some(path)),
                None => (CodexWakeupState::default(), None),
            },
            None => (CodexWakeupState::default(), None),
        };

        let tx = conn.transaction().map_err(error)?;
        {
            let mut read = tx
                .prepare("SELECT payload FROM task_runs WHERE task_id=?1")
                .map_err(error)?;
            for task in &mut state.tasks {
                match read_run(&mut read, &task.id)? {
                    Some(runtime) => runtime.apply(task),
                    None if complete => RunState::default().apply(task),
                    None => {}
                }
            }
        }
        tx.commit().map_err(error)?;
        if !complete
            && (source.is_some()
                || migration.is_some()
                || self.legacy.exists()
                || self.legacy_backup_path().exists())
        {
            let store = self.clone();
            maintenance::schedule(
                format!("codex-wakeup-runs:{}", self.legacy.display()),
                move || store.migrate_batch(source.as_deref()),
            );
        }
        Ok(state)
    }

    pub(super) fn save_rules(
        &self,
        state: &CodexWakeupState,
        new_tasks: &[&CodexWakeupTask],
    ) -> Result<(), String> {
        // If a task ID is added again, retire old execution metadata before the
        // rule makes that ID runnable. A failed file write leaves it invisible.
        let mut conn = self.open()?;
        let migration = progress(&conn)?;
        let pending = !migration.as_ref().is_some_and(|row| row.complete)
            && (migration.is_some() || self.legacy.exists() || self.legacy_backup_path().exists());
        if !new_tasks.is_empty() {
            for chunk in new_tasks.chunks(maintenance::BATCH_ROWS) {
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(error)?;
                for task in chunk {
                    let runtime = RunState::from(*task);
                    put_run(
                        &tx,
                        &task.id,
                        runtime.has_result().then_some(&runtime),
                        true,
                    )?;
                }
                tx.commit().map_err(error)?;
            }
        }
        // A rule edit can precede background cutover. Keep its last known run
        // state available until the corresponding SQLite rows take over.
        atomic_write::write_string_atomic(&self.rules_path(), &rules_json(state, pending)?)
    }

    pub(super) fn save_run(&self, task: &CodexWakeupTask) -> Result<(), String> {
        let conn = self.open()?;
        put_run(&conn, &task.id, Some(&RunState::from(task)), true)
    }

    fn read_legacy(&self) -> Result<Option<(PathBuf, CodexWakeupState)>, String> {
        read_document_with_backup(&self.legacy)
    }

    fn migrate_batch(&self, preferred: Option<&Path>) -> Result<bool, String> {
        let mut conn = self.open()?;
        let before = progress(&conn)?;
        if before.as_ref().is_some_and(|row| row.complete) {
            return Ok(true);
        }
        let backup = self.legacy_backup_path();
        let (source, state, batch) = if let Some(previous) = &before {
            let source = if previous
                .stamp
                .starts_with(&format!("{}:", backup.display()))
            {
                &backup
            } else {
                &self.legacy
            };
            let (state, batch) =
                read_migration_batch(source, Some(previous)).map_err(legacy_error)?;
            (source.as_path(), state, batch)
        } else if let Some(source) = preferred {
            let (state, batch) = read_migration_batch(source, None).map_err(legacy_error)?;
            (source, state, batch)
        } else {
            match read_migration_batch(&self.legacy, None) {
                Ok((state, batch)) => (self.legacy.as_path(), state, batch),
                Err(ReadError::Budget) => return Err(legacy_error(ReadError::Budget)),
                Err(primary) => match read_migration_batch(&backup, None) {
                    Ok((state, batch)) => (backup.as_path(), state, batch),
                    Err(_) => return Err(legacy_error(primary)),
                },
            }
        };
        if state.complete {
            self.preserve_rules(source, &state.stamp)?;
        }
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(error)?;
        if progress(&tx)? != before {
            return Ok(false);
        }
        for row in batch {
            let id = row.id.trim();
            if !id.is_empty() && row.run.has_result() {
                put_run(&tx, id, Some(&row.run), false)?;
            }
        }
        tx.execute(
            "INSERT INTO migration(singleton,stamp,cursor,complete) VALUES (1,?1,?2,?3)
             ON CONFLICT(singleton) DO UPDATE SET stamp=excluded.stamp,
                 cursor=excluded.cursor,complete=excluded.complete",
            params![
                state.stamp,
                serde_json::to_string(&state.cursor).map_err(error)?,
                state.complete
            ],
        )
        .map_err(error)?;
        tx.commit().map_err(error)?;
        Ok(state.complete)
    }

    fn preserve_rules(&self, source: &Path, stamp: &str) -> Result<(), String> {
        let path = self.rules_path();
        if path.try_exists().map_err(error)? {
            return Ok(());
        }
        let file = File::open(source).map_err(error)?;
        if source_stamp(source, &file).map_err(error)? != stamp {
            return Err("迁移期间旧 Codex 唤醒任务文件发生变化".into());
        }
        let state = serde_json::from_reader(std::io::BufReader::new(file)).map_err(error)?;
        let content = rules_json(&state, true)?;
        let _lock = task_state::lock_task_state_at(&self.legacy)?;
        atomic_write::replace_secret_string_if_matches(&path, None, Some(&content))?;
        Ok(())
    }
}

fn read_document_with_backup(path: &Path) -> Result<Option<(PathBuf, CodexWakeupState)>, String> {
    match read_document(path) {
        Ok(value) => Ok(value.map(|value| (path.to_path_buf(), value))),
        Err(original) => {
            let backup = path.with_extension("json.bak");
            match read_document(&backup) {
                Ok(Some(value)) => Ok(Some((backup, value))),
                _ => Err(original),
            }
        }
    }
}

fn read_document(path: &Path) -> Result<Option<CodexWakeupState>, String> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(cause) => return Err(error(cause)),
    };
    if content.trim().is_empty() {
        return Ok(None);
    }
    serde_json::from_str(&content).map(Some).map_err(error)
}

fn rules_json(state: &CodexWakeupState, keep_pending_runs: bool) -> Result<String, String> {
    let mut rules = state.clone();
    for task in &mut rules.tasks {
        if !keep_pending_runs {
            RunState::default().apply(task);
        }
        task.next_run_at = None;
    }
    serde_json::to_string_pretty(&rules).map_err(error)
}

fn source_stamp(path: &Path, file: &File) -> std::io::Result<String> {
    let metadata = file.metadata()?;
    Ok(format!(
        "{}:{}:{:?}",
        path.display(),
        metadata.len(),
        metadata.modified()?
    ))
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
struct RunState {
    #[serde(skip_serializing_if = "Option::is_none")]
    quota_reset_plan: Option<QuotaResetPlan>,
    last_run_at: Option<i64>,
    last_status: Option<String>,
    last_message: Option<String>,
    last_success_count: Option<u32>,
    last_failure_count: Option<u32>,
    last_duration_ms: Option<u64>,
    updated_at: i64,
}

impl From<&CodexWakeupTask> for RunState {
    fn from(task: &CodexWakeupTask) -> Self {
        Self {
            quota_reset_plan: task.quota_reset_plan.clone(),
            last_run_at: task.last_run_at,
            last_status: task.last_status.clone(),
            last_message: task.last_message.clone(),
            last_success_count: task.last_success_count,
            last_failure_count: task.last_failure_count,
            last_duration_ms: task.last_duration_ms,
            updated_at: task.updated_at,
        }
    }
}

impl RunState {
    fn has_result(&self) -> bool {
        self.quota_reset_plan.is_some()
            || self.last_run_at.is_some()
            || self.last_status.is_some()
            || self.last_message.is_some()
            || self.last_success_count.is_some()
            || self.last_failure_count.is_some()
            || self.last_duration_ms.is_some()
    }

    fn apply(self, task: &mut CodexWakeupTask) {
        task.quota_reset_plan = self.quota_reset_plan;
        task.last_run_at = self.last_run_at;
        task.last_status = self.last_status;
        task.last_message = self.last_message;
        task.last_success_count = self.last_success_count;
        task.last_failure_count = self.last_failure_count;
        task.last_duration_ms = self.last_duration_ms;
        task.updated_at = task.updated_at.max(self.updated_at);
    }
}

fn read_run(statement: &mut rusqlite::Statement<'_>, id: &str) -> Result<Option<RunState>, String> {
    let stored: Option<Option<String>> = statement
        .query_row([id.trim()], |row| row.get(0))
        .optional()
        .map_err(error)?;
    stored
        .map(|payload| match payload {
            Some(payload) => serde_json::from_str(&payload).map_err(error),
            None => Ok(RunState::default()),
        })
        .transpose()
}

fn put_run(conn: &Connection, id: &str, run: Option<&RunState>, live: bool) -> Result<(), String> {
    let payload = run.map(serde_json::to_string).transpose().map_err(error)?;
    conn.execute(
        "INSERT INTO task_runs(task_id,payload) VALUES (?1,?2)
         ON CONFLICT(task_id) DO UPDATE SET payload=excluded.payload WHERE ?3",
        params![id, payload, live],
    )
    .map_err(error)?;
    Ok(())
}

#[derive(Clone, PartialEq)]
struct Progress {
    stamp: String,
    cursor: ArrayCursor,
    complete: bool,
}

fn progress(conn: &Connection) -> Result<Option<Progress>, String> {
    let row: Option<(String, String, bool)> = conn
        .query_row(
            "SELECT stamp,cursor,complete FROM migration WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(error)?;
    row.map(|(stamp, cursor, complete)| {
        Ok(Progress {
            stamp,
            cursor: serde_json::from_str(&cursor).map_err(error)?,
            complete,
        })
    })
    .transpose()
}

#[derive(Deserialize)]
struct LegacyTask {
    id: String,
    #[serde(flatten)]
    run: RunState,
}

fn read_migration_batch(
    source: &Path,
    previous: Option<&Progress>,
) -> Result<(Progress, Vec<LegacyTask>), ReadError> {
    let file = File::open(source)?;
    let stamp = source_stamp(source, &file)?;
    if previous.is_some_and(|row| row.stamp != stamp) {
        return Err(std::io::Error::other("迁移期间旧 Codex 唤醒任务文件发生变化").into());
    }
    let mut cursor = previous.map(|row| row.cursor.clone()).unwrap_or_default();
    let mut input = Input::new(file, cursor.offset, maintenance::BATCH_BYTES)?;
    let (rows, complete) = read_array_batch(
        &mut input,
        &mut cursor,
        "tasks",
        maintenance::BATCH_ROWS,
        maintenance::BATCH_TIME,
    )?;
    Ok((
        Progress {
            stamp,
            cursor,
            complete,
        },
        rows,
    ))
}

fn error(cause: impl std::fmt::Display) -> String {
    format!("读写 Codex 唤醒状态失败: {cause}")
}

fn legacy_error(cause: ReadError) -> String {
    match cause {
        ReadError::Budget => "旧 Codex 唤醒任务单条记录超出迁移预算".into(),
        ReadError::Format => "旧 Codex 唤醒任务格式无效，原文件已保留".into(),
        ReadError::Io(cause) => error(cause),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(path: &Path, count: usize) -> CodexWakeupState {
        let tasks = (0..count)
            .map(|index| {
                serde_json::json!({
                    "id": format!("task-{index:03}"), "name": format!("Task {index}"),
                    "enabled": true, "accountIds": ["account"], "createdAt": 1, "updatedAt": 2,
                    "schedule": {"kind": "interval", "weeklyDays": [], "intervalHours": 1},
                    "lastRunAt": 2, "lastStatus": "success", "lastSuccessCount": 1,
                })
            })
            .collect::<Vec<_>>();
        let state: CodexWakeupState =
            serde_json::from_value(serde_json::json!({ "enabled": true, "tasks": tasks })).unwrap();
        fs::write(path, serde_json::to_vec_pretty(&state).unwrap()).unwrap();
        state
    }

    #[test]
    fn codex_wakeup_migration_resumes_and_keeps_new_claim_and_edited_rules() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("tasks.json");
        let mut state = fixture(&legacy, 75);
        let original = fs::read(&legacy).unwrap();
        let store = RunStore::new(legacy.clone());
        assert!(!store.migrate_batch(Some(&legacy)).unwrap());
        let checkpoint = progress(&store.open().unwrap()).unwrap().unwrap();
        assert!(checkpoint.cursor.offset > 0);
        state.tasks[74].name = "Edited rule".into();
        state.tasks[74].last_run_at = Some(120);
        state.tasks[74].last_status = Some("pending_confirmation".into());
        store.save_rules(&state, &[]).unwrap();
        store.save_run(&state.tasks[74]).unwrap();

        let reopened = RunStore::new(legacy.clone());
        assert!(progress(&reopened.open().unwrap()).unwrap().unwrap() == checkpoint);
        while !reopened.migrate_batch(None).unwrap() {}
        let loaded = reopened.load().unwrap();
        assert_eq!(loaded.tasks.len(), 75);
        assert_eq!(loaded.tasks[0].last_run_at, Some(2));
        assert_eq!(loaded.tasks[74].name, "Edited rule");
        assert_eq!(loaded.tasks[74].last_run_at, Some(120));
        assert_eq!(
            loaded.tasks[74].last_status.as_deref(),
            Some("pending_confirmation")
        );
        reopened.save_run(&loaded.tasks[74]).unwrap();
        reopened.save_rules(&loaded, &[]).unwrap();
        assert_eq!(fs::read(&legacy).unwrap(), original);
        let rules = read_document(&reopened.rules_path()).unwrap().unwrap();
        assert!(rules
            .tasks
            .iter()
            .all(|task| task.last_run_at.is_none() && task.last_status.is_none()));
        fs::write(legacy, "damaged old file").unwrap();
        assert_eq!(reopened.load().unwrap().tasks[74].last_run_at, Some(120));
    }

    #[test]
    fn codex_wakeup_saved_rules_and_runs_load_when_pending_legacy_file_is_damaged() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("tasks.json");
        let mut state = fixture(&legacy, 75);
        let store = RunStore::new(legacy.clone());
        assert!(!store.migrate_batch(Some(&legacy)).unwrap());
        state.tasks[74].name = "Edited rule".into();
        store.save_rules(&state, &[]).unwrap();
        state.tasks[0].last_run_at = Some(120);
        store.save_run(&state.tasks[0]).unwrap();
        fs::write(&legacy, "damaged old file").unwrap();

        let reopened = RunStore::new(legacy.clone());
        assert!(reopened.migrate_batch(None).is_err());
        let loaded = reopened.load().unwrap();
        assert_eq!(loaded.tasks.len(), 75);
        assert_eq!(loaded.tasks[0].last_run_at, Some(120));
        assert_eq!(loaded.tasks[74].name, "Edited rule");
        assert_eq!(loaded.tasks[74].last_run_at, Some(2));
        assert_eq!(fs::read_to_string(legacy).unwrap(), "damaged old file");
    }

    #[test]
    fn codex_wakeup_deleted_and_recreated_id_cannot_inherit_an_old_claim() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("tasks.json");
        let mut old = fixture(&legacy, 40);
        let store = RunStore::new(legacy.clone());
        assert!(!store.migrate_batch(Some(&legacy)).unwrap());
        let mut replacement = old.tasks.pop().unwrap();
        let previous_claim = replacement.clone();
        store.save_rules(&old, &[]).unwrap();
        replacement.last_run_at = None;
        replacement.last_status = None;
        replacement.last_success_count = None;
        old.tasks.push(replacement);
        store
            .save_rules(&old, &[old.tasks.last().unwrap()])
            .unwrap();
        while !store.migrate_batch(None).unwrap() {}
        let mut reloaded = store.load().unwrap();
        assert!(reloaded.tasks.last().unwrap().last_run_at.is_none());
        assert!(task_state::claimed_task_mut(&mut reloaded, &previous_claim).is_none());
    }

    #[test]
    fn codex_wakeup_migration_reads_valid_backup_without_replacing_old_files() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("tasks.json");
        let backup = legacy.with_extension("json.bak");
        fixture(&backup, 40);
        fs::write(&legacy, "broken").unwrap();
        let store = RunStore::new(legacy.clone());
        let (source, _) = store.read_legacy().unwrap().unwrap();
        assert_eq!(source, backup);
        assert!(!store.migrate_batch(None).unwrap());
        let store = RunStore::new(legacy.clone());
        while !store.migrate_batch(None).unwrap() {}
        let state = store.load().unwrap();
        assert_eq!(state.tasks.len(), 40);
        assert_eq!(state.tasks[0].last_status.as_deref(), Some("success"));
        assert_eq!(fs::read_to_string(legacy).unwrap(), "broken");
    }
}
