use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension, Transaction};

use serde::{Deserialize, Serialize};

use crate::{CodexTaskEvidence, ManagedCodexTask};

const DEFAULT_LIST_LIMIT: usize = 100;
const MAX_LIST_LIMIT: usize = 1_000;
const CURRENT_SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceCursor {
    pub observed_at: i64,
    pub id: String,
}

#[derive(Debug, Clone)]
pub struct CodexTaskStore {
    path: PathBuf,
}

impl CodexTaskStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn initialize(&self) -> Result<(), String> {
        let connection = self.open()?;
        initialize_schema(&connection)
    }

    pub fn save_task(&self, task: &ManagedCodexTask) -> Result<(), String> {
        let connection = self.open()?;
        initialize_schema(&connection)?;
        upsert_task(&connection, task)
    }

    pub fn save_transition(
        &self,
        task: &ManagedCodexTask,
        evidence: &CodexTaskEvidence,
    ) -> Result<(), String> {
        let mut connection = self.open()?;
        initialize_schema(&connection)?;
        let transaction = connection
            .transaction()
            .map_err(|error| format!("start Codex supervisor transaction failed: {error}"))?;
        upsert_task_in_transaction(&transaction, task)?;
        insert_evidence_in_transaction(&transaction, &task.id, evidence)?;
        transaction
            .commit()
            .map_err(|error| format!("commit Codex supervisor transition failed: {error}"))
    }

    pub fn get_task(&self, task_id: &str) -> Result<Option<ManagedCodexTask>, String> {
        let connection = self.open()?;
        initialize_schema(&connection)?;
        let payload = connection
            .query_row(
                "SELECT payload_json FROM codex_managed_tasks WHERE id = ?1",
                [task_id.trim()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("read managed Codex task failed: {error}"))?;
        payload
            .map(|payload| deserialize_task(&payload))
            .transpose()
    }

    pub fn list_tasks(&self, limit: Option<usize>) -> Result<Vec<ManagedCodexTask>, String> {
        let connection = self.open()?;
        initialize_schema(&connection)?;
        let limit = limit.unwrap_or(DEFAULT_LIST_LIMIT).clamp(1, MAX_LIST_LIMIT) as i64;
        let mut statement = connection
            .prepare(
                "SELECT payload_json FROM codex_managed_tasks ORDER BY updated_at DESC LIMIT ?1",
            )
            .map_err(|error| format!("prepare managed Codex task list failed: {error}"))?;
        let rows = statement
            .query_map([limit], |row| row.get::<_, String>(0))
            .map_err(|error| format!("query managed Codex task list failed: {error}"))?;

        let mut tasks = Vec::new();
        for row in rows {
            let payload =
                row.map_err(|error| format!("read managed Codex task row failed: {error}"))?;
            tasks.push(deserialize_task(&payload)?);
        }
        Ok(tasks)
    }

    pub fn list_evidence(
        &self,
        task_id: &str,
        limit: Option<usize>,
    ) -> Result<Vec<CodexTaskEvidence>, String> {
        self.list_evidence_page(task_id, None, limit)
    }

    pub fn list_evidence_page(
        &self,
        task_id: &str,
        cursor: Option<&EvidenceCursor>,
        limit: Option<usize>,
    ) -> Result<Vec<CodexTaskEvidence>, String> {
        let connection = self.open()?;
        initialize_schema(&connection)?;
        let limit = limit.unwrap_or(DEFAULT_LIST_LIMIT).clamp(1, MAX_LIST_LIMIT) as i64;
        let (query, cursor_at, cursor_id) = match cursor {
            Some(cursor) => (
                "SELECT payload_json FROM codex_task_evidence \
                 WHERE task_id = ?1 AND (observed_at < ?2 OR (observed_at = ?2 AND id < ?3)) \
                 ORDER BY observed_at DESC, id DESC LIMIT ?4",
                cursor.observed_at,
                cursor.id.trim(),
            ),
            None => (
                "SELECT payload_json FROM codex_task_evidence \
                 WHERE task_id = ?1 ORDER BY observed_at DESC, id DESC LIMIT ?4",
                i64::MAX,
                "",
            ),
        };
        let mut statement = connection
            .prepare(query)
            .map_err(|error| format!("prepare Codex task evidence list failed: {error}"))?;
        let rows = statement
            .query_map(
                params![task_id.trim(), cursor_at, cursor_id, limit],
                |row| row.get::<_, String>(0),
            )
            .map_err(|error| format!("query Codex task evidence failed: {error}"))?;

        let mut evidence = Vec::new();
        for row in rows {
            let payload =
                row.map_err(|error| format!("read Codex task evidence row failed: {error}"))?;
            evidence.push(
                serde_json::from_str(&payload)
                    .map_err(|error| format!("decode Codex task evidence failed: {error}"))?,
            );
        }
        evidence.reverse();
        Ok(evidence)
    }

    fn open(&self) -> Result<Connection, String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "create Codex supervisor data directory failed ({}): {error}",
                    parent.display()
                )
            })?;
        }
        Connection::open(&self.path).map_err(|error| {
            format!(
                "open Codex supervisor database failed ({}): {error}",
                self.path.display()
            )
        })
    }
}

fn initialize_schema(connection: &Connection) -> Result<(), String> {
    let version = connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .map_err(|error| format!("read Codex supervisor schema version failed: {error}"))?;
    if version > CURRENT_SCHEMA_VERSION {
        return Err(format!(
            "Codex supervisor database schema {version} is newer than supported schema {CURRENT_SCHEMA_VERSION}"
        ));
    }
    connection
        .execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             BEGIN IMMEDIATE;
             CREATE TABLE IF NOT EXISTS codex_managed_tasks (
                 id TEXT PRIMARY KEY,
                 status TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 payload_json TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_codex_managed_tasks_updated_at
                 ON codex_managed_tasks(updated_at DESC);
             CREATE TABLE IF NOT EXISTS codex_task_evidence (
                 id TEXT PRIMARY KEY,
                 task_id TEXT NOT NULL,
                 observed_at INTEGER NOT NULL,
                 source TEXT NOT NULL,
                 kind TEXT NOT NULL,
                 confidence TEXT NOT NULL,
                 payload_json TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_codex_task_evidence_task_time
                 ON codex_task_evidence(task_id, observed_at DESC, id DESC);
             PRAGMA user_version = 1;
             COMMIT;",
        )
        .map_err(|error| {
            format!("migrate Codex supervisor database from schema {version} failed: {error}")
        })
}

fn upsert_task(connection: &Connection, task: &ManagedCodexTask) -> Result<(), String> {
    let payload = serde_json::to_string(task)
        .map_err(|error| format!("encode managed Codex task failed: {error}"))?;
    connection
        .execute(
            "INSERT INTO codex_managed_tasks (id, status, created_at, updated_at, payload_json)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
                 status = excluded.status,
                 updated_at = excluded.updated_at,
                 payload_json = excluded.payload_json",
            params![
                task.id,
                status_text(task)?,
                task.created_at,
                task.updated_at,
                payload
            ],
        )
        .map(|_| ())
        .map_err(|error| format!("save managed Codex task failed: {error}"))
}

fn upsert_task_in_transaction(
    transaction: &Transaction<'_>,
    task: &ManagedCodexTask,
) -> Result<(), String> {
    let payload = serde_json::to_string(task)
        .map_err(|error| format!("encode managed Codex task failed: {error}"))?;
    transaction
        .execute(
            "INSERT INTO codex_managed_tasks (id, status, created_at, updated_at, payload_json)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
                 status = excluded.status,
                 updated_at = excluded.updated_at,
                 payload_json = excluded.payload_json",
            params![
                task.id,
                status_text(task)?,
                task.created_at,
                task.updated_at,
                payload
            ],
        )
        .map(|_| ())
        .map_err(|error| format!("save managed Codex task transition failed: {error}"))
}

fn insert_evidence_in_transaction(
    transaction: &Transaction<'_>,
    task_id: &str,
    evidence: &CodexTaskEvidence,
) -> Result<(), String> {
    let payload = serde_json::to_string(evidence)
        .map_err(|error| format!("encode Codex task evidence failed: {error}"))?;
    transaction
        .execute(
            "INSERT OR IGNORE INTO codex_task_evidence
             (id, task_id, observed_at, source, kind, confidence, payload_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                evidence.id,
                task_id,
                evidence.observed_at,
                enum_text(&evidence.source)?,
                enum_text(&evidence.kind)?,
                enum_text(&evidence.confidence)?,
                payload
            ],
        )
        .map(|_| ())
        .map_err(|error| format!("save Codex task evidence failed: {error}"))
}

fn deserialize_task(payload: &str) -> Result<ManagedCodexTask, String> {
    serde_json::from_str(payload)
        .map_err(|error| format!("decode managed Codex task failed: {error}"))
}

fn status_text(task: &ManagedCodexTask) -> Result<String, String> {
    enum_text(&task.status)
}

fn enum_text<T: serde::Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_value(value)
        .map_err(|error| format!("encode Codex supervisor enum failed: {error}"))?
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| "Codex supervisor enum did not serialize as a string".to_string())
}

#[cfg(test)]
mod codex_task_store_tests {
    use serde_json::json;

    use super::*;
    use crate::{
        classify_codex_event, CodexEventSource, ManagedCodexAccountScope, ManagedCodexTaskConfig,
        ManagedCodexTaskStatus,
    };

    fn test_store_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "cockpit-codex-task-store-{}.sqlite3",
            uuid::Uuid::new_v4()
        ))
    }

    fn cleanup_store(path: &Path) {
        for suffix in ["", "-wal", "-shm"] {
            let candidate = PathBuf::from(format!("{}{}", path.display(), suffix));
            let _ = std::fs::remove_file(candidate);
        }
    }

    fn test_task(objective: &str) -> ManagedCodexTask {
        ManagedCodexTask::create(ManagedCodexTaskConfig {
            objective: objective.to_string(),
            cwd: "C:/workspace".to_string(),
            account_scope: ManagedCodexAccountScope::Selected {
                account_ids: vec!["account-a".to_string(), "account-b".to_string()],
            },
            initial_account_id: Some("account-a".to_string()),
            model: None,
            reasoning_effort: None,
            max_switches: None,
        })
        .expect("create task")
    }

    #[test]
    fn persists_task_and_evidence_atomically() {
        let path = test_store_path();
        let store = CodexTaskStore::new(&path);
        let mut task = test_task("finish work");
        task.mark_preparing("account-a").expect("prepare task");
        store.save_task(&task).expect("save task");

        let evidence = classify_codex_event(
            CodexEventSource::Proxy,
            &json!({
                "status": 429,
                "error": { "type": "usage_limit_reached" }
            }),
        );
        task.apply_evidence(&evidence);
        store
            .save_transition(&task, &evidence)
            .expect("save transition");

        let loaded = store
            .get_task(&task.id)
            .expect("load task")
            .expect("task exists");
        assert_eq!(loaded.status, ManagedCodexTaskStatus::Draining);
        assert_eq!(store.list_tasks(None).expect("list tasks").len(), 1);
        let stored_evidence = store.list_evidence(&task.id, None).expect("list evidence");
        assert_eq!(stored_evidence, vec![evidence]);

        cleanup_store(&path);
    }

    #[test]
    fn initializes_reopens_and_rejects_future_schema_versions() {
        let path = test_store_path();
        let store = CodexTaskStore::new(&path);
        store.initialize().expect("initialize schema");
        store.initialize().expect("reopen current schema");
        let connection = Connection::open(&path).expect("open test database");
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("read schema version");
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
        connection
            .execute_batch("PRAGMA user_version = 99;")
            .expect("set future schema");
        drop(connection);
        assert!(store
            .initialize()
            .expect_err("future schema must be rejected")
            .contains("newer than supported"));
        cleanup_store(&path);
    }

    #[test]
    fn returns_latest_evidence_page_in_stable_chronological_order() {
        let path = test_store_path();
        let store = CodexTaskStore::new(&path);
        let task = test_task("page evidence");
        store.save_task(&task).expect("save task");
        for index in 1..=5_i64 {
            let mut item =
                classify_codex_event(CodexEventSource::Proxy, &json!({ "type": "activity" }))
                    .with_id(format!("event-{index}"));
            item.observed_at = 1_000 + index;
            store.save_transition(&task, &item).expect("save evidence");
        }

        let latest = store
            .list_evidence_page(&task.id, None, Some(2))
            .expect("latest page");
        assert_eq!(
            latest
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["event-4", "event-5"]
        );
        let cursor = EvidenceCursor {
            observed_at: latest[0].observed_at,
            id: latest[0].id.clone(),
        };
        let older = store
            .list_evidence_page(&task.id, Some(&cursor), Some(2))
            .expect("older page");
        assert_eq!(
            older
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["event-2", "event-3"]
        );
        cleanup_store(&path);
    }

    #[test]
    fn preserves_fifo_creation_order_across_store_reopen() {
        let path = test_store_path();
        let store = CodexTaskStore::new(&path);
        for (index, objective) in ["first", "second", "third"].into_iter().enumerate() {
            let mut task = test_task(objective);
            task.created_at = 10_000 + index as i64;
            task.updated_at = 20_000 - index as i64;
            store.save_task(&task).expect("save queued task");
        }
        drop(store);

        let reopened = CodexTaskStore::new(&path);
        let mut tasks = reopened.list_tasks(None).expect("reload queued tasks");
        tasks.sort_by_key(|task| (task.created_at, task.id.clone()));
        assert_eq!(
            tasks
                .iter()
                .map(|task| task.config.objective.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "second", "third"]
        );
        cleanup_store(&path);
    }

    #[test]
    fn typed_store_does_not_persist_unmodeled_credentials_or_transcripts() {
        let path = test_store_path();
        let store = CodexTaskStore::new(&path);
        let task = test_task("privacy contract");
        store.save_task(&task).expect("save task");
        let database_bytes = std::fs::read(&path).expect("read database");
        let database = String::from_utf8_lossy(&database_bytes);
        assert!(!database.contains("secret-access-token-for-test"));
        assert!(!database.contains("refresh_token"));
        assert!(!database.contains("auth.json"));
        assert!(!database.contains("assistant transcript body"));
        cleanup_store(&path);
    }
}
