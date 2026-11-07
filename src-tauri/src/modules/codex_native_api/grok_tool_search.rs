//! Request-local lowering of client tool discovery; hosted search stays hosted.
use super::*;

fn key() -> ToolKey {
    ToolKey {
        namespace: None,
        name: "tool_search".into(),
        custom: false,
        search: true,
    }
}

pub(super) fn client_declaration(tool: &Value) -> bool {
    super::super::client_tool_discovery::client_declaration(tool)
}
pub(super) fn lower_declaration(
    tool: &Value,
    context: &mut Context,
) -> Result<Value, ServiceError> {
    let wire = context.bind(key())?;
    super::super::client_tool_discovery::lower_declaration(tool, &wire)
}
pub(super) fn prepare(body: &mut Value, context: &mut Context) -> Result<(), ServiceError> {
    let discovery = super::super::client_tool_discovery::promote(body)?;
    collect_names(body, &mut context.reserved, 0)?;
    let wire = if discovery.active() {
        context.bind(key())?
    } else {
        String::new()
    };
    super::super::client_tool_discovery::lower_history(body, &wire, &discovery)
}

pub(super) fn rewrite_choice(choice: &mut Value, context: &Context) {
    if text(choice, "type") == Some("tool_search") && text(choice, "execution") != Some("server") {
        if let Some(wire) = context.by_key.get(&key()) {
            choice["type"] = json!("function");
            choice["name"] = json!(wire);
            choice.as_object_mut().unwrap().remove("execution");
        }
    }
}

pub(super) fn wire_item_id(id: &str) -> String {
    id.strip_prefix("tsc_")
        .map(|suffix| format!("fc_{suffix}"))
        .unwrap_or_else(|| id.to_owned())
}
fn client_item_id(id: &str) -> String {
    id.strip_prefix("fc_")
        .map(|suffix| format!("tsc_{suffix}"))
        .unwrap_or_else(|| id.to_owned())
}
fn decode_arguments(raw: &str, partial: bool) -> Result<Value, ServiceError> {
    if raw.is_empty() && partial {
        return Ok(json!({}));
    }
    let arguments: Value = serde_json::from_str(raw)
        .map_err(|_| upstream("Grok returned invalid tool search arguments"))?;
    if !arguments.is_object() {
        return Err(upstream("Grok tool search arguments must be an object"));
    }
    Ok(arguments)
}
pub(super) fn restore_item(item: &mut Value, partial: bool) -> Result<(), ServiceError> {
    let raw = text(item, "arguments").unwrap_or("");
    let arguments = decode_arguments(raw, partial)?;
    item["type"] = json!("tool_search_call");
    item["execution"] = json!("client");
    item["arguments"] = arguments;
    if let Some(id) = text(item, "id").map(str::to_owned) {
        item["id"] = json!(client_item_id(&id));
    }
    let object = item.as_object_mut().unwrap();
    object.remove("name");
    object.remove("namespace");
    Ok(())
}
pub(super) fn stream_key(context: &Context, event: &Value) -> Option<String> {
    let keys = [
        call_key(event),
        text(event, "call_id").map(|id| format!("call:{id}")),
        event
            .get("output_index")
            .and_then(Value::as_u64)
            .map(|index| format!("index:{index}")),
    ];
    keys.into_iter().flatten().find_map(|key| {
        context
            .call_aliases
            .get(&key)
            .cloned()
            .or_else(|| context.calls.contains_key(&key).then_some(key))
    })
}
pub(super) fn arguments_event(
    context: &mut Context,
    key: &str,
    event: &Value,
) -> Result<bool, ServiceError> {
    if !context
        .calls
        .get(key)
        .and_then(|call| call.binding.as_ref())
        .is_some_and(|binding| binding.key.search)
    {
        return Ok(false);
    }
    if text(event, "type") == Some("response.function_call_arguments.delta") {
        let delta = text(event, "delta")
            .ok_or_else(|| upstream("Grok tool argument delta must be text"))?;
        context.account(delta.len())?;
        context
            .calls
            .get_mut(key)
            .unwrap()
            .arguments
            .push_str(delta);
    } else {
        let arguments = text(event, "arguments")
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| context.calls[key].arguments.clone());
        decode_arguments(&arguments, false)?;
        if arguments.len() > MAX_STATE_BYTES {
            return Err(upstream("Grok tool arguments exceed the stream limit"));
        }
        context.calls.get_mut(key).unwrap().arguments = arguments;
    }
    // Client tool search is dispatched from completed output items, not from
    // function-argument events (which would expose the private proxy function).
    Ok(true)
}
pub(super) fn complete_item(context: &Context, event: &mut Value) -> Result<(), ServiceError> {
    let mut lookup = event.clone();
    if let Some(id) = event.pointer("/item/id").cloned() {
        lookup["item_id"] = id;
    }
    if let Some(key) = stream_key(context, &lookup) {
        if let Some(call) = context.calls.get(&key).filter(|call| {
            call.binding
                .as_ref()
                .is_some_and(|binding| binding.key.search)
        }) {
            if event
                .pointer("/item/arguments")
                .is_none_or(|value| value.as_str().is_some_and(str::is_empty))
                && !call.arguments.is_empty()
            {
                event["item"]["arguments"] = json!(call.arguments);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "grok_tool_search_tests.rs"]
mod tests;
