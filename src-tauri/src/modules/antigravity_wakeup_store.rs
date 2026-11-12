use super::{PersistedWakeupState, ResetState, WakeupTaskInput};
use crate::modules::{
    atomic_write, background_maintenance as maintenance,
    legacy_json_input::{read_array_batch, ArrayCursor, Input, ReadError},
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::ErrorKind,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone)]
pub(super) struct WakeupStore {
    legacy: PathBuf,
}

#[derive(Clone, Default)]
pub(super) struct Runtime {
    pub last_run_at: Option<i64>,
    pub last_claim_at: Option<i64>,
    pub reset: ResetState,
}

impl WakeupStore {
    pub(super) fn new(legacy: PathBuf) -> Self {
        Self { legacy }
    }

    fn rules_path(&self) -> PathBuf {
        self.legacy.with_extension("rules.json")
    }

    fn open(&self) -> Result<Connection, String> {
        let conn = Connection::open(
            self.legacy
                .with_file_name("antigravity_wakeup_state.sqlite"),
        )
        .map_err(error)?;
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
                 CREATE TABLE IF NOT EXISTS task_runtime (
                    task_id TEXT PRIMARY KEY, last_run_at INTEGER, last_claim_at INTEGER, reset TEXT
                 );
                 CREATE TABLE IF NOT EXISTS migration (
                    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
                    stamp TEXT NOT NULL, cursor TEXT NOT NULL, complete INTEGER NOT NULL
                 );
                 PRAGMA user_version=1;
                 COMMIT;",
                )
                .map_err(error)?,
            _ => return Err("Antigravity 唤醒状态数据库版本不受支持".into()),
        }
        Ok(conn)
    }

    pub(super) fn load(
        &self,
    ) -> Result<(Option<PersistedWakeupState>, HashMap<String, Runtime>), String> {
        let mut conn = self.open()?;
        let complete = progress(&conn)?.is_some_and(|progress| progress.complete);
        let legacy = if complete {
            None
        } else {
            read_document_with_backup(&self.legacy)?
        };
        let rules = read_document_with_backup(&self.rules_path())?;
        let Some(mut config) = rules
            .map(|(_, config)| config)
            .or_else(|| legacy.as_ref().map(|(_, config)| config.clone()))
        else {
            return Ok((None, HashMap::new()));
        };
        let legacy_runs: HashMap<&str, i64> = legacy
            .as_ref()
            .map(|(_, config)| {
                config
                    .tasks
                    .iter()
                    .filter_map(|task| task.last_run_at.map(|run| (task.id.as_str(), run)))
                    .collect()
            })
            .unwrap_or_default();
        let tx = conn.transaction().map_err(error)?;
        let mut runtime = HashMap::with_capacity(config.tasks.len());
        for task in &mut config.tasks {
            let row = load_runtime(&tx, &task.id)?.unwrap_or_else(|| Runtime {
                last_run_at: legacy_runs.get(task.id.as_str()).copied(),
                ..Runtime::default()
            });
            task.last_run_at = row.last_run_at;
            runtime.insert(task.id.clone(), row);
        }
        tx.commit().map_err(error)?;
        if let Some((source, _)) = legacy {
            let store = self.clone();
            maintenance::schedule(
                format!("antigravity-wakeup-state:{}", self.legacy.display()),
                move || store.migrate_batch(&source),
            );
        }
        Ok((Some(config), runtime))
    }

    pub(super) fn save_rules(
        &self,
        config: &PersistedWakeupState,
        new_tasks: &[&WakeupTaskInput],
    ) -> Result<(), String> {
        if !new_tasks.is_empty() {
            let mut conn = self.open()?;
            for chunk in new_tasks.chunks(maintenance::BATCH_ROWS) {
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(error)?;
                for task in chunk {
                    tx.execute(
                        "INSERT INTO task_runtime(task_id,last_run_at) VALUES (?1,?2)
                         ON CONFLICT(task_id) DO UPDATE SET last_run_at=excluded.last_run_at,
                            last_claim_at=NULL,reset=NULL",
                        params![task.id, task.last_run_at],
                    )
                    .map_err(error)?;
                }
                tx.commit().map_err(error)?;
            }
        }
        atomic_write::write_string_atomic(&self.rules_path(), &rules_json(config)?)
    }

    pub(super) fn seed_latest_run(&self, task: &WakeupTaskInput) -> Result<(), String> {
        let Some(last_run) = task.last_run_at else {
            return Ok(());
        };
        self.open()?
            .execute(
                "INSERT INTO task_runtime(task_id,last_run_at) VALUES (?1,?2)
             ON CONFLICT(task_id) DO UPDATE SET last_run_at=excluded.last_run_at
             WHERE last_run_at IS NULL OR last_run_at < excluded.last_run_at",
                params![task.id, last_run],
            )
            .map_err(error)?;
        Ok(())
    }

    pub(super) fn claim(
        &self,
        id: &str,
        previous_run: Option<i64>,
        due_at: Option<i64>,
        now: i64,
        reset: Option<&ResetState>,
        models: &mut Vec<String>,
    ) -> Result<bool, String> {
        let mut conn = self.open()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(error)?;
        let stored = load_runtime(&tx, id)?.unwrap_or_default();
        let last_run = stored.last_run_at.max(previous_run);
        if due_at.is_some_and(|due| {
            stored
                .last_claim_at
                .max(last_run)
                .is_some_and(|at| at >= due)
        }) {
            return Ok(false);
        }
        let reset = if let Some(candidate) = reset {
            let mut merged = stored.reset;
            models.retain(|model| {
                let Some(window) = candidate.last_reset_trigger_timestamps.get(model) else {
                    return false;
                };
                if merged.last_reset_trigger_timestamps.get(model) == Some(window) {
                    return false;
                }
                merged
                    .last_reset_trigger_timestamps
                    .insert(model.clone(), window.clone());
                if let Some(at) = candidate.last_reset_trigger_at.get(model) {
                    merged.last_reset_trigger_at.insert(model.clone(), *at);
                }
                if let Some(remaining) = candidate.last_reset_remaining.get(model) {
                    merged
                        .last_reset_remaining
                        .insert(model.clone(), *remaining);
                }
                true
            });
            if models.is_empty() {
                return Ok(false);
            }
            Some(serde_json::to_string(&merged).map_err(error)?)
        } else {
            None
        };
        tx.execute(
            "INSERT INTO task_runtime(task_id,last_run_at,last_claim_at,reset) VALUES (?1,?2,?3,?4)
             ON CONFLICT(task_id) DO UPDATE SET last_run_at=excluded.last_run_at,
                last_claim_at=excluded.last_claim_at,reset=COALESCE(excluded.reset,task_runtime.reset)",
            params![id, last_run, now, reset],
        ).map_err(error)?;
        tx.commit().map_err(error)?;
        Ok(true)
    }

    pub(super) fn finish(
        &self,
        id: &str,
        claimed_at: i64,
        completed_at: i64,
    ) -> Result<bool, String> {
        Ok(self
            .open()?
            .execute(
                "UPDATE task_runtime SET last_run_at=?3 WHERE task_id=?1 AND last_claim_at=?2",
                params![id, claimed_at, completed_at],
            )
            .map_err(error)?
            != 0)
    }

    fn migrate_batch(&self, source: &Path) -> Result<bool, String> {
        let mut conn = self.open()?;
        let before = progress(&conn)?;
        if before.as_ref().is_some_and(|row| row.complete) {
            return Ok(true);
        }
        let file = File::open(source).map_err(error)?;
        let stamp = source_stamp(source, &file)?;
        if before.as_ref().is_some_and(|row| row.stamp != stamp) {
            return Err("迁移期间旧 Antigravity 唤醒配置发生变化".into());
        }
        let mut next = before.clone().unwrap_or(Progress {
            stamp,
            cursor: ArrayCursor::default(),
            complete: false,
        });
        let mut input =
            Input::new(file, next.cursor.offset, maintenance::BATCH_BYTES).map_err(error)?;
        let (rows, complete) = read_array_batch::<WakeupTaskInput>(
            &mut input,
            &mut next.cursor,
            "tasks",
            maintenance::BATCH_ROWS,
            maintenance::BATCH_TIME,
        )
        .map_err(legacy_error)?;
        next.complete = complete;
        if complete && !self.rules_path().try_exists().map_err(error)? {
            let file = File::open(source).map_err(error)?;
            if source_stamp(source, &file)? != next.stamp {
                return Err("迁移期间旧 Antigravity 唤醒配置发生变化".into());
            }
            let config = serde_json::from_reader(std::io::BufReader::new(file)).map_err(error)?;
            let content = rules_json(&config)?;
            atomic_write::replace_secret_string_if_matches(
                &self.rules_path(),
                None,
                Some(&content),
            )?;
        }
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(error)?;
        if progress(&tx)? != before {
            return Ok(false);
        }
        for task in rows {
            if let Some(last_run) = task.last_run_at {
                tx.execute(
                    "INSERT OR IGNORE INTO task_runtime(task_id,last_run_at) VALUES (?1,?2)",
                    params![task.id, last_run],
                )
                .map_err(error)?;
            }
        }
        tx.execute(
            "INSERT INTO migration(singleton,stamp,cursor,complete) VALUES (1,?1,?2,?3)
             ON CONFLICT(singleton) DO UPDATE SET stamp=excluded.stamp,cursor=excluded.cursor,
                 complete=excluded.complete",
            params![
                next.stamp,
                serde_json::to_string(&next.cursor).map_err(error)?,
                next.complete
            ],
        )
        .map_err(error)?;
        tx.commit().map_err(error)?;
        Ok(complete)
    }
}

fn load_runtime(conn: &Connection, id: &str) -> Result<Option<Runtime>, String> {
    let row: Option<(Option<i64>, Option<i64>, Option<String>)> = conn
        .query_row(
            "SELECT last_run_at,last_claim_at,reset FROM task_runtime WHERE task_id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(error)?;
    row.map(|(last_run_at, last_claim_at, reset)| {
        Ok(Runtime {
            last_run_at,
            last_claim_at,
            reset: reset
                .map(|value| serde_json::from_str(&value))
                .transpose()
                .map_err(error)?
                .unwrap_or_default(),
        })
    })
    .transpose()
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

fn rules_json(config: &PersistedWakeupState) -> Result<String, String> {
    let mut rules = config.clone();
    for task in &mut rules.tasks {
        task.last_run_at = None;
    }
    serde_json::to_string_pretty(&rules).map_err(error)
}

fn read_document_with_backup(
    path: &Path,
) -> Result<Option<(PathBuf, PersistedWakeupState)>, String> {
    match read_document(path) {
        Ok(state) => Ok(state.map(|state| (path.to_path_buf(), state))),
        Err(original) => {
            let backup = path.with_extension("json.bak");
            match read_document(&backup) {
                Ok(Some(state)) => Ok(Some((backup, state))),
                _ => Err(original),
            }
        }
    }
}

fn read_document(path: &Path) -> Result<Option<PersistedWakeupState>, String> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(cause) if cause.kind() == ErrorKind::NotFound => return Ok(None),
        Err(cause) => return Err(error(cause)),
    };
    if content.trim().is_empty() {
        return Ok(None);
    }
    serde_json::from_str(&content).map(Some).map_err(error)
}

fn source_stamp(path: &Path, file: &File) -> Result<String, String> {
    let metadata = file.metadata().map_err(error)?;
    Ok(format!(
        "{}:{}:{:?}",
        path.display(),
        metadata.len(),
        metadata.modified().map_err(error)?
    ))
}

fn error(cause: impl std::fmt::Display) -> String {
    format!("读写 Antigravity 唤醒状态失败: {cause}")
}

fn legacy_error(cause: ReadError) -> String {
    match cause {
        ReadError::Budget => "旧 Antigravity 唤醒任务单条记录超出迁移预算".into(),
        ReadError::Format => "旧 Antigravity 唤醒任务格式无效，原文件已保留".into(),
        ReadError::Io(cause) => error(cause),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(path: &Path, count: usize) -> PersistedWakeupState {
        let tasks = (0..count).map(|index| serde_json::json!({
            "id": format!("task-{index:03}"), "name": "Native rule", "enabled": true,
            "lastRunAt": 100,
            "schedule": { "repeatMode": "daily", "selectedModels": ["model"], "selectedAccounts": ["test@example.org"] },
        })).collect::<Vec<_>>();
        let config: PersistedWakeupState =
            serde_json::from_value(serde_json::json!({"enabled": true, "tasks": tasks})).unwrap();
        fs::write(path, serde_json::to_vec(&config).unwrap()).unwrap();
        config
    }

    #[test]
    fn antigravity_wakeup_webview_seed_cannot_replace_native_rules_or_restore_deleted_tasks() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("tasks.json");
        fixture(&legacy, 1);
        let source = fs::read(&legacy).unwrap();
        let mut old_webview = fixture(&temp.path().join("webview.json"), 2);
        old_webview.enabled = false;
        old_webview.tasks[0].name = "Stale WebView rule".into();
        old_webview.tasks[0].created_at = 25;
        old_webview.tasks[0].last_run_at = Some(150);
        let store = WakeupStore::new(legacy.clone());
        super::super::seed_webview(&store, old_webview.clone()).unwrap();
        old_webview.tasks[0].last_run_at = Some(300);
        super::super::seed_webview(&store, old_webview).unwrap();
        let (config, _) = store.load().unwrap();
        let config = config.unwrap();
        assert!(config.enabled);
        assert_eq!(config.tasks.len(), 1);
        assert_eq!(config.tasks[0].name, "Native rule");
        assert_eq!(config.tasks[0].created_at, 25);
        assert_eq!(config.tasks[0].last_run_at, Some(150));
        assert_eq!(fs::read(legacy).unwrap(), source);
    }

    #[test]
    fn antigravity_wakeup_legacy_resume_keeps_new_claim_and_prevents_replay() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("tasks.json");
        let mut rules = fixture(&legacy, 75);
        let source = fs::read(&legacy).unwrap();
        let store = WakeupStore::new(legacy.clone());
        assert!(!store.migrate_batch(&legacy).unwrap());
        let checkpoint = progress(&store.open().unwrap()).unwrap().unwrap();
        assert!(checkpoint.cursor.offset > 0);
        rules.tasks[74].name = "Edited rule".into();
        store.save_rules(&rules, &[]).unwrap();
        let mut models = vec!["model".to_string()];
        assert!(store
            .claim("task-074", Some(100), Some(150), 170, None, &mut models)
            .unwrap());

        let reopened = WakeupStore::new(legacy.clone());
        assert!(progress(&reopened.open().unwrap()).unwrap().unwrap() == checkpoint);
        while !reopened.migrate_batch(&legacy).unwrap() {}
        assert!(!reopened
            .claim("task-074", None, Some(150), 180, None, &mut models)
            .unwrap());
        assert!(reopened.finish("task-074", 170, 190).unwrap());
        let (config, rows) = reopened.load().unwrap();
        let config = config.unwrap();
        assert_eq!(config.tasks[0].last_run_at, Some(100));
        assert_eq!(config.tasks[74].name, "Edited rule");
        assert_eq!(config.tasks[74].last_run_at, Some(190));
        assert_eq!(rows["task-074"].last_claim_at, Some(170));
        assert_eq!(fs::read(legacy).unwrap(), source);
    }

    #[test]
    fn antigravity_wakeup_quota_window_is_claimed_once_and_deleted_task_state_is_retired() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("tasks.json");
        let mut rules = fixture(&legacy, 1);
        let store = WakeupStore::new(legacy.clone());
        while !store.migrate_batch(&legacy).unwrap() {}
        let mut reset = ResetState::default();
        reset
            .last_reset_trigger_timestamps
            .insert("model".into(), "window-1".into());
        reset.last_reset_trigger_at.insert("model".into(), 100);
        assert!(store
            .claim(
                "task-000",
                Some(100),
                None,
                150,
                Some(&reset),
                &mut vec!["model".into()]
            )
            .unwrap());
        let reopened = WakeupStore::new(legacy);
        assert!(!reopened
            .claim(
                "task-000",
                None,
                None,
                200,
                Some(&reset),
                &mut vec!["model".into()]
            )
            .unwrap());

        let mut readded = rules.tasks.pop().unwrap();
        readded.last_run_at = None;
        reopened.save_rules(&rules, &[]).unwrap();
        rules.tasks.push(readded);
        reopened.save_rules(&rules, &[&rules.tasks[0]]).unwrap();
        assert!(!reopened.finish("task-000", 150, 250).unwrap());
        let (_, rows) = reopened.load().unwrap();
        assert!(rows["task-000"].last_run_at.is_none());
        assert!(rows["task-000"].reset.last_reset_trigger_at.is_empty());
    }
}
