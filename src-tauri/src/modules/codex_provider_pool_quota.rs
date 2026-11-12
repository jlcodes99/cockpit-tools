use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

use crate::models::codex::CodexAccount;
use crate::modules::codex_account;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderPoolQuota {
    pub(crate) name: String,
    remaining: Option<f64>,
    total: Option<f64>,
    unit: Option<String>,
    unlimited: bool,
    status: ProviderPoolQuotaStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ProviderPoolQuotaStatus {
    Ready,
    Stale,
    Failed,
    Unavailable,
}

fn finite(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.trim().parse().ok())
        .filter(|number: &f64| number.is_finite())
}

fn text(value: &Value) -> Option<&str> {
    value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub(crate) fn scoped_rows(
    scope: &[String],
    accounts: &[CodexAccount],
    now: i64,
    hide_relay_balance: bool,
) -> Vec<ProviderPoolQuota> {
    let accounts: HashMap<_, _> = accounts
        .iter()
        .map(|account| (account.id.as_str(), account))
        .collect();
    let mut seen = HashSet::new();
    scope
        .iter()
        .filter(|id| seen.insert(id.as_str()))
        .filter_map(|id| project(accounts.get(id.as_str())?, now, hide_relay_balance))
        .collect()
}

fn project(
    account: &CodexAccount,
    now: i64,
    hide_relay_balance: bool,
) -> Option<ProviderPoolQuota> {
    if !account.is_api_key_auth() || codex_account::is_chatgpt_plan_account(account) {
        return None;
    }
    let raw = account
        .quota
        .as_ref()
        .and_then(|quota| quota.raw_data.as_ref())
        .unwrap_or(&Value::Null);
    let summary = &raw["provider_usage"];
    let mode = text(&summary["mode"]);
    let legacy = raw["object"] == "codex_cockpit_api_quota";
    if hide_relay_balance
        && !matches!(
            mode,
            Some("cockpit" | "deepseek" | "token_plan" | "openai_official")
        )
    {
        return None;
    }

    let (remaining, total, unit, unlimited) = if summary.is_object() {
        let quota_remaining = finite(&summary["quotaRemaining"]);
        (
            quota_remaining
                .or_else(|| finite(&summary["balance"]))
                .or_else(|| finite(&summary["remaining"])),
            quota_remaining.and_then(|_| finite(&summary["quotaLimit"])),
            text(&summary["unit"]),
            summary["quotaUnlimited"] == true,
        )
    } else if legacy {
        let profile = &raw["profile"];
        let usage = raw
            .get("usage")
            .filter(|value| value.is_object())
            .unwrap_or(&profile["usage"]);
        let display = usage
            .get("display")
            .filter(|value| value.is_object())
            .unwrap_or(&profile["display"]);
        let unit = text(&display["unit"]);
        let unlimited = usage["unlimited_quota"] == true;
        let (remaining, total) = if unit.is_some() {
            (finite(&display["remaining"]), finite(&display["total"]))
        } else if unlimited {
            (None, None)
        } else {
            (
                finite(&usage["total_available"]),
                finite(&usage["total_granted"]),
            )
        };
        (remaining, total, unit, unlimited)
    } else {
        (None, None, None, false)
    };
    let status = if account.quota_error.is_some()
        || raw["provider_usage_query_failed"] == true
        || summary["isValid"] == false
    {
        ProviderPoolQuotaStatus::Failed
    } else if remaining.is_none() && !unlimited {
        ProviderPoolQuotaStatus::Unavailable
    } else if !account
        .usage_updated_at
        .is_some_and(|at| at > 0 && at <= now && now - at <= 900)
    {
        ProviderPoolQuotaStatus::Stale
    } else {
        ProviderPoolQuotaStatus::Ready
    };
    // The key-scoped reply contains no account IDs, credentials or endpoint URLs.
    let name = account
        .api_provider_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("API_KEY")
        .to_owned();
    Some(ProviderPoolQuota {
        name,
        remaining,
        total,
        unit: unit.map(str::to_owned),
        unlimited,
        status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::codex::{CodexAuthMode, CodexTokens};
    use serde_json::json;

    fn account(id: &str, raw: Value) -> CodexAccount {
        let mut account = CodexAccount::new(
            id.into(),
            "not-exported@example.invalid".into(),
            CodexTokens {
                access_token: String::new(),
                id_token: String::new(),
                refresh_token: None,
            },
        );
        account.auth_mode = CodexAuthMode::Apikey;
        account.openai_api_key = Some("do-not-export".into());
        account.api_provider_name = Some(id.into());
        account.api_base_url = Some("https://not-exported.example.invalid".into());
        account.usage_updated_at = Some(1_000);
        account.quota = Some(
            serde_json::from_value(json!({
                "hourly_percentage": 94, "weekly_percentage": 0, "raw_data": raw,
            }))
            .unwrap(),
        );
        account
    }

    #[test]
    fn provider_quota_projection_keeps_key_scope_units_and_status() {
        let percent = account(
            "percent",
            json!({"object":"codex_cockpit_api_quota","usage":{
                "display": {"unit":"%","remaining":94,"total":100},"total_available":940_000
            }}),
        );
        let wallet = account(
            "wallet",
            json!({"provider_usage":{
                "mode":"sub2api","balance":23,"unit":"CNY","quotaUnlimited":true
            }}),
        );
        let outside = account(
            "outside",
            json!({"provider_usage":{"balance":200,"unit":"USD"}}),
        );
        let scope = vec![
            "percent".to_string(),
            "wallet".to_string(),
            "percent".to_string(),
        ];
        let values = json!(scoped_rows(
            &scope,
            &[percent.clone(), wallet.clone(), outside],
            1001,
            false
        ));
        assert_eq!(
            values,
            json!([
                {"name":"percent","remaining":94.0,"total":100.0,"unit":"%","unlimited":false,"status":"ready"},
                {"name":"wallet","remaining":23.0,"total":null,"unit":"CNY","unlimited":true,"status":"ready"}
            ])
        );
        assert!(scoped_rows(&scope, &[percent.clone(), wallet], 1001, true).is_empty());
        assert_eq!(
            project(&percent, 2000, false).unwrap().status,
            ProviderPoolQuotaStatus::Stale
        );
        let failed = account("failed", json!({"provider_usage_query_failed":true}));
        assert_eq!(
            project(&failed, 1001, false).unwrap().status,
            ProviderPoolQuotaStatus::Failed
        );
    }
}
