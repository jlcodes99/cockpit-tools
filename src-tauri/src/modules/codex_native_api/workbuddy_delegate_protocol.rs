//! Synchronous, fail-closed protocol boundary for native WorkBuddy DelegateTool.
//! This module does not launch processes, read credentials, or execute tools.
use super::contract::ServiceError;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

const MAX_BYTES: usize = 1_048_576;
const MAX_TOOLS: usize = 128;
const MAX_DEPTH: usize = 48;
const MAX_NODES: usize = 32_768;
const MAX_SCHEMA_NODES: usize = 4_096;
const MAX_STEPS: usize = 100_000;

fn invalid(message: &'static str) -> ServiceError {
    ServiceError::new(400, "workbuddy_delegate_invalid", message)
}
fn limit() -> ServiceError {
    ServiceError::new(
        400,
        "workbuddy_delegate_limit",
        "WorkBuddy delegate protocol limit exceeded",
    )
}
fn bounded(value: &Value) -> Result<(), ServiceError> {
    fn walk(v: &Value, depth: usize, nodes: &mut usize) -> Result<(), ServiceError> {
        *nodes += 1;
        if depth > MAX_DEPTH || *nodes > MAX_NODES {
            return Err(limit());
        }
        match v {
            Value::Array(a) => {
                for x in a {
                    walk(x, depth + 1, nodes)?;
                }
            }
            Value::Object(o) => {
                for x in o.values() {
                    walk(x, depth + 1, nodes)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    walk(value, 0, &mut 0)?;
    if serde_json::to_vec(value)
        .map_err(|_| invalid("Invalid JSON value"))?
        .len()
        > MAX_BYTES
    {
        return Err(limit());
    }
    Ok(())
}
fn only(o: &Map<String, Value>, allowed: &[&str]) -> Result<(), ServiceError> {
    if o.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err(invalid("Unsupported protocol field"));
    }
    Ok(())
}
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str, ServiceError> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid("Required text field is missing"))
}
fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}
fn canonical(v: &Value) -> Value {
    match v {
        Value::Object(o) => {
            let sorted: BTreeMap<_, _> = o.iter().map(|(k, v)| (k.clone(), canonical(v))).collect();
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(a) => Value::Array(a.iter().map(canonical).collect()),
        _ => v.clone(),
    }
}
fn digest(v: &Value) -> String {
    format!("{:x}", Sha256::digest(canonical(v).to_string().as_bytes()))
}

#[derive(Clone, Debug)]
struct Tool {
    alias: String,
    name: String,
    namespace: Option<String>,
    custom: bool,
    description: String,
    schema: Value,
}
#[derive(Clone, Debug)]
pub(super) struct ToolCatalog {
    tools: BTreeMap<String, Tool>,
}
impl ToolCatalog {
    pub(super) fn from_tools(tools: Option<&Value>) -> Result<Self, ServiceError> {
        let mut catalog = Self {
            tools: BTreeMap::new(),
        };
        let Some(value) = tools else {
            return Ok(catalog);
        };
        bounded(value)?;
        let declarations = value
            .as_array()
            .ok_or_else(|| invalid("Tools must be an array"))?;
        for declaration in declarations {
            catalog.add(declaration, None, None)?;
        }
        bounded(&Value::Array(catalog.delegate_definitions()))?;
        Ok(catalog)
    }
    fn add(
        &mut self,
        declaration: &Value,
        inherited_namespace: Option<&str>,
        namespace_description: Option<&str>,
    ) -> Result<(), ServiceError> {
        let object = declaration
            .as_object()
            .ok_or_else(|| invalid("Tool declaration must be an object"))?;
        let kind = text(declaration, "type")?;
        if kind == "namespace" {
            if inherited_namespace.is_some() {
                return Err(invalid("Nested tool namespaces are unsupported"));
            }
            only(object, &["type", "name", "description", "tools"])?;
            let namespace = text(declaration, "name")?;
            if !identifier(namespace) {
                return Err(invalid("Invalid tool namespace"));
            }
            if declaration
                .get("description")
                .is_some_and(|v| v.as_str().is_none_or(|s| s.len() > 16_384))
            {
                return Err(invalid("Invalid namespace description"));
            }
            let children = declaration
                .get("tools")
                .and_then(Value::as_array)
                .ok_or_else(|| invalid("Namespace tools must be an array"))?;
            for child in children {
                self.add(
                    child,
                    Some(namespace),
                    declaration.get("description").and_then(Value::as_str),
                )?;
            }
            return Ok(());
        }
        if !matches!(kind, "function" | "custom") {
            return Err(invalid(
                "Hosted or unknown tools are not supported by WorkBuddy delegate",
            ));
        }
        only(
            object,
            &[
                "type",
                "name",
                "namespace",
                "description",
                "parameters",
                "strict",
                "defer_loading",
                "format",
            ],
        )?;
        let name = text(declaration, "name")?;
        if !identifier(name) {
            return Err(invalid("Invalid tool name"));
        }
        let declared_namespace = declaration
            .get("namespace")
            .map(|v| {
                v.as_str()
                    .filter(|s| identifier(s))
                    .ok_or_else(|| invalid("Invalid tool namespace"))
            })
            .transpose()?;
        if inherited_namespace.is_some()
            && declared_namespace.is_some()
            && inherited_namespace != declared_namespace
        {
            return Err(invalid("Conflicting tool namespaces"));
        }
        let namespace = inherited_namespace
            .or(declared_namespace)
            .map(str::to_owned);
        for field in ["strict", "defer_loading"] {
            if declaration.get(field).is_some_and(|v| !v.is_boolean()) {
                return Err(invalid("Invalid tool boolean field"));
            }
        }
        let description = match declaration.get("description") {
            Some(Value::String(s)) if s.len() <= 16_384 => s.clone(),
            None => String::new(),
            _ => return Err(invalid("Invalid tool description")),
        };
        let description = namespace_description
            .filter(|s| !s.is_empty())
            .map_or(description.clone(), |parent| {
                format!("{parent}\n\n{description}")
            });
        let custom = kind == "custom";
        let schema = if custom {
            if declaration.get("parameters").is_some() || declaration.get("strict").is_some() {
                return Err(invalid("Custom tools cannot have function parameters"));
            }
            if let Some(format) = declaration.get("format") {
                let o = format
                    .as_object()
                    .ok_or_else(|| invalid("Invalid custom tool format"))?;
                only(o, &["type"])?;
                if format["type"] != "text" {
                    return Err(invalid("Only freeform custom tools are supported"));
                }
            }
            json!({"type":"object","properties":{"input":{"type":"string"}},"required":["input"],"additionalProperties":false})
        } else {
            if declaration.get("format").is_some() {
                return Err(invalid("Function tools cannot have custom format"));
            }
            declaration
                .get("parameters")
                .cloned()
                .unwrap_or_else(|| json!({"type":"object"}))
        };
        check_schema(&schema)?;
        let alias = format!("codex_{}", &digest(&json!([kind, namespace, name]))[..32]);
        if self.tools.contains_key(&alias) {
            return Err(invalid("Duplicate tool identity"));
        }
        if self.tools.len() >= MAX_TOOLS {
            return Err(limit());
        }
        self.tools.insert(
            alias.clone(),
            Tool {
                alias,
                name: name.to_owned(),
                namespace,
                custom,
                description,
                schema,
            },
        );
        Ok(())
    }
    pub(super) fn delegate_definitions(&self) -> Vec<Value> {
        self.tools.values().map(|t| {
            let label = t.namespace.as_ref().map_or_else(|| t.name.clone(), |ns| format!("{ns}.{}", t.name));
            json!({"id":t.alias,"name":label,"description":t.description,"inputSchema":t.schema,"provider":"codex","category":"custom","requiresApproval":true})
        }).collect()
    }
    pub(super) fn fingerprint(&self) -> String {
        digest(&Value::Array(self.tools.values().map(|t| json!({"alias":t.alias,"name":t.name,"namespace":t.namespace,"custom":t.custom,"description":t.description,"schema":t.schema})).collect()))
    }
    fn checked_call<'a>(
        &'a self,
        params: &'a Value,
        permission: bool,
    ) -> Result<(&'a Tool, &'a Value), ServiceError> {
        bounded(params)?;
        let object = params
            .as_object()
            .ok_or_else(|| invalid("Delegate parameters must be an object"))?;
        if permission {
            only(object, &["toolId", "input", "timeout"])?;
        } else {
            only(
                object,
                &["sessionId", "toolCallId", "toolId", "input", "timeout"],
            )?;
            // Installed ACP emits this opaque presentation identifier in the
            // delegate callback, but not in permission rawInput. It is neither
            // the public Codex call ID nor a connection/RPC ownership proof.
            if params.get("toolCallId").is_some_and(|value| {
                value
                    .as_str()
                    .is_none_or(|id| id.is_empty() || id.len() > 256)
            }) {
                return Err(invalid("Invalid delegate tool-call presentation ID"));
            }
        }
        if params.get("sessionId").is_some_and(|v| !v.is_string()) {
            return Err(invalid("Invalid delegate session ID"));
        }
        if params
            .get("timeout")
            .is_some_and(|v| v.as_u64().is_none_or(|n| n == 0 || n > 3_600_000))
        {
            return Err(invalid("Invalid delegate timeout"));
        }
        let alias = text(params, "toolId")?;
        let tool = self
            .tools
            .get(alias)
            .ok_or_else(|| invalid("Undeclared delegate tool"))?;
        let input = params
            .get("input")
            .ok_or_else(|| invalid("Delegate input is missing"))?;
        if !input.is_object() {
            return Err(invalid("Delegate input must be an object"));
        }
        validate_schema(&tool.schema, input)?;
        Ok((tool, input))
    }
    pub(super) fn call_item(
        &self,
        params: &Value,
        public_call_id: &str,
    ) -> Result<Value, ServiceError> {
        if !identifier(public_call_id) {
            return Err(invalid("Invalid public call ID"));
        }
        let (tool, input) = self.checked_call(params, false)?;
        let prefix = if tool.custom { "ctc" } else { "fc" };
        let mut item = json!({"id":format!("{prefix}_{}", &digest(&json!([public_call_id,tool.alias]))[..32]),"type":if tool.custom {"custom_tool_call"} else {"function_call"},"status":"completed","call_id":public_call_id,"name":tool.name});
        if let Some(namespace) = &tool.namespace {
            item["namespace"] = json!(namespace);
        }
        if tool.custom {
            item["input"] = input["input"].clone();
        } else {
            item["arguments"] =
                json!(serde_json::to_string(input).map_err(|_| invalid("Invalid function input"))?);
        }
        Ok(item)
    }
    pub(super) fn validate_permission(&self, params: &Value) -> bool {
        if bounded(params).is_err() {
            return false;
        }
        let call = &params["toolCall"];
        call["_meta"]["codebuddy.ai/toolName"] == "DelegateTool"
            && call
                .get("rawInput")
                .is_some_and(|input| self.checked_call(input, true).is_ok())
    }
    pub(super) fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

// Exact decimal comparisons avoid f64 rounding accepting out-of-schema values.
#[derive(Clone, Debug)]
struct Decimal {
    negative: bool,
    digits: String,
    exponent: i32,
}
impl Decimal {
    fn parse(v: &Value) -> Result<Self, ServiceError> {
        let n = v
            .as_number()
            .ok_or_else(|| invalid("Schema bound must be a number"))?
            .to_string();
        if n.len() > 256 {
            return Err(limit());
        }
        let negative = n.starts_with('-');
        let unsigned = n.trim_start_matches('-');
        let (mantissa, exp) = match unsigned.split_once(['e', 'E']) {
            Some((m, e)) => (m, e.parse::<i32>().map_err(|_| limit())?),
            None => (unsigned, 0),
        };
        if !(-4096..=4096).contains(&exp) {
            return Err(limit());
        }
        let fraction = mantissa.split_once('.').map_or(0, |(_, f)| f.len());
        let mut digits = mantissa.replace('.', "").trim_start_matches('0').to_owned();
        let mut exponent = exp - fraction as i32;
        if digits.is_empty() {
            return Ok(Self {
                negative: false,
                digits: "0".into(),
                exponent: 0,
            });
        }
        while digits.ends_with('0') {
            digits.pop();
            exponent += 1;
        }
        Ok(Self {
            negative,
            digits,
            exponent,
        })
    }
    fn cmp(&self, other: &Self) -> Ordering {
        if self.digits == "0" && other.digits == "0" {
            return Ordering::Equal;
        }
        if self.negative != other.negative {
            return if self.negative {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
        let magnitude = if self.digits == "0" {
            Ordering::Less
        } else if other.digits == "0" {
            Ordering::Greater
        } else {
            let left = self.digits.len() as i32 + self.exponent;
            let right = other.digits.len() as i32 + other.exponent;
            left.cmp(&right).then_with(|| {
                for i in 0..self.digits.len().max(other.digits.len()) {
                    let a = self.digits.as_bytes().get(i).copied().unwrap_or(b'0');
                    let b = other.digits.as_bytes().get(i).copied().unwrap_or(b'0');
                    if a != b {
                        return a.cmp(&b);
                    }
                }
                Ordering::Equal
            })
        };
        if self.negative {
            magnitude.reverse()
        } else {
            magnitude
        }
    }
}
fn same_json(a: &Value, b: &Value) -> Result<bool, ServiceError> {
    Ok(match (a, b) {
        (Value::Number(_), Value::Number(_)) => {
            Decimal::parse(a)?.cmp(&Decimal::parse(b)?) == Ordering::Equal
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                false
            } else {
                let mut equal = true;
                for (a, b) in a.iter().zip(b) {
                    equal &= same_json(a, b)?;
                }
                equal
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            if a.len() != b.len() {
                false
            } else {
                let mut equal = true;
                for (key, a) in a {
                    equal &= if let Some(b) = b.get(key) {
                        same_json(a, b)?
                    } else {
                        false
                    };
                }
                equal
            }
        }
        _ => a == b,
    })
}
fn reference<'a>(root: &'a Value, value: &Value) -> Result<&'a Value, ServiceError> {
    let reference = value
        .as_str()
        .ok_or_else(|| invalid("Schema reference must be text"))?;
    if reference == "#" {
        return Ok(root);
    }
    if !reference.starts_with("#/") {
        return Err(invalid(
            "Only local JSON pointer schema references are supported",
        ));
    }
    let target = root
        .pointer(&reference[1..])
        .ok_or_else(|| invalid("Unresolved local schema reference"))?;
    if !target.is_object() && !target.is_boolean() {
        return Err(invalid("Reference does not target a schema"));
    }
    Ok(target)
}
fn schema_count(value: &Value) -> Result<u64, ServiceError> {
    value
        .as_u64()
        .filter(|n| *n <= MAX_BYTES as u64)
        .ok_or_else(|| invalid("Invalid schema size bound"))
}
fn check_schema(root: &Value) -> Result<(), ServiceError> {
    bounded(root)?;
    fn check(
        root: &Value,
        value: &Value,
        depth: usize,
        nodes: &mut usize,
        references: &mut BTreeSet<String>,
    ) -> Result<(), ServiceError> {
        *nodes += 1;
        if depth > MAX_DEPTH || *nodes > MAX_SCHEMA_NODES {
            return Err(limit());
        }
        if value.is_boolean() {
            return Ok(());
        }
        let object = value
            .as_object()
            .ok_or_else(|| invalid("Schema must be an object or boolean"))?;
        for (key, value) in object {
            match key.as_str() {
                "$ref" => {
                    let target = reference(root, value)?;
                    if references.insert(value.as_str().unwrap().to_owned()) {
                        check(root, target, depth + 1, nodes, references)?;
                    }
                }
                "$defs" | "definitions" | "properties" => {
                    let schemas = value
                        .as_object()
                        .ok_or_else(|| invalid("Schema map must be an object"))?;
                    for child in schemas.values() {
                        check(root, child, depth + 1, nodes, references)?;
                    }
                }
                "additionalProperties" | "items" | "not" => {
                    check(root, value, depth + 1, nodes, references)?
                }
                "anyOf" | "oneOf" | "allOf" => {
                    let schemas = value
                        .as_array()
                        .filter(|a| !a.is_empty())
                        .ok_or_else(|| invalid("Schema alternatives must be a nonempty array"))?;
                    for child in schemas {
                        check(root, child, depth + 1, nodes, references)?;
                    }
                }
                "type" => {
                    let types: Vec<&str> = match value {
                        Value::String(s) => vec![s.as_str()],
                        Value::Array(a) if !a.is_empty() => a
                            .iter()
                            .map(|v| v.as_str().ok_or_else(|| invalid("Invalid schema type")))
                            .collect::<Result<_, _>>()?,
                        _ => return Err(invalid("Invalid schema type")),
                    };
                    let mut seen = BTreeSet::new();
                    for t in types {
                        if !matches!(
                            t,
                            "object"
                                | "array"
                                | "string"
                                | "integer"
                                | "number"
                                | "boolean"
                                | "null"
                        ) || !seen.insert(t)
                        {
                            return Err(invalid("Unknown or duplicate schema type"));
                        }
                    }
                }
                "required" => {
                    let required = value
                        .as_array()
                        .ok_or_else(|| invalid("Schema required must be an array"))?;
                    let mut seen = BTreeSet::new();
                    for field in required {
                        let field = field
                            .as_str()
                            .ok_or_else(|| invalid("Required property must be text"))?;
                        if !seen.insert(field) {
                            return Err(invalid("Duplicate required property"));
                        }
                    }
                }
                "enum" => {
                    if value.as_array().is_none_or(|a| a.is_empty()) {
                        return Err(invalid("Schema enum must be nonempty"));
                    }
                }
                "const" | "default" | "examples" => {}
                "minLength" | "maxLength" | "minItems" | "maxItems" | "minProperties"
                | "maxProperties" => {
                    schema_count(value)?;
                }
                "minimum" | "maximum" | "exclusiveMinimum" | "exclusiveMaximum" => {
                    Decimal::parse(value)?;
                }
                "multipleOf" => {
                    let divisor = Decimal::parse(value)?;
                    if divisor.negative || divisor.digits == "0" || divisor.digits.len() > 18 {
                        return Err(invalid("Unsupported or nonpositive schema multipleOf"));
                    }
                }
                "uniqueItems" | "deprecated" | "readOnly" | "writeOnly" => {
                    if !value.is_boolean() {
                        return Err(invalid("Schema annotation must be boolean"));
                    }
                }
                "title" | "description" | "$comment" => {
                    if !value.is_string() {
                        return Err(invalid("Schema annotation must be text"));
                    }
                }
                "$schema" => {
                    if !matches!(
                        value.as_str(),
                        Some(
                            "https://json-schema.org/draft/2020-12/schema"
                                | "https://json-schema.org/draft/2019-09/schema"
                                | "http://json-schema.org/draft-07/schema#"
                        )
                    ) {
                        return Err(invalid("Unsupported schema dialect"));
                    }
                }
                _ => {
                    return Err(invalid(
                        "Unsupported JSON Schema keyword; validation cannot be bypassed",
                    ))
                }
            }
        }
        Ok(())
    }
    check(root, root, 0, &mut 0, &mut BTreeSet::from(["#".to_owned()]))
}
fn validate_schema(root: &Value, input: &Value) -> Result<(), ServiceError> {
    bounded(input)?;
    fn validate(
        root: &Value,
        schema: &Value,
        value: &Value,
        depth: usize,
        steps: &mut usize,
    ) -> Result<(), ServiceError> {
        *steps += 1;
        if depth > MAX_DEPTH || *steps > MAX_STEPS {
            return Err(limit());
        }
        let mismatch = || invalid("Delegate input does not match its declared schema");
        if schema == &Value::Bool(true) {
            return Ok(());
        }
        if schema == &Value::Bool(false) {
            return Err(mismatch());
        }
        if let Some(reference_value) = schema.get("$ref") {
            validate(
                root,
                reference(root, reference_value)?,
                value,
                depth + 1,
                steps,
            )?;
        }
        if let Some(kind) = schema.get("type") {
            let types: Vec<&str> = if let Some(s) = kind.as_str() {
                vec![s]
            } else {
                kind.as_array()
                    .unwrap()
                    .iter()
                    .filter_map(Value::as_str)
                    .collect()
            };
            let integer = if value.is_number() {
                Decimal::parse(value)?.exponent >= 0
            } else {
                false
            };
            if !types.iter().any(|kind| match *kind {
                "object" => value.is_object(),
                "array" => value.is_array(),
                "string" => value.is_string(),
                "integer" => integer,
                "number" => value.is_number(),
                "boolean" => value.is_boolean(),
                "null" => value.is_null(),
                _ => false,
            }) {
                return Err(mismatch());
            }
        }
        if let Some(constant) = schema.get("const") {
            if !same_json(constant, value)? {
                return Err(mismatch());
            }
        }
        if let Some(alternatives) = schema.get("enum").and_then(Value::as_array) {
            let mut found = false;
            for alternative in alternatives {
                found |= same_json(alternative, value)?;
            }
            if !found {
                return Err(mismatch());
            }
        }
        for keyword in ["anyOf", "oneOf", "allOf"] {
            if let Some(alternatives) = schema.get(keyword).and_then(Value::as_array) {
                let mut matches = 0;
                for alternative in alternatives {
                    match validate(root, alternative, value, depth + 1, steps) {
                        Ok(()) => matches += 1,
                        Err(e) if e.code == "workbuddy_delegate_limit" => return Err(e),
                        Err(_) => {}
                    }
                }
                if (keyword == "anyOf" && matches == 0)
                    || (keyword == "oneOf" && matches != 1)
                    || (keyword == "allOf" && matches != alternatives.len())
                {
                    return Err(mismatch());
                }
            }
        }
        if let Some(negated) = schema.get("not") {
            match validate(root, negated, value, depth + 1, steps) {
                Ok(()) => return Err(mismatch()),
                Err(e) if e.code == "workbuddy_delegate_limit" => return Err(e),
                Err(_) => {}
            }
        }
        if let Some(object) = value.as_object() {
            for (keyword, exceeds) in [("minProperties", false), ("maxProperties", true)] {
                if let Some(bound) = schema.get(keyword) {
                    let n = schema_count(bound)? as usize;
                    if if exceeds {
                        object.len() > n
                    } else {
                        object.len() < n
                    } {
                        return Err(mismatch());
                    }
                }
            }
            if let Some(required) = schema.get("required").and_then(Value::as_array) {
                for key in required {
                    if !object.contains_key(key.as_str().unwrap()) {
                        return Err(mismatch());
                    }
                }
            }
            let properties = schema.get("properties").and_then(Value::as_object);
            for (key, child) in object {
                if let Some(child_schema) = properties.and_then(|p| p.get(key)) {
                    validate(root, child_schema, child, depth + 1, steps)?;
                } else if let Some(additional) = schema.get("additionalProperties") {
                    validate(root, additional, child, depth + 1, steps)?;
                }
            }
        }
        if let Some(array) = value.as_array() {
            for (keyword, exceeds) in [("minItems", false), ("maxItems", true)] {
                if let Some(bound) = schema.get(keyword) {
                    let n = schema_count(bound)? as usize;
                    if if exceeds {
                        array.len() > n
                    } else {
                        array.len() < n
                    } {
                        return Err(mismatch());
                    }
                }
            }
            if schema.get("uniqueItems") == Some(&Value::Bool(true)) {
                if array.len() > 512 {
                    return Err(limit());
                }
                for i in 0..array.len() {
                    for other in &array[..i] {
                        *steps += 1;
                        if *steps > MAX_STEPS {
                            return Err(limit());
                        }
                        if same_json(&array[i], other)? {
                            return Err(mismatch());
                        }
                    }
                }
            }
            if let Some(items) = schema.get("items") {
                for child in array {
                    validate(root, items, child, depth + 1, steps)?;
                }
            }
        }
        if let Some(string) = value.as_str() {
            for (keyword, exceeds) in [("minLength", false), ("maxLength", true)] {
                if let Some(bound) = schema.get(keyword) {
                    let n = schema_count(bound)? as usize;
                    if if exceeds {
                        string.chars().count() > n
                    } else {
                        string.chars().count() < n
                    } {
                        return Err(mismatch());
                    }
                }
            }
        }
        if value.is_number() {
            let number = Decimal::parse(value)?;
            for keyword in ["minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum"] {
                if let Some(bound) = schema.get(keyword) {
                    let comparison = number.cmp(&Decimal::parse(bound)?);
                    if match keyword {
                        "minimum" => comparison == Ordering::Less,
                        "maximum" => comparison == Ordering::Greater,
                        "exclusiveMinimum" => comparison != Ordering::Greater,
                        _ => comparison != Ordering::Less,
                    } {
                        return Err(mismatch());
                    }
                }
            }
            if let Some(divisor) = schema.get("multipleOf") {
                let divisor = Decimal::parse(divisor)?;
                if number.digits != "0" {
                    if number.exponent < divisor.exponent {
                        return Err(mismatch());
                    }
                    let denominator = divisor.digits.parse::<u64>().map_err(|_| limit())?;
                    let mut remainder = 0_u128;
                    for digit in number.digits.bytes() {
                        *steps += 1;
                        if *steps > MAX_STEPS {
                            return Err(limit());
                        }
                        remainder =
                            (remainder * 10 + u128::from(digit - b'0')) % u128::from(denominator);
                    }
                    for _ in 0..number.exponent - divisor.exponent {
                        if remainder == 0 {
                            break;
                        }
                        *steps += 1;
                        if *steps > MAX_STEPS {
                            return Err(limit());
                        }
                        remainder = remainder * 10 % u128::from(denominator);
                    }
                    if remainder != 0 {
                        return Err(mismatch());
                    }
                }
            }
        }
        Ok(())
    }
    validate(root, root, input, 0, &mut 0)
}

fn message_text(item: &Value) -> Result<Vec<&str>, ServiceError> {
    let content = item
        .get("content")
        .ok_or_else(|| invalid("Message content is missing"))?;
    if let Some(text) = content.as_str() {
        return Ok(vec![text]);
    }
    let parts = content
        .as_array()
        .ok_or_else(|| invalid("Only text message content is supported"))?;
    let mut texts = Vec::new();
    for part in parts {
        if !matches!(
            part.get("type").and_then(Value::as_str),
            Some("input_text" | "output_text" | "text")
        ) {
            return Err(invalid(
                "Images and unknown message content are unsupported",
            ));
        }
        texts.push(
            part.get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("Text content must be a string"))?,
        );
    }
    Ok(texts)
}
fn check_item(item: &Value) -> Result<(), ServiceError> {
    if !item.is_object() {
        return Err(invalid("Input item must be an object"));
    }
    match item.get("type").and_then(Value::as_str) {
        None | Some("message") => {
            if !matches!(
                item.get("role").and_then(Value::as_str),
                Some("user" | "assistant" | "system" | "developer")
            ) {
                return Err(invalid("Unknown message role"));
            }
            message_text(item)?;
        }
        Some("function_call" | "custom_tool_call") => {
            text(item, "call_id")?;
            text(item, "name")?;
            let field = if item["type"] == "function_call" {
                "arguments"
            } else {
                "input"
            };
            if !item.get(field).is_some_and(Value::is_string) {
                return Err(invalid("Malformed historical tool call"));
            }
            if item.get("namespace").is_some_and(|v| !v.is_string()) {
                return Err(invalid("Invalid historical namespace"));
            }
        }
        Some("function_call_output" | "custom_tool_call_output") => {
            text(item, "call_id")?;
            if item.get("output").is_none() {
                return Err(invalid("Tool output is missing"));
            }
        }
        Some("reasoning") => {} // Opaque history only; never turned into a new prompt or executable call.
        _ => return Err(invalid("Unknown or unsupported Responses input item")),
    }
    Ok(())
}
pub(super) fn input_items(body: &Value) -> Result<Vec<Value>, ServiceError> {
    bounded(body)?;
    let input = body
        .get("input")
        .ok_or_else(|| invalid("Responses input is required"))?;
    if let Some(text) = input.as_str() {
        return Ok(vec![
            json!({"type":"message","role":"user","content":[{"type":"input_text","text":text}]}),
        ]);
    }
    let items = input
        .as_array()
        .ok_or_else(|| invalid("Responses input must be text or an array"))?;
    for item in items {
        check_item(item)?;
    }
    Ok(items.clone())
}
pub(super) fn tool_results(items: &[Value]) -> Result<Vec<(String, Value)>, ServiceError> {
    bounded(&Value::Array(items.to_vec()))?;
    let mut results = Vec::new();
    for item in items {
        check_item(item)?;
        if matches!(
            item.get("type").and_then(Value::as_str),
            Some("function_call_output" | "custom_tool_call_output")
        ) {
            results.push((text(item, "call_id")?.to_owned(), item["output"].clone()));
        }
    }
    Ok(results)
}
pub(super) fn user_prompt(items: &[Value]) -> Result<String, ServiceError> {
    bounded(&Value::Array(items.to_vec()))?;
    let mut parts = Vec::new();
    for item in items {
        check_item(item)?;
        if item["role"] == "user" {
            parts.extend(message_text(item)?);
        }
    }
    Ok(parts.join("\n"))
}

/// Render only a newly reconciled turn delta, not an entire conversation.
///
/// Native ACP accepts one text prompt rather than separate instruction roles.
/// Explicit role and UTF-8 byte-length frames preserve instruction boundaries,
/// every original text byte and multipart order without treating leading slash
/// text as a native command. The runtime still supplies its outer plain-letter
/// header. Assistant, reasoning and tool items are opaque history/results and
/// must never be promoted into a fresh user or instruction message.
pub(super) fn turn_prompt(items: &[Value]) -> Result<String, ServiceError> {
    bounded(&Value::Array(items.to_vec()))?;
    let mut prompt = String::new();
    for (message_index, item) in items.iter().enumerate() {
        check_item(item)?;
        if !matches!(
            item.get("type").and_then(Value::as_str),
            None | Some("message")
        ) {
            continue;
        }
        let Some(role @ ("user" | "system" | "developer")) =
            item.get("role").and_then(Value::as_str)
        else {
            continue;
        };
        for (part_index, text) in message_text(item)?.iter().enumerate() {
            prompt.push_str(&format!("Message begin role {role} message {message_index} part {part_index} UTF8 bytes {}\n", text.len()));
            prompt.push_str(text);
            prompt.push_str("\nMessage end\n");
            if prompt.len() > MAX_BYTES {
                return Err(limit());
            }
        }
    }
    Ok(prompt)
}

pub(super) fn response_document(response_id: &str, model: &str, mut items: Vec<Value>) -> Value {
    for (index, item) in items.iter_mut().enumerate() {
        if let Some(object) = item.as_object_mut() {
            if !object.contains_key("id") {
                let prefix = match object.get("type").and_then(Value::as_str) {
                    Some("function_call") => "fc",
                    Some("custom_tool_call") => "ctc",
                    _ => "msg",
                };
                object.insert(
                    "id".into(),
                    json!(format!(
                        "{prefix}_{}",
                        &digest(&json!([response_id, index]))[..32]
                    )),
                );
            }
            if object.get("type").and_then(Value::as_str) == Some("message") {
                if let Some(content) = object.get_mut("content").and_then(Value::as_array_mut) {
                    for part in content {
                        if let Some(part) = part.as_object_mut() {
                            if part.get("type").and_then(Value::as_str) == Some("output_text") {
                                part.entry("annotations").or_insert(json!([]));
                            }
                        }
                    }
                }
            }
            object.entry("status").or_insert(json!("completed"));
        }
    }
    json!({"id":response_id,"object":"response","created_at":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs(),"status":"completed","model":model,"output":items})
}
pub(super) fn response_events(document: &Value) -> Result<String, ServiceError> {
    bounded(document)?;
    text(document, "id")?;
    text(document, "model")?;
    if document["object"] != "response" || document["status"] != "completed" {
        return Err(invalid("Invalid completed response document"));
    }
    let items = document
        .get("output")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("Response output must be an array"))?;
    let mut events = Vec::new();
    let mut created = document.clone();
    created["status"] = json!("in_progress");
    created["output"] = json!([]);
    created.as_object_mut().unwrap().remove("usage");
    events.push(json!({"type":"response.created","response":created}));
    let mut ids = BTreeSet::new();
    let mut call_ids = BTreeSet::new();
    for (index, item) in items.iter().enumerate() {
        let id = text(item, "id")?;
        if !ids.insert(id) || item["status"] != "completed" {
            return Err(invalid("Duplicate or incomplete response item"));
        }
        let mut added = item.clone();
        added["status"] = json!("in_progress");
        match item.get("type").and_then(Value::as_str) {
            Some("function_call" | "custom_tool_call") => {
                let call_id = text(item, "call_id")?;
                text(item, "name")?;
                if !call_ids.insert(call_id) {
                    return Err(invalid("Duplicate response call ID"));
                }
                let custom = item["type"] == "custom_tool_call";
                let field = if custom { "input" } else { "arguments" };
                let content = item
                    .get(field)
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid("Tool call payload must be text"))?;
                if !custom && !serde_json::from_str::<Value>(content).is_ok_and(|v| v.is_object()) {
                    return Err(invalid("Function arguments must encode an object"));
                }
                added[field] = json!("");
                events.push(
                    json!({"type":"response.output_item.added","output_index":index,"item":added}),
                );
                let prefix = if custom {
                    "response.custom_tool_call_input"
                } else {
                    "response.function_call_arguments"
                };
                events.push(json!({"type":format!("{prefix}.delta"),"item_id":id,"output_index":index,"delta":content}));
                let mut done =
                    json!({"type":format!("{prefix}.done"),"item_id":id,"output_index":index});
                done[field] = json!(content);
                events.push(done);
            }
            Some("message") => {
                if item["role"] != "assistant" {
                    return Err(invalid("Only assistant output messages are supported"));
                }
                let parts = item
                    .get("content")
                    .and_then(Value::as_array)
                    .ok_or_else(|| invalid("Output message content must be an array"))?;
                added["content"] = json!([]);
                events.push(
                    json!({"type":"response.output_item.added","output_index":index,"item":added}),
                );
                for (content_index, part) in parts.iter().enumerate() {
                    if part["type"] != "output_text" {
                        return Err(invalid("Only text output is supported"));
                    }
                    let text = part
                        .get("text")
                        .and_then(Value::as_str)
                        .ok_or_else(|| invalid("Output text must be a string"))?;
                    let mut empty = part.clone();
                    empty["text"] = json!("");
                    events.push(json!({"type":"response.content_part.added","item_id":id,"output_index":index,"content_index":content_index,"part":empty}));
                    events.push(json!({"type":"response.output_text.delta","item_id":id,"output_index":index,"content_index":content_index,"delta":text}));
                    events.push(json!({"type":"response.output_text.done","item_id":id,"output_index":index,"content_index":content_index,"text":text}));
                    events.push(json!({"type":"response.content_part.done","item_id":id,"output_index":index,"content_index":content_index,"part":part}));
                }
            }
            _ => return Err(invalid("Unsupported output item")),
        }
        events.push(json!({"type":"response.output_item.done","output_index":index,"item":item}));
    }
    events.push(json!({"type":"response.completed","response":document}));
    let mut output = String::new();
    for (sequence, mut event) in events.into_iter().enumerate() {
        event["sequence_number"] = json!(sequence);
        output.push_str("data: ");
        output.push_str(&event.to_string());
        output.push_str("\n\n");
        if output.len() > MAX_BYTES * 4 {
            return Err(limit());
        }
    }
    Ok(output)
}

#[cfg(test)]
#[path = "workbuddy_delegate_protocol_tests.rs"]
mod tests;
