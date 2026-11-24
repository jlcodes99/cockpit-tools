use super::{accounts::StoredAccount, Failure, Platform};
use crate::modules::web_report::{
    compute_cursor_used_percent_from_amount, extract_codebuddy_resources,
    extract_workbuddy_resources, get_copilot_snapshot, pick_first_number, pick_first_string,
    pick_number_in_item, pick_preferred_trae_pack, pick_string_in_item, pick_trae_pack_reset,
};
use crate::{commands, models};
use serde::Serialize;
use serde_json::{json, Map, Value};
use tauri::AppHandle;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct QuotaSnapshot {
    status: &'static str,
    updated_at: Option<i64>,
    data: Value,
}

impl QuotaSnapshot {
    fn new(updated_at: Option<i64>, failed: bool, mut data: Value) -> Self {
        discard_empty_fields(&mut data);
        let status = match (data.is_null(), failed) {
            (true, false) => "unavailable",
            (true, true) => "failed",
            (false, false) => "available",
            (false, true) => "stale",
        };
        Self {
            status,
            updated_at,
            data,
        }
    }

    pub(super) fn refresh_failed(&self) -> bool {
        matches!(self.status, "failed" | "stale")
    }
}

fn discard_empty_fields(value: &mut Value) {
    let empty = match value {
        Value::Object(fields) => {
            fields.retain(|_, field| {
                discard_empty_fields(field);
                !field.is_null()
            });
            fields.is_empty()
        }
        Value::Array(fields) => {
            for field in fields.iter_mut() {
                discard_empty_fields(field);
            }
            fields.retain(|field| !field.is_null());
            fields.is_empty()
        }
        _ => false,
    };
    if empty {
        *value = Value::Null;
    }
}

pub(super) fn snapshot(account: StoredAccount) -> QuotaSnapshot {
    macro_rules! status {
        ($account:ident, $data:expr) => {
            QuotaSnapshot::new(
                $account.usage_updated_at,
                $account.quota_query_last_error.is_some(),
                $data,
            )
        };
    }
    match account {
        StoredAccount::Antigravity(a) => {
            let quota = a.quota.as_ref();
            let data = quota.map(|q| json!({
                "models": q.models,
                "credits": q.credits,
                "summaryStale": q.quota_summary_stale.then_some(true),
                "forbidden": q.is_forbidden.then_some(true),
            })).unwrap_or(Value::Null);
            QuotaSnapshot::new(a.usage_updated_at.or_else(|| quota.map(|q| q.last_updated)),
                a.quota_error.is_some() || quota.is_some_and(|q| q.quota_summary_stale || q.is_forbidden), data)
        }
        StoredAccount::Codex(a) => {
            let data = a.quota.as_ref().map(|q| codex_quota(q, a.is_api_key_auth())).unwrap_or(Value::Null);
            let provider_failed = a.quota.as_ref().and_then(|q| q.raw_data.as_ref())
                .and_then(|raw| raw.get("provider_usage_query_failed")).and_then(Value::as_bool) == Some(true);
            QuotaSnapshot::new(a.usage_updated_at, a.quota_error.is_some() || provider_failed, data)
        }
        StoredAccount::Claude(a) => QuotaSnapshot::new(a.usage_updated_at, a.quota_error.is_some(),
            a.quota.as_ref().map(|q| json!({
                "fiveHour": { "usedPercent": q.five_hour_percentage, "resetAt": q.five_hour_reset_time },
                "sevenDay": { "usedPercent": q.seven_day_percentage, "resetAt": q.seven_day_reset_time },
                "sevenDaySonnet": { "usedPercent": q.seven_day_sonnet_percentage, "resetAt": q.seven_day_sonnet_reset_time },
                "extraUsage": { "usedPercent": q.extra_usage_percentage, "resetAt": q.extra_usage_reset_time,
                    "usedCents": q.extra_usage_used_cents, "limitCents": q.extra_usage_limit_cents }
            })).unwrap_or(Value::Null)),
        StoredAccount::Zed(a) => status!(a, json!({
            "tokenSpend": { "usedCents": a.token_spend_used_cents, "limitCents": a.token_spend_limit_cents, "remainingCents": a.token_spend_remaining_cents },
            "editPredictions": { "used": a.edit_predictions_used, "limit": a.edit_predictions_limit_raw, "remaining": a.edit_predictions_remaining_raw },
            "billingPeriodEndAt": a.billing_period_end_at,
        })),
        StoredAccount::Copilot(a) => status!(a, json!({
            "windows": copilot_windows(a.copilot_quota_snapshots.as_ref()),
            "limited": copilot_limited(a.copilot_limited_user_quotas.as_ref()),
            "resetDate": a.copilot_quota_reset_date,
            "limitedResetAt": a.copilot_limited_user_reset_date,
        })),
        StoredAccount::Windsurf(a) => status!(a, json!({
            "windows": copilot_windows(a.copilot_quota_snapshots.as_ref()),
            "plan": windsurf_quota(&a),
            "resetDate": a.copilot_quota_reset_date,
        })),
        StoredAccount::Kiro(a) => status!(a, json!({
            "credits": { "used": a.credits_used, "total": a.credits_total },
            "bonus": { "used": a.bonus_used, "total": a.bonus_total, "expiresInDays": a.bonus_expire_days },
            "resetAt": a.usage_reset_at,
        })),
        StoredAccount::Cursor(a) => status!(a, json!({
            "plan": a.cursor_usage_raw.as_ref().map(cursor_plan),
            "sand": a.sand_usage,
        })),
        StoredAccount::Grok(a) => status!(a, json!(a.quota)),
        StoredAccount::Codebuddy(a) => status!(a, json!({ "packages": buddy_packages(extract_codebuddy_resources(&a)) })),
        StoredAccount::Qoder(a) => status!(a, json!({
            "credits": { "used": a.credits_used, "total": a.credits_total, "remaining": a.credits_remaining,
                "usedPercent": a.credits_usage_percent },
        })),
        StoredAccount::Zcode(a) => status!(a, json!({
            "used": a.quota_used, "total": a.quota_total, "remaining": a.quota_remaining, "resetAt": a.quota_reset_at
        })),
        StoredAccount::Trae(a) => status!(a, trae_quota(&a)),
        StoredAccount::Workbuddy(a) => status!(a, json!({ "packages": buddy_packages(extract_workbuddy_resources(&a)) })),
        StoredAccount::Minimax(a) => status!(a, a.quota_raw.as_ref().map(minimax_quota).unwrap_or(Value::Null)),
    }
}

fn codex_quota(quota: &models::codex::CodexQuota, api_key: bool) -> Value {
    let raw = quota.raw_data.as_ref();
    let mut data = json!({
        "hourly": (quota.hourly_window_present != Some(false)).then(|| json!({
            "remainingPercent": quota.hourly_percentage, "resetAt": quota.hourly_reset_time,
            "windowMinutes": quota.hourly_window_minutes,
        })),
        "weekly": (quota.weekly_window_present != Some(false)).then(|| json!({
            "remainingPercent": quota.weekly_percentage, "resetAt": quota.weekly_reset_time,
            "windowMinutes": quota.weekly_window_minutes,
        })),
        "resetCreditsAvailable": quota.reset_credits_available,
        "resetCreditsNextExpiresAt": quota.reset_credits_next_expires_at,
    });
    if api_key {
        if let Some(provider) = raw.and_then(|raw| raw.get("provider_usage")) {
            let mut fields = Map::new();
            for key in [
                "remaining",
                "balance",
                "quotaLimit",
                "quotaUsed",
                "quotaRemaining",
                "todayRequests",
                "todayTotalTokens",
                "todayCost",
                "totalRequests",
                "totalTotalTokens",
                "totalCost",
            ] {
                if let Some(value) = provider.get(key).and_then(Value::as_f64) {
                    fields.insert(key.into(), json!(value));
                }
            }
            if let Some(unlimited) = provider.get("quotaUnlimited").and_then(Value::as_bool) {
                fields.insert("quotaUnlimited".into(), json!(unlimited));
            }
            data["providerUsage"] = Value::Object(fields);
        }
    }
    data
}

fn copilot_windows(snapshots: Option<&Value>) -> Option<Value> {
    let snapshots = snapshots?;
    let windows: Map<_, _> = ["completions", "chat", "premium_interactions"]
        .into_iter()
        .filter_map(|key| {
            let window = get_copilot_snapshot(snapshots, key)?;
            Some((
                key.to_string(),
                json!({
                    "remainingPercent": pick_first_number(window, &[&["percent_remaining"]]),
                    "remaining": pick_first_number(window, &[&["remaining"]]),
                    "entitlement": pick_first_number(window, &[&["entitlement"]]),
                    "unlimited": window.get("unlimited").and_then(Value::as_bool),
                    "hasQuota": window.get("has_quota").and_then(Value::as_bool),
                }),
            ))
        })
        .collect();
    Some(Value::Object(windows))
}

fn copilot_limited(quotas: Option<&Value>) -> Option<Value> {
    let quotas = quotas?;
    Some(json!({
        "chat": pick_first_number(quotas, &[&["chat"]]),
        "completions": pick_first_number(quotas, &[&["completions"]]),
    }))
}

fn windsurf_quota(account: &models::windsurf::WindsurfAccount) -> Value {
    let snapshots = account.copilot_quota_snapshots.as_ref();
    let user = account.windsurf_user_status.as_ref();
    let candidates = [
        snapshots.and_then(|v| v.get("windsurfPlanStatus")),
        snapshots.and_then(|v| v.pointer("/windsurfUserStatus/planStatus")),
        user.and_then(|v| v.pointer("/userStatus/planStatus")),
        user.and_then(|v| v.get("planStatus")),
        account.windsurf_plan_status.as_ref(),
    ];
    for candidate in candidates.into_iter().flatten() {
        let mut value = json!({
            "dailyRemainingPercent": pick_first_number(candidate, &[&["dailyQuotaRemainingPercent"], &["daily_quota_remaining_percent"]]),
            "weeklyRemainingPercent": pick_first_number(candidate, &[&["weeklyQuotaRemainingPercent"], &["weekly_quota_remaining_percent"]]),
            "dailyResetAt": pick_first_number(candidate, &[&["dailyQuotaResetAtUnix"], &["daily_quota_reset_at_unix"]]),
            "weeklyResetAt": pick_first_number(candidate, &[&["weeklyQuotaResetAtUnix"], &["weekly_quota_reset_at_unix"]]),
            "promptCredits": {
                "total": pick_first_number(candidate, &[&["availablePromptCredits"], &["available_prompt_credits"], &["promptCredits"], &["prompt_credits"]]),
                "used": pick_first_number(candidate, &[&["usedPromptCredits"], &["used_prompt_credits"], &["promptCreditsUsed"], &["prompt_credits_used"], &["consumedPromptCredits"], &["consumed_prompt_credits"]]),
            },
            "flowCredits": {
                "total": pick_first_number(candidate, &[&["availableFlowCredits"], &["available_flow_credits"], &["flowCredits"], &["flow_credits"]]),
                "used": pick_first_number(candidate, &[&["usedFlowCredits"], &["used_flow_credits"], &["flowCreditsUsed"], &["flow_credits_used"], &["consumedFlowCredits"], &["consumed_flow_credits"]]),
            },
            "overageBalanceMicros": pick_first_number(candidate, &[&["overageBalanceMicros"], &["overage_balance_micros"]]),
        });
        discard_empty_fields(&mut value);
        if !value.is_null() {
            return value;
        }
    }
    Value::Null
}

fn cursor_plan(raw: &Value) -> Value {
    json!({
        "usedPercent": pick_first_number(raw, &[
            &["individualUsage", "plan", "totalPercentUsed"],
            &["individual_usage", "plan", "total_percent_used"],
            &["planUsage", "totalPercentUsed"], &["plan_usage", "total_percent_used"],
        ]).or_else(|| compute_cursor_used_percent_from_amount(raw)),
        "resetDate": pick_first_string(raw, &[&["billingCycleEnd"], &["billing_cycle_end"]]),
    })
}

fn buddy_packages(items: Vec<&Map<String, Value>>) -> Vec<Value> {
    items
        .into_iter()
        .filter_map(|item| {
            let total = pick_number_in_item(
                item,
                &[
                    "CycleCapacitySizePrecise",
                    "CycleCapacitySize",
                    "CapacitySizePrecise",
                    "CapacitySize",
                ],
            );
            let remaining = pick_number_in_item(
                item,
                &[
                    "CycleCapacityRemainPrecise",
                    "CycleCapacityRemain",
                    "CapacityRemainPrecise",
                    "CapacityRemain",
                ],
            );
            if total.is_none() && remaining.is_none() {
                return None;
            }
            Some(json!({
                "name": pick_string_in_item(item, &["PackageName", "PackageCode"]),
                "total": total, "remaining": remaining,
                "resetDate": pick_string_in_item(item, &["CycleEndTime", "ExpiredTime"]),
            }))
        })
        .collect()
}

fn trae_quota(account: &models::trae::TraeAccount) -> Value {
    let Some(pack) = pick_preferred_trae_pack(account.trae_usage_raw.as_ref()) else {
        return Value::Null;
    };
    json!({
        "basic": {
            "used": pick_first_number(pack, &[&["usage", "basic_usage_amount"]]),
            "total": pick_first_number(pack, &[&["entitlement_base_info", "quota", "basic_usage_limit"]]),
        },
        "bonus": {
            "used": pick_first_number(pack, &[&["usage", "bonus_usage_amount"]]),
            "total": pick_first_number(pack, &[&["entitlement_base_info", "quota", "bonus_usage_limit"]]),
        },
        "payAsYouGoAmount": pick_first_number(pack, &[&["usage", "pay_go_amount"]]),
        "resetDate": pick_trae_pack_reset(pack),
        "planResetAt": account.plan_reset_at,
    })
}

fn minimax_quota(raw: &Value) -> Value {
    json!({
        "remaining": pick_first_number(raw, &[
            &["membership", "op_credit_summary", "total_remaining_amount"],
            &["membership", "opcredit_balance"], &["workspace", "opcredit_balance"],
        ]),
        "purchasedRemaining": pick_first_number(raw, &[
            &["membership", "op_credit_summary", "purchased_remaining_amount"],
            &["membership", "op_credit_summary", "purchased_credit"],
        ]),
        "giftedRemaining": pick_first_number(raw, &[
            &["membership", "op_credit_summary", "free_remaining_amount"],
            &["membership", "op_credit_summary", "free_credit"],
        ]),
    })
}

pub(super) async fn refresh(app: AppHandle, platform: Platform, id: String) -> Result<(), Failure> {
    use Platform::*;
    let result = match platform {
        Antigravity | AntigravityIde => commands::account::fetch_account_quota(id)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string()),
        Codex => Box::pin(commands::codex::refresh_codex_quota(app, id))
            .await
            .map(|_| ()),
        Claude => Box::pin(commands::claude::refresh_claude_quota(app, id))
            .await
            .map(|_| ()),
        Zed => commands::zed::refresh_zed_token(app, id).await.map(|_| ()),
        Copilot => commands::github_copilot::refresh_github_copilot_token(app, id)
            .await
            .map(|_| ()),
        Windsurf => Box::pin(commands::windsurf::refresh_windsurf_token(app, id))
            .await
            .map(|_| ()),
        Kiro => commands::kiro::refresh_kiro_token(app, id)
            .await
            .map(|_| ()),
        Cursor => Box::pin(commands::cursor::refresh_cursor_token(app, id))
            .await
            .map(|_| ()),
        Grok => Box::pin(commands::grok::refresh_grok_account(app, id))
            .await
            .map(|_| ()),
        Codebuddy => commands::codebuddy::refresh_codebuddy_token(app, id)
            .await
            .map(|_| ()),
        CodebuddyCn => commands::codebuddy_cn::refresh_codebuddy_cn_token(app, id)
            .await
            .map(|_| ()),
        Qoder => commands::qoder::refresh_qoder_token(app, id)
            .await
            .map(|_| ()),
        QoderCn => commands::qoder_cn::refresh_qoder_cn_token(app, id)
            .await
            .map(|_| ()),
        Zcode => commands::zcode::refresh_zcode_account(app, id)
            .await
            .map(|_| ()),
        Trae | TraeSolo | TraeCn | TraeSoloCn => {
            Box::pin(commands::trae::refresh_trae_token(app, id))
                .await
                .map(|_| ())
        }
        Workbuddy => commands::workbuddy::refresh_workbuddy_token(app, id)
            .await
            .map(|_| ()),
        WorkbuddyAi => commands::workbuddy_ai::refresh_workbuddy_ai_token(id)
            .await
            .map(|_| ()),
        Minimax => commands::minimax_code::refresh_minimax_code_token(app, id)
            .await
            .map(|_| ()),
    };
    result.map_err(Failure::host)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automation_projects_billing_data_without_provider_secrets_or_invented_windows() {
        let quota: models::codex::CodexQuota = serde_json::from_value(json!({
            "hourly_percentage": 0, "weekly_percentage": 0,
            "hourly_window_present": false, "weekly_window_present": false,
            "raw_data": {"provider_usage": {"remaining": 0, "quotaLimit": 100,
                "api_key": "fixture-secret", "details": [{"key":"token","value":"fixture-secret"}]}}
        }))
        .unwrap();
        let snapshot = QuotaSnapshot::new(Some(123), false, codex_quota(&quota, true));
        assert_eq!(
            serde_json::to_value(snapshot).unwrap(),
            json!({
                "status": "available", "updatedAt": 123,
                "data": { "providerUsage": { "remaining": 0.0, "quotaLimit": 100.0 } }
            })
        );

        let no_data = QuotaSnapshot::new(None, false, json!({"hours": {"remaining": null}}));
        assert_eq!(no_data.status, "unavailable");
        assert!(no_data.data.is_null());
    }
}
