// 赞助商线路漂移修复。
//
// 远端公告中的 `integration.baseUrl` 是当前线路，`baseUrlAliases` 是历史线路。
// 当本地已保存的供应商/账号地址命中历史线路时，自动改写为当前线路。

use crate::modules::announcement::Sponsor;
use crate::modules::{account, atomic_write, claude_account, codex_account, logger};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

const SPONSOR_ROUTE_SYNC_STATE_FILE: &str = "sponsor_route_sync_state.json";
const CODEX_MODEL_PROVIDERS_FILE: &str = "codex_model_providers.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SponsorRouteRule {
    pub(crate) id: String,
    pub(crate) base_url: String,
    pub(crate) aliases: Vec<String>,
}

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SponsorRouteSyncSummary {
    pub changed: bool,
    pub codex_providers: usize,
    pub codex_accounts: usize,
    pub claude_accounts: usize,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SponsorRouteStateEntry {
    base_url: String,
    #[serde(default)]
    aliases: Vec<String>,
}

type SponsorRouteState = BTreeMap<String, SponsorRouteStateEntry>;

pub(crate) fn normalize_route_url(raw: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(raw.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let host = parsed.host_str()?;
    let port = parsed
        .port()
        .map(|value| format!(":{}", value))
        .unwrap_or_default();
    let path = parsed.path().trim_end_matches('/');
    Some(format!("{}://{}{}{}", parsed.scheme(), host, port, path).to_ascii_lowercase())
}

pub(crate) fn resolve_sponsor_base_url(
    current: Option<&str>,
    rules: &[SponsorRouteRule],
) -> Option<String> {
    let current = normalize_route_url(current?)?;
    rules
        .iter()
        .find(|rule| rule.aliases.iter().any(|alias| alias == &current))
        .map(|rule| rule.base_url.clone())
}

fn normalized_aliases(aliases: &[String]) -> Vec<String> {
    let mut aliases: Vec<String> = aliases
        .iter()
        .filter_map(|alias| normalize_route_url(alias))
        .collect();
    aliases.sort();
    aliases.dedup();
    aliases
}

fn build_sponsor_route_state(sponsors: &[Sponsor]) -> SponsorRouteState {
    let mut state = SponsorRouteState::new();
    for sponsor in sponsors {
        let Some(integration) = sponsor.integration.as_ref() else {
            continue;
        };
        if !integration.enabled {
            continue;
        }
        let Some(base_url) = normalize_route_url(&integration.base_url) else {
            continue;
        };
        let aliases = normalized_aliases(&integration.base_url_aliases)
            .into_iter()
            .filter(|alias| alias != &base_url)
            .collect();
        state.insert(
            sponsor.id.clone(),
            SponsorRouteStateEntry { base_url, aliases },
        );
    }
    state
}

fn effective_aliases(
    declared_aliases: &[String],
    previous_base_url: Option<&str>,
    current_base_url: &str,
) -> Vec<String> {
    let mut aliases = declared_aliases.to_vec();
    if let Some(previous_base_url) = previous_base_url {
        aliases.push(previous_base_url.to_string());
    }
    aliases = normalized_aliases(&aliases);
    aliases.retain(|alias| alias != current_base_url);
    aliases
}

fn build_sponsor_route_rules(
    state: &SponsorRouteState,
    previous_state: &SponsorRouteState,
) -> Vec<SponsorRouteRule> {
    let mut rules = Vec::new();
    for (id, entry) in state {
        let aliases = effective_aliases(
            &entry.aliases,
            previous_state.get(id).map(|item| item.base_url.as_str()),
            &entry.base_url,
        );
        if aliases.is_empty() {
            continue;
        }
        rules.push(SponsorRouteRule {
            id: id.clone(),
            base_url: entry.base_url.clone(),
            aliases,
        });
    }
    rules
}

fn sync_state_path() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join(SPONSOR_ROUTE_SYNC_STATE_FILE))
}

fn rewrite_codex_model_provider_base_urls(
    value: &mut Value,
    rules: &[SponsorRouteRule],
) -> usize {
    let Some(providers) = value.as_array_mut() else {
        return 0;
    };
    let mut changed = 0;
    for provider in providers {
        let Some(item) = provider.as_object_mut() else {
            continue;
        };
        let current = item
            .get("baseUrl")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        let Some(next) = resolve_sponsor_base_url(current.as_deref(), rules) else {
            continue;
        };
        item.insert("baseUrl".to_string(), Value::String(next));
        changed += 1;
    }
    changed
}

fn sync_codex_model_provider_routes(rules: &[SponsorRouteRule]) -> Result<usize, String> {
    let path = account::get_data_dir()?.join(CODEX_MODEL_PROVIDERS_FILE);
    if !path.exists() {
        return Ok(0);
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|error| format!("读取 Codex 模型供应商失败 ({}): {}", path.display(), error))?;
    let mut value: Value = serde_json::from_str(&content)
        .map_err(|error| format!("解析 Codex 模型供应商失败 ({}): {}", path.display(), error))?;
    let changed = rewrite_codex_model_provider_base_urls(&mut value, rules);
    if changed > 0 {
        let content = serde_json::to_string_pretty(&value)
            .map_err(|error| format!("序列化 Codex 模型供应商失败: {}", error))?;
        atomic_write::write_string_atomic(&path, &content)?;
    }
    Ok(changed)
}

pub fn sync_sponsor_routes(sponsors: &[Sponsor]) -> Result<SponsorRouteSyncSummary, String> {
    let state = build_sponsor_route_state(sponsors);
    if state.is_empty() {
        return Ok(SponsorRouteSyncSummary::default());
    }
    let state_path = sync_state_path()?;
    let previous_state: SponsorRouteState = std::fs::read_to_string(&state_path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default();
    if previous_state == state {
        return Ok(SponsorRouteSyncSummary::default());
    }

    let rules = build_sponsor_route_rules(&state, &previous_state);
    let mut codex_providers = 0;
    let mut codex_accounts = 0;
    let mut claude_accounts = 0;
    if !rules.is_empty() {
        codex_providers = sync_codex_model_provider_routes(&rules)?;
        codex_accounts = codex_account::sync_sponsor_base_urls(&rules)?;
        claude_accounts = claude_account::sync_sponsor_base_urls(&rules)?;
    }

    let content = serde_json::to_string_pretty(&state)
        .map_err(|error| format!("序列化赞助商线路同步状态失败: {}", error))?;
    atomic_write::write_string_atomic(&state_path, &content)?;

    let summary = SponsorRouteSyncSummary {
        changed: codex_providers + codex_accounts + claude_accounts > 0,
        codex_providers,
        codex_accounts,
        claude_accounts,
    };
    if summary.changed {
        logger::log_info(&format!(
            "[SponsorRouteSync] 已更新线路: codex_providers={}, codex_accounts={}, claude_accounts={}",
            summary.codex_providers, summary.codex_accounts, summary.claude_accounts
        ));
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::{
        effective_aliases, normalize_route_url, resolve_sponsor_base_url,
        rewrite_codex_model_provider_base_urls, SponsorRouteRule,
    };
    use serde_json::json;

    fn test_rules() -> Vec<SponsorRouteRule> {
        vec![SponsorRouteRule {
            id: "apikey-fun".to_string(),
            base_url: "https://api.apikey.fan/v1".to_string(),
            aliases: vec![
                "https://api.apikey.fun/v1".to_string(),
                "https://slb.apikey.fun".to_string(),
            ],
        }]
    }

    #[test]
    fn normalizes_route_urls_for_alias_matching() {
        assert_eq!(
            normalize_route_url("HTTPS://API.APIKEY.FUN/v1/"),
            Some("https://api.apikey.fun/v1".to_string())
        );
        assert_eq!(normalize_route_url("not-a-url"), None);
    }

    #[test]
    fn resolves_aliases_to_the_current_sponsor_route() {
        let rules = test_rules();
        assert_eq!(
            resolve_sponsor_base_url(Some("https://api.apikey.fun/v1/"), &rules),
            Some("https://api.apikey.fan/v1".to_string())
        );
        assert_eq!(
            resolve_sponsor_base_url(Some("https://relay.example.com/v1"), &rules),
            None
        );
    }

    #[test]
    fn previous_remote_route_becomes_an_alias_after_a_route_change() {
        let aliases = effective_aliases(
            &["https://api.apikey.fun/v1".to_string()],
            Some("https://api.apikey.fan/v1"),
            "https://new.apikey.fan/v1",
        );

        assert!(aliases.contains(&"https://api.apikey.fun/v1".to_string()));
        assert!(aliases.contains(&"https://api.apikey.fan/v1".to_string()));
        assert!(!aliases.contains(&"https://new.apikey.fan/v1".to_string()));
    }

    #[test]
    fn rewrites_codex_model_provider_alias_routes_only() {
        let rules = test_rules();
        let mut value = json!([
            { "id": "legacy", "baseUrl": "https://api.apikey.fun/v1" },
            { "id": "current", "baseUrl": "https://api.apikey.fan/v1" },
            { "id": "custom", "baseUrl": "https://relay.example.com/v1" }
        ]);

        assert_eq!(rewrite_codex_model_provider_base_urls(&mut value, &rules), 1);
        assert_eq!(value[0]["baseUrl"], "https://api.apikey.fan/v1");
        assert_eq!(value[1]["baseUrl"], "https://api.apikey.fan/v1");
        assert_eq!(value[2]["baseUrl"], "https://relay.example.com/v1");
    }
}
