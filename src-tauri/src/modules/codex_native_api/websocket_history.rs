//! Connection-local HTTP replay. Every cached item remains scoped to its original owner.
use super::super::{contract::ServiceError, mapping::Prepared};
use serde_json::{json, Value};
use std::collections::{BTreeMap, VecDeque};

const MAX_CALLS: usize = 512;
// Only the auxiliary call LRU is bounded; the one current anchor stays complete.
const MAX_CACHE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Default)]
pub(super) struct History {
    scope: Option<ReplayOwner>,
    checkpoint: Option<Checkpoint>,
    prewarm: Option<(String, Value)>,
    calls: BTreeMap<String, Value>,
    order: VecDeque<String>,
    call_bytes: usize,
    output: BTreeMap<usize, Value>,
    output_bytes: usize,
    output_invalid_for_rebuild: bool,
}
struct Checkpoint {
    id: String,
    input: Vec<Value>,
    output: Vec<Value>,
    connection_local: bool,
    complete_for_rebuild: bool,
    owner: ReplayOwner,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct ReplayOwner {
    pub account: String,
    owner: String,
    caller: String,
    session: Option<String>,
    thread: Option<String>,
}
impl ReplayOwner {
    fn from_prepared(prepared: &Prepared) -> Self {
        Self {
            account: prepared.route.continuity_id().to_owned(),
            owner: prepared.route.owner_id.clone(),
            caller: prepared.caller.id.clone(),
            session: mapped_header(prepared, "session-id"),
            thread: mapped_header(prepared, "thread-id"),
        }
    }
    pub fn matches(&self, prepared: &Prepared) -> bool {
        self.account == prepared.route.continuity_id()
            && self.owner == prepared.route.owner_id
            && self.caller == prepared.caller.id
            && self.session == mapped_header(prepared, "session-id")
            && self.thread == mapped_header(prepared, "thread-id")
    }
}
fn mapped_header(prepared: &Prepared, name: &str) -> Option<String> {
    prepared
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

impl History {
    /// An incomplete HTTP response is only a connection-local replay anchor.
    /// Expand it before durable completed-response lookup, then hard-pin every
    /// account and mapped identity field before any request can reach upstream.
    pub fn resolve_local(
        &self,
        prepared: &Prepared,
        mut body: Value,
    ) -> Result<(Value, Option<ReplayOwner>), ServiceError> {
        let Some(checkpoint) = &self.checkpoint else {
            return Ok((body, None));
        };
        if !checkpoint.connection_local
            || body.get("previous_response_id").and_then(Value::as_str) != Some(&checkpoint.id)
        {
            return Ok((body, None));
        }
        if !checkpoint.owner.matches(prepared) {
            return Err(ServiceError::new(
                409,
                "continuation_owner_mismatch",
                "The partial conversation belongs to a different account or thread",
            ));
        }
        let input = body
            .get("input")
            .and_then(Value::as_array)
            .ok_or_else(invalid_input)?;
        let mut previous = checkpoint.input.clone();
        previous.extend(checkpoint.output.iter().cloned());
        body["input"] = Value::Array(extend_input(&previous, input));
        body.as_object_mut().unwrap().remove("previous_response_id");
        Ok((body, Some(checkpoint.owner.clone())))
    }

    /// Only a completed, fully observed chain can replace a server-side
    /// continuation when its owner is no longer selectable. Partial anchors and
    /// an initial delta without its root never authorize cross-account replay.
    pub fn completed_replay(
        &self,
        prepared: &Prepared,
        body: &Value,
    ) -> Result<Option<Value>, ServiceError> {
        let Some(checkpoint) = &self.checkpoint else {
            return Ok(None);
        };
        if checkpoint.connection_local
            || !checkpoint.complete_for_rebuild
            || !checkpoint.owner.matches(prepared)
            || body.get("previous_response_id").and_then(Value::as_str) != Some(&checkpoint.id)
        {
            return Ok(None);
        }
        let Some(input) = self.logical_input(body) else {
            return Ok(None);
        };
        if !super::super::identity::rebuild::replay_items(&input)
            || !super::super::identity::rebuild::paired_tools(&input)
        {
            return Ok(None);
        }
        let mut full = body.clone();
        full["input"] = Value::Array(input);
        full.as_object_mut().unwrap().remove("previous_response_id");
        Ok(Some(full))
    }

    pub fn resolve_prewarm(&mut self, mut body: Value) -> Result<Value, ServiceError> {
        let Some((id, previous)) = &self.prewarm else {
            return Ok(body);
        };
        if body.get("previous_response_id").and_then(Value::as_str) != Some(id) {
            return Ok(body);
        }
        let input = body
            .get("input")
            .and_then(Value::as_array)
            .ok_or_else(invalid_input)?;
        let previous = previous
            .get("input")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        body["input"] = Value::Array(extend_input(&previous, input));
        body.as_object_mut().unwrap().remove("previous_response_id");
        self.prewarm = None;
        Ok(body)
    }

    pub fn begin(&mut self, prepared: &Prepared) {
        self.begin_scope(ReplayOwner::from_prepared(prepared));
        if let Some(input) = prepared
            .replay_body()
            .get("input")
            .and_then(Value::as_array)
        {
            for item in input {
                self.remember_call(item);
            }
        }
    }

    fn begin_scope(&mut self, scope: ReplayOwner) {
        if self.scope.as_ref() != Some(&scope) {
            self.checkpoint = None;
            self.calls.clear();
            self.order.clear();
            self.call_bytes = 0;
            self.scope = Some(scope);
        }
        self.output.clear();
        self.output_bytes = 0;
        self.output_invalid_for_rebuild = false;
    }

    pub fn prewarm(&mut self, body: &Value) -> Vec<Value> {
        let id = format!("resp_prewarm_{}", uuid::Uuid::now_v7());
        self.prewarm = Some((id.clone(), body.clone()));
        let response = json!({"id":id,"object":"response","created_at":chrono::Utc::now().timestamp(),"model":body["model"],"status":"completed","output":[],"usage":{"input_tokens":0,"output_tokens":0,"total_tokens":0}});
        let mut created = response.clone();
        created["status"] = json!("in_progress");
        vec![
            json!({"type":"response.created","response":created}),
            json!({"type":"response.completed","response":response}),
        ]
    }

    pub fn replay(&mut self, prepared: &Prepared) -> Result<Value, ServiceError> {
        if self
            .scope
            .as_ref()
            .is_some_and(|scope| !scope.matches(prepared))
        {
            self.begin(prepared);
        }
        let mut body = prepared.replay_body().clone();
        let input = self.logical_input(&body);
        if let Some(input) = input {
            body["input"] = Value::Array(input);
            body.as_object_mut().unwrap().remove("previous_response_id");
        } else if body
            .get("previous_response_id")
            .and_then(Value::as_str)
            .is_some()
            && prepared.route.wire_api == super::super::contract::WireApi::Chat
        {
            let input = body
                .get("input")
                .and_then(Value::as_array)
                .ok_or_else(invalid_input)?;
            if !full_transcript(input) {
                return Err(ServiceError::new(409,"previous_response_not_found","The HTTP Chat connection requires the full conversation input for this checkpoint"));
            }
            body.as_object_mut().unwrap().remove("previous_response_id");
        }
        let input = body
            .get("input")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(invalid_input)?;
        body["input"] = Value::Array(self.repair_tools(&input)?);
        body.as_object_mut().unwrap().remove("generate");
        Ok(body)
    }

    fn logical_input(&self, body: &Value) -> Option<Vec<Value>> {
        let checkpoint = self.checkpoint.as_ref()?;
        if body.get("previous_response_id").and_then(Value::as_str) != Some(&checkpoint.id) {
            return None;
        }
        let incoming = body.get("input")?.as_array()?;
        let mut previous = checkpoint.input.clone();
        previous.extend(checkpoint.output.iter().cloned());
        Some(extend_input(&previous, incoming))
    }

    pub fn invalidate_rebuild_proof(&mut self) {
        self.output_invalid_for_rebuild = true;
    }

    pub fn observe(&mut self, prepared: &Prepared, event: &Value) {
        // A late terminal from an interrupted request cannot replace its local
        // anchor or promote partial output into cross-account replay evidence.
        if matches!(
            event["type"].as_str(),
            Some("response.completed" | "response.done")
        ) && self.checkpoint.as_ref().is_some_and(|checkpoint| {
            checkpoint.connection_local
                && event.pointer("/response/id").and_then(Value::as_str)
                    == Some(checkpoint.id.as_str())
        }) {
            return;
        }
        if super::super::events::response_terminal_kind(event)
            .is_some_and(|kind| !matches!(kind, "response.completed" | "response.done"))
            || event.get("error").is_some_and(|error| !error.is_null())
            || event
                .pointer("/response/error")
                .is_some_and(|error| !error.is_null())
        {
            self.output_invalid_for_rebuild = true;
        }
        if event["type"] == "response.output_item.done" {
            if let Some(item) = event.get("item") {
                self.remember_call(item);
                let bytes = item.to_string().len();
                let index = event
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .and_then(|v| usize::try_from(v).ok())
                    .unwrap_or(self.output.len());
                if event.get("output_index").and_then(Value::as_u64).is_none()
                    || self
                        .output
                        .get(&index)
                        .is_some_and(|previous| previous != item)
                {
                    self.output_invalid_for_rebuild = true;
                }
                let previous = self
                    .output
                    .get(&index)
                    .map(|value| value.to_string().len())
                    .unwrap_or(0);
                let size = self
                    .output_bytes
                    .saturating_sub(previous)
                    .saturating_add(bytes);
                self.output.insert(index, item.clone());
                self.output_bytes = size;
            }
        }
        if !matches!(
            event["type"].as_str(),
            Some("response.completed" | "response.done")
        ) || event.pointer("/response/status").and_then(Value::as_str) != Some("completed")
        {
            return;
        }
        let Some(id) = event
            .pointer("/response/id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        else {
            return;
        };
        self.save_checkpoint(
            prepared,
            id,
            event.pointer("/response/output").and_then(Value::as_array),
        );
    }

    fn save_checkpoint(
        &mut self,
        prepared: &Prepared,
        id: &str,
        terminal_output: Option<&Vec<Value>>,
    ) {
        let root_known = prepared
            .replay_body()
            .get("previous_response_id")
            .is_none_or(Value::is_null)
            || self.checkpoint.as_ref().is_some_and(|checkpoint| {
                checkpoint.complete_for_rebuild
                    && !checkpoint.connection_local
                    && prepared
                        .replay_body()
                        .get("previous_response_id")
                        .and_then(Value::as_str)
                        == Some(&checkpoint.id)
            });
        let mut input = self
            .logical_input(&prepared.replay_body())
            .or_else(|| {
                prepared
                    .replay_body()
                    .get("input")
                    .and_then(Value::as_array)
                    .cloned()
            })
            .unwrap_or_default();
        input.retain(|item| item["type"] != "compaction_trigger");
        let complete_for_rebuild = root_known
            && super::super::identity::rebuild::replay_items(&input)
            && super::super::identity::rebuild::paired_tools(&input)
            && !self.output_invalid_for_rebuild
            && terminal_output.is_some_and(|output| {
                output.iter().all(replayable_output)
                    && self
                        .output
                        .iter()
                        .all(|(index, item)| output.get(*index) == Some(item))
            });
        let mut output: Vec<Value> = terminal_output
            .into_iter()
            .flatten()
            .filter(|item| replayable_output(item))
            .cloned()
            .collect();
        if output.is_empty() {
            output = self
                .output
                .values()
                .filter(|item| replayable_output(item))
                .cloned()
                .collect();
        }
        for item in &output {
            self.remember_call(item);
        }
        // Retain one full current anchor. The auxiliary call cache has its own
        // LRU budget and must not truncate conversation or interruption state.
        self.checkpoint = Some(Checkpoint {
            id: id.into(),
            input,
            output,
            connection_local: false,
            complete_for_rebuild,
            owner: ReplayOwner::from_prepared(prepared),
        });
        self.output.clear();
        self.output_bytes = 0;
    }

    /// A real incomplete HTTP turn can resume only by local full-input replay.
    /// It remains a failed/incomplete upstream result, never a durable response ID.
    pub fn incomplete_anchor(&mut self, prepared: &Prepared, event: &Value) -> Option<String> {
        if event["type"] != "response.incomplete"
            || event.pointer("/response/status").and_then(Value::as_str) != Some("incomplete")
        {
            return None;
        }
        let terminal_output = event.pointer("/response/output").and_then(Value::as_array);
        if prepared
            .replay_body()
            .get("input")
            .and_then(Value::as_array)
            .is_none()
            || (terminal_output.is_none_or(Vec::is_empty)
                && self.output.is_empty()
                && event
                    .pointer("/response/incomplete_details/reason")
                    .and_then(Value::as_str)
                    != Some("interrupted"))
        {
            return None;
        }
        let id = event
            .pointer("/response/id")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())?;
        self.save_checkpoint(prepared, id, terminal_output);
        let checkpoint = self.checkpoint.as_mut()?;
        checkpoint.connection_local = true;
        Some(id.to_owned())
    }

    pub fn interrupted(&mut self, prepared: &Prepared, id: &str) -> Result<Value, ServiceError> {
        let output: Vec<_> = self
            .output
            .values()
            .filter(|item| replayable_output(item))
            .cloned()
            .collect();
        self.save_checkpoint(prepared, id, Some(&output));
        if let Some(checkpoint) = self.checkpoint.as_mut() {
            checkpoint.connection_local = true;
        }
        Ok(
            json!({"type":"response.incomplete","response":{"id":id,"status":"incomplete","incomplete_details":{"reason":"interrupted"},"output":output}}),
        )
    }

    fn remember_call(&mut self, item: &Value) {
        if !super::super::output_items::complete_tool_call(item) {
            return;
        }
        let Some(id) = item["call_id"].as_str().filter(|id| !id.is_empty()) else {
            return;
        };
        let bytes = item.to_string().len();
        if let Some(old) = self.calls.remove(id) {
            self.call_bytes = self.call_bytes.saturating_sub(old.to_string().len());
            self.order.retain(|old| old != id);
        }
        self.calls.insert(id.into(), item.clone());
        self.order.push_back(id.into());
        self.call_bytes += bytes;
        // A single complete call may exceed the auxiliary budget; never discard
        // its arguments solely for that reason. Full current history also stays
        // available when later calls evict this entry from the LRU.
        while self.calls.len() > MAX_CALLS
            || (self.calls.len() > 1 && self.call_bytes > MAX_CACHE_BYTES)
        {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            if let Some(item) = self.calls.remove(&old) {
                self.call_bytes = self.call_bytes.saturating_sub(item.to_string().len());
            }
        }
    }

    fn repair_tools(&self, input: &[Value]) -> Result<Vec<Value>, ServiceError> {
        // Ambiguous history belongs to the upstream. Do not choose a cached
        // call or partly repair this payload before discovering a conflict.
        let mut original_calls = BTreeMap::new();
        for item in input {
            if matches!(
                item["type"].as_str(),
                Some("function_call" | "custom_tool_call")
            ) {
                if let Some(id) = item["call_id"].as_str() {
                    if original_calls
                        .insert(id, item)
                        .is_some_and(|previous| previous != item)
                    {
                        return Ok(input.to_vec());
                    }
                }
            }
        }
        // Index borrowed references once. Prefer complete current input/output
        // and checkpoint calls over the bounded auxiliary LRU, without copying
        // large arguments into an additional repair cache.
        let previous = self
            .checkpoint
            .iter()
            .flat_map(|checkpoint| checkpoint.input.iter().chain(checkpoint.output.iter()));
        let mut available = BTreeMap::new();
        for item in self
            .calls
            .values()
            .chain(previous)
            .chain(self.output.values())
            .chain(input.iter())
        {
            if super::super::output_items::complete_tool_call(item) {
                if let Some(id) = item["call_id"].as_str() {
                    available.insert(id, item);
                }
            }
        }
        let mut seen = BTreeMap::<&str, &Value>::new();
        let mut result = Vec::new();
        for item in input {
            match item["type"].as_str() {
                Some("function_call" | "custom_tool_call") => {
                    if let Some(id) = item["call_id"].as_str() {
                        if seen.contains_key(id) {
                            continue;
                        }
                        seen.insert(id, item);
                    }
                }
                Some("function_call_output" | "custom_tool_call_output") => {
                    if let Some(id) = item["call_id"].as_str() {
                        if !seen.contains_key(id) {
                            if let Some(call) = available.get(id) {
                                result.push((*call).clone());
                                seen.insert(id, *call);
                            }
                        }
                    }
                }
                _ => {}
            }
            result.push(item.clone());
        }
        Ok(result)
    }
}
fn replayable_output(item: &Value) -> bool {
    !matches!(
        item["type"].as_str(),
        Some("function_call" | "custom_tool_call")
    ) || super::super::output_items::complete_tool_call(item)
}
fn invalid_input() -> ServiceError {
    ServiceError::new(
        400,
        "invalid_websocket_input",
        "WebSocket response input must be an array",
    )
}
fn extend_input(previous: &[Value], incoming: &[Value]) -> Vec<Value> {
    if incoming.starts_with(previous) {
        incoming.to_vec()
    } else {
        previous.iter().chain(incoming.iter()).cloned().collect()
    }
}
fn full_transcript(input: &[Value]) -> bool {
    input.iter().any(|item| {
        item["role"] == "assistant"
            || matches!(
                item["type"].as_str(),
                Some("function_call" | "custom_tool_call" | "compaction")
            )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prewarm_checkpoint_materializes_only_its_own_input_and_keeps_unknown_fields() {
        let mut history = History::default();
        let events =
            history.prewarm(&json!({"model":"m","input":[{"role":"user","content":"first"}]}));
        let next=history.resolve_prewarm(json!({"model":"m","previous_response_id":events[1]["response"]["id"],"input":[{"role":"user","content":"next"}],"extension":true})).unwrap();
        assert_eq!(next["input"].as_array().unwrap().len(), 2);
        assert_eq!(next["extension"], true);
        assert!(next.get("previous_response_id").is_none());
    }
    #[test]
    fn repair_uses_only_exact_cached_calls_and_preserves_conflicting_ids() {
        let mut history = History::default();
        let call = json!({"type":"function_call","call_id":"exact","name":"tool","arguments":"{}"});
        history.remember_call(&call);
        let output = json!({"type":"function_call_output","call_id":"exact","output":"ok"});
        assert_eq!(
            history.repair_tools(&[output.clone()]).unwrap(),
            vec![call.clone(), output]
        );
        assert_eq!(
            history
                .repair_tools(&[
                    json!({"type":"function_call_output","call_id":"other","output":"ok"})
                ])
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            history.repair_tools(&[call.clone(), call.clone()]).unwrap(),
            vec![call.clone()]
        );
        let mut conflict = call.clone();
        conflict["arguments"] = json!("different");
        let input = vec![call, conflict];
        assert_eq!(history.repair_tools(&input).unwrap(), input);
    }
    include!("websocket_history_regression_tests.rs");
}
