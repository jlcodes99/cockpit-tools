//! Claude capability snapshot from CPA 9717e325, shared with the Go registry.
use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Deserialize)]
struct Catalog {
    models: Vec<Model>,
}
#[derive(Deserialize)]
pub(super) struct Model {
    pub id: String,
    pub context_length: Option<u64>,
    pub max_completion_tokens: Option<u64>,
    pub thinking: Option<Thinking>,
}

pub(super) fn output_limit(model: &str, requested: Option<u64>) -> u64 {
    let limit = get(model).and_then(|model| model.max_completion_tokens);
    let default = if model.to_ascii_lowercase().contains("fable") {
        limit.unwrap_or(64000).min(64000)
    } else {
        limit.unwrap_or(32000)
    };
    requested
        .filter(|value| *value > 0)
        .map(|value| limit.map_or(value, |limit| value.min(limit)))
        .unwrap_or(default)
}
#[derive(Deserialize)]
pub(super) struct Thinking {
    #[serde(default)]
    pub min: u64,
    #[serde(default)]
    pub max: u64,
    #[serde(default)]
    pub zero_allowed: bool,
    pub disable_type: Option<String>,
    #[serde(default)]
    pub levels: Vec<String>,
}

pub(super) fn get(model: &str) -> Option<&'static Model> {
    static MODELS: OnceLock<Catalog> = OnceLock::new();
    let models = &MODELS
        .get_or_init(|| {
            serde_json::from_str(include_str!("anthropic_models.json"))
                .expect("valid bundled Claude capability snapshot")
        })
        .models;
    let model = model.split('[').next().unwrap_or(model);
    let dated = |id: &str, base: &str| {
        id.strip_prefix(base)
            .and_then(|tail| tail.strip_prefix('-'))
            .is_some_and(|tail| tail.len() == 8 && tail.bytes().all(|b| b.is_ascii_digit()))
    };
    models.iter().find(|entry| entry.id == model).or_else(|| {
        models
            .iter()
            .find(|entry| dated(model, &entry.id) || dated(&entry.id, model))
    })
}
