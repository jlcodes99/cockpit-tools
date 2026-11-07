//! Pure quota selection for Codex badges; product shares are not unified limits.
use crate::models::grok::GrokQuota;

fn remaining(used_percent: f64) -> Option<i64> {
    used_percent
        .is_finite()
        .then(|| (100.0 - used_percent.clamp(0.0, 100.0).floor()) as i64)
}

pub(super) fn quota_lines(quota: &GrokQuota, shared_label: &str) -> Vec<(String, i64)> {
    if quota.is_unified_billing_user == Some(true) {
        let used_percent =
            quota
                .weekly_limit_percent
                .or_else(|| match (quota.weekly_used, quota.weekly_total) {
                    (Some(used), Some(total)) if total > 0.0 => Some(used / total * 100.0),
                    _ => None,
                });
        return used_percent
            .and_then(remaining)
            .map(|value| vec![(shared_label.to_string(), value)])
            .unwrap_or_default();
    }
    quota
        .products
        .iter()
        .filter_map(|product| {
            let normalized = product
                .product
                .chars()
                .filter(|ch| ch.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase();
            if normalized != "grokbuild" && normalized != "productgrokbuild" {
                return None;
            }
            let used_percent = product.usage_percent.or_else(|| {
                match (product.used, product.total, product.remaining) {
                    (Some(used), Some(total), _) if total > 0.0 => Some(used / total * 100.0),
                    (_, Some(total), Some(left)) if total > 0.0 => {
                        Some((total - left) / total * 100.0)
                    }
                    _ => None,
                }
            })?;
            remaining(used_percent).map(|value| (product.product.clone(), value))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::grok::GrokProductUsage;

    #[test]
    fn unified_badge_uses_the_seat_pool_instead_of_grok_build_share() {
        let mut quota = GrokQuota {
            is_unified_billing_user: Some(true),
            weekly_limit_percent: Some(99.99),
            products: vec![GrokProductUsage {
                product: "PRODUCT_GROK_BUILD".into(),
                usage_percent: Some(100.0),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(
            quota_lines(&quota, "共享额度"),
            vec![("共享额度".into(), 1)]
        );
        quota.weekly_limit_percent = None;
        assert!(quota_lines(&quota, "共享额度").is_empty());
        quota.is_unified_billing_user = None;
        assert_eq!(
            quota_lines(&quota, "共享额度"),
            vec![("PRODUCT_GROK_BUILD".into(), 0)]
        );
    }
}
