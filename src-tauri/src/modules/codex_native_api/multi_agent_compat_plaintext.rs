//! Equivalent to the host's vendored Go collaboration.PlaintextPolicy.
//! CPA namespace optimization alone is not evidence of plaintext arguments.
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Rule {
    plaintext: bool,
    wire_alias: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Policy {
    tools: HashMap<String, Rule>,
    additional: HashMap<String, Rule>,
    top_declared: bool,
    invalid_extra: bool,
    blocked: HashSet<String>,
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

fn tool_name<'a>(mut name: &'a str, mut namespace: &str) -> Option<&'a str> {
    if !matches!(namespace, "" | "collaboration" | "collaboration-optimize") {
        return None;
    }
    for prefix in [
        "collaboration__",
        "collaboration.",
        "collaboration::",
        "collaboration-optimize__",
        "collaboration-optimize.",
        "collaboration-optimize::",
    ] {
        if let Some(short) = name.strip_prefix(prefix) {
            namespace = "collaboration";
            name = short;
            break;
        }
    }
    (!namespace.is_empty() && matches!(name, "spawn_agent" | "send_message" | "followup_task"))
        .then_some(name)
}

fn alias(value: &Value, namespace: &str) -> bool {
    namespace == "collaboration-optimize"
        || text(value, "name").starts_with("collaboration-optimize")
}

fn merge(rules: &mut HashMap<String, Rule>, name: &str, mut rule: Rule) {
    if let Some(previous) = rules.get(name) {
        rule.plaintext &= previous.plaintext;
        rule.wire_alias |= previous.wire_alias;
    }
    rules.insert(name.to_owned(), rule);
}

fn scan(tools: &Value, namespace: &str, allow_alias: bool, rules: &mut HashMap<String, Rule>) {
    let Some(tools) = tools.as_array() else {
        return;
    };
    for tool in tools {
        if text(tool, "type") == "namespace" {
            if let Some(children) = tool.get("tools") {
                scan(children, text(tool, "name"), allow_alias, rules);
            }
            continue;
        }
        let Some(name) = tool_name(text(tool, "name"), namespace) else {
            continue;
        };
        let message = tool.pointer("/parameters/properties/message");
        let plain = text(tool, "type") == "function"
            && message.is_some_and(|message| {
                message.is_object()
                    && text(message, "type") == "string"
                    && message
                        .get("encrypted")
                        .is_none_or(|encrypted| encrypted == false)
            });
        let alias = alias(tool, namespace);
        merge(
            rules,
            name,
            Rule {
                plaintext: plain && (!alias || allow_alias),
                wire_alias: alias && allow_alias,
            },
        );
    }
}

impl Policy {
    pub fn can_mark(&self) -> bool {
        self.rules().values().any(|rule| rule.plaintext)
    }
    pub fn for_request(body: &Value, optimized: bool) -> Self {
        if !body.is_object() {
            return Self {
                top_declared: true,
                invalid_extra: true,
                ..Default::default()
            };
        }
        let mut policy = Self {
            top_declared: body.get("tools").is_some(),
            ..Default::default()
        };
        if let Some(tools) = body.get("tools") {
            scan(tools, "", optimized, &mut policy.tools);
        }
        if let Some(input) = body.get("input").and_then(Value::as_array) {
            for item in input {
                if text(item, "type") == "additional_tools" {
                    if let Some(tools) = item.get("tools").filter(|tools| tools.is_array()) {
                        scan(tools, "", optimized, &mut policy.additional);
                    } else {
                        policy.invalid_extra = true;
                    }
                }
            }
        }
        if policy.invalid_extra {
            policy.additional.clear();
        }
        policy
    }

    pub fn inherit(&mut self, previous: &Self) {
        self.blocked = previous.blocked.clone();
        for rules in [
            &self.tools,
            &self.additional,
            &previous.tools,
            &previous.additional,
        ] {
            for (name, rule) in rules {
                if !rule.plaintext {
                    self.blocked.insert(name.clone());
                }
            }
        }
        if !self.top_declared {
            self.tools = previous.tools.clone();
        }
        if !self.invalid_extra {
            let mut merged = previous.additional.clone();
            for (name, rule) in &self.additional {
                merge(&mut merged, name, *rule);
            }
            self.additional = merged;
        }
    }

    fn rules(&self) -> HashMap<String, Rule> {
        let mut rules = self.tools.clone();
        for (name, rule) in &self.additional {
            merge(&mut rules, name, *rule);
        }
        for name in &self.blocked {
            rules.insert(name.clone(), Rule::default());
        }
        rules
    }

    pub fn mark(&self, value: &mut Value, wire: bool) {
        let rules = self.rules();
        mark_item(value, &rules, wire);
        if matches!(
            text(value, "type"),
            "response.output_item.added" | "response.output_item.done"
        ) {
            if let Some(item) = value.get_mut("item") {
                mark_item(item, &rules, wire);
            }
        }
        for path in ["/output", "/response/output"] {
            if let Some(items) = value.pointer_mut(path).and_then(Value::as_array_mut) {
                for item in items {
                    mark_item(item, &rules, wire);
                }
            }
        }
    }
}

fn mark_item(item: &mut Value, rules: &HashMap<String, Rule>, wire: bool) {
    if text(item, "type") != "function_call" {
        return;
    }
    let namespace = text(item, "namespace");
    let alias = alias(item, namespace);
    if alias && !wire {
        return;
    }
    let Some(name) = tool_name(text(item, "name"), namespace) else {
        return;
    };
    let Some(rule) = rules.get(name) else {
        return;
    };
    if !rule.plaintext
        || (alias && !rule.wire_alias)
        || item.get("encrypted_content").is_some()
        || item.get("signature").is_some()
        || item
            .get("encrypted_function_args")
            .is_some_and(|value| !value.is_null())
    {
        return;
    }
    item["encrypted_function_args"] = json!([]);
}
