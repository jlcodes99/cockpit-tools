//! CPA's spawn-agent model list rendering; supplied catalog entries stay local.
use serde_json::Value;

const HEADING: &str = "Available model overrides (optional; inherited parent model is preferred):";
const MARKER: &str = "Spawns an agent";

#[derive(Clone, Debug, Default)]
pub struct ModelProfile {
    pub id: String,
    pub description: String,
    pub reasoning_efforts: Vec<String>,
    pub default_reasoning_effort: String,
    pub service_tiers: Vec<String>,
}

fn string(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_owned()
}

fn effort(value: &str) -> Option<String> {
    let value = value.trim().to_ascii_lowercase();
    matches!(
        value.as_str(),
        "none" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra"
    )
    .then_some(value)
}

impl ModelProfile {
    /// Same metadata keys as the official Codex model catalog and CPA registry.
    /// Filtering unavailable models and sorting remain the caller's responsibility.
    pub fn from_metadata(id: impl Into<String>, metadata: &Value) -> Self {
        let reasoning_efforts: Vec<String> = metadata
            .get("supported_reasoning_levels")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| effort(&string(entry, "effort")))
            .collect();
        let default_effort = effort(&string(metadata, "default_reasoning_level"));
        let default_reasoning_effort = default_effort
            .filter(|default| reasoning_efforts.contains(default))
            .or_else(|| reasoning_efforts.first().cloned())
            .unwrap_or_default();
        let mut service_tiers = Vec::new();
        for tier in metadata
            .get("service_tiers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let id = string(tier, "id");
            if !id.is_empty() && !service_tiers.contains(&id) {
                service_tiers.push(id);
            }
        }
        Self {
            id: id.into(),
            description: string(metadata, "description"),
            reasoning_efforts,
            default_reasoning_effort,
            service_tiers,
        }
    }

    /// CPA applies configured ThinkingSupport only to synthesized model entries.
    pub fn apply_thinking(&mut self, levels: &[String]) {
        let efforts: Vec<String> = levels.iter().filter_map(|value| effort(value)).collect();
        if efforts.is_empty() {
            return;
        }
        let mut default = "";
        for effort in &efforts {
            if (default.is_empty() && effort != "none") || effort == "medium" {
                default = effort;
            }
        }
        self.default_reasoning_effort = if default.is_empty() {
            efforts[0].clone()
        } else {
            default.to_owned()
        };
        self.reasoning_efforts = efforts;
    }
}

fn single_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn format_models(profiles: &[ModelProfile]) -> String {
    let mut lines = Vec::with_capacity(profiles.len());
    for profile in profiles {
        let id = single_line(&profile.id);
        if id.is_empty() {
            continue;
        }
        let code = if id.contains('`') {
            format!("`` {id} ``")
        } else {
            format!("`{id}`")
        };
        let mut line = format!("- {code}: ");
        let description = single_line(&profile.description);
        let mut details = Vec::new();
        if !description.is_empty() {
            let sentence = if description.ends_with(['.', '!', '?']) {
                description
            } else {
                format!("{description}.")
            };
            details.push(sentence);
        }
        if !profile.reasoning_efforts.is_empty() {
            let efforts = profile
                .reasoning_efforts
                .iter()
                .map(|effort| {
                    if effort == &profile.default_reasoning_effort {
                        format!("{effort} (default)")
                    } else {
                        effort.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            details.push(format!("Reasoning efforts: {efforts}."));
        }
        if !profile.service_tiers.is_empty() {
            details.push(format!(
                "Service tiers: {}.",
                profile.service_tiers.join(", ")
            ));
        }
        line.push_str(&details.join(" "));
        lines.push(line);
    }
    lines.join("\n")
}

pub(super) fn replace_model_sections(description: &str, model_list: &str) -> String {
    if model_list.is_empty() {
        return description.to_owned();
    }
    let mut cleaned = String::new();
    let mut indent = String::new();
    let mut lines = description.split_inclusive('\n').peekable();
    while let Some(line) = lines.next() {
        if line.trim() != HEADING {
            cleaned.push_str(line);
            continue;
        }
        if indent.is_empty() {
            if let Some(index) = line.find(HEADING).filter(|index| *index > 0) {
                indent = line[..index].to_owned();
            }
        }
        while lines
            .peek()
            .is_some_and(|line| line.trim().starts_with("- "))
        {
            lines.next();
        }
    }
    let section = format!("{indent}{HEADING}\n{model_list}\n");
    if let Some(marker) = cleaned.find(MARKER) {
        let line_start = cleaned[..marker]
            .rfind('\n')
            .map(|index| index + 1)
            .unwrap_or(0);
        cleaned.insert_str(line_start, &section);
        cleaned
    } else {
        if !cleaned.is_empty() && !cleaned.ends_with('\n') {
            cleaned.push_str("\n\n");
        }
        cleaned.push_str(section.trim_end_matches('\n'));
        cleaned
    }
}
