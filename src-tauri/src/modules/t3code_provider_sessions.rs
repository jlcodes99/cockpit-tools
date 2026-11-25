//! Read-only access to T3 Code's indexed session projection and command receipts.

use std::path::Path;
use std::time::Duration;

use rusqlite::{params, Connection, OpenFlags, OptionalExtension};

pub(super) fn open(state_dir: &Path) -> rusqlite::Result<Connection> {
    let connection = Connection::open_with_flags(
        state_dir.join("statev2.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(Duration::from_millis(100))?;
    Ok(connection)
}

pub(super) fn attached_sessions(
    db: &Connection,
    instance_id: &str,
    driver: &str,
) -> rusqlite::Result<Vec<(String, String)>> {
    let mut statement = db.prepare(
        "SELECT s.provider_session_id, b.thread_id
         FROM orchestration_v2_projection_provider_sessions AS s
         JOIN orchestration_v2_projection_provider_session_bindings AS b
           ON b.provider_session_id = s.provider_session_id
         WHERE s.provider_instance_id = ?1 AND s.driver = ?2
           AND s.status IN ('starting', 'ready', 'running', 'waiting')",
    )?;
    let rows = statement.query_map(params![instance_id, driver], |row| {
        Ok((row.get(0)?, row.get(1)?))
    })?;
    rows.collect()
}

pub(super) fn has_active_runs(db: &Connection, instance_id: &str) -> rusqlite::Result<bool> {
    // Include the recovery index predicate so SQLite can use the partial index.
    db.query_row(
        "SELECT EXISTS (
            SELECT 1 FROM orchestration_v2_projection_runs
            WHERE provider_instance_id = ?1
              AND status IN ('queued', 'preparing', 'starting', 'running', 'waiting')
              AND status <> 'queued'
        )",
        [instance_id],
        |row| row.get(0),
    )
}

pub(super) fn detach_receipts(
    db: &Connection,
    effects: &[String],
) -> rusqlite::Result<Vec<Option<String>>> {
    let mut statement =
        db.prepare("SELECT status FROM orchestration_v2_effect_outbox WHERE effect_id = ?1")?;
    effects
        .iter()
        .map(|effect| statement.query_row([effect], |row| row.get(0)).optional())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detaches_only_selected_sessions_and_waits_for_active_runs() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE orchestration_v2_projection_provider_sessions (
                 provider_session_id TEXT PRIMARY KEY, provider_instance_id TEXT, driver TEXT, status TEXT);
             CREATE TABLE orchestration_v2_projection_provider_session_bindings (
                 provider_session_id TEXT, thread_id TEXT);
             CREATE TABLE orchestration_v2_projection_runs (
                 run_id TEXT PRIMARY KEY, thread_id TEXT, ordinal INTEGER, status TEXT, provider_instance_id TEXT);
             INSERT INTO orchestration_v2_projection_provider_sessions VALUES
                 ('selected-session', 'work', 'codex', 'ready'),
                 ('other-session', 'personal', 'codex', 'ready'),
                 ('past-session', 'work', 'codex', 'stopped'),
                 ('detached-idle', 'work', 'codex', 'ready');
             INSERT INTO orchestration_v2_projection_provider_session_bindings VALUES
                 ('selected-session', 'mixed'), ('selected-session', 'only-work'),
                 ('other-session', 'mixed'), ('past-session', 'past');
             INSERT INTO orchestration_v2_projection_runs VALUES
                 ('mixed-run', 'mixed', 1, 'completed', 'work'),
                 ('queued-run', 'only-work', 1, 'queued', 'work'),
                 ('other-run', 'mixed', 2, 'running', 'personal');",
        )
        .unwrap();

        let mut attached = attached_sessions(&db, "work", "codex").unwrap();
        attached.sort();
        assert_eq!(
            attached,
            vec![
                ("selected-session".into(), "mixed".into()),
                ("selected-session".into(), "only-work".into()),
            ]
        );
        assert!(attached_sessions(&db, "work", "claudeAgent")
            .unwrap()
            .is_empty());
        assert!(!has_active_runs(&db, "work").unwrap());
        assert!(has_active_runs(&db, "personal").unwrap());
        db.execute(
            "UPDATE orchestration_v2_projection_runs SET status = 'preparing'
             WHERE run_id = 'queued-run'",
            [],
        )
        .unwrap();
        assert!(has_active_runs(&db, "work").unwrap());
    }
}
