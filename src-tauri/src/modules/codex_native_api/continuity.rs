//! Versioned, private handoff between the two local API engines.
//! Callers stop the other engine before importing/exporting. This module accepts
//! only server-observed ownership records, never client-provided continuation data.
use std::{
    fs::File,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
};

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{checked_identity, IdentityStore};

#[path = "continuity_io.rs"]
mod io;
use io::{write_private_atomic, HashReader, HashWriter};
#[path = "continuity_journal.rs"]
mod journal;

const MAX_JOURNAL_LINE_BYTES: usize = 16 * 1024 * 1024;

pub fn handoff_scope(identity_path: &Path) -> String {
    hash(identity_path.to_string_lossy().as_bytes())
}

pub fn handoff_snapshot_path(identity_path: &Path) -> PathBuf {
    identity_path.with_extension("handoff.json")
}

#[derive(Default, Serialize, Deserialize)]
struct State {
    #[serde(default, deserialize_with = "nullable_list")]
    mappings: Vec<Mapping>,
    #[serde(default, deserialize_with = "nullable_list")]
    affinities: Vec<Affinity>,
    #[serde(default, deserialize_with = "nullable_list")]
    opaque: Vec<Opaque>,
    #[serde(
        default,
        deserialize_with = "nullable_list",
        skip_serializing_if = "Vec::is_empty"
    )]
    recovery_completed: Vec<super::super::recovery::Completion>,
}

fn nullable_list<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    // Go's encoding/json encodes an unallocated slice as null. Both null and
    // an absent collection mean no records, never a request to erase records.
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Serialize, Deserialize)]
struct Mapping {
    caller: String,
    owner: String,
    original: String,
    mapped: String,
}

#[derive(Serialize, Deserialize)]
struct Affinity {
    caller: String,
    kind: String,
    original: String,
    account: String,
    owner: String,
}

#[derive(Serialize, Deserialize)]
struct Opaque {
    caller: String,
    kind: String,
    digest: String,
    account: String,
    owner: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    thread: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    turn: Option<String>,
    expires: i64,
}

#[derive(Serialize, Deserialize)]
struct Snapshot {
    schema_version: u32,
    scope: String,
    generation: String,
    /// The immediately preceding journal can remain after a crash between
    /// snapshot replacement and journal truncation. It is already in state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    predecessor_generation: Option<String>,
    state: State,
}

#[derive(Serialize, Deserialize)]
struct JournalEntry {
    schema_version: u32,
    scope: String,
    generation: String,
    seq: u64,
    payload: String,
    sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    transaction: Option<journal::Chunk>,
}

fn failure(message: &str) -> String {
    format!("API continuity handoff: {message}")
}

fn io_failure(error: impl std::fmt::Display) -> String {
    format!("API continuity handoff storage: {error}")
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn required(value: &str) -> Result<(), String> {
    checked_identity(value)
        .map_err(|_| failure("invalid identifier"))?
        .ok_or_else(|| failure("empty identifier"))
        .map(|_| ())
}

fn generation(value: &str) -> Result<(), String> {
    let id = uuid::Uuid::parse_str(value).map_err(|_| failure("invalid generation"))?;
    if id.get_version_num() != 7 {
        return Err(failure("unsupported generation"));
    }
    Ok(())
}

fn version_scope(version: u32, actual: &str, expected: &str) -> Result<(), String> {
    if version != 1 || actual != expected {
        return Err(failure("unsupported version or different scope"));
    }
    Ok(())
}

fn read_snapshot(path: &Path, scope: &str) -> Result<Option<(Snapshot, String)>, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_failure(error)),
    };
    // Decode trusted durable history incrementally. A lifetime row/byte ceiling
    // would eventually prevent both engines from resuming any new conversation.
    let mut reader = BufReader::new(HashReader::new(file));
    let mut decoder = serde_json::Deserializer::from_reader(&mut reader);
    let snapshot = Snapshot::deserialize(&mut decoder).map_err(|_| failure("invalid snapshot"))?;
    decoder
        .end()
        .map_err(|_| failure("invalid trailing snapshot data"))?;
    let snapshot_hash = reader.into_inner().finish();
    version_scope(snapshot.schema_version, &snapshot.scope, scope)?;
    generation(&snapshot.generation)?;
    if let Some(previous) = &snapshot.predecessor_generation {
        generation(previous)?;
        if previous == &snapshot.generation {
            return Err(failure("cyclic generation"));
        }
    }
    Ok(Some((snapshot, snapshot_hash)))
}

fn create_cursor_table(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS native_handoff_v1 (
         scope TEXT NOT NULL, generation TEXT NOT NULL, snapshot_sha256 TEXT NOT NULL,
         seq INTEGER NOT NULL, PRIMARY KEY(scope,generation));",
        )
        .map_err(io_failure)
}

impl IdentityStore {
    /// Import a stopped Go engine's server-observed state atomically. A partial
    /// final journal line was never committed and is intentionally not replayed.
    pub fn import_handoff(&self, snapshot_path: &Path, scope: &str) -> Result<(), String> {
        required(scope)?;
        let snapshot = read_snapshot(snapshot_path, scope)?;
        let journal_path = snapshot_path.with_extension("jsonl");
        let Some((snapshot, snapshot_hash)) = snapshot else {
            if journal_path.exists() {
                return Err(failure("journal has no snapshot"));
            }
            return Ok(());
        };
        let mut connection = self.connection.lock().map_err(io_failure)?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(io_failure)?;
        create_cursor_table(&transaction)?;
        let cursor: Option<(String, i64)> = transaction.query_row(
            "SELECT snapshot_sha256,seq FROM native_handoff_v1 WHERE scope=?1 AND generation=?2",
            params![scope,snapshot.generation], |row| Ok((row.get(0)?,row.get(1)?)),
        ).optional().map_err(io_failure)?;
        let imported_seq = if let Some((previous_hash, seq)) = cursor {
            if previous_hash != snapshot_hash {
                return Err(failure("generation snapshot changed"));
            }
            seq as u64
        } else {
            apply_state(&transaction, &snapshot.state)?;
            0
        };
        let seq = import_journal(&transaction, &journal_path, &snapshot, imported_seq)?;
        transaction.execute(
            "INSERT INTO native_handoff_v1(scope,generation,snapshot_sha256,seq) VALUES(?1,?2,?3,?4)
             ON CONFLICT(scope,generation) DO UPDATE SET seq=excluded.seq",
            params![scope,snapshot.generation,snapshot_hash,seq as i64],
        ).map_err(io_failure)?;
        transaction.commit().map_err(io_failure)
    }

    /// Publish SQLite's complete state for a stopped Go engine, then retire its
    /// journal. Publishing the predecessor makes the two writes crash-safe.
    pub fn export_handoff(&self, snapshot_path: &Path, scope: &str) -> Result<(), String> {
        self.import_handoff(snapshot_path, scope)?;
        let previous = read_snapshot(snapshot_path, scope)?.map(|(value, _)| value);
        if let Some(previous) = &previous {
            normalize_journal_for_export(snapshot_path, previous)?;
        }
        let predecessor = previous.map(|value| value.generation);
        let connection = self.connection.lock().map_err(io_failure)?;
        let state = export_state(&connection)?;
        let snapshot = Snapshot {
            schema_version: 1,
            scope: scope.to_owned(),
            generation: uuid::Uuid::now_v7().to_string(),
            predecessor_generation: predecessor,
            state,
        };
        let snapshot_hash = write_private_atomic(snapshot_path, |file| {
            let mut writer = HashWriter::new(std::io::BufWriter::new(file));
            serde_json::to_writer(&mut writer, &snapshot).map_err(io_failure)?;
            writer.finish().map_err(io_failure)
        })?;
        sync_parent(snapshot_path)?;
        // Do not modify the previous journal before the new snapshot is durable.
        crate::modules::atomic_write::write_secret_string_atomic(
            &snapshot_path.with_extension("jsonl"),
            "",
        )?;
        sync_parent(snapshot_path)?;
        create_cursor_table(&connection)?;
        connection.execute(
            "INSERT INTO native_handoff_v1(scope,generation,snapshot_sha256,seq) VALUES(?1,?2,?3,0)",
            params![scope,snapshot.generation,snapshot_hash],
        ).map_err(io_failure)?;
        Ok(())
    }
}

fn normalize_journal_for_export(path: &Path, snapshot: &Snapshot) -> Result<(), String> {
    let journal_path = path.with_extension("jsonl");
    if !journal_path.exists() {
        return Ok(());
    }
    write_private_atomic(&journal_path, |output| {
        let mut written = 0_u64;
        let mut transaction_start = None;
        journal::scan(&journal_path, snapshot, |action, previous, _, line| {
            if previous {
                return Ok(());
            }
            match action {
                journal::Action::Begin => transaction_start = Some(written),
                journal::Action::Batch(_) => {
                    std::io::Write::write_all(output, line).map_err(io_failure)?;
                    written += line.len() as u64;
                }
                journal::Action::Commit => transaction_start = None,
                journal::Action::Abort => {
                    output
                        .set_len(transaction_start.unwrap_or(written))
                        .map_err(io_failure)?;
                }
            }
            Ok(())
        })?;
        Ok(())
    })?;
    sync_parent(path)
}

fn sync_parent(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        File::open(parent)
            .map_err(io_failure)?
            .sync_all()
            .map_err(io_failure)?;
    }
    Ok(())
}

fn import_journal(
    transaction: &Transaction<'_>,
    path: &Path,
    snapshot: &Snapshot,
    imported_seq: u64,
) -> Result<u64, String> {
    // Savepoints keep an unfinished large transaction invisible while importing
    // earlier committed records. Each chunk is decoded/applied independently.
    let scan = journal::scan(path, snapshot, |action, previous, seq, _| {
        match action {
            journal::Action::Begin => transaction
                .execute_batch("SAVEPOINT native_chunked_handoff")
                .map_err(io_failure)?,
            journal::Action::Batch(state) if !previous && seq > imported_seq => {
                apply_state(transaction, &state)?
            }
            journal::Action::Commit => transaction
                .execute_batch("RELEASE native_chunked_handoff")
                .map_err(io_failure)?,
            journal::Action::Abort => transaction
                .execute_batch("ROLLBACK TO native_chunked_handoff; RELEASE native_chunked_handoff")
                .map_err(io_failure)?,
            _ => {}
        }
        Ok(())
    })?;
    if scan.seq < imported_seq {
        return Err(failure("acknowledged journal was truncated"));
    }
    Ok(scan.seq)
}

fn validate_state(state: &State) -> Result<(), String> {
    if state.recovery_completed.len() > 1024 {
        return Err(failure("recovery history exceeds capacity"));
    }
    for row in &state.recovery_completed {
        generation(&row.id)?;
        for value in [&row.caller, &row.thread, &row.account, &row.owner] {
            required(value)?;
        }
        if row.completed_at <= 0 {
            return Err(failure("invalid recovery completion"));
        }
    }
    for row in &state.mappings {
        for value in [&row.caller, &row.owner, &row.original, &row.mapped] {
            required(value)?;
        }
    }
    for row in &state.affinities {
        for value in [&row.caller, &row.original, &row.account, &row.owner] {
            required(value)?;
        }
        if !matches!(row.kind.as_str(), "session" | "thread") {
            return Err(failure("invalid affinity kind"));
        }
    }
    for row in &state.opaque {
        for value in [&row.caller, &row.account, &row.owner] {
            required(value)?;
        }
        for value in [&row.session, &row.thread, &row.turn].into_iter().flatten() {
            required(value)?;
        }
        if !matches!(
            row.kind.as_str(),
            "response" | "parent_response" | "encrypted" | "turn_state"
        ) || row.digest.len() != 64
            || !row
                .digest
                .bytes()
                .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
            || (row.kind == "turn_state" && row.expires <= 0)
            || (row.kind != "turn_state" && row.expires != 0)
        {
            return Err(failure("invalid opaque ownership record"));
        }
    }
    Ok(())
}

fn apply_state(transaction: &Transaction<'_>, state: &State) -> Result<(), String> {
    validate_state(state)?;
    for row in &state.recovery_completed {
        let existing:Option<(String,String,String,String,i64)>=transaction.query_row("SELECT caller,thread,account,owner,completed_at FROM native_recovery_commits_v1 WHERE id=?1",[&row.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(io_failure)?;
        if let Some(old) = existing {
            if old
                != (
                    row.caller.clone(),
                    row.thread.clone(),
                    row.account.clone(),
                    row.owner.clone(),
                    row.completed_at,
                )
            {
                return Err(failure("recovery completion conflict"));
            }
        } else {
            transaction.execute("INSERT INTO native_recovery_commits_v1(id,caller,thread,account,owner,completed_at) VALUES(?1,?2,?3,?4,?5,?6)",params![row.id,row.caller,row.thread,row.account,row.owner,row.completed_at]).map_err(io_failure)?;
        }
    }
    transaction.execute("DELETE FROM native_recovery_commits_v1 WHERE id IN (SELECT id FROM native_recovery_commits_v1 ORDER BY completed_at DESC,id DESC LIMIT -1 OFFSET 1024)",[]).map_err(io_failure)?;
    for row in &state.mappings {
        let existing: Option<String> = transaction.query_row("SELECT mapped FROM native_identity_v1 WHERE caller=?1 AND owner=?2 AND original=?3",params![row.caller,row.owner,row.original],|value|value.get(0)).optional().map_err(io_failure)?;
        if let Some(existing) = existing {
            if existing != row.mapped {
                return Err(failure("identity mapping conflict"));
            }
        } else {
            transaction.execute("INSERT INTO native_identity_v1(caller,owner,original,mapped) VALUES(?1,?2,?3,?4)",params![row.caller,row.owner,row.original,row.mapped]).map_err(|_| failure("identity mapping collision"))?;
        }
    }
    for row in &state.opaque {
        let existing: Option<(String,String)> = transaction.query_row("SELECT account,owner FROM native_opaque_v1 WHERE caller=?1 AND kind=?2 AND digest=?3",params![row.caller,row.kind,row.digest],|value|Ok((value.get(0)?,value.get(1)?))).optional().map_err(io_failure)?;
        if let Some((account, owner)) = &existing {
            if account != &row.account || owner != &row.owner {
                return Err(failure("opaque owner conflict"));
            }
        }
        transaction.execute("INSERT INTO native_opaque_v1(caller,kind,digest,account,owner,session,thread,turn,expires) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(caller,kind,digest) DO UPDATE SET expires=MAX(native_opaque_v1.expires,excluded.expires)",params![row.caller,row.kind,row.digest,row.account,row.owner,row.session,row.thread,row.turn,row.expires]).map_err(io_failure)?;
    }
    for row in &state.affinities {
        transaction.execute("INSERT INTO native_affinity_v1(caller,kind,original,account,owner) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(caller,kind,original) DO UPDATE SET account=excluded.account,owner=excluded.owner",params![row.caller,row.kind,row.original,row.account,row.owner]).map_err(io_failure)?;
    }
    Ok(())
}

fn export_state(connection: &Connection) -> Result<State, String> {
    let recovery_completed=connection.prepare("SELECT id,caller,thread,account,owner,completed_at FROM native_recovery_commits_v1 ORDER BY completed_at,id").map_err(io_failure)?.query_map([],|r|Ok(super::super::recovery::Completion{id:r.get(0)?,caller:r.get(1)?,thread:r.get(2)?,account:r.get(3)?,owner:r.get(4)?,completed_at:r.get(5)?})).map_err(io_failure)?.collect::<Result<Vec<_>,_>>().map_err(io_failure)?;
    let mappings = connection.prepare("SELECT caller,owner,original,mapped FROM native_identity_v1 ORDER BY caller,owner,original").map_err(io_failure)?.query_map([],|row|Ok(Mapping{caller:row.get(0)?,owner:row.get(1)?,original:row.get(2)?,mapped:row.get(3)?})).map_err(io_failure)?.collect::<Result<Vec<_>,_>>().map_err(io_failure)?;
    let affinities = connection.prepare("SELECT caller,kind,original,account,owner FROM native_affinity_v1 ORDER BY caller,kind,original").map_err(io_failure)?.query_map([],|row|Ok(Affinity{caller:row.get(0)?,kind:row.get(1)?,original:row.get(2)?,account:row.get(3)?,owner:row.get(4)?})).map_err(io_failure)?.collect::<Result<Vec<_>,_>>().map_err(io_failure)?;
    let opaque = connection.prepare("SELECT caller,kind,digest,account,owner,session,thread,turn,expires FROM native_opaque_v1 ORDER BY caller,kind,digest").map_err(io_failure)?.query_map([],|row|Ok(Opaque{caller:row.get(0)?,kind:row.get(1)?,digest:row.get(2)?,account:row.get(3)?,owner:row.get(4)?,session:row.get(5)?,thread:row.get(6)?,turn:row.get(7)?,expires:row.get(8)?})).map_err(io_failure)?.collect::<Result<Vec<_>,_>>().map_err(io_failure)?;
    let state = State {
        mappings,
        affinities,
        opaque,
        recovery_completed,
    };
    validate_state(&state)?;
    Ok(state)
}

#[cfg(test)]
#[path = "continuity_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "continuity_transaction_tests.rs"]
mod transaction_tests;
