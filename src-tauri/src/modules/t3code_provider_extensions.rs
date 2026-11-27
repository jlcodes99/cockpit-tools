//! Additional T3 Code providers that can use stored Cockpit accounts.

use chrono::Utc;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::models::codebuddy::CodebuddyAccount;
use crate::models::grok::{GrokAccountView, GrokAuthMode};
use crate::models::minimax_code::MinimaxCodeAccount;
use crate::models::t3code::{T3CodeBindableAccountView, T3CodeQuotaRemaining, T3CodeQuotaWindowView};
use crate::models::workbuddy::WorkbuddyAccount;
use crate::modules::{
    atomic_write, codebuddy_account, codebuddy_cn_account, config, grok_account,
    minimax_code_account, workbuddy_account, workbuddy_ai_account,
};

use super::{
    restrict_dir, restrict_file, PersistedBindingsMap,
    PersistedProviderBinding, DRIVER_ANTIGRAVITY, DRIVER_CLAUDE, DRIVER_CODEX,
};

#[path = "t3code_qoder_provider.rs"]
mod qoder;
pub(super) use qoder::QoderProvider;

pub const PLATFORM_WORKBUDDY: &str = "workbuddy";
pub const PLATFORM_WORKBUDDY_AI: &str = "workbuddy_ai";
pub const PLATFORM_CODEBUDDY: &str = "codebuddy";
pub const PLATFORM_CODEBUDDY_CN: &str = "codebuddy_cn";
pub const PLATFORM_GROK: &str = "grok";
pub const PLATFORM_MINIMAXCODE: &str = "minimaxcode";
pub const PLATFORM_QODER: &str = "qoder";
pub const PLATFORM_QODER_CN: &str = "qoder_cn";
const T3_DRIVER_ACP_REGISTRY: &str = "acpRegistry";
const T3_DRIVER_GROK: &str = "grok";

pub const EXTENDED_ACCOUNT_PLATFORMS: &[&str] = &[
    PLATFORM_WORKBUDDY,
    PLATFORM_WORKBUDDY_AI,
    PLATFORM_CODEBUDDY,
    PLATFORM_CODEBUDDY_CN,
    PLATFORM_GROK,
    PLATFORM_MINIMAXCODE,
    PLATFORM_QODER,
    PLATFORM_QODER_CN,
];

pub fn is_extended_platform(platform: &str) -> bool {
    EXTENDED_ACCOUNT_PLATFORMS.contains(&platform)
}

pub fn t3_raw_driver_for_platform(platform: &str) -> Option<&'static str> {
    match platform {
        DRIVER_CODEX => Some(DRIVER_CODEX),
        DRIVER_ANTIGRAVITY => Some(DRIVER_ANTIGRAVITY),
        DRIVER_CLAUDE => Some(DRIVER_CLAUDE),
        PLATFORM_GROK => Some(T3_DRIVER_GROK),
        PLATFORM_WORKBUDDY
        | PLATFORM_WORKBUDDY_AI
        | PLATFORM_CODEBUDDY
        | PLATFORM_CODEBUDDY_CN
        | PLATFORM_MINIMAXCODE
        | PLATFORM_QODER
        | PLATFORM_QODER_CN => Some(T3_DRIVER_ACP_REGISTRY),
        _ => None,
    }
}

pub fn default_display_name(platform: &str) -> &'static str {
    match platform {
        DRIVER_CODEX => "Codex",
        DRIVER_ANTIGRAVITY => "Antigravity",
        DRIVER_CLAUDE => "Claude",
        PLATFORM_WORKBUDDY => "WorkBuddy",
        PLATFORM_WORKBUDDY_AI => "WorkBuddy AI",
        PLATFORM_CODEBUDDY => "CodeBuddy",
        PLATFORM_CODEBUDDY_CN => "CodeBuddy CN",
        PLATFORM_GROK => "Grok",
        PLATFORM_MINIMAXCODE => "MiniMax Code",
        PLATFORM_QODER => "Qoder",
        PLATFORM_QODER_CN => "Qoder CN",
        _ => "Provider",
    }
}

pub fn cockpit_provider_dir(state_dir: &Path, platform: &str, instance_id: &str) -> PathBuf {
    state_dir
        .join("providers")
        .join("cockpit")
        .join(platform)
        .join(instance_id)
}

fn entry_has_env_value(entry: Option<&Map<String, Value>>, key: &str, expected: &str) -> bool {
    entry
        .and_then(|obj| obj.get("environment"))
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().any(|item| {
                item.get("name").and_then(Value::as_str) == Some(key)
                    && item
                        .get("value")
                        .and_then(Value::as_str)
                        .is_some_and(|val| val.eq_ignore_ascii_case(expected))
            })
        })
}

fn extract_cockpit_platform_from_path(raw: &str) -> Option<&'static str> {
    let normalized = raw.replace('\\', "/");
    for &platform in EXTENDED_ACCOUNT_PLATFORMS {
        let needle = format!("/providers/cockpit/{platform}/");
        if normalized.contains(&needle) {
            return Some(platform);
        }
    }
    None
}

pub fn infer_platform_from_t3_entry(
    instance_id: &str,
    raw_driver: &str,
    entry: Option<&Map<String, Value>>,
    persisted: &PersistedBindingsMap,
) -> Option<String> {
    if matches!(raw_driver, DRIVER_CODEX | DRIVER_ANTIGRAVITY | DRIVER_CLAUDE) {
        return Some(raw_driver.to_string());
    }

    if let Some(saved) = persisted.get(instance_id) {
        if is_extended_platform(&saved.driver)
            && QoderProvider::from_platform(&saved.driver).is_none()
            && t3_raw_driver_for_platform(&saved.driver) == Some(raw_driver)
        {
            return Some(saved.driver.clone());
        }
    }

    match raw_driver {
        T3_DRIVER_GROK => return Some(PLATFORM_GROK.to_string()),
        T3_DRIVER_ACP_REGISTRY => {}
        _ => return None,
    }

    let config_obj = entry.and_then(|e| e.get("config")).and_then(Value::as_object);
    let source = config_obj
        .and_then(|c| c.get("source"))
        .and_then(Value::as_str)
        .unwrap_or("registry");
    let command_path = config_obj
        .and_then(|c| c.get("commandPath"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();

    if let Some(found) = extract_cockpit_platform_from_path(command_path) {
        return Some(found.to_string());
    }
    if let Some(args) = config_obj
        .and_then(|c| c.get("commandArgs"))
        .and_then(Value::as_array)
    {
        for arg in args.iter().filter_map(Value::as_str) {
            if let Some(found) = extract_cockpit_platform_from_path(arg) {
                return Some(found.to_string());
            }
        }
    }

    if source == "registry" {
        let agent_id = config_obj
            .and_then(|c| c.get("agentId"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        return match agent_id {
            "qoder" => Some(PLATFORM_QODER.to_string()),
            "minimax-code" => Some(PLATFORM_MINIMAXCODE.to_string()),
            "codebuddy-code" => {
                if entry_has_env_value(entry, "CODEBUDDY_INTERNET_ENVIRONMENT", "internal")
                    || instance_id.starts_with("codebuddy_cn")
                {
                    Some(PLATFORM_CODEBUDDY_CN.to_string())
                } else {
                    Some(PLATFORM_CODEBUDDY.to_string())
                }
            }
            _ => None,
        };
    }

    let lower_cmd = command_path.replace('\\', "/").to_ascii_lowercase();
    let file_stem = Path::new(lower_cmd.rsplit('/').next().unwrap_or(""))
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if lower_cmd.contains("workbuddy ai.app/") || file_stem == "workbuddyai" {
        return Some(PLATFORM_WORKBUDDY_AI.to_string());
    }
    if lower_cmd.contains("workbuddy.app/") || file_stem == "workbuddy" {
        return Some(PLATFORM_WORKBUDDY.to_string());
    }
    match file_stem.as_str() {
        "qoder" | "qodercli" | "qodercn" | "qoderclicn"
            if config_obj
                .and_then(|config| config.get("commandArgs"))
                .and_then(Value::as_array)
                .is_some_and(|args| args.iter().any(|arg| arg.as_str() == Some("--acp"))) =>
        {
            Some(match file_stem.as_str() {
                "qodercn" | "qoderclicn" => PLATFORM_QODER_CN,
                _ => PLATFORM_QODER,
            }.to_string())
        }
        "mcode" => Some(PLATFORM_MINIMAXCODE.to_string()),
        "codebuddy" => {
            if entry_has_env_value(entry, "CODEBUDDY_INTERNET_ENVIRONMENT", "internal")
                || instance_id.starts_with("codebuddy_cn")
            {
                Some(PLATFORM_CODEBUDDY_CN.to_string())
            } else {
                Some(PLATFORM_CODEBUDDY.to_string())
            }
        }
        _ => None,
    }
}

fn json_number(value: Option<&Value>) -> Option<f64> {
    let val = value?;
    if let Some(n) = val.as_f64() {
        return n.is_finite().then_some(n);
    }
    if let Some(s) = val.as_str() {
        if let Ok(n) = s.trim().replace(',', "").parse::<f64>() {
            return n.is_finite().then_some(n);
        }
    }
    None
}

fn quota_resource_root<'a>(
    quota: Option<&'a Value>,
    usage: Option<&'a Value>,
) -> Option<&'a Value> {
    quota
        .and_then(|raw| raw.get("userResource"))
        .or(usage)
        .or(quota)
}

fn resource_accounts(root: &Value) -> Option<&Vec<Value>> {
    [
        "/data/Response/Data/Accounts",
        "/Response/Data/Accounts",
        "/data/data/Response/Data/Accounts",
        "/data/ResponseData/Accounts",
        "/ResponseData/Accounts",
        "/data/accounts",
        "/accounts",
        "/list",
        "/data/resources",
        "/data/data/resources",
        "/resources",
    ]
    .iter()
    .find_map(|path| {
        root.pointer(path)?
            .as_array()
            .filter(|rows| !rows.is_empty())
    })
}

fn active_credit_resource(item: &Value) -> bool {
    json_number(item.get("Status").or_else(|| item.get("status")))
        .is_none_or(|status| status == 0.0 || status == 3.0)
}

fn codebuddy_resource_remaining(item: &Value) -> Option<f64> {
    let number = |keys: &[&str]| keys.iter().find_map(|key| json_number(item.get(*key)));
    if number(&["CapacityType"]) == Some(4.0) {
        if let Some(slice) = item.pointer("/SlicePeriodUsageDetails/0") {
            return json_number(slice.get("SlicePeriodCapacityRemainPrecise"));
        }
    }
    number(&[
        "CycleCapacityRemainPrecise",
        "CycleCapacityRemain",
        "CapacityRemainPrecise",
        "CapacityRemain",
        "CycleRemainSize",
        "cycleRemainSize",
        "RemainSize",
        "CycleRemainCapacity",
        "remaining",
        "remain",
    ])
    .or_else(|| {
        let total = number(&[
            "CycleCapacitySizePrecise",
            "CycleCapacitySize",
            "CapacitySizePrecise",
            "CapacitySize",
            "CycleTotalSize",
            "cycleTotalSize",
            "TotalSize",
            "CycleTotalCapacity",
            "total",
            "limit_num",
            "limitNum",
        ])?;
        let used = number(&[
            "CycleCapacityUsedPrecise",
            "CycleUsedCapacity",
            "UsedSize",
            "used",
            "used_num",
            "usedNum",
            "credit",
        ])?;
        (total >= 0.0).then_some((total - used).max(0.0))
    })
}

fn extract_codebuddy_suite_windows(
    quota_raw: Option<&Value>,
    usage_raw: Option<&Value>,
) -> Vec<T3CodeQuotaWindowView> {
    let Some(root) = quota_resource_root(quota_raw, usage_raw) else {
        return Vec::new();
    };
    let entries = root
        .pointer("/data/resourceSummary/Packages")
        .and_then(Value::as_array)
        .filter(|items| {
            !items.is_empty()
                && items
                    .iter()
                    .all(|item| json_number(item.get("CycleRemainCapacity")).is_some())
        })
        .or_else(|| resource_accounts(root))
        .map(Vec::as_slice)
        .unwrap_or_else(|| std::slice::from_ref(root.get("data").unwrap_or(root)));

    let mut reported = false;
    let mut amount = 0.0;
    for item in entries.iter().filter(|item| active_credit_resource(item)) {
        if let Some(remaining) = codebuddy_resource_remaining(item).filter(|value| *value >= 0.0) {
            reported = true;
            amount += remaining;
        }
    }
    if !reported || !amount.is_finite() {
        return Vec::new();
    }
    vec![credit_window(amount)]
}

fn credit_window(remaining_amount: f64) -> T3CodeQuotaWindowView {
    T3CodeQuotaWindowView {
        id: "credits".to_string(),
        label: "Credits".to_string(),
        remaining: T3CodeQuotaRemaining::Amount { remaining_amount },
        reset_time: None,
    }
}

fn reported_plan(raw: Option<&str>) -> Option<&str> {
    raw.map(str::trim)
        .filter(|label| !label.is_empty() && !label.eq_ignore_ascii_case("unknown"))
}

fn credit_suite_plan<'a>(
    stored: Option<&'a str>,
    profile: Option<&'a Value>,
    root: Option<&'a Value>,
    international: bool,
) -> Option<&'a str> {
    let profile_field = |key| profile?.get(key)?.as_str();
    if let Some(kind @ ("ultimate" | "exclusive" | "premise")) = profile_field("type") {
        return Some(match (international, kind) {
            (true, "ultimate") => "Team",
            (true, "exclusive") => "Enterprise Plan",
            _ => "ENTERPRISE",
        });
    }
    let summary = root.and_then(|value| value.pointer("/data/resourceSummary"));
    if let Some(name) = reported_plan(
        summary
            .and_then(|value| value.get("SubscriptionPackageName"))
            .and_then(Value::as_str),
    ) {
        return Some(name);
    }
    fn package_plan(code: &str) -> Option<(u8, &'static str)> {
        Some(match code {
            "TCACA_code_043_6oIkxT7Z0S" => (8, "Buddy One VIP"),
            "TCACA_code_027_0FCGVA6vSa" => (7, "Ultimate"),
            "TCACA_code_026_BaESVICNoi" => (6, "Advanced"),
            "TCACA_code_023_4xbGhMrE6q" => (5, "Youth"),
            "TCACA_code_002_AkiJS3ZHF5"
            | "TCACA_code_003_FAnt7lcmRT"
            | "TCACA_code_005_maRGyrHhw1" => (4, "PRO"),
            "TCACA_code_039_KRcQj7wUat" | "TCACA_code_040_mi9rCYg46x" => (3, "Pro-Trial"),
            "TCACA_code_006_DbXS0lrypC" => (2, "TRIAL"),
            "TCACA_code_001_PqouKr6QWV"
            | "TCACA_code_008_cfWoLwvjU4"
            | "TCACA_code_035_ArVxJcGDsm" => (1, "FREE"),
            _ => return None,
        })
    }
    let package_code = summary
        .and_then(|value| value.get("SubscriptionPackageCode"))
        .and_then(Value::as_str);
    if let Some((_, label)) = package_code.and_then(package_plan) {
        return Some(label);
    }
    root.and_then(resource_accounts)
        .into_iter()
        .flatten()
        .chain(
            summary
                .and_then(|value| value.get("Packages"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten(),
        )
        .filter(|item| active_credit_resource(item))
        .filter_map(|item| package_plan(item.get("PackageCode")?.as_str()?))
        .max_by_key(|(rank, _)| *rank)
        .map(|(_, label)| label)
        .or_else(|| reported_plan(stored))
        .or_else(|| reported_plan(profile_field("editionType")))
        .or_else(|| {
            (summary?.get("IsPaidUser")?.as_bool() == Some(false) && package_code.is_none())
                .then_some("FREE")
        })
}

fn build_workbuddy_family_view(
    platform: &str,
    acc: WorkbuddyAccount,
) -> Option<T3CodeBindableAccountView> {
    if acc.access_token.trim().is_empty() {
        return None;
    }
    let windows = extract_codebuddy_suite_windows(acc.quota_raw.as_ref(), acc.usage_raw.as_ref());
    let plan_badge = credit_suite_plan(
        reported_plan(acc.plan_type.as_deref())
            .or_else(|| reported_plan(acc.payment_type.as_deref())),
        acc.profile_raw.as_ref(),
        quota_resource_root(acc.quota_raw.as_ref(), acc.usage_raw.as_ref()),
        platform == PLATFORM_WORKBUDDY_AI,
    )
    .unwrap_or("UNKNOWN")
    .to_owned();
    let display_name = acc
        .nickname
        .clone()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| acc.enterprise_name.clone().filter(|s| !s.trim().is_empty()));
    Some(T3CodeBindableAccountView {
        id: acc.id,
        driver: platform.to_string(),
        email: acc.email,
        display_name,
        plan_badge,
        plan_badge_kind: None,
        auth_mode: "oauth".to_string(),
        is_api_service: false,
        workspace_id: acc.enterprise_id,
        windows,
        quota_summary_groups: None,
        provider_quota: None,
        primary_percent: None,
        secondary_percent: None,
        speed_tok_per_sec: None,
        cache_hit_percent: None,
    })
}

fn build_codebuddy_family_view(
    platform: &str,
    acc: CodebuddyAccount,
) -> Option<T3CodeBindableAccountView> {
    if acc.access_token.trim().is_empty() {
        return None;
    }
    let windows = extract_codebuddy_suite_windows(acc.quota_raw.as_ref(), acc.usage_raw.as_ref());
    let plan_badge = credit_suite_plan(
        reported_plan(acc.plan_type.as_deref())
            .or_else(|| reported_plan(acc.payment_type.as_deref())),
        acc.profile_raw.as_ref(),
        quota_resource_root(acc.quota_raw.as_ref(), acc.usage_raw.as_ref()),
        false,
    )
    .unwrap_or("UNKNOWN")
    .to_owned();
    let display_name = acc
        .nickname
        .clone()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| acc.enterprise_name.clone().filter(|s| !s.trim().is_empty()));
    Some(T3CodeBindableAccountView {
        id: acc.id,
        driver: platform.to_string(),
        email: acc.email,
        display_name,
        plan_badge,
        plan_badge_kind: None,
        auth_mode: "oauth".to_string(),
        is_api_service: false,
        workspace_id: acc.enterprise_id,
        windows,
        quota_summary_groups: None,
        provider_quota: None,
        primary_percent: None,
        secondary_percent: None,
        speed_tok_per_sec: None,
        cache_hit_percent: None,
    })
}

fn build_grok_view(acc: GrokAccountView) -> T3CodeBindableAccountView {
    let mut windows = Vec::new();
    let mut primary = None;
    let mut secondary = None;

    if let Some(quota) = &acc.quota {
        let from_percent = |used: f64| (100.0 - used).round().clamp(0.0, 100.0) as i32;
        let mut append = |id: &str, label: &str, amount: Option<f64>, used_percent: Option<f64>| {
            if windows.len() >= 3 {
                return;
            }
            let remaining = if let Some(amount) = amount.filter(|value| value.is_finite()) {
                T3CodeQuotaRemaining::Amount {
                    remaining_amount: amount,
                }
            } else if let Some(percent) = used_percent.filter(|value| value.is_finite()) {
                let remaining_percent = from_percent(percent);
                if primary.is_none() {
                    primary = Some(remaining_percent);
                } else if secondary.is_none() {
                    secondary = Some(remaining_percent);
                }
                T3CodeQuotaRemaining::Percent { remaining_percent }
            } else {
                return;
            };
            windows.push(T3CodeQuotaWindowView {
                id: id.to_owned(),
                label: label.to_owned(),
                remaining,
                reset_time: quota.period_end.clone(),
            });
        };
        append(
            "weekly",
            "Weekly",
            quota
                .weekly_total
                .zip(quota.weekly_used)
                .map(|(total, used)| (total - used).max(0.0)),
            quota.weekly_limit_percent,
        );
        if quota.is_unified_billing_user != Some(true) {
            for product in &quota.products {
                append(
                    &product.product,
                    &product.product,
                    product.remaining.or_else(|| {
                        product
                            .total
                            .zip(product.used)
                            .map(|(total, used)| (total - used).max(0.0))
                    }),
                    product.usage_percent,
                );
            }
        }
    }

    let plan_badge = reported_plan(acc.plan_type.as_deref())
        .or_else(|| reported_plan(acc.quota.as_ref()?.subscription_tier.as_deref()))
        .unwrap_or(if acc.auth_mode == GrokAuthMode::ApiKey {
            "API_KEY"
        } else {
            "UNKNOWN"
        })
        .to_owned();

    let full_name = [acc.first_name.as_deref(), acc.last_name.as_deref()]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");

    T3CodeBindableAccountView {
        id: acc.id,
        driver: PLATFORM_GROK.to_string(),
        email: acc.email,
        display_name: (!full_name.is_empty()).then_some(full_name),
        plan_badge,
        plan_badge_kind: None,
        auth_mode: if acc.auth_mode == GrokAuthMode::ApiKey {
            "apikey".to_string()
        } else {
            "oauth".to_string()
        },
        is_api_service: false,
        workspace_id: acc.team_id,
        windows,
        quota_summary_groups: None,
        provider_quota: None,
        primary_percent: primary,
        secondary_percent: secondary,
        speed_tok_per_sec: None,
        cache_hit_percent: None,
    }
}

fn minimax_credit_amount(raw: &Value) -> Option<f64> {
    let summary = [
        "/membership/op_credit_summary/total_remaining_amount",
        "/membership/opcredit_balance",
        "/workspace/opcredit_balance",
    ]
    .iter()
    .find_map(|path| json_number(raw.pointer(path)).filter(|value| *value >= 0.0));
    if summary.is_some() {
        return summary;
    }
    let details = raw.get("userResource").unwrap_or(raw);
    let rows = details
        .get("details")
        .or_else(|| details.pointer("/data/details"))?
        .as_array()?;
    let mut amounts = rows
        .iter()
        .filter(|row| matches!(row.get("credit_type").and_then(Value::as_u64), Some(1 | 2)))
        .map(|row| json_number(row.get("remaining_amount")).filter(|value| *value >= 0.0));
    let first = amounts.next()??;
    let total = amounts.try_fold(first, |sum, amount| Some(sum + amount?))?;
    total.is_finite().then_some(total)
}

fn build_minimaxcode_view(acc: MinimaxCodeAccount) -> Option<T3CodeBindableAccountView> {
    if acc
        .refresh_token
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        return None;
    }
    let windows = acc
        .quota_raw
        .as_ref()
        .and_then(minimax_credit_amount)
        .map(credit_window)
        .into_iter()
        .collect();

    Some(T3CodeBindableAccountView {
        id: acc.id,
        driver: PLATFORM_MINIMAXCODE.to_string(),
        email: acc.email,
        display_name: acc.nickname.filter(|s| !s.trim().is_empty()),
        plan_badge: reported_plan(acc.plan_type.as_deref())
            .or_else(|| {
                ["token_plan_tier", "plan_name"].iter().find_map(|key| {
                    reported_plan(
                        acc.quota_raw
                            .as_ref()?
                            .get("membership")?
                            .get(*key)?
                            .as_str(),
                    )
                })
            })
            .unwrap_or("UNKNOWN")
            .to_owned(),
        plan_badge_kind: None,
        auth_mode: "oauth".to_string(),
        is_api_service: false,
        workspace_id: None,
        windows,
        quota_summary_groups: None,
        provider_quota: None,
        primary_percent: None,
        secondary_percent: None,
        speed_tok_per_sec: None,
        cache_hit_percent: None,
    })
}

pub fn build_all_extra_bindable_accounts() -> HashMap<String, Vec<T3CodeBindableAccountView>> {
    let mut map = HashMap::with_capacity(EXTENDED_ACCOUNT_PLATFORMS.len());

    let wb_accounts = workbuddy_account::list_accounts()
        .into_iter()
        .filter_map(|acc| build_workbuddy_family_view(PLATFORM_WORKBUDDY, acc))
        .collect::<Vec<_>>();
    map.insert(PLATFORM_WORKBUDDY.to_string(), wb_accounts);

    let wb_ai_accounts = workbuddy_ai_account::list()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|acc| build_workbuddy_family_view(PLATFORM_WORKBUDDY_AI, acc))
        .collect::<Vec<_>>();
    map.insert(PLATFORM_WORKBUDDY_AI.to_string(), wb_ai_accounts);

    let cb_accounts = codebuddy_account::list_accounts()
        .into_iter()
        .filter_map(|acc| build_codebuddy_family_view(PLATFORM_CODEBUDDY, acc))
        .collect::<Vec<_>>();
    map.insert(PLATFORM_CODEBUDDY.to_string(), cb_accounts);

    let cb_cn_accounts = codebuddy_cn_account::list_accounts()
        .into_iter()
        .filter_map(|acc| build_codebuddy_family_view(PLATFORM_CODEBUDDY_CN, acc))
        .collect::<Vec<_>>();
    map.insert(PLATFORM_CODEBUDDY_CN.to_string(), cb_cn_accounts);

    let grok_accounts = grok_account::list_accounts_checked()
        .unwrap_or_default()
        .into_iter()
        .map(build_grok_view)
        .collect::<Vec<_>>();
    map.insert(PLATFORM_GROK.to_string(), grok_accounts);

    let mm_accounts = minimax_code_account::list_accounts()
        .into_iter()
        .filter_map(build_minimaxcode_view)
        .collect::<Vec<_>>();
    map.insert(PLATFORM_MINIMAXCODE.to_string(), mm_accounts);
    for platform in [PLATFORM_QODER, PLATFORM_QODER_CN] {
        if let Some(provider) = QoderProvider::from_platform(platform) {
            map.insert(platform.to_string(), provider.list_accounts());
        }
    }

    map
}

pub fn detect_extended_bound_account(
    state_dir: &Path,
    instance_id: &str,
    platform: &str,
    entry: Option<&Map<String, Value>>,
    persisted: &PersistedBindingsMap,
    candidates: &[T3CodeBindableAccountView],
) -> (Option<T3CodeBindableAccountView>, Option<String>) {
    let meta_path = cockpit_provider_dir(state_dir, platform, instance_id).join("binding-meta.json");
    let meta = fs::read_to_string(meta_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    let disk_account_id = meta
        .as_ref()
        .and_then(|v| v.get("accountId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(provider) = QoderProvider::from_platform(platform) {
        let saved = persisted.get(instance_id);
        if saved.is_some_and(|item| item.unlinked) {
            return (None, None);
        }
        let saved_id = saved.filter(|item| item.driver == platform &&
            item.account_id != "__cli__").map(|item| item.account_id.as_str());
        let found = [saved_id, disk_account_id].into_iter().flatten().find_map(|id| {
            let account = candidates.iter().find(|account| account.id == id)?;
            provider.matches_binding(state_dir, instance_id, id, entry).then_some(account)
        });
        return match found {
            Some(account) => (Some(account.clone()), Some(account.email.clone())),
            None => (None, None),
        };
    }
    let disk_email = meta
        .as_ref()
        .and_then(|v| v.get("email"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);

    if let Some(id) = disk_account_id {
        if let Some(found) = candidates.iter().find(|c| c.id == id) {
            return (Some(found.clone()), Some(found.email.clone()));
        }
    }

    if let Some(saved) = persisted
        .get(instance_id)
        .filter(|item| item.driver == platform)
    {
        if let Some(found) = candidates.iter().find(|c| c.id == saved.account_id) {
            return (Some(found.clone()), Some(found.email.clone()));
        }
    }

    if let Some(email) = &disk_email {
        let mut iter = candidates
            .iter()
            .filter(|c| c.email.eq_ignore_ascii_case(email));
        if let Some(first) = iter.next() {
            if iter.next().is_none() {
                return (Some(first.clone()), Some(email.clone()));
            }
        }
    }

    (None, disk_email)
}

pub async fn refresh_extended_account_quota(
    platform: &str,
    account_id: &str,
) -> Result<(), String> {
    if let Some(provider) = QoderProvider::from_platform(platform) {
        return provider.refresh_quota(account_id).await;
    }
    match platform {
        PLATFORM_WORKBUDDY => workbuddy_account::refresh_account_token(account_id)
            .await
            .map(|_| ()),
        PLATFORM_WORKBUDDY_AI => {
            crate::commands::workbuddy_ai::refresh_workbuddy_ai_token(account_id.to_string())
                .await
                .map(|_| ())
        }
        PLATFORM_CODEBUDDY => codebuddy_account::refresh_account_token(account_id)
            .await
            .map(|_| ()),
        PLATFORM_CODEBUDDY_CN => codebuddy_cn_account::refresh_account_token(account_id)
            .await
            .map(|_| ()),
        PLATFORM_GROK => grok_account::refresh_account(account_id).await.map(|_| ()),
        PLATFORM_MINIMAXCODE => minimax_code_account::refresh_account_token(account_id)
            .await
            .map(|_| ()),
        _ => Err("该平台暂不支持通过 Cockpit 刷新账号额度".to_string()),
    }
}

fn candidate_search_dirs(extra_home_subdirs: &[&str]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(path_os) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path_os));
    }
    if let Some(home) = dirs::home_dir() {
        for rel in extra_home_subdirs {
            dirs.push(home.join(rel));
        }
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join("bin"));
        dirs.push(home.join(".npm-global/bin"));
    }
    #[cfg(unix)]
    {
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
        dirs.push(PathBuf::from("/usr/bin"));
    }
    #[cfg(windows)]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            dirs.push(PathBuf::from(appdata).join("npm"));
        }
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let local_path = PathBuf::from(local);
            dirs.push(local_path.join("Programs"));
            dirs.push(local_path.join("Microsoft\\WinGet\\Links"));
        }
    }
    dirs
}

fn resolve_cli_binary(
    name: &str,
    extra_home_subdirs: &[&str],
    allow_windows_cmd: bool,
) -> Option<PathBuf> {
    #[cfg(windows)]
    let exts: &[&str] = if allow_windows_cmd {
        &[".exe", ".cmd", ".bat", ""]
    } else {
        &[".exe", ""]
    };
    #[cfg(not(windows))]
    let exts: &[&str] = &[""];
    let _ = allow_windows_cmd;

    for dir in candidate_search_dirs(extra_home_subdirs) {
        for ext in exts {
            let candidate = dir.join(format!("{name}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn resolve_workbuddy_domestic_executable() -> Result<PathBuf, String> {
    let cfg_path = config::get_user_config().workbuddy_app_path;
    let trimmed = cfg_path.trim();

    #[cfg(target_os = "macos")]
    {
        let mut bundles = Vec::new();
        if !trimmed.is_empty() {
            let p = PathBuf::from(trimmed);
            if p.extension().is_some_and(|e| e == "app") {
                bundles.push(p);
            } else if let Some(bundle) = p.ancestors().nth(3).filter(|b| b.extension().is_some_and(|e| e == "app")) {
                bundles.push(bundle.to_path_buf());
            } else if p.is_file() {
                return Ok(p);
            }
        }
        bundles.push(PathBuf::from("/Applications/WorkBuddy.app"));
        if let Some(home) = dirs::home_dir() {
            bundles.push(home.join("Applications/WorkBuddy.app"));
        }
        for bundle in bundles {
            for bin_name in ["Electron", "WorkBuddy"] {
                let exe = bundle.join("Contents/MacOS").join(bin_name);
                if exe.is_file() && workbuddy_ai_account::executable_at(&bundle).is_none() {
                    return Ok(exe);
                }
            }
        }
        Err("未找到 WorkBuddy 桌面客户端，请先安装 WorkBuddy 或在设置中指定路径".to_string())
    }

    #[cfg(target_os = "windows")]
    {
        let mut candidates = Vec::new();
        if !trimmed.is_empty() {
            let p = PathBuf::from(trimmed);
            if p.is_dir() {
                candidates.push(p.join("WorkBuddy.exe"));
            } else {
                candidates.push(p);
            }
        }
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            candidates.push(PathBuf::from(local).join("Programs\\WorkBuddy\\WorkBuddy.exe"));
        }
        if let Ok(pf) = std::env::var("PROGRAMFILES") {
            candidates.push(PathBuf::from(pf).join("WorkBuddy\\WorkBuddy.exe"));
        }
        for exe in candidates {
            if exe.is_file() && workbuddy_ai_account::executable_at(&exe).is_none() {
                return Ok(exe);
            }
        }
        Err("未找到 WorkBuddy 桌面客户端，请先安装 WorkBuddy 或在设置中指定路径".to_string())
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = trimmed;
        Err("当前系统暂不支持 WorkBuddy 桌面版内置组件启动".to_string())
    }
}

fn resolve_workbuddy_ai_executable() -> Result<PathBuf, String> {
    let cfg_path = config::get_user_config().workbuddy_ai_app_path;
    let trimmed = cfg_path.trim();
    if !trimmed.is_empty() {
        if let Some(exe) = workbuddy_ai_account::executable_at(Path::new(trimmed)) {
            return Ok(exe);
        }
    }
    workbuddy_ai_account::detect_executable().ok_or_else(|| {
        "未找到 WorkBuddy AI 桌面客户端，请先安装 WorkBuddy AI 或在设置中指定路径".to_string()
    })
}

fn resolve_workbuddy_unpacked_cli(electron_exe: &Path) -> Result<PathBuf, String> {
    let exe_dir = electron_exe
        .parent()
        .ok_or_else(|| "WorkBuddy 可执行文件路径无效".to_string())?;
    let candidates = [
        exe_dir.join("../Resources/app.asar.unpacked/cli/bin/codebuddy"),
        exe_dir.join("../Resources/app.asar.unpacked/cli/dist/codebuddy-lite-wb.mjs"),
        exe_dir.join("resources/app.asar.unpacked/cli/bin/codebuddy"),
        exe_dir.join("resources/app.asar.unpacked/cli/dist/codebuddy-lite-wb.mjs"),
    ];
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| "未找到 WorkBuddy 内置 ACP CLI 组件，请升级客户端后重试".to_string())
}

fn write_private_file(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
        restrict_dir(parent);
    }
    atomic_write::write_string_atomic(path, content)?;
    restrict_file(path);
    Ok(())
}

#[cfg(unix)]
fn write_executable_script(path: &Path, content: &str) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    write_private_file(path, content)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("设置启动器执行权限失败: {e}"))
}

#[cfg(windows)]
fn resolve_node_host_for_wrapper() -> Result<(PathBuf, bool), String> {
    if let Some(node) = resolve_cli_binary("node", &[], false) {
        return Ok((node, false));
    }
    let t3_exe = crate::modules::t3code_instance::resolve_executable()
        .map_err(|_| "未找到 node.exe 或 T3 Code 主程序以承载本地启动包装器".to_string())?;
    Ok((t3_exe, true))
}

const WORKBUDDY_NODE_LAUNCHER_JS: &str = r#"const { mkdirSync, readFileSync } = require('node:fs');
const { join } = require('node:path');
const { pathToFileURL } = require('node:url');
const { StringDecoder } = require('node:string_decoder');

function useAcpStdout() {
  const write = process.stdout.write.bind(process.stdout);
  const decoder = new StringDecoder('utf8');
  let pending = '';
  process.stdout.write = (chunk, encoding, callback) => {
    const done = typeof encoding === 'function' ? encoding : callback;
    pending += decoder.write(typeof chunk === 'string'
      ? Buffer.from(chunk, typeof encoding === 'string' ? encoding : 'utf8') : chunk);
    const lines = pending.split('\n');
    pending = lines.pop();
    const frames = lines.filter((line) => {
      if (!line.trimStart().startsWith('{')) return false;
      try { return JSON.parse(line).jsonrpc === '2.0'; } catch { return false; }
    });
    if (frames.length > 0) return write(frames.join('\n') + '\n', 'utf8', done);
    if (done) process.nextTick(done);
    return true;
  };
}

for (const key of Object.keys(process.env)) {
  if (key.startsWith('ACC_PRODUCT_CONFIG_') || key.startsWith('CODEBUDDY_AUTH_')) {
    delete process.env[key];
  }
}
delete process.env.NODE_OPTIONS;
delete process.env.CODEBUDDY_API_KEY;
delete process.env.CODEBUDDY_BASE_URL;
delete process.env.CODEBUDDY_CUSTOM_HEADERS;

const secret = JSON.parse(readFileSync(join(__dirname, 'auth-secret.json'), 'utf8'));
const profile = join(__dirname, 'profile');
mkdirSync(profile, { recursive: true, mode: 0o700 });

process.env.CODEBUDDY_CONFIG_DIR = profile;
process.env.WORKBUDDY_CONFIG_DIR = profile;
process.env.WORKBUDDY_USER_DATA_DIR = join(profile, 'app');
process.env.CODEBUDDY_AUTH_TOKEN = secret.accessToken;
process.env.SERVER__HOST = '127.0.0.1';
process.env.CODEBUDDY_HOST = 'workbuddy-desktop';
process.env.CODEBUDDY_FORCE_LITE_WB_BUNDLE = '1';
process.env.CODEBUDDY_PROJECT_CONFIG_DIR = '.workbuddy';
process.env.WORKBUDDY_DATA_FOLDER_NAME = '.workbuddy';
process.env.CODEBUDDY_DISABLE_PRODUCT_CACHE = '1';
process.env.CODEBUDDY_DISABLE_SESSION_HISTORY_CLEANUP = '1';
process.env.CODEBUDDY_PLUGIN_BACKGROUND_MANAGEMENT = '0';
process.env.DISABLE_AUTOUPDATER = '1';

if (secret.domestic) {
  process.env.CODEBUDDY_INTERNET_ENVIRONMENT = 'internal';
  process.env.CODEBUDDY_COPILOT_INTERNET_ENVIRONMENT = 'internal';
} else {
  delete process.env.CODEBUDDY_INTERNET_ENVIRONMENT;
  delete process.env.CODEBUDDY_COPILOT_INTERNET_ENVIRONMENT;
}

useAcpStdout();
const cliPath = secret.cliPath;
process.argv = [
  process.execPath,
  cliPath,
  '--serve',
  '--host',
  '127.0.0.1',
  '--port',
  '0',
  '--auth',
  'password',
  '--acp',
  '--acp-transport',
  'stdio',
];

if (cliPath.endsWith('.mjs')) {
  import(pathToFileURL(cliPath).href);
} else {
  require(cliPath);
}
"#;

pub struct ExtendedBindingWriteOutcome {
    pub config_patch: Map<String, Value>,
    pub environment: Vec<Value>,
    pub binding: PersistedProviderBinding,
}

fn env_item(name: &str, value: &str) -> Value {
    json!({
        "name": name,
        "value": value,
        "sensitive": false,
    })
}

pub fn merge_managed_environment(
    entry: &mut Map<String, Value>,
    managed_vars: &[Value],
    bump_generation: bool,
) {
    let managed_names: Vec<&str> = managed_vars
        .iter()
        .filter_map(|v| v.get("name").and_then(Value::as_str))
        .collect();
    let existing = entry
        .entry("environment")
        .or_insert_with(|| json!([]))
        .as_array_mut();
    if let Some(arr) = existing {
        arr.retain(|item| {
            let Some(name) = item.get("name").and_then(Value::as_str) else {
                return false;
            };
            if bump_generation && name == "COCKPIT_PROVIDER_GENERATION" {
                return false;
            }
            !managed_names.contains(&name)
        });
        arr.extend(managed_vars.iter().cloned());
        if bump_generation {
            arr.push(env_item(
                "COCKPIT_PROVIDER_GENERATION",
                &uuid::Uuid::new_v4().to_string(),
            ));
        }
    }
}

#[cfg(unix)]
fn write_codebuddy_unix_launcher(
    wrapper: &Path,
    domestic: bool,
    cli: Option<&Path>,
) -> Result<bool, String> {
    let env_setup = if domestic {
        "export CODEBUDDY_INTERNET_ENVIRONMENT=internal\nexport CODEBUDDY_COPILOT_INTERNET_ENVIRONMENT=internal"
    } else {
        "unset CODEBUDDY_INTERNET_ENVIRONMENT CODEBUDDY_COPILOT_INTERNET_ENVIRONMENT"
    };
    let cli = cli.unwrap_or_else(|| Path::new("codebuddy"));
    let escaped = cli.to_string_lossy().replace('\'', "'\\''");
    let script = format!(
        "#!/bin/sh\nset -e\nDIR=\"$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd)\"\nCB_TOKEN=\"$(cat \"$DIR/secret.token\")\"\nexport CODEBUDDY_AUTH_TOKEN=\"$CB_TOKEN\"\nexport CODEBUDDY_CONFIG_DIR=\"$DIR/profile\"\nunset CODEBUDDY_API_KEY CODEBUDDY_BASE_URL\n{env_setup}\nexec '{escaped}' --acp \"$@\"\n"
    );
    if fs::read_to_string(wrapper).is_ok_and(|current| current == script) {
        return Ok(false);
    }
    write_executable_script(wrapper, &script)?;
    Ok(true)
}

#[cfg(windows)]
const CODEBUDDY_WINDOWS_LAUNCHER: &str = r#"const { spawn } = require('node:child_process');
const { readFileSync } = require('node:fs');
const { join } = require('node:path');
const secret = JSON.parse(readFileSync(join(__dirname, 'auth-secret.json'), 'utf8'));
const env = { ...process.env, CODEBUDDY_AUTH_TOKEN: secret.accessToken, CODEBUDDY_CONFIG_DIR: join(__dirname, 'profile') };
delete env.ELECTRON_RUN_AS_NODE;
delete env.CODEBUDDY_API_KEY;
delete env.CODEBUDDY_BASE_URL;
if (secret.domestic) {
  env.CODEBUDDY_INTERNET_ENVIRONMENT = 'internal';
  env.CODEBUDDY_COPILOT_INTERNET_ENVIRONMENT = 'internal';
} else {
  delete env.CODEBUDDY_INTERNET_ENVIRONMENT;
  delete env.CODEBUDDY_COPILOT_INTERNET_ENVIRONMENT;
}
const isBatch = !secret.cliPath || /\.(?:cmd|bat)$/i.test(secret.cliPath);
const options = { stdio: 'inherit', env, windowsHide: true };
let child;
if (isBatch) {
  env.COCKPIT_CODEBUDDY_CLI_BINARY = secret.cliPath || 'codebuddy';
  child = spawn(process.env.ComSpec || 'cmd.exe',
    ['/d', '/v:off', '/s', '/c', '""%COCKPIT_CODEBUDDY_CLI_BINARY%" --acp"'],
    { ...options, windowsVerbatimArguments: true });
} else {
  child = spawn(secret.cliPath, ['--acp', ...process.argv.slice(2)], options);
}
child.on('error', (error) => { console.error(error.message); process.exit(1); });
child.on('exit', (code) => process.exit(code ?? 1));
"#;

pub(super) fn update_codebuddy_runtime(
    state_dir: &Path,
    instance_id: &str,
    platform: &str,
    entry: &Value,
    cli: Option<&Path>,
) -> Result<Option<bool>, String> {
    let instance_root = cockpit_provider_dir(state_dir, platform, instance_id);
    let config = &entry["config"];
    if config["source"] != "local" {
        return Ok(None);
    }
    #[cfg(unix)]
    {
        let wrapper = instance_root.join("run-codebuddy-acp");
        if config["commandPath"].as_str().map(Path::new) != Some(wrapper.as_path())
            || !instance_root.join("secret.token").is_file()
        {
            return Ok(None);
        }
        write_codebuddy_unix_launcher(&wrapper, platform == PLATFORM_CODEBUDDY_CN, cli).map(Some)
    }
    #[cfg(windows)]
    {
        let launcher = instance_root.join("launch-codebuddy.cjs");
        if config["commandArgs"][0].as_str().map(Path::new) != Some(launcher.as_path()) {
            return Ok(None);
        }
        let path = instance_root.join("auth-secret.json");
        let data = fs::read(&path).map_err(|error| error.to_string())?;
        let mut secret: Value = serde_json::from_slice(&data).map_err(|error| error.to_string())?;
        let path_changed = secret["cliPath"].as_str().map(Path::new) != cli;
        let launcher_changed = !fs::read_to_string(&launcher)
            .is_ok_and(|current| current == CODEBUDDY_WINDOWS_LAUNCHER);
        if launcher_changed {
            write_private_file(&launcher, CODEBUDDY_WINDOWS_LAUNCHER)?;
        }
        if path_changed {
            secret["cliPath"] = json!(cli);
            write_private_file(&path, &secret.to_string())?;
        }
        Ok(Some(path_changed || launcher_changed))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (instance_root, cli);
        Ok(None)
    }
}

pub fn write_extended_account_binding(
    state_dir: &Path,
    instance_id: &str,
    platform: &str,
    account_id: &str,
) -> Result<ExtendedBindingWriteOutcome, String> {
    let instance_root = cockpit_provider_dir(state_dir, platform, instance_id);
    fs::create_dir_all(&instance_root).map_err(|e| format!("创建 Provider 隔离目录失败: {e}"))?;
    restrict_dir(&instance_root);

    let mut config_patch = Map::new();
    let mut environment = Vec::new();
    let email: String;

    match platform {
        PLATFORM_WORKBUDDY | PLATFORM_WORKBUDDY_AI => {
            let domestic = platform == PLATFORM_WORKBUDDY;
            let account = if domestic {
                workbuddy_account::load_account(account_id)
                    .ok_or_else(|| "WorkBuddy 账号不存在".to_string())?
            } else {
                workbuddy_ai_account::load(account_id)
                    .map_err(|_| "WorkBuddy AI 账号不存在".to_string())?
            };
            if account.access_token.trim().is_empty() {
                return Err("所选 WorkBuddy 账号缺少有效登录凭据".to_string());
            }
            email = account.email.clone();
            let electron_exe = if domestic {
                resolve_workbuddy_domestic_executable()?
            } else {
                resolve_workbuddy_ai_executable()?
            };
            let cli_path = resolve_workbuddy_unpacked_cli(&electron_exe)?;
            let secret_json = serde_json::to_string_pretty(&json!({
                "accessToken": account.access_token,
                "cliPath": cli_path,
                "domestic": domestic,
            }))
            .map_err(|e| e.to_string())?;
            write_private_file(&instance_root.join("auth-secret.json"), &secret_json)?;
            let launcher_path = instance_root.join("launch.cjs");
            write_private_file(&launcher_path, WORKBUDDY_NODE_LAUNCHER_JS)?;

            config_patch.insert("source".to_string(), json!("local"));
            config_patch.insert("commandPath".to_string(), json!(electron_exe));
            config_patch.insert("commandArgs".to_string(), json!([launcher_path]));
            environment.push(env_item("ELECTRON_RUN_AS_NODE", "1"));
        }
        PLATFORM_CODEBUDDY | PLATFORM_CODEBUDDY_CN => {
            let domestic = platform == PLATFORM_CODEBUDDY_CN;
            let account = if domestic {
                codebuddy_cn_account::load_account(account_id)
                    .ok_or_else(|| "CodeBuddy CN 账号不存在".to_string())?
            } else {
                codebuddy_account::load_account(account_id)
                    .ok_or_else(|| "CodeBuddy 账号不存在".to_string())?
            };
            if account.access_token.trim().is_empty() {
                return Err("所选 CodeBuddy 账号缺少有效登录凭据".to_string());
            }
            email = account.email.clone();
            let cli_bin = crate::modules::codebuddy_cli::find_cli()
                .ok_or("CLI_PATH_NOT_FOUND:codebuddy_cli")?;
            environment.push(env_item(super::launch_paths::BORROWED_PATH, &cli_bin.to_string_lossy()));
            let profile_dir = instance_root.join("profile");
            fs::create_dir_all(&profile_dir)
                .map_err(|e| format!("创建 CodeBuddy 配置目录失败: {e}"))?;
            restrict_dir(&profile_dir);

            #[cfg(unix)]
            {
                write_private_file(&instance_root.join("secret.token"), account.access_token.trim())?;
                let wrapper = instance_root.join("run-codebuddy-acp");
                write_codebuddy_unix_launcher(&wrapper, domestic, Some(&cli_bin))?;
                config_patch.insert("source".to_string(), json!("local"));
                config_patch.insert("commandPath".to_string(), json!(wrapper));
                config_patch.insert("commandArgs".to_string(), json!([]));
            }

            #[cfg(windows)]
            {
                let (host_exe, needs_electron_node) = resolve_node_host_for_wrapper()?;
                let secret_json = serde_json::to_string_pretty(&json!({
                    "accessToken": account.access_token.trim(),
                    "cliPath": cli_bin,
                    "domestic": domestic,
                }))
                .map_err(|e| e.to_string())?;
                write_private_file(&instance_root.join("auth-secret.json"), &secret_json)?;
                let launcher = instance_root.join("launch-codebuddy.cjs");
                write_private_file(&launcher, CODEBUDDY_WINDOWS_LAUNCHER)?;
                config_patch.insert("source".to_string(), json!("local"));
                config_patch.insert("commandPath".to_string(), json!(host_exe));
                config_patch.insert("commandArgs".to_string(), json!([launcher]));
                if needs_electron_node {
                    environment.push(env_item("ELECTRON_RUN_AS_NODE", "1"));
                }
            }
        }
        PLATFORM_GROK => {
            let account = grok_account::load_account(account_id)
                .ok_or_else(|| "Grok 账号不存在".to_string())?;
            email = account.email.clone();
            let home_dir = instance_root.join("home");
            grok_account::write_account_to_profile(&account, &home_dir)?;
            environment.push(env_item("GROK_HOME", &home_dir.to_string_lossy()));

            let grok_bin = resolve_cli_binary("grok", &[".grok/bin", ".local/bin"], true);
            if account.is_api_key_auth() {
                let api_key = account
                    .resolved_api_key()
                    .ok_or_else(|| "Grok API Key 账号缺少 api_key".to_string())?;
                let real_grok = grok_bin
                    .ok_or_else(|| "未检测到 grok CLI，请先安装 Grok Build CLI".to_string())?;
                write_private_file(&instance_root.join("secret.key"), api_key)?;

                #[cfg(unix)]
                {
                    let cli_escaped = real_grok.to_string_lossy().replace('\'', "'\\''");
                    let script = format!(
                        "#!/bin/sh\nset -e\nDIR=\"$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd)\"\nIFS= read -r GROK_KEY < \"$DIR/secret.key\"\nexport XAI_API_KEY=\"$GROK_KEY\"\nexport GROK_HOME=\"$DIR/home\"\nexec '{cli_escaped}' \"$@\"\n"
                    );
                    let wrapper = instance_root.join("run-grok");
                    write_executable_script(&wrapper, &script)?;
                    config_patch.insert("binaryPath".to_string(), json!(wrapper));
                }
                #[cfg(windows)]
                {
                    let wrapper_cmd = instance_root.join("run-grok.cmd");
                    let script = format!(
                        "@echo off\r\nset /p XAI_API_KEY=<\"%~dp0secret.key\"\r\nset \"GROK_HOME=%~dp0home\"\r\n\"{}\" %*\r\n",
                        real_grok.display()
                    );
                    write_private_file(&wrapper_cmd, &script)?;
                    config_patch.insert("binaryPath".to_string(), json!(wrapper_cmd));
                }
            } else if let Some(bin) = grok_bin {
                config_patch.insert("binaryPath".to_string(), json!(bin));
            } else {
                config_patch.remove("binaryPath");
            }
        }
        PLATFORM_MINIMAXCODE => {
            let account = minimax_code_account::load_account(account_id)
                .ok_or_else(|| "MiniMax Code 账号不存在".to_string())?;
            email = account.email.clone();
            let data_dir = instance_root.join("data");
            fs::create_dir_all(&data_dir)
                .map_err(|e| format!("创建 MiniMax Code 数据目录失败: {e}"))?;
            restrict_dir(&data_dir);
            minimax_code_account::switch_minimax_code_account_to_data_dir(&account, &data_dir)?;
            environment.push(env_item("MINIMAX_DATA_DIR", &data_dir.to_string_lossy()));

            if let Some(mcode_bin) = resolve_cli_binary("mcode", &[".local/bin"], false) {
                config_patch.insert("source".to_string(), json!("local"));
                config_patch.insert("commandPath".to_string(), json!(mcode_bin));
                config_patch.insert("commandArgs".to_string(), json!(["acp"]));
            } else {
                config_patch.insert("source".to_string(), json!("registry"));
                config_patch.insert("agentId".to_string(), json!("minimax-code"));
            }
        }
        platform => {
            let provider = QoderProvider::from_platform(platform)
                .ok_or_else(|| format!("该平台不支持 Cockpit 账号绑定: {platform}"))?;
            email = provider.write_binding(state_dir, instance_id, account_id, &mut config_patch, &mut environment)?;
        }
    }

    let meta_json = serde_json::to_string_pretty(&json!({
        "platform": platform,
        "accountId": account_id,
        "email": email,
        "updatedAt": Utc::now().timestamp(),
    }))
    .map_err(|e| e.to_string())?;
    write_private_file(&instance_root.join("binding-meta.json"), &meta_json)?;

    Ok(ExtendedBindingWriteOutcome {
        config_patch,
        environment,
        binding: PersistedProviderBinding {
            driver: platform.to_string(),
            account_id: account_id.to_string(),
            updated_at: Utc::now().timestamp(),
            unlinked: false,
            verified_cache: None,
            live_generation: None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn codebuddy_rebuilds_existing_launchers_and_reads_credentials_without_a_newline() {
        let state = tempfile::tempdir().unwrap();
        let cli = state.path().join("mock codebuddy");
        write_executable_script(&cli, "#!/bin/sh\n[ \"$CODEBUDDY_AUTH_TOKEN\" = fixture-token ] && printf '%s:%s' \"${CODEBUDDY_INTERNET_ENVIRONMENT:-global}\" \"$1\"\n").unwrap();
        for (platform, region) in [(PLATFORM_CODEBUDDY_CN, "internal"), (PLATFORM_CODEBUDDY, "global")] {
            let profile = cockpit_provider_dir(state.path(), platform, platform);
            let wrapper = profile.join("run-codebuddy-acp");
            write_private_file(&profile.join("secret.token"), "fixture-token").unwrap();
            write_executable_script(&wrapper, "#!/bin/sh\nexit 1\n").unwrap();
            let entry = json!({"config": {"source": "local", "commandPath": wrapper}});

            assert_eq!(update_codebuddy_runtime(state.path(), platform, platform, &entry, Some(&cli)).unwrap(), Some(true));
            assert_eq!(update_codebuddy_runtime(state.path(), platform, platform, &entry, Some(&cli)).unwrap(), Some(false));
            let result = std::process::Command::new(wrapper).output().unwrap();
            assert!(result.status.success());
            assert_eq!(String::from_utf8(result.stdout).unwrap(), format!("{region}:--acp"));
        }
    }

    #[test]
    fn infers_extended_platforms_from_registry_and_local_entries() {
        let empty = PersistedBindingsMap::new();

        let copilot = json!({
            "driver": "acpRegistry",
            "config": { "source": "registry", "agentId": "github-copilot-cli" }
        });
        assert_eq!(
            infer_platform_from_t3_entry("ghcp_1", "acpRegistry", copilot.as_object(), &empty)
                .as_deref(),
            None
        );

        let cb_cn = json!({
            "driver": "acpRegistry",
            "environment": [{ "name": "CODEBUDDY_INTERNET_ENVIRONMENT", "value": "internal", "sensitive": false }],
            "config": { "source": "registry", "agentId": "codebuddy-code" }
        });
        assert_eq!(
            infer_platform_from_t3_entry("cb_1", "acpRegistry", cb_cn.as_object(), &empty)
                .as_deref(),
            Some(PLATFORM_CODEBUDDY_CN)
        );

        let wb_ai = json!({
            "driver": "acpRegistry",
            "config": {
                "source": "local",
                "commandPath": "/Applications/WorkBuddy AI.app/Contents/MacOS/Electron",
                "commandArgs": ["/tmp/providers/cockpit/workbuddy_ai/wb_1/launch.cjs"]
            }
        });
        assert_eq!(
            infer_platform_from_t3_entry("wb_1", "acpRegistry", wb_ai.as_object(), &empty)
                .as_deref(),
            Some(PLATFORM_WORKBUDDY_AI)
        );

        let trae = json!({
            "driver": "acpRegistry",
            "config": { "source": "local", "commandPath": "/Users/test/.local/bin/traecli", "commandArgs": ["acp", "serve"] }
        });
        assert_eq!(
            infer_platform_from_t3_entry("trae_1", "acpRegistry", trae.as_object(), &empty)
                .as_deref(),
            None
        );
    }

    #[test]
    fn ignores_legacy_cli_only_bindings_and_rejects_them_for_new_providers() {
        let mut persisted = PersistedBindingsMap::new();
        for (platform, raw_driver) in [
            ("cursor", "cursor"),
            ("kiro", "kiro"),
            ("github-copilot", "acpRegistry"),
            ("windsurf", "acpRegistry"),
            ("trae_cn", "acpRegistry"),
        ] {
            persisted.insert(
                platform.to_string(),
                PersistedProviderBinding {
                    driver: platform.to_string(),
                    account_id: "__cli__".to_string(),
                    ..Default::default()
                },
            );
            assert_eq!(t3_raw_driver_for_platform(platform), None);
            assert_eq!(
                infer_platform_from_t3_entry(platform, raw_driver, None, &persisted),
                None
            );
        }
        for platform in [PLATFORM_QODER, PLATFORM_QODER_CN] {
            persisted.insert(
                platform.to_string(),
                PersistedProviderBinding {
                    driver: platform.to_string(),
                    account_id: "__cli__".to_string(),
                    ..Default::default()
                },
            );
            assert_eq!(t3_raw_driver_for_platform(platform), Some("acpRegistry"));
            assert_eq!(infer_platform_from_t3_entry(platform, "acpRegistry", None, &persisted), None);
        }
    }

    #[test]
    fn extracts_codebuddy_suite_credit_amount() {
        let raw = json!({
            "userResource": {
                "data": {
                    "ResponseData": {
                        "Accounts": [
                            { "Status": 0, "CycleTotalSize": 1000, "CycleRemainSize": 650, "CycleEndTime": "2026-12-01T00:00:00Z" }
                        ]
                    }
                }
            }
        });
        let windows = extract_codebuddy_suite_windows(Some(&raw), None);
        assert_eq!(windows.len(), 1);
        assert_eq!(
            serde_json::to_value(&windows[0]).unwrap()["remainingAmount"],
            json!(650.0)
        );
        assert!(serde_json::to_value(&windows[0])
            .unwrap()
            .get("remainingPercent")
            .is_none());
    }

    #[test]
    fn reads_current_workbuddy_credit_slices_and_package_amounts() {
        let raw = json!({"data": {"Response": {"Data": {"Accounts": [
            {
                "Status": 0, "CapacityType": 4,
                "CycleCapacitySize": 1000, "CycleCapacityRemain": 990,
                "SlicePeriodUsageDetails": [{
                    "SlicePeriodCapacitySizePrecise": "100",
                    "SlicePeriodCapacityRemainPrecise": "75"
                }]
            },
            {"Status": 3, "CycleCapacitySizePrecise": "50", "CycleCapacityRemainPrecise": "0"},
            {"Status": 2, "CycleCapacitySize": 500, "CycleCapacityRemain": 500}
        ]}}}});
        let windows = extract_codebuddy_suite_windows(None, Some(&raw));
        assert_eq!(
            windows[0].remaining,
            T3CodeQuotaRemaining::Amount {
                remaining_amount: 75.0
            }
        );

        let summary = json!({"userResource": {"data": {"resourceSummary": {"Packages": [
            {"CycleTotalCapacity": 500, "CycleRemainCapacity": 200}
        ]}}}});
        let windows = extract_codebuddy_suite_windows(Some(&summary), None);
        assert_eq!(
            windows[0].remaining,
            T3CodeQuotaRemaining::Amount {
                remaining_amount: 200.0
            }
        );

        let incomplete = json!({"data": {"resources": [{"total": 500}]}});
        assert!(extract_codebuddy_suite_windows(Some(&incomplete), None).is_empty());
    }

    #[test]
    fn minimax_uses_reported_credit_balance_or_sums_credit_details() {
        let raw =
            json!({"membership": {"op_credit_summary": {"total_remaining_amount": "1,250.5"}}});
        assert_eq!(minimax_credit_amount(&raw), Some(1250.5));
        let raw = json!({"userResource": {"details": [
            {"credit_type": 1, "remaining_amount": "1,000"},
            {"credit_type": 2, "remaining_amount": "250.5"},
            {"credit_type": 9, "remaining_amount": "500"}
        ]}});
        assert_eq!(minimax_credit_amount(&raw), Some(1250.5));
    }

    #[test]
    fn resolves_credit_suite_plan_from_billing_and_never_guesses_from_a_bonus() {
        let billing =
            json!({"data": {"resourceSummary": {"SubscriptionPackageName": "Buddy One VIP"}}});
        assert_eq!(
            credit_suite_plan(Some("UNKNOWN"), None, Some(&billing), true),
            Some("Buddy One VIP")
        );

        let resources = json!({"data": {"Response": {"Data": {"Accounts": [
            {"Status": 0, "PackageCode": "TCACA_code_008_cfWoLwvjU4"},
            {"Status": 0, "PackageCode": "TCACA_code_005_maRGyrHhw1"}
        ]}}}});
        assert_eq!(
            credit_suite_plan(Some("UNKNOWN"), None, Some(&resources), false),
            Some("PRO")
        );
        let bonus =
            json!({"accounts": [{"Status": 0, "PackageCode": "TCACA_code_007_nzdH5h4Nl0"}]});
        assert_eq!(
            credit_suite_plan(Some("UNKNOWN"), None, Some(&bonus), false),
            None
        );
    }
}
