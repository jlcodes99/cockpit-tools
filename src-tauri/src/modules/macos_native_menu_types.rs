// macOS Native Menu：Native menu FFI declarations, snapshots and shared data types。
// 通过 include! 保持原 imp 模块作用域和 Objective-C FFI 调用路径。
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
        fn macos_native_menu_update_status_item(
            account_prefix: *const c_char,
            value_text: *const c_char,
            remaining_percent: i32,
            enabled: i32,
            status_item_ptr: *mut c_void,
        );
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
        /// 为 true 时右键打开菜单应强制选中 selected_platform_id（菜单栏额度配置的平台）并展示当前账号。
        #[serde(default)]
        prefer_selected_platform: bool,
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
        remaining_percent: Option<i32>,
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

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct MenuBarStatus {
        /// 前缀：邮箱前 4 位 / "API" / 空
        account_prefix: String,
        /// 主数值文案：如 "45%" / "$12.50" / "--"
        value_text: String,
        /// 用于着色的剩余百分比；无则灰色
        remaining_percent: Option<i32>,
    }

