use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AntigravityDesktopLayout {
    KnowledgeWork,
    #[default]
    SoftwareEngineering,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AntigravityDesktopPermission {
    RequestReview,
    Default,
    #[default]
    Auto,
    Turbo,
}
