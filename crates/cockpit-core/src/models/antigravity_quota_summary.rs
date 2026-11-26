use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaSummaryGroup {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
    pub windows: Vec<QuotaSummaryWindow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaSummaryWindow {
    pub id: String,
    pub period: String,
    pub percentage: i32,
}

impl QuotaSummaryGroup {
    pub(crate) fn parse(summary: &Value) -> Option<Vec<Self>> {
        let mut groups = Vec::new();
        for group in summary.get("groups")?.as_array()? {
            let mut windows = Vec::new();
            for bucket in group.get("buckets")?.as_array()? {
                let id = bucket.get("bucketId")?.as_str()?.trim();
                if id.is_empty() {
                    return None;
                }
                // Argon retired into the shared Gemini pool; its legacy buckets
                // must not resurrect an independent auto-switch group.
                if QuotaSummaryWindow::is_retired_argon(id) {
                    continue;
                }
                windows.push(QuotaSummaryWindow {
                    id: id.to_owned(),
                    period: bucket
                        .get("window")
                        .and_then(Value::as_str)
                        .or_else(|| id.ends_with("weekly").then_some("weekly"))
                        .or_else(|| id.ends_with("5h").then_some("5h"))
                        .unwrap_or_default()
                        .to_owned(),
                    percentage: (bucket.get("remainingFraction")?.as_f64()? * 100.0) as i32,
                });
            }
            let Some(id) = windows.iter().map(QuotaSummaryWindow::group_key).min() else {
                continue;
            };
            groups.push(Self {
                id: id.to_owned(),
                name: group
                    .get("displayName")
                    .and_then(Value::as_str)
                    .unwrap_or(id)
                    .to_owned(),
                models: group
                    .get("description")
                    .and_then(Value::as_str)
                    .and_then(|description| {
                        description.trim().strip_prefix("Models within this group:")
                    })
                    .map(|models| {
                        models
                            .split(',')
                            .map(str::trim)
                            .filter(|model| !model.is_empty())
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default(),
                windows,
            });
        }
        Some(groups)
    }

    /// Drop retired Argon pool groups and windows from previously persisted
    /// summaries so legacy data matches fresh snapshots (filtering only, no
    /// full migration). Other official groups and custom groupings are kept.
    pub fn without_retired_argon(groups: Vec<Self>) -> Vec<Self> {
        groups
            .into_iter()
            .filter(|group| {
                !matches!(group.id.as_str(), "argon" | "gemini-argon" | "gemini-argon-wtu")
            })
            .filter_map(|mut group| {
                group
                    .windows
                    .retain(|window| !QuotaSummaryWindow::is_retired_argon(&window.id));
                (!group.windows.is_empty()).then_some(group)
            })
            .collect()
    }
}

impl QuotaSummaryWindow {
    fn group_key(&self) -> &str {
        // The quota response identifies windows, but carries no separate group ID.
        let key = self
            .id
            .strip_suffix(self.period.as_str())
            .map(|id| id.trim_end_matches([':', '-']))
            .filter(|id| !id.is_empty())
            .unwrap_or(&self.id);
        match key {
            "3p" => "claude",
            "gemini-carbon-wtu" | "gemini-carbon" => "carbon",
            _ => key,
        }
    }

    fn is_retired_argon(id: &str) -> bool {
        let id = id.trim().to_ascii_lowercase();
        matches!(id.as_str(), "argon:weekly" | "argon:5h")
            || (id.starts_with("gemini-argon-") && (id.ends_with("-weekly") || id.ends_with("-5h")))
    }
}
