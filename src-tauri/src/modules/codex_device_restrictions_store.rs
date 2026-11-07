//! Machine-local observations, never account credentials or official device slots.
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::Path;
use std::time::Duration;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Restriction {
    pub subject: String,
    pub scope: String,
    pub source: String,
    pub instance_id: Option<String>,
    pub status: String,
    pub suspended: bool,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
    pub last_success_at: Option<i64>,
    pub occurrences: i64,
    pub revision: i64,
}

pub struct Observation<'a> {
    pub subject: &'a str,
    pub scope: &'a str,
    pub source: &'a str,
    pub instance_id: Option<&'a str>,
    /// Request start time, not completion time: late responses cannot undo newer decisions.
    pub started_at: i64,
    pub restricted: bool,
}

pub fn open(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let db = Connection::open(path).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    db.busy_timeout(Duration::from_millis(50))
        .map_err(|e| e.to_string())?;
    let version: i64 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if version == 0 {
        db.execute_batch(
            "BEGIN IMMEDIATE;
            CREATE TABLE IF NOT EXISTS state (
                subject TEXT NOT NULL, scope TEXT NOT NULL, source TEXT NOT NULL,
                instance_id TEXT, status TEXT NOT NULL, suspended INTEGER NOT NULL,
                first_seen_at INTEGER NOT NULL, last_seen_at INTEGER NOT NULL,
                last_success_at INTEGER, occurrences INTEGER NOT NULL, revision INTEGER NOT NULL,
                PRIMARY KEY(subject, scope));
            CREATE TABLE IF NOT EXISTS events (
                id INTEGER PRIMARY KEY, subject TEXT NOT NULL, scope TEXT NOT NULL,
                action TEXT NOT NULL, at INTEGER NOT NULL);
            CREATE INDEX IF NOT EXISTS events_subject ON events(subject, id);
            CREATE INDEX IF NOT EXISTS events_age ON events(at);
            CREATE INDEX IF NOT EXISTS state_age ON state(status, last_seen_at);
            CREATE TABLE IF NOT EXISTS retired (subject TEXT PRIMARY KEY, at INTEGER NOT NULL);
            PRAGMA user_version = 1;
            COMMIT;",
        )
        .map_err(|e| e.to_string())?;
    } else if version != 1 {
        return Err("CODEX_DEVICE_RESTRICTION_SCHEMA".into());
    }
    Ok(db)
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Restriction> {
    Ok(Restriction {
        subject: r.get(0)?,
        scope: r.get(1)?,
        source: r.get(2)?,
        instance_id: r.get(3)?,
        status: r.get(4)?,
        suspended: r.get(5)?,
        first_seen_at: r.get(6)?,
        last_seen_at: r.get(7)?,
        last_success_at: r.get(8)?,
        occurrences: r.get(9)?,
        revision: r.get(10)?,
    })
}

pub fn read(db: &Connection, subject: &str, scope: &str) -> Result<Option<Restriction>, String> {
    db.query_row(
        "SELECT * FROM state WHERE subject=?1 AND scope=?2",
        params![subject, scope],
        row,
    )
    .optional()
    .map_err(|e| e.to_string())
}

pub fn list(db: &Connection, after: &str) -> Result<Vec<Restriction>, String> {
    // Cursor is the subject; never split a subject's (at most five) scopes across pages.
    let mut stmt = db
        .prepare(
            "SELECT * FROM state WHERE subject IN
        (SELECT DISTINCT subject FROM state WHERE subject > ?1 ORDER BY subject LIMIT 100)
        ORDER BY subject, scope",
        )
        .map_err(|e| e.to_string())?;
    let records = stmt
        .query_map([after], row)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string());
    records
}

fn event(db: &Connection, subject: &str, scope: &str, action: &str, at: i64) -> Result<(), String> {
    db.execute(
        "INSERT INTO events(subject,scope,action,at) VALUES(?1,?2,?3,?4)",
        params![subject, scope, action, at],
    )
    .map_err(|e| e.to_string())?;
    // At most 32 events per subject. No raw messages, tokens, URLs or response bodies.
    db.execute(
        "DELETE FROM events WHERE subject=?1 AND id NOT IN
        (SELECT id FROM events WHERE subject=?1 ORDER BY id DESC LIMIT 32)",
        [subject],
    )
    .map_err(|e| e.to_string())?;
    // Retention is bounded maintenance inside this small transaction, not a startup
    // scan. Unresolved account restrictions are never expired into healthy states.
    let cutoff = at.saturating_sub(30 * 24 * 60 * 60 * 1000);
    db.execute("DELETE FROM events WHERE id IN (SELECT id FROM events WHERE at < ?1 ORDER BY at LIMIT 100)", [cutoff])
        .map_err(|e| e.to_string())?;
    db.execute(
        "DELETE FROM state WHERE (subject,scope) IN (SELECT subject,scope FROM state
        WHERE status='recovered' AND last_seen_at < ?1 ORDER BY last_seen_at LIMIT 100)",
        [cutoff],
    )
    .map_err(|e| e.to_string())?;
    db.execute(
        "DELETE FROM state WHERE (subject,scope) IN (SELECT subject,scope FROM state
        WHERE subject >= 'login:' AND subject < 'login;' AND last_seen_at < ?1 LIMIT 100)",
        [cutoff],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn observe(db: &mut Connection, input: Observation<'_>) -> Result<bool, String> {
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let retired: Option<i64> = tx
        .query_row(
            "SELECT at FROM retired WHERE subject=?1",
            [input.subject],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if retired.is_some_and(|at| input.started_at <= at) {
        return Ok(false);
    }
    let previous = read(&tx, input.subject, input.scope)?;
    if previous
        .as_ref()
        .is_some_and(|p| input.started_at <= p.revision)
    {
        return Ok(false);
    }
    if !input.restricted && previous.is_none() {
        return Ok(false);
    }
    // Successful token refresh / quota reads only reach their own scope, never desktop/API state.
    let status = if input.restricted {
        "restricted"
    } else {
        "recovered"
    };
    let changed = previous
        .as_ref()
        .is_none_or(|p| p.status != status || p.suspended != input.restricted);
    let first = previous
        .as_ref()
        .map_or(input.started_at, |p| p.first_seen_at);
    let count = previous.as_ref().map_or(0, |p| p.occurrences) + i64::from(input.restricted);
    let success = if input.restricted {
        previous.as_ref().and_then(|p| p.last_success_at)
    } else {
        Some(input.started_at)
    };
    tx.execute("INSERT INTO state VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
        ON CONFLICT(subject,scope) DO UPDATE SET source=excluded.source,instance_id=excluded.instance_id,
        status=excluded.status,suspended=excluded.suspended,last_seen_at=excluded.last_seen_at,
        last_success_at=excluded.last_success_at,occurrences=excluded.occurrences,revision=excluded.revision",
        params![input.subject,input.scope,input.source,input.instance_id,status,input.restricted,first,
            input.started_at,success,count,input.started_at]).map_err(|e| e.to_string())?;
    if changed {
        event(&tx, input.subject, input.scope, status, input.started_at)?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(changed)
}

pub fn allow_retry(
    db: &mut Connection,
    subject: &str,
    scope: &str,
    revision: i64,
    now: i64,
) -> Result<(), String> {
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let changed = tx
        .execute(
            "UPDATE state SET suspended=0,revision=?4
        WHERE subject=?1 AND scope=?2 AND revision=?3",
            params![subject, scope, revision, now.max(revision + 1)],
        )
        .map_err(|e| e.to_string())?;
    if changed != 1 {
        return Err("CODEX_DEVICE_RESTRICTION_CHANGED".into());
    }
    event(&tx, subject, scope, "retry_allowed", now)?;
    tx.commit().map_err(|e| e.to_string())
}

pub fn retire(db: &mut Connection, subject: &str, now: i64) -> Result<(), String> {
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    tx.execute("INSERT INTO retired VALUES(?1,?2) ON CONFLICT(subject) DO UPDATE SET at=MAX(at,excluded.at)", params![subject,now])
        .map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM state WHERE subject=?1", [subject])
        .map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM events WHERE subject=?1", [subject])
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "codex_device_restrictions_tests.rs"]
mod tests;
