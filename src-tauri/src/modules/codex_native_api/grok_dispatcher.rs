//! Over-limit namespaces become callable dispatchers. No declared child is discarded.
//! Dispatcher streams are buffered until the selected child is known, then flow
//! through the same custom/function restoration as ordinary calls.
use super::*;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
struct Group {
    children: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Default)]
struct Pending {
    dispatcher: String,
    added: Option<Value>,
    arguments: String,
    emitted: bool,
    resolved: Option<(String, String)>,
    item_done: bool,
}
#[derive(Clone, Debug, Default)]
pub(super) struct State {
    groups: HashMap<String, Group>,
    pending: HashMap<String, Pending>,
    aliases: HashMap<String, String>,
}

pub(super) fn prepare(body: &mut Value, context: &mut Context) -> Result<(), ServiceError> {
    let Some(tools) = body
        .get("tools")
        .and_then(Value::as_array)
        .filter(|tools| tools.len() > 200)
        .cloned()
    else {
        return Ok(());
    };
    let mut namespaces: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for tool in &tools {
        if let Some(binding) = text(tool, "name").and_then(|name| context.by_wire.get(name)) {
            if let Some(namespace) = &binding.key.namespace {
                if !binding.key.search {
                    namespaces
                        .entry(namespace.clone())
                        .or_default()
                        .push(tool.clone());
                }
            }
        }
    }
    let forced = body
        .pointer("/tool_choice/name")
        .or_else(|| body.pointer("/tool_choice/function/name"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut child_dispatcher: HashMap<String, (String, String)> = HashMap::new();
    let mut declarations = HashMap::new();
    let mut catalog_bytes = 0usize;
    for (namespace, children) in namespaces
        .into_iter()
        .filter(|(_, children)| children.len() > 1)
    {
        let digest = format!("{:x}", Sha256::digest(namespace.as_bytes()));
        let base = format!("grok_namespace_{}", &digest[..32]);
        let mut wire = base.clone();
        let mut suffix = 0;
        while !context.reserved.insert(wire.clone()) {
            suffix += 1;
            wire = format!("{base}_{suffix}");
        }
        let selected = forced.as_ref().filter(|name| {
            children
                .iter()
                .any(|tool| text(tool, "name") == Some(name.as_str()))
        });
        let mut counts = HashMap::new();
        for child in &children {
            let binding = &context.by_wire[text(child, "name").unwrap()];
            *counts.entry(binding.key.name.clone()).or_insert(0usize) += 1;
        }
        let mut group = Group {
            children: BTreeMap::new(),
        };
        let mut catalog = Vec::new();
        for child in &children {
            let child_wire = text(child, "name").unwrap();
            let binding = &context.by_wire[child_wire];
            let selector = if counts[&binding.key.name] > 1 {
                child_wire.to_owned()
            } else {
                binding.key.name.clone()
            };
            // Keep the history mapping for every child even when a forced
            // choice narrows the new dispatcher declaration to one child.
            child_dispatcher.insert(child_wire.into(), (wire.clone(), selector.clone()));
            if selected.is_some_and(|selected| selected != child_wire) {
                continue;
            }
            group.children.insert(selector.clone(), child_wire.into());
            let mut entry = child.clone();
            entry["name"] = json!(selector);
            entry["original_name"] = json!(binding.key.name);
            entry["original_type"] = json!(if binding.key.custom {
                "custom"
            } else {
                "function"
            });
            catalog_bytes = catalog_bytes.saturating_add(
                serde_json::to_vec(&entry)
                    .map_err(|_| invalid("Invalid Grok namespace catalog"))?
                    .len(),
            );
            if catalog_bytes > MAX_STATE_BYTES {
                return Err(invalid("Grok namespace catalog exceeds its size limit"));
            }
            catalog.push(entry);
        }
        let names: Vec<Value> = catalog.iter().map(|entry| entry["name"].clone()).collect();
        let description = format!("Tools in namespace {namespace}. Select exactly one child using name. Pass its arguments object (or exact JSON text) in arguments. Child tool declarations:\n{}", serde_json::to_string(&catalog).map_err(|_| invalid("Invalid Grok namespace catalog"))?);
        if description.len() > MAX_STATE_BYTES {
            return Err(invalid("Grok namespace catalog exceeds its size limit"));
        }
        declarations.insert(wire.clone(), json!({"type":"function","name":wire,"description":description,"parameters":{"type":"object","properties":{"name":{"type":"string","enum":names},"arguments":{"description":"Arguments for the selected child tool. Custom tools use the documented input string wrapper.","anyOf":[{"type":"object","additionalProperties":true},{"type":"string"}]}},"required":["name","arguments"],"additionalProperties":false}}));
        context.dispatchers.groups.insert(wire, group);
    }
    if child_dispatcher.is_empty() {
        return Ok(());
    }
    let mut output = Vec::new();
    let mut emitted = HashSet::new();
    for tool in tools {
        if let Some((wire, _)) = text(&tool, "name").and_then(|name| child_dispatcher.get(name)) {
            if emitted.insert(wire.clone()) {
                output.push(declarations.remove(wire).unwrap());
            }
        } else {
            output.push(tool);
        }
    }
    body["tools"] = json!(output);
    if let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) {
        for item in input {
            if text(item, "type") != Some("function_call") {
                continue;
            }
            if let Some((wire, child)) =
                text(item, "name").and_then(|name| child_dispatcher.get(name))
            {
                let raw = text(item, "arguments").unwrap_or("{}");
                item["arguments"] = json!(json!({"name":child,"arguments":raw}).to_string());
                item["name"] = json!(wire);
            }
        }
    }
    if let Some(forced) = forced.and_then(|name| child_dispatcher.get(&name)) {
        if let Some(choice) = body.get_mut("tool_choice") {
            if let Some(function) = choice.get_mut("function") {
                function["name"] = json!(forced.0);
            } else {
                choice["name"] = json!(forced.0);
            }
        }
    }
    Ok(())
}

// Scan a validated JSON object to retain raw argument numbers and whitespace.
// Deserializing the nested argument object would round numbers beyond u64.
fn fields(raw: &str) -> Result<Vec<(String, &str)>, ServiceError> {
    let parsed: Value = serde_json::from_str(raw)
        .map_err(|_| upstream("Invalid Grok namespace dispatcher arguments"))?;
    if !parsed.is_object() {
        return Err(upstream(
            "Grok namespace dispatcher arguments must be an object",
        ));
    }
    let bytes = raw.as_bytes();
    let mut cursor = bytes.iter().position(|b| *b == b'{').unwrap() + 1;
    let mut fields = Vec::new();
    loop {
        while bytes
            .get(cursor)
            .is_some_and(|b| b.is_ascii_whitespace() || *b == b',')
        {
            cursor += 1;
        }
        if bytes.get(cursor) == Some(&b'}') {
            break;
        }
        let key_start = cursor;
        cursor = string_end(bytes, cursor);
        let key: String = serde_json::from_str(&raw[key_start..cursor])
            .map_err(|_| upstream("Invalid Grok dispatcher field"))?;
        while bytes
            .get(cursor)
            .is_some_and(|b| b.is_ascii_whitespace() || *b == b':')
        {
            cursor += 1;
        }
        let start = cursor;
        let mut depth = 0usize;
        while let Some(byte) = bytes.get(cursor) {
            match *byte {
                b'"' => {
                    cursor = string_end(bytes, cursor);
                    continue;
                }
                b'{' | b'[' => depth += 1,
                b'}' | b']' if depth > 0 => depth -= 1,
                b',' | b'}' if depth == 0 => break,
                _ => {}
            }
            cursor += 1;
        }
        fields.push((key, raw[start..cursor].trim()));
    }
    Ok(fields)
}
fn string_end(bytes: &[u8], mut cursor: usize) -> usize {
    cursor += 1;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\\' => cursor += 2,
            b'"' => return cursor + 1,
            _ => cursor += 1,
        }
    }
    bytes.len()
}
fn unwrap(group: &Group, raw: &str) -> Result<(String, String), ServiceError> {
    if raw.len() > MAX_STATE_BYTES {
        return Err(upstream("Grok namespace arguments exceed the stream limit"));
    }
    let values = fields(raw)?;
    let name = values
        .iter()
        .rev()
        .find(|(key, _)| key == "name")
        .and_then(|(_, value)| serde_json::from_str::<String>(value).ok())
        .ok_or_else(|| upstream("Grok namespace dispatcher requires a child name"))?;
    let wire = group
        .children
        .get(&name)
        .ok_or_else(|| upstream("Grok namespace dispatcher selected an undeclared child"))?;
    let arguments = if let Some((_, raw)) = values.iter().rev().find(|(key, _)| key == "arguments")
    {
        if raw.starts_with('"') {
            serde_json::from_str::<String>(raw)
                .map_err(|_| upstream("Invalid Grok child arguments"))?
        } else {
            raw.to_string()
        }
    } else {
        format!(
            "{{{}}}",
            values
                .iter()
                .filter(|(key, _)| key != "name")
                .map(|(key, value)| format!("{}:{value}", json!(key)))
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    if !serde_json::from_str::<Value>(&arguments).is_ok_and(|value| value.is_object()) {
        return Err(upstream("Grok child arguments must be a JSON object"));
    }
    Ok((wire.clone(), arguments))
}
fn unwrap_item(state: &State, item: &mut Value) -> Result<(), ServiceError> {
    if text(item, "type") != Some("function_call") {
        return Ok(());
    }
    let pending = keys(&json!({"item":item}))
        .iter()
        .find_map(|key| state.aliases.get(key))
        .and_then(|key| state.pending.get(key));
    if let Some(pending) = pending {
        if let Some(name) = text(item, "name").filter(|name| *name != pending.dispatcher) {
            if pending.resolved.as_ref().is_some_and(|(wire, arguments)| {
                name == wire && text(item, "arguments") == Some(arguments.as_str())
            }) {
                return Ok(());
            }
            return Err(upstream("Grok namespace call changed after completion"));
        }
        if item.get("name").is_none() {
            item["name"] = json!(pending.dispatcher);
        }
        if text(item, "arguments").is_none_or(str::is_empty) {
            item["arguments"] = json!(pending.arguments);
        }
    }
    if let Some(group) = text(item, "name").and_then(|name| state.groups.get(name)) {
        let (wire, arguments) = unwrap(group, text(item, "arguments").unwrap_or(""))?;
        if pending
            .and_then(|pending| pending.resolved.as_ref())
            .is_some_and(|resolved| resolved != &(wire.clone(), arguments.clone()))
        {
            return Err(upstream("Grok namespace call changed after completion"));
        }
        item["name"] = json!(wire);
        item["arguments"] = json!(arguments);
    }
    Ok(())
}
fn keys(event: &Value) -> Vec<String> {
    let mut keys = Vec::new();
    for value in [event.pointer("/item/id"), event.get("item_id")] {
        if let Some(id) = value.and_then(Value::as_str) {
            keys.push(format!("id:{id}"));
        }
    }
    if let Some(id) = event
        .pointer("/item/call_id")
        .or_else(|| event.get("call_id"))
        .and_then(Value::as_str)
    {
        keys.push(format!("call:{id}"));
    }
    if let Some(index) = event.get("output_index").and_then(Value::as_u64) {
        keys.push(format!("index:{index}"));
    }
    keys
}
fn emit(
    state: &mut State,
    key: &str,
    terminal: Option<&Value>,
) -> Result<Vec<Value>, ServiceError> {
    let pending = state.pending.get(key).unwrap();
    let raw = terminal
        .and_then(|event| event.pointer("/item/arguments"))
        .and_then(Value::as_str)
        .filter(|raw| !raw.is_empty())
        .unwrap_or(&pending.arguments);
    let raw = raw.to_owned();
    let (wire, arguments) = unwrap(&state.groups[&pending.dispatcher], &raw)?;
    if pending
        .resolved
        .as_ref()
        .is_some_and(|resolved| resolved != &(wire.clone(), arguments.clone()))
    {
        return Err(upstream("Grok namespace call changed after completion"));
    }
    let emitted = pending.emitted;
    let added = pending.added.clone().or_else(|| terminal.cloned());
    let pending = state.pending.get_mut(key).unwrap();
    pending.arguments = raw;
    pending.resolved = Some((wire.clone(), arguments.clone()));
    let Some(mut added) = added else {
        return Ok(vec![]);
    };
    if emitted {
        return Ok(vec![]);
    }
    added["type"] = json!("response.output_item.added");
    added["item"]["name"] = json!(wire);
    added["item"]["arguments"] = json!("");
    added["item"]["status"] = json!("in_progress");
    let mut delta = json!({"type":"response.function_call_arguments.delta","delta":arguments});
    let mut done = json!({"type":"response.function_call_arguments.done","arguments":arguments});
    for event in [&mut delta, &mut done] {
        if let Some(id) = added.pointer("/item/id") {
            event["item_id"] = id.clone();
        }
        if let Some(index) = added.get("output_index") {
            event["output_index"] = index.clone();
        }
    }
    state.pending.get_mut(key).unwrap().emitted = true;
    Ok(vec![added, delta, done])
}

pub(super) fn restore(context: &mut Context, mut event: Value) -> Result<Vec<Value>, ServiceError> {
    if context.dispatchers.groups.is_empty() {
        return Ok(vec![event]);
    }
    let kind = text(&event, "type").unwrap_or("").to_owned();
    let dispatcher = event
        .pointer("/item/name")
        .or_else(|| event.get("name"))
        .and_then(Value::as_str)
        .filter(|name| context.dispatchers.groups.contains_key(*name))
        .map(str::to_owned);
    let aliases = keys(&event);
    let known = aliases
        .iter()
        .find_map(|key| context.dispatchers.aliases.get(key).cloned());
    let mut output = Vec::new();
    if dispatcher.is_some() || known.is_some() {
        let key = known
            .or_else(|| aliases.first().cloned())
            .ok_or_else(|| upstream("Grok namespace call requires an item identity"))?;
        if context.dispatchers.pending.len() >= MAX_BINDINGS
            && !context.dispatchers.pending.contains_key(&key)
        {
            return Err(upstream("Too many streamed Grok namespace calls"));
        }
        if context.dispatchers.aliases.len() + aliases.len() > MAX_BINDINGS * 3 {
            return Err(upstream("Too many streamed Grok namespace identities"));
        }
        for alias in aliases {
            context.dispatchers.aliases.insert(alias, key.clone());
        }
        let pending = context.dispatchers.pending.entry(key.clone()).or_default();
        if let Some(dispatcher) = dispatcher {
            pending.dispatcher = dispatcher;
        }
        match kind.as_str() {
            "response.output_item.added" => {
                if let Some(raw) = event
                    .pointer("/item/arguments")
                    .and_then(Value::as_str)
                    .filter(|raw| !raw.is_empty())
                {
                    pending.arguments = raw.to_owned();
                }
                pending.added = Some(event);
                return Ok(vec![]);
            }
            "response.function_call_arguments.delta" => {
                let delta = text(&event, "delta")
                    .ok_or_else(|| upstream("Grok namespace argument delta must be text"))?
                    .to_owned();
                context.account(delta.len())?;
                context
                    .dispatchers
                    .pending
                    .get_mut(&key)
                    .unwrap()
                    .arguments
                    .push_str(&delta);
                return Ok(vec![]);
            }
            "response.function_call_arguments.done" => {
                if let Some(arguments) = text(&event, "arguments").filter(|raw| !raw.is_empty()) {
                    pending.arguments = arguments.into();
                }
                return emit(&mut context.dispatchers, &key, None);
            }
            "response.output_item.done" => {
                if event
                    .pointer("/item/arguments")
                    .and_then(Value::as_str)
                    .is_none_or(str::is_empty)
                {
                    event["item"]["arguments"] = json!(pending.arguments);
                }
                output.extend(emit(&mut context.dispatchers, &key, Some(&event))?);
                unwrap_item(&context.dispatchers, &mut event["item"])?;
                let pending = context.dispatchers.pending.get_mut(&key).unwrap();
                if pending.item_done {
                    return Ok(vec![]);
                }
                pending.item_done = true;
            }
            _ => {}
        }
    }
    if let Some(item) = event.get_mut("item") {
        unwrap_item(&context.dispatchers, item)?;
    }
    if let Some(response) = event.get_mut("response") {
        if let Some(items) = response.get_mut("output").and_then(Value::as_array_mut) {
            for item in items {
                unwrap_item(&context.dispatchers, item)?;
            }
        }
    }
    if let Some(items) = event.get_mut("output").and_then(Value::as_array_mut) {
        for item in items {
            unwrap_item(&context.dispatchers, item)?;
        }
    }
    output.push(event);
    Ok(output)
}

#[cfg(test)]
#[path = "grok_dispatcher_tests.rs"]
mod tests;
