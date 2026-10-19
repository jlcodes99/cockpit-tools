//! Complete wire-history checkpoints with a bounded hot cache. Blob provenance stays immutable.
use super::*;
use std::collections::HashSet;

#[path = "identity_checkpoint.rs"]
mod checkpoint_store;

const MAX_CHECKPOINT_BYTES: usize = 8 * 1024 * 1024;
const MAX_CACHE_BYTES: i64 = 64 * 1024 * 1024;
const MAX_CHECKPOINTS: i64 = 128;

#[derive(Clone, Debug)]
pub(super) struct Generation {
    pub id: String,
    pub base_revision: i64,
    pub activate: bool,
    pub input: Option<Vec<Value>>,
    pub parameters: Option<Value>,
    pub output: Arc<std::sync::Mutex<std::collections::BTreeMap<u64, String>>>,
    pub cancelled: Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Clone, Debug)]
pub(crate) struct Plan {
    pub headers: HeaderMap,
    pub body: Value,
    pub base_revision: i64,
}

pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS native_rebuild_active_v1 (
           caller TEXT NOT NULL, thread TEXT NOT NULL, account TEXT NOT NULL,
           owner TEXT NOT NULL, generation TEXT NOT NULL, revision INTEGER NOT NULL,
           PRIMARY KEY(caller,thread));
         CREATE TABLE IF NOT EXISTS native_rebuild_refs_v1 (
           caller TEXT NOT NULL, kind TEXT NOT NULL, digest TEXT NOT NULL,
           generation TEXT NOT NULL, PRIMARY KEY(caller,kind,digest));
         CREATE TABLE IF NOT EXISTS native_rebuild_checkpoints_v1 (
           caller TEXT NOT NULL, response TEXT NOT NULL, thread TEXT NOT NULL,
           account TEXT NOT NULL, generation TEXT NOT NULL, transcript TEXT NOT NULL,
           bytes INTEGER NOT NULL, created INTEGER NOT NULL, parameters TEXT,
           PRIMARY KEY(caller,response));
         CREATE TABLE IF NOT EXISTS native_rebuild_blobs_v1 (
           caller TEXT NOT NULL, thread TEXT NOT NULL, generation TEXT NOT NULL,
           digest TEXT NOT NULL, account TEXT NOT NULL, owner TEXT NOT NULL,
           PRIMARY KEY(caller,thread,generation,digest));
         CREATE TABLE IF NOT EXISTS native_rebuild_portable_v1 (
           caller TEXT NOT NULL, thread TEXT NOT NULL, generation TEXT NOT NULL,
           account TEXT NOT NULL, owner TEXT NOT NULL,
           PRIMARY KEY(caller,thread,generation));",
    )?;
    let has_parameters = connection
        .prepare("PRAGMA table_info(native_rebuild_checkpoints_v1)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .any(|name| name == "parameters");
    if !has_parameters {
        connection.execute(
            "ALTER TABLE native_rebuild_checkpoints_v1 ADD COLUMN parameters TEXT",
            [],
        )?;
    }
    checkpoint_store::initialize(connection)
}

pub(super) fn reference_generation(
    connection: &Connection,
    caller: &str,
    refs: &[OpaqueRef],
) -> Result<Option<String>, ServiceError> {
    let mut generation = None;
    for reference in refs {
        // Sticky routing tokens can be reissued; they are not conversation IDs.
        if reference.kind == "turn_state" {
            continue;
        }
        let current = connection.query_row(
            "SELECT generation FROM native_rebuild_refs_v1 WHERE caller=?1 AND kind=?2 AND digest=?3",
            params![caller,reference.kind,reference.digest], |row| row.get::<_,String>(0),
        ).optional().map_err(storage)?.unwrap_or_default();
        if generation
            .as_ref()
            .is_some_and(|expected| expected != &current)
        {
            return Err(ServiceError::new(
                409,
                "mixed_upstream_generation",
                "Request contains state from different upstream conversations",
            ));
        }
        generation = Some(current);
    }
    Ok(generation)
}

fn active(
    connection: &Connection,
    caller: &str,
    thread: &str,
) -> Result<Option<(String, String, i64)>, ServiceError> {
    connection.query_row(
        "SELECT owner,generation,revision FROM native_rebuild_active_v1 WHERE caller=?1 AND thread=?2",
        params![caller,thread], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).optional().map_err(storage)
}

/// A completed generation may retain accepted input when rebuilding is disabled.
pub(super) fn accepted_scope(
    connection: &Connection,
    caller: &str,
    thread: Option<&str>,
    account: Option<&str>,
    generation: Option<&str>,
) -> Result<Option<(String, String, String)>, ServiceError> {
    let Some(thread) = thread else {
        return Ok(None);
    };
    if let (Some(account), Some(generation)) = (account, generation) {
        return Ok(connection.query_row("SELECT account,owner,generation FROM native_rebuild_portable_v1 WHERE caller=?1 AND thread=?2 AND account=?3 AND generation=?4",params![caller,thread,account,generation],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(storage)?);
    }
    connection.query_row("SELECT a.account,a.owner,a.generation FROM native_rebuild_active_v1 a WHERE a.caller=?1 AND a.thread=?2 AND EXISTS(SELECT 1 FROM native_rebuild_portable_v1 b WHERE b.caller=a.caller AND b.thread=a.thread AND b.account=a.account AND b.owner=a.owner AND b.generation=a.generation) AND (?3 IS NULL OR a.account=?3)",params![caller,thread,account],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(storage)
}
pub(super) fn full_input(
    connection: &Connection,
    caller: &str,
    thread: Option<&str>,
    body: &Value,
) -> Result<bool, ServiceError> {
    let Some(input) = body.get("input").and_then(Value::as_array) else {
        return Ok(false);
    };
    if !replay_items(input)
        || body.get("conversation").is_some_and(|v| !v.is_null())
        || body.get("conversation_id").is_some_and(|v| !v.is_null())
    {
        return Ok(false);
    }
    let Some(previous) = body.get("previous_response_id").and_then(Value::as_str) else {
        return Ok(true);
    };
    let Some(previous) = checkpoint_store::transcript(connection, caller, thread, previous)? else {
        return Ok(false);
    };
    let strip = |items: &[Value]| {
        items
            .iter()
            .map(|item| {
                let mut item = item.clone();
                item.as_object_mut().map(|o| o.remove("id"));
                item
            })
            .collect::<Vec<_>>()
    };
    Ok(strip(input).starts_with(&strip(&previous)))
}

pub(super) fn blob_accepted(
    connection: &Connection,
    caller: &str,
    thread: Option<&str>,
    reference: &OpaqueRef,
    scope: &(String, String, String),
) -> Result<bool, ServiceError> {
    Ok(connection.query_row("SELECT 1 FROM native_rebuild_blobs_v1 WHERE caller=?1 AND thread=?2 AND generation=?3 AND digest=?4 AND account=?5 AND owner=?6",params![caller,thread,scope.2,reference.digest,scope.0,scope.1],|_|Ok(())).optional().map_err(storage)?.is_some())
}

pub(super) fn select_generation(
    transaction: &Transaction<'_>,
    context: &RequestIdentity,
    owner: &str,
) -> Result<Option<Generation>, ServiceError> {
    let Some(thread) = context.thread_id.as_deref() else {
        return Ok(None);
    };
    let existing = active(transaction, &context.caller, thread)?;
    if let Some(generation) = &context.generation {
        return Ok(Some(Generation {
            id: generation.clone(),
            base_revision: 0,
            activate: false,
            input: context.rebuild_input.clone(),
            parameters: context.rebuild_parameters.clone(),
            output: Default::default(),
            cancelled: Default::default(),
        }));
    }
    if let Some((old_owner, generation, revision)) = existing
        .as_ref()
        .filter(|(old_owner, _, _)| old_owner == owner && !context.rebuild_new_generation)
    {
        let _ = old_owner;
        return Ok(Some(Generation {
            id: generation.clone(),
            base_revision: *revision,
            activate: false,
            input: context.rebuild_input.clone(),
            parameters: context.rebuild_parameters.clone(),
            output: Default::default(),
            cancelled: Default::default(),
        }));
    }
    if !context.rebuild_enabled {
        return Ok(None);
    }
    let mut changed_origin = false;
    if context.portable_body {
        for reference in context.opaque.iter().filter(|r| r.kind == "encrypted") {
            let old: Option<String> = transaction
                .query_row(
                    "SELECT owner FROM native_opaque_v1 WHERE caller=?1 AND kind=?2 AND digest=?3",
                    params![context.caller, reference.kind, reference.digest],
                    |row| row.get(0),
                )
                .optional()
                .map_err(storage)?;
            changed_origin |= old.is_some_and(|old| old != owner);
        }
    }
    let old_affinity: Option<String> = transaction.query_row("SELECT owner FROM native_affinity_v1 WHERE caller=?1 AND kind='thread' AND original=?2",params![context.caller,thread],|row|row.get(0)).optional().map_err(storage)?;
    let changed_origin = changed_origin || old_affinity.is_some_and(|old| old != owner);
    // A same-account child may inherit its parent's completed mapping namespace.
    if let Some(parent) = context
        .parent_thread_id
        .as_deref()
        .filter(|_| !context.rebuild_new_generation && !changed_origin)
    {
        if let Some((parent_owner, generation, _)) = active(transaction, &context.caller, parent)? {
            if parent_owner == owner {
                return Ok(Some(Generation {
                    id: generation,
                    base_revision: 0,
                    activate: true,
                    input: context.rebuild_input.clone(),
                    parameters: context.rebuild_parameters.clone(),
                    output: Default::default(),
                    cancelled: Default::default(),
                }));
            }
        }
    }
    Ok(Some(Generation {
        id: if existing.is_none() && !context.rebuild_new_generation && !changed_origin {
            String::new()
        } else {
            uuid::Uuid::now_v7().to_string()
        },
        base_revision: context
            .rebuild_base_revision
            .unwrap_or_else(|| existing.map_or(0, |(_, _, revision)| revision)),
        activate: true,
        input: context.rebuild_input.clone(),
        parameters: context.rebuild_parameters.clone(),
        output: Default::default(),
        cancelled: Default::default(),
    }))
}

pub(super) fn mapping_owner(owner: &str, generation: Option<&Generation>) -> String {
    match generation.filter(|g| !g.id.is_empty()) {
        Some(g) => format!("{owner}:native-generation:{}", g.id),
        None => owner.to_owned(),
    }
}

pub(super) fn observe_output(binding: &Binding, event: &Value) -> Result<(), ServiceError> {
    let Some(generation) = binding
        .generation
        .as_ref()
        .filter(|generation| generation.input.is_some())
    else {
        return Ok(());
    };
    if event["type"] != "response.output_item.done" {
        return Ok(());
    }
    let mut output = generation.output.lock().map_err(storage)?;
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
        // Every delivered item must agree with the terminal transcript; do not
        // let a later duplicate overwrite evidence already seen by the client.
        binding.abort_rebuild();
    } else {
        output.insert(index, digest);
    }
    Ok(())
}

pub(super) fn may_publish_affinity(
    connection: &Connection,
    context: &RequestIdentity,
    owner: &str,
    generation: Option<&Generation>,
) -> Result<bool, ServiceError> {
    let Some(generation) = generation else {
        return Ok(true);
    };
    if generation.activate {
        return Ok(false);
    }
    let Some(thread) = context.thread_id.as_deref() else {
        return Ok(true);
    };
    Ok(active(connection, &context.caller, thread)?.is_none_or(
        |(active_owner, active_generation, _)| {
            active_owner == owner && active_generation == generation.id
        },
    ))
}

/// Only explicit plaintext representations are accepted. Summaries never decrypt reasoning.
fn plain_items(items: &[Value]) -> bool {
    items.iter().all(|item| {
        if !item.is_object() || has_opaque_or_attachment(item) {
            return false;
        }
        match item.get("type").and_then(Value::as_str) {
            Some("message") | None => {
                matches!(
                    item.get("role").and_then(Value::as_str),
                    Some("user" | "assistant" | "developer" | "system")
                ) && item.get("content").is_some_and(|content| {
                    content.is_string()
                        || content.as_array().is_some_and(|parts| {
                            parts.iter().all(|part| {
                                matches!(
                                    part.get("type").and_then(Value::as_str),
                                    Some("input_text" | "output_text" | "text" | "refusal")
                                ) && part
                                    .get(if part["type"] == "refusal" {
                                        "refusal"
                                    } else {
                                        "text"
                                    })
                                    .is_some_and(Value::is_string)
                            })
                        })
                })
            }
            Some("function_call") => {
                let communication = item
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| {
                        matches!(
                            name,
                            "spawn_agent"
                                | "send_message"
                                | "followup_task"
                                | "collaboration.spawn_agent"
                                | "collaboration.send_message"
                                | "collaboration.followup_task"
                        )
                    });
                let explicit_plaintext = item
                    .get("encrypted_function_args")
                    .and_then(Value::as_array)
                    .is_some_and(Vec::is_empty);
                item.get("arguments")
                    .and_then(Value::as_str)
                    .is_some_and(|arguments| serde_json::from_str::<Value>(arguments).is_ok())
                    && (!communication || explicit_plaintext)
                    && super::agent_communication::encrypted_message(item).is_none()
            }
            Some("function_call_output") => item.get("output").is_some_and(Value::is_string),
            Some("additional_tools") => {
                item["role"] == "developer"
                    && item
                        .get("tools")
                        .and_then(Value::as_array)
                        .is_some_and(|tools| tools.iter().all(Value::is_object))
            }
            _ => false,
        }
    })
}

fn has_opaque_or_attachment(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            (matches!(
                key.as_str(),
                "encrypted_content" | "file_id" | "asset_pointer" | "signature"
            ) && !value.is_null())
                || (key == "encrypted_function_args"
                    && !value.is_null()
                    && value.as_array().is_none_or(|args| !args.is_empty()))
                || has_opaque_or_attachment(value)
        }),
        Value::Array(array) => array.iter().any(has_opaque_or_attachment),
        _ => false,
    }
}

// A parameter schema describes future arguments; its property names are not
// resource handles. Only executable tool/configuration values are checked.
fn resource_handles(value: &Value, inherited_definition: bool) -> bool {
    match value {
        Value::Object(object) => {
            let definition = inherited_definition
                || matches!(
                    object.get("type").and_then(Value::as_str),
                    Some("function" | "custom")
                )
                || (object.get("type").is_none()
                    && object.get("name").is_some_and(Value::is_string)
                    && (object.contains_key("parameters") || object.contains_key("input_schema")));
            object.iter().any(|(key, value)| {
                if definition && matches!(key.as_str(), "parameters" | "input_schema" | "schema") {
                    return false;
                }
                (matches!(
                    key.as_str(),
                    "file_id" | "file_ids" | "vector_store_ids" | "container_id" | "asset_pointer"
                ) && !value.is_null())
                    || (key == "container" && value.is_string() && value != "auto")
                    || resource_handles(value, definition)
            })
        }
        Value::Array(array) => array
            .iter()
            .any(|value| resource_handles(value, inherited_definition)),
        _ => false,
    }
}
pub(super) fn replay_tools(tools: Option<&Value>) -> bool {
    tools.is_none_or(|tools| !resource_handles(tools, false))
}

/// Validate a self-contained wire representation without decrypting or dropping
/// content. Server-only item references and nonportable attachment handles decline.
pub(super) fn replay_items(items: &[Value]) -> bool {
    fn parts(value: &Value, agent: bool) -> bool {
        value.as_array().is_some_and(|parts| {
            parts.iter().all(|part| match part["type"].as_str() {
                Some("input_text" | "output_text" | "text") => part["text"].is_string(),
                Some("refusal") if !agent => part["refusal"].is_string(),
                Some("encrypted_content") => part["encrypted_content"]
                    .as_str()
                    .is_some_and(|v| !v.is_empty()),
                Some("input_image") if !agent => part["image_url"].is_string(),
                Some("input_audio") if !agent => part["audio_url"].is_string(),
                _ => false,
            })
        })
    }
    items.iter().all(|item| {
        if !item.is_object() || resource_handles(item, false) {
            return false;
        }
        if item
            .get("encrypted_content")
            .is_some_and(|v| !v.is_null() && !v.is_string())
        {
            return false;
        }
        if item.get("encrypted_function_args").is_some_and(|v| {
            !v.is_null() && v.as_array().is_none_or(|v| !v.iter().all(Value::is_string))
        }) {
            return false;
        }
        match item["type"].as_str() {
            Some("message") | None => {
                matches!(
                    item["role"].as_str(),
                    Some("user" | "assistant" | "developer" | "system")
                ) && (item["content"].is_string() || parts(&item["content"], false))
            }
            Some("agent_message") => {
                item["author"].is_string()
                    && item["recipient"].is_string()
                    && parts(&item["content"], true)
            }
            Some("reasoning") => {
                item["encrypted_content"]
                    .as_str()
                    .is_some_and(|v| !v.is_empty())
                    || item["summary"].is_array()
                    || item["content"].is_array()
            }
            Some("compaction" | "compaction_summary" | "context_compaction") => item
                ["encrypted_content"]
                .as_str()
                .is_some_and(|v| !v.is_empty()),
            Some("function_call") => {
                item["name"].is_string()
                    && item["arguments"]
                        .as_str()
                        .is_some_and(|a| serde_json::from_str::<Value>(a).is_ok())
            }
            Some("custom_tool_call") => item["name"].is_string() && item["input"].is_string(),
            Some("function_call_output" | "custom_tool_call_output") => {
                item["output"].is_string() || parts(&item["output"], false)
            }
            Some("additional_tools" | "tool_search_output") => item["tools"]
                .as_array()
                .is_some_and(|v| v.iter().all(Value::is_object)),
            Some("local_shell_call") => item["action"].is_object(),
            Some("tool_search_call") => item.get("arguments").is_some(),
            Some("web_search_call") => item["action"].is_object(),
            Some("image_generation_call") => item["result"].is_string(),
            Some("configuration_update") => item["reasoning"].is_object(),
            Some("compaction_trigger") => true,
            _ => false,
        }
    })
}

pub(super) fn paired_tools(items: &[Value]) -> bool {
    let host_outputs = super::super::codex_app_outputs::Classifier::new(items);
    let mut pending = HashSet::new();
    let mut seen = HashSet::new();
    for item in items {
        match item.get("type").and_then(Value::as_str) {
            Some("function_call" | "custom_tool_call") => {
                let Some(id) = item.get("call_id").and_then(Value::as_str) else {
                    return false;
                };
                if !seen.insert(id) {
                    return false;
                }
                pending.insert(id);
            }
            Some("function_call_output" | "custom_tool_call_output") => {
                if host_outputs.classify(item)
                    == super::super::codex_app_outputs::Classification::Standalone
                {
                    // This host notification is not the result of a model call.
                    // Keep it in the transcript without consuming a pending call.
                    continue;
                }
                let Some(id) = item.get("call_id").and_then(Value::as_str) else {
                    return false;
                };
                if !pending.remove(id) {
                    return false;
                }
            }
            _ => {}
        }
    }
    pending.is_empty()
}

fn merge_checkpoint(previous: Vec<Value>, incoming: &[Value]) -> Option<Vec<Value>> {
    let without_ids = |items: &[Value]| {
        items
            .iter()
            .map(|item| {
                let mut item = item.clone();
                item.as_object_mut()?.remove("id");
                Some(item)
            })
            .collect::<Option<Vec<_>>>()
    };
    if without_ids(incoming)?.starts_with(&without_ids(&previous)?) {
        return Some(incoming.to_vec());
    }
    // Incremental input may only finish pending tools or append a fresh user turn.
    // An assistant/tool-call prefix could be truncated full history, not a delta.
    if incoming.iter().any(|item| {
        item.get("role").and_then(Value::as_str) == Some("assistant")
            || matches!(
                item.get("type").and_then(Value::as_str),
                Some("function_call")
            )
    }) {
        return None;
    }
    let mut complete = previous;
    complete.extend_from_slice(incoming);
    Some(complete)
}

pub(super) fn complete_input_available(
    connection: &Connection,
    caller: &str,
    thread: Option<&str>,
    headers: &HeaderMap,
    body: &Value,
) -> Result<bool, ServiceError> {
    if automatic_history::complete_input(headers, body) {
        return Ok(true);
    }
    if headers.contains_key("conversation_id")
        || ["conversation", "conversation_id"]
            .iter()
            .any(|key| body.get(key).is_some_and(|v| !v.is_null()))
        || !replay_tools(body.get("tools"))
    {
        return Ok(false);
    }
    let (Some(previous), Some(input)) = (
        body.get("previous_response_id").and_then(Value::as_str),
        body.get("input").and_then(Value::as_array),
    ) else {
        return Ok(false);
    };
    let Some(transcript) = checkpoint_store::transcript(connection, caller, thread, previous)?
        .and_then(|previous| merge_checkpoint(previous, input))
    else {
        return Ok(false);
    };
    Ok(replay_items(&transcript) && paired_tools(&transcript))
}

impl IdentityStore {
    /// Normalize only an OAuth HTTP request (or a planning-only copy), before
    /// checkpoint capture. Keep the logical identities generated by inspection
    /// when a generic SDK supplied no explicit session/thread metadata.
    pub(crate) fn normalize_http_input(
        &self,
        context: &mut RequestIdentity,
        headers: &HeaderMap,
        body: &mut Value,
    ) -> Result<bool, ServiceError> {
        let Some(Value::String(text)) = body.get("input") else {
            return Ok(false);
        };
        body["input"] = serde_json::json!([{"type":"message","role":"user","content":[{"type":"input_text","text":text}]}]);
        let mut probe_headers = headers.clone();
        let mut probe = body.clone();
        supplement_identities(context, &mut probe_headers, &mut probe)?;
        *context = self.inspect_for_rebuild(&context.caller, &probe_headers, &probe)?;
        Ok(true)
    }

    /// ChatGPT's HTTP endpoint cannot resolve previous_response_id. Reconstruct
    /// the complete request from this caller's authenticated checkpoint before
    /// sending, even when the selected account has not changed. The checkpoint
    /// parameter manifest supplies only omitted fields; explicit null/empty
    /// values remain authoritative. This does not relax cross-account planning.
    pub(crate) fn expand_http_previous(
        &self,
        context: &mut RequestIdentity,
        headers: &HeaderMap,
        body: &Value,
        parameter_source: &Value,
    ) -> Result<Plan, ServiceError> {
        let incomplete = || {
            ServiceError::new(
            409,
            "continuation_history_incomplete",
            "Previous response history is unavailable for HTTP continuation; resend the complete conversation input; existing history was preserved",
        )
        };
        let previous = body
            .get("previous_response_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(incomplete)?;
        let checkpoint = {
            let connection = self.checkpoint_reader()?;
            checkpoint_store::metadata(&connection, &context.caller, previous)?
        };
        let checkpoint_store::Metadata {
            thread,
            account,
            parameters,
        } = checkpoint.ok_or_else(incomplete)?;
        if context.thread_id.as_deref() != Some(thread.as_str())
            || context.state_account_id.as_deref() != Some(account.as_str())
        {
            return Err(incomplete());
        }
        let parameters = parameters.as_deref().ok_or_else(incomplete)?;
        let parameters = serde_json::from_str::<serde_json::Map<String, Value>>(parameters)
            .map_err(|_| incomplete())?;
        let mut candidate = body.clone();
        self.normalize_http_input(context, headers, &mut candidate)?;
        let object = candidate.as_object_mut().ok_or_else(incomplete)?;
        for key in [
            "instructions",
            "tools",
            "tool_choice",
            "parallel_tool_calls",
            "text",
            "reasoning",
        ] {
            if parameter_source.get(key).is_none() {
                if let Some(value) = parameters.get(key) {
                    object.insert(key.into(), value.clone());
                }
            }
        }
        // Revalidate the real request, including parameter inheritance and all
        // cached tool/cipher evidence, instead of trusting a planning preview.
        let mut expanded = self.inspect_for_rebuild(&context.caller, headers, &candidate)?;
        let plan = self
            .configure_rebuild(&mut expanded, headers, &candidate, true)?
            .ok_or_else(incomplete)?;
        *context = expanded;
        Ok(plan)
    }

    /// Planning-only capability probe for transports which add their defaults
    /// after account selection. Never returns a request eligible for sending:
    /// Bind must validate the transport's actual normalized body separately.
    pub(crate) fn preview_rebuild_with_checkpoint_parameters(
        &self,
        context: &RequestIdentity,
        headers: &HeaderMap,
        body: &Value,
    ) -> Result<Option<Plan>, ServiceError> {
        let Some(previous) = body.get("previous_response_id").and_then(Value::as_str) else {
            return Ok(None);
        };
        let raw = {
            let connection = self.checkpoint_reader()?;
            checkpoint_store::metadata(&connection, &context.caller, previous)?
                .and_then(|checkpoint| checkpoint.parameters)
        };
        let Some(raw) = raw else {
            return Ok(None);
        };
        let Ok(parameters) = serde_json::from_str::<serde_json::Map<String, Value>>(&raw) else {
            return Ok(None);
        };
        let mut probe = body.clone();
        let mut probe_context = context.clone();
        let normalized = self.normalize_http_input(&mut probe_context, headers, &mut probe)?;
        let Some(object) = probe.as_object_mut() else {
            return Ok(None);
        };
        let mut missing = false;
        for (key, value) in parameters {
            if !object.contains_key(&key) {
                object.insert(key, value);
                missing = true;
            }
        }
        if !missing && !normalized {
            return Ok(None);
        }
        self.configure_rebuild(&mut probe_context, headers, &probe, true)
    }

    pub(crate) fn protect_unrebuildable(
        &self,
        context: &mut RequestIdentity,
    ) -> Result<(), ServiceError> {
        let connection = self.connection.lock().map_err(storage)?;
        if context.state_account_id.is_none() {
            for thread in [
                context.thread_id.as_deref(),
                context.parent_thread_id.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                let bound:Option<(String,String)>=connection.query_row("SELECT account,generation FROM native_rebuild_active_v1 WHERE caller=?1 AND thread=?2",params![context.caller,thread],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(storage)?;
                if let Some((account, generation)) = bound {
                    context.state_account_id = Some(account.clone());
                    context.preferred_account_id = Some(account);
                    context.generation = Some(generation);
                    break;
                }
            }
        }
        let accepted = accepted_scope(
            &connection,
            &context.caller,
            context.thread_id.as_deref(),
            context.state_account_id.as_deref(),
            context.generation.as_deref(),
        )?;
        let mut origin: Option<String> = context.state_account_id.clone();
        for reference in &context.opaque {
            if automatic_history::is_unregistered(context, reference) {
                return Err(automatic_history::incomplete());
            }
            if context.detached_parent && reference.kind == "parent_response" {
                continue;
            }
            if reference.kind == "turn_state"
                && context
                    .detached_turn_states
                    .iter()
                    .any(|r| r.digest == reference.digest)
            {
                continue;
            }
            let (account, _) = validate_opaque(
                &connection,
                &context.caller,
                reference,
                context.thread_id.as_deref(),
                context.turn_id.as_deref(),
            )?;
            if reference.kind == "encrypted" {
                if let Some(scope) = &accepted {
                    if blob_accepted(
                        &connection,
                        &context.caller,
                        context.thread_id.as_deref(),
                        reference,
                        scope,
                    )? {
                        continue;
                    }
                }
            }
            if origin.as_ref().is_some_and(|origin| origin != &account) {
                return Err(ServiceError::new(
                    409,
                    "mixed_upstream_state",
                    "An incomplete history cannot be replayed across upstream owners",
                ));
            }
            origin = Some(account);
        }
        if context.state_account_id.is_none() {
            if let Some(account) = origin {
                context.state_account_id = Some(account.clone());
                context.preferred_account_id = Some(account);
                context.generation =
                    reference_generation(&connection, &context.caller, &context.opaque)?;
            } else {
                let affinity:Option<String>=connection.query_row("SELECT account FROM native_affinity_v1 WHERE caller=?1 AND kind='thread' AND original=?2",params![context.caller,context.thread_id],|row|row.get(0)).optional().map_err(storage)?;
                if let Some(account) = affinity {
                    context.state_account_id = Some(account.clone());
                    context.preferred_account_id = Some(account);
                }
            }
        }
        if accepted.is_none() {
            context.portable_body = false;
        }
        Ok(())
    }
    pub(crate) fn configure_rebuild(
        &self,
        context: &mut RequestIdentity,
        headers: &HeaderMap,
        body: &Value,
        enabled: bool,
    ) -> Result<Option<Plan>, ServiceError> {
        context.rebuild_enabled = false;
        if !enabled || (!context.portable_body && context.parent_thread_id.is_some()) {
            return Ok(None);
        }
        if body.get("conversation").is_some_and(|v| !v.is_null())
            || body.get("conversation_id").is_some_and(|v| !v.is_null())
            || headers.contains_key("conversation_id")
            || !replay_tools(body.get("tools"))
            || (!context.portable_body
                && body
                    .get("tools")
                    .and_then(Value::as_array)
                    .is_some_and(|tools| tools.iter().any(|tool| tool["type"] != "function")))
        {
            return Ok(None);
        }
        let Some(input) = body.get("input").and_then(Value::as_array) else {
            return Ok(None);
        };
        if !(if context.portable_body {
            replay_items(input)
        } else {
            plain_items(input)
        }) {
            return Ok(None);
        }
        let transcript =
            if let Some(previous) = body.get("previous_response_id").and_then(Value::as_str) {
                // A separate read connection keeps large history decoding off the
                // shared identity mutex; WAL readers do not block identity writes.
                let connection = self.checkpoint_reader()?;
                let Some(checkpoint_store::Metadata {
                    thread,
                    account,
                    parameters: Some(parameters),
                }) = checkpoint_store::metadata(&connection, &context.caller, previous)?
                else {
                    return Ok(None);
                };
                if context.thread_id.as_deref() != Some(thread.as_str())
                    || context.state_account_id.as_deref() != Some(account.as_str())
                {
                    return Ok(None);
                }
                let Some(previous) = checkpoint_store::transcript(
                    &connection,
                    &context.caller,
                    Some(&thread),
                    previous,
                )?
                else {
                    return Ok(None);
                };
                let Ok(parameters) =
                    serde_json::from_str::<serde_json::Map<String, Value>>(&parameters)
                else {
                    return Ok(None);
                };
                // Never guess whether omitted instructions/tools should inherit across accounts.
                if parameters.keys().any(|key| body.get(key).is_none()) {
                    return Ok(None);
                }
                let Some(transcript) = merge_checkpoint(previous, input) else {
                    return Ok(None);
                };
                transcript
            } else {
                if !context.portable_body && !context.opaque.is_empty() {
                    return Ok(None);
                }
                input.clone()
            };
        // Cache admission is checked when publishing a checkpoint, not when
        // validating a complete request that can already be sent upstream.
        context.rebuild_enabled = true;
        context.rebuild_input = Some(transcript.clone());
        context.rebuild_parameters = Some(Value::Object(
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
                    .filter(|value| !value.is_null())
                    .map(|value| (key.to_owned(), value.clone()))
            })
            .collect(),
        ));
        let connection = self.checkpoint_reader()?;
        let base_revision = active(
            &connection,
            &context.caller,
            context.thread_id.as_deref().unwrap_or_default(),
        )?
        .map_or(0, |(_, generation, revision)| {
            if context
                .generation
                .as_ref()
                .is_some_and(|source| source != &generation)
            {
                -1
            } else {
                revision
            }
        });
        drop(connection);
        context.rebuild_base_revision = Some(base_revision);
        if !paired_tools(&transcript) {
            return Ok(None);
        }
        let mut upstream_headers = headers.clone();
        let mut upstream_body = body.clone();
        let mut fresh_input = transcript;
        if !context.portable_body {
            for item in &mut fresh_input {
                item.as_object_mut().unwrap().remove("id");
            }
        }
        upstream_body["input"] = Value::Array(fresh_input);
        for key in ["previous_response_id", "conversation", "conversation_id"] {
            upstream_body.as_object_mut().unwrap().remove(key);
        }
        for key in ["x-codex-turn-state", "conversation_id"] {
            upstream_headers.remove(key);
        }
        if let Some(metadata) = upstream_body
            .get_mut("client_metadata")
            .and_then(Value::as_object_mut)
        {
            metadata.remove("x-codex-turn-state");
            metadata.remove("parent_response_id");
            if let Some(raw) = metadata
                .get("x-codex-turn-metadata")
                .and_then(Value::as_str)
            {
                let mut canonical = parse_metadata(raw)?;
                for key in ["parent_response_id", "x-codex-turn-state"] {
                    canonical.as_object_mut().unwrap().remove(key);
                }
                metadata.insert(
                    "x-codex-turn-metadata".into(),
                    Value::String(ascii_json(&canonical)?),
                );
            }
        }
        if let Some(raw) = upstream_headers.get("x-codex-turn-metadata") {
            let mut canonical = parse_metadata(raw.to_str().map_err(storage)?)?;
            for key in ["parent_response_id", "x-codex-turn-state"] {
                canonical.as_object_mut().unwrap().remove(key);
            }
            upstream_headers.insert(
                "x-codex-turn-metadata",
                HeaderValue::from_str(&ascii_json(&canonical)?).map_err(storage)?,
            );
        }
        Ok(Some(Plan {
            headers: upstream_headers,
            body: upstream_body,
            base_revision,
        }))
    }

    pub(super) fn commit_rebuild(
        &self,
        binding: &Binding,
        event: &Value,
    ) -> Result<(), ServiceError> {
        let Some(generation) = &binding.generation else {
            return Ok(());
        };
        if generation
            .cancelled
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Ok(());
        }

        let Some(thread) = binding.thread_id.as_deref() else {
            return Ok(());
        };
        let response = event.get("response").unwrap_or(event);
        if response.get("output").and_then(Value::as_array).is_none() {
            return Ok(());
        }
        let Some(id) = response.get("id").and_then(Value::as_str) else {
            return Ok(());
        };
        let output = response["output"].as_array().unwrap();
        let streamed = generation.output.lock().map_err(storage)?;
        let complete_output = replay_items(output)
            && streamed.iter().all(|(index, digest)| {
                output.get(*index as usize).is_some_and(|item| {
                    format!("{:x}", Sha256::digest(item.to_string().as_bytes())) == *digest
                })
            });
        drop(streamed);
        let checkpoint = if complete_output
            && (if binding.portable_body {
                replay_items(output)
            } else {
                plain_items(output)
            }) {
            generation
                .input
                .as_ref()
                .map(|input| {
                    checkpoint_store::Encoded::prepare(
                        input,
                        output,
                        generation.parameters.as_ref(),
                    )
                })
                .transpose()?
        } else {
            None
        };
        let mut connection = self.connection.lock().map_err(storage)?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage)?;
        if generation
            .cancelled
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Ok(());
        }
        let mut activated = false;
        if generation.activate
            && complete_output
            && !generation
                .cancelled
                .load(std::sync::atomic::Ordering::Acquire)
        {
            if generation.base_revision == 0 {
                activated = transaction.execute("INSERT OR IGNORE INTO native_rebuild_active_v1(caller,thread,account,owner,generation,revision) VALUES(?1,?2,?3,?4,?5,1)",params![binding.caller,thread,binding.account_id,binding.owner_id,generation.id]).map_err(storage)? == 1;
            } else {
                activated = transaction.execute("UPDATE native_rebuild_active_v1 SET account=?3,owner=?4,generation=?5,revision=revision+1 WHERE caller=?1 AND thread=?2 AND revision=?6",params![binding.caller,thread,binding.account_id,binding.owner_id,generation.id,generation.base_revision]).map_err(storage)? == 1;
            }
        }
        if activated {
            for (kind, value) in [
                ("thread", binding.thread_id.as_ref()),
                ("session", binding.session_id.as_ref()),
            ] {
                if let Some(value) =
                    value.filter(|_| kind != "session" || binding.parent_thread_id.is_none())
                {
                    save_affinity(
                        &transaction,
                        &binding.caller,
                        kind,
                        value,
                        &binding.account_id,
                        &binding.owner_id,
                    )?;
                }
            }
        }
        if binding.portable_body && complete_output {
            automatic_history::accept_input(&transaction, binding)?;
            transaction.execute("INSERT OR IGNORE INTO native_rebuild_portable_v1(caller,thread,generation,account,owner) VALUES(?1,?2,?3,?4,?5)",params![binding.caller,thread,generation.id,binding.account_id,binding.owner_id]).map_err(storage)?;
            for reference in &binding.input_blobs {
                transaction.execute("INSERT OR IGNORE INTO native_rebuild_blobs_v1(caller,thread,generation,digest,account,owner) VALUES(?1,?2,?3,?4,?5,?6)",params![binding.caller,thread,generation.id,reference.digest,binding.account_id,binding.owner_id]).map_err(storage)?;
            }
        }
        if let Some(checkpoint) = checkpoint {
            checkpoint.publish(&transaction, binding, generation, thread, id)?;
        }
        // Only the hot cache is evicted. Accepted history, identity and ciphertext
        // ownership remain durable, including for large and old response IDs.
        loop {
            let (count, bytes): (i64, i64) = transaction
                .query_row(
                    "SELECT COUNT(*),COALESCE(SUM(bytes),0) FROM native_rebuild_checkpoints_v1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(storage)?;
            if count <= MAX_CHECKPOINTS && bytes <= MAX_CACHE_BYTES {
                break;
            }
            // A pre-upgrade hot row may be its response's only history copy.
            // Pin those bounded legacy rows rather than scanning/migrating all
            // history at startup or silently erasing their continuation proof.
            let removed = transaction.execute("DELETE FROM native_rebuild_checkpoints_v1 WHERE rowid=(SELECT c.rowid FROM native_rebuild_checkpoints_v1 c WHERE EXISTS(SELECT 1 FROM native_rebuild_history_v1 h WHERE h.caller=c.caller AND h.response=c.response) ORDER BY c.created,c.rowid LIMIT 1)",[]).map_err(storage)?;
            if removed == 0 {
                break;
            }
        }
        // Publish at most one complete generation, or let cancellation win and
        // roll the entire transaction back, including newly accepted input.
        if generation
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

impl RequestIdentity {
    pub(crate) fn start_rebuild_generation(&mut self, plan: &Plan) {
        self.rebuild_base_revision = Some(plan.base_revision);
        self.rebuild_new_generation = true;
        self.generation = None;
    }
    pub(crate) fn retain_binding_generation(&mut self, binding: &Binding) {
        if let Some(generation) = &binding.generation {
            self.generation = Some(generation.id.clone());
        }
    }
    pub(crate) fn disable_checkpoints(&mut self) {
        self.rebuild_enabled = false;
        self.rebuild_input = None;
        self.rebuild_parameters = None;
    }
    pub(crate) fn disable_rebuild(&mut self) {
        self.disable_checkpoints();
        self.portable_body = false;
        self.detached_parent = false;
        self.detached_turn_states.clear();
    }
}

impl Binding {
    pub(crate) fn rebuild_abort_handle(&self) -> Option<Arc<std::sync::atomic::AtomicBool>> {
        self.generation
            .as_ref()
            .map(|generation| generation.cancelled.clone())
            .or_else(|| {
                self.pending_input
                    .as_ref()
                    .map(|pending| pending.cancelled.clone())
            })
    }
    pub(crate) fn abort_rebuild(&self) {
        if let Some(abort) = self.rebuild_abort_handle() {
            abort.store(true, std::sync::atomic::Ordering::Release);
        }
    }
}

#[cfg(test)]
#[path = "identity_rebuild_tests.rs"]
mod tests;
