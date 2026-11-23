//! Qoder 1.38 usage envelope. Keep account/enterprise scopes separate.
use serde_json::Value;

pub fn is_envelope(raw: &Value) -> bool {
    ["displayMode", "qoderUsage", "enterpriseUsage"]
        .iter()
        .any(|key| raw.get(*key).is_some())
}

pub fn personal(raw: &Value) -> Option<&Value> {
    if raw.get("displayMode").and_then(Value::as_str) == Some("enterprise") {
        return None;
    }
    if let Some(value) = raw.get("qoderUsage").filter(|v| v.is_object()) {
        return Some(value);
    }
    if !is_envelope(raw) && raw.is_object() {
        return Some(raw);
    }
    None
}

pub fn enterprise(raw: &Value) -> Option<&Value> {
    match raw.get("displayMode").and_then(Value::as_str) {
        Some("enterprise" | "hybrid") => raw.get("enterpriseUsage").filter(|v| v.is_object()),
        _ => None,
    }
}

/// Qoder's quota payload reports `percentage` as a 0–1 ratio; older data used 0–100.
pub fn percent_points(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    (if value <= 1.0 { value * 100.0 } else { value }).clamp(0.0, 100.0)
}

pub fn derived_percent(used: Option<f64>, total: Option<f64>) -> Option<f64> {
    match (used, total) {
        (Some(used), Some(total)) if total > 0.0 => {
            Some(((used / total) * 100.0).clamp(0.0, 100.0))
        }
        _ => None,
    }
}

pub fn scoped_credits(raw: &Value) -> Option<(Option<f64>, Option<f64>, Option<f64>, Option<f64>)> {
    if !is_envelope(raw) && !raw.is_null() && raw.get("userQuota").is_none() {
        return None;
    }
    let Some(usage) = personal(raw) else {
        return Some((None, None, None, None));
    };
    let number = |value: Option<&Value>| {
        value
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite() && *v >= 0.0)
    };
    let bucket = usage.get("userQuota");
    let used = number(bucket.and_then(|v| v.get("used")));
    let total = number(bucket.and_then(|v| v.get("total")));
    let remaining = number(bucket.and_then(|v| v.get("remaining")));
    let percent = derived_percent(used, total).or_else(|| {
        number(bucket.and_then(|v| v.get("percentage")))
            .or_else(|| number(usage.get("totalUsagePercentage")))
            .map(percent_points)
    });
    Some((used, total, remaining, percent))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn hybrid_uses_personal_bucket_without_mixing_enterprise_numbers() {
        let raw = json!({"displayMode":"hybrid","qoderUsage":{"userQuota":{"used":1,"total":100,"remaining":99,"percentage":1}},"enterpriseUsage":{"detailUrl":"https://qoder.com/usage","used":999}});
        assert_eq!(
            scoped_credits(&raw),
            Some((Some(1.0), Some(100.0), Some(99.0), Some(1.0)))
        );
        assert!(enterprise(&raw).is_some());
    }
    #[test]
    fn enterprise_never_reuses_stale_personal_quota() {
        let raw = json!({"displayMode":"enterprise","qoderUsage":{"used":999},"enterpriseUsage":{"detailUrl":"https://qoder.com/usage"}});
        assert!(personal(&raw).is_none());
        assert_eq!(scoped_credits(&raw), Some((None, None, None, None)));
        assert!(enterprise(&json!({"displayMode":"personal","enterpriseUsage":{}})).is_none());
    }
    #[test]
    fn ratio_percentage_is_converted_to_percent_points() {
        let raw = json!({"qoderUsage":{"userQuota":{"used":9,"total":300,"remaining":291,"percentage":0.03}}});
        assert_eq!(
            scoped_credits(&raw),
            Some((Some(9.0), Some(300.0), Some(291.0), Some(3.0)))
        );
        let only_percent = json!({"qoderUsage":{"userQuota":{"percentage":0.62}}});
        assert_eq!(
            scoped_credits(&only_percent),
            Some((None, None, None, Some(62.0)))
        );
    }
}
