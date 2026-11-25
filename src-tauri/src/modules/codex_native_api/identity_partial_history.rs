//! Recover explicit, observed incomplete terminals without claiming successful
//! responses. Self-contained rows are strictly bounded inline on write.
//! All database/serialization work runs on the existing bounded blocking pool.
use super::*;
use rebuild::checkpoint_store;

pub(crate) const MAX_PARTIAL_HISTORY_ROWS: i64 = 512;
pub(crate) const MAX_PARTIAL_CACHE_BYTES: i64 = 32 * 1024 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct PartialHistory {
    pub body: Value,
    pub account_id: String,
}

impl RequestIdentity {
    pub(crate) fn prefer_partial_owner(&mut self, account: &str) -> Result<(), ServiceError> {
        if self
            .state_account_id
            .as_deref()
            .is_some_and(|current| current != account)
        {
            return Err(ServiceError::new(
                409,
                "continuation_owner_mismatch",
                "Partial history conflicts with another upstream resource owner",
            ));
        }
        self.state_account_id = Some(account.to_owned());
        self.preferred_account_id = Some(account.to_owned());
        Ok(())
    }
}

pub(super) fn has_column(
    connection: &Connection,
    table: &str,
    column: &str,
) -> rusqlite::Result<bool> {
    Ok(connection
        .prepare(&format!("PRAGMA table_info({table})"))?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .any(|name| name == column))
}

pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    let table_exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='native_partial_history_v1')",
        [],
        |row| row.get(0),
    )?;
    if table_exists && !has_column(connection, "native_partial_history_v1", "transcript")? {
        // Legacy pre-slimming schema stored `manifest` referencing `native_rebuild_items_v1`.
        // Salvage is performed before `initialize` when items exist; drop any remaining legacy schema.
        connection.execute_batch("DROP TABLE IF EXISTS native_partial_history_v1;")?;
    }
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS native_partial_history_v1 (
            caller TEXT NOT NULL, response TEXT NOT NULL, thread TEXT NOT NULL,
            session TEXT, account TEXT NOT NULL, owner TEXT NOT NULL,
            transcript TEXT NOT NULL, parameters TEXT, bytes INTEGER NOT NULL,
            created INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY(caller,response));
         CREATE INDEX IF NOT EXISTS native_partial_history_v1_response
            ON native_partial_history_v1(response);
         CREATE INDEX IF NOT EXISTS native_partial_history_v1_evict
            ON native_partial_history_v1(created ASC, response ASC);",
    )?;
    super::maintenance::ensure_created_column(connection, "native_partial_history_v1")
}

pub(super) fn evict_partial_cache(transaction: &Transaction<'_>) -> Result<(), ServiceError> {
    let (count, bytes): (i64, i64) = transaction
        .query_row(
            "SELECT COUNT(*),COALESCE(SUM(LENGTH(transcript) + COALESCE(LENGTH(parameters),0)),0)
             FROM native_partial_history_v1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(storage)?;
    if count <= MAX_PARTIAL_HISTORY_ROWS && bytes <= MAX_PARTIAL_CACHE_BYTES {
        return Ok(());
    }
    transaction
        .execute(
            "DELETE FROM native_partial_history_v1 WHERE rowid IN (
               SELECT rowid FROM (
                 SELECT rowid,
                        COALESCE(SUM(LENGTH(transcript) + COALESCE(LENGTH(parameters),0)) OVER (
                          ORDER BY created, rowid
                          ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING
                        ), 0) AS prev_bytes,
                        COUNT(*) OVER (
                          ORDER BY created, rowid
                          ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING
                        ) AS prev_count
                 FROM native_partial_history_v1
               )
               WHERE ?1 - prev_count > ?3 OR ?2 - prev_bytes > ?4
             )",
            params![
                count,
                bytes,
                MAX_PARTIAL_HISTORY_ROWS,
                MAX_PARTIAL_CACHE_BYTES
            ],
        )
        .map_err(storage)?;
    Ok(())
}

impl Binding {
    // Deltas do not need SQLite work, but an empty terminal after visible output
    // cannot prove that the output is complete enough for a later conversation.
    pub(crate) fn note_partial_content(&self, event: &Value) {
        if event["type"]
            .as_str()
            .is_some_and(|kind| kind.ends_with(".delta"))
        {
            self.partial_content_seen.store(true, Ordering::Release);
        }
    }
}

impl IdentityStore {
    pub(super) fn commit_partial_history(
        &self,
        binding: &Binding,
        event: &Value,
        replay: Option<&Value>,
    ) -> Result<(), ServiceError> {
        let Some(generation) = binding.generation.as_ref() else {
            return Ok(());
        };
        if generation.cancelled.load(Ordering::Acquire)
            || binding.thread_id.is_none()
            || event.pointer("/response/status").and_then(Value::as_str) != Some("incomplete")
            || event.get("error").is_some_and(|value| !value.is_null())
            || event
                .pointer("/response/error")
                .is_some_and(|value| !value.is_null())
        {
            return Ok(());
        }
        let Some(id) = identity_value(event.pointer("/response/id"))? else {
            return Ok(());
        };
        let Some(output) = event.pointer("/response/output").and_then(Value::as_array) else {
            return Ok(());
        };
        let Some(input) = generation
            .input
            .as_ref()
            .or_else(|| replay?.get("input")?.as_array())
        else {
            return Ok(());
        };
        if (output.is_empty() && binding.partial_content_seen.load(Ordering::Acquire))
            || !rebuild::replay_items(input)
            || !rebuild::paired_tools(input)
            || !rebuild::replay_items(output)
            || output.iter().any(|item| {
                matches!(
                    item["type"].as_str(),
                    Some("function_call" | "custom_tool_call")
                ) && (!super::super::output_items::complete_tool_call(item)
                    || item
                        .get("status")
                        .is_some_and(|status| status != "completed"))
            })
        {
            return Ok(());
        }
        if !generation
            .output
            .lock()
            .map_err(storage)?
            .iter()
            .all(|(index, digest)| {
                output.get(*index as usize).is_some_and(|item| {
                    format!("{:x}", Sha256::digest(item.to_string().as_bytes())) == *digest
                })
            })
        {
            return Ok(());
        }
        let parameters = generation.parameters.clone().or_else(|| {
            replay.map(|body| {
                Value::Object(
                    [
                        "instructions",
                        "tools",
                        "tool_choice",
                        "parallel_tool_calls",
                        "text",
                        "reasoning",
                    ]
                    .into_iter()
                    .filter_map(|key| {
                        body.get(key)
                            .filter(|v| !v.is_null())
                            .map(|v| (key.into(), v.clone()))
                    })
                    .collect(),
                )
            })
        });
        // Serialize/compress before the short publication transaction. No account
        // state or identity mutex is held while processing large transcripts.
        let encoded =
            checkpoint_store::Encoded::prepare(input, output, parameters.as_ref())?;
        let mut connection = self.connection.lock().map_err(storage)?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage)?;
        if generation.cancelled.load(Ordering::Acquire) {
            return Ok(());
        }
        let parent = digest("parent_response", &id);
        let origin: Option<(String, String)> = transaction.query_row(
            "SELECT account,owner FROM native_opaque_v1 WHERE caller=?1 AND kind='parent_response' AND digest=?2",
            params![binding.caller,parent.digest], |row| Ok((row.get(0)?,row.get(1)?)),
        ).optional().map_err(storage)?;
        if origin.as_ref().is_some_and(|(account, owner)| {
            account != &binding.account_id || owner != &binding.owner_id
        }) {
            return Err(ServiceError::new(
                409,
                "upstream_state_owner_conflict",
                "Upstream state is already bound to a different owner",
            ));
        }
        // Publish the observed parent and its partial transcript atomically,
        // including upstreams that omit response.created before the terminal.
        transaction.execute(
            "INSERT OR IGNORE INTO native_opaque_v1(caller,kind,digest,account,owner,session,thread,turn,expires) VALUES(?1,'parent_response',?2,?3,?4,?5,?6,?7,0)",
            params![binding.caller,parent.digest,binding.account_id,binding.owner_id,binding.session_id,binding.thread_id,binding.turn_id],
        ).map_err(storage)?;
        encoded.publish_partial(&transaction, binding, &id)?;
        // Cancellation and publication contend on the same request gate. A
        // failed/cancelled candidate can never publish after its caller left.
        if generation
            .cancelled
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(());
        }
        transaction.commit().map_err(storage)
    }

    pub(crate) fn expand_partial_history(
        &self,
        caller: &str,
        headers: &HeaderMap,
        body: &Value,
        max_bytes: usize,
    ) -> Result<Option<PartialHistory>, ServiceError> {
        let Some(previous) = identity_value(body.get("previous_response_id"))? else {
            return Ok(None);
        };
        if headers.contains_key("conversation_id")
            || ["conversation", "conversation_id"]
                .iter()
                .any(|key| body.get(key).is_some_and(|v| !v.is_null()))
        {
            return Ok(None);
        }
        self.adopt_client_continuation(caller, headers, body)?;
        let reader = self.checkpoint_reader()?;
        let connection = reader.unchecked_transaction().map_err(storage)?;
        type Row = (
            String,
            Option<String>,
            String,
            String,
            String,
            Option<String>,
            i64,
        );
        let saved: Option<Row> = connection.query_row(
            "SELECT thread,session,account,owner,transcript,parameters,bytes FROM native_partial_history_v1 WHERE caller=?1 AND response=?2",
            params![caller,previous], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)),
        ).optional().map_err(storage)?;
        let Some((thread, session, account, owner, raw_transcript, parameters, bytes)) = saved
        else {
            let foreign: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM native_partial_history_v1 WHERE response=?1 AND caller!=?2)",params![previous,caller],|r|r.get(0)).map_err(storage)?;
            return if foreign {
                Err(ServiceError::new(
                    409,
                    "upstream_state_caller_mismatch",
                    "Partial history belongs to another API caller",
                ))
            } else {
                Ok(None)
            };
        };
        let canonical = canonical_metadata(headers, body)?;
        let flat = body.get("client_metadata");
        let supplied_thread = primary_identity(
            canonical.as_ref().and_then(|v| v.get("thread_id")),
            flat.and_then(|v| v.get("thread_id")),
            headers,
            "thread-id",
        )?;
        let supplied_session = primary_identity(
            canonical.as_ref().and_then(|v| v.get("session_id")),
            flat.and_then(|v| v.get("session_id")),
            headers,
            "session-id",
        )?
        .or(header_identity(headers, "session_id")?);
        if supplied_thread
            .as_ref()
            .is_some_and(|value| value != &thread)
            || supplied_session
                .as_ref()
                .is_some_and(|value| session.as_ref().is_some_and(|session| value != session))
        {
            return Err(ServiceError::new(
                409,
                "upstream_state_thread_mismatch",
                "Partial history belongs to another conversation",
            ));
        }
        // Recovery must agree with the immutable observed parent ownership.
        let origin: Option<(String,String)> = connection.query_row("SELECT account,owner FROM native_opaque_v1 WHERE caller=?1 AND kind='parent_response' AND digest=?2",params![caller,digest("parent_response",&previous).digest],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(storage)?;
        if origin.as_ref() != Some(&(account.clone(), owner)) {
            return Ok(None);
        }
        if max_bytes != usize::MAX && bytes > max_bytes as i64 {
            return Err(history_too_large());
        }
        let incoming = match body.get("input") {
            Some(Value::Array(items)) => std::borrow::Cow::Borrowed(items),
            Some(Value::String(text)) => {
                std::borrow::Cow::Owned(vec![serde_json::json!({"role":"user","content":text})])
            }
            None | Some(Value::Null) => std::borrow::Cow::Owned(Vec::new()),
            _ => return Ok(None),
        };
        let transcript = checkpoint_store::decode_transcript(&raw_transcript)?;
        let Some(input) = rebuild::merge_checkpoint(transcript, &incoming) else {
            return Ok(None);
        };
        if !rebuild::replay_items(&input) || !rebuild::paired_tools(&input) {
            return Ok(None);
        }
        let mut full = body.clone();
        full["input"] = Value::Array(input);
        full.as_object_mut()
            .expect("request object")
            .remove("previous_response_id");
        if let Some(parameters) = parameters {
            let decoded_params = checkpoint_store::decode_text(&parameters)?;
            let parameters: serde_json::Map<String, Value> =
                serde_json::from_str(&decoded_params).map_err(storage)?;
            for (key, value) in parameters {
                let value = rebuild::inherited_parameter(&key, value);
                full.as_object_mut().unwrap().entry(key).or_insert(value);
            }
        }
        if !rebuild::replay_tools(full.get("tools")) {
            return Ok(None);
        }
        restore_identity(&mut full, "thread_id", &thread);
        if let Some(session) = &session {
            restore_identity(&mut full, "session_id", session);
        }
        if max_bytes != usize::MAX && serde_json::to_vec(&full).map_err(storage)?.len() > max_bytes
        {
            return Err(history_too_large());
        }
        Ok(Some(PartialHistory {
            body: full,
            account_id: account,
        }))
    }
}

fn restore_identity(body: &mut Value, key: &str, value: &str) {
    if !body.get("client_metadata").is_some_and(Value::is_object) {
        body["client_metadata"] = serde_json::json!({});
    }
    let metadata = body["client_metadata"].as_object_mut().unwrap();
    metadata
        .entry(key)
        .or_insert_with(|| Value::String(value.to_owned()));
}

fn history_too_large() -> ServiceError {
    ServiceError::new(
        413,
        "continuation_history_too_large",
        "Partial continuation exceeds the configured request size limit; existing history was preserved",
    )
}

#[cfg(test)]
#[path = "identity_partial_history_tests.rs"]
mod tests;
