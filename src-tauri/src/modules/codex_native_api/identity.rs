//! Durable, caller- and owner-scoped protocol identities. Never rewrites model content.
//!
//! Public methods perform SQLite/history work and must run on a blocking worker.
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::http::{HeaderMap, HeaderValue};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{agent_communication, contract::ServiceError};

#[path = "continuity.rs"]
mod continuity;
pub use continuity::{handoff_scope, handoff_snapshot_path};
#[path = "identity_handoff.rs"]
mod identity_handoff;
pub(crate) use identity_handoff::begin_handoff;
#[path = "identity_automatic_history.rs"]
mod automatic_history;
#[cfg(test)]
#[path = "identity_opaque_replay_tests.rs"]
mod opaque_replay_tests;
#[path = "identity_rebuild.rs"]
pub(crate) mod rebuild;
// Per-request work is bounded independently of lifetime history size.
const MAX_REQUEST_OPAQUE_REFS: usize = 100_000;
const OPAQUE_TTL_SECONDS: i64 = 24 * 60 * 60;
const MAX_ID_BYTES: usize = 4096;
// Canonical metadata includes the tool inventory. Bound metadata expansion
// independently of conversation input; HTTP headers have their transport limit.
const MAX_METADATA_BYTES: usize = 16 * 1024 * 1024;

pub struct IdentityStore {
    connection: Mutex<Connection>,
    path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct RequestIdentity {
    pub session_id: Option<String>,
    pub thread_id: Option<String>,
    pub parent_thread_id: Option<String>,
    /// Compatibility projection of known ownership; never a durable soft hint.
    pub preferred_account_id: Option<String>,
    /// Hard ownership of an upstream resource required by an incremental request.
    pub state_account_id: Option<String>,
    caller: String,
    turn_id: Option<String>,
    opaque: Vec<OpaqueRef>,
    canonical_metadata: Option<Value>,
    generation: Option<String>,
    rebuild_enabled: bool,
    rebuild_input: Option<Vec<Value>>,
    rebuild_parameters: Option<Value>,
    rebuild_base_revision: Option<i64>,
    rebuild_new_generation: bool,
    portable_body: bool,
    detached_parent: bool,
    detached_turn_states: Vec<OpaqueRef>,
    unregistered_blobs: std::collections::HashSet<String>,
}

#[derive(Clone, Debug)]
pub struct Binding {
    caller: String,
    account_id: String,
    owner_id: String,
    session_id: Option<String>,
    thread_id: Option<String>,
    turn_id: Option<String>,
    parent_thread_id: Option<String>,
    portable_body: bool,
    input_blobs: Vec<OpaqueRef>,
    generation: Option<rebuild::Generation>,
    pending_input: Option<Arc<automatic_history::Admission>>,
}

impl Binding {
    /// A provider projection may omit incompatible opaque state. A successful
    /// request cannot establish ownership of bytes that were never sent.
    pub(crate) fn retain_transmitted_input_blobs(&mut self, body: &Value) {
        let mut sent = Vec::new();
        collect_encrypted(body.get("input"), &mut sent);
        let sent: std::collections::HashSet<_> = sent
            .into_iter()
            .map(|reference| (reference.kind, reference.digest))
            .collect();
        self.input_blobs
            .retain(|reference| sent.contains(&(reference.kind, reference.digest.clone())));
    }
}

#[derive(Clone, Debug)]
struct OpaqueRef {
    kind: &'static str,
    digest: String,
}

fn invalid(message: impl Into<String>) -> ServiceError {
    ServiceError::new(400, "invalid_protocol_identity", message)
}

fn storage(error: impl std::fmt::Display) -> ServiceError {
    // SQLite errors contain schema/operation details, never include caller-provided values.
    ServiceError::new(
        503,
        "identity_store_unavailable",
        format!("Identity storage unavailable: {error}"),
    )
}

fn timestamp() -> i64 {
    chrono::Utc::now().timestamp()
}

fn identity_value(value: Option<&Value>) -> Result<Option<String>, ServiceError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => checked_identity(value),
        Some(_) => Err(invalid("Protocol identifiers must be strings")),
    }
}

fn checked_identity(value: &str) -> Result<Option<String>, ServiceError> {
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > MAX_ID_BYTES || value.chars().any(char::is_control) {
        return Err(invalid(
            "Protocol identifier exceeds its allowed size or contains control characters",
        ));
    }
    // Identity is opaque: do not trim, lowercase, or otherwise merge distinct client values.
    Ok(Some(value.to_owned()))
}

fn header_identity(headers: &HeaderMap, name: &str) -> Result<Option<String>, ServiceError> {
    match headers.get(name) {
        None => Ok(None),
        Some(value) => checked_identity(
            value
                .to_str()
                .map_err(|_| invalid("Invalid identity header"))?,
        ),
    }
}

fn primary_identity(
    canonical: Option<&Value>,
    flat: Option<&Value>,
    headers: &HeaderMap,
    header: &str,
) -> Result<Option<String>, ServiceError> {
    for candidate in [canonical, flat] {
        if let Some(value) = identity_value(candidate)? {
            return Ok(Some(value));
        }
    }
    header_identity(headers, header)
}

fn parse_metadata(raw: &str) -> Result<Value, ServiceError> {
    if raw.len() > MAX_METADATA_BYTES {
        return Err(invalid("Turn metadata exceeds its allowed size"));
    }
    let metadata: Value =
        serde_json::from_str(raw).map_err(|_| invalid("Invalid turn metadata JSON"))?;
    if !metadata.is_object() {
        return Err(invalid("Turn metadata must be an object"));
    }
    Ok(metadata)
}

fn canonical_metadata(headers: &HeaderMap, body: &Value) -> Result<Option<Value>, ServiceError> {
    if let Some(raw) = body.pointer("/client_metadata/x-codex-turn-metadata") {
        return raw
            .as_str()
            .ok_or_else(|| invalid("Turn metadata must be a JSON string"))
            .and_then(parse_metadata)
            .map(Some);
    }
    headers
        .get("x-codex-turn-metadata")
        .map(|value| {
            value
                .to_str()
                .map_err(|_| invalid("Invalid turn metadata header"))
                .and_then(parse_metadata)
        })
        .transpose()
}

fn digest(kind: &'static str, value: &str) -> OpaqueRef {
    let bytes = Sha256::digest(value.as_bytes());
    OpaqueRef {
        kind,
        digest: format!("{bytes:x}"),
    }
}

fn turn_state_reference(value: &str, thread: Option<&str>, turn: Option<&str>) -> OpaqueRef {
    // An opaque routing token may legitimately be reissued to multiple turns.
    // Include presence and length so absent, empty and concatenated values cannot alias.
    let mut hasher = Sha256::new();
    for part in [Some(value), thread, turn] {
        match part {
            Some(part) => {
                hasher.update([1]);
                hasher.update((part.len() as u64).to_be_bytes());
                hasher.update(part.as_bytes());
            }
            None => hasher.update([0]),
        }
    }
    OpaqueRef {
        kind: "turn_state",
        digest: format!("{:x}", hasher.finalize()),
    }
}

fn collect_encrypted(items: Option<&Value>, output: &mut Vec<OpaqueRef>) {
    if let Some(items) = items.and_then(Value::as_array) {
        for item in items {
            collect_encrypted_item(item, output);
        }
    }
}

fn collect_encrypted_item(item: &Value, output: &mut Vec<OpaqueRef>) {
    if matches!(
        item.get("type").and_then(Value::as_str),
        Some(
            "message"
                | "reasoning"
                | "local_shell_call"
                | "function_call"
                | "tool_search_call"
                | "custom_tool_call"
                | "web_search_call"
                | "image_generation_call"
                | "compaction"
                | "compaction_summary"
                | "context_compaction"
        )
    ) {
        if let Some(value) = item
            .get("encrypted_content")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
        {
            output.push(digest("encrypted", value));
        }
    }
    if item.get("type").and_then(Value::as_str) == Some("function_call") {
        if let Some(arguments) = item
            .get("encrypted_function_args")
            .and_then(Value::as_array)
        {
            for argument in arguments
                .iter()
                .filter_map(Value::as_str)
                .filter(|v| !v.is_empty())
            {
                output.push(digest("encrypted", argument));
            }
        }
        if let Some(message) = agent_communication::encrypted_message(item) {
            output.push(digest("encrypted", &message));
        }
    }
    if matches!(
        item.get("type").and_then(Value::as_str),
        Some("message" | "agent_message" | "function_call_output" | "custom_tool_call_output")
    ) {
        let content_field = if matches!(
            item["type"].as_str(),
            Some("function_call_output" | "custom_tool_call_output")
        ) {
            "output"
        } else {
            "content"
        };
        if let Some(content) = item.get(content_field).and_then(Value::as_array) {
            for part in content {
                if part.get("type").and_then(Value::as_str) == Some("encrypted_content") {
                    if let Some(value) = part
                        .get("encrypted_content")
                        .and_then(Value::as_str)
                        .filter(|v| !v.is_empty())
                    {
                        output.push(digest("encrypted", value));
                    }
                }
            }
        }
    }
}

impl IdentityStore {
    pub fn open(path: PathBuf) -> Result<Self, String> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let connection = Connection::open(&path).map_err(|error| error.to_string())?;
        connection
            .busy_timeout(Duration::from_secs(2))
            .map_err(|error| error.to_string())?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS native_identity_v1 (
                caller TEXT NOT NULL, owner TEXT NOT NULL, original TEXT NOT NULL,
                mapped TEXT NOT NULL, PRIMARY KEY(caller, owner, original));
             CREATE UNIQUE INDEX IF NOT EXISTS native_identity_v1_mapped ON native_identity_v1(mapped);
             CREATE TABLE IF NOT EXISTS native_affinity_v1 (
                caller TEXT NOT NULL, kind TEXT NOT NULL, original TEXT NOT NULL,
                account TEXT NOT NULL, owner TEXT NOT NULL,
                PRIMARY KEY(caller, kind, original));
             CREATE TABLE IF NOT EXISTS native_opaque_v1 (
                caller TEXT NOT NULL, kind TEXT NOT NULL, digest TEXT NOT NULL,
                account TEXT NOT NULL, owner TEXT NOT NULL,
                session TEXT, thread TEXT, turn TEXT, expires INTEGER NOT NULL,
                PRIMARY KEY(caller, kind, digest));
             CREATE INDEX IF NOT EXISTS native_opaque_v1_expiry ON native_opaque_v1(expires);
             CREATE INDEX IF NOT EXISTS native_opaque_v1_reference ON native_opaque_v1(kind,digest);
             CREATE TABLE IF NOT EXISTS native_recovery_commits_v1 (
                id TEXT PRIMARY KEY, caller TEXT NOT NULL, thread TEXT NOT NULL,
                account TEXT NOT NULL, owner TEXT NOT NULL, completed_at INTEGER NOT NULL);"
        ).map_err(|error| error.to_string())?;
        rebuild::initialize(&connection).map_err(|error| error.to_string())?;
        Ok(Self {
            connection: Mutex::new(connection),
            path,
        })
    }

    pub fn inspect(
        &self,
        caller: &str,
        headers: &HeaderMap,
        body: &Value,
    ) -> Result<RequestIdentity, ServiceError> {
        self.inspect_mode(caller, headers, body, false)
    }

    pub(crate) fn inspect_for_rebuild(
        &self,
        caller: &str,
        headers: &HeaderMap,
        body: &Value,
    ) -> Result<RequestIdentity, ServiceError> {
        self.inspect_mode(caller, headers, body, true)
    }

    fn checkpoint_reader(&self) -> Result<Connection, ServiceError> {
        let connection =
            Connection::open_with_flags(&self.path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(storage)?;
        connection
            .busy_timeout(Duration::from_secs(2))
            .map_err(storage)?;
        Ok(connection)
    }

    fn inspect_mode(
        &self,
        caller: &str,
        headers: &HeaderMap,
        body: &Value,
        portable_requested: bool,
    ) -> Result<RequestIdentity, ServiceError> {
        if checked_identity(caller)?.is_none() {
            return Err(invalid("A caller identity is required"));
        }
        let canonical_value = canonical_metadata(headers, body)?;
        let canonical = canonical_value.as_ref();
        let flat = body.get("client_metadata");
        let mut session_id = primary_identity(
            canonical.and_then(|v| v.get("session_id")),
            flat.and_then(|v| v.get("session_id")),
            headers,
            "session-id",
        )?;
        if session_id.is_none() {
            // Legacy clients such as Pi use the underscore spelling. Resolve it
            // before generating a root, preserving the existing validation order.
            session_id = header_identity(headers, "session_id")?;
        }
        let mut thread_id = primary_identity(
            canonical.and_then(|v| v.get("thread_id")),
            flat.and_then(|v| v.get("thread_id")),
            headers,
            "thread-id",
        )?;
        let parent_thread_id = if let Some(canonical) = canonical {
            identity_value(canonical.get("parent_thread_id"))?
        } else {
            primary_identity(
                None,
                flat.and_then(|v| v.get("x-codex-parent-thread-id")),
                headers,
                "x-codex-parent-thread-id",
            )?
        };
        let turn_id = if let Some(canonical) = canonical {
            identity_value(canonical.get("turn_id"))?
        } else {
            identity_value(flat.and_then(|v| v.get("turn_id")))?
        };
        // Input/tool scans and accepted-history decoding can be large. Keep
        // them off the publication mutex and use one consistent WAL snapshot.
        let reader = self.checkpoint_reader()?;
        let connection = reader.unchecked_transaction().map_err(storage)?;
        let mut opaque = Vec::new();
        if let Some(value) = identity_value(body.get("previous_response_id"))? {
            let reference = digest("response", &value);
            let restored: Option<(Option<String>, Option<String>)> = connection.query_row(
                    "SELECT session,thread FROM native_opaque_v1 WHERE caller=?1 AND kind='response' AND digest=?2",
                    params![caller, reference.digest], |row| Ok((row.get(0)?, row.get(1)?)),
                ).optional().map_err(storage)?;
            if restored.is_none() {
                automatic_history::lookup(
                    &connection,
                    caller,
                    &reference,
                    thread_id.as_deref(),
                    turn_id.as_deref(),
                )?;
                // Official clients can retry a missing checkpoint with full
                // input. This never authorizes replaying unknown ciphertext.
                return Err(ServiceError::new(
                    409,
                    "previous_response_not_found",
                    "Previous response is unavailable; resend the full conversation input",
                ));
            }
            let (previous_session, previous_thread) = restored.unwrap_or_default();
            if session_id.is_none() || thread_id.is_none() {
                session_id = session_id.or(previous_session);
                thread_id = thread_id.or(previous_thread);
                if session_id.is_none() && thread_id.is_none() {
                    return Err(unknown_state());
                }
            }
            opaque.push(reference);
        }
        // Guardian can review a live parent response before it becomes a resumable
        // checkpoint. Its reference still belongs to the parent's upstream owner.
        for value in [
            identity_value(flat.and_then(|v| v.get("parent_response_id")))?,
            identity_value(canonical.and_then(|v| v.get("parent_response_id")))?,
        ]
        .into_iter()
        .flatten()
        {
            opaque.push(digest("parent_response", &value));
        }
        // Generic Responses clients need not expose Codex metadata. A new stateless
        // request gets its own root; a child without a session uses its own thread,
        // never the parent or cache routing alias as a guessed root.
        match (&session_id, &thread_id) {
            (None, None) => {
                let root = uuid::Uuid::now_v7().to_string();
                session_id = Some(root.clone());
                thread_id = Some(root);
            }
            (Some(session), None) => thread_id = Some(session.clone()),
            (None, Some(thread)) => session_id = Some(thread.clone()),
            (Some(_), Some(_)) => {}
        }
        for value in [
            header_identity(headers, "x-codex-turn-state")?,
            identity_value(flat.and_then(|v| v.get("x-codex-turn-state")))?,
            identity_value(canonical.and_then(|v| v.get("x-codex-turn-state")))?,
        ]
        .into_iter()
        .flatten()
        {
            opaque.push(turn_state_reference(
                &value,
                thread_id.as_deref(),
                turn_id.as_deref(),
            ));
        }
        collect_encrypted(body.get("input"), &mut opaque);
        let mut seen = std::collections::HashSet::new();
        opaque.retain(|reference| seen.insert((reference.kind, reference.digest.clone())));
        if portable_requested && opaque.len() > MAX_REQUEST_OPAQUE_REFS {
            return Err(ServiceError::new(413, "continuation_history_too_large", "Conversation input exceeds the per-request reference limit; existing history was preserved"));
        }

        let mut preferred_account_id = None;
        let mut expected_owner = None;
        let portable_requested = portable_requested
            && body
                .get("input")
                .and_then(Value::as_array)
                .is_some_and(|items| rebuild::replay_items(items));
        let mut portable_body = portable_requested;
        let automatic_input = portable_requested
            && rebuild::complete_input_available(
                &connection,
                caller,
                thread_id.as_deref(),
                headers,
                body,
            )?;
        let mut unregistered_blobs = std::collections::HashSet::new();
        // Server references determine continuation ownership. Body blobs retain
        // their observed provenance, but may be replayed as complete payloads.
        for reference in opaque.iter().filter(|reference| {
            reference.kind != "encrypted"
                && reference.kind != "parent_response"
                && reference.kind != "turn_state"
        }) {
            let (account, owner) = validate_opaque(
                &connection,
                caller,
                reference,
                thread_id.as_deref(),
                turn_id.as_deref(),
            )?;
            if expected_owner
                .as_ref()
                .is_some_and(|expected| expected != &owner)
            {
                return Err(ServiceError::new(
                    409,
                    "mixed_upstream_state",
                    "Request contains state from different upstream owners",
                ));
            }
            expected_owner = Some(owner);
            preferred_account_id = Some(account);
        }
        let server_refs = opaque
            .iter()
            .filter(|reference| {
                reference.kind != "encrypted"
                    && reference.kind != "parent_response"
                    && reference.kind != "turn_state"
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut generation = rebuild::reference_generation(&connection, caller, &server_refs)?;
        // Closing the switch stops new migrations, not a generation's already
        // accepted history. A successful replay grants no rights to other callers.
        let accepted_scope = rebuild::accepted_scope(
            &connection,
            caller,
            thread_id.as_deref(),
            preferred_account_id.as_deref(),
            generation.as_deref(),
        )?;
        if preferred_account_id.is_none() {
            if let Some((account, owner, accepted_generation)) = &accepted_scope {
                preferred_account_id = Some(account.clone());
                expected_owner = Some(owner.clone());
                generation = Some(accepted_generation.clone());
            }
        }
        let mut detached_turn_states = Vec::new();
        for reference in opaque
            .iter()
            .filter(|reference| reference.kind == "turn_state")
        {
            // A complete input does not depend on an unregistered sticky route.
            // Known routes still validate caller, logical thread and turn.
            let found = automatic_history::lookup(
                &connection,
                caller,
                reference,
                thread_id.as_deref(),
                turn_id.as_deref(),
            )?;
            let Some((account, owner)) = found else {
                if automatic_input || (portable_requested && !server_refs.is_empty()) {
                    detached_turn_states.push(reference.clone());
                    continue;
                }
                return Err(if portable_requested {
                    automatic_history::incomplete()
                } else {
                    unknown_state()
                });
            };
            if expected_owner
                .as_ref()
                .is_some_and(|expected| expected != &owner)
            {
                if accepted_scope.is_some() {
                    detached_turn_states.push(reference.clone());
                    continue;
                }
                return Err(ServiceError::new(
                    409,
                    "mixed_upstream_state",
                    "Routing token belongs to a different upstream owner",
                ));
            }
            if expected_owner.is_none() {
                preferred_account_id = Some(account);
                expected_owner = Some(owner);
            }
        }
        portable_body |= accepted_scope.is_some();
        let full_input = (portable_requested || accepted_scope.is_some())
            && !headers.contains_key("conversation_id")
            && rebuild::full_input(&connection, caller, thread_id.as_deref(), body)?;
        let mut detached_parent = false;
        for reference in opaque
            .iter()
            .filter(|reference| reference.kind == "parent_response")
        {
            let found = automatic_history::lookup(
                &connection,
                caller,
                reference,
                thread_id.as_deref(),
                turn_id.as_deref(),
            )?;
            let Some((account, owner)) = found else {
                if automatic_input {
                    detached_parent = true;
                    continue;
                }
                return Err(if portable_requested {
                    automatic_history::incomplete()
                } else {
                    unknown_state()
                });
            };
            if expected_owner
                .as_ref()
                .is_some_and(|expected| expected != &owner)
            {
                if full_input {
                    detached_parent = true;
                    continue;
                }
                return Err(ServiceError::new(
                    409,
                    "mixed_upstream_state",
                    "Incremental history cannot detach a different owner's parent response",
                ));
            }
            if expected_owner.is_none() {
                expected_owner = Some(owner);
                preferred_account_id = Some(account);
                generation =
                    rebuild::reference_generation(&connection, caller, &[reference.clone()])?;
            }
        }
        for reference in opaque
            .iter()
            .filter(|reference| reference.kind == "encrypted")
        {
            let found = automatic_history::lookup(
                &connection,
                caller,
                reference,
                thread_id.as_deref(),
                turn_id.as_deref(),
            )?;
            let Some((account, owner)) = found else {
                if automatic_input {
                    unregistered_blobs.insert(reference.digest.clone());
                    continue;
                }
                return Err(if portable_requested {
                    automatic_history::incomplete()
                } else {
                    unknown_state()
                });
            };
            let accepted = if let Some(scope) = &accepted_scope {
                rebuild::blob_accepted(&connection, caller, thread_id.as_deref(), reference, scope)?
            } else {
                false
            };
            if portable_requested || accepted {
                portable_body = true;
                continue;
            }
            if expected_owner
                .as_ref()
                .is_some_and(|expected| expected != &owner)
            {
                return Err(ServiceError::new(
                    409,
                    "mixed_upstream_state",
                    "Request contains state from different upstream owners",
                ));
            }
            expected_owner = Some(owner);
            preferred_account_id = Some(account);
        }
        if !portable_body {
            generation = rebuild::reference_generation(&connection, caller, &opaque)?;
        } else if preferred_account_id.is_none() {
            if let Some((account, _, accepted_generation)) = accepted_scope {
                preferred_account_id = Some(account);
                generation = Some(accepted_generation);
            }
        }
        let state_account_id = preferred_account_id.clone();
        // Durable affinities retain handoff history, but do not bypass the host's
        // caller/model-scoped TTL routing cache or its user-controlled switch.
        Ok(RequestIdentity {
            session_id,
            thread_id,
            parent_thread_id,
            preferred_account_id,
            state_account_id,
            caller: caller.to_owned(),
            turn_id,
            opaque,
            canonical_metadata: canonical_value,
            generation,
            rebuild_enabled: false,
            rebuild_input: None,
            rebuild_parameters: None,
            rebuild_base_revision: None,
            rebuild_new_generation: false,
            portable_body,
            detached_parent,
            detached_turn_states,
            unregistered_blobs,
        })
    }

    /// A fallback marker describes authorization, not a change of owner. Reuse
    /// existing opaque state only when this caller's evidence proves that the
    /// selected credential already owns it. This performs no publication.
    pub(crate) fn same_upstream_owner(
        &self,
        context: &RequestIdentity,
        account_id: &str,
        owner_id: &str,
    ) -> Result<bool, ServiceError> {
        if context.state_account_id.as_deref() != Some(account_id)
            || !context.unregistered_blobs.is_empty()
        {
            return Ok(false);
        }
        let reader = self.checkpoint_reader()?;
        let connection = reader.unchecked_transaction().map_err(storage)?;
        let scope = rebuild::accepted_scope(
            &connection, &context.caller, context.thread_id.as_deref(),
            Some(account_id), context.generation.as_deref(),
        )?;
        let accepted_here = scope.as_ref().is_some_and(|(account, owner, _)| {
            account == account_id && owner == owner_id
        });
        let mut proven = accepted_here;
        for reference in &context.opaque {
            if automatic_history::detached(context, reference) {
                continue;
            }
            let Some((account, owner)) = automatic_history::lookup(
                &connection, &context.caller, reference,
                context.thread_id.as_deref(), context.turn_id.as_deref(),
            )? else {
                return Ok(false);
            };
            if account == account_id && owner == owner_id {
                proven = true;
                continue;
            }
            if accepted_here && reference.kind == "encrypted"
                && rebuild::blob_accepted(
                    &connection, &context.caller, context.thread_id.as_deref(), reference,
                    scope.as_ref().expect("accepted scope checked"),
                )?
            {
                continue;
            }
            return Ok(false);
        }
        Ok(proven)
    }

    pub fn rewrite(
        &self,
        context: &RequestIdentity,
        account_id: &str,
        owner_id: &str,
        headers: &mut HeaderMap,
        body: &mut Value,
    ) -> Result<Binding, ServiceError> {
        if checked_identity(account_id)?.is_none() || checked_identity(owner_id)?.is_none() {
            return Err(invalid("An upstream account and stable owner are required"));
        }
        let mut connection = self.connection.lock().map_err(storage)?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage)?;

        for reference in &context.opaque {
            let found = automatic_history::lookup(
                &transaction,
                &context.caller,
                reference,
                context.thread_id.as_deref(),
                context.turn_id.as_deref(),
            )?;
            if automatic_history::detached(context, reference) {
                continue;
            }
            let Some((_, owner)) = found else {
                if context.portable_body
                    && automatic_history::is_unregistered(context, reference)
                    && (automatic_history::complete_input(headers, body)
                        || (context.rebuild_enabled
                            && context.rebuild_input.as_ref().is_some_and(|input| {
                                rebuild::replay_items(input) && rebuild::paired_tools(input)
                            })))
                {
                    continue;
                }
                return Err(automatic_history::incomplete());
            };
            if owner != owner_id
                && !(context.portable_body && reference.kind == "encrypted")
                && !(context.detached_parent && reference.kind == "parent_response")
                && !(reference.kind == "turn_state"
                    && context
                        .detached_turn_states
                        .iter()
                        .any(|stale| stale.digest == reference.digest))
            {
                return Err(ServiceError::new(
                    409,
                    "upstream_state_owner_changed",
                    "Existing upstream state cannot be transferred to a different account owner",
                ));
            }
        }
        // Commit both mappings and affinity before publishing any transformed request.
        let mut new_headers = headers.clone();
        let mut new_body = body.clone();
        supplement_identities(context, &mut new_headers, &mut new_body)?;
        if context.detached_parent {
            strip_parent_anchor(&mut new_headers, &mut new_body)?;
        }
        if !context.detached_turn_states.is_empty() {
            strip_stale_turn_states(context, &mut new_headers, &mut new_body)?;
        }
        let generation = rebuild::select_generation(&transaction, context, owner_id)?;
        let mapping_owner = rebuild::mapping_owner(owner_id, generation.as_ref());
        rewrite_headers(
            &transaction,
            &context.caller,
            &mapping_owner,
            &mut new_headers,
        )?;
        rewrite_body(&transaction, &context.caller, &mapping_owner, &mut new_body)?;
        let publish_affinity =
            rebuild::may_publish_affinity(&transaction, context, owner_id, generation.as_ref())?;
        for (kind, value) in [
            ("thread", context.thread_id.as_ref()),
            ("session", context.session_id.as_ref()),
        ] {
            if let Some(value) = value.filter(|_| {
                publish_affinity && (kind != "session" || context.parent_thread_id.is_none())
            }) {
                save_affinity(
                    &transaction,
                    &context.caller,
                    kind,
                    value,
                    account_id,
                    owner_id,
                )?;
            }
        }
        transaction.commit().map_err(storage)?;
        *headers = new_headers;
        *body = new_body;
        let pending_input = (generation.is_none() && !context.unregistered_blobs.is_empty())
            .then(|| Arc::new(automatic_history::Admission::default()));
        Ok(Binding {
            caller: context.caller.clone(),
            account_id: account_id.to_owned(),
            owner_id: owner_id.to_owned(),
            session_id: context.session_id.clone(),
            thread_id: context.thread_id.clone(),
            turn_id: context.turn_id.clone(),
            parent_thread_id: context.parent_thread_id.clone(),
            portable_body: context.portable_body,
            input_blobs: context
                .opaque
                .iter()
                .filter(|reference| reference.kind == "encrypted")
                .cloned()
                .collect(),
            generation,
            pending_input,
        })
    }

    pub fn observe_headers(
        &self,
        binding: &Binding,
        headers: &HeaderMap,
    ) -> Result<(), ServiceError> {
        let Some(value) = header_identity(headers, "x-codex-turn-state")? else {
            return Ok(());
        };
        self.record(
            binding,
            &[turn_state_reference(
                &value,
                binding.thread_id.as_deref(),
                binding.turn_id.as_deref(),
            )],
        )
    }

    pub fn observe_event(&self, binding: &Binding, event: &Value) -> Result<(), ServiceError> {
        rebuild::observe_output(binding, event)?;
        automatic_history::observe_output(binding, event)?;
        if matches!(
            event.get("type").and_then(Value::as_str),
            Some(
                "error"
                    | "response.error"
                    | "response.failed"
                    | "response.incomplete"
                    | "response.cancelled"
            )
        ) || matches!(
            event
                .get("response")
                .unwrap_or(event)
                .get("status")
                .and_then(Value::as_str),
            Some("failed" | "incomplete" | "cancelled")
        ) {
            binding.abort_rebuild();
        }
        if matches!(
            event.get("type").and_then(Value::as_str),
            Some("error" | "response.error")
        ) || event.get("error").is_some_and(|error| !error.is_null())
            || event
                .pointer("/response/error")
                .is_some_and(|error| !error.is_null())
        {
            binding.abort_rebuild();
            return Ok(());
        }
        let completed = successful_response_event(event).map_err(|error| {
            // A malformed terminal must never activate a new history generation.
            binding.abort_rebuild();
            error
        })?;
        if !completed
            && matches!(
                event.get("type").and_then(Value::as_str),
                Some("response.completed" | "response.done")
            )
        {
            binding.abort_rebuild();
        }
        let mut references = Vec::new();
        match event.get("type").and_then(Value::as_str) {
            Some("response.metadata") => {
                if let Some(headers) = event.get("headers").and_then(Value::as_object) {
                    for (name, value) in headers {
                        if name.eq_ignore_ascii_case("x-codex-turn-state") {
                            if let Some(value) = identity_value(Some(value))? {
                                references.push(turn_state_reference(
                                    &value,
                                    binding.thread_id.as_deref(),
                                    binding.turn_id.as_deref(),
                                ));
                            }
                        }
                    }
                }
            }
            Some("response.output_item.done") => {
                if let Some(item) = event.get("item") {
                    collect_encrypted_item(item, &mut references);
                }
            }
            Some(
                "response.created"
                | "response.completed"
                | "response.done"
                | "response.incomplete"
                | "response.failed"
                | "response"
                | "",
            ) => {
                let response = event.get("response").unwrap_or(event);
                if let Some(value) = identity_value(response.get("id"))? {
                    references.push(digest("parent_response", &value));
                }
                // Only a completed response is a valid continuation checkpoint. The
                // active transport holds created response IDs needed for interrupts.
                if completed {
                    if let Some(value) = identity_value(response.get("id"))? {
                        references.push(digest("response", &value));
                    }
                }
                collect_encrypted(response.get("output"), &mut references);
            }
            None => {
                let response = event.get("response").unwrap_or(event);
                if response.get("id").is_none() || response.get("output").is_none() {
                    return Ok(());
                }
                if let Some(value) = identity_value(response.get("id"))? {
                    references.push(digest("parent_response", &value));
                }
                if completed {
                    if let Some(value) = identity_value(response.get("id"))? {
                        references.push(digest("response", &value));
                    }
                }
                collect_encrypted(response.get("output"), &mut references);
            }
            _ => return Ok(()),
        }
        self.record(binding, &references)?;
        if completed {
            self.commit_rebuild(binding, event)?;
            self.commit_automatic_input(binding, event)?;
        }

        Ok(())
    }

    fn record(&self, binding: &Binding, references: &[OpaqueRef]) -> Result<(), ServiceError> {
        if references.is_empty() {
            return Ok(());
        }
        let mut connection = self.connection.lock().map_err(storage)?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage)?;
        let now = timestamp();
        // Routing expiry never erases server-observed ownership evidence.
        // Re-observation must still reject a different account for server resources.
        for reference in references {
            let existing: Option<(String, String)> = transaction
                .query_row(
                    "SELECT account,owner FROM native_opaque_v1 WHERE caller=?1 AND kind=?2 AND digest=?3",
                    params![binding.caller, reference.kind, reference.digest],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(storage)?;
            if reference.kind == "encrypted"
                && binding.portable_body
                && (existing.is_some()
                    || binding
                        .input_blobs
                        .iter()
                        .any(|blob| blob.digest == reference.digest))
            {
                // A successful upstream can echo a carried blob. Its original
                // provenance remains immutable; it is not a new server resource.
                continue;
            }
            if existing.as_ref().is_some_and(|(account, owner)| {
                account != &binding.account_id || owner != &binding.owner_id
            }) {
                return Err(ServiceError::new(
                    409,
                    "upstream_state_owner_conflict",
                    "Upstream state is already bound to a different owner",
                ));
            }
            transaction.execute(
                "INSERT INTO native_opaque_v1(caller,kind,digest,account,owner,session,thread,turn,expires)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)
                 ON CONFLICT(caller,kind,digest) DO UPDATE SET expires=MAX(native_opaque_v1.expires,excluded.expires)",
                params![binding.caller, reference.kind, reference.digest, binding.account_id, binding.owner_id, binding.session_id, binding.thread_id, binding.turn_id, if reference.kind == "turn_state" { now + OPAQUE_TTL_SECONDS } else { 0 }],
            ).map_err(storage)?;
            if let Some(generation) = binding
                .generation
                .as_ref()
                .filter(|_| reference.kind != "encrypted" || !binding.portable_body)
            {
                transaction.execute("INSERT OR IGNORE INTO native_rebuild_refs_v1(caller,kind,digest,generation) VALUES(?1,?2,?3,?4)", params![binding.caller,reference.kind,reference.digest,generation.id]).map_err(storage)?;
            }
        }
        transaction.commit().map_err(storage)
    }
}

fn successful_response_event(event: &Value) -> Result<bool, ServiceError> {
    if !matches!(
        event.get("type").and_then(Value::as_str),
        None | Some("" | "response" | "response.done" | "response.completed")
    ) {
        return Ok(false);
    }
    let response = event.get("response").unwrap_or(event);
    if response.get("status").and_then(Value::as_str) != Some("completed")
        || response.get("error").is_some_and(|error| !error.is_null())
        || event.get("error").is_some_and(|error| !error.is_null())
    {
        return Ok(false);
    }
    Ok(identity_value(response.get("id"))?.is_some_and(|id| !id.trim().is_empty()))
}

fn validate_opaque(
    connection: &Connection,
    caller: &str,
    reference: &OpaqueRef,
    thread: Option<&str>,
    turn: Option<&str>,
) -> Result<(String, String), ServiceError> {
    automatic_history::lookup(connection, caller, reference, thread, turn)?
        .ok_or_else(unknown_state)
}

fn unknown_state() -> ServiceError {
    ServiceError::new(409, "unknown_upstream_state", "This continuation belongs to an unknown or expired upstream session; start a new conversation in this service")
}

fn supplement_identities(
    context: &RequestIdentity,
    headers: &mut HeaderMap,
    body: &mut Value,
) -> Result<(), ServiceError> {
    // Session-Id may deliberately contain a cache alias (ephemeral forks and
    // internal requests). Thread-Id and x-client-request-id are projections of
    // the logical thread, however, and must never retain stale header values.
    if header_identity(headers, "session-id")?.is_none() {
        set_identity_header(headers, "session-id", context.session_id.as_deref())?;
    }
    // Forward only the canonical spelling through the caller/owner mapping;
    // never leave the original alias alongside the isolated upstream identity.
    headers.remove("session_id");
    for (name, value) in [
        ("thread-id", context.thread_id.as_deref()),
        ("x-client-request-id", context.thread_id.as_deref()),
    ] {
        set_identity_header(headers, name, value)?;
    }
    let object = body
        .as_object_mut()
        .ok_or_else(|| invalid("Responses body must be an object"))?;
    let flat = object
        .entry("client_metadata")
        .or_insert_with(|| Value::Object(Default::default()));
    if flat.is_null() {
        *flat = Value::Object(Default::default());
    }
    let flat = flat
        .as_object_mut()
        .ok_or_else(|| invalid("Client metadata must be an object"))?;
    for (name, value) in [
        ("session_id", context.session_id.as_ref()),
        ("thread_id", context.thread_id.as_ref()),
    ] {
        if let Some(value) = value {
            flat.insert(name.to_owned(), Value::String(value.clone()));
        }
    }
    if let Some(canonical) = &context.canonical_metadata {
        // Official client_metadata and compatibility headers are projections of
        // the canonical body snapshot, not independent identity sources. Memory
        // requests intentionally omit thread/window identity from that snapshot,
        // while retaining the flat transport fields; do not fill the omissions.
        for (source, target) in [
            ("turn_id", "turn_id"),
            ("parent_turn_id", "parent_turn_id"),
            ("root_turn_id", "root_turn_id"),
            ("parent_thread_id", "x-codex-parent-thread-id"),
        ] {
            match identity_value(canonical.get(source))? {
                Some(value) => {
                    flat.insert(target.to_owned(), Value::String(value));
                }
                None => {
                    flat.remove(target);
                }
            }
        }
        set_identity_header(
            headers,
            "x-codex-parent-thread-id",
            identity_value(canonical.get("parent_thread_id"))?.as_deref(),
        )?;
        if canonical.get("request_kind").and_then(Value::as_str) != Some("memory") {
            let window = identity_value(canonical.get("window_id"))?;
            match &window {
                Some(value) => {
                    flat.insert("x-codex-window-id".to_owned(), Value::String(value.clone()));
                }
                None => {
                    flat.remove("x-codex-window-id");
                }
            }
            set_identity_header(headers, "x-codex-window-id", window.as_deref())?;
        }
        if flat.contains_key("x-codex-turn-metadata") {
            flat.insert(
                "x-codex-turn-metadata".to_owned(),
                Value::String(serde_json::to_string(canonical).map_err(storage)?),
            );
        }
        if headers.contains_key("x-codex-turn-metadata") {
            let mut compatibility = canonical.clone();
            compatibility
                .as_object_mut()
                .unwrap()
                .remove("tool_namespaces_info");
            headers.insert(
                "x-codex-turn-metadata",
                HeaderValue::from_str(&ascii_json(&compatibility)?)
                    .map_err(|_| invalid("Invalid turn metadata header"))?,
            );
        }
    }
    Ok(())
}

fn set_identity_header(
    headers: &mut HeaderMap,
    name: &'static str,
    value: Option<&str>,
) -> Result<(), ServiceError> {
    if let Some(value) = value {
        headers.insert(
            name,
            HeaderValue::from_str(value).map_err(|_| invalid("Invalid identity header"))?,
        );
    } else {
        headers.remove(name);
    }
    Ok(())
}

fn ascii_json(value: &Value) -> Result<String, ServiceError> {
    let serialized = serde_json::to_string(value).map_err(storage)?;
    let mut result = String::with_capacity(serialized.len());
    for character in serialized.chars() {
        if character.is_ascii() {
            result.push(character);
        } else {
            for code_unit in character.encode_utf16(&mut [0; 2]) {
                use std::fmt::Write;
                write!(&mut result, "\\u{code_unit:04x}").map_err(storage)?;
            }
        }
    }
    Ok(result)
}

fn map_identity(
    transaction: &Transaction<'_>,
    caller: &str,
    owner: &str,
    original: &str,
) -> Result<String, ServiceError> {
    if checked_identity(original)?.is_none() {
        return Ok(original.to_owned());
    }
    if let Some(mapped) = transaction
        .query_row(
            "SELECT mapped FROM native_identity_v1 WHERE caller=?1 AND owner=?2 AND original=?3",
            params![caller, owner, original],
            |row| row.get(0),
        )
        .optional()
        .map_err(storage)?
    {
        return Ok(mapped);
    }
    let mapped = uuid::Uuid::now_v7().to_string();
    transaction
        .execute(
            "INSERT INTO native_identity_v1(caller,owner,original,mapped) VALUES(?1,?2,?3,?4)",
            params![caller, owner, original, mapped],
        )
        .map_err(storage)?;
    Ok(mapped)
}

fn map_window(
    transaction: &Transaction<'_>,
    caller: &str,
    owner: &str,
    value: &str,
) -> Result<String, ServiceError> {
    if let Some((thread, number)) = value.rsplit_once(':') {
        if !thread.is_empty()
            && !number.is_empty()
            && number.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Ok(format!(
                "{}:{number}",
                map_identity(transaction, caller, owner, thread)?
            ));
        }
    }
    map_identity(transaction, caller, owner, value)
}

fn strip_stale_turn_states(
    context: &RequestIdentity,
    headers: &mut HeaderMap,
    body: &mut Value,
) -> Result<(), ServiceError> {
    let stale = |value: Option<&Value>| -> Result<bool, ServiceError> {
        Ok(identity_value(value)?.is_some_and(|value| {
            let reference = turn_state_reference(
                &value,
                context.thread_id.as_deref(),
                context.turn_id.as_deref(),
            );
            context
                .detached_turn_states
                .iter()
                .any(|stale| stale.digest == reference.digest)
        }))
    };
    if let Some(value) = header_identity(headers, "x-codex-turn-state")? {
        if stale(Some(&Value::String(value)))? {
            headers.remove("x-codex-turn-state");
        }
    }
    if let Some(metadata) = body
        .get_mut("client_metadata")
        .and_then(Value::as_object_mut)
    {
        if stale(metadata.get("x-codex-turn-state"))? {
            metadata.remove("x-codex-turn-state");
        }
        if let Some(raw) = metadata
            .get("x-codex-turn-metadata")
            .and_then(Value::as_str)
        {
            let mut canonical = parse_metadata(raw)?;
            if stale(canonical.get("x-codex-turn-state"))? {
                canonical
                    .as_object_mut()
                    .unwrap()
                    .remove("x-codex-turn-state");
            }
            metadata.insert(
                "x-codex-turn-metadata".into(),
                Value::String(ascii_json(&canonical)?),
            );
        }
    }
    if let Some(raw) = headers.get("x-codex-turn-metadata") {
        let mut canonical = parse_metadata(raw.to_str().map_err(storage)?)?;
        if stale(canonical.get("x-codex-turn-state"))? {
            canonical
                .as_object_mut()
                .unwrap()
                .remove("x-codex-turn-state");
        }
        headers.insert(
            "x-codex-turn-metadata",
            HeaderValue::from_str(&ascii_json(&canonical)?).map_err(storage)?,
        );
    }
    Ok(())
}

fn strip_parent_anchor(headers: &mut HeaderMap, body: &mut Value) -> Result<(), ServiceError> {
    if let Some(metadata) = body
        .get_mut("client_metadata")
        .and_then(Value::as_object_mut)
    {
        metadata.remove("parent_response_id");
        if let Some(raw) = metadata
            .get("x-codex-turn-metadata")
            .and_then(Value::as_str)
        {
            let mut canonical = parse_metadata(raw)?;
            canonical
                .as_object_mut()
                .unwrap()
                .remove("parent_response_id");
            metadata.insert(
                "x-codex-turn-metadata".into(),
                Value::String(ascii_json(&canonical)?),
            );
        }
    }
    if let Some(raw) = headers.get("x-codex-turn-metadata") {
        let mut canonical = parse_metadata(raw.to_str().map_err(storage)?)?;
        canonical
            .as_object_mut()
            .unwrap()
            .remove("parent_response_id");
        headers.insert(
            "x-codex-turn-metadata",
            HeaderValue::from_str(&ascii_json(&canonical)?).map_err(storage)?,
        );
    }
    Ok(())
}

fn parent_mapping_owner(
    transaction: &Transaction<'_>,
    caller: &str,
    owner: &str,
    parent: &str,
) -> Result<String, ServiceError> {
    let stable_owner = owner.split(":native-generation:").next().unwrap_or(owner);
    let active: Option<(String, String)> = transaction
        .query_row(
            "SELECT owner,generation FROM native_rebuild_active_v1 WHERE caller=?1 AND thread=?2",
            params![caller, parent],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(storage)?;
    Ok(
        match active.filter(|(parent_owner, _)| parent_owner == stable_owner) {
            Some((_, generation)) if !generation.is_empty() => {
                format!("{stable_owner}:native-generation:{generation}")
            }
            _ => stable_owner.to_owned(),
        },
    )
}

fn map_field(
    transaction: &Transaction<'_>,
    caller: &str,
    owner: &str,
    object: &mut Value,
    key: &str,
    window: bool,
) -> Result<(), ServiceError> {
    if let Some(original) = identity_value(object.get(key))? {
        let parent_owner;
        let owner = if matches!(
            key,
            "parent_thread_id" | "x-codex-parent-thread-id" | "forked_from_thread_id"
        ) {
            parent_owner = parent_mapping_owner(transaction, caller, owner, &original)?;
            parent_owner.as_str()
        } else {
            owner
        };
        let mapped = if window {
            map_window(transaction, caller, owner, &original)?
        } else {
            map_identity(transaction, caller, owner, &original)?
        };
        object[key] = Value::String(mapped);
    }
    Ok(())
}

fn rewrite_canonical(
    transaction: &Transaction<'_>,
    caller: &str,
    owner: &str,
    metadata: &mut Value,
) -> Result<(), ServiceError> {
    for key in [
        "session_id",
        "thread_id",
        "turn_id",
        "parent_thread_id",
        "forked_from_thread_id",
        "parent_turn_id",
        "root_turn_id",
        "context_window_id",
        "first_window_id",
        "previous_window_id",
        "prompt_cache_key",
    ] {
        map_field(transaction, caller, owner, metadata, key, false)?;
    }
    map_field(transaction, caller, owner, metadata, "window_id", true)
}

fn rewrite_headers(
    transaction: &Transaction<'_>,
    caller: &str,
    owner: &str,
    headers: &mut HeaderMap,
) -> Result<(), ServiceError> {
    for name in [
        "session-id",
        "thread-id",
        "x-client-request-id",
        "x-codex-parent-thread-id",
        "conversation_id",
        "x-codex-window-id",
    ] {
        if let Some(original) = header_identity(headers, name)? {
            let parent_owner;
            let owner = if name == "x-codex-parent-thread-id" {
                parent_owner = parent_mapping_owner(transaction, caller, owner, &original)?;
                parent_owner.as_str()
            } else {
                owner
            };
            let mapped = if name == "x-codex-window-id" {
                map_window(transaction, caller, owner, &original)?
            } else {
                map_identity(transaction, caller, owner, &original)?
            };
            headers.insert(
                name,
                HeaderValue::from_str(&mapped)
                    .map_err(|_| invalid("Invalid mapped identity header"))?,
            );
        }
    }
    if let Some(raw) = headers.get("x-codex-turn-metadata") {
        let mut metadata = parse_metadata(
            raw.to_str()
                .map_err(|_| invalid("Invalid turn metadata header"))?,
        )?;
        rewrite_canonical(transaction, caller, owner, &mut metadata)?;
        let raw = ascii_json(&metadata)?;
        headers.insert(
            "x-codex-turn-metadata",
            HeaderValue::from_str(&raw)
                .map_err(|_| invalid("Invalid mapped turn metadata header"))?,
        );
    }
    Ok(())
}

fn rewrite_body(
    transaction: &Transaction<'_>,
    caller: &str,
    owner: &str,
    body: &mut Value,
) -> Result<(), ServiceError> {
    map_field(transaction, caller, owner, body, "prompt_cache_key", false)?;
    if let Some(metadata) = body.get_mut("client_metadata") {
        for key in [
            "session_id",
            "thread_id",
            "turn_id",
            "parent_turn_id",
            "root_turn_id",
            "x-codex-parent-thread-id",
        ] {
            map_field(transaction, caller, owner, metadata, key, false)?;
        }
        map_field(
            transaction,
            caller,
            owner,
            metadata,
            "x-codex-window-id",
            true,
        )?;
        if let Some(raw) = metadata.get("x-codex-turn-metadata") {
            let mut canonical = parse_metadata(
                raw.as_str()
                    .ok_or_else(|| invalid("Turn metadata must be a JSON string"))?,
            )?;
            rewrite_canonical(transaction, caller, owner, &mut canonical)?;
            metadata["x-codex-turn-metadata"] =
                Value::String(serde_json::to_string(&canonical).map_err(storage)?);
        }
    }
    Ok(())
}

fn save_affinity(
    transaction: &Transaction<'_>,
    caller: &str,
    kind: &str,
    original: &str,
    account: &str,
    owner: &str,
) -> Result<(), ServiceError> {
    transaction.execute(
        "INSERT INTO native_affinity_v1(caller,kind,original,account,owner) VALUES(?1,?2,?3,?4,?5)
         ON CONFLICT(caller,kind,original) DO UPDATE SET account=excluded.account, owner=excluded.owner",
        params![caller, kind, original, account, owner],
    ).map_err(storage)?;
    Ok(())
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "identity_capacity_tests.rs"]
mod capacity_tests;

#[cfg(test)]
#[path = "identity_agent_communication_tests.rs"]
mod agent_communication_tests;

#[cfg(test)]
#[path = "identity_automatic_history_tests.rs"]
mod automatic_history_tests;
