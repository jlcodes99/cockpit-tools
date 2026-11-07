//! A response reference is a continuation capability, not an API-key identity.
//! Only the main API service enables this policy. Authorization/routing/billing
//! always use the current Caller; administrative history access stays isolated.
use super::*;
use std::collections::HashSet;

type Row = Vec<rusqlite::types::Value>;
struct CopyRows {
    table: &'static str,
    columns: Vec<String>,
    rows: Vec<Row>,
}

impl CopyRows {
    fn read(
        connection: &Connection,
        table: &'static str,
        predicate: &str,
        args: &[&dyn rusqlite::ToSql],
    ) -> Result<Self, ServiceError> {
        let mut query = connection
            .prepare(&format!("SELECT * FROM {table} WHERE {predicate}"))
            .map_err(storage)?;
        let columns = query
            .column_names()
            .iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>();
        let width = columns.len();
        let rows = query
            .query_map(args, |row| {
                (0..width)
                    .map(|i| row.get(i))
                    .collect::<rusqlite::Result<Row>>()
            })
            .map_err(storage)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(storage)?;
        Ok(Self {
            table,
            columns,
            rows,
        })
    }
    fn publish(&self, transaction: &Transaction<'_>, caller: &str) -> Result<(), ServiceError> {
        let caller_index = self
            .columns
            .iter()
            .position(|name| name == "caller")
            .unwrap();
        let placeholders = vec!["?"; self.columns.len()].join(",");
        let mut insert = transaction
            .prepare_cached(&format!(
                "INSERT OR IGNORE INTO {} ({}) VALUES ({placeholders})",
                self.table,
                self.columns.join(",")
            ))
            .map_err(storage)?;
        for saved in &self.rows {
            let mut row = saved.clone();
            row[caller_index] = caller.to_owned().into();
            insert
                .execute(rusqlite::params_from_iter(row))
                .map_err(storage)?;
        }
        Ok(())
    }
}

impl IdentityStore {
    pub(crate) fn set_shared_continuation_references(&self, enabled: bool) {
        self.shared_continuation_references
            .store(enabled, Ordering::Release);
    }

    /// Called on the bounded blocking worker, before inspection/partial recovery.
    /// Copy only references the client possesses and the checkpoint explicitly
    /// addressed by previous_response_id. Never search by a guessed thread alone,
    /// migrate a whole caller, replace newer target records or reactivate a key.
    pub(super) fn adopt_client_continuation(
        &self,
        caller: &str,
        headers: &HeaderMap,
        body: &Value,
    ) -> Result<(), ServiceError> {
        if !self.shared_continuation_references.load(Ordering::Acquire) {
            return Ok(());
        }
        let canonical = canonical_metadata(headers, body)?;
        let flat = body.get("client_metadata");
        let thread = primary_identity(
            canonical.as_ref().and_then(|v| v.get("thread_id")),
            flat.and_then(|v| v.get("thread_id")),
            headers,
            "thread-id",
        )?;
        let turn = if let Some(canonical) = &canonical {
            identity_value(canonical.get("turn_id"))?
        } else {
            identity_value(flat.and_then(|v| v.get("turn_id")))?
        };
        let previous = identity_value(body.get("previous_response_id"))?;
        let mut references = Vec::new();
        collect_encrypted(body.get("input"), &mut references);
        if let Some(previous) = &previous {
            references.push(digest("response", previous));
            references.push(digest("parent_response", previous));
        }
        for value in [
            flat.and_then(|v| v.get("parent_response_id")),
            canonical.as_ref().and_then(|v| v.get("parent_response_id")),
        ] {
            if let Some(value) = identity_value(value)? {
                references.push(digest("parent_response", &value));
            }
        }
        let turn_states: Vec<String> = [
            header_identity(headers, "x-codex-turn-state")?,
            identity_value(flat.and_then(|v| v.get("x-codex-turn-state")))?,
            identity_value(canonical.as_ref().and_then(|v| v.get("x-codex-turn-state")))?,
        ]
        .into_iter()
        .flatten()
        .collect();
        if references.is_empty() && turn_states.is_empty() {
            return Ok(());
        }
        let reader = self.checkpoint_reader()?;
        let snapshot = reader.unchecked_transaction().map_err(storage)?;
        // Match inspection's legacy session alias and omitted-thread restoration
        // before hashing a turn token; the hash includes its thread/turn scope.
        let session = primary_identity(
            canonical.as_ref().and_then(|v| v.get("session_id")),
            flat.and_then(|v| v.get("session_id")),
            headers,
            "session-id",
        )?
        .or(header_identity(headers, "session_id")?);
        let mut routing_thread = thread.clone().or(session);
        if routing_thread.is_none() {
            if let Some(previous) = &previous {
                routing_thread = snapshot.query_row(
                    "SELECT thread FROM native_opaque_v1 WHERE kind IN ('response','parent_response') AND digest=?1 ORDER BY caller=?2 DESC LIMIT 1",
                    params![digest("response",previous).digest,caller],|r|r.get::<_,Option<String>>(0),
                ).optional().map_err(storage)?.flatten();
            }
        }
        for value in turn_states {
            references.push(turn_state_reference(
                &value,
                routing_thread.as_deref(),
                turn.as_deref(),
            ));
        }
        let mut copies = Vec::new();
        let mut sources = HashSet::new();
        let mut seen = HashSet::new();
        // Read/validate compressed history outside the publication lock. A single
        // read snapshot pairs checkpoint manifests with their immutable chunks.
        let mut index = 0;
        while index < references.len() {
            if references.len() > MAX_REQUEST_OPAQUE_REFS {
                return Err(ServiceError::new(413, "continuation_history_too_large", "Conversation history exceeds the per-request reference limit; existing history was preserved"));
            }
            let reference = &references[index];
            index += 1;
            if !seen.insert((reference.kind, reference.digest.clone())) {
                continue;
            }
            let local: bool = snapshot.query_row("SELECT EXISTS(SELECT 1 FROM native_opaque_v1 WHERE caller=?1 AND kind=?2 AND digest=?3)",params![caller,reference.kind,reference.digest],|r|r.get(0)).map_err(storage)?;
            if local {
                continue;
            }
            let origin: Option<(String,Option<String>)> = snapshot.query_row("SELECT caller,thread FROM native_opaque_v1 WHERE kind=?1 AND digest=?2 AND caller!=?3 ORDER BY caller LIMIT 1",params![reference.kind,reference.digest,caller],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(storage)?;
            let Some((source, source_thread)) = origin else {
                continue;
            };
            // Input ciphertext is already supplied by the client and can occur in
            // forked histories. Server-held history, however, keeps its thread.
            if reference.kind == "response"
                && thread
                    .as_ref()
                    .zip(source_thread.as_ref())
                    .is_some_and(|(a, b)| a != b)
            {
                return Err(ServiceError::new(
                    409,
                    "upstream_state_thread_mismatch",
                    "Response reference belongs to a different conversation",
                ));
            }
            copies.push(CopyRows::read(
                &snapshot,
                "native_opaque_v1",
                "caller=? AND kind=? AND digest=?",
                &[&source, &reference.kind, &reference.digest],
            )?);
            copies.push(CopyRows::read(
                &snapshot,
                "native_rebuild_refs_v1",
                "caller=? AND kind=? AND digest=?",
                &[&source, &reference.kind, &reference.digest],
            )?);
            if let Some(previous) = &previous {
                if matches!(reference.kind, "response" | "parent_response")
                    && reference.digest == digest(reference.kind, previous).digest
                    && sources.insert(source.clone())
                {
                    copy_checkpoint(&snapshot, &source, previous, &mut copies, &mut references)?;
                }
            }
        }
        if copies.is_empty() {
            return Ok(());
        }
        // No provenance/body rewriting: source rows survive deletion of their key.
        // Concurrent admissions are idempotent and current caller state wins.
        let mut connection = self.connection.lock().map_err(storage)?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage)?;
        for copy in copies {
            copy.publish(&transaction, caller)?;
        }
        transaction.commit().map_err(storage)
    }
}

fn copy_checkpoint(
    connection: &Connection,
    source: &str,
    response: &str,
    copies: &mut Vec<CopyRows>,
    references: &mut Vec<OpaqueRef>,
) -> Result<(), ServiceError> {
    for table in [
        "native_rebuild_checkpoints_v1",
        "native_rebuild_history_v1",
        "native_partial_history_v1",
    ] {
        let copy = CopyRows::read(
            connection,
            table,
            "caller=? AND response=?",
            &[&source, &response],
        )?;
        for row in &copy.rows {
            if let Some(i) = copy.columns.iter().position(|c| c == "manifest") {
                let rusqlite::types::Value::Text(raw) = &row[i] else {
                    return Err(storage("invalid history manifest"));
                };
                let manifest: Vec<String> = serde_json::from_str(raw).map_err(storage)?;
                for digest in manifest {
                    let item = CopyRows::read(
                        connection,
                        "native_rebuild_items_v1",
                        "caller=? AND digest=?",
                        &[&source, &digest],
                    )?;
                    if item.rows.len() != 1 {
                        return Err(storage("missing history item"));
                    }
                    let rusqlite::types::Value::Blob(bytes) = &item.rows[0][2] else {
                        return Err(storage("invalid history item"));
                    };
                    let raw = zstd::stream::decode_all(bytes.as_slice()).map_err(storage)?;
                    if format!("{:x}", Sha256::digest(&raw)) != digest {
                        return Err(storage("history item digest mismatch"));
                    }
                    let value: Value = serde_json::from_slice(&raw).map_err(storage)?;
                    collect_encrypted_item(&value, references);
                    copies.push(item);
                }
            } else if let Some(i) = copy.columns.iter().position(|c| c == "transcript") {
                let rusqlite::types::Value::Text(raw) = &row[i] else {
                    return Err(storage("invalid history transcript"));
                };
                let items: Value = serde_json::from_str(raw).map_err(storage)?;
                collect_encrypted(Some(&items), references);
            }
        }
        copies.push(copy);
    }
    Ok(())
}

#[cfg(test)]
#[path = "identity_key_continuity_tests.rs"]
mod tests;
