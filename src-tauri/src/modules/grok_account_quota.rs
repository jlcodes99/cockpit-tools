//! Grok billing normalization. Keep current shared pools separate from prepaid credits.

use super::normalize_text;
use crate::models::grok::{GrokProductUsage, GrokQuota};
use serde_json::Value;

fn number(value: Option<&Value>) -> Option<f64> {
    match value {
        Some(Value::Number(value)) => value.as_f64().filter(|value| value.is_finite()),
        Some(Value::String(value)) => value.parse::<f64>().ok().filter(|value| value.is_finite()),
        Some(Value::Object(value)) => value.get("val").and_then(|item| number(Some(item))),
        _ => None,
    }
}

// Official Cent messages omit `val` at zero. Only Cent fields use this default.
fn cents(value: Option<&Value>) -> Option<f64> {
    match value {
        Some(Value::Object(object)) if object.is_empty() => Some(0.0),
        _ => number(value),
    }
}

fn raw_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .and_then(|value| normalize_text(Some(value)))
}

pub(super) fn active_subscription(value: &Value) -> Option<&Value> {
    value
        .get("subscriptions")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find(|item| {
                    matches!(
                        item.get("status").and_then(Value::as_str),
                        Some("SUBSCRIPTION_STATUS_ACTIVE" | "SubscriptionStatusActive")
                    )
                })
                .or_else(|| items.first())
        })
}

pub(super) fn user_payload(value: &Value) -> &Value {
    value
        .get("user")
        .filter(|item| item.is_object())
        .unwrap_or(value)
}

pub(super) fn user_subscription(value: &Value) -> Option<&Value> {
    let user = user_payload(value);
    user.get("subscription")
        .filter(|item| item.is_object())
        .or_else(|| active_subscription(user))
}

fn nested_number(value: &Value, key: &str) -> Option<f64> {
    match value {
        Value::Object(object) => object
            .get(key)
            .and_then(|item| number(Some(item)))
            .or_else(|| object.values().find_map(|item| nested_number(item, key))),
        Value::Array(items) => items.iter().find_map(|item| nested_number(item, key)),
        _ => None,
    }
}

fn credit_bag_amounts(value: &Value) -> Option<(Option<f64>, Option<f64>, Option<f64>)> {
    if let Some(items) = value.as_array() {
        return items.iter().find_map(credit_bag_amounts);
    }
    let object = value.as_object()?;
    let total = number(
        object
            .get("total")
            .or_else(|| object.get("limit"))
            .or_else(|| object.get("cap"))
            .or_else(|| object.get("allocation"))
            .or_else(|| object.get("amount")),
    );
    let used = number(
        object
            .get("used")
            .or_else(|| object.get("spent"))
            .or_else(|| object.get("consumed"))
            .or_else(|| object.get("usage")),
    );
    let remaining = number(
        object
            .get("remaining")
            .or_else(|| object.get("balance"))
            .or_else(|| object.get("left")),
    );
    if total.is_none() && used.is_none() && remaining.is_none() {
        return object
            .get("bags")
            .or_else(|| object.get("items"))
            .and_then(credit_bag_amounts);
    }
    let resolved_used = used.or_else(|| match (total, remaining) {
        (Some(total), Some(remaining)) => Some((total - remaining).max(0.0)),
        _ => None,
    });
    let resolved_remaining = remaining.or_else(|| match (total, resolved_used) {
        (Some(total), Some(used)) => Some((total - used).max(0.0)),
        _ => None,
    });
    Some((resolved_used, total, resolved_remaining))
}

fn credit_bag_usage_percent(value: &Value) -> Option<f64> {
    let (used, total, _) = credit_bag_amounts(value)?;
    if let Some(total) = total.filter(|value| value.is_finite() && *value > 0.0) {
        let used = used.filter(|value| value.is_finite())?;
        return Some(((used.max(0.0) / total) * 100.0).clamp(0.0, 100.0));
    }
    None
}

fn credit_usage_sources<'a>(billing: &'a Value, config: &'a Value) -> [Option<&'a Value>; 8] {
    let unified = config.get("isUnifiedBillingUser").and_then(Value::as_bool) == Some(true);
    [
        config.get("sharedPool"),
        config.get("weeklyCredits"),
        config.get("subscriptionCredits"),
        config.get("includedCredits"),
        (!unified).then(|| config.get("credits")).flatten(),
        (!unified).then(|| billing.get("credits")).flatten(),
        (!unified).then(|| billing.get("usage")).flatten(),
        (!unified).then(|| billing.get("creditBalance")).flatten(),
    ]
}

fn credit_usage_percent(billing: &Value, config: &Value) -> Option<f64> {
    credit_usage_sources(billing, config)
        .into_iter()
        .flatten()
        .find_map(credit_bag_usage_percent)
}

fn credit_usage_amounts(billing: &Value, config: &Value) -> (Option<f64>, Option<f64>) {
    credit_usage_sources(billing, config)
        .into_iter()
        .flatten()
        .find_map(|value| {
            let (used, total, _) = credit_bag_amounts(value)?;
            if used.is_some() || total.is_some() {
                Some((used, total))
            } else {
                None
            }
        })
        .unwrap_or((None, None))
}

fn product_usage_from_value(item: &Value) -> Option<GrokProductUsage> {
    let product = raw_string(item.get("product"))
        .or_else(|| raw_string(item.get("name")))
        .or_else(|| raw_string(item.get("productName")))?;
    let (used, total, remaining) = credit_bag_amounts(item).unwrap_or((None, None, None));
    let usage_percent = number(item.get("usagePercent"))
        .or_else(|| number(item.get("usedPercent")))
        .or_else(|| match (used, total) {
            (Some(used), Some(total)) if total > 0.0 => {
                Some(((used.max(0.0) / total) * 100.0).clamp(0.0, 100.0))
            }
            _ => None,
        });
    Some(GrokProductUsage {
        product,
        usage_percent,
        used,
        total,
        remaining,
    })
}

pub(super) fn quota_from_payload(
    billing: &Value,
    subscriptions: Option<&Value>,
    cli_user: Option<&Value>,
    task_usage: Option<&Value>,
) -> GrokQuota {
    let config = billing.get("config").unwrap_or(billing);
    let period = config.get("currentPeriod").unwrap_or(&Value::Null);
    let subscription = cli_user
        .and_then(user_subscription)
        .or_else(|| subscriptions.and_then(active_subscription))
        .or_else(|| active_subscription(billing))
        .or_else(|| active_subscription(config));
    // /user?include=subscription is live. Billing can carry a cached JWT tier.
    let live_tier = cli_user.and_then(|value| {
        let user = user_payload(value);
        raw_string(user.get("subscriptionTier"))
            .or_else(|| raw_string(user.get("subscription_tier")))
            .or_else(|| user_subscription(value).and_then(|item| raw_string(item.get("tier"))))
    });
    let subscription_tier = live_tier
        .or_else(|| raw_string(billing.get("subscription_tier")))
        .or_else(|| raw_string(billing.get("subscriptionTier")))
        .or_else(|| raw_string(config.get("subscription_tier")))
        .or_else(|| raw_string(config.get("subscriptionTier")))
        .or_else(|| subscription.and_then(|item| raw_string(item.get("tier"))))
        .or_else(|| {
            cli_user.and_then(|value| {
                let user = user_payload(value);
                raw_string(user.get("subscriptionTier"))
                    .or_else(|| raw_string(user.get("subscription_tier")))
            })
        });
    let products = config
        .get("productUsage")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(product_usage_from_value).collect())
        .unwrap_or_default();
    let (weekly_used, weekly_total) = credit_usage_amounts(billing, config);
    let is_unified_billing_user = config.get("isUnifiedBillingUser").and_then(Value::as_bool);
    let legacy_total = cents(config.get("monthlyLimit"));
    let legacy_used = cents(config.get("used"));
    let usage_percent = number(config.get("creditUsagePercent"))
        .or_else(|| credit_usage_percent(billing, config))
        .or_else(|| match (legacy_used, legacy_total) {
            (Some(used), Some(total)) if total > 0.0 && is_unified_billing_user != Some(true) => {
                Some((used / total * 100.0).clamp(0.0, 100.0))
            }
            _ => None,
        });
    // proto3 omits a zero percentage. A complete current period is evidence
    // of a known allowance; an empty/failed response must remain unknown.
    let usage_percent = usage_percent.or_else(|| {
        if config.get("creditUsagePercent").is_some() {
            return None;
        }
        let period_type = raw_string(period.get("type"))?;
        let known_period = matches!(
            period_type.to_ascii_uppercase().as_str(),
            "WEEKLY" | "MONTHLY" | "USAGE_PERIOD_TYPE_WEEKLY" | "USAGE_PERIOD_TYPE_MONTHLY"
        );
        let valid_dates = ["start", "end"].iter().all(|key| {
            period
                .get(key)
                .and_then(Value::as_str)
                .is_some_and(|value| chrono::DateTime::parse_from_rfc3339(value).is_ok())
        });
        (known_period && valid_dates).then_some(0.0)
    });
    GrokQuota {
        is_unified_billing_user,
        on_demand_enabled: billing
            .get("onDemandEnabled")
            .or_else(|| billing.get("on_demand_enabled"))
            .and_then(Value::as_bool),
        period_type: raw_string(period.get("type")).or_else(|| {
            legacy_total
                .filter(|value| *value > 0.0)
                .map(|_| "USAGE_PERIOD_TYPE_MONTHLY".to_string())
        }),
        period_start: raw_string(period.get("start"))
            .or_else(|| raw_string(config.get("billingPeriodStart"))),
        period_end: raw_string(period.get("end"))
            .or_else(|| raw_string(config.get("billingPeriodEnd"))),
        weekly_limit_percent: usage_percent,
        weekly_used,
        weekly_total,
        on_demand_used: cents(config.get("onDemandUsed")),
        on_demand_cap: cents(config.get("onDemandCap")),
        prepaid_balance: cents(config.get("prepaidBalance")),
        frequent_usage: task_usage.and_then(|value| nested_number(value, "frequentUsage")),
        frequent_limit: task_usage.and_then(|value| nested_number(value, "frequentLimit")),
        occasional_usage: task_usage.and_then(|value| nested_number(value, "occasionalUsage")),
        occasional_limit: task_usage.and_then(|value| nested_number(value, "occasionalLimit")),
        subscription_tier,
        subscription_status: subscription.and_then(|item| raw_string(item.get("status"))),
        products,
    }
}

#[cfg(test)]
mod tests {
    use super::quota_from_payload;
    use serde_json::json;

    #[test]
    fn live_tier_wins_over_cached_billing_tier_and_disabled_spending_is_preserved() {
        let billing = json!({"subscription_tier":"SuperGrok", "on_demand_enabled":false,
            "config":{"subscriptionTier":"SuperGrok"}});
        let user = json!({"user":{"subscriptionTier":"Premium"}});
        let quota = quota_from_payload(&billing, None, Some(&user), None);
        assert_eq!(quota.subscription_tier.as_deref(), Some("Premium"));
        assert_eq!(quota.on_demand_enabled, Some(false));
    }

    #[test]
    fn official_unified_billing_keeps_one_pool_and_cent_zeroes() {
        let billing = json!({
            "subscription_tier": "Premium",
            "onDemandEnabled": false,
            "config": {
                "isUnifiedBillingUser": true,
                "creditUsagePercent": 42.5,
                "currentPeriod": {
                    "type": "USAGE_PERIOD_TYPE_WEEKLY",
                    "start": "2026-10-12T00:00:00Z",
                    "end": "2026-10-19T00:00:00Z"
                },
                "prepaidBalance": {}, "onDemandUsed": {}, "onDemandCap": {"val":"500"},
                "productUsage": [{"product":"PRODUCT_CURSOR", "usagePercent":20.0}]
            }
        });
        let quota = quota_from_payload(&billing, None, None, None);
        assert_eq!(quota.is_unified_billing_user, Some(true));
        assert_eq!(quota.on_demand_enabled, Some(false));
        assert_eq!(quota.subscription_tier.as_deref(), Some("Premium"));
        assert_eq!(quota.weekly_limit_percent, Some(42.5));
        assert_eq!(quota.prepaid_balance, Some(0.0));
        assert_eq!(quota.on_demand_used, Some(0.0));
        assert_eq!(quota.on_demand_cap, Some(500.0));
        assert_eq!(quota.products[0].product, "PRODUCT_CURSOR");
        assert_eq!(quota.period_end.as_deref(), Some("2026-10-19T00:00:00Z"));
    }

    #[test]
    fn omitted_proto_percentage_is_zero_only_for_a_complete_current_period() {
        let billing = json!({"config": {
            "isUnifiedBillingUser": true,
            "currentPeriod": {"type":"USAGE_PERIOD_TYPE_WEEKLY",
                "start":"2026-10-12T00:00:00Z", "end":"2026-10-19T00:00:00Z"}
        }, "creditBalance":{"total":100,"used":100}});
        let quota = quota_from_payload(&billing, None, None, None);
        assert_eq!(quota.weekly_limit_percent, Some(0.0));
        assert_eq!(quota.weekly_total, None);
        for billing in [
            json!({}),
            json!({"config":{"currentPeriod":{}}}),
            json!({"config":{"currentPeriod":{"type":"weekly","start":"bad","end":"bad"}}}),
        ] {
            assert_eq!(
                quota_from_payload(&billing, None, None, None).weekly_limit_percent,
                None
            );
        }
    }

    #[test]
    fn included_pool_wins_over_purchased_balance() {
        let billing = json!({"credits":{"total":100,"used":100},"config":{
            "isUnifiedBillingUser":true,"sharedPool":{"total":100,"remaining":75}
        }});
        let quota = quota_from_payload(&billing, None, None, None);
        assert_eq!(quota.weekly_limit_percent, Some(25.0));
        assert_eq!(quota.weekly_used, Some(25.0));
        assert_eq!(quota.weekly_total, Some(100.0));
    }

    #[test]
    fn root_subscriptions_and_legacy_monthly_billing_still_work() {
        let billing = json!({"subscriptions":[
            {"tier":"Free", "status":"SUBSCRIPTION_STATUS_CANCELLED"},
            {"tier":"Super", "status":"SUBSCRIPTION_STATUS_ACTIVE"}],
            "config":{"monthlyLimit":{"val":2000},"used":{"val":500},
                "billingPeriodEnd":"2026-11-12T00:00:00Z"}
        });
        let quota = quota_from_payload(&billing, None, None, None);
        assert_eq!(quota.subscription_tier.as_deref(), Some("Super"));
        assert_eq!(quota.weekly_limit_percent, Some(25.0));
        assert_eq!(
            quota.period_type.as_deref(),
            Some("USAGE_PERIOD_TYPE_MONTHLY")
        );
    }

    #[test]
    fn unified_pool_never_falls_back_to_deprecated_monthly_budget() {
        let billing = json!({"config":{"isUnifiedBillingUser":true,
            "monthlyLimit":{"val":100},"used":{"val":100}}});
        assert_eq!(
            quota_from_payload(&billing, None, None, None).weekly_limit_percent,
            None
        );
    }
}
