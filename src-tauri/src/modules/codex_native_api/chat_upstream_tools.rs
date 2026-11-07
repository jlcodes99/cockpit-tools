//! Request-scoped identities for the Go-compatible Chat function bridge.
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
struct Declaration {
    name: String,
    namespace: String,
    chat_name: String,
    custom: bool,
    tool: Value,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ToolContext {
    declarations: Vec<Declaration>,
    indices: BTreeMap<String, usize>,
    local_names: BTreeMap<String, Option<String>>,
    wire_names: BTreeMap<String, String>,
}

pub(super) fn qualify(namespace: &str, name: &str) -> String {
    let name = name.trim();
    if namespace.is_empty()
        || name.is_empty()
        || name.starts_with("mcp__")
        || name.starts_with(namespace)
    {
        name.into()
    } else if namespace.ends_with("__") {
        format!("{namespace}{name}")
    } else {
        format!("{namespace}__{name}")
    }
}

impl ToolContext {
    pub(crate) fn for_anthropic(body: &Value) -> Self {
        use sha2::{Digest, Sha256};
        let mut context = Self::from_responses(body);
        let mut reserved = context.indices.keys().cloned().collect::<std::collections::BTreeSet<_>>();
        for declaration in &mut context.declarations {
            if declaration.custom {
                let description = declaration.tool["description"].as_str().unwrap_or("").replace("This is a FREEFORM tool, so do not wrap the patch in JSON.","");
                let mut description = format!("{description}\n\nPass the complete freeform tool input as the input string property. Keep its contents unchanged.");
                if declaration.name=="apply_patch" {
                    description.push_str("\nUse the Codex apply_patch format. Start with *** Begin Patch and end with *** End Patch. Use *** Add File: path, *** Delete File: path, or *** Update File: path. Every added-file content line starts with +. For updates, use @@; context lines start with one space, removed lines with -, and added lines with +. Use *** Move to: path for a rename and *** End of File when required by the patch grammar.");
                }
                if let Some(format) = declaration.tool.get("format") {
                    description.push_str(&format!("\n\nFreeform input format:\n{format}"));
                }
                declaration.tool["description"] = json!(description);
            }
            let original = declaration.chat_name.clone();
            if original.len() <= 64 && original.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-') { continue; }
            let digest = format!("{:x}", Sha256::digest(original.as_bytes()));
            let base = format!("tool_{}", &digest[..32]);
            let mut name = base.clone();
            let mut suffix = 0;
            while reserved.contains(&name) { suffix += 1; name = format!("{base}_{suffix}"); }
            reserved.insert(name.clone());
            context.wire_names.insert(original, name.clone());
            declaration.chat_name = name;
        }
        context.indices = context.declarations.iter().enumerate().map(|(index, d)|(d.chat_name.clone(), index)).collect();
        for name in context.local_names.values_mut().flatten() {
            if let Some(wire) = context.wire_names.get(name) { *name = wire.clone(); }
        }
        context
    }
    pub(crate) fn from_responses(body: &Value) -> Self {
        let mut result = Self::default();
        result.scan(&body["tools"]);
        for item in body["input"].as_array().into_iter().flatten() {
            if item["type"] == "additional_tools" {
                result.scan(&item["tools"]);
            }
        }
        result
    }
    fn scan(&mut self, tools: &Value) {
        for tool in tools.as_array().into_iter().flatten() {
            if tool["type"] == "namespace" {
                let namespace = tool["name"].as_str().unwrap_or("").trim();
                for child in tool["tools"].as_array().into_iter().flatten() {
                    self.add(child, namespace);
                }
            } else {
                self.add(tool, "");
            }
        }
    }
    fn add(&mut self, tool: &Value, namespace: &str) {
        let kind = tool["type"].as_str().unwrap_or("").trim();
        if !matches!(kind, "" | "function" | "custom") {
            return;
        }
        let name = tool["name"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| tool.pointer("/function/name").and_then(Value::as_str))
            .unwrap_or("")
            .trim();
        if name.is_empty() {
            return;
        }
        let chat_name = qualify(namespace, name);
        if self.indices.contains_key(&chat_name) {
            return;
        }
        self.indices
            .insert(chat_name.clone(), self.declarations.len());
        self.local_names
            .entry(name.into())
            .and_modify(|value| *value = None)
            .or_insert_with(|| Some(chat_name.clone()));
        self.declarations.push(Declaration {
            name: name.into(),
            namespace: namespace.into(),
            chat_name,
            custom: kind == "custom",
            tool: tool.clone(),
        });
    }
    pub(crate) fn canonical(&self, name: &str, namespace: Option<&str>) -> String {
        if let Some(namespace) = namespace.filter(|s| !s.is_empty()) {
            let qualified = qualify(namespace, name);
            return self.wire_names.get(&qualified).cloned().unwrap_or(qualified);
        }
        if let Some(wire) = self.wire_names.get(name) { return wire.clone(); }
        if self.indices.contains_key(name) {
            return name.into();
        }
        self.local_names
            .get(name)
            .and_then(Option::as_ref)
            .cloned()
            .unwrap_or_else(|| name.into())
    }
    pub(crate) fn custom(&self, name: &str) -> bool {
        self.indices
            .get(name)
            .is_some_and(|index| self.declarations[*index].custom)
    }
    pub(crate) fn is_apply_patch(&self,name:&str)->bool {
        self.indices.get(name).is_some_and(|index|self.declarations[*index].custom&&self.declarations[*index].name=="apply_patch")
    }
    pub(super) fn pending_name(&self, name: &str) -> bool {
        !self.indices.contains_key(name)
            && self
                .indices
                .range(name.to_owned()..)
                .next()
                .is_some_and(|(candidate, _)| candidate.starts_with(name))
    }
    pub(super) fn identity(&self, name: &str) -> (String, Option<String>) {
        self.indices
            .get(name)
            .map(|index| &self.declarations[*index])
            .map(|d| {
                (
                    d.name.clone(),
                    (!d.namespace.is_empty()).then(|| d.namespace.clone()),
                )
            })
            .unwrap_or_else(|| (name.into(), None))
    }

    pub(crate) fn tools(&self, native_custom: bool) -> Vec<Value> {
        self.declarations.iter().map(|d| {
            if d.custom && native_custom {
                let mut details = d.tool.as_object().cloned().unwrap_or_default();
                details.remove("type"); details.insert("name".into(),json!(d.chat_name));
                return json!({"type":"custom","custom":details});
            }
            let mut function = json!({"name":d.chat_name,"description":d.tool.get("description").or_else(||d.tool.pointer("/function/description")).and_then(Value::as_str).unwrap_or(""),"parameters":{}});
            if d.custom { function["parameters"] = json!({"type":"object","properties":{"input":{"type":"string"}},"required":["input"]}); }
            else if let Some(parameters) = ["/parameters","/parametersJsonSchema","/input_schema","/function/parameters","/function/parametersJsonSchema"].iter().find_map(|path|d.tool.pointer(path)) { function["parameters"] = parameters.clone(); }
            // Preserve the existing Rust strict-schema extension.
            if !d.custom { if let Some(strict) = d.tool.get("strict") { function["strict"] = strict.clone(); } }
            json!({"type":"function","function":function})
        }).collect()
    }
    pub(crate) fn restore_item(&self, item: &mut Value) {
        if !matches!(
            item["type"].as_str(),
            Some("function_call" | "custom_tool_call")
        ) {
            return;
        }
        let name = item["name"].as_str().unwrap_or("").to_owned();
        let (local, namespace) = self.identity(&name);
        item["name"] = json!(local);
        if let Some(namespace) = namespace {
            item["namespace"] = json!(namespace);
        }
        if self.custom(&name) && item["type"] == "function_call" {
            let input = unwrap_input(item["arguments"].as_str().unwrap_or(""), false);
            item["type"] = json!("custom_tool_call");
            item["input"] = json!(input);
            item.as_object_mut().unwrap().remove("arguments");
        }
        super::super::agent_communication::declare_plaintext_message(item);
    }
}

/// Decode a complete wrapper or the available prefix of its JSON string. Partial
/// escapes remain buffered, so split Unicode/surrogate pairs never corrupt input.
pub(crate) fn unwrap_input(raw: &str, partial: bool) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(raw) {
        return value
            .get("input")
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| v.to_string())
            })
            .unwrap_or_else(|| raw.into());
    }
    if !partial || !raw.trim_start().starts_with('{') {
        return raw.into();
    }
    let Some((_, tail)) = raw.split_once("\"input\"") else {
        return String::new();
    };
    let Some(tail) = tail.trim_start().strip_prefix(':') else {
        return String::new();
    };
    let Some(content) = tail.trim_start().strip_prefix('"') else {
        return String::new();
    };
    let mut escaped = false;
    let end = content
        .char_indices()
        .find_map(|(index, c)| {
            if escaped {
                escaped = false;
                None
            } else if c == '\\' {
                escaped = true;
                None
            } else if c == '"' {
                Some(index)
            } else {
                None
            }
        })
        .unwrap_or(content.len());
    let prefix = &content[..end];
    for trim in 0..=12.min(prefix.len()) {
        let end = prefix.len() - trim;
        if !prefix.is_char_boundary(end) {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<String>(&format!("\"{}\"", &prefix[..end])) {
            return value;
        }
    }
    String::new()
}
