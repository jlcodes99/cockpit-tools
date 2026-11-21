use crate::modules::config;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

const REPEAT_INTERVAL_SECONDS: i64 = 60;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ResumeTarget {
    #[default]
    RecentActivity,
    LastCreated,
    PreviousDesktopSession,
    Specific,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct ResumeSettings {
    pub enabled: bool,
    pub target: ResumeTarget,
    pub thread_id: Option<Uuid>,
    pub prompt: String,
}

impl ResumeSettings {
    pub(crate) fn uses_previous_desktop_session(&self) -> bool {
        self.enabled && self.target == ResumeTarget::PreviousDesktopSession
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.enabled && self.prompt.trim().is_empty() {
            return Err("CODEX_SESSION_RESUME_PROMPT_REQUIRED".into());
        }
        if self.enabled && self.target == ResumeTarget::Specific && self.thread_id.is_none() {
            return Err("CODEX_SESSION_RESUME_THREAD_REQUIRED".into());
        }
        Ok(())
    }
}

fn open() -> Result<Connection, String> {
    let db = Connection::open(database_path()?).map_err(|error| error.to_string())?;
    db.busy_timeout(Duration::from_secs(2))
        .map_err(|error| error.to_string())?;
    initialize(&db).map_err(|error| error.to_string())?;
    Ok(db)
}

fn database_path() -> Result<std::path::PathBuf, String> {
    Ok(config::get_data_dir()?.join("codex_session_resume.sqlite"))
}

fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS resume_settings (
            instance_id TEXT PRIMARY KEY NOT NULL,
            settings_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS resume_stopped_threads (
            instance_id TEXT PRIMARY KEY NOT NULL,
            thread_id TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS resume_attempts (
            instance_id TEXT NOT NULL,
            thread_id TEXT NOT NULL,
            launch_key TEXT NOT NULL,
            claimed_at INTEGER NOT NULL,
            outcome TEXT NOT NULL,
            PRIMARY KEY(instance_id, thread_id)
        );",
    )
}

fn read_settings(db: &Connection, instance: &str) -> Result<ResumeSettings, String> {
    let json: Option<String> = db
        .query_row(
            "SELECT settings_json FROM resume_settings WHERE instance_id = ?1",
            [instance],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    json.map(|value| serde_json::from_str(&value).map_err(|error| error.to_string()))
        .transpose()
        .map(|settings| settings.unwrap_or_default())
}

pub(crate) fn get(instance: &str) -> Result<ResumeSettings, String> {
    if !database_path()?
        .try_exists()
        .map_err(|error| error.to_string())?
    {
        return Ok(ResumeSettings::default());
    }
    read_settings(&open()?, instance)
}

pub(crate) fn save(instance: &str, settings: &ResumeSettings) -> Result<(), String> {
    settings.validate()?;
    let settings_json = serde_json::to_string(settings).map_err(|error| error.to_string())?;
    let mut db = open()?;
    let transaction = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "INSERT INTO resume_settings(instance_id, settings_json) VALUES (?1, ?2)
         ON CONFLICT(instance_id) DO UPDATE SET settings_json = excluded.settings_json",
            params![instance, settings_json],
        )
        .map_err(|error| error.to_string())?;
    if !settings.uses_previous_desktop_session() {
        transaction
            .execute(
                "DELETE FROM resume_stopped_threads WHERE instance_id = ?1",
                [instance],
            )
            .map_err(|error| error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())
}

/// A confirmed previous runtime with no identifiable selection replaces any
/// older selection. A stop with no previous process does not call this function.
pub(crate) fn record_stopped_thread(instance: &str, thread: Option<Uuid>) -> Result<(), String> {
    record_stopped_thread_in_db(&mut open()?, instance, thread)
}

fn record_stopped_thread_in_db(
    db: &mut Connection,
    instance: &str,
    thread: Option<Uuid>,
) -> Result<(), String> {
    let transaction = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    if !read_settings(&transaction, instance)?.uses_previous_desktop_session() {
        return Ok(());
    }
    if let Some(thread) = thread {
        transaction
            .execute(
                "INSERT INTO resume_stopped_threads(instance_id, thread_id) VALUES (?1, ?2)
             ON CONFLICT(instance_id) DO UPDATE SET thread_id = excluded.thread_id",
                params![instance, thread.to_string()],
            )
            .map_err(|error| error.to_string())?;
    } else {
        transaction
            .execute(
                "DELETE FROM resume_stopped_threads WHERE instance_id = ?1",
                [instance],
            )
            .map_err(|error| error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())
}

/// Consume only after the replacement desktop starts. A UI restart or repeated
/// startup callback must not replay a previously consumed selection.
pub(crate) fn take_stopped_thread(
    instance: &str,
    expected: &ResumeSettings,
) -> Result<Option<Uuid>, String> {
    take_stopped_thread_in_db(&mut open()?, instance, expected)
}

fn take_stopped_thread_in_db(
    db: &mut Connection,
    instance: &str,
    expected: &ResumeSettings,
) -> Result<Option<Uuid>, String> {
    let transaction = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    if !expected.uses_previous_desktop_session()
        || read_settings(&transaction, instance)? != *expected
    {
        return Ok(None);
    }
    let saved: Option<String> = transaction
        .query_row(
            "DELETE FROM resume_stopped_threads WHERE instance_id = ?1 RETURNING thread_id",
            [instance],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    saved
        .map(|raw| Uuid::parse_str(&raw).map_err(|error| error.to_string()))
        .transpose()
}

pub(crate) fn claim(
    instance: &str,
    thread: Uuid,
    launch: &str,
    expected: &ResumeSettings,
) -> Result<bool, String> {
    claim_in_db(
        &mut open()?,
        instance,
        thread,
        launch,
        expected,
        chrono::Utc::now().timestamp(),
    )
}

fn claim_in_db(
    db: &mut Connection,
    instance: &str,
    thread: Uuid,
    launch: &str,
    expected: &ResumeSettings,
    now: i64,
) -> Result<bool, String> {
    let transaction = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let saved = read_settings(&transaction, instance)?;
    if !saved.enabled || &saved != expected {
        return Ok(false);
    }
    // A timed-out send may have reached turn/start. Commit the claim before
    // crossing that boundary and keep the same-launch claim even after a timeout.
    let claimed = transaction
        .execute(
            "INSERT INTO resume_attempts(instance_id, thread_id, launch_key, claimed_at, outcome)
             SELECT ?1, ?2, ?3, ?4, 'uncertain'
             WHERE NOT EXISTS (
                 SELECT 1 FROM resume_attempts WHERE instance_id = ?1 AND launch_key = ?3
             )
             ON CONFLICT(instance_id, thread_id) DO UPDATE SET
                launch_key = excluded.launch_key, claimed_at = excluded.claimed_at,
                outcome = excluded.outcome
             WHERE resume_attempts.launch_key <> excluded.launch_key
                AND (resume_attempts.outcome NOT IN ('submitted', 'uncertain')
                     OR resume_attempts.claimed_at <= ?5)",
            params![
                instance,
                thread.to_string(),
                launch,
                now,
                now.saturating_sub(REPEAT_INTERVAL_SECONDS)
            ],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(claimed != 0)
}

pub(crate) fn settle(
    instance: &str,
    thread: Uuid,
    launch: &str,
    outcome: &str,
) -> Result<(), String> {
    open()?
        .execute(
            "UPDATE resume_attempts SET outcome = ?4
             WHERE instance_id = ?1 AND thread_id = ?2 AND launch_key = ?3",
            params![instance, thread.to_string(), launch, outcome],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claim_survives_database_reopen_and_checks_profile_settings() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("resume.sqlite");
        let settings = ResumeSettings {
            enabled: true,
            prompt: "Continue fixture".into(),
            ..Default::default()
        };
        let setup = Connection::open(&path).unwrap();
        initialize(&setup).unwrap();
        for instance in ["a", "b"] {
            setup
                .execute(
                    "INSERT INTO resume_settings VALUES(?1, ?2)",
                    params![instance, serde_json::to_string(&settings).unwrap()],
                )
                .unwrap();
        }
        let one = Uuid::new_v4();
        let two = Uuid::new_v4();
        let mut first = Connection::open(&path).unwrap();
        assert!(claim_in_db(&mut first, "a", one, "launch1", &settings, 100).unwrap());
        drop(first);
        let mut second = Connection::open(&path).unwrap();
        assert!(!claim_in_db(&mut second, "a", one, "launch2", &settings, 159).unwrap());
        assert!(!claim_in_db(&mut second, "a", one, "launch1", &settings, 160).unwrap());
        assert!(claim_in_db(&mut second, "b", one, "launch1", &settings, 100).unwrap());
        assert!(!claim_in_db(&mut second, "a", two, "launch1", &settings, 200).unwrap());
        assert!(claim_in_db(&mut second, "a", two, "launch3", &settings, 200).unwrap());
        assert!(claim_in_db(&mut second, "a", one, "launch2", &settings, 160).unwrap());
        let changed = ResumeSettings {
            prompt: "Changed".into(),
            ..settings.clone()
        };
        assert!(!claim_in_db(&mut second, "a", one, "launch3", &changed, 230).unwrap());
    }

    #[test]
    fn concurrent_dispatch_claims_and_definite_no_send() {
        use std::sync::{Arc, Barrier};

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("resume.sqlite");
        let setup = Connection::open(&path).unwrap();
        initialize(&setup).unwrap();
        let settings = ResumeSettings {
            enabled: true,
            prompt: "Continue".into(),
            ..Default::default()
        };
        setup
            .execute(
                "INSERT INTO resume_settings VALUES(?1, ?2)",
                params!["a", serde_json::to_string(&settings).unwrap()],
            )
            .unwrap();

        let barrier = Arc::new(Barrier::new(2));
        let thread = Uuid::new_v4();
        let candidates: Vec<_> = ["left", "right"]
            .into_iter()
            .map(|launch| {
                let barrier = Arc::clone(&barrier);
                let path = path.clone();
                let settings = settings.clone();
                std::thread::spawn(move || {
                    let mut db = Connection::open(path).unwrap();
                    db.busy_timeout(Duration::from_secs(2)).unwrap();
                    barrier.wait();
                    (
                        launch,
                        claim_in_db(&mut db, "a", thread, launch, &settings, 100).unwrap(),
                    )
                })
            })
            .collect();
        let winners: Vec<_> = candidates
            .into_iter()
            .filter_map(|worker| {
                let (launch, success) = worker.join().unwrap();
                success.then_some(launch)
            })
            .collect();
        assert_eq!(winners.len(), 1);
        setup
            .execute(
                "UPDATE resume_attempts SET outcome = 'busy' WHERE instance_id = 'a'",
                [],
            )
            .unwrap();
        let mut later = Connection::open(path).unwrap();
        assert!(!claim_in_db(&mut later, "a", thread, winners[0], &settings, 101).unwrap());
        assert!(claim_in_db(&mut later, "a", thread, "new-start", &settings, 101).unwrap());
    }

    #[test]
    fn previous_desktop_selection_is_scoped_consumed_and_replaced() {
        let mut db = Connection::open_in_memory().unwrap();
        initialize(&db).unwrap();
        let config = ResumeSettings {
            enabled: true,
            target: ResumeTarget::PreviousDesktopSession,
            prompt: "Continue".into(),
            ..Default::default()
        };
        db.execute(
            "INSERT INTO resume_settings VALUES(?1, ?2)",
            params!["one", serde_json::to_string(&config).unwrap()],
        )
        .unwrap();
        let id = Uuid::new_v4();
        record_stopped_thread_in_db(&mut db, "one", Some(id)).unwrap();
        assert_eq!(
            take_stopped_thread_in_db(&mut db, "other", &config).unwrap(),
            None
        );
        let different = ResumeSettings {
            prompt: "New".into(),
            ..config.clone()
        };
        assert_eq!(
            take_stopped_thread_in_db(&mut db, "one", &different).unwrap(),
            None
        );
        assert_eq!(
            take_stopped_thread_in_db(&mut db, "one", &config).unwrap(),
            Some(id)
        );
        assert_eq!(
            take_stopped_thread_in_db(&mut db, "one", &config).unwrap(),
            None
        );
        record_stopped_thread_in_db(&mut db, "one", Some(id)).unwrap();
        record_stopped_thread_in_db(&mut db, "one", None).unwrap();
        assert_eq!(
            take_stopped_thread_in_db(&mut db, "one", &config).unwrap(),
            None
        );
    }
}
