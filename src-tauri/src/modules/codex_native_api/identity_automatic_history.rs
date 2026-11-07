//! Automatic admission of client-supplied history, without claiming unknown
//! server resources or publishing input provenance before a successful response.
use super::*;

/// Images can accept full Responses history without producing a durable replay
/// checkpoint. They still must validate the terminal before claiming input.
#[derive(Debug, Default)]
pub(super) struct Admission {
    pub cancelled: Arc<std::sync::atomic::AtomicBool>,
    output: Mutex<std::collections::BTreeMap<u64, String>>,
}

pub(super) fn observe_output(binding: &Binding, event: &Value) -> Result<(), ServiceError> {
    let Some(pending) = &binding.pending_input else {
        return Ok(());
    };
    if event["type"] != "response.output_item.done" {
        return Ok(());
    }
    let mut output = pending.output.lock().map_err(storage)?;
    let index = event
        .get("output_index")
        .and_then(Value::as_u64)
        .filter(|index| *index < 4096)
        .unwrap_or(u64::MAX);
    let digest = event
        .get("item")
        .map(|item| format!("{:x}", Sha256::digest(item.to_string().as_bytes())))
        .unwrap_or_default();
    if output
        .get(&index)
        .is_some_and(|previous| previous != &digest)
    {
        pending
            .cancelled
            .store(true, std::sync::atomic::Ordering::Release);
    } else {
        output.insert(index, digest);
    }
    Ok(())
}

impl IdentityStore {
    pub(super) fn commit_automatic_input(
        &self,
        binding: &Binding,
        event: &Value,
    ) -> Result<(), ServiceError> {
        let Some(pending) = &binding.pending_input else {
            return Ok(());
        };
        if pending.cancelled.load(std::sync::atomic::Ordering::Acquire) {
            return Ok(());
        }
        let response = event.get("response").unwrap_or(event);
        let Some(output) = response.get("output").and_then(Value::as_array) else {
            return Ok(());
        };
        if !rebuild::replay_items(output) {
            return Ok(());
        }
        if !pending
            .output
            .lock()
            .map_err(storage)?
            .iter()
            .all(|(index, hash)| {
                output.get(*index as usize).is_some_and(|item| {
                    format!("{:x}", Sha256::digest(item.to_string().as_bytes())) == *hash
                })
            })
        {
            return Ok(());
        }
        let mut connection = self.connection.lock().map_err(storage)?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage)?;
        if pending.cancelled.load(std::sync::atomic::Ordering::Acquire) {
            return Ok(());
        }
        accept_input(&transaction, binding)?;
        // Cancellation and a completed admission compete for this single gate.
        // If cancellation won while SQLite was busy, dropping rolls back input.
        if pending
            .cancelled
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .is_err()
        {
            return Ok(());
        }
        transaction.commit().map_err(storage)
    }
}

pub(super) fn complete_input(headers: &HeaderMap, body: &Value) -> bool {
    !headers.contains_key("conversation_id")
        && ["previous_response_id", "conversation", "conversation_id"]
            .iter()
            .all(|key| body.get(key).is_none_or(Value::is_null))
        && rebuild::replay_tools(body.get("tools"))
        && body
            .get("input")
            .and_then(Value::as_array)
            .is_some_and(|items| {
                !items.is_empty() && rebuild::replay_items(items) && rebuild::paired_tools(items)
            })
}

pub(super) fn incomplete() -> ServiceError {
    ServiceError::new(
        400,
        "continuation_history_incomplete",
        "Continuation requires complete conversation input, including matching tool calls and results; the original history was preserved",
    )
}

/// Absence in this caller's namespace is not proof that a reference is unknown.
/// Check other namespaces before allowing an unregistered payload to be replayed.
pub(super) fn lookup(
    connection: &Connection,
    caller: &str,
    reference: &OpaqueRef,
    thread: Option<&str>,
    turn: Option<&str>,
) -> Result<Option<(String, String)>, ServiceError> {
    let record: Option<(String, String, Option<String>, Option<String>, i64)> = connection
        .query_row(
            "SELECT account,owner,thread,turn,expires FROM native_opaque_v1 WHERE caller=?1 AND kind=?2 AND digest=?3",
            params![caller, reference.kind, reference.digest],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .optional()
        .map_err(storage)?;
    if let Some((account, owner, recorded_thread, recorded_turn, expires)) = record {
        if reference.kind == "turn_state" {
            if recorded_thread
                .as_deref()
                .is_some_and(|recorded| thread != Some(recorded))
                || recorded_turn
                    .as_deref()
                    .is_some_and(|recorded| turn != Some(recorded))
            {
                return Err(ServiceError::new(
                    409,
                    "upstream_turn_state_mismatch",
                    "Routing state cannot be reused by a different thread or turn",
                ));
            }
            if expires < timestamp() {
                return Ok(None);
            }
        }
        return Ok(Some((account, owner)));
    }
    let foreign: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM native_opaque_v1 WHERE kind=?1 AND digest=?2 AND caller!=?3)",
        params![reference.kind, reference.digest, caller], |row| row.get(0),
    ).map_err(storage)?;
    if foreign {
        return Err(ServiceError::new(
            409,
            "upstream_state_caller_mismatch",
            "Upstream state belongs to a different API caller",
        ));
    }
    Ok(None)
}

pub(super) fn is_unregistered(context: &RequestIdentity, reference: &OpaqueRef) -> bool {
    reference.kind == "encrypted" && context.unregistered_blobs.contains(&reference.digest)
}

pub(super) fn detached(context: &RequestIdentity, reference: &OpaqueRef) -> bool {
    (reference.kind == "parent_response" && context.detached_parent)
        || (reference.kind == "turn_state"
            && context
                .detached_turn_states
                .iter()
                .any(|token| token.digest == reference.digest))
}

pub(super) fn accept_input(
    transaction: &Transaction<'_>,
    binding: &Binding,
) -> Result<(), ServiceError> {
    for reference in &binding.input_blobs {
        if lookup(
            transaction,
            &binding.caller,
            reference,
            binding.thread_id.as_deref(),
            binding.turn_id.as_deref(),
        )?
        .is_some()
        {
            continue;
        }
        transaction.execute(
            "INSERT OR IGNORE INTO native_opaque_v1(caller,kind,digest,account,owner,session,thread,turn,expires) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,0)",
            params![binding.caller, reference.kind, reference.digest, binding.account_id, binding.owner_id, binding.session_id, binding.thread_id, binding.turn_id],
        ).map_err(storage)?;
    }
    Ok(())
}
