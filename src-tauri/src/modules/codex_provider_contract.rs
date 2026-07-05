use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexCatalogToolProfile {
    ProxyChat,
    NativeResponses,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexProviderWireApi {
    Responses,
    ChatCompletions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexCatalogModelEntry {
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_parallel_tool_calls: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_modalities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_instructions: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexStructuredModelCatalog {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<CodexCatalogToolProfile>,
    #[serde(default)]
    pub models: Vec<CodexCatalogModelEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexCommonConfigSnippet {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub format: String,
    pub content: String,
    #[serde(default)]
    pub managed_by_cockpit: bool,
}

impl CodexCatalogModelEntry {
    pub fn from_model_id(model: impl Into<String>) -> Option<Self> {
        let model = model.into().trim().to_string();
        if model.is_empty() {
            return None;
        }

        Some(Self {
            model,
            display_name: None,
            context_window: None,
            supports_parallel_tool_calls: None,
            input_modalities: Vec::new(),
            base_instructions: None,
        })
    }
}
pub fn catalog_entries_from_model_ids<I, S>(models: I) -> Vec<CodexCatalogModelEntry>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    models
        .into_iter()
        .filter_map(CodexCatalogModelEntry::from_model_id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_entries_from_model_ids_trims_and_drops_empty_values() {
        let entries = catalog_entries_from_model_ids([" gpt-5.5 ", "", "mimo-v1"]);

        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.model.as_str())
                .collect::<Vec<_>>(),
            vec!["gpt-5.5", "mimo-v1"]
        );
    }

    #[test]
    fn structured_catalog_serializes_with_snake_case_profile() {
        let catalog = CodexStructuredModelCatalog {
            profile: Some(CodexCatalogToolProfile::NativeResponses),
            models: catalog_entries_from_model_ids(["mimo-v1"]),
        };

        let value = serde_json::to_value(catalog).expect("serialize catalog");

        assert_eq!(value["profile"], "native_responses");
        assert_eq!(value["models"][0]["model"], "mimo-v1");
    }
}
