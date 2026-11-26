//! Read the Desktop setting store used by the native SSH connections.
use super::ssh_server_identity as identity;
use rusqlite::{Connection, ErrorCode, OpenFlags, TransactionBehavior};
use serde_json::{Map, Value};
use std::{fs, path::Path, time::Duration};

const DATABASE: &str = ".codex-global-state.sqlite";
pub(super) const BUSY: &str = "profile_state_busy";
const UNAVAILABLE: &str = "profile_state_unavailable";
const INVALID: &str = "profile_state_invalid";
const UNSUPPORTED: &str = "profile_state_unsupported";

type StateResult<T> = Result<T, &'static str>;

fn sqlite_error(error: rusqlite::Error) -> &'static str {
    match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => BUSY,
        Some(ErrorCode::CannotOpen | ErrorCode::PermissionDenied | ErrorCode::SystemIoFailure) => {
            UNAVAILABLE
        }
        _ => INVALID,
    }
}

fn open_sqlite(profile: &Path, flags: OpenFlags) -> StateResult<Option<Connection>> {
    let path = profile.join(DATABASE);
    if !path.try_exists().map_err(|_| UNAVAILABLE)? {
        return Ok(None);
    }
    let db = Connection::open_with_flags(path, flags).map_err(sqlite_error)?;
    db.busy_timeout(Duration::from_millis(150))
        .map_err(sqlite_error)?;
    let version: u32 = db
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(sqlite_error)?;
    match version {
        1 => Ok(Some(db)),
        // The Desktop may have created the file before committing its migration.
        0 => Err(BUSY),
        _ => Err(UNSUPPORTED),
    }
}

fn read_sqlite_state(db: &Connection, _next_owner: Option<&str>) -> StateResult<Value> {
    let mut statement = db
        .prepare("SELECT key, value FROM settings WHERE scope = 'global' ORDER BY key")
        .map_err(sqlite_error)?;
    let mut rows = statement.query([]).map_err(sqlite_error)?;
    let mut snapshot = Map::new();
    while let Some(row) = rows.next().map_err(sqlite_error)? {
        let key: String = row.get(0).map_err(sqlite_error)?;
        let raw: String = row.get(1).map_err(sqlite_error)?;
        let value: Value = serde_json::from_str(&raw).map_err(|_| INVALID)?;
        snapshot.insert(key, value);
    }
    Ok(Value::Object(snapshot))
}

pub(super) fn load(profile: &Path) -> StateResult<Value> {
    // After migration the JSON file can still exist, but Desktop writes its SQLite store.
    if let Some(mut db) = open_sqlite(profile, OpenFlags::SQLITE_OPEN_READ_ONLY)? {
        let snapshot = db.transaction().map_err(sqlite_error)?;
        return read_sqlite_state(&snapshot, None);
    }
    let raw = fs::read(profile.join(".codex-global-state.json")).map_err(|_| UNAVAILABLE)?;
    serde_json::from_slice(&raw).map_err(|_| INVALID)
}

/// Called under the launch lease after the Desktop instance has stopped.
pub(super) fn prepare_sqlite_startup(
    profile: &Path,
    next_owner: Option<&str>,
) -> StateResult<Option<(Value, bool)>> {
    let Some(mut db) = open_sqlite(profile, OpenFlags::SQLITE_OPEN_READ_WRITE)? else {
        return Ok(None);
    };
    let next_owner = next_owner.unwrap_or(identity::SIGNED_OUT_OWNER);
    let transaction = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sqlite_error)?;
    let mut state = read_sqlite_state(&transaction, Some(next_owner))?;
    let mut changed_keys = identity::inherit_scoped_owner_local_state(&mut state, next_owner);
    if identity::inherit_scoped_auto_connect(&mut state, next_owner) {
        changed_keys.push(identity::scoped_auto_connect_key(next_owner));
    }
    for key in &changed_keys {
        transaction
            .execute(
                "INSERT INTO settings (scope, key, value) VALUES ('global', ?1, ?2)
                 ON CONFLICT (scope, key) DO UPDATE SET value = excluded.value",
                (key, state[key].to_string()),
            )
            .map_err(sqlite_error)?;
    }
    transaction.commit().map_err(sqlite_error)?;
    Ok(Some((state, !changed_keys.is_empty())))
}

pub(super) fn load_optional_sqlite_state(profile: &Path) -> Result<Option<Value>, String> {
    let Some(mut db) = open_sqlite(profile, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    let snapshot = db.transaction().map_err(|e| e.to_string())?;
    read_sqlite_state(&snapshot, None)
        .map(Some)
        .map_err(|e| e.to_string())
}

pub(super) fn sync_repaired_project_state_to_sqlite(
    profile: &Path,
    before: &Value,
    after: &Value,
) -> Result<(), String> {
    let Some(mut db) = open_sqlite(profile, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|e| e.to_string())?
    else {
        return Ok(());
    };
    let transaction = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let mut sqlite_state =
        read_sqlite_state(&transaction, None).map_err(|e| e.to_string())?;
    let mut changed_keys = Vec::new();
    if let (Some(after_map), Some(sqlite_map)) =
        (after.as_object(), sqlite_state.as_object_mut())
    {
        for (key, value) in after_map {
            if before.get(key) != Some(value) && sqlite_map.get(key) != Some(value) {
                sqlite_map.insert(key.clone(), value.clone());
                changed_keys.push(key.clone());
            }
        }
    }
    let owner = identity::renderer_owner(&sqlite_state)
        .unwrap_or(identity::SIGNED_OUT_OWNER)
        .to_owned();
    let owner_changed = identity::inherit_scoped_owner_local_state(&mut sqlite_state, &owner);
    for key in owner_changed {
        if !changed_keys.contains(&key) {
            changed_keys.push(key);
        }
    }
    for key in &changed_keys {
        if let Some(val) = sqlite_state.get(key) {
            transaction
                .execute(
                    "INSERT INTO settings (scope, key, value) VALUES ('global', ?1, ?2)
                     ON CONFLICT (scope, key) DO UPDATE SET value = excluded.value",
                    (key, val.to_string()),
                )
                .map_err(|e| e.to_string())?;
        }
    }
    transaction.commit().map_err(|e| e.to_string())
}

pub(super) fn rollback_repaired_project_state_in_sqlite(
    profile: &Path,
    before: &Value,
    after: &Value,
) -> Result<(), String> {
    let Some(mut db) = open_sqlite(profile, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|e| e.to_string())?
    else {
        return Ok(());
    };
    let transaction = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let current = read_sqlite_state(&transaction, None).map_err(|e| e.to_string())?;
    let Some(after_map) = after.as_object() else {
        return Ok(());
    };
    for (key, written) in after_map {
        let original = before.get(key);
        if original == Some(written) || current.get(key) != Some(written) {
            continue;
        }
        match original {
            Some(orig_val) => {
                transaction
                    .execute(
                        "INSERT INTO settings (scope, key, value) VALUES ('global', ?1, ?2)
                         ON CONFLICT (scope, key) DO UPDATE SET value = excluded.value",
                        (key, orig_val.to_string()),
                    )
                    .map_err(|e| e.to_string())?;
            }
            None => {
                transaction
                    .execute(
                        "DELETE FROM settings WHERE scope = 'global' AND key = ?1",
                        [key],
                    )
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    transaction.commit().map_err(|e| e.to_string())
}

#[cfg(test)]
pub(super) fn desktop_settings_fixture(profile: &Path, settings: &[(&str, Value)]) -> Connection {
    let db = Connection::open(profile.join(DATABASE)).unwrap();
    db.execute_batch(
        "CREATE TABLE settings (scope TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL,
             PRIMARY KEY (scope, key)) WITHOUT ROWID;
         PRAGMA user_version = 1;",
    )
    .unwrap();
    for (key, value) in settings {
        db.execute(
            "INSERT INTO settings VALUES ('global', ?1, ?2)",
            (*key, value.to_string()),
        )
        .unwrap();
    }
    db
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn legacy_json_is_used_only_without_a_sqlite_store() {
        let profile = tempfile::tempdir().unwrap();
        let legacy = json!({identity::RENDERER_OWNER: "old"});
        fs::write(
            profile.path().join(".codex-global-state.json"),
            legacy.to_string(),
        )
        .unwrap();
        assert_eq!(load(profile.path()).unwrap(), legacy);
        let db = desktop_settings_fixture(
            profile.path(),
            &[
                (identity::RENDERER_OWNER, json!("new")),
                (
                    identity::CONNECTIONS,
                    json!([
                        {"hostId":"enabled","alias":"current-host"},
                        {"hostId":"disabled","alias":"disabled-host"}
                    ]),
                ),
                (
                    &identity::scoped_auto_connect_key("new"),
                    json!({"enabled":true,"disabled":false}),
                ),
                (
                    &identity::scoped_auto_connect_key("old"),
                    json!({"disabled":true}),
                ),
            ],
        );
        let active = load(profile.path()).unwrap();
        assert_eq!(identity::renderer_owner(&active), Some("new"));
        assert_eq!(
            identity::aliases(&active),
            [("enabled".into(), "current-host".into())]
        );
        db.pragma_update(None, "user_version", 0).unwrap();
        assert_eq!(load(profile.path()), Err(BUSY));
        db.pragma_update(None, "user_version", 2).unwrap();
        assert_eq!(load(profile.path()), Err(UNSUPPORTED));
    }

    #[test]
    fn a_locked_sqlite_store_never_falls_back_to_stale_json() {
        let profile = tempfile::tempdir().unwrap();
        fs::write(profile.path().join(".codex-global-state.json"), "{}").unwrap();
        let db = desktop_settings_fixture(profile.path(), &[]);
        db.execute_batch("BEGIN EXCLUSIVE").unwrap();
        assert_eq!(load(profile.path()), Err(BUSY));
        db.execute_batch("ROLLBACK").unwrap();
        assert_eq!(load(profile.path()).unwrap(), json!({}));
    }
}
