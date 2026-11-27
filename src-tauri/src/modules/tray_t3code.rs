//! T3 Code 托盘显示构建与格式化逻辑

use crate::models::t3code::T3CodeQuotaRemaining;
use crate::modules::t3code_provider_bindings::{
    self, parse_t3code_reset_ts, resolve_t3code_summary_windows, T3CodeTrayProviderSummary,
};

fn translate_or(lang: &str, key: &str, fallback: &str) -> String {
    let translated = crate::modules::i18n::translate(lang, key, &[]);
    if translated == key {
        fallback.to_string()
    } else {
        translated
    }
}

fn format_reset_time_from_ts(lang: &str, reset_ts: i64) -> String {
    let now = chrono::Utc::now().timestamp();
    let remaining_secs = reset_ts - now;
    if remaining_secs <= 0 {
        return translate_or(lang, "common.shared.quota.resetDone", "Reset");
    }
    let mut secs = remaining_secs;
    let days = secs / 86_400;
    secs %= 86_400;
    let hours = secs / 3_600;
    secs %= 3_600;
    let minutes = (secs / 60).max(1);

    let duration = if days > 0 {
        format!("{days}d {hours}h {minutes}m")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    };

    let translated = crate::modules::i18n::translate(
        lang,
        "common.shared.quota.resetAt",
        &[("time", duration.as_str())],
    );
    if translated == "common.shared.quota.resetAt" {
        format!("Reset {duration}")
    } else {
        translated
    }
}

fn format_quota_number(value: f64) -> String {
    let normalized = if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    };
    if normalized.fract().abs() < 1e-6 {
        format!("{normalized:.0}")
    } else {
        let raw = format!("{normalized:.6}");
        raw.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn resolve_account_label(summary: &T3CodeTrayProviderSummary, lang: &str) -> String {
    if summary.bound_is_api_service {
        return translate_or(lang, "common.apiBudget.gatewayName", "Codex API Service");
    }
    summary
        .bound_account_email
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| summary.display_name.clone())
}

fn format_provider_title(summary: &T3CodeTrayProviderSummary, account_label: &str) -> String {
    if account_label == summary.display_name {
        summary.display_name.clone()
    } else {
        format!("{} · {}", summary.display_name, account_label)
    }
}

fn format_window_label(
    driver: &str,
    window: &crate::models::t3code::T3CodeQuotaWindowView,
    lang: &str,
) -> String {
    if window.id == "credits" {
        return translate_or(lang, "accounts.credits.label", "Credits");
    }
    if driver == t3code_provider_bindings::DRIVER_CODEX
        && (window.id == "weekly" || window.label == "7d")
    {
        return "Weekly".to_string();
    }
    window.label.clone()
}

fn format_window_line(
    summary: &T3CodeTrayProviderSummary,
    window: &crate::models::t3code::T3CodeQuotaWindowView,
    lang: &str,
    prefix: Option<&str>,
) -> String {
    let raw_label = format_window_label(&summary.driver, window, lang);
    let label = match prefix {
        Some(prefix) if !prefix.is_empty() => format!("{prefix} · {raw_label}"),
        _ => raw_label,
    };
    let value_text = match &window.remaining {
        T3CodeQuotaRemaining::Percent { remaining_percent } => {
            format!("{}%", (*remaining_percent).max(0))
        }
        T3CodeQuotaRemaining::Amount { remaining_amount } => format_quota_number(*remaining_amount),
    };
    if let Some(reset_ts) = parse_t3code_reset_ts(window.reset_time.as_deref()) {
        format!(
            "{label}: {value_text} · {}",
            format_reset_time_from_ts(lang, reset_ts)
        )
    } else {
        format!("{label}: {value_text}")
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn build_t3code_display_info(lang: &str) -> super::AccountDisplayInfo {
    let summaries = t3code_provider_bindings::list_default_tray_provider_summaries();
    format_t3code_display_info(&summaries, lang)
}

pub(super) fn format_t3code_display_info(
    summaries: &[T3CodeTrayProviderSummary],
    lang: &str,
) -> super::AccountDisplayInfo {
    let Some(first) = summaries.first() else {
        return super::AccountDisplayInfo {
            account: format!(
                "📧 {}",
                translate_or(lang, "t3code.providerWorkspace.unbound", "未绑定")
            ),
            quota_lines: vec!["—".to_string()],
        };
    };

    let first_label = resolve_account_label(first, lang);
    let first_title = format_provider_title(first, &first_label);
    let account = if summaries.len() == 1 {
        format!("📧 {first_title}")
    } else {
        format!("📧 {first_title} (+{})", summaries.len() - 1)
    };

    let mut quota_lines = Vec::new();
    if summaries.len() == 1 {
        let windows = resolve_t3code_summary_windows(
            &first.driver,
            &first.windows,
            first.quota_summary_groups.as_deref(),
            4,
        );
        for window in &windows {
            quota_lines.push(format_window_line(first, window, lang, None));
        }
        if quota_lines.is_empty() {
            if let Some(value) = first
                .provider_quota
                .as_ref()
                .and_then(|quota| quota.summary_value_text())
            {
                quota_lines.push(format!("{}: {value}", first.display_name));
            }
        }
    } else {
        for summary in summaries.iter().take(4) {
            let windows = resolve_t3code_summary_windows(
                &summary.driver,
                &summary.windows,
                summary.quota_summary_groups.as_deref(),
                2,
            );
            if windows.is_empty() {
                if let Some(value) = summary
                    .provider_quota
                    .as_ref()
                    .and_then(|quota| quota.summary_value_text())
                {
                    quota_lines.push(format!("{}: {value}", summary.display_name));
                } else {
                    let label = resolve_account_label(summary, lang);
                    quota_lines.push(format!("{}: {label}", summary.display_name));
                }
                continue;
            }
            for window in &windows {
                quota_lines.push(format_window_line(
                    summary,
                    window,
                    lang,
                    Some(&summary.display_name),
                ));
            }
        }
    }

    if quota_lines.is_empty() {
        quota_lines.push("—".to_string());
    }

    super::AccountDisplayInfo {
        account,
        quota_lines,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::quota::{QuotaSummaryGroup, QuotaSummaryWindow};
    use crate::models::t3code::T3CodeQuotaWindowView;

    fn percent_window(id: &str, label: &str, remaining_percent: i32) -> T3CodeQuotaWindowView {
        T3CodeQuotaWindowView {
            id: id.to_string(),
            label: label.to_string(),
            remaining: T3CodeQuotaRemaining::Percent { remaining_percent },
            reset_time: None,
        }
    }

    fn amount_window(id: &str, label: &str, remaining_amount: f64) -> T3CodeQuotaWindowView {
        T3CodeQuotaWindowView {
            id: id.to_string(),
            label: label.to_string(),
            remaining: T3CodeQuotaRemaining::Amount { remaining_amount },
            reset_time: None,
        }
    }

    #[test]
    fn empty_summaries_show_unbound_and_placeholder() {
        let info = format_t3code_display_info(&[], "en");
        assert!(info.account.starts_with("📧 "));
        assert_eq!(info.quota_lines, vec!["—".to_string()]);
    }

    #[test]
    fn single_codex_provider_formats_header_and_weekly_label() {
        let summary = T3CodeTrayProviderSummary {
            instance_id: "codex".to_string(),
            driver: "codex".to_string(),
            display_name: "Codex".to_string(),
            bound_account_id: Some("acc-1".to_string()),
            bound_account_email: Some("alice@example.com".to_string()),
            bound_account_plan: Some("PRO".to_string()),
            bound_is_api_service: false,
            bound_updated_at: Some(1_700_000_000),
            windows: vec![
                percent_window("5h", "5h", 85),
                percent_window("weekly", "7d", 42),
            ],
            quota_summary_groups: None,
            provider_quota: None,
        };

        let info = format_t3code_display_info(&[summary], "en");
        assert_eq!(info.account, "📧 Codex · alice@example.com");
        assert_eq!(
            info.quota_lines,
            vec!["5h: 85%".to_string(), "Weekly: 42%".to_string()]
        );
    }

    #[test]
    fn single_api_service_provider_uses_gateway_name() {
        let summary = T3CodeTrayProviderSummary {
            instance_id: "codex".to_string(),
            driver: "codex".to_string(),
            display_name: "Codex".to_string(),
            bound_account_id: Some("__api_service__".to_string()),
            bound_account_email: Some("Cockpit Codex API Service".to_string()),
            bound_account_plan: None,
            bound_is_api_service: true,
            bound_updated_at: None,
            windows: vec![percent_window("5h", "5h", 92)],
            quota_summary_groups: None,
            provider_quota: None,
        };

        let expected_gateway = translate_or("en", "common.apiBudget.gatewayName", "Codex API Service");
        let info = format_t3code_display_info(&[summary], "en");
        assert_eq!(info.account, format!("📧 Codex · {expected_gateway}"));
        assert_eq!(info.quota_lines, vec!["5h: 92%".to_string()]);
    }

    #[test]
    fn multiple_providers_include_extra_count_grouped_ag_and_credits() {
        let codex = T3CodeTrayProviderSummary {
            instance_id: "codex".to_string(),
            driver: "codex".to_string(),
            display_name: "Codex".to_string(),
            bound_account_id: Some("c1".to_string()),
            bound_account_email: Some("alice@example.com".to_string()),
            bound_account_plan: Some("PRO".to_string()),
            bound_is_api_service: false,
            bound_updated_at: None,
            windows: vec![percent_window("5h", "5h", 80)],
            quota_summary_groups: None,
            provider_quota: None,
        };
        let antigravity = T3CodeTrayProviderSummary {
            instance_id: "antigravity".to_string(),
            driver: "antigravity".to_string(),
            display_name: "Antigravity".to_string(),
            bound_account_id: Some("ag1".to_string()),
            bound_account_email: Some("bob@example.com".to_string()),
            bound_account_plan: Some("PRO".to_string()),
            bound_is_api_service: false,
            bound_updated_at: None,
            windows: vec![percent_window("raw-model", "Raw", 50)],
            quota_summary_groups: Some(vec![QuotaSummaryGroup {
                id: "gemini".to_string(),
                name: "Gemini".to_string(),
                models: vec![],
                windows: vec![QuotaSummaryWindow {
                    id: "raw-model".to_string(),
                    period: "5h".to_string(),
                    percentage: 90,
                    reset_time: String::new(),
                    summary_exhausted: None,
                }],
            }]),
            provider_quota: None,
        };
        let qoder = T3CodeTrayProviderSummary {
            instance_id: "qoder_cn".to_string(),
            driver: "qoder_cn".to_string(),
            display_name: "Qoder CN".to_string(),
            bound_account_id: Some("q1".to_string()),
            bound_account_email: Some("carol@example.com".to_string()),
            bound_account_plan: Some("FREE".to_string()),
            bound_is_api_service: false,
            bound_updated_at: None,
            windows: vec![amount_window("credits", "Credits", 125.5)],
            quota_summary_groups: None,
            provider_quota: None,
        };

        let expected_credits = translate_or("en", "accounts.credits.label", "Credits");
        let info = format_t3code_display_info(&[codex, antigravity, qoder], "en");
        assert_eq!(info.account, "📧 Codex · alice@example.com (+2)");
        assert_eq!(
            info.quota_lines,
            vec![
                "Codex · 5h: 80%".to_string(),
                "Antigravity · Gemini 5h: 90%".to_string(),
                format!("Qoder CN · {expected_credits}: 125.5"),
            ]
        );
    }
}
