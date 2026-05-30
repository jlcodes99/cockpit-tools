#[cfg(not(target_os = "macos"))]
use tauri::{AppHandle, Rect, Runtime};

#[cfg(not(target_os = "macos"))]
pub fn toggle_tray_menu<R: Runtime>(_app: &AppHandle<R>, _rect: Rect) {}

#[cfg(target_os = "macos")]
mod imp {
    use std::cmp::Ordering;
    use std::collections::{HashMap, HashSet};
    use std::ffi::{c_char, c_void, CStr, CString};

    use objc2::rc::Retained;
    use serde::Serialize;
    use serde_json::Value;
    use tauri::{AppHandle, Rect, Runtime};

    use crate::commands;
    use crate::modules;
    use crate::modules::tray::{PlatformId, TRAY_ID};

    unsafe extern "C" {
        fn macos_native_menu_toggle(snapshot_json: *const c_char, status_item_ptr: *mut c_void);
        fn macos_native_menu_update_snapshot(snapshot_json: *const c_char);
    }

    #[derive(Debug, Clone, Serialize)]
    struct MenuStrings {
        view_recommended: String,
        back_to_current: String,
        switch_to_viewed: String,
        refresh: String,
        open_cockpit_tools: String,
        open_details: String,
        view_all_accounts: String,
        settings: String,
        quit: String,
        empty_title: String,
        empty_desc: String,
    }

    #[derive(Debug, Clone, Serialize)]
    struct MenuSnapshot {
        strings: MenuStrings,
        platforms: Vec<PlatformSnapshot>,
        selected_platform_id: String,
    }

    #[derive(Debug, Clone, Serialize)]
    struct PlatformSnapshot {
        id: String,
        title: String,
        short_title: String,
        nav_target: String,
        accent_hex: String,
        current_account_id: Option<String>,
        recommended_account_id: Option<String>,
        cards: Vec<RenderedAccountCard>,
    }

    #[derive(Debug, Clone)]
    struct AccountCard {
        id: String,
        title: String,
        plan: Option<String>,
        updated_at: Option<i64>,
        quota_rows: Vec<QuotaRow>,
    }

    #[derive(Debug, Clone, Serialize)]
    struct RenderedAccountCard {
        id: String,
        title: String,
        plan: Option<String>,
        updated_text: String,
        quota_rows: Vec<QuotaRow>,
    }

    #[derive(Debug, Clone, Serialize)]
    struct QuotaRow {
        label: String,
        value: String,
        progress: Option<i32>,
        progress_tone: Option<ProgressTone>,
        subtext: Option<String>,
    }

    #[derive(Debug, Clone, Copy, Serialize)]
    #[serde(rename_all = "snake_case")]
    enum ProgressTone {
        High,
        Medium,
        Low,
        Critical,
    }

    #[derive(Debug, Clone, Copy)]
    struct CopilotMetric {
        used_percent: Option<i32>,
        included: bool,
    }

    #[derive(Debug, Clone, Copy)]
    struct CopilotUsage {
        inline: CopilotMetric,
        chat: CopilotMetric,
        premium: CopilotMetric,
        reset_ts: Option<i64>,
    }

    #[derive(Debug, Clone, Copy)]
    enum WindsurfUsageMode {
        Quota,
        Credits,
    }

    #[derive(Debug, Clone, Default)]
    struct WindsurfQuotaUsageSummary {
        daily_used_percent: Option<i32>,
        weekly_used_percent: Option<i32>,
        daily_reset_ts: Option<i64>,
        weekly_reset_ts: Option<i64>,
        overage_balance_micros: Option<f64>,
    }

    #[derive(Debug, Clone, Default)]
    struct WindsurfCreditsSummary {
        credits_left: Option<f64>,
        prompt_left: Option<f64>,
        prompt_total: Option<f64>,
        prompt_used: Option<f64>,
        add_on_left: Option<f64>,
        add_on_total: Option<f64>,
        add_on_used: Option<f64>,
        plan_end_ts: Option<i64>,
    }

    #[derive(Debug, Clone, Default)]
    struct CursorTrayUsage {
        total_used_percent: Option<i32>,
        auto_used_percent: Option<i32>,
        api_used_percent: Option<i32>,
        reset_ts: Option<i64>,
        on_demand_text: Option<String>,
        on_demand_percent: Option<i32>,
    }

    #[derive(Debug, Clone)]
    struct GeminiBucketRemaining {
        model_id: String,
        remaining_percent: i32,
        reset_at: Option<i64>,
    }

    #[derive(Debug, Clone, Default)]
    struct ResourceQuotaEntry {
        package_code: Option<String>,
        package_name: Option<String>,
        total: f64,
        remain: f64,
        used: f64,
        used_percent: i32,
        refresh_at: Option<i64>,
        expire_at: Option<i64>,
        is_base_package: bool,
    }

    #[derive(Debug, Clone, Default)]
    struct ResourceQuotaModel {
        resources: Vec<ResourceQuotaEntry>,
        extra: ResourceQuotaEntry,
    }

    #[derive(Debug, Clone, Default)]
    struct QoderQuotaBucket {
        used: Option<f64>,
        total: Option<f64>,
        remaining: Option<f64>,
        percentage: Option<i32>,
    }

    #[derive(Debug, Clone, Default)]
    struct QoderSubscriptionInfo {
        user_quota: QoderQuotaBucket,
        add_on_quota: QoderQuotaBucket,
        shared_credit_package_used: Option<f64>,
        total_usage_percentage: Option<i32>,
    }

    #[derive(Debug, Clone, Default)]
    struct TraeUsageSummary {
        used_percent: Option<i32>,
        spent_usd: Option<f64>,
        total_usd: Option<f64>,
        reset_at: Option<i64>,
        pay_as_you_go_open: Option<bool>,
        pay_as_you_go_usd: Option<f64>,
    }

    pub(crate) fn toggle_tray_menu<R: Runtime>(app: &AppHandle<R>, _rect: Rect) {
        let Some(snapshot) = build_snapshot() else {
            return;
        };
        let Ok(snapshot_json) = serde_json::to_string(&snapshot) else {
            return;
        };
        let snapshot_json = to_cstring(&snapshot_json);
        let app = app.clone();

        let _ = app.clone().run_on_main_thread(move || {
            let Some(tray) = app.tray_by_id(TRAY_ID) else {
                return;
            };
            let status_item_ptr = tray
                .with_inner_tray_icon(|tray_icon| {
                    tray_icon
                        .ns_status_item()
                        .map(|status_item| Retained::as_ptr(&status_item) as usize)
                })
                .ok()
                .flatten();

            let Some(status_item_ptr) = status_item_ptr else {
                return;
            };

            unsafe {
                macos_native_menu_toggle(snapshot_json.as_ptr(), status_item_ptr as *mut c_void);
            }
        });
    }

    fn visible_platforms() -> Vec<PlatformId> {
        let layout = modules::tray_layout::load_tray_layout();
        let visible = sanitize_platform_list(&layout.tray_platform_ids);
        let visible_set: HashSet<PlatformId> = visible.iter().copied().collect();

        let mut groups_by_id: HashMap<String, modules::tray_layout::TrayLayoutGroup> =
            HashMap::new();
        for group in layout.platform_groups {
            groups_by_id.insert(group.id.clone(), group);
        }

        let mut ordered = Vec::new();
        let mut used_platforms: HashSet<PlatformId> = HashSet::new();

        for raw_entry in &layout.ordered_entry_ids {
            if let Some(platform) = parse_platform_entry_id(raw_entry) {
                if visible_set.contains(&platform) && used_platforms.insert(platform) {
                    ordered.push(platform);
                }
                continue;
            }

            let Some(group_id) = parse_group_entry_id(raw_entry) else {
                continue;
            };
            let Some(group) = groups_by_id.get(&group_id) else {
                continue;
            };

            for raw_platform in &group.platform_ids {
                let Some(platform) = PlatformId::from_str(raw_platform.trim()) else {
                    continue;
                };
                if visible_set.contains(&platform) && used_platforms.insert(platform) {
                    ordered.push(platform);
                }
            }
        }

        for platform in normalize_platform_order(&layout.ordered_platform_ids) {
            if visible_set.contains(&platform) && used_platforms.insert(platform) {
                ordered.push(platform);
            }
        }

        ordered
    }

    fn sanitize_platform_list(ids: &[String]) -> Vec<PlatformId> {
        let mut result = Vec::new();
        let mut seen = HashSet::new();

        for raw in ids {
            let Some(platform) = PlatformId::from_str(raw.trim()) else {
                continue;
            };
            if seen.insert(platform) {
                result.push(platform);
            }
        }

        result
    }

    fn normalize_platform_order(ids: &[String]) -> Vec<PlatformId> {
        let mut result = sanitize_platform_list(ids);
        let mut seen: HashSet<PlatformId> = result.iter().copied().collect();

        for platform in PlatformId::default_order() {
            if seen.insert(platform) {
                result.push(platform);
            }
        }

        result
    }

    fn parse_platform_entry_id(raw: &str) -> Option<PlatformId> {
        let value = raw.strip_prefix("platform:")?;
        PlatformId::from_str(value.trim())
    }

    fn parse_group_entry_id(raw: &str) -> Option<String> {
        let value = raw.strip_prefix("group:")?.trim();
        if value.is_empty() {
            return None;
        }
        Some(value.to_string())
    }

    fn build_snapshot() -> Option<MenuSnapshot> {
        let visible_platforms = visible_platforms();
        let config = modules::config::get_user_config();
        let lang = normalize_lang(&config.language);
        let platforms = visible_platforms
            .into_iter()
            .map(|platform| build_platform_snapshot(platform, &lang))
            .collect::<Vec<_>>();
        let selected_platform_id = platforms
            .first()
            .map(|item| item.id.clone())
            .unwrap_or_default();

        Some(MenuSnapshot {
            strings: build_strings(&lang),
            platforms,
            selected_platform_id,
        })
    }

    fn build_platform_snapshot(platform: PlatformId, lang: &str) -> PlatformSnapshot {
        let (cards, current_account_id, recommended_account_id) =
            build_platform_cards(platform, lang);
        let cards = cards
            .into_iter()
            .map(|card| RenderedAccountCard {
                id: card.id,
                title: card.title,
                plan: card.plan,
                updated_text: format_updated_label(lang, card.updated_at.unwrap_or(0)),
                quota_rows: card.quota_rows,
            })
            .collect();

        PlatformSnapshot {
            id: platform.as_str().to_string(),
            title: platform.title().to_string(),
            short_title: switcher_title(platform).to_string(),
            nav_target: platform.nav_target().to_string(),
            accent_hex: platform_accent_hex(platform).to_string(),
            current_account_id,
            recommended_account_id,
            cards,
        }
    }

    fn build_strings(lang: &str) -> MenuStrings {
        MenuStrings {
            view_recommended: modules::i18n::translate(
                lang,
                "floatingCard.actions.viewRecommended",
                &[],
            ),
            back_to_current: modules::i18n::translate(
                lang,
                "floatingCard.actions.backToCurrent",
                &[],
            ),
            switch_to_viewed: modules::i18n::translate(
                lang,
                "floatingCard.actions.switchToThisAccount",
                &[],
            ),
            refresh: modules::i18n::translate(lang, "common.refresh", &[]),
            open_cockpit_tools: modules::i18n::translate(
                lang,
                "floatingCard.actions.openCockpitTools",
                &[],
            ),
            open_details: modules::i18n::translate(lang, "accounts.actions.viewDetails", &[]),
            view_all_accounts: modules::i18n::translate(lang, "dashboard.viewAllAccounts", &[]),
            settings: modules::i18n::translate(lang, "nav.settings", &[]),
            quit: modules::i18n::translate(lang, "closeDialog.quit", &[]),
            empty_title: modules::i18n::translate(lang, "floatingCard.empty.title", &[]),
            empty_desc: modules::i18n::translate(lang, "floatingCard.empty.desc", &[]),
        }
    }

    fn switcher_title(platform: PlatformId) -> &'static str {
        match platform {
            PlatformId::Antigravity => "AG IDE",
            PlatformId::GitHubCopilot => "Copilot",
            PlatformId::CodebuddyCn => "CodeBuddy CN",
            _ => platform.title(),
        }
    }

    fn platform_accent_hex(platform: PlatformId) -> &'static str {
        match platform {
            PlatformId::Antigravity => "#67c27b",
            PlatformId::Codex => "#1976ff",
            PlatformId::Zed => "#8b92a1",
            PlatformId::GitHubCopilot => "#8b92a1",
            PlatformId::Windsurf => "#21c7b7",
            PlatformId::Kiro => "#8b92a1",
            PlatformId::Cursor => "#21c7b7",
            PlatformId::Gemini => "#a972ff",
            PlatformId::Codebuddy => "#4b74ff",
            PlatformId::CodebuddyCn => "#4b74ff",
            PlatformId::Qoder => "#5664ff",
            PlatformId::Trae => "#4f46e5",
            PlatformId::Workbuddy => "#2fa36a",
        }
    }

    fn to_cstring(value: &str) -> CString {
        let sanitized = value.replace('\0', "");
        CString::new(sanitized).unwrap_or_else(|_| CString::new("{}").unwrap())
    }

    fn read_optional_c_string(ptr: *const c_char) -> Option<String> {
        if ptr.is_null() {
            return None;
        }
        Some(unsafe { CStr::from_ptr(ptr) }.to_string_lossy().to_string())
    }

    #[no_mangle]
    pub extern "C" fn macos_native_menu_dispatch_action(
        action: *const c_char,
        platform_id: *const c_char,
        account_id: *const c_char,
    ) {
        let Some(action) = read_optional_c_string(action) else {
            return;
        };
        let raw_platform_id = read_optional_c_string(platform_id);
        let platform = raw_platform_id.as_deref().and_then(PlatformId::from_str);
        let account_id = read_optional_c_string(account_id);

        match action.as_str() {
            "refresh" => {
                if let Some(platform) = platform {
                    spawn_refresh(platform, account_id);
                }
            }
            "switch" => {
                if let (Some(platform), Some(account_id)) = (platform, account_id) {
                    spawn_switch_account(platform, account_id);
                }
            }
            "open_details" | "view_all_accounts" => {
                if let Some(platform) = platform {
                    open_main_window_page(platform.nav_target());
                }
            }
            "open_cockpit_tools" => {
                open_main_window();
            }
            "settings" => {
                open_main_window_page("settings");
            }
            "quit" => {
                if let Some(app) = crate::get_app_handle() {
                    app.exit(0);
                }
            }
            _ => {}
        }
    }

    fn normalize_lang(lang: &str) -> String {
        lang.trim().replace('_', "-").to_ascii_lowercase()
    }

    fn normalize_unix_timestamp(value: i64) -> Option<i64> {
        if value <= 0 {
            return None;
        }
        if value > 1_000_000_000_000 {
            return Some(value / 1000);
        }
        Some(value)
    }

    fn format_updated_label(lang: &str, updated_at: i64) -> String {
        let Some(updated_at) = normalize_unix_timestamp(updated_at) else {
            return modules::i18n::translate(lang, "common.shared.quota.noData", &[]);
        };

        let now = chrono::Utc::now().timestamp();
        let diff = (now - updated_at).max(0);
        let relative = if diff < 60 {
            modules::i18n::translate(lang, "common.shared.time.lessThanMinute", &[])
        } else {
            let days = diff / 86_400;
            let hours = (diff % 86_400) / 3600;
            let minutes = (diff % 3600) / 60;
            if days > 0 {
                modules::i18n::translate(
                    lang,
                    "common.shared.time.relativeDaysHours",
                    &[("days", &days.to_string()), ("hours", &hours.to_string())],
                )
            } else if hours > 0 {
                modules::i18n::translate(
                    lang,
                    "common.shared.time.relativeHoursMinutes",
                    &[
                        ("hours", &hours.to_string()),
                        ("minutes", &minutes.to_string()),
                    ],
                )
            } else {
                modules::i18n::translate(
                    lang,
                    "common.shared.time.relativeMinutes",
                    &[("minutes", &minutes.to_string())],
                )
            }
        };

        modules::i18n::translate(
            lang,
            "common.shared.updated.label",
            &[("relative", &relative)],
        )
    }

    fn display_updated_at(
        usage_updated_at: Option<i64>,
        last_used: i64,
        created_at: i64,
    ) -> Option<i64> {
        Some(usage_updated_at.unwrap_or(last_used.max(created_at)))
    }

    fn format_reset_subtext(lang: &str, ts: Option<i64>) -> Option<String> {
        let ts = ts?;
        if ts <= 0 {
            return None;
        }
        let now = chrono::Utc::now().timestamp();
        if ts <= now {
            return Some(modules::i18n::translate(
                lang,
                "common.shared.quota.resetDone",
                &[],
            ));
        }
        let diff = ts - now;
        let days = diff / 86_400;
        let hours = (diff % 86_400) / 3600;
        let minutes = (diff % 3600) / 60;
        let relative = if days > 0 {
            modules::i18n::translate(
                lang,
                "common.shared.time.relativeDaysHours",
                &[("days", &days.to_string()), ("hours", &hours.to_string())],
            )
        } else if hours > 0 {
            modules::i18n::translate(
                lang,
                "common.shared.time.relativeHoursMinutes",
                &[
                    ("hours", &hours.to_string()),
                    ("minutes", &minutes.to_string()),
                ],
            )
        } else if minutes > 0 {
            modules::i18n::translate(
                lang,
                "common.shared.time.relativeMinutes",
                &[("minutes", &minutes.to_string())],
            )
        } else {
            modules::i18n::translate(lang, "common.shared.time.lessThanMinute", &[])
        };

        Some(modules::i18n::translate(
            lang,
            "common.shared.credits.planEndsInHours",
            &[("hours", &relative)],
        ))
    }

    fn translate_or(
        lang: &str,
        key: &str,
        fallback: &str,
        replacements: &[(&str, &str)],
    ) -> String {
        let translated = modules::i18n::translate(lang, key, replacements);
        if translated != key {
            return translated;
        }

        let mut output = fallback.to_string();
        for (name, value) in replacements {
            output = output.replace(&format!("{{{{{}}}}}", name), value);
        }
        output
    }

    fn clamp_percent(value: f64) -> i32 {
        if !value.is_finite() {
            return 0;
        }
        value.round().clamp(0.0, 100.0) as i32
    }

    fn first_non_empty<'a>(values: &[Option<&'a str>]) -> Option<&'a str> {
        values
            .iter()
            .flatten()
            .map(|value| value.trim())
            .find(|value| !value.is_empty())
    }

    fn format_quota_number(value: f64) -> String {
        let normalized = if value.is_finite() {
            value.max(0.0)
        } else {
            0.0
        };
        if (normalized.fract()).abs() < f64::EPSILON {
            format!("{normalized:.0}")
        } else {
            format_trimmed_decimal(normalized)
        }
    }

    fn format_currency_dollars(value: f64) -> String {
        format!("${:.2}", value.max(0.0))
    }

    fn format_currency_cents(value: f64) -> String {
        format!("${:.2}", (value / 100.0).max(0.0))
    }

    fn format_micros_usd(value: f64) -> String {
        format!("${:.2}", value.max(0.0) / 1_000_000.0)
    }

    fn format_zed_plan_label(plan_raw: &str) -> String {
        let trimmed = plan_raw.trim();
        if trimmed.is_empty() {
            return "UNKNOWN".to_string();
        }

        let normalized = if trimmed.len() >= 4 && trimmed[..4].eq_ignore_ascii_case("zed_") {
            trimmed[4..].trim()
        } else {
            trimmed
        };

        if normalized.is_empty() {
            "UNKNOWN".to_string()
        } else {
            normalized.to_uppercase()
        }
    }

    fn antigravity_remaining_tone(remaining_percent: i32) -> ProgressTone {
        if remaining_percent >= 70 {
            ProgressTone::High
        } else if remaining_percent >= 30 {
            ProgressTone::Medium
        } else {
            ProgressTone::Low
        }
    }

    fn codex_remaining_tone(remaining_percent: i32) -> ProgressTone {
        if remaining_percent >= 80 {
            ProgressTone::High
        } else if remaining_percent >= 40 {
            ProgressTone::Medium
        } else if remaining_percent >= 10 {
            ProgressTone::Low
        } else {
            ProgressTone::Critical
        }
    }

    fn usage_warning_tone(used_percent: i32) -> ProgressTone {
        if used_percent <= 20 {
            ProgressTone::High
        } else if used_percent <= 60 {
            ProgressTone::Medium
        } else if used_percent <= 85 {
            ProgressTone::Low
        } else {
            ProgressTone::Critical
        }
    }

    fn cursor_usage_tone(used_percent: i32) -> ProgressTone {
        if used_percent >= 90 {
            ProgressTone::Low
        } else if used_percent >= 70 {
            ProgressTone::Medium
        } else {
            ProgressTone::High
        }
    }

    fn remaining_balance_tone(remaining_percent: i32) -> ProgressTone {
        if remaining_percent <= 10 {
            ProgressTone::Low
        } else if remaining_percent <= 30 {
            ProgressTone::Medium
        } else {
            ProgressTone::High
        }
    }

    fn resource_remaining_tone(resource: &ResourceQuotaEntry) -> ProgressTone {
        let remaining_percent = if resource.total > 0.0 {
            clamp_percent((resource.remain / resource.total) * 100.0)
        } else {
            (100 - resource.used_percent).clamp(0, 100)
        };
        remaining_balance_tone(remaining_percent)
    }

    fn make_progress_row(
        label: String,
        value: String,
        progress: i32,
        subtext: Option<String>,
        progress_tone: ProgressTone,
    ) -> QuotaRow {
        QuotaRow {
            label,
            value,
            progress: Some(progress.clamp(0, 100)),
            progress_tone: Some(progress_tone),
            subtext,
        }
    }

    fn make_text_row(label: String, value: String, subtext: Option<String>) -> QuotaRow {
        QuotaRow {
            label,
            value,
            progress: None,
            progress_tone: None,
            subtext,
        }
    }

    fn json_path<'a>(root: Option<&'a Value>, path: &[&str]) -> Option<&'a Value> {
        let mut current = root?;
        for key in path {
            current = current.as_object()?.get(*key)?;
        }
        Some(current)
    }

    fn parse_json_number(value: &Value) -> Option<f64> {
        match value {
            Value::Number(number) => number.as_f64(),
            Value::String(text) => text.trim().parse::<f64>().ok(),
            _ => None,
        }
        .filter(|number| number.is_finite())
    }

    fn json_bool(value: &Value) -> Option<bool> {
        match value {
            Value::Bool(flag) => Some(*flag),
            Value::Number(number) => {
                if number.as_i64() == Some(1) {
                    Some(true)
                } else if number.as_i64() == Some(0) {
                    Some(false)
                } else {
                    None
                }
            }
            Value::String(text) => {
                let normalized = text.trim().to_ascii_lowercase();
                match normalized.as_str() {
                    "1" | "true" | "yes" => Some(true),
                    "0" | "false" | "no" => Some(false),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn first_string_from_roots<'a>(
        roots: &[Option<&'a Value>],
        paths: &[&[&str]],
    ) -> Option<&'a str> {
        for root in roots.iter().flatten() {
            for path in paths {
                if let Some(value) = json_path(Some(*root), path).and_then(|item| item.as_str()) {
                    let trimmed = value.trim();
                    if !trimmed.is_empty() {
                        return Some(value);
                    }
                }
            }
        }
        None
    }

    fn first_number_from_roots(roots: &[Option<&Value>], paths: &[&[&str]]) -> Option<f64> {
        for root in roots.iter().flatten() {
            for path in paths {
                if let Some(value) = json_path(Some(*root), path).and_then(parse_json_number) {
                    return Some(value);
                }
            }
        }
        None
    }

    fn first_timestamp_from_roots(roots: &[Option<&Value>], paths: &[&[&str]]) -> Option<i64> {
        for root in roots.iter().flatten() {
            for path in paths {
                if let Some(value) = json_path(Some(*root), path).and_then(parse_timestamp_like) {
                    return Some(value);
                }
            }
        }
        None
    }

    fn parse_timestamp_number(raw: f64) -> Option<i64> {
        if !raw.is_finite() || raw <= 0.0 {
            return None;
        }
        if raw > 1e12 {
            return Some((raw / 1000.0).floor() as i64);
        }
        Some(raw.floor() as i64)
    }

    fn parse_timestamp_like(value: &Value) -> Option<i64> {
        match value {
            Value::Number(number) => parse_timestamp_number(number.as_f64()?),
            Value::String(text) => {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    return None;
                }
                if let Ok(number) = trimmed.parse::<f64>() {
                    return parse_timestamp_number(number);
                }
                chrono::DateTime::parse_from_rfc3339(trimmed)
                    .ok()
                    .map(|timestamp| timestamp.timestamp())
            }
            Value::Object(object) => {
                if let Some(seconds) = object.get("seconds").and_then(|item| item.as_i64()) {
                    return Some(seconds);
                }
                if let Some(seconds) = object.get("unixSeconds").and_then(|item| item.as_i64()) {
                    return Some(seconds);
                }
                object.get("value").and_then(parse_timestamp_like)
            }
            _ => None,
        }
    }

    fn calc_used_percent(total: Option<f64>, remaining: Option<f64>) -> Option<i32> {
        let total = total?;
        let remaining = remaining?;
        if total <= 0.0 {
            return None;
        }
        Some(clamp_percent(
            ((total - remaining).max(0.0) / total) * 100.0,
        ))
    }

    fn sum_option_f64(left: Option<f64>, right: Option<f64>) -> Option<f64> {
        match (left, right) {
            (Some(a), Some(b)) => Some(a + b),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
    }

    fn parse_token_map(token: &str) -> HashMap<String, String> {
        let mut values = HashMap::new();
        let token_str = if token.contains(";sku=") {
            token
        } else {
            token.split(':').next().unwrap_or(token)
        };
        for item in token_str.split(';') {
            let mut parts = item.splitn(2, '=');
            let key = parts.next().unwrap_or("").trim();
            if key.is_empty() {
                continue;
            }
            let value = parts.next().unwrap_or("").trim();
            values.insert(key.to_string(), value.to_string());
        }
        values
    }

    fn parse_token_number(map: &HashMap<String, String>, key: &str) -> Option<f64> {
        map.get(key)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .and_then(|value| value.split(':').next())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite())
    }

    fn parse_reset_date_to_ts(reset_date: Option<&str>) -> Option<i64> {
        let reset_date = reset_date?.trim();
        if reset_date.is_empty() {
            return None;
        }
        chrono::DateTime::parse_from_rfc3339(reset_date)
            .ok()
            .map(|timestamp| timestamp.timestamp())
    }

    fn format_resource_time_text(
        lang: &str,
        resource: &ResourceQuotaEntry,
        updated_key: &str,
        expire_key: &str,
    ) -> Option<String> {
        let primary = if resource.is_base_package {
            resource.refresh_at
        } else {
            resource.expire_at
        };
        let fallback = if resource.is_base_package {
            resource.expire_at
        } else {
            resource.refresh_at
        };

        if let Some(primary_text) = format_reset_subtext(lang, primary) {
            let key = if resource.is_base_package {
                updated_key
            } else {
                expire_key
            };
            let fallback_text = if resource.is_base_package {
                "下次刷新时间：{{time}}"
            } else {
                "到期时间：{{time}}"
            };
            return Some(translate_or(
                lang,
                key,
                fallback_text,
                &[("time", primary_text.as_str())],
            ));
        }

        if let Some(fallback_text) = format_reset_subtext(lang, fallback) {
            let key = if resource.is_base_package {
                expire_key
            } else {
                updated_key
            };
            let fallback_value = if resource.is_base_package {
                "到期时间：{{time}}"
            } else {
                "下次刷新时间：{{time}}"
            };
            return Some(translate_or(
                lang,
                key,
                fallback_value,
                &[("time", fallback_text.as_str())],
            ));
        }

        None
    }

    fn parse_rfc3339_ts(value: &str) -> Option<i64> {
        chrono::DateTime::parse_from_rfc3339(value)
            .ok()
            .map(|item| item.timestamp())
    }

    fn normalize_antigravity_model_for_match(value: &str) -> String {
        let normalized = value.trim().to_lowercase();
        if normalized.is_empty() {
            return normalized;
        }
        if normalized.starts_with("gemini-3.1-flash")
            || normalized.starts_with("gemini-2.5-flash")
            || normalized.starts_with("gemini-3-flash")
        {
            return "gemini-3-flash".to_string();
        }
        if normalized.starts_with("gemini-3.1-pro-high")
            || normalized.starts_with("gemini-3-pro-high")
        {
            return "gemini-3.1-pro-high".to_string();
        }
        if normalized.starts_with("gemini-3.1-pro-low")
            || normalized.starts_with("gemini-3-pro-low")
        {
            return "gemini-3.1-pro-low".to_string();
        }
        if normalized.starts_with("claude-sonnet-4-6")
            || normalized.starts_with("claude-sonnet-4-5")
        {
            return "claude-sonnet-4-6".to_string();
        }
        if normalized.starts_with("claude-opus-4-6-thinking")
            || normalized.starts_with("claude-opus-4-5-thinking")
        {
            return "claude-opus-4-6-thinking".to_string();
        }
        match normalized.as_str() {
            "gemini-3-pro-high" => "gemini-3.1-pro-high".to_string(),
            "gemini-3-pro-low" => "gemini-3.1-pro-low".to_string(),
            "claude-sonnet-4-5" => "claude-sonnet-4-6".to_string(),
            "claude-sonnet-4-5-thinking" => "claude-sonnet-4-6".to_string(),
            "claude-opus-4-5-thinking" => "claude-opus-4-6-thinking".to_string(),
            _ => normalized,
        }
    }

    fn antigravity_model_matches(model_name: &str, target: &str) -> bool {
        let left = normalize_antigravity_model_for_match(model_name);
        let right = normalize_antigravity_model_for_match(target);
        if left.is_empty() || right.is_empty() {
            return false;
        }
        left == right
            || left.starts_with(&(right.clone() + "-"))
            || right.starts_with(&(left + "-"))
    }

    fn resolve_antigravity_plan_label(
        quota: Option<&crate::models::quota::QuotaData>,
    ) -> Option<String> {
        let raw = quota
            .and_then(|item| item.subscription_tier.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        let normalized = raw.to_ascii_lowercase();
        Some(
            if normalized.contains("ultra") {
                "ULTRA"
            } else if normalized.contains("pro") {
                "PRO"
            } else {
                "FREE"
            }
            .to_string(),
        )
    }

    fn parse_decimal_amount(raw: &str) -> Option<f64> {
        let sanitized = raw.replace(',', "");
        let trimmed = sanitized.trim();
        if trimmed.is_empty() {
            return None;
        }
        trimmed
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
    }

    fn format_trimmed_decimal(value: f64) -> String {
        let mut rendered = format!("{value:.2}");
        while rendered.ends_with('0') {
            rendered.pop();
        }
        if rendered.ends_with('.') {
            rendered.pop();
        }
        rendered
    }

    fn format_antigravity_available_credits(
        quota: Option<&crate::models::quota::QuotaData>,
    ) -> Option<String> {
        let quota = quota?;
        let mut total = 0.0;
        let mut has_valid_amount = false;

        for credit in &quota.credits {
            let Some(raw_amount) = credit.credit_amount.as_deref() else {
                continue;
            };
            let Some(parsed) = parse_decimal_amount(raw_amount) else {
                continue;
            };
            total += parsed;
            has_valid_amount = true;
        }

        has_valid_amount.then(|| format_trimmed_decimal(total))
    }

    fn build_antigravity_group_quota_rows(
        lang: &str,
        quota: &crate::models::quota::QuotaData,
    ) -> Vec<QuotaRow> {
        let settings = crate::modules::group_settings::load_group_settings();
        let ordered_groups = settings.get_ordered_groups(Some(3));
        if ordered_groups.is_empty() {
            return Vec::new();
        }

        let mut rows = Vec::new();
        for group_id in ordered_groups {
            let group_models = settings.get_models_in_group(&group_id);
            if group_models.is_empty() {
                continue;
            }

            let mut total_percentage: i64 = 0;
            let mut count: i64 = 0;
            let mut earliest_reset_ts: Option<i64> = None;

            for model in &quota.models {
                let belongs = group_models
                    .iter()
                    .any(|group_model| antigravity_model_matches(&model.name, group_model));
                if !belongs {
                    continue;
                }

                total_percentage += i64::from(model.percentage.clamp(0, 100));
                count += 1;
                if let Some(reset_ts) = parse_rfc3339_ts(&model.reset_time) {
                    earliest_reset_ts = Some(match earliest_reset_ts {
                        Some(current) => current.min(reset_ts),
                        None => reset_ts,
                    });
                }
            }

            if count <= 0 {
                continue;
            }

            let avg_percentage = (total_percentage as f64 / count as f64).round() as i32;
            let percentage = avg_percentage.clamp(0, 100);
            rows.push(make_progress_row(
                settings.get_group_name(&group_id),
                format!("{percentage}%"),
                percentage,
                format_reset_subtext(lang, earliest_reset_ts),
                antigravity_remaining_tone(percentage),
            ));
        }

        rows
    }

    fn build_antigravity_fallback_quota_rows(
        lang: &str,
        quota: &crate::models::quota::QuotaData,
    ) -> Vec<QuotaRow> {
        quota
            .models
            .iter()
            .take(3)
            .map(|model| {
                let percentage = model.percentage.clamp(0, 100);
                make_progress_row(
                    model
                        .display_name
                        .clone()
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or_else(|| model.name.clone()),
                    format!("{percentage}%"),
                    percentage,
                    format_reset_subtext(lang, parse_rfc3339_ts(&model.reset_time)),
                    antigravity_remaining_tone(percentage),
                )
            })
            .collect()
    }

    fn build_antigravity_quota_rows(
        lang: &str,
        quota: Option<&crate::models::quota::QuotaData>,
    ) -> Vec<QuotaRow> {
        let Some(quota) = quota else {
            return Vec::new();
        };

        let mut rows = build_antigravity_group_quota_rows(lang, quota);
        if rows.is_empty() {
            rows = build_antigravity_fallback_quota_rows(lang, quota);
        }

        rows.push(QuotaRow {
            label: modules::i18n::translate(lang, "common.shared.credits.availableAiCredits", &[]),
            value: format_antigravity_available_credits(Some(quota)).unwrap_or_default(),
            progress: None,
            progress_tone: None,
            subtext: None,
        });
        rows
    }

    fn format_codex_quota_metric_label(window_minutes: Option<i64>, fallback: &str) -> String {
        let Some(minutes) = window_minutes.filter(|value| *value > 0) else {
            return fallback.to_string();
        };

        const HOUR_MINUTES: i64 = 60;
        const DAY_MINUTES: i64 = 24 * HOUR_MINUTES;
        const WEEK_MINUTES: i64 = 7 * DAY_MINUTES;

        if minutes >= WEEK_MINUTES - 1 {
            let weeks = (minutes + WEEK_MINUTES - 1) / WEEK_MINUTES;
            return if weeks <= 1 {
                "Weekly".to_string()
            } else {
                format!("{weeks} Week")
            };
        }
        if minutes >= DAY_MINUTES - 1 {
            return format!("{}d", (minutes + DAY_MINUTES - 1) / DAY_MINUTES);
        }
        if minutes >= HOUR_MINUTES {
            return format!("{}h", (minutes + HOUR_MINUTES - 1) / HOUR_MINUTES);
        }
        format!("{}m", minutes.max(1))
    }

    fn parse_code_review_metric(
        quota: Option<&crate::models::codex::CodexQuota>,
    ) -> Option<QuotaRow> {
        let raw = quota?.raw_data.as_ref()?;
        let rate_limit = raw.get("code_review_rate_limit")?.as_object()?;
        let primary_window = rate_limit.get("primary_window");
        let secondary_window = rate_limit.get("secondary_window");

        let normalize_window = |window: &Value| -> Option<QuotaRow> {
            let used_percent = window.get("used_percent").and_then(parse_json_number)?;
            let percentage = clamp_percent(100.0 - used_percent);
            let limit_window_seconds = window
                .get("limit_window_seconds")
                .and_then(parse_json_number)
                .filter(|value| *value > 0.0);
            let window_minutes = limit_window_seconds.map(|value| (value / 60.0).ceil() as i64);
            let reset_at = window
                .get("reset_at")
                .and_then(parse_json_number)
                .map(|value| value.floor() as i64);
            let reset_after_seconds = window
                .get("reset_after_seconds")
                .and_then(parse_json_number)
                .filter(|value| *value >= 0.0)
                .map(|value| chrono::Utc::now().timestamp() + value.floor() as i64);

            let _reset_ts = reset_at.or(reset_after_seconds);
            Some(make_progress_row(
                "Code Review".to_string(),
                format!("{percentage}%"),
                percentage,
                None,
                codex_remaining_tone(percentage),
            ))
            .map(|mut row| {
                row.label = format_codex_quota_metric_label(window_minutes, "Code Review");
                row
            })
        };

        primary_window
            .and_then(normalize_window)
            .or_else(|| secondary_window.and_then(normalize_window))
    }

    fn quota_row_from_copilot_metric(
        lang: &str,
        label: String,
        metric: CopilotMetric,
        reset_ts: Option<i64>,
    ) -> QuotaRow {
        let value = if metric.included {
            translate_or(lang, "githubCopilot.usage.included", "Included", &[])
        } else {
            metric
                .used_percent
                .map(|percentage| format!("{percentage}%"))
                .unwrap_or_else(|| "-".to_string())
        };

        QuotaRow {
            label,
            value,
            progress: Some(metric.used_percent.unwrap_or(0).clamp(0, 100)),
            progress_tone: Some(usage_warning_tone(metric.used_percent.unwrap_or(0))),
            subtext: format_reset_subtext(lang, reset_ts),
        }
    }

    fn get_quota_snapshot<'a>(
        quota_snapshots: Option<&'a Value>,
        key: &str,
    ) -> Option<&'a serde_json::Map<String, Value>> {
        let snapshots = quota_snapshots.and_then(|value| value.as_object())?;
        let primary = snapshots.get(key).and_then(|snapshot| snapshot.as_object());
        if primary.is_some() {
            return primary;
        }
        if key == "premium_interactions" {
            return snapshots
                .get("premium_models")
                .and_then(|snapshot| snapshot.as_object());
        }
        None
    }

    fn entitlement_from_snapshot(snapshot: Option<&serde_json::Map<String, Value>>) -> Option<f64> {
        snapshot
            .and_then(|data| data.get("entitlement"))
            .and_then(parse_json_number)
            .filter(|value| *value > 0.0)
    }

    fn remaining_from_snapshot(snapshot: Option<&serde_json::Map<String, Value>>) -> Option<f64> {
        if let Some(remaining) = snapshot
            .and_then(|data| data.get("remaining"))
            .and_then(parse_json_number)
        {
            return Some(remaining);
        }

        let entitlement = snapshot
            .and_then(|data| data.get("entitlement"))
            .and_then(parse_json_number)?;
        let percent_remaining = snapshot
            .and_then(|data| data.get("percent_remaining"))
            .and_then(parse_json_number)?;
        if entitlement <= 0.0 {
            return None;
        }
        Some((entitlement * (percent_remaining / 100.0)).max(0.0))
    }

    fn is_included_snapshot(snapshot: Option<&serde_json::Map<String, Value>>) -> bool {
        if snapshot
            .and_then(|data| data.get("unlimited"))
            .and_then(|value| value.as_bool())
            == Some(true)
        {
            return true;
        }

        snapshot
            .and_then(|data| data.get("entitlement"))
            .and_then(parse_json_number)
            .map(|value| value < 0.0)
            .unwrap_or(false)
    }

    fn used_percent_from_snapshot(
        snapshot: Option<&serde_json::Map<String, Value>>,
    ) -> Option<i32> {
        if snapshot
            .and_then(|data| data.get("unlimited"))
            .and_then(|value| value.as_bool())
            == Some(true)
        {
            return Some(0);
        }

        let entitlement = snapshot
            .and_then(|data| data.get("entitlement"))
            .and_then(parse_json_number);
        let remaining = snapshot
            .and_then(|data| data.get("remaining"))
            .and_then(parse_json_number);
        if let (Some(total), Some(left)) = (entitlement, remaining) {
            return calc_used_percent(Some(total), Some(left));
        }

        snapshot
            .and_then(|data| data.get("percent_remaining"))
            .and_then(parse_json_number)
            .map(|value| clamp_percent(100.0 - value))
    }

    fn compute_copilot_usage(
        token: &str,
        plan: Option<&str>,
        limited_quotas: Option<&Value>,
        quota_snapshots: Option<&Value>,
        limited_reset_ts: Option<i64>,
        quota_reset_date: Option<&str>,
    ) -> CopilotUsage {
        let token_map = parse_token_map(token);
        let reset_ts = limited_reset_ts
            .or_else(|| parse_reset_date_to_ts(quota_reset_date))
            .or_else(|| parse_token_number(&token_map, "rd").map(|value| value.floor() as i64));
        let sku = token_map
            .get("sku")
            .map(|value| value.to_ascii_lowercase())
            .unwrap_or_default();
        let is_free_limited = sku.contains("free_limited")
            || sku.contains("no_auth_limited")
            || plan
                .map(|value| value.to_ascii_lowercase().contains("free_limited"))
                .unwrap_or(false);

        let completions_snapshot = get_quota_snapshot(quota_snapshots, "completions");
        let chat_snapshot = get_quota_snapshot(quota_snapshots, "chat");
        let premium_snapshot = get_quota_snapshot(quota_snapshots, "premium_interactions");

        let limited = limited_quotas.and_then(|value| value.as_object());
        let remaining_inline = remaining_from_snapshot(completions_snapshot).or_else(|| {
            limited
                .and_then(|object| object.get("completions"))
                .and_then(parse_json_number)
        });
        let remaining_chat = remaining_from_snapshot(chat_snapshot).or_else(|| {
            limited
                .and_then(|object| object.get("chat"))
                .and_then(parse_json_number)
        });

        let total_inline = entitlement_from_snapshot(completions_snapshot)
            .or_else(|| parse_token_number(&token_map, "cq"))
            .or(remaining_inline);
        let total_chat = entitlement_from_snapshot(chat_snapshot)
            .or_else(|| parse_token_number(&token_map, "tq"))
            .or_else(|| {
                if is_free_limited {
                    remaining_chat.map(|_| 500.0)
                } else {
                    remaining_chat
                }
            });

        CopilotUsage {
            inline: CopilotMetric {
                used_percent: used_percent_from_snapshot(completions_snapshot)
                    .or_else(|| calc_used_percent(total_inline, remaining_inline)),
                included: is_included_snapshot(completions_snapshot),
            },
            chat: CopilotMetric {
                used_percent: used_percent_from_snapshot(chat_snapshot)
                    .or_else(|| calc_used_percent(total_chat, remaining_chat)),
                included: is_included_snapshot(chat_snapshot),
            },
            premium: CopilotMetric {
                used_percent: used_percent_from_snapshot(premium_snapshot),
                included: is_included_snapshot(premium_snapshot),
            },
            reset_ts,
        }
    }

    fn windsurf_plan_status_roots<'a>(
        account: &'a crate::models::windsurf::WindsurfAccount,
    ) -> Vec<Option<&'a Value>> {
        let user_status = account.windsurf_user_status.as_ref();
        let snapshots = account.copilot_quota_snapshots.as_ref();
        let direct_plan_status = account.windsurf_plan_status.as_ref();

        vec![
            direct_plan_status,
            json_path(direct_plan_status, &["planStatus"]),
            json_path(user_status, &["userStatus", "planStatus"]),
            json_path(user_status, &["planStatus"]),
            json_path(snapshots, &["windsurfPlanStatus"]),
            json_path(snapshots, &["windsurfPlanStatus", "planStatus"]),
            json_path(
                snapshots,
                &["windsurfUserStatus", "userStatus", "planStatus"],
            ),
        ]
    }

    fn windsurf_plan_info_roots<'a>(
        account: &'a crate::models::windsurf::WindsurfAccount,
    ) -> Vec<Option<&'a Value>> {
        let direct_plan_status = account.windsurf_plan_status.as_ref();
        let snapshots = account.copilot_quota_snapshots.as_ref();
        vec![
            json_path(direct_plan_status, &["planInfo"]),
            json_path(direct_plan_status, &["plan_info"]),
            json_path(snapshots, &["windsurfPlanInfo"]),
            json_path(snapshots, &["windsurf_plan_info"]),
        ]
    }

    fn resolve_windsurf_plan_end_ts(
        account: &crate::models::windsurf::WindsurfAccount,
    ) -> Option<i64> {
        let user_status = account.windsurf_user_status.as_ref();
        let snapshots = account.copilot_quota_snapshots.as_ref();
        let candidates = [
            json_path(user_status, &["userStatus", "planStatus", "planEnd"]),
            json_path(user_status, &["userStatus", "planStatus", "plan_end"]),
            json_path(user_status, &["planStatus", "planEnd"]),
            json_path(user_status, &["planStatus", "plan_end"]),
            json_path(snapshots, &["windsurfPlanStatus", "planEnd"]),
            json_path(snapshots, &["windsurfPlanStatus", "plan_end"]),
            json_path(snapshots, &["windsurfPlanStatus", "planStatus", "planEnd"]),
            json_path(snapshots, &["windsurfPlanStatus", "planStatus", "plan_end"]),
            json_path(
                snapshots,
                &["windsurfUserStatus", "userStatus", "planStatus", "planEnd"],
            ),
            json_path(
                snapshots,
                &["windsurfUserStatus", "userStatus", "planStatus", "plan_end"],
            ),
        ];

        candidates
            .into_iter()
            .flatten()
            .find_map(parse_timestamp_like)
    }

    fn resolve_windsurf_billing_strategy(
        account: &crate::models::windsurf::WindsurfAccount,
    ) -> Option<String> {
        let plan_status_roots = windsurf_plan_status_roots(account);
        let plan_info_roots = windsurf_plan_info_roots(account);
        let raw = first_string_from_roots(
            &plan_status_roots,
            &[&["billingStrategy"], &["billing_strategy"]],
        )
        .or_else(|| {
            first_string_from_roots(
                &plan_info_roots,
                &[&["billingStrategy"], &["billing_strategy"]],
            )
        })?;

        let normalized = raw.trim().to_ascii_lowercase();
        let canonical = normalized
            .trim_start_matches("billing_strategy_")
            .trim_start_matches("billing-strategy-")
            .trim_start_matches("billingstrategy")
            .trim_matches('_')
            .trim_matches('-')
            .to_string();

        if canonical == "quota" {
            return Some("quota".to_string());
        }
        if canonical.contains("credit") {
            return Some("credits".to_string());
        }
        Some(canonical)
    }

    fn resolve_windsurf_usage_mode(
        account: &crate::models::windsurf::WindsurfAccount,
    ) -> WindsurfUsageMode {
        match resolve_windsurf_billing_strategy(account).as_deref() {
            Some("quota") => WindsurfUsageMode::Quota,
            _ => WindsurfUsageMode::Credits,
        }
    }

    fn resolve_windsurf_quota_usage_summary(
        account: &crate::models::windsurf::WindsurfAccount,
    ) -> WindsurfQuotaUsageSummary {
        let plan_status_roots = windsurf_plan_status_roots(account);
        let is_quota_billing =
            resolve_windsurf_billing_strategy(account).as_deref() == Some("quota");
        let daily_used = first_number_from_roots(
            &plan_status_roots,
            &[
                &["dailyQuotaRemainingPercent"],
                &["daily_quota_remaining_percent"],
            ],
        );
        let weekly_used = first_number_from_roots(
            &plan_status_roots,
            &[
                &["weeklyQuotaRemainingPercent"],
                &["weekly_quota_remaining_percent"],
            ],
        );

        WindsurfQuotaUsageSummary {
            daily_used_percent: daily_used
                .map(|value| clamp_percent(100.0 - value))
                .or(if is_quota_billing { Some(100) } else { None }),
            weekly_used_percent: weekly_used
                .map(|value| clamp_percent(100.0 - value))
                .or(if is_quota_billing { Some(100) } else { None }),
            daily_reset_ts: first_timestamp_from_roots(
                &plan_status_roots,
                &[&["dailyQuotaResetAtUnix"], &["daily_quota_reset_at_unix"]],
            ),
            weekly_reset_ts: first_timestamp_from_roots(
                &plan_status_roots,
                &[&["weeklyQuotaResetAtUnix"], &["weekly_quota_reset_at_unix"]],
            ),
            overage_balance_micros: first_number_from_roots(
                &plan_status_roots,
                &[&["overageBalanceMicros"], &["overage_balance_micros"]],
            ),
        }
    }

    fn resolve_windsurf_credits_summary(
        account: &crate::models::windsurf::WindsurfAccount,
    ) -> WindsurfCreditsSummary {
        let plan_status_roots = windsurf_plan_status_roots(account);
        let plan_info_roots = windsurf_plan_info_roots(account);

        let prompt_left = first_number_from_roots(
            &plan_status_roots,
            &[&["availablePromptCredits"], &["available_prompt_credits"]],
        );
        let prompt_used = first_number_from_roots(
            &plan_status_roots,
            &[&["usedPromptCredits"], &["used_prompt_credits"]],
        );
        let mut prompt_total = first_number_from_roots(
            &plan_info_roots,
            &[&["monthlyPromptCredits"], &["monthly_prompt_credits"]],
        )
        .or(prompt_left);
        if prompt_total.is_none() && prompt_left.is_some() {
            prompt_total = prompt_left;
        }
        let prompt_left_actual = match (prompt_total, prompt_used, prompt_left) {
            (Some(total), Some(used), _) => Some((total - used).max(0.0)),
            (Some(total), None, Some(left)) if total >= left => Some(left),
            (_, _, left) => left,
        };

        let add_on_left = first_number_from_roots(
            &plan_status_roots,
            &[
                &["availableFlexCredits"],
                &["available_flex_credits"],
                &["flexCreditsAvailable"],
                &["flex_credits_available"],
                &["availableAddOnCredits"],
                &["available_add_on_credits"],
                &["addOnCreditsAvailable"],
                &["add_on_credits_available"],
                &["availableTopUpCredits"],
                &["available_top_up_credits"],
                &["topUpCreditsAvailable"],
                &["top_up_credits_available"],
            ],
        )
        .or(Some(0.0));
        let add_on_used = first_number_from_roots(
            &plan_status_roots,
            &[
                &["usedFlexCredits"],
                &["used_flex_credits"],
                &["usedAddOnCredits"],
                &["used_add_on_credits"],
                &["usedTopUpCredits"],
                &["used_top_up_credits"],
            ],
        );
        let mut add_on_total = first_number_from_roots(
            &plan_info_roots,
            &[
                &["monthlyFlexCreditPurchaseAmount"],
                &["monthly_flex_credit_purchase_amount"],
                &["monthlyAddOnCredits"],
                &["monthly_add_on_credits"],
                &["monthlyTopUpCredits"],
                &["monthly_top_up_credits"],
            ],
        )
        .or(add_on_left);
        if let (Some(total), Some(left)) = (add_on_total, add_on_left) {
            if total < left {
                add_on_total = Some(left);
            }
        }
        let add_on_left_actual = match (add_on_total, add_on_used, add_on_left) {
            (Some(total), Some(used), _) => Some((total - used).max(0.0)),
            (Some(total), None, Some(left)) if total >= left => Some(left),
            (_, _, left) => left,
        };

        WindsurfCreditsSummary {
            credits_left: sum_option_f64(prompt_left_actual, add_on_left_actual),
            prompt_left: prompt_left_actual,
            prompt_total,
            prompt_used,
            add_on_left: add_on_left_actual,
            add_on_total,
            add_on_used,
            plan_end_ts: resolve_windsurf_plan_end_ts(account),
        }
    }

    fn clamp_cursor_percent(value: f64) -> i32 {
        if !value.is_finite() {
            return 0;
        }
        if value <= 0.0 {
            return 0;
        }
        if value >= 100.0 {
            return 100;
        }
        value.round() as i32
    }

    fn pick_cursor_number(value: Option<&Value>, keys: &[&str]) -> Option<f64> {
        let object = value?.as_object()?;
        for key in keys {
            let Some(raw) = object.get(*key) else {
                continue;
            };
            if let Some(value) = parse_json_number(raw) {
                return Some(value);
            }
        }
        None
    }

    fn pick_cursor_bool(value: Option<&Value>, keys: &[&str]) -> Option<bool> {
        let object = value?.as_object()?;
        for key in keys {
            let Some(raw) = object.get(*key) else {
                continue;
            };
            if let Some(value) = json_bool(raw) {
                return Some(value);
            }
        }
        None
    }

    fn read_cursor_tray_usage(account: &crate::models::cursor::CursorAccount) -> CursorTrayUsage {
        let Some(raw) = account.cursor_usage_raw.as_ref() else {
            return CursorTrayUsage::default();
        };
        let Some(raw_obj) = raw.as_object() else {
            return CursorTrayUsage::default();
        };

        let plan = raw_obj
            .get("individualUsage")
            .and_then(|value| value.as_object())
            .and_then(|value| value.get("plan"))
            .or_else(|| {
                raw_obj
                    .get("individual_usage")
                    .and_then(|value| value.as_object())
                    .and_then(|value| value.get("plan"))
            })
            .or_else(|| raw_obj.get("planUsage"))
            .or_else(|| raw_obj.get("plan_usage"));

        let total_direct = pick_cursor_number(plan, &["totalPercentUsed", "total_percent_used"]);
        let auto_direct = pick_cursor_number(plan, &["autoPercentUsed", "auto_percent_used"]);
        let api_direct = pick_cursor_number(plan, &["apiPercentUsed", "api_percent_used"]);

        let plan_used = pick_cursor_number(plan, &["used", "totalSpend", "total_spend"]);
        let plan_limit = pick_cursor_number(plan, &["limit"]);
        let total_ratio = match (plan_used, plan_limit) {
            (Some(used), Some(limit)) if limit > 0.0 => Some((used / limit) * 100.0),
            _ => None,
        };

        let individual_on_demand = raw_obj
            .get("individualUsage")
            .and_then(|value| value.as_object())
            .and_then(|value| value.get("onDemand"))
            .or_else(|| {
                raw_obj
                    .get("individual_usage")
                    .and_then(|value| value.as_object())
                    .and_then(|value| value.get("onDemand"))
            });
        let team_on_demand = raw_obj
            .get("teamUsage")
            .and_then(|value| value.as_object())
            .and_then(|value| value.get("onDemand"))
            .or_else(|| {
                raw_obj
                    .get("team_usage")
                    .and_then(|value| value.as_object())
                    .and_then(|value| value.get("onDemand"))
            });
        let spend_limit_usage = raw_obj
            .get("spendLimitUsage")
            .or_else(|| raw_obj.get("spend_limit_usage"));

        let on_demand_obj = individual_on_demand.or(spend_limit_usage);
        let on_demand_limit = pick_cursor_number(
            on_demand_obj,
            &[
                "limit",
                "individualLimit",
                "individual_limit",
                "pooledLimit",
                "pooled_limit",
            ],
        );
        let on_demand_used = pick_cursor_number(
            on_demand_obj,
            &[
                "used",
                "totalSpend",
                "total_spend",
                "individualUsed",
                "individual_used",
            ],
        );
        let team_on_demand_used = pick_cursor_number(team_on_demand, &["used"]);
        let on_demand_enabled = pick_cursor_bool(individual_on_demand, &["enabled"]);

        let limit_type = raw_obj
            .get("limitType")
            .or_else(|| raw_obj.get("limit_type"))
            .or_else(|| {
                spend_limit_usage
                    .and_then(|value| value.as_object())
                    .and_then(|value| value.get("limitType").or_else(|| value.get("limit_type")))
            })
            .and_then(|value| value.as_str())
            .map(|value| value.trim().to_ascii_lowercase());
        let is_team_limit = matches!(limit_type.as_deref(), Some("team"));

        let effective_used = if on_demand_used.unwrap_or(0.0) > 0.0 {
            on_demand_used.unwrap_or(0.0)
        } else if is_team_limit {
            team_on_demand_used.unwrap_or(0.0)
        } else {
            on_demand_used.unwrap_or(0.0)
        };

        let has_on_demand_hint = on_demand_obj.is_some()
            || on_demand_enabled.is_some()
            || is_team_limit
            || on_demand_limit.is_some();
        let (on_demand_text, on_demand_percent) = if !has_on_demand_hint {
            (None, None)
        } else if let Some(limit) = on_demand_limit {
            if limit > 0.0 {
                let percent = clamp_cursor_percent((effective_used / limit) * 100.0);
                (
                    Some(format!(
                        "{} / {}",
                        format_currency_cents(effective_used),
                        format_currency_cents(limit)
                    )),
                    Some(percent),
                )
            } else {
                (None, None)
            }
        } else if on_demand_enabled == Some(true) && !is_team_limit {
            (Some("Unlimited".to_string()), Some(0))
        } else {
            (Some("Disabled".to_string()), Some(0))
        };

        let reset_ts = raw_obj
            .get("billingCycleEnd")
            .or_else(|| raw_obj.get("billing_cycle_end"))
            .and_then(|value| value.as_str())
            .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
            .map(|value| value.timestamp());

        CursorTrayUsage {
            total_used_percent: total_direct.or(total_ratio).map(clamp_cursor_percent),
            auto_used_percent: auto_direct.map(clamp_cursor_percent),
            api_used_percent: api_direct.map(clamp_cursor_percent),
            reset_ts,
            on_demand_text,
            on_demand_percent,
        }
    }

    fn collect_gemini_bucket_remaining(
        account: &crate::models::gemini::GeminiAccount,
    ) -> Vec<GeminiBucketRemaining> {
        let Some(raw_usage) = account.gemini_usage_raw.as_ref() else {
            return Vec::new();
        };
        let Some(buckets) = raw_usage.get("buckets").and_then(|value| value.as_array()) else {
            return Vec::new();
        };

        let mut values = Vec::new();
        for bucket in buckets {
            let model_id = bucket
                .get("modelId")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let remaining = bucket
                .get("remainingFraction")
                .and_then(parse_json_number)
                .map(|value| clamp_percent(value * 100.0));
            let reset_at = bucket.get("resetTime").and_then(parse_timestamp_like);
            let (Some(model_id), Some(remaining_percent)) = (model_id, remaining) else {
                continue;
            };
            values.push(GeminiBucketRemaining {
                model_id,
                remaining_percent,
                reset_at,
            });
        }

        values.sort_by(|left, right| left.model_id.cmp(&right.model_id));
        values
    }

    fn pick_lowest_gemini_bucket<'a, F>(
        buckets: &'a [GeminiBucketRemaining],
        matcher: F,
    ) -> Option<&'a GeminiBucketRemaining>
    where
        F: Fn(&str) -> bool,
    {
        let mut matched = buckets.iter().filter(|bucket| matcher(&bucket.model_id));
        let mut best = matched.next()?;
        for current in matched {
            if current.remaining_percent < best.remaining_percent {
                best = current;
                continue;
            }
            if current.remaining_percent > best.remaining_percent {
                continue;
            }
            match (best.reset_at, current.reset_at) {
                (None, Some(_)) => best = current,
                (Some(best_ts), Some(current_ts)) if current_ts < best_ts => best = current,
                _ => {}
            }
        }
        Some(best)
    }

    fn resource_account_roots<'a>(
        quota_raw: Option<&'a Value>,
        usage_raw: Option<&'a Value>,
    ) -> Vec<&'a Value> {
        let quota_root = json_path(quota_raw, &["userResource"]).or(usage_raw);
        let accounts = json_path(quota_root, &["data", "Response", "Data", "Accounts"])
            .and_then(|value| value.as_array());
        accounts
            .into_iter()
            .flatten()
            .filter(|item| item.is_object())
            .collect()
    }

    fn is_active_resource(raw: &Value) -> bool {
        matches!(
            raw.get("Status")
                .and_then(parse_json_number)
                .map(|value| value as i64),
            Some(0 | 3)
        )
    }

    fn resource_package_code(raw: &Value) -> Option<String> {
        raw.get("PackageCode")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }

    fn parse_cycle_total(raw: &Value) -> f64 {
        raw.get("CycleCapacitySizePrecise")
            .and_then(parse_json_number)
            .or_else(|| raw.get("CycleCapacitySize").and_then(parse_json_number))
            .or_else(|| raw.get("CapacitySizePrecise").and_then(parse_json_number))
            .or_else(|| raw.get("CapacitySize").and_then(parse_json_number))
            .unwrap_or(0.0)
    }

    fn parse_cycle_remain(raw: &Value) -> f64 {
        raw.get("CycleCapacityRemainPrecise")
            .and_then(parse_json_number)
            .or_else(|| raw.get("CycleCapacityRemain").and_then(parse_json_number))
            .or_else(|| raw.get("CapacityRemainPrecise").and_then(parse_json_number))
            .or_else(|| raw.get("CapacityRemain").and_then(parse_json_number))
            .unwrap_or(0.0)
    }

    fn aggregate_resource_entries(entries: &[&Value]) -> Option<Value> {
        if entries.is_empty() {
            return None;
        }
        let mut merged = (*entries.first()?).clone();
        let total: f64 = entries.iter().map(|item| parse_cycle_total(item)).sum();
        let remain: f64 = entries.iter().map(|item| parse_cycle_remain(item)).sum();
        if let Some(object) = merged.as_object_mut() {
            object.insert(
                "CycleCapacitySizePrecise".to_string(),
                Value::String(total.to_string()),
            );
            object.insert(
                "CycleCapacityRemainPrecise".to_string(),
                Value::String(remain.to_string()),
            );
        }
        Some(merged)
    }

    fn to_resource_quota_entry(raw: &Value, extra_code: &str) -> ResourceQuotaEntry {
        let package_code = resource_package_code(raw);
        let package_name = raw
            .get("PackageName")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let total = parse_cycle_total(raw);
        let remain = parse_cycle_remain(raw);
        let used = (total - remain).max(0.0);
        let used_percent = if total > 0.0 {
            clamp_percent((used / total) * 100.0)
        } else {
            0
        };
        let cycle_end_at = raw
            .get("CycleEndTime")
            .and_then(|value| value.as_str())
            .and_then(parse_rfc3339_ts);
        let deduction_end_time = raw.get("DeductionEndTime").and_then(parse_json_number);
        let expire_at = deduction_end_time
            .and_then(parse_timestamp_number)
            .or_else(|| {
                raw.get("ExpiredTime")
                    .and_then(|value| value.as_str())
                    .and_then(parse_rfc3339_ts)
            })
            .or(cycle_end_at);
        let refresh_at =
            if cycle_end_at.is_some() && expire_at.is_some() && cycle_end_at != expire_at {
                cycle_end_at.map(|value| value + 1)
            } else {
                None
            };
        let is_base_package = package_code.as_deref() != Some(extra_code);

        ResourceQuotaEntry {
            package_code,
            package_name,
            total,
            remain,
            used,
            used_percent,
            refresh_at,
            expire_at,
            is_base_package,
        }
    }

    fn resolve_codebuddy_plan_badge(
        account: &crate::models::codebuddy::CodebuddyAccount,
    ) -> String {
        const PRO_MON: &str = "TCACA_code_002_AkiJS3ZHF5";
        const PRO_YEAR: &str = "TCACA_code_003_FAnt7lcmRT";
        const GIFT: &str = "TCACA_code_006_DbXS0lrypC";

        let profile_type = account
            .profile_raw
            .as_ref()
            .and_then(|value| value.get("type"))
            .and_then(|value| value.as_str())
            .map(|value| value.to_ascii_lowercase())
            .unwrap_or_default();
        if matches!(profile_type.as_str(), "ultimate" | "exclusive" | "premise") {
            return "ENTERPRISE".to_string();
        }

        let accounts =
            resource_account_roots(account.quota_raw.as_ref(), account.usage_raw.as_ref());
        let active: Vec<&Value> = accounts
            .into_iter()
            .filter(|item| is_active_resource(item))
            .collect();
        if active.iter().any(|item| {
            matches!(
                resource_package_code(item).as_deref(),
                Some(PRO_MON | PRO_YEAR)
            )
        }) {
            return "PRO".to_string();
        }
        if active
            .iter()
            .any(|item| resource_package_code(item).as_deref() == Some(GIFT))
        {
            return "TRIAL".to_string();
        }
        if active.is_empty() {
            let source = first_non_empty(&[
                account.payment_type.as_deref(),
                account.plan_type.as_deref(),
            ])
            .unwrap_or("");
            let normalized = source.to_ascii_lowercase();
            if normalized.contains("enterprise") {
                return "ENTERPRISE".to_string();
            }
            if normalized.contains("trial") {
                return "TRIAL".to_string();
            }
            if normalized.contains("pro") {
                return "PRO".to_string();
            }
            if normalized.contains("free") {
                return "FREE".to_string();
            }
            if !source.is_empty() {
                return source.to_ascii_uppercase();
            }
            return "UNKNOWN".to_string();
        }
        "FREE".to_string()
    }

    fn resolve_workbuddy_plan_badge(
        account: &crate::models::workbuddy::WorkbuddyAccount,
    ) -> String {
        const PRO_MON: &str = "TCACA_code_002_AkiJS3ZHF5";
        const PRO_YEAR: &str = "TCACA_code_003_FAnt7lcmRT";
        const GIFT: &str = "TCACA_code_006_DbXS0lrypC";

        let profile_type = account
            .profile_raw
            .as_ref()
            .and_then(|value| value.get("type"))
            .and_then(|value| value.as_str())
            .map(|value| value.to_ascii_lowercase())
            .unwrap_or_default();
        if matches!(profile_type.as_str(), "ultimate" | "exclusive" | "premise") {
            return "ENTERPRISE".to_string();
        }

        let accounts =
            resource_account_roots(account.quota_raw.as_ref(), account.usage_raw.as_ref());
        let active: Vec<&Value> = accounts
            .into_iter()
            .filter(|item| is_active_resource(item))
            .collect();
        if active.iter().any(|item| {
            matches!(
                resource_package_code(item).as_deref(),
                Some(PRO_MON | PRO_YEAR)
            )
        }) {
            return "PRO".to_string();
        }
        if active
            .iter()
            .any(|item| resource_package_code(item).as_deref() == Some(GIFT))
        {
            return "TRIAL".to_string();
        }
        if active.is_empty() {
            let source = first_non_empty(&[
                account.payment_type.as_deref(),
                account.plan_type.as_deref(),
            ])
            .unwrap_or("");
            let normalized = source.to_ascii_lowercase();
            if normalized.contains("enterprise") {
                return "ENTERPRISE".to_string();
            }
            if normalized.contains("trial") {
                return "TRIAL".to_string();
            }
            if normalized.contains("pro") {
                return "PRO".to_string();
            }
            if normalized.contains("free") {
                return "FREE".to_string();
            }
            if !source.is_empty() {
                return source.to_ascii_uppercase();
            }
            return "UNKNOWN".to_string();
        }
        "FREE".to_string()
    }

    fn build_resource_quota_model(
        quota_raw: Option<&Value>,
        usage_raw: Option<&Value>,
    ) -> ResourceQuotaModel {
        const FREE: &str = "TCACA_code_001_PqouKr6QWV";
        const PRO_MON: &str = "TCACA_code_002_AkiJS3ZHF5";
        const PRO_YEAR: &str = "TCACA_code_003_FAnt7lcmRT";
        const GIFT: &str = "TCACA_code_006_DbXS0lrypC";
        const ACTIVITY: &str = "TCACA_code_007_nzdH5h4Nl0";
        const FREE_MON: &str = "TCACA_code_008_cfWoLwvjU4";
        const EXTRA: &str = "TCACA_code_009_0XmEQc2xOf";

        let all: Vec<&Value> = resource_account_roots(quota_raw, usage_raw)
            .into_iter()
            .filter(|item| is_active_resource(item))
            .collect();
        if all.is_empty() {
            return ResourceQuotaModel {
                resources: Vec::new(),
                extra: ResourceQuotaEntry {
                    package_code: Some(EXTRA.to_string()),
                    is_base_package: false,
                    ..Default::default()
                },
            };
        }

        let pro: Vec<&Value> = all
            .iter()
            .copied()
            .filter(|item| {
                matches!(
                    resource_package_code(item).as_deref(),
                    Some(PRO_MON | PRO_YEAR)
                )
            })
            .collect();
        let extras: Vec<&Value> = all
            .iter()
            .copied()
            .filter(|item| resource_package_code(item).as_deref() == Some(EXTRA))
            .collect();
        let trial_or_free_mon: Vec<&Value> = all
            .iter()
            .copied()
            .filter(|item| {
                matches!(
                    resource_package_code(item).as_deref(),
                    Some(GIFT | FREE_MON)
                )
            })
            .collect();
        let free: Vec<&Value> = all
            .iter()
            .copied()
            .filter(|item| resource_package_code(item).as_deref() == Some(FREE))
            .collect();
        let activity: Vec<&Value> = all
            .iter()
            .copied()
            .filter(|item| resource_package_code(item).as_deref() == Some(ACTIVITY))
            .collect();

        let merged_trial_or_free_mon = aggregate_resource_entries(&trial_or_free_mon);
        let merged_free = aggregate_resource_entries(&free);
        let mut ordered = Vec::new();
        if let Some(item) = merged_trial_or_free_mon.as_ref() {
            ordered.push(item);
        }
        ordered.extend(pro.iter().copied());
        ordered.extend(activity.iter().copied());
        if let Some(item) = merged_free.as_ref() {
            ordered.push(item);
        }

        let resources = ordered
            .into_iter()
            .map(|item| to_resource_quota_entry(item, EXTRA))
            .collect();
        let extra = aggregate_resource_entries(&extras)
            .map(|value| to_resource_quota_entry(&value, EXTRA))
            .unwrap_or(ResourceQuotaEntry {
                package_code: Some(EXTRA.to_string()),
                is_base_package: false,
                ..Default::default()
            });

        ResourceQuotaModel { resources, extra }
    }

    fn resolve_codebuddy_resource_label(lang: &str, resource: &ResourceQuotaEntry) -> String {
        match resource.package_code.as_deref() {
            Some("TCACA_code_009_0XmEQc2xOf") => {
                translate_or(lang, "codebuddy.extraCredit.title", "加量包", &[])
            }
            Some("TCACA_code_007_nzdH5h4Nl0") => translate_or(
                lang,
                "codebuddy.quotaQuery.packageTitle.activity",
                "活动赠送包",
                &[],
            ),
            Some(
                "TCACA_code_001_PqouKr6QWV"
                | "TCACA_code_006_DbXS0lrypC"
                | "TCACA_code_008_cfWoLwvjU4",
            ) => translate_or(
                lang,
                "codebuddy.quotaQuery.packageTitle.base",
                "基础体验包",
                &[],
            ),
            Some("TCACA_code_002_AkiJS3ZHF5" | "TCACA_code_003_FAnt7lcmRT") => translate_or(
                lang,
                "codebuddy.quotaQuery.packageTitle.pro",
                "专业版订阅",
                &[],
            ),
            _ => resource.package_name.clone().unwrap_or_else(|| {
                translate_or(
                    lang,
                    "codebuddy.quotaQuery.packageUnknown",
                    "套餐信息未知",
                    &[],
                )
            }),
        }
    }

    fn resolve_workbuddy_resource_label(lang: &str, resource: &ResourceQuotaEntry) -> String {
        match resource.package_code.as_deref() {
            Some("TCACA_code_009_0XmEQc2xOf") => {
                translate_or(lang, "workbuddy.extraCredit.title", "加量包", &[])
            }
            Some("TCACA_code_007_nzdH5h4Nl0") => translate_or(
                lang,
                "workbuddy.quotaQuery.packageTitle.activity",
                "活动赠送包",
                &[],
            ),
            Some(
                "TCACA_code_001_PqouKr6QWV"
                | "TCACA_code_006_DbXS0lrypC"
                | "TCACA_code_008_cfWoLwvjU4",
            ) => translate_or(
                lang,
                "workbuddy.quotaQuery.packageTitle.base",
                "基础体验包",
                &[],
            ),
            Some("TCACA_code_002_AkiJS3ZHF5" | "TCACA_code_003_FAnt7lcmRT") => resource
                .package_name
                .clone()
                .unwrap_or_else(|| "PRO".to_string()),
            _ => resource.package_name.clone().unwrap_or_else(|| {
                translate_or(
                    lang,
                    "workbuddy.quotaQuery.packageUnknown",
                    "套餐信息未知",
                    &[],
                )
            }),
        }
    }

    fn build_qoder_subscription_info(
        account: &crate::models::qoder::QoderAccount,
    ) -> QoderSubscriptionInfo {
        let roots = [
            account.auth_credit_usage_raw.as_ref(),
            account.auth_user_plan_raw.as_ref(),
            account.auth_user_info_raw.as_ref(),
        ];
        let _plan_tag = first_string_from_roots(
            &roots,
            &[
                &["plan_tier_name"],
                &["tier_name"],
                &["tierName"],
                &["planTierName"],
                &["plan"],
                &["userTag"],
                &["user_tag"],
            ],
        )
        .map(str::to_string)
        .or_else(|| account.plan_type.clone())
        .unwrap_or_else(|| "UNKNOWN".to_string());

        let parse_bucket =
            |sources: &[Option<&Value>], fallback: QoderQuotaBucket| -> QoderQuotaBucket {
                let raw = sources.iter().find_map(|value| *value);
                let used = raw
                    .and_then(|value| {
                        json_path(Some(value), &["used"])
                            .or_else(|| json_path(Some(value), &["usage"]))
                            .or_else(|| json_path(Some(value), &["consumed"]))
                            .and_then(parse_json_number)
                    })
                    .or(fallback.used);
                let total = raw
                    .and_then(|value| {
                        json_path(Some(value), &["total"])
                            .or_else(|| json_path(Some(value), &["quota"]))
                            .or_else(|| json_path(Some(value), &["limit"]))
                            .and_then(parse_json_number)
                    })
                    .or(fallback.total);
                let remaining = raw
                    .and_then(|value| {
                        json_path(Some(value), &["remaining"])
                            .or_else(|| json_path(Some(value), &["available"]))
                            .or_else(|| json_path(Some(value), &["left"]))
                            .and_then(parse_json_number)
                    })
                    .or(fallback.remaining)
                    .or_else(|| match (used, total) {
                        (Some(used), Some(total)) => Some((total - used).max(0.0)),
                        _ => None,
                    });
                let percentage = raw
                    .and_then(|value| {
                        json_path(Some(value), &["percentage"])
                            .or_else(|| json_path(Some(value), &["usagePercent"]))
                            .or_else(|| json_path(Some(value), &["usage_percentage"]))
                            .and_then(parse_json_number)
                    })
                    .map(clamp_percent)
                    .or(fallback.percentage)
                    .or_else(|| match (used, total) {
                        (Some(used), Some(total)) if total > 0.0 => {
                            Some(clamp_percent((used / total) * 100.0))
                        }
                        _ => None,
                    });

                QoderQuotaBucket {
                    used,
                    total,
                    remaining,
                    percentage,
                }
            };

        let user_quota = parse_bucket(
            &[
                json_path(account.auth_credit_usage_raw.as_ref(), &["userQuota"]),
                json_path(account.auth_user_plan_raw.as_ref(), &["userQuota"]),
                json_path(account.auth_user_info_raw.as_ref(), &["userQuota"]),
            ],
            QoderQuotaBucket {
                used: account.credits_used,
                total: account.credits_total,
                remaining: account.credits_remaining,
                percentage: account.credits_usage_percent.map(clamp_percent),
            },
        );
        let add_on_quota = parse_bucket(
            &[
                json_path(account.auth_credit_usage_raw.as_ref(), &["addOnQuota"]),
                json_path(account.auth_credit_usage_raw.as_ref(), &["addonQuota"]),
                json_path(account.auth_credit_usage_raw.as_ref(), &["add_on_quota"]),
                json_path(account.auth_user_plan_raw.as_ref(), &["addOnQuota"]),
                json_path(account.auth_user_plan_raw.as_ref(), &["addonQuota"]),
                json_path(account.auth_user_plan_raw.as_ref(), &["add_on_quota"]),
            ],
            QoderQuotaBucket::default(),
        );

        let shared_credit_root = [
            json_path(
                account.auth_credit_usage_raw.as_ref(),
                &["orgResourcePackage"],
            ),
            json_path(
                account.auth_credit_usage_raw.as_ref(),
                &["organizationResourcePackage"],
            ),
            json_path(
                account.auth_credit_usage_raw.as_ref(),
                &["sharedCreditPackage"],
            ),
            json_path(account.auth_credit_usage_raw.as_ref(), &["resourcePackage"]),
            json_path(account.auth_user_plan_raw.as_ref(), &["orgResourcePackage"]),
        ];
        let shared_credit_package_used =
            shared_credit_root.into_iter().flatten().find_map(|value| {
                json_path(Some(value), &["used"])
                    .or_else(|| json_path(Some(value), &["usage"]))
                    .or_else(|| json_path(Some(value), &["consumed"]))
                    .or_else(|| json_path(Some(value), &["count"]))
                    .and_then(parse_json_number)
            });

        QoderSubscriptionInfo {
            user_quota,
            add_on_quota,
            shared_credit_package_used,
            total_usage_percentage: first_number_from_roots(
                &roots,
                &[&["totalUsagePercentage"], &["total_usage_percentage"]],
            )
            .map(clamp_percent),
        }
    }

    fn build_trae_usage_summary(account: &crate::models::trae::TraeAccount) -> TraeUsageSummary {
        let Some(usage_root) = account
            .trae_usage_raw
            .as_ref()
            .and_then(|value| value.as_object())
        else {
            return TraeUsageSummary {
                reset_at: account.plan_reset_at,
                pay_as_you_go_open: Some(false),
                ..Default::default()
            };
        };

        if usage_root
            .get("code")
            .and_then(parse_json_number)
            .map(|value| value as i64)
            != Some(0)
        {
            return TraeUsageSummary {
                reset_at: account.plan_reset_at,
                pay_as_you_go_open: Some(false),
                ..Default::default()
            };
        }

        let packs = usage_root
            .get("user_entitlement_pack_list")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        if packs.is_empty() {
            return TraeUsageSummary {
                reset_at: account.plan_reset_at,
                pay_as_you_go_open: Some(false),
                ..Default::default()
            };
        }

        let product_type = |pack: &Value| -> i64 {
            json_path(Some(pack), &["entitlement_base_info", "product_type"])
                .or_else(|| json_path(Some(pack), &["product_type"]))
                .and_then(parse_json_number)
                .map(|value| value as i64)
                .unwrap_or(-1)
        };

        let valid_packs: Vec<Value> = packs
            .into_iter()
            .filter(|pack| product_type(pack) != 3)
            .collect();
        let find_pack = |target: i64| valid_packs.iter().find(|pack| product_type(pack) == target);

        let selected_pack = find_pack(6)
            .or_else(|| find_pack(4))
            .or_else(|| find_pack(1))
            .or_else(|| find_pack(9))
            .or_else(|| find_pack(8))
            .or_else(|| find_pack(0));
        let pay_go_pack = find_pack(7);

        let spent_usd = selected_pack
            .and_then(|pack| {
                json_path(Some(pack), &["usage", "basic_usage_amount"])
                    .or_else(|| json_path(Some(pack), &["usage", "basic_usage"]))
            })
            .and_then(parse_json_number)
            .unwrap_or(0.0);
        let total_usd = selected_pack
            .and_then(|pack| {
                json_path(
                    Some(pack),
                    &["entitlement_base_info", "quota", "basic_usage_limit"],
                )
                .or_else(|| {
                    json_path(
                        Some(pack),
                        &["entitlement_base_info", "quota", "basic_quota"],
                    )
                })
            })
            .and_then(parse_json_number)
            .unwrap_or(0.0);
        let reset_at = selected_pack
            .and_then(|pack| {
                json_path(Some(pack), &["entitlement_base_info", "quota_reset_time"])
                    .or_else(|| json_path(Some(pack), &["entitlement_base_info", "end_time"]))
                    .or_else(|| json_path(Some(pack), &["next_reset_time"]))
            })
            .and_then(parse_timestamp_like)
            .or(account.plan_reset_at);
        let pay_as_you_go_usd = pay_go_pack
            .and_then(|pack| {
                json_path(Some(pack), &["usage", "basic_usage_amount"])
                    .or_else(|| json_path(Some(pack), &["usage", "basic_usage"]))
            })
            .and_then(parse_json_number);

        TraeUsageSummary {
            used_percent: if total_usd > 0.0 {
                Some(clamp_percent((spent_usd / total_usd) * 100.0))
            } else {
                None
            },
            spent_usd: Some(spent_usd),
            total_usd: Some(total_usd),
            reset_at,
            pay_as_you_go_open: Some(pay_go_pack.is_some()),
            pay_as_you_go_usd,
        }
    }

    fn build_platform_cards(
        platform: PlatformId,
        lang: &str,
    ) -> (Vec<AccountCard>, Option<String>, Option<String>) {
        match platform {
            PlatformId::Antigravity => build_antigravity_cards(lang),
            PlatformId::Codex => build_codex_cards(lang),
            PlatformId::GitHubCopilot => build_ghcp_cards(lang),
            PlatformId::Windsurf => build_windsurf_cards(lang),
            PlatformId::Kiro => build_kiro_cards(lang),
            PlatformId::Cursor => build_cursor_cards(lang),
            PlatformId::Gemini => build_gemini_cards(lang),
            PlatformId::Qoder => build_qoder_cards(lang),
            PlatformId::Trae => build_trae_cards(lang),
            PlatformId::Codebuddy => build_codebuddy_cards(lang),
            PlatformId::CodebuddyCn => build_codebuddy_cn_cards(lang),
            PlatformId::Workbuddy => build_workbuddy_cards(lang),
            PlatformId::Zed => build_zed_cards(lang),
        }
    }

    fn build_antigravity_cards(lang: &str) -> (Vec<AccountCard>, Option<String>, Option<String>) {
        let accounts = modules::account::list_accounts().unwrap_or_default();
        let current_id = modules::account::get_current_account()
            .ok()
            .flatten()
            .map(|account| account.id);

        let mut sorted = accounts;
        sorted.sort_by_key(|account| std::cmp::Reverse(account.last_used.max(account.created_at)));

        let recommended = current_id
            .as_deref()
            .and_then(|id| modules::account::pick_quota_alert_recommendation(&sorted, id))
            .map(|account| account.id);

        let cards = sorted
            .into_iter()
            .map(|account| {
                let quota = account.quota.as_ref();
                AccountCard {
                    id: account.id,
                    title: account.email,
                    plan: resolve_antigravity_plan_label(quota),
                    updated_at: display_updated_at(
                        account.usage_updated_at,
                        account.last_used,
                        account.created_at,
                    ),
                    quota_rows: build_antigravity_quota_rows(lang, quota),
                }
            })
            .collect();

        (cards, current_id, recommended)
    }

    fn build_codex_cards(lang: &str) -> (Vec<AccountCard>, Option<String>, Option<String>) {
        let mut accounts = modules::codex_account::list_accounts();
        let current_id = modules::codex_account::resolve_current_account_id(&accounts);
        accounts
            .sort_by_key(|account| std::cmp::Reverse(account.last_used.max(account.created_at)));

        let recommended = current_id.as_deref().and_then(|id| {
            accounts
                .iter()
                .filter(|account| account.id != id && account.quota.is_some())
                .max_by(|left, right| {
                    let score_left = left
                        .quota
                        .as_ref()
                        .map(|quota| quota.hourly_percentage + quota.weekly_percentage)
                        .unwrap_or(-1);
                    let score_right = right
                        .quota
                        .as_ref()
                        .map(|quota| quota.hourly_percentage + quota.weekly_percentage)
                        .unwrap_or(-1);
                    score_left.cmp(&score_right)
                })
                .map(|account| account.id.clone())
        });

        let cards = accounts
            .into_iter()
            .map(|account| {
                let mut rows = Vec::new();
                if let Some(quota) = account.quota.as_ref() {
                    let has_presence_flags = quota.hourly_window_present.is_some()
                        || quota.weekly_window_present.is_some();
                    if !has_presence_flags || quota.hourly_window_present == Some(true) {
                        let percentage = quota.hourly_percentage.clamp(0, 100);
                        rows.push(make_progress_row(
                            format_codex_quota_metric_label(quota.hourly_window_minutes, "5h"),
                            format!("{percentage}%"),
                            percentage,
                            format_reset_subtext(lang, quota.hourly_reset_time),
                            codex_remaining_tone(percentage),
                        ));
                    }
                    if !has_presence_flags || quota.weekly_window_present == Some(true) {
                        let percentage = quota.weekly_percentage.clamp(0, 100);
                        rows.push(make_progress_row(
                            format_codex_quota_metric_label(quota.weekly_window_minutes, "Weekly"),
                            format!("{percentage}%"),
                            percentage,
                            format_reset_subtext(lang, quota.weekly_reset_time),
                            codex_remaining_tone(percentage),
                        ));
                    }
                    if let Some(mut code_review) = parse_code_review_metric(Some(quota)) {
                        code_review.label = "Code Review".to_string();
                        code_review.subtext = quota
                            .raw_data
                            .as_ref()
                            .and_then(|raw| raw.get("code_review_rate_limit"))
                            .and_then(|rate_limit| {
                                rate_limit
                                    .get("primary_window")
                                    .or_else(|| rate_limit.get("secondary_window"))
                            })
                            .and_then(|window| {
                                window
                                    .get("reset_at")
                                    .and_then(parse_json_number)
                                    .map(|value| value.floor() as i64)
                                    .or_else(|| {
                                        window
                                            .get("reset_after_seconds")
                                            .and_then(parse_json_number)
                                            .map(|value| {
                                                chrono::Utc::now().timestamp()
                                                    + value.floor() as i64
                                            })
                                    })
                            })
                            .and_then(|ts| format_reset_subtext(lang, Some(ts)));
                        rows.push(code_review);
                    }
                }
                AccountCard {
                    id: account.id,
                    title: if matches!(
                        account.auth_mode,
                        crate::models::codex::CodexAuthMode::Apikey
                    ) && account
                        .account_name
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .is_some()
                    {
                        account.account_name.unwrap_or(account.email)
                    } else {
                        account.email
                    },
                    plan: account.plan_type,
                    updated_at: display_updated_at(
                        account.usage_updated_at,
                        account.last_used,
                        account.created_at,
                    ),
                    quota_rows: rows,
                }
            })
            .collect();

        (cards, current_id, recommended)
    }

    fn build_ghcp_cards(lang: &str) -> (Vec<AccountCard>, Option<String>, Option<String>) {
        let mut accounts = modules::github_copilot_account::list_accounts();
        let current_id = modules::github_copilot_account::resolve_current_account_id(&accounts);
        accounts
            .sort_by_key(|account| std::cmp::Reverse(account.last_used.max(account.created_at)));
        let recommended = current_id.as_deref().and_then(|id| {
            accounts
                .iter()
                .filter(|account| account.id != id)
                .filter_map(|account| {
                    let metrics = modules::github_copilot_account::extract_quota_metrics(account);
                    if metrics.is_empty() {
                        return None;
                    }
                    let avg = metrics.iter().map(|(_, pct)| *pct).sum::<i32>() as f64
                        / metrics.len() as f64;
                    Some((account.id.clone(), avg, account.last_used))
                })
                .max_by(|left, right| {
                    left.1
                        .partial_cmp(&right.1)
                        .unwrap_or(Ordering::Equal)
                        .then_with(|| left.2.cmp(&right.2))
                })
                .map(|item| item.0)
        });
        let cards = accounts
            .into_iter()
            .map(|account| {
                let usage = compute_copilot_usage(
                    &account.copilot_token,
                    account.copilot_plan.as_deref(),
                    account.copilot_limited_user_quotas.as_ref(),
                    account.copilot_quota_snapshots.as_ref(),
                    account.copilot_limited_user_reset_date,
                    account.copilot_quota_reset_date.as_deref(),
                );
                let rows = vec![
                    quota_row_from_copilot_metric(
                        lang,
                        translate_or(
                            lang,
                            "common.shared.instances.quota.inline",
                            "Inline Suggestions",
                            &[],
                        ),
                        usage.inline,
                        usage.reset_ts,
                    ),
                    quota_row_from_copilot_metric(
                        lang,
                        translate_or(
                            lang,
                            "common.shared.instances.quota.chat",
                            "Chat messages",
                            &[],
                        ),
                        usage.chat,
                        usage.reset_ts,
                    ),
                    quota_row_from_copilot_metric(
                        lang,
                        translate_or(
                            lang,
                            "githubCopilot.columns.premium",
                            "Premium requests",
                            &[],
                        ),
                        usage.premium,
                        None,
                    ),
                ];
                AccountCard {
                    id: account.id,
                    title: account
                        .github_email
                        .clone()
                        .filter(|text| !text.trim().is_empty())
                        .unwrap_or(account.github_login),
                    plan: account.copilot_plan,
                    updated_at: display_updated_at(
                        account.usage_updated_at,
                        account.last_used,
                        account.created_at,
                    ),
                    quota_rows: rows,
                }
            })
            .collect();
        (cards, current_id, recommended)
    }

    fn build_windsurf_cards(lang: &str) -> (Vec<AccountCard>, Option<String>, Option<String>) {
        let mut accounts = modules::windsurf_account::list_accounts();
        let current_id = modules::windsurf_account::resolve_current_account_id(&accounts);
        accounts
            .sort_by_key(|account| std::cmp::Reverse(account.last_used.max(account.created_at)));
        let recommended = current_id.as_deref().and_then(|id| {
            accounts
                .iter()
                .filter(|account| account.id != id)
                .filter_map(|account| {
                    let metrics = modules::windsurf_account::extract_quota_metrics(account);
                    if metrics.is_empty() {
                        return None;
                    }
                    let avg = metrics.iter().map(|(_, pct)| *pct).sum::<i32>() as f64
                        / metrics.len() as f64;
                    Some((account.id.clone(), avg, account.last_used))
                })
                .max_by(|left, right| {
                    left.1
                        .partial_cmp(&right.1)
                        .unwrap_or(Ordering::Equal)
                        .then_with(|| left.2.cmp(&right.2))
                })
                .map(|item| item.0)
        });
        let cards = accounts
            .into_iter()
            .map(|account| {
                let rows = match resolve_windsurf_usage_mode(&account) {
                    WindsurfUsageMode::Quota => {
                        let summary = resolve_windsurf_quota_usage_summary(&account);
                        let mut rows = Vec::new();
                        if let Some(percentage) = summary.daily_used_percent {
                            rows.push(make_progress_row(
                                translate_or(
                                    lang,
                                    "windsurf.usageSummary.dailyQuota",
                                    "Daily quota usage",
                                    &[],
                                ),
                                format!("{percentage}%"),
                                percentage,
                                format_reset_subtext(lang, summary.daily_reset_ts),
                                usage_warning_tone(percentage),
                            ));
                        }
                        if let Some(percentage) = summary.weekly_used_percent {
                            rows.push(make_progress_row(
                                translate_or(
                                    lang,
                                    "windsurf.usageSummary.weeklyQuota",
                                    "Weekly quota usage",
                                    &[],
                                ),
                                format!("{percentage}%"),
                                percentage,
                                format_reset_subtext(lang, summary.weekly_reset_ts),
                                usage_warning_tone(percentage),
                            ));
                        }
                        rows.push(make_text_row(
                            translate_or(
                                lang,
                                "windsurf.usageSummary.extraUsageBalance",
                                "Extra usage balance",
                                &[],
                            ),
                            format_micros_usd(summary.overage_balance_micros.unwrap_or(0.0)),
                            None,
                        ));
                        rows
                    }
                    WindsurfUsageMode::Credits => {
                        let summary = resolve_windsurf_credits_summary(&account);
                        let cycle_text = format_reset_subtext(lang, summary.plan_end_ts);
                        let mut rows = Vec::new();
                        let credits_left_text = if let Some(value) = summary.credits_left {
                            let formatted = format_quota_number(value);
                            translate_or(
                                lang,
                                "windsurf.credits.left",
                                "{{value}} credits left",
                                &[("value", formatted.as_str())],
                            )
                        } else {
                            translate_or(
                                lang,
                                "windsurf.credits.leftUnknown",
                                "Credits left -",
                                &[],
                            )
                        };
                        rows.push(make_text_row(
                            translate_or(lang, "windsurf.credits.title", "Plan", &[]),
                            credits_left_text,
                            None,
                        ));

                        let prompt_progress = match (summary.prompt_total, summary.prompt_used) {
                            (Some(total), Some(used)) if total > 0.0 => {
                                clamp_percent((used / total) * 100.0)
                            }
                            _ => 0,
                        };
                        let prompt_value = match (summary.prompt_left, summary.prompt_total) {
                            (Some(left), Some(total)) if total > 0.0 => {
                                let remaining = format_quota_number(left);
                                let total_text = format_quota_number(total);
                                translate_or(
                                    lang,
                                    "windsurf.credits.promptLeft",
                                    "{{remaining}}/{{total}} prompt credits left",
                                    &[
                                        ("remaining", remaining.as_str()),
                                        ("total", total_text.as_str()),
                                    ],
                                )
                            }
                            (Some(left), _) if left > 0.0 => {
                                let remaining = format_quota_number(left);
                                translate_or(
                                    lang,
                                    "windsurf.credits.promptLeftNoTotal",
                                    "{{remaining}} prompt credits left",
                                    &[("remaining", remaining.as_str())],
                                )
                            }
                            _ => translate_or(
                                lang,
                                "windsurf.credits.promptLeftUnknown",
                                "Prompt credits left -",
                                &[],
                            ),
                        };
                        rows.push(make_progress_row(
                            translate_or(
                                lang,
                                "windsurf.credits.promptCreditsLeftLabel",
                                "prompt credits left",
                                &[],
                            ),
                            prompt_value,
                            prompt_progress,
                            cycle_text.clone(),
                            usage_warning_tone(prompt_progress),
                        ));

                        let add_on_progress = match (summary.add_on_total, summary.add_on_used) {
                            (Some(total), Some(used)) if total > 0.0 => {
                                clamp_percent((used / total) * 100.0)
                            }
                            _ => 0,
                        };
                        let add_on_left = format_quota_number(summary.add_on_left.unwrap_or(0.0));
                        rows.push(make_progress_row(
                            translate_or(
                                lang,
                                "windsurf.credits.addOnCreditsAvailableLabel",
                                "add-on credits available",
                                &[],
                            ),
                            translate_or(
                                lang,
                                "windsurf.credits.addOnAvailable",
                                "{{count}} add-on credits available",
                                &[("count", add_on_left.as_str())],
                            ),
                            add_on_progress,
                            cycle_text,
                            usage_warning_tone(add_on_progress),
                        ));
                        rows
                    }
                };
                AccountCard {
                    id: account.id,
                    title: account
                        .github_email
                        .clone()
                        .filter(|text| !text.trim().is_empty())
                        .unwrap_or(account.github_login),
                    plan: account.copilot_plan,
                    updated_at: display_updated_at(
                        account.usage_updated_at,
                        account.last_used,
                        account.created_at,
                    ),
                    quota_rows: rows,
                }
            })
            .collect();
        (cards, current_id, recommended)
    }

    fn build_kiro_cards(lang: &str) -> (Vec<AccountCard>, Option<String>, Option<String>) {
        let mut accounts = modules::kiro_account::list_accounts();
        let current_id = modules::kiro_account::resolve_current_account_id(&accounts);
        accounts
            .sort_by_key(|account| std::cmp::Reverse(account.last_used.max(account.created_at)));
        let recommended = current_id.as_deref().and_then(|id| {
            accounts
                .iter()
                .filter(|account| account.id != id)
                .filter(|account| !modules::kiro_account::is_banned_account(account))
                .filter_map(|account| {
                    let metrics = modules::kiro_account::extract_quota_metrics(account);
                    if metrics.is_empty() {
                        return None;
                    }
                    let avg = metrics.iter().map(|(_, pct)| *pct).sum::<i32>() as f64
                        / metrics.len() as f64;
                    Some((account.id.clone(), avg, account.last_used))
                })
                .max_by(|left, right| {
                    left.1
                        .partial_cmp(&right.1)
                        .unwrap_or(Ordering::Equal)
                        .then_with(|| left.2.cmp(&right.2))
                })
                .map(|item| item.0)
        });
        let cards = accounts
            .into_iter()
            .map(|account| {
                let mut rows = Vec::new();
                if let (Some(total), Some(used)) = (account.credits_total, account.credits_used) {
                    if total > 0.0 {
                        let percentage = clamp_percent((used / total) * 100.0);
                        rows.push(make_progress_row(
                            translate_or(
                                lang,
                                "common.shared.columns.promptCredits",
                                "User Prompt credits",
                                &[],
                            ),
                            format!("{percentage}%"),
                            percentage,
                            format_reset_subtext(lang, account.usage_reset_at),
                            usage_warning_tone(percentage),
                        ));
                    }
                }
                if let (Some(total), Some(used)) = (account.bonus_total, account.bonus_used) {
                    if total > 0.0 || used > 0.0 {
                        let percentage = if total > 0.0 {
                            clamp_percent((used / total) * 100.0)
                        } else {
                            0
                        };
                        rows.push(make_progress_row(
                            translate_or(
                                lang,
                                "common.shared.columns.addOnPromptCredits",
                                "Add-on prompt credits",
                                &[],
                            ),
                            format!("{percentage}%"),
                            percentage,
                            format_reset_subtext(lang, account.usage_reset_at),
                            usage_warning_tone(percentage),
                        ));
                    }
                }
                let account_id = account.id.clone();
                AccountCard {
                    id: account_id.clone(),
                    title: if account.email.trim().is_empty() {
                        account_id
                    } else {
                        account.email
                    },
                    plan: account.plan_name.or(account.plan_tier),
                    updated_at: display_updated_at(
                        account.usage_updated_at,
                        account.last_used,
                        account.created_at,
                    ),
                    quota_rows: rows,
                }
            })
            .collect();
        (cards, current_id, recommended)
    }

    fn build_cursor_cards(lang: &str) -> (Vec<AccountCard>, Option<String>, Option<String>) {
        let mut accounts = modules::cursor_account::list_accounts();
        let current_id = modules::cursor_account::resolve_current_account_id(&accounts);
        accounts
            .sort_by_key(|account| std::cmp::Reverse(account.last_used.max(account.created_at)));
        let recommended = current_id.as_deref().and_then(|id| {
            accounts
                .iter()
                .filter(|account| account.id != id)
                .filter(|account| !modules::cursor_account::is_banned_account(account))
                .filter_map(|account| {
                    let metrics = modules::cursor_account::extract_quota_metrics(account);
                    if metrics.is_empty() {
                        return None;
                    }
                    let avg = metrics.iter().map(|(_, pct)| *pct).sum::<i32>() as f64
                        / metrics.len() as f64;
                    Some((account.id.clone(), avg, account.last_used))
                })
                .max_by(|left, right| {
                    left.1
                        .partial_cmp(&right.1)
                        .unwrap_or(Ordering::Equal)
                        .then_with(|| left.2.cmp(&right.2))
                })
                .map(|item| item.0)
        });
        let cards = accounts
            .into_iter()
            .map(|account| {
                let usage = read_cursor_tray_usage(&account);
                let mut rows = Vec::new();
                if let Some(percentage) = usage.total_used_percent {
                    rows.push(make_progress_row(
                        "Total Usage".to_string(),
                        format!("{percentage}%"),
                        percentage,
                        format_reset_subtext(lang, usage.reset_ts),
                        cursor_usage_tone(percentage),
                    ));
                }
                if let Some(percentage) = usage.auto_used_percent {
                    rows.push(make_progress_row(
                        "Auto + Composer".to_string(),
                        format!("{percentage}%"),
                        percentage,
                        None,
                        cursor_usage_tone(percentage),
                    ));
                }
                if let Some(percentage) = usage.api_used_percent {
                    rows.push(make_progress_row(
                        "API Usage".to_string(),
                        format!("{percentage}%"),
                        percentage,
                        None,
                        cursor_usage_tone(percentage),
                    ));
                }
                if let Some(value) = usage.on_demand_text.clone() {
                    let progress = if value == "Unlimited" || value == "Disabled" {
                        None
                    } else {
                        usage.on_demand_percent
                    };
                    rows.push(QuotaRow {
                        label: translate_or(lang, "cursor.quota.onDemand", "On-Demand", &[]),
                        value: if value == "Unlimited" {
                            translate_or(lang, "common.shared.unlimited", "Unlimited", &[])
                        } else if value == "Disabled" {
                            translate_or(lang, "common.disabled", "Disabled", &[])
                        } else {
                            value
                        },
                        progress,
                        progress_tone: progress.map(cursor_usage_tone),
                        subtext: None,
                    });
                }
                let account_id = account.id.clone();
                AccountCard {
                    id: account_id.clone(),
                    title: if account.email.trim().is_empty() {
                        account_id
                    } else {
                        account.email
                    },
                    plan: account.membership_type,
                    updated_at: display_updated_at(
                        account.usage_updated_at,
                        account.last_used,
                        account.created_at,
                    ),
                    quota_rows: rows,
                }
            })
            .collect();
        (cards, current_id, recommended)
    }

    fn build_gemini_cards(lang: &str) -> (Vec<AccountCard>, Option<String>, Option<String>) {
        let mut accounts = modules::gemini_account::list_accounts();
        let current = modules::gemini_account::resolve_current_account(&accounts);
        let current_id = current.map(|account| account.id);
        accounts
            .sort_by_key(|account| std::cmp::Reverse(account.last_used.max(account.created_at)));
        let recommended = current_id.as_deref().and_then(|id| {
            accounts
                .iter()
                .filter(|account| account.id != id)
                .filter_map(|account| {
                    let metrics = modules::gemini_account::extract_account_model_remaining(account);
                    let lowest = metrics.iter().map(|(_, pct)| *pct).min()?;
                    Some((account.id.clone(), lowest))
                })
                .max_by_key(|item| item.1)
                .map(|item| item.0)
        });
        let cards = accounts
            .into_iter()
            .map(|account| {
                let buckets = collect_gemini_bucket_remaining(&account);
                let mut rows = Vec::new();
                if let Some(pro_bucket) =
                    pick_lowest_gemini_bucket(&buckets, |model_id| model_id.contains("pro"))
                {
                    let value = translate_or(
                        lang,
                        "gemini.quota.left",
                        "{{value}}% left",
                        &[("value", &pro_bucket.remaining_percent.to_string())],
                    );
                    rows.push(make_progress_row(
                        translate_or(lang, "gemini.quota.pro", "Pro", &[]),
                        value,
                        pro_bucket.remaining_percent,
                        format_reset_subtext(lang, pro_bucket.reset_at),
                        cursor_usage_tone((100 - pro_bucket.remaining_percent).clamp(0, 100)),
                    ));
                }
                if let Some(flash_bucket) =
                    pick_lowest_gemini_bucket(&buckets, |model_id| model_id.contains("flash"))
                {
                    let value = translate_or(
                        lang,
                        "gemini.quota.left",
                        "{{value}}% left",
                        &[("value", &flash_bucket.remaining_percent.to_string())],
                    );
                    rows.push(make_progress_row(
                        translate_or(lang, "gemini.quota.flash", "Flash", &[]),
                        value,
                        flash_bucket.remaining_percent,
                        format_reset_subtext(lang, flash_bucket.reset_at),
                        cursor_usage_tone((100 - flash_bucket.remaining_percent).clamp(0, 100)),
                    ));
                }
                AccountCard {
                    id: account.id,
                    title: account.email,
                    plan: account.plan_name.or(account.tier_id),
                    updated_at: display_updated_at(
                        account.usage_updated_at,
                        account.last_used,
                        account.created_at,
                    ),
                    quota_rows: rows,
                }
            })
            .collect();
        (cards, current_id, recommended)
    }

    fn build_qoder_cards(lang: &str) -> (Vec<AccountCard>, Option<String>, Option<String>) {
        let mut accounts = modules::qoder_account::list_accounts();
        let current_id = modules::qoder_account::resolve_current_account_id(&accounts);
        accounts
            .sort_by_key(|account| std::cmp::Reverse(account.last_used.max(account.created_at)));
        let cards = accounts
            .into_iter()
            .map(|account| {
                let subscription = build_qoder_subscription_info(&account);
                let mut rows = Vec::new();
                let remaining_percent = subscription
                    .total_usage_percentage
                    .map(|value| (100 - value).clamp(0, 100))
                    .or_else(|| {
                        match (
                            subscription.user_quota.remaining,
                            subscription.user_quota.total,
                        ) {
                            (Some(remaining), Some(total)) if total > 0.0 => {
                                Some(clamp_percent((remaining / total) * 100.0))
                            }
                            _ => None,
                        }
                    });
                let used_percent = remaining_percent.map(|value| (100 - value).clamp(0, 100));
                if remaining_percent.is_some()
                    || subscription.user_quota.total.is_some()
                    || subscription.user_quota.used.is_some()
                    || subscription.user_quota.remaining.is_some()
                {
                    let value = remaining_percent
                        .map(|value| {
                            translate_or(
                                lang,
                                "common.shared.remaining",
                                "剩余 {{value}}",
                                &[("value", format!("{value}%").as_str())],
                            )
                        })
                        .unwrap_or_else(|| "--".to_string());
                    let used = format_quota_number(subscription.user_quota.used.unwrap_or(0.0));
                    let total = format_quota_number(subscription.user_quota.total.unwrap_or(0.0));
                    rows.push(make_progress_row(
                        translate_or(
                            lang,
                            "qoder.usageOverview.includedCredits",
                            "套餐内 Credits",
                            &[],
                        ),
                        value,
                        used_percent.unwrap_or(0),
                        Some(translate_or(
                            lang,
                            "qoder.usageOverview.usedOfTotal",
                            "{{used}} / {{total}}",
                            &[("used", used.as_str()), ("total", total.as_str())],
                        )),
                        cursor_usage_tone(used_percent.unwrap_or(0)),
                    ));
                }

                if subscription.add_on_quota.total.unwrap_or(0.0) > 0.0
                    || subscription.add_on_quota.remaining.unwrap_or(0.0) > 0.0
                {
                    let total = subscription.add_on_quota.total.unwrap_or(0.0);
                    let remaining = subscription.add_on_quota.remaining.unwrap_or(0.0);
                    let remaining_percent = if total > 0.0 {
                        clamp_percent((remaining / total) * 100.0)
                    } else {
                        0
                    };
                    let remaining_text = format_quota_number(remaining);
                    let total_text = format_quota_number(total);
                    rows.push(make_progress_row(
                        translate_or(
                            lang,
                            "common.shared.columns.creditPackage",
                            "Credit Package",
                            &[],
                        ),
                        translate_or(
                            lang,
                            "qoder.usageOverview.usedOfTotal",
                            "{{used}} / {{total}}",
                            &[
                                ("used", remaining_text.as_str()),
                                ("total", total_text.as_str()),
                            ],
                        ),
                        remaining_percent,
                        None,
                        cursor_usage_tone((100 - remaining_percent).clamp(0, 100)),
                    ));
                }

                if let Some(shared_used) = subscription.shared_credit_package_used {
                    rows.push(make_text_row(
                        translate_or(
                            lang,
                            "common.shared.columns.sharedCreditPackage",
                            "Shared Credit Package",
                            &[],
                        ),
                        format_quota_number(shared_used),
                        None,
                    ));
                }
                AccountCard {
                    id: account.id.clone(),
                    title: account
                        .display_name
                        .clone()
                        .filter(|text| !text.trim().is_empty())
                        .unwrap_or(account.email),
                    plan: account.plan_type,
                    updated_at: display_updated_at(
                        account.usage_updated_at,
                        account.last_used,
                        account.created_at,
                    ),
                    quota_rows: rows,
                }
            })
            .collect();
        (cards, current_id, None)
    }

    fn build_trae_cards(lang: &str) -> (Vec<AccountCard>, Option<String>, Option<String>) {
        let mut accounts = modules::trae_account::list_accounts();
        let current_id = modules::trae_account::resolve_current_account_id(&accounts);
        accounts
            .sort_by_key(|account| std::cmp::Reverse(account.last_used.max(account.created_at)));
        let cards = accounts
            .into_iter()
            .map(|account| {
                let usage = build_trae_usage_summary(&account);
                let mut rows = Vec::new();
                if usage.used_percent.is_some()
                    || usage.spent_usd.is_some()
                    || usage.total_usd.is_some()
                    || usage.reset_at.is_some()
                {
                    let remaining = usage.used_percent.map(|value| (100 - value).clamp(0, 100));
                    let spent_text = format_currency_dollars(usage.spent_usd.unwrap_or(0.0));
                    let total_text = format_currency_dollars(usage.total_usd.unwrap_or(0.0));
                    rows.push(make_progress_row(
                        translate_or(lang, "trae.columns.usage", "Usage", &[]),
                        remaining
                            .map(|value| {
                                translate_or(
                                    lang,
                                    "common.shared.remaining",
                                    "剩余 {{value}}",
                                    &[("value", format!("{value}%").as_str())],
                                )
                            })
                            .unwrap_or_else(|| "--".to_string()),
                        usage.used_percent.unwrap_or(0),
                        Some(if usage.spent_usd.is_some() || usage.total_usd.is_some() {
                            translate_or(
                                lang,
                                "trae.quota.usedOfTotal",
                                "${{used}} / ${{total}}",
                                &[
                                    ("used", spent_text.trim_start_matches('$')),
                                    ("total", total_text.trim_start_matches('$')),
                                ],
                            )
                        } else {
                            format_reset_subtext(lang, usage.reset_at).unwrap_or_default()
                        }),
                        cursor_usage_tone(usage.used_percent.unwrap_or(0)),
                    ));
                }
                if let Some(opened) = usage.pay_as_you_go_open {
                    rows.push(make_text_row(
                        translate_or(lang, "trae.quota.payAsYouGoLabel", "On-Demand Usage", &[]),
                        usage
                            .pay_as_you_go_usd
                            .map(format_currency_dollars)
                            .unwrap_or_else(|| {
                                if opened {
                                    translate_or(lang, "common.enabled", "Enabled", &[])
                                } else {
                                    translate_or(lang, "common.disabled", "Disabled", &[])
                                }
                            }),
                        None,
                    ));
                }
                AccountCard {
                    id: account.id.clone(),
                    title: account
                        .nickname
                        .clone()
                        .filter(|text| !text.trim().is_empty())
                        .unwrap_or(account.email),
                    plan: account.plan_type,
                    updated_at: display_updated_at(
                        account.usage_updated_at,
                        account.last_used,
                        account.created_at,
                    ),
                    quota_rows: rows,
                }
            })
            .collect();
        (cards, current_id, None)
    }

    fn build_codebuddy_cards(lang: &str) -> (Vec<AccountCard>, Option<String>, Option<String>) {
        let mut accounts = modules::codebuddy_account::list_accounts();
        let current_id = modules::codebuddy_account::resolve_current_account_id(&accounts);
        accounts
            .sort_by_key(|account| std::cmp::Reverse(account.last_used.max(account.created_at)));
        let cards = accounts
            .into_iter()
            .map(|account| {
                let model = build_resource_quota_model(
                    account.quota_raw.as_ref(),
                    account.usage_raw.as_ref(),
                );
                let mut resources = model.resources.clone();
                if model.extra.total > 0.0 || model.extra.remain > 0.0 || model.extra.used > 0.0 {
                    resources.push(model.extra);
                }
                let rows = resources
                    .into_iter()
                    .filter(|resource| resource.total > 0.0 || resource.remain > 0.0)
                    .map(|resource| {
                        let used = format_quota_number(resource.used);
                        let total = format_quota_number(resource.total);
                        make_progress_row(
                            resolve_codebuddy_resource_label(lang, &resource),
                            translate_or(
                                lang,
                                "codebuddy.quota.usedOfTotal",
                                "{{used}} / {{total}}",
                                &[("used", used.as_str()), ("total", total.as_str())],
                            ),
                            resource.used_percent,
                            format_resource_time_text(
                                lang,
                                &resource,
                                "codebuddy.quotaQuery.updatedAt",
                                "codebuddy.quotaQuery.expireAt",
                            ),
                            resource_remaining_tone(&resource),
                        )
                    })
                    .collect();
                let title = account
                    .nickname
                    .clone()
                    .filter(|text| !text.trim().is_empty())
                    .unwrap_or_else(|| account.email.clone());
                AccountCard {
                    id: account.id.clone(),
                    title,
                    plan: Some(resolve_codebuddy_plan_badge(&account)),
                    updated_at: display_updated_at(
                        account.usage_updated_at,
                        account.last_used,
                        account.created_at,
                    ),
                    quota_rows: rows,
                }
            })
            .collect();
        (cards, current_id, None)
    }

    fn build_codebuddy_cn_cards(lang: &str) -> (Vec<AccountCard>, Option<String>, Option<String>) {
        let mut accounts = modules::codebuddy_cn_account::list_accounts();
        let current_id = modules::codebuddy_cn_account::resolve_current_account_id(&accounts);
        accounts
            .sort_by_key(|account| std::cmp::Reverse(account.last_used.max(account.created_at)));
        let cards = accounts
            .into_iter()
            .map(|account| {
                let model = build_resource_quota_model(
                    account.quota_raw.as_ref(),
                    account.usage_raw.as_ref(),
                );
                let mut resources = model.resources.clone();
                if model.extra.total > 0.0 || model.extra.remain > 0.0 || model.extra.used > 0.0 {
                    resources.push(model.extra);
                }
                let rows = resources
                    .into_iter()
                    .filter(|resource| resource.total > 0.0 || resource.remain > 0.0)
                    .map(|resource| {
                        let used = format_quota_number(resource.used);
                        let total = format_quota_number(resource.total);
                        make_progress_row(
                            resolve_codebuddy_resource_label(lang, &resource),
                            translate_or(
                                lang,
                                "codebuddy.quota.usedOfTotal",
                                "{{used}} / {{total}}",
                                &[("used", used.as_str()), ("total", total.as_str())],
                            ),
                            resource.used_percent,
                            format_resource_time_text(
                                lang,
                                &resource,
                                "codebuddy.quotaQuery.updatedAt",
                                "codebuddy.quotaQuery.expireAt",
                            ),
                            resource_remaining_tone(&resource),
                        )
                    })
                    .collect();
                let title = account
                    .nickname
                    .clone()
                    .filter(|text| !text.trim().is_empty())
                    .unwrap_or_else(|| account.email.clone());
                AccountCard {
                    id: account.id.clone(),
                    title,
                    plan: Some(resolve_codebuddy_plan_badge(&account)),
                    updated_at: display_updated_at(
                        account.usage_updated_at,
                        account.last_used,
                        account.created_at,
                    ),
                    quota_rows: rows,
                }
            })
            .collect();
        (cards, current_id, None)
    }

    fn build_workbuddy_cards(lang: &str) -> (Vec<AccountCard>, Option<String>, Option<String>) {
        let mut accounts = modules::workbuddy_account::list_accounts();
        let current_id = modules::workbuddy_account::resolve_current_account_id(&accounts);
        accounts
            .sort_by_key(|account| std::cmp::Reverse(account.last_used.max(account.created_at)));
        let cards = accounts
            .into_iter()
            .map(|account| {
                let model = build_resource_quota_model(
                    account.quota_raw.as_ref(),
                    account.usage_raw.as_ref(),
                );
                let mut resources = model.resources.clone();
                if model.extra.total > 0.0 || model.extra.remain > 0.0 || model.extra.used > 0.0 {
                    resources.push(model.extra);
                }
                let rows = resources
                    .into_iter()
                    .filter(|resource| resource.total > 0.0 || resource.remain > 0.0)
                    .map(|resource| {
                        let used = format_quota_number(resource.used);
                        let total = format_quota_number(resource.total);
                        make_progress_row(
                            resolve_workbuddy_resource_label(lang, &resource),
                            translate_or(
                                lang,
                                "workbuddy.quota.usedOfTotal",
                                "{{used}} / {{total}}",
                                &[("used", used.as_str()), ("total", total.as_str())],
                            ),
                            resource.used_percent,
                            format_resource_time_text(
                                lang,
                                &resource,
                                "workbuddy.quotaQuery.updatedAt",
                                "workbuddy.quotaQuery.expireAt",
                            ),
                            resource_remaining_tone(&resource),
                        )
                    })
                    .collect();
                let title = account
                    .nickname
                    .clone()
                    .filter(|text| !text.trim().is_empty())
                    .unwrap_or_else(|| account.email.clone());
                AccountCard {
                    id: account.id.clone(),
                    title,
                    plan: Some(resolve_workbuddy_plan_badge(&account)),
                    updated_at: display_updated_at(
                        account.usage_updated_at,
                        account.last_used,
                        account.created_at,
                    ),
                    quota_rows: rows,
                }
            })
            .collect();
        (cards, current_id, None)
    }

    fn build_zed_cards(lang: &str) -> (Vec<AccountCard>, Option<String>, Option<String>) {
        let mut accounts = modules::zed_account::list_accounts();
        let current_id = modules::zed_account::resolve_current_account_id();
        accounts
            .sort_by_key(|account| std::cmp::Reverse(account.last_used.max(account.created_at)));
        let recommended = current_id.as_deref().and_then(|id| {
            accounts
                .iter()
                .filter(|account| account.id != id)
                .filter_map(|account| {
                    let metrics = modules::zed_account::extract_quota_metrics(account);
                    if metrics.is_empty() {
                        return None;
                    }
                    let avg = metrics.iter().map(|(_, pct)| *pct).sum::<i32>() as f64
                        / metrics.len() as f64;
                    Some((account.id.clone(), avg, account.last_used))
                })
                .max_by(|left, right| {
                    left.1
                        .partial_cmp(&right.1)
                        .unwrap_or(Ordering::Equal)
                        .then_with(|| left.2.cmp(&right.2))
                })
                .map(|item| item.0)
        });
        let cards = accounts
            .into_iter()
            .map(|account| {
                let mut rows = Vec::new();
                if account.edit_predictions_used.is_some()
                    || account
                        .edit_predictions_limit_raw
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .is_some()
                {
                    let used = account.edit_predictions_used.unwrap_or(0).max(0);
                    let total = account
                        .edit_predictions_limit_raw
                        .as_deref()
                        .and_then(|value| value.trim().parse::<f64>().ok())
                        .filter(|value| value.is_finite() && *value >= 0.0)
                        .unwrap_or(0.0);
                    let progress = if total > 0.0 {
                        clamp_percent((used as f64 / total) * 100.0)
                    } else {
                        0
                    };
                    rows.push(make_progress_row(
                        translate_or(lang, "zed.page.editPredictions", "Edit Predictions", &[]),
                        format!("{used} / {}", format_quota_number(total)),
                        progress,
                        None,
                        remaining_balance_tone((100 - progress).clamp(0, 100)),
                    ));
                }
                if let Some(overdue) = account.has_overdue_invoices {
                    rows.push(make_text_row(
                        translate_or(lang, "zed.page.overdueField", "Overdue", &[]),
                        if overdue {
                            translate_or(lang, "zed.page.overdueYes", "Yes", &[])
                        } else {
                            translate_or(lang, "zed.page.overdueNo", "No", &[])
                        },
                        None,
                    ));
                }
                AccountCard {
                    id: account.id.clone(),
                    title: account
                        .display_name
                        .clone()
                        .filter(|text| !text.trim().is_empty())
                        .unwrap_or(account.github_login),
                    plan: account
                        .plan_raw
                        .as_deref()
                        .map(format_zed_plan_label)
                        .filter(|value| !value.is_empty()),
                    updated_at: display_updated_at(
                        account.usage_updated_at,
                        account.last_used,
                        account.created_at,
                    ),
                    quota_rows: rows,
                }
            })
            .collect();
        (cards, current_id, recommended)
    }

    fn spawn_refresh(platform: PlatformId, account_id: Option<String>) {
        let Some(app) = crate::get_app_handle().cloned() else {
            return;
        };

        tauri::async_runtime::spawn(async move {
            let refresh_result = match (platform, account_id) {
                (PlatformId::Antigravity, Some(account_id)) => {
                    commands::account::fetch_account_quota(account_id)
                        .await
                        .map(|_| 0)
                        .map_err(|err| err.to_string())
                }
                (PlatformId::Antigravity, None) => {
                    commands::account::refresh_current_quota(app.clone())
                        .await
                        .map(|_| 0)
                }
                (PlatformId::Codex, Some(account_id)) => {
                    commands::codex::refresh_codex_quota(app.clone(), account_id)
                        .await
                        .map(|_| 0)
                }
                (PlatformId::Codex, None) => {
                    commands::codex::refresh_current_codex_quota(app.clone())
                        .await
                        .map(|_| 0)
                }
                (PlatformId::GitHubCopilot, Some(account_id)) => {
                    commands::github_copilot::refresh_github_copilot_token(app.clone(), account_id)
                        .await
                        .map(|_| 0)
                }
                (PlatformId::GitHubCopilot, None) => {
                    commands::github_copilot::refresh_all_github_copilot_tokens(app.clone()).await
                }
                (PlatformId::Windsurf, Some(account_id)) => {
                    commands::windsurf::refresh_windsurf_token(app.clone(), account_id)
                        .await
                        .map(|_| 0)
                }
                (PlatformId::Windsurf, None) => {
                    commands::windsurf::refresh_all_windsurf_tokens(app.clone()).await
                }
                (PlatformId::Kiro, Some(account_id)) => {
                    commands::kiro::refresh_kiro_token(app.clone(), account_id)
                        .await
                        .map(|_| 0)
                }
                (PlatformId::Kiro, None) => {
                    commands::kiro::refresh_all_kiro_tokens(app.clone()).await
                }
                (PlatformId::Cursor, Some(account_id)) => {
                    commands::cursor::refresh_cursor_token(app.clone(), account_id)
                        .await
                        .map(|_| 0)
                }
                (PlatformId::Cursor, None) => {
                    commands::cursor::refresh_all_cursor_tokens(app.clone()).await
                }
                (PlatformId::Gemini, Some(account_id)) => {
                    commands::gemini::refresh_gemini_token(app.clone(), account_id)
                        .await
                        .map(|_| 0)
                }
                (PlatformId::Gemini, None) => {
                    commands::gemini::refresh_all_gemini_tokens(app.clone()).await
                }
                (PlatformId::Codebuddy, Some(account_id)) => {
                    commands::codebuddy::refresh_codebuddy_token(app.clone(), account_id)
                        .await
                        .map(|_| 0)
                }
                (PlatformId::Codebuddy, None) => {
                    commands::codebuddy::refresh_all_codebuddy_tokens(app.clone()).await
                }
                (PlatformId::CodebuddyCn, Some(account_id)) => {
                    commands::codebuddy_cn::refresh_codebuddy_cn_token(app.clone(), account_id)
                        .await
                        .map(|_| 0)
                }
                (PlatformId::CodebuddyCn, None) => {
                    commands::codebuddy_cn::refresh_all_codebuddy_cn_tokens(app.clone()).await
                }
                (PlatformId::Qoder, Some(account_id)) => {
                    commands::qoder::refresh_qoder_token(app.clone(), account_id)
                        .await
                        .map(|_| 0)
                }
                (PlatformId::Qoder, None) => {
                    commands::qoder::refresh_all_qoder_tokens(app.clone()).await
                }
                (PlatformId::Trae, Some(account_id)) => {
                    commands::trae::refresh_trae_token(app.clone(), account_id)
                        .await
                        .map(|_| 0)
                }
                (PlatformId::Trae, None) => {
                    commands::trae::refresh_all_trae_tokens(app.clone()).await
                }
                (PlatformId::Workbuddy, Some(account_id)) => {
                    commands::workbuddy::refresh_workbuddy_token(app.clone(), account_id)
                        .await
                        .map(|_| 0)
                }
                (PlatformId::Workbuddy, None) => {
                    commands::workbuddy::refresh_all_workbuddy_tokens(app.clone()).await
                }
                (PlatformId::Zed, Some(account_id)) => {
                    commands::zed::refresh_zed_token(app.clone(), account_id)
                        .await
                        .map(|_| 0)
                }
                (PlatformId::Zed, None) => commands::zed::refresh_all_zed_tokens(app.clone()).await,
            };
            let _ = refresh_result;
            refresh_native_menu_snapshot();
        });
    }

    fn refresh_native_menu_snapshot() {
        let Some(snapshot) = build_snapshot() else {
            return;
        };
        let Ok(snapshot_json) = serde_json::to_string(&snapshot) else {
            return;
        };
        let snapshot_json = to_cstring(&snapshot_json);
        unsafe {
            macos_native_menu_update_snapshot(snapshot_json.as_ptr());
        }
    }

    fn spawn_switch_account(platform: PlatformId, account_id: String) {
        let Some(app) = crate::get_app_handle().cloned() else {
            return;
        };

        tauri::async_runtime::spawn(async move {
            let _ = match platform {
                PlatformId::Antigravity => commands::account::switch_account(app, account_id, None)
                    .await
                    .map(|_| ()),
                PlatformId::Codex => commands::codex::switch_codex_account(app, account_id)
                    .await
                    .map(|_| ()),
                PlatformId::GitHubCopilot => {
                    commands::github_copilot::inject_github_copilot_to_vscode(app, account_id)
                        .await
                        .map(|_| ())
                }
                PlatformId::Windsurf => {
                    commands::windsurf::inject_windsurf_to_vscode(app, account_id)
                        .await
                        .map(|_| ())
                }
                PlatformId::Kiro => commands::kiro::inject_kiro_to_vscode(app, account_id)
                    .await
                    .map(|_| ()),
                PlatformId::Cursor => commands::cursor::inject_cursor_account(app, account_id)
                    .await
                    .map(|_| ()),
                PlatformId::Gemini => {
                    commands::gemini::inject_gemini_account(app, account_id).map(|_| ())
                }
                PlatformId::Codebuddy => {
                    commands::codebuddy::inject_codebuddy_to_vscode(app, account_id)
                        .await
                        .map(|_| ())
                }
                PlatformId::CodebuddyCn => {
                    commands::codebuddy_cn::inject_codebuddy_cn_to_vscode(app, account_id)
                        .await
                        .map(|_| ())
                }
                PlatformId::Qoder => commands::qoder::inject_qoder_account(app, account_id)
                    .await
                    .map(|_| ()),
                PlatformId::Trae => commands::trae::inject_trae_account(app, account_id)
                    .await
                    .map(|_| ()),
                PlatformId::Workbuddy => {
                    commands::workbuddy::inject_workbuddy_to_vscode(app, account_id)
                        .await
                        .map(|_| ())
                }
                PlatformId::Zed => commands::zed::inject_zed_account(app, account_id)
                    .await
                    .map(|_| ()),
            };
        });
    }

    fn open_main_window_page(page: &str) {
        if let Some(app) = crate::get_app_handle() {
            let _ = modules::floating_card_window::show_main_window_and_navigate(app, page);
        }
    }

    fn open_main_window() {
        if let Some(app) = crate::get_app_handle() {
            let _ = modules::floating_card_window::show_main_window(app);
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) use imp::toggle_tray_menu;
