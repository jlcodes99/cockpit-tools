//! 系统托盘模块
//! 管理系统托盘图标和菜单

#[cfg(not(target_os = "macos"))]
use std::collections::{HashMap, HashSet};
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(not(target_os = "macos"))]
use tauri::menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    Emitter, Runtime,
};
use tracing::info;

use crate::modules::logger;

/// 托盘菜单 ID
pub const TRAY_ID: &str = "main-tray";

#[cfg(target_os = "macos")]
static MACOS_TRAY_SKIP_LOGGED: AtomicBool = AtomicBool::new(false);

/// 单层最多直出的平台数量（超出进入“更多平台”子菜单）
#[cfg(not(target_os = "macos"))]
const TRAY_PLATFORM_MAX_VISIBLE: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PlatformId {
    Antigravity,
    Codex,
    Zed,
    GitHubCopilot,
    Windsurf,
    Kiro,
    Cursor,
    Gemini,
    Codebuddy,
    CodebuddyCn,
    Qoder,
    Trae,
    Workbuddy,
}

impl PlatformId {
    pub(crate) fn default_order() -> [Self; 13] {
        [
            Self::Antigravity,
            Self::Codex,
            Self::Zed,
            Self::GitHubCopilot,
            Self::Windsurf,
            Self::Kiro,
            Self::Cursor,
            Self::Gemini,
            Self::Codebuddy,
            Self::CodebuddyCn,
            Self::Qoder,
            Self::Trae,
            Self::Workbuddy,
        ]
    }

    pub(crate) fn from_str(value: &str) -> Option<Self> {
        match value {
            crate::modules::tray_layout::PLATFORM_ANTIGRAVITY => Some(Self::Antigravity),
            crate::modules::tray_layout::PLATFORM_CODEX => Some(Self::Codex),
            crate::modules::tray_layout::PLATFORM_ZED => Some(Self::Zed),
            crate::modules::tray_layout::PLATFORM_GITHUB_COPILOT => Some(Self::GitHubCopilot),
            crate::modules::tray_layout::PLATFORM_WINDSURF => Some(Self::Windsurf),
            crate::modules::tray_layout::PLATFORM_KIRO => Some(Self::Kiro),
            crate::modules::tray_layout::PLATFORM_CURSOR => Some(Self::Cursor),
            crate::modules::tray_layout::PLATFORM_GEMINI => Some(Self::Gemini),
            crate::modules::tray_layout::PLATFORM_CODEBUDDY => Some(Self::Codebuddy),
            crate::modules::tray_layout::PLATFORM_CODEBUDDY_CN => Some(Self::CodebuddyCn),
            crate::modules::tray_layout::PLATFORM_QODER => Some(Self::Qoder),
            crate::modules::tray_layout::PLATFORM_TRAE => Some(Self::Trae),
            crate::modules::tray_layout::PLATFORM_WORKBUDDY => Some(Self::Workbuddy),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Antigravity => crate::modules::tray_layout::PLATFORM_ANTIGRAVITY,
            Self::Codex => crate::modules::tray_layout::PLATFORM_CODEX,
            Self::Zed => crate::modules::tray_layout::PLATFORM_ZED,
            Self::GitHubCopilot => crate::modules::tray_layout::PLATFORM_GITHUB_COPILOT,
            Self::Windsurf => crate::modules::tray_layout::PLATFORM_WINDSURF,
            Self::Kiro => crate::modules::tray_layout::PLATFORM_KIRO,
            Self::Cursor => crate::modules::tray_layout::PLATFORM_CURSOR,
            Self::Gemini => crate::modules::tray_layout::PLATFORM_GEMINI,
            Self::Codebuddy => crate::modules::tray_layout::PLATFORM_CODEBUDDY,
            Self::CodebuddyCn => crate::modules::tray_layout::PLATFORM_CODEBUDDY_CN,
            Self::Qoder => crate::modules::tray_layout::PLATFORM_QODER,
            Self::Trae => crate::modules::tray_layout::PLATFORM_TRAE,
            Self::Workbuddy => crate::modules::tray_layout::PLATFORM_WORKBUDDY,
        }
    }

    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Antigravity => "Antigravity",
            Self::Codex => "Codex",
            Self::Zed => "Zed",
            Self::GitHubCopilot => "GitHub Copilot",
            Self::Windsurf => "Windsurf",
            Self::Kiro => "Kiro",
            Self::Cursor => "Cursor",
            Self::Gemini => "Gemini Cli",
            Self::Codebuddy => "CodeBuddy",
            Self::CodebuddyCn => "CodeBuddy CN",
            Self::Qoder => "Qoder",
            Self::Trae => "Trae",
            Self::Workbuddy => "WorkBuddy",
        }
    }

    pub(crate) fn nav_target(self) -> &'static str {
        match self {
            Self::Antigravity => "overview",
            Self::Codex => "codex",
            Self::Zed => "zed",
            Self::GitHubCopilot => "github-copilot",
            Self::Windsurf => "windsurf",
            Self::Kiro => "kiro",
            Self::Cursor => "cursor",
            Self::Gemini => "gemini",
            Self::Codebuddy => "codebuddy",
            Self::CodebuddyCn => "codebuddy-cn",
            Self::Qoder => "qoder",
            Self::Trae => "trae",
            Self::Workbuddy => "workbuddy",
        }
    }
}

/// 菜单项 ID
pub mod menu_ids {
    pub const SHOW_WINDOW: &str = "show_window";
    pub const SHOW_FLOATING_CARD: &str = "show_floating_card";
    pub const REFRESH_QUOTA: &str = "refresh_quota";
    pub const SETTINGS: &str = "settings";
    pub const QUIT: &str = "quit";
}

/// 账号显示信息
#[cfg(not(target_os = "macos"))]
struct AccountDisplayInfo {
    account: String,
    quota_lines: Vec<String>,
}

#[derive(Debug, Clone)]
#[cfg(not(target_os = "macos"))]
enum TrayMenuEntry {
    Platform(PlatformId),
    Group {
        id: String,
        name: String,
        platforms: Vec<PlatformId>,
    },
}

#[derive(Debug, Clone, Copy)]
#[cfg(not(target_os = "macos"))]
struct CopilotMetric {
    used_percent: Option<i32>,
    included: bool,
}

#[derive(Debug, Clone, Copy)]
#[cfg(not(target_os = "macos"))]
struct CopilotUsage {
    inline: CopilotMetric,
    chat: CopilotMetric,
    premium: CopilotMetric,
    reset_ts: Option<i64>,
}

/// 创建系统托盘（完整菜单，包含账号数据加载）
/// 创建骨架托盘（无账号文件 I/O，仅基础菜单项，用于快速启动）
pub fn create_tray_skeleton<R: Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<TrayIcon<R>, tauri::Error> {
    info!("[Tray] создание базового трей-меню...");

    #[cfg(not(target_os = "macos"))]
    let lang = crate::modules::config::get_user_config().language;

    #[cfg(not(target_os = "macos"))]
    let show_window = MenuItem::with_id(
        app,
        menu_ids::SHOW_WINDOW,
        get_text("show_window", &lang),
        true,
        None::<&str>,
    )?;
    #[cfg(not(target_os = "macos"))]
    let refresh_quota = MenuItem::with_id(
        app,
        menu_ids::REFRESH_QUOTA,
        get_text("refresh_quota", &lang),
        true,
        None::<&str>,
    )?;
    #[cfg(not(target_os = "macos"))]
    let show_floating_card = MenuItem::with_id(
        app,
        menu_ids::SHOW_FLOATING_CARD,
        get_text("show_floating_card", &lang),
        true,
        None::<&str>,
    )?;
    #[cfg(not(target_os = "macos"))]
    let settings = MenuItem::with_id(
        app,
        menu_ids::SETTINGS,
        get_text("settings", &lang),
        true,
        None::<&str>,
    )?;
    #[cfg(not(target_os = "macos"))]
    let quit = MenuItem::with_id(
        app,
        menu_ids::QUIT,
        get_text("quit", &lang),
        true,
        None::<&str>,
    )?;
    #[cfg(not(target_os = "macos"))]
    let loading = MenuItem::with_id(
        app,
        "tray_loading",
        get_text("loading", &lang),
        false,
        None::<&str>,
    )?;

    #[cfg(not(target_os = "macos"))]
    let menu = {
        let menu = Menu::new(app)?;
        menu.append(&show_window)?;
        menu.append(&show_floating_card)?;
        menu.append(&PredefinedMenuItem::separator(app)?)?;
        menu.append(&loading)?;
        menu.append(&PredefinedMenuItem::separator(app)?)?;
        menu.append(&refresh_quota)?;
        menu.append(&settings)?;
        menu.append(&PredefinedMenuItem::separator(app)?)?;
        menu.append(&quit)?;
        menu
    };

    let builder = TrayIconBuilder::with_id(TRAY_ID)
        .icon(app.default_window_icon().unwrap().clone())
        .show_menu_on_left_click(false)
        .tooltip("Cockpit Tools")
        .on_menu_event(handle_menu_event)
        .on_tray_icon_event(handle_tray_event);

    #[cfg(not(target_os = "macos"))]
    let builder = builder.menu(&menu);

    let tray = builder.build(app)?;

    #[cfg(target_os = "macos")]
    let _ = tray.set_show_menu_on_left_click(false);

    info!("[Tray] базовое трей-меню создано, жду фоновую загрузку полного меню");
    Ok(tray)
}

/// 构建托盘菜单
#[cfg(not(target_os = "macos"))]
fn build_tray_menu<R: Runtime>(app: &tauri::AppHandle<R>) -> Result<Menu<R>, tauri::Error> {
    let config = crate::modules::config::get_user_config();
    let lang = &config.language;

    let show_window = MenuItem::with_id(
        app,
        menu_ids::SHOW_WINDOW,
        get_text("show_window", lang),
        true,
        None::<&str>,
    )?;
    let refresh_quota = MenuItem::with_id(
        app,
        menu_ids::REFRESH_QUOTA,
        get_text("refresh_quota", lang),
        true,
        None::<&str>,
    )?;
    let show_floating_card = MenuItem::with_id(
        app,
        menu_ids::SHOW_FLOATING_CARD,
        get_text("show_floating_card", lang),
        true,
        None::<&str>,
    )?;
    let settings = MenuItem::with_id(
        app,
        menu_ids::SETTINGS,
        get_text("settings", lang),
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(
        app,
        menu_ids::QUIT,
        get_text("quit", lang),
        true,
        None::<&str>,
    )?;

    let ordered_entries = resolve_tray_entries();
    let split_index = ordered_entries.len().min(TRAY_PLATFORM_MAX_VISIBLE);
    let (visible_entries, overflow_entries) = ordered_entries.split_at(split_index);

    let mut visible_submenus: Vec<Submenu<R>> = Vec::new();
    for entry in visible_entries {
        visible_submenus.push(build_tray_entry_submenu(app, entry, lang)?);
    }

    let mut overflow_submenus: Vec<Submenu<R>> = Vec::new();
    for entry in overflow_entries {
        overflow_submenus.push(build_tray_entry_submenu(app, entry, lang)?);
    }

    let overflow_refs: Vec<&dyn IsMenuItem<R>> = overflow_submenus
        .iter()
        .map(|submenu| submenu as &dyn IsMenuItem<R>)
        .collect();
    let more_platforms_submenu = if overflow_refs.is_empty() {
        None
    } else {
        Some(Submenu::with_id_and_items(
            app,
            "tray_more_platforms",
            get_text("more_platforms", lang),
            true,
            &overflow_refs,
        )?)
    };

    let no_platform_item = if visible_submenus.is_empty() && overflow_submenus.is_empty() {
        Some(MenuItem::with_id(
            app,
            "tray_no_platform_selected",
            get_text("no_platform_selected", lang),
            true,
            None::<&str>,
        )?)
    } else {
        None
    };

    let menu = Menu::with_id(app, "tray_menu")?;
    menu.append(&show_window)?;
    menu.append(&show_floating_card)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;

    if let Some(item) = &no_platform_item {
        menu.append(item)?;
    } else {
        for submenu in &visible_submenus {
            menu.append(submenu)?;
        }
        if let Some(submenu) = &more_platforms_submenu {
            menu.append(submenu)?;
        }
    }

    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&refresh_quota)?;
    menu.append(&settings)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&quit)?;
    Ok(menu)
}

#[cfg(not(target_os = "macos"))]
fn build_tray_entry_submenu<R: Runtime>(
    app: &tauri::AppHandle<R>,
    entry: &TrayMenuEntry,
    lang: &str,
) -> Result<Submenu<R>, tauri::Error> {
    match entry {
        TrayMenuEntry::Platform(platform) => build_platform_submenu(app, *platform, lang),
        TrayMenuEntry::Group {
            id,
            name,
            platforms,
        } => build_platform_group_submenu(app, id, name, platforms, lang),
    }
}

#[cfg(not(target_os = "macos"))]
fn build_platform_group_submenu<R: Runtime>(
    app: &tauri::AppHandle<R>,
    group_id: &str,
    group_name: &str,
    platforms: &[PlatformId],
    lang: &str,
) -> Result<Submenu<R>, tauri::Error> {
    if let [platform] = platforms {
        return build_platform_details_submenu(
            app,
            &format!("group:{}:submenu", group_id),
            group_name,
            *platform,
            lang,
        );
    }

    let mut submenus: Vec<Submenu<R>> = Vec::new();
    for platform in platforms {
        submenus.push(build_platform_submenu(app, *platform, lang)?);
    }

    let refs: Vec<&dyn IsMenuItem<R>> = submenus
        .iter()
        .map(|submenu| submenu as &dyn IsMenuItem<R>)
        .collect();

    Submenu::with_id_and_items(
        app,
        format!("group:{}:submenu", group_id),
        group_name,
        true,
        &refs,
    )
}

#[cfg(not(target_os = "macos"))]
fn resolve_tray_entries() -> Vec<TrayMenuEntry> {
    let layout = crate::modules::tray_layout::load_tray_layout();
    let visible = sanitize_platform_list(&layout.tray_platform_ids);
    let visible_set: HashSet<PlatformId> = visible.iter().copied().collect();

    if visible_set.is_empty() {
        return Vec::new();
    }

    let mut groups_by_id: HashMap<String, crate::modules::tray_layout::TrayLayoutGroup> =
        HashMap::new();
    for group in layout.platform_groups {
        groups_by_id.insert(group.id.clone(), group);
    }

    let mut entries = Vec::new();
    let mut used_platforms: HashSet<PlatformId> = HashSet::new();

    for raw_entry in &layout.ordered_entry_ids {
        if let Some(platform) = parse_platform_entry_id(raw_entry) {
            if !visible_set.contains(&platform) || !used_platforms.insert(platform) {
                continue;
            }
            entries.push(TrayMenuEntry::Platform(platform));
            continue;
        }

        let Some(group_id) = parse_group_entry_id(raw_entry) else {
            continue;
        };
        let Some(group) = groups_by_id.get(&group_id) else {
            continue;
        };

        let mut group_platforms: Vec<PlatformId> = Vec::new();
        for raw_platform in &group.platform_ids {
            let Some(platform) = PlatformId::from_str(raw_platform.trim()) else {
                continue;
            };
            if !visible_set.contains(&platform) || !used_platforms.insert(platform) {
                continue;
            }
            group_platforms.push(platform);
        }

        if group_platforms.is_empty() {
            continue;
        }

        let group_name = if group.name.trim().is_empty() {
            group.id.clone()
        } else {
            group.name.clone()
        };

        entries.push(TrayMenuEntry::Group {
            id: group.id.clone(),
            name: group_name,
            platforms: group_platforms,
        });
    }

    for platform in normalize_platform_order(&layout.ordered_platform_ids) {
        if !visible_set.contains(&platform) || !used_platforms.insert(platform) {
            continue;
        }
        entries.push(TrayMenuEntry::Platform(platform));
    }

    entries
}

#[cfg(not(target_os = "macos"))]
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

#[cfg(not(target_os = "macos"))]
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

#[cfg(not(target_os = "macos"))]
fn parse_platform_entry_id(raw: &str) -> Option<PlatformId> {
    let value = raw.strip_prefix("platform:")?;
    PlatformId::from_str(value.trim())
}

#[cfg(not(target_os = "macos"))]
fn parse_group_entry_id(raw: &str) -> Option<String> {
    let value = raw.strip_prefix("group:")?.trim();
    if value.is_empty() {
        return None;
    }
    Some(value.to_string())
}

#[cfg(not(target_os = "macos"))]
fn build_platform_submenu<R: Runtime>(
    app: &tauri::AppHandle<R>,
    platform: PlatformId,
    lang: &str,
) -> Result<Submenu<R>, tauri::Error> {
    build_platform_details_submenu(
        app,
        &format!("platform:{}:submenu", platform.as_str()),
        platform.title(),
        platform,
        lang,
    )
}

#[cfg(not(target_os = "macos"))]
fn build_platform_details_submenu<R: Runtime>(
    app: &tauri::AppHandle<R>,
    submenu_id: &str,
    title: &str,
    platform: PlatformId,
    lang: &str,
) -> Result<Submenu<R>, tauri::Error> {
    let info = get_account_display_info(platform, lang);
    let mut items: Vec<MenuItem<R>> = Vec::new();

    items.push(MenuItem::with_id(
        app,
        format!("platform:{}:account", platform.as_str()),
        info.account,
        true,
        None::<&str>,
    )?);

    for (idx, line) in info.quota_lines.iter().enumerate() {
        items.push(MenuItem::with_id(
            app,
            format!("platform:{}:quota:{}", platform.as_str(), idx),
            line,
            true,
            None::<&str>,
        )?);
    }

    let refs: Vec<&dyn IsMenuItem<R>> = items
        .iter()
        .map(|item| item as &dyn IsMenuItem<R>)
        .collect();

    Submenu::with_id_and_items(app, submenu_id, title, true, &refs)
}

#[cfg(not(target_os = "macos"))]
fn get_account_display_info(platform: PlatformId, lang: &str) -> AccountDisplayInfo {
    match platform {
        PlatformId::Antigravity => build_antigravity_display_info(lang),
        PlatformId::Codex => build_codex_display_info(lang),
        PlatformId::Zed => build_zed_display_info(lang),
        PlatformId::GitHubCopilot => build_github_copilot_display_info(lang),
        PlatformId::Windsurf => build_windsurf_display_info(lang),
        PlatformId::Kiro => build_kiro_display_info(lang),
        PlatformId::Cursor => build_cursor_display_info(lang),
        PlatformId::Gemini => build_gemini_display_info(lang),
        PlatformId::Codebuddy => build_codebuddy_display_info(lang),
        PlatformId::CodebuddyCn => build_codebuddy_cn_display_info(lang),
        PlatformId::Qoder => build_qoder_display_info(lang),
        PlatformId::Trae => build_trae_display_info(lang),
        PlatformId::Workbuddy => build_workbuddy_display_info(lang),
    }
}

#[cfg(not(target_os = "macos"))]
fn build_antigravity_display_info(lang: &str) -> AccountDisplayInfo {
    match crate::modules::account::get_current_account() {
        Ok(Some(account)) => {
            let quota_lines = if let Some(quota) = &account.quota {
                let grouped_lines = build_antigravity_group_quota_lines(lang, &quota.models);
                if grouped_lines.is_empty() {
                    build_model_quota_lines(lang, &quota.models)
                } else {
                    grouped_lines
                }
            } else {
                vec![get_text("loading", lang)]
            };
            AccountDisplayInfo {
                account: format!("📧 {}", account.email),
                quota_lines,
            }
        }
        _ => AccountDisplayInfo {
            account: format!("📧 {}", get_text("not_logged_in", lang)),
            quota_lines: vec!["—".to_string()],
        },
    }
}

#[cfg(not(target_os = "macos"))]
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
    if normalized.starts_with("gemini-3.1-pro-high") || normalized.starts_with("gemini-3-pro-high")
    {
        return "gemini-3.1-pro-high".to_string();
    }
    if normalized.starts_with("gemini-3.1-pro-low") || normalized.starts_with("gemini-3-pro-low") {
        return "gemini-3.1-pro-low".to_string();
    }
    if normalized.starts_with("claude-sonnet-4-6") || normalized.starts_with("claude-sonnet-4-5") {
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

#[cfg(not(target_os = "macos"))]
fn antigravity_model_matches(model_name: &str, target: &str) -> bool {
    let left = normalize_antigravity_model_for_match(model_name);
    let right = normalize_antigravity_model_for_match(target);
    if left.is_empty() || right.is_empty() {
        return false;
    }
    left == right || left.starts_with(&(right.clone() + "-")) || right.starts_with(&(left + "-"))
}

#[cfg(not(target_os = "macos"))]
fn parse_model_reset_ts(reset_time: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(reset_time)
        .ok()
        .map(|value| value.timestamp())
}

#[cfg(not(target_os = "macos"))]
fn build_antigravity_group_quota_lines(
    lang: &str,
    models: &[crate::models::quota::ModelQuota],
) -> Vec<String> {
    let settings = crate::modules::group_settings::load_group_settings();
    let ordered_groups = settings.get_ordered_groups(Some(3));
    if ordered_groups.is_empty() {
        return Vec::new();
    }

    let mut lines = Vec::new();
    for group_id in ordered_groups {
        let group_models = settings.get_models_in_group(&group_id);
        if group_models.is_empty() {
            continue;
        }

        let mut total_percentage: i64 = 0;
        let mut count: i64 = 0;
        let mut earliest_reset_ts: Option<i64> = None;

        for model in models {
            let belongs = group_models
                .iter()
                .any(|group_model| antigravity_model_matches(&model.name, group_model));
            if !belongs {
                continue;
            }

            total_percentage += i64::from(model.percentage.clamp(0, 100));
            count += 1;
            if let Some(reset_ts) = parse_model_reset_ts(&model.reset_time) {
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
        let reset_text = earliest_reset_ts.map(|ts| format_reset_time_from_ts(lang, Some(ts)));
        lines.push(format_quota_line(
            lang,
            &settings.get_group_name(&group_id),
            &format_percent_text(avg_percentage),
            reset_text.as_deref(),
        ));
    }

    lines
}

#[cfg(not(target_os = "macos"))]
fn format_codex_window_label(window_minutes: Option<i64>, fallback: &str) -> String {
    const HOUR_MINUTES: i64 = 60;
    const DAY_MINUTES: i64 = 24 * HOUR_MINUTES;
    const WEEK_MINUTES: i64 = 7 * DAY_MINUTES;

    let Some(minutes) = window_minutes.filter(|value| *value > 0) else {
        return fallback.to_string();
    };

    if minutes >= WEEK_MINUTES - 1 {
        let weeks = (minutes + WEEK_MINUTES - 1) / WEEK_MINUTES;
        return if weeks <= 1 {
            "Weekly".to_string()
        } else {
            format!("{} Week", weeks)
        };
    }

    if minutes >= DAY_MINUTES - 1 {
        let days = (minutes + DAY_MINUTES - 1) / DAY_MINUTES;
        return format!("{}d", days);
    }

    if minutes >= HOUR_MINUTES {
        let hours = (minutes + HOUR_MINUTES - 1) / HOUR_MINUTES;
        return format!("{}h", hours);
    }

    format!("{}m", minutes)
}

#[cfg(not(target_os = "macos"))]
fn build_codex_display_info(lang: &str) -> AccountDisplayInfo {
    if let Some(account) = crate::modules::codex_account::get_current_account() {
        let mut quota_lines = if let Some(quota) = &account.quota {
            let has_presence =
                quota.hourly_window_present.is_some() || quota.weekly_window_present.is_some();
            let mut lines = Vec::new();

            if !has_presence || quota.hourly_window_present.unwrap_or(false) {
                lines.push(format_quota_line(
                    lang,
                    &format_codex_window_label(quota.hourly_window_minutes, "5h"),
                    &format_percent_text(quota.hourly_percentage),
                    Some(&format_reset_time_from_ts(lang, quota.hourly_reset_time)),
                ));
            }

            if !has_presence || quota.weekly_window_present.unwrap_or(false) {
                lines.push(format_quota_line(
                    lang,
                    &format_codex_window_label(quota.weekly_window_minutes, "Weekly"),
                    &format_percent_text(quota.weekly_percentage),
                    Some(&format_reset_time_from_ts(lang, quota.weekly_reset_time)),
                ));
            }

            if lines.is_empty() {
                lines.push(format_quota_line(
                    lang,
                    &format_codex_window_label(quota.hourly_window_minutes, "5h"),
                    &format_percent_text(quota.hourly_percentage),
                    Some(&format_reset_time_from_ts(lang, quota.hourly_reset_time)),
                ));
            }

            lines
        } else {
            vec![get_text("loading", lang)]
        };

        if quota_lines.is_empty() {
            quota_lines.push("—".to_string());
        }

        AccountDisplayInfo {
            account: format!("📧 {}", account.email),
            quota_lines,
        }
    } else {
        AccountDisplayInfo {
            account: format!("📧 {}", get_text("not_logged_in", lang)),
            quota_lines: vec!["—".to_string()],
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn build_github_copilot_display_info(lang: &str) -> AccountDisplayInfo {
    let accounts = crate::modules::github_copilot_account::list_accounts();
    let Some(account) = resolve_github_copilot_current_account(&accounts) else {
        return AccountDisplayInfo {
            account: format!("📧 {}", get_text("not_logged_in", lang)),
            quota_lines: vec!["—".to_string()],
        };
    };

    let usage = compute_copilot_usage(
        &account.copilot_token,
        account.copilot_plan.as_deref(),
        account.copilot_limited_user_quotas.as_ref(),
        account.copilot_quota_snapshots.as_ref(),
        account.copilot_limited_user_reset_date,
        account.copilot_quota_reset_date.as_deref(),
    );

    AccountDisplayInfo {
        account: format!(
            "📧 {}",
            display_login_email(account.github_email.as_deref(), &account.github_login)
        ),
        quota_lines: build_copilot_quota_lines(lang, usage),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg(not(target_os = "macos"))]
enum WindsurfUsageMode {
    Credits,
    Quota,
}

#[cfg(not(target_os = "macos"))]
struct WindsurfQuotaUsageSummary {
    daily_used_percent: Option<i32>,
    weekly_used_percent: Option<i32>,
    daily_reset_ts: Option<i64>,
    weekly_reset_ts: Option<i64>,
    overage_balance_micros: Option<f64>,
}

#[cfg(not(target_os = "macos"))]
struct WindsurfCreditsSummary {
    credits_left: Option<f64>,
    prompt_left: Option<f64>,
    prompt_total: Option<f64>,
    add_on_left: Option<f64>,
    plan_end_ts: Option<i64>,
}

#[cfg(not(target_os = "macos"))]
fn build_windsurf_display_info(lang: &str) -> AccountDisplayInfo {
    let accounts = crate::modules::windsurf_account::list_accounts();
    let Some(account) = resolve_windsurf_current_account(&accounts) else {
        return AccountDisplayInfo {
            account: format!("📧 {}", get_text("not_logged_in", lang)),
            quota_lines: vec!["—".to_string()],
        };
    };

    let quota_lines = match resolve_windsurf_usage_mode(&account) {
        WindsurfUsageMode::Quota => {
            build_windsurf_quota_usage_lines(lang, resolve_windsurf_quota_usage_summary(&account))
        }
        WindsurfUsageMode::Credits => {
            build_windsurf_credit_usage_lines(lang, resolve_windsurf_credits_summary(&account))
        }
    };

    AccountDisplayInfo {
        account: format!(
            "📧 {}",
            display_login_email(account.github_email.as_deref(), &account.github_login)
        ),
        quota_lines,
    }
}

#[cfg(not(target_os = "macos"))]
fn build_kiro_display_info(lang: &str) -> AccountDisplayInfo {
    let accounts = crate::modules::kiro_account::list_accounts();
    let Some(account) = resolve_kiro_current_account(&accounts) else {
        return AccountDisplayInfo {
            account: format!("📧 {}", get_text("not_logged_in", lang)),
            quota_lines: vec!["—".to_string()],
        };
    };

    let mut quota_lines = Vec::new();
    let reset_text = format_reset_time_from_ts(lang, account.usage_reset_at);

    if let Some(plan) =
        first_non_empty(&[account.plan_name.as_deref(), account.plan_tier.as_deref()])
    {
        quota_lines.push(format!("Plan: {}", plan));
    }

    if let Some(remaining_pct) = calc_remaining_percent(account.credits_total, account.credits_used)
    {
        quota_lines.push(format_quota_line(
            lang,
            "Prompt",
            &format_percent_text(remaining_pct),
            Some(&reset_text),
        ));
    }

    if let Some(remaining_pct) = calc_remaining_percent(account.bonus_total, account.bonus_used) {
        quota_lines.push(format_quota_line(
            lang,
            "Add-on",
            &format_percent_text(remaining_pct),
            Some(&reset_text),
        ));
    }

    if quota_lines.is_empty() {
        quota_lines.push(get_text("loading", lang));
    }

    AccountDisplayInfo {
        account: format!(
            "📧 {}",
            first_non_empty(&[Some(account.email.as_str()), Some(account.id.as_str())])
                .unwrap_or("—")
        ),
        quota_lines,
    }
}

#[cfg(not(target_os = "macos"))]
fn build_cursor_display_info(lang: &str) -> AccountDisplayInfo {
    let accounts = crate::modules::cursor_account::list_accounts();
    let Some(account) = resolve_cursor_current_account(&accounts) else {
        return AccountDisplayInfo {
            account: format!("📧 {}", get_text("not_logged_in", lang)),
            quota_lines: vec!["—".to_string()],
        };
    };

    let mut quota_lines = Vec::new();
    let usage = read_cursor_tray_usage(&account);
    let reset_text = format_reset_time_from_ts(lang, usage.reset_ts);

    if let Some(total_used) = usage.total_used_percent {
        quota_lines.push(format_quota_line(
            lang,
            "Total",
            &format_percent_text(total_used),
            Some(&reset_text),
        ));
    }

    if let Some(auto_used) = usage.auto_used_percent {
        quota_lines.push(format_quota_line(
            lang,
            "Auto + Composer",
            &format_percent_text(auto_used),
            None,
        ));
    }

    if let Some(api_used) = usage.api_used_percent {
        quota_lines.push(format_quota_line(
            lang,
            "API",
            &format_percent_text(api_used),
            None,
        ));
    }

    if let Some(on_demand_text) = usage.on_demand_text {
        quota_lines.push(format!("On-Demand: {}", on_demand_text));
    }

    if quota_lines.is_empty() {
        quota_lines.push(get_text("loading", lang));
    }

    AccountDisplayInfo {
        account: format!(
            "📧 {}",
            first_non_empty(&[Some(account.email.as_str()), Some(account.id.as_str())])
                .unwrap_or("—")
        ),
        quota_lines,
    }
}

#[cfg(not(target_os = "macos"))]
fn parse_gemini_remaining_percent(value: Option<&serde_json::Value>) -> Option<i32> {
    let raw = value?;
    if let Some(v) = raw.as_f64() {
        if v.is_finite() {
            return Some((v * 100.0).round().clamp(0.0, 100.0) as i32);
        }
    }
    if let Some(text) = raw.as_str() {
        if let Ok(v) = text.trim().parse::<f64>() {
            if v.is_finite() {
                return Some((v * 100.0).round().clamp(0.0, 100.0) as i32);
            }
        }
    }
    None
}

#[derive(Debug, Clone)]
#[cfg(not(target_os = "macos"))]
struct GeminiBucketRemaining {
    model_id: String,
    remaining_percent: i32,
    reset_at: Option<i64>,
}

#[cfg(not(target_os = "macos"))]
fn collect_gemini_bucket_remaining(
    account: &crate::models::gemini::GeminiAccount,
) -> Vec<GeminiBucketRemaining> {
    let Some(raw) = account.gemini_usage_raw.as_ref() else {
        return Vec::new();
    };
    let Some(buckets) = raw.get("buckets").and_then(|item| item.as_array()) else {
        return Vec::new();
    };

    let mut values = Vec::new();
    for bucket in buckets {
        let model_id = bucket
            .get("modelId")
            .and_then(|item| item.as_str())
            .map(|item| item.trim())
            .filter(|item| !item.is_empty())
            .map(|item| item.to_string());
        let remaining = parse_gemini_remaining_percent(bucket.get("remainingFraction"));
        let reset_at = bucket.get("resetTime").and_then(parse_timestamp_like);
        let (Some(model_id), Some(remaining)) = (model_id, remaining) else {
            continue;
        };
        values.push(GeminiBucketRemaining {
            model_id,
            remaining_percent: remaining,
            reset_at,
        });
    }

    values.sort_by(|a, b| a.model_id.cmp(&b.model_id));
    values
}

#[cfg(not(target_os = "macos"))]
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
            (Some(_), None) => {}
            (Some(best_ts), Some(current_ts)) if current_ts < best_ts => best = current,
            _ => {}
        }
    }
    Some(best)
}

#[cfg(not(target_os = "macos"))]
fn normalize_gemini_plan_label(raw_plan: &str) -> &'static str {
    let lower = raw_plan.trim().to_lowercase();
    if lower.is_empty() {
        return "UNKNOWN";
    }
    if lower.contains("ultra") {
        return "ULTRA";
    }
    if lower == "standard-tier" {
        return "FREE";
    }
    if lower.contains("pro") || lower.contains("premium") {
        return "PRO";
    }
    if lower == "free-tier" || lower.contains("free") {
        return "FREE";
    }
    "UNKNOWN"
}

#[cfg(not(target_os = "macos"))]
fn resolve_gemini_current_account(
    accounts: &[crate::models::gemini::GeminiAccount],
) -> Option<crate::models::gemini::GeminiAccount> {
    crate::modules::gemini_account::resolve_current_account(accounts)
}

#[cfg(not(target_os = "macos"))]
fn build_gemini_display_info(lang: &str) -> AccountDisplayInfo {
    let accounts = crate::modules::gemini_account::list_accounts();
    let Some(account) = resolve_gemini_current_account(&accounts) else {
        return AccountDisplayInfo {
            account: format!("📧 {}", get_text("not_logged_in", lang)),
            quota_lines: vec!["—".to_string()],
        };
    };

    let mut quota_lines = Vec::new();

    if let Some(plan) = first_non_empty(&[account.plan_name.as_deref(), account.tier_id.as_deref()])
    {
        quota_lines.push(format!("Plan: {}", normalize_gemini_plan_label(plan)));
    }

    let buckets = collect_gemini_bucket_remaining(&account);
    let pro_bucket =
        pick_lowest_gemini_bucket(&buckets, |model_id| model_id.to_lowercase().contains("pro"));
    let flash_bucket = pick_lowest_gemini_bucket(&buckets, |model_id| {
        model_id.to_lowercase().contains("flash")
    });

    for (label, bucket) in [("Pro", pro_bucket), ("Flash", flash_bucket)] {
        let value_text = if let Some(item) = bucket {
            format!("{}% {}", item.remaining_percent, get_text("left", lang))
        } else {
            "--".to_string()
        };
        let reset_text = if let Some(item) = bucket {
            format_reset_time_from_ts(lang, item.reset_at)
        } else {
            get_text("reset_unknown", lang)
        };
        quota_lines.push(format_quota_line(
            lang,
            label,
            &value_text,
            Some(&reset_text),
        ));
    }

    if quota_lines.is_empty() {
        quota_lines.push(get_text("loading", lang));
    }

    AccountDisplayInfo {
        account: format!(
            "📧 {}",
            first_non_empty(&[Some(account.email.as_str()), Some(account.id.as_str())])
                .unwrap_or("—")
        ),
        quota_lines,
    }
}

#[cfg(not(target_os = "macos"))]
fn build_codebuddy_display_info(lang: &str) -> AccountDisplayInfo {
    let accounts = crate::modules::codebuddy_account::list_accounts();
    build_codebuddy_family_display_info(lang, resolve_codebuddy_current_account(&accounts))
}

#[cfg(not(target_os = "macos"))]
fn build_codebuddy_cn_display_info(lang: &str) -> AccountDisplayInfo {
    let accounts = crate::modules::codebuddy_cn_account::list_accounts();
    build_codebuddy_family_display_info(lang, resolve_codebuddy_cn_current_account(&accounts))
}

#[cfg(not(target_os = "macos"))]
fn build_workbuddy_display_info(lang: &str) -> AccountDisplayInfo {
    let accounts = crate::modules::workbuddy_account::list_accounts();
    build_workbuddy_family_display_info(lang, resolve_workbuddy_current_account(&accounts))
}

#[cfg(not(target_os = "macos"))]
fn build_zed_display_info(lang: &str) -> AccountDisplayInfo {
    let accounts = crate::modules::zed_account::list_accounts();
    let current_id = crate::modules::zed_account::resolve_current_account_id();
    let account = current_id
        .as_deref()
        .and_then(|id| accounts.iter().find(|item| item.id == id))
        .cloned()
        .or_else(|| {
            accounts
                .iter()
                .max_by_key(|item| item.last_used.max(item.created_at))
                .cloned()
        });

    let Some(account) = account else {
        return AccountDisplayInfo {
            account: format!("📧 {}", get_text("not_logged_in", lang)),
            quota_lines: vec!["—".to_string()],
        };
    };

    let display_value = first_non_empty(&[
        account.display_name.as_deref(),
        Some(account.github_login.as_str()),
        Some(account.user_id.as_str()),
        Some(account.id.as_str()),
    ])
    .unwrap_or("—");

    let mut quota_lines = Vec::new();
    if let Some(plan) = account
        .plan_raw
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        quota_lines.push(format!(
            "{}: {}",
            get_text("plan", lang),
            format_zed_plan_label(plan)
        ));
    }

    quota_lines.push(format!(
        "{}: {} / {}",
        get_text("edit_predictions", lang),
        format_zed_edit_predictions_used(account.edit_predictions_used),
        format_zed_edit_predictions_total(account.edit_predictions_limit_raw.as_deref()),
    ));

    quota_lines.push(format!(
        "{}: {}",
        get_text("overdue_field", lang),
        if account.has_overdue_invoices.unwrap_or(false) {
            get_text("overdue_yes", lang)
        } else {
            get_text("overdue_no", lang)
        },
    ));

    AccountDisplayInfo {
        account: format!("📧 {}", display_value),
        quota_lines,
    }
}

#[cfg(not(target_os = "macos"))]
fn format_zed_edit_predictions_used(value: Option<i64>) -> String {
    value
        .map(|used| format_quota_number((used.max(0)) as f64))
        .unwrap_or_else(|| "0".to_string())
}

#[cfg(not(target_os = "macos"))]
fn format_zed_plan_label(plan_raw: &str) -> String {
    let normalized = plan_raw.trim().trim_start_matches("zed_").trim();
    if normalized.is_empty() {
        "UNKNOWN".to_string()
    } else {
        normalized.to_uppercase()
    }
}

#[cfg(not(target_os = "macos"))]
fn format_zed_edit_predictions_total(limit_raw: Option<&str>) -> String {
    let Some(limit_raw) = limit_raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return "0".to_string();
    };

    if limit_raw.eq_ignore_ascii_case("unlimited") {
        return "0".to_string();
    }

    match limit_raw.parse::<f64>() {
        Ok(value) if value.is_finite() => format_quota_number(value.max(0.0)),
        _ => "0".to_string(),
    }
}

#[cfg(not(target_os = "macos"))]
fn build_codebuddy_family_display_info(
    lang: &str,
    account: Option<crate::models::codebuddy::CodebuddyAccount>,
) -> AccountDisplayInfo {
    let Some(account) = account else {
        return AccountDisplayInfo {
            account: format!("📧 {}", get_text("not_logged_in", lang)),
            quota_lines: vec!["—".to_string()],
        };
    };

    let display_email = first_non_empty(&[
        Some(account.email.as_str()),
        account.nickname.as_deref(),
        account.uid.as_deref(),
        Some(account.id.as_str()),
    ])
    .unwrap_or("—");

    AccountDisplayInfo {
        account: format!("📧 {}", display_email),
        quota_lines: vec![build_codebuddy_usage_status_line(lang, &account)],
    }
}

#[cfg(not(target_os = "macos"))]
fn build_workbuddy_family_display_info(
    lang: &str,
    account: Option<crate::models::workbuddy::WorkbuddyAccount>,
) -> AccountDisplayInfo {
    let Some(account) = account else {
        return AccountDisplayInfo {
            account: format!("📧 {}", get_text("not_logged_in", lang)),
            quota_lines: vec!["—".to_string()],
        };
    };

    let display_email = first_non_empty(&[
        Some(account.email.as_str()),
        account.nickname.as_deref(),
        account.uid.as_deref(),
        Some(account.id.as_str()),
    ])
    .unwrap_or("—");

    AccountDisplayInfo {
        account: format!("📧 {}", display_email),
        quota_lines: vec![build_workbuddy_usage_status_line(lang, &account)],
    }
}

#[cfg(not(target_os = "macos"))]
fn build_codebuddy_usage_status_line(
    lang: &str,
    account: &crate::models::codebuddy::CodebuddyAccount,
) -> String {
    build_usage_status_line(
        lang,
        account.dosage_notify_code.as_deref(),
        account.dosage_notify_zh.as_deref(),
        account.dosage_notify_en.as_deref(),
    )
}

#[cfg(not(target_os = "macos"))]
fn build_workbuddy_usage_status_line(
    lang: &str,
    account: &crate::models::workbuddy::WorkbuddyAccount,
) -> String {
    build_usage_status_line(
        lang,
        account.dosage_notify_code.as_deref(),
        account.dosage_notify_zh.as_deref(),
        account.dosage_notify_en.as_deref(),
    )
}

#[cfg(not(target_os = "macos"))]
fn build_usage_status_line(
    lang: &str,
    dosage_notify_code: Option<&str>,
    dosage_notify_zh: Option<&str>,
    dosage_notify_en: Option<&str>,
) -> String {
    let label = get_text("usage_status", lang);
    let code = dosage_notify_code.unwrap_or("").trim();

    if code.is_empty() {
        return format!("{}: --", label);
    }

    if code == "0" || code.eq_ignore_ascii_case("USAGE_NORMAL") {
        return format!("{}: {}", label, get_text("status_normal_short", lang));
    }

    let raw = if is_chinese_lang(lang) {
        dosage_notify_zh.or(dosage_notify_en).unwrap_or(code)
    } else {
        dosage_notify_en.or(dosage_notify_zh).unwrap_or(code)
    };

    format!("{}: {}", label, strip_codebuddy_status_prefix(raw))
}

#[cfg(not(target_os = "macos"))]
fn is_chinese_lang(lang: &str) -> bool {
    lang.to_ascii_lowercase().starts_with("zh")
}

#[cfg(not(target_os = "macos"))]
fn strip_codebuddy_status_prefix(raw: &str) -> String {
    let trimmed = raw.trim();
    for prefix in [
        "用量状态：",
        "用量状态:",
        "用量狀態：",
        "用量狀態:",
        "状态：",
        "状态:",
        "狀態：",
        "狀態:",
        "Usage Status:",
        "Usage:",
        "Status:",
    ] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return rest.trim().to_string();
        }
    }
    trimmed.to_string()
}

#[cfg(not(target_os = "macos"))]
fn resolve_codebuddy_current_account(
    accounts: &[crate::models::codebuddy::CodebuddyAccount],
) -> Option<crate::models::codebuddy::CodebuddyAccount> {
    crate::modules::codebuddy_account::resolve_current_account_id(accounts).and_then(|account_id| {
        accounts
            .iter()
            .find(|account| account.id == account_id)
            .cloned()
    })
}

#[cfg(not(target_os = "macos"))]
fn resolve_codebuddy_cn_current_account(
    accounts: &[crate::models::codebuddy::CodebuddyAccount],
) -> Option<crate::models::codebuddy::CodebuddyAccount> {
    crate::modules::codebuddy_cn_account::resolve_current_account_id(accounts).and_then(
        |account_id| {
            accounts
                .iter()
                .find(|account| account.id == account_id)
                .cloned()
        },
    )
}

#[cfg(not(target_os = "macos"))]
fn resolve_workbuddy_current_account(
    accounts: &[crate::models::workbuddy::WorkbuddyAccount],
) -> Option<crate::models::workbuddy::WorkbuddyAccount> {
    crate::modules::workbuddy_account::resolve_current_account_id(accounts).and_then(|account_id| {
        accounts
            .iter()
            .find(|account| account.id == account_id)
            .cloned()
    })
}

#[cfg(not(target_os = "macos"))]
fn json_as_f64(value: &serde_json::Value) -> Option<f64> {
    if let Some(v) = value.as_f64() {
        if v.is_finite() {
            return Some(v);
        }
    }
    if let Some(s) = value.as_str() {
        if let Ok(v) = s.trim().parse::<f64>() {
            if v.is_finite() {
                return Some(v);
            }
        }
    }
    None
}

#[cfg(not(target_os = "macos"))]
fn build_qoder_display_info(lang: &str) -> AccountDisplayInfo {
    let accounts = crate::modules::qoder_account::list_accounts();
    let account = crate::modules::qoder_account::resolve_current_account_id(&accounts)
        .and_then(|account_id| accounts.iter().find(|item| item.id == account_id).cloned());

    let Some(account) = account else {
        return AccountDisplayInfo {
            account: format!("📧 {}", get_text("not_logged_in", lang)),
            quota_lines: vec!["—".to_string()],
        };
    };

    let mut quota_lines = Vec::new();

    // Parse plan tag from raw data (matching frontend getRawPlanTag)
    let plan_tag = json_first_string(&[
        json_nested(&account.auth_user_plan_raw, &["plan_tier_name"]),
        json_nested(&account.auth_user_plan_raw, &["tier_name"]),
        json_nested(&account.auth_user_plan_raw, &["tierName"]),
        json_nested(&account.auth_user_plan_raw, &["planTierName"]),
        json_nested(&account.auth_user_plan_raw, &["plan"]),
        json_nested(&account.auth_user_info_raw, &["userTag"]),
        json_nested(&account.auth_user_info_raw, &["user_tag"]),
        json_nested(&account.auth_credit_usage_raw, &["plan_tier_name"]),
        json_nested(&account.auth_credit_usage_raw, &["tier_name"]),
        json_nested(&account.auth_credit_usage_raw, &["tierName"]),
        json_nested(&account.auth_credit_usage_raw, &["planTierName"]),
        account.plan_type.as_deref().map(|s| s.to_string()),
    ]);
    if let Some(ref tag) = plan_tag {
        quota_lines.push(format!("Plan: {}", tag));
    }

    // Parse userQuota from auth_credit_usage_raw / auth_user_plan_raw / auth_user_info_raw
    let user_quota = parse_qoder_quota_bucket(
        &[
            json_nested_obj(&account.auth_credit_usage_raw, &["userQuota"]),
            json_nested_obj(&account.auth_user_plan_raw, &["userQuota"]),
            json_nested_obj(&account.auth_user_info_raw, &["userQuota"]),
        ],
        Some((
            &account.credits_used,
            &account.credits_total,
            &account.credits_remaining,
        )),
    );

    let credits_label = if lang == "zh" || lang == "zh-CN" {
        "套餐内 Credits"
    } else {
        "Credits"
    };
    quota_lines.push(format_qoder_quota_line(
        lang,
        credits_label,
        &plan_tag,
        &user_quota,
    ));

    // Parse addOnQuota
    let addon_quota = parse_qoder_quota_bucket(
        &[
            json_nested_obj(&account.auth_credit_usage_raw, &["addOnQuota"]),
            json_nested_obj(&account.auth_credit_usage_raw, &["addonQuota"]),
            json_nested_obj(&account.auth_credit_usage_raw, &["add_on_quota"]),
            json_nested_obj(&account.auth_user_plan_raw, &["addOnQuota"]),
            json_nested_obj(&account.auth_user_plan_raw, &["addonQuota"]),
            json_nested_obj(&account.auth_user_plan_raw, &["add_on_quota"]),
        ],
        None,
    );

    let addon_label = if lang == "zh" || lang == "zh-CN" {
        "附加 Credits"
    } else {
        "Add-on Credits"
    };
    quota_lines.push(format_qoder_quota_line(
        lang,
        addon_label,
        &None,
        &addon_quota,
    ));

    // Parse shared credit package
    let shared_used = json_first_f64(&[
        json_nested_f64(
            &account.auth_credit_usage_raw,
            &["orgResourcePackage", "used"],
        ),
        json_nested_f64(
            &account.auth_credit_usage_raw,
            &["orgResourcePackage", "usage"],
        ),
        json_nested_f64(
            &account.auth_credit_usage_raw,
            &["orgResourcePackage", "consumed"],
        ),
        json_nested_f64(
            &account.auth_credit_usage_raw,
            &["orgResourcePackage", "count"],
        ),
        json_nested_f64(
            &account.auth_credit_usage_raw,
            &["organizationResourcePackage", "used"],
        ),
        json_nested_f64(
            &account.auth_credit_usage_raw,
            &["sharedCreditPackage", "used"],
        ),
        json_nested_f64(&account.auth_credit_usage_raw, &["resourcePackage", "used"]),
        json_nested_f64(&account.auth_user_plan_raw, &["orgResourcePackage", "used"]),
    ]);
    let shared_label = if lang == "zh" || lang == "zh-CN" {
        "共享资源包"
    } else {
        "Shared Package"
    };
    if let Some(used) = shared_used {
        quota_lines.push(format!("{}: {:.0}", shared_label, used));
    } else {
        quota_lines.push(format!("{}: --", shared_label));
    }

    let display_email = first_non_empty(&[
        Some(account.email.as_str()),
        account.display_name.as_deref(),
        account.user_id.as_deref(),
        Some(account.id.as_str()),
    ])
    .unwrap_or("—");

    AccountDisplayInfo {
        account: format!("📧 {}", display_email),
        quota_lines,
    }
}

/// Qoder quota bucket parsed from nested JSON
#[cfg(not(target_os = "macos"))]
struct QoderQuotaBucket {
    used: Option<f64>,
    total: Option<f64>,
    percentage: Option<f64>,
}

#[cfg(not(target_os = "macos"))]
fn parse_qoder_quota_bucket(
    sources: &[Option<serde_json::Value>],
    fallback: Option<(&Option<f64>, &Option<f64>, &Option<f64>)>,
) -> QoderQuotaBucket {
    let raw = sources.iter().find_map(|s| s.clone());

    let used = raw
        .as_ref()
        .and_then(|r| {
            json_first_f64(&[
                r.get("used").and_then(json_as_f64),
                r.get("usage").and_then(json_as_f64),
                r.get("consumed").and_then(json_as_f64),
            ])
        })
        .or_else(|| fallback.and_then(|(u, _, _)| *u));

    let total = raw
        .as_ref()
        .and_then(|r| {
            json_first_f64(&[
                r.get("total").and_then(json_as_f64),
                r.get("quota").and_then(json_as_f64),
                r.get("limit").and_then(json_as_f64),
            ])
        })
        .or_else(|| fallback.and_then(|(_, t, _)| *t));

    let percentage = raw
        .as_ref()
        .and_then(|r| {
            json_first_f64(&[
                r.get("percentage").and_then(json_as_f64),
                r.get("usagePercent").and_then(json_as_f64),
                r.get("usage_percentage").and_then(json_as_f64),
            ])
        })
        .or_else(|| match (total, used) {
            (Some(t), Some(u)) if t > 0.0 => Some((u / t) * 100.0),
            _ => None,
        });

    QoderQuotaBucket {
        used,
        total,
        percentage,
    }
}

/// Format a Qoder quota line like "套餐内 Credits [Free]: 0% 0 / 0"
#[cfg(not(target_os = "macos"))]
fn format_qoder_quota_line(
    _lang: &str,
    label: &str,
    plan_tag: &Option<String>,
    bucket: &QoderQuotaBucket,
) -> String {
    let pct_text = bucket
        .percentage
        .map(|p| format!("{:.0}%", p.clamp(0.0, 100.0)))
        .unwrap_or_else(|| "0%".to_string());
    let used_text = bucket
        .used
        .map(|v| format!("{:.0}", v))
        .unwrap_or_else(|| "0".to_string());
    let total_text = bucket
        .total
        .map(|v| format!("{:.0}", v))
        .unwrap_or_else(|| "0".to_string());

    if let Some(tag) = plan_tag {
        format!(
            "{} [{}]: {} {} / {}",
            label, tag, pct_text, used_text, total_text
        )
    } else {
        format!("{}: {} {} / {}", label, pct_text, used_text, total_text)
    }
}

/// Helpers for navigating nested JSON
#[cfg(not(target_os = "macos"))]
fn json_nested(root: &Option<serde_json::Value>, path: &[&str]) -> Option<String> {
    let mut current = root.as_ref()?;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str().map(|s| s.to_string())
}

#[cfg(not(target_os = "macos"))]
fn json_nested_obj(root: &Option<serde_json::Value>, path: &[&str]) -> Option<serde_json::Value> {
    let mut current = root.as_ref()?;
    for key in path {
        current = current.get(*key)?;
    }
    if current.is_object() {
        Some(current.clone())
    } else {
        None
    }
}

#[cfg(not(target_os = "macos"))]
fn json_nested_f64(root: &Option<serde_json::Value>, path: &[&str]) -> Option<f64> {
    let mut current = root.as_ref()?;
    for key in path {
        current = current.get(*key)?;
    }
    json_as_f64(current)
}

#[cfg(not(target_os = "macos"))]
fn json_first_string(values: &[Option<String>]) -> Option<String> {
    values
        .iter()
        .find_map(|v| v.as_ref().filter(|s| !s.trim().is_empty()).cloned())
}

#[cfg(not(target_os = "macos"))]
fn json_first_f64(values: &[Option<f64>]) -> Option<f64> {
    values.iter().find_map(|v| *v)
}

#[cfg(not(target_os = "macos"))]
fn build_trae_display_info(lang: &str) -> AccountDisplayInfo {
    let accounts = crate::modules::trae_account::list_accounts();
    let Some(account) = resolve_trae_current_account(&accounts) else {
        return AccountDisplayInfo {
            account: format!("📧 {}", get_text("not_logged_in", lang)),
            quota_lines: vec!["—".to_string()],
        };
    };

    let mut quota_lines = Vec::new();

    // Parse usage from trae_usage_raw
    let trae_usage = extract_trae_usage(&account);
    if let Some(ref usage) = trae_usage {
        // Plan badge from usage identity
        if let Some(ref identity) = usage.identity_str {
            if !identity.is_empty() {
                quota_lines.push(format!("Plan: {}", identity));
            }
        }

        // Usage percentage + USD amounts
        if usage.total_usd > 0.0 {
            let used_pct = ((usage.spent_usd / usage.total_usd) * 100.0)
                .round()
                .clamp(0.0, 100.0) as i32;
            let reset_text = usage
                .reset_at
                .map(|ts| format_reset_time_from_ts(lang, Some(ts)));
            quota_lines.push(format_quota_line(
                lang,
                if lang == "zh" || lang == "zh-CN" {
                    "配额"
                } else {
                    "Quota"
                },
                &format!("{}%", used_pct),
                reset_text.as_deref(),
            ));
            quota_lines.push(format!("${:.2} / ${:.2}", usage.spent_usd, usage.total_usd));
        } else {
            // total_usd is 0 — show 0% with $0 / $0
            let reset_text = usage
                .reset_at
                .map(|ts| format_reset_time_from_ts(lang, Some(ts)));
            quota_lines.push(format_quota_line(
                lang,
                if lang == "zh" || lang == "zh-CN" {
                    "配额"
                } else {
                    "Quota"
                },
                "0%",
                reset_text.as_deref(),
            ));
            quota_lines.push(format!("${:.0} / ${:.0}", usage.spent_usd, usage.total_usd));
        }
    }

    // Fallback: show plan_type from account field if no usage data
    if trae_usage.is_none() {
        if let Some(plan) = account.plan_type.as_deref() {
            let trimmed = plan.trim();
            if !trimmed.is_empty() {
                quota_lines.push(format!("Plan: {}", trimmed));
            }
        }
    }

    // Add subscription reset time if available
    if let Some(reset_ts) = account.plan_reset_at {
        // Only show if not already shown as part of usage line
        let already_has_reset = trae_usage.as_ref().and_then(|u| u.reset_at).is_some();
        if !already_has_reset {
            quota_lines.push(format!(
                "{}: {}",
                get_text("subscription_reset", lang),
                format_reset_time_from_ts(lang, Some(reset_ts))
            ));
        }
    }

    if quota_lines.is_empty() {
        quota_lines.push(get_text("loading", lang));
    }

    // Prefer nickname > email > user_id > id
    let display_email = first_non_empty(&[
        account.nickname.as_deref(),
        Some(account.email.as_str()),
        account.user_id.as_deref(),
        Some(account.id.as_str()),
    ])
    .unwrap_or("—");

    AccountDisplayInfo {
        account: format!("📧 {}", display_email),
        quota_lines,
    }
}

#[cfg(not(target_os = "macos"))]
struct TraeUsageSummary {
    identity_str: Option<String>,
    spent_usd: f64,
    total_usd: f64,
    reset_at: Option<i64>,
}

#[cfg(not(target_os = "macos"))]
fn extract_trae_usage(account: &crate::models::trae::TraeAccount) -> Option<TraeUsageSummary> {
    let usage_root = account.trae_usage_raw.as_ref()?.as_object()?;

    // Check API code
    if let Some(code) = usage_root.get("code").and_then(|v| v.as_i64()) {
        if code != 0 {
            return None;
        }
    }

    let packs = usage_root
        .get("user_entitlement_pack_list")
        .and_then(|v| v.as_array())?;

    if packs.is_empty() {
        return None;
    }

    // Product type constants (matching frontend trae.ts exactly)
    const PRODUCT_FREE: i64 = 0;
    const PRODUCT_PRO: i64 = 1;
    // const PRODUCT_PACKAGE: i64 = 2;
    const PRODUCT_PROMO_CODE: i64 = 3;
    const PRODUCT_PRO_PLUS: i64 = 4;
    const PRODUCT_ULTRA: i64 = 6;
    // const PRODUCT_PAY_GO: i64 = 7;
    const PRODUCT_LITE: i64 = 8;
    const PRODUCT_TRIAL: i64 = 9;

    let get_product_type = |pack: &serde_json::Value| -> i64 {
        // Try entitlement_base_info.product_type first, then pack.product_type
        pack.get("entitlement_base_info")
            .and_then(|e| e.get("product_type"))
            .and_then(|v| v.as_i64())
            .or_else(|| pack.get("product_type").and_then(|v| v.as_i64()))
            .unwrap_or(-1)
    };

    // Filter out promo code packs
    let valid_packs: Vec<_> = packs
        .iter()
        .filter(|p| get_product_type(p) != PRODUCT_PROMO_CODE)
        .collect();

    if valid_packs.is_empty() {
        return None;
    }

    // Find best pack (priority: ultra > pro_plus > pro > trial > lite > free)
    let find_by_type = |product_type: i64| -> Option<&serde_json::Value> {
        valid_packs
            .iter()
            .find(|p| get_product_type(p) == product_type)
            .copied()
    };

    let selected_pack = find_by_type(PRODUCT_ULTRA)
        .or_else(|| find_by_type(PRODUCT_PRO_PLUS))
        .or_else(|| find_by_type(PRODUCT_PRO))
        .or_else(|| find_by_type(PRODUCT_TRIAL))
        .or_else(|| find_by_type(PRODUCT_LITE))
        .or_else(|| find_by_type(PRODUCT_FREE));

    let selected_pack = selected_pack?;

    // Extract usage: pack.usage.basic_usage_amount
    let usage_obj = selected_pack.get("usage");
    let spent_usd = usage_obj
        .and_then(|u| u.get("basic_usage_amount").or_else(|| u.get("basic_usage")))
        .and_then(json_as_f64)
        .unwrap_or(0.0);

    // Extract quota: pack.entitlement_base_info.quota.basic_usage_limit
    let entitlement_base = selected_pack.get("entitlement_base_info");
    let quota_obj = entitlement_base.and_then(|e| e.get("quota"));
    let total_usd = quota_obj
        .and_then(|q| q.get("basic_usage_limit").or_else(|| q.get("basic_quota")))
        .and_then(json_as_f64)
        .unwrap_or(0.0);

    // Extract reset_at: pack.entitlement_base_info.end_time (+1)
    let reset_at = entitlement_base
        .and_then(|e| e.get("end_time"))
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
        })
        .filter(|ts| *ts > 0)
        .map(|ts| {
            let normalized = if ts > 1_000_000_000_000 {
                ts / 1000
            } else {
                ts
            };
            normalized + 1
        });

    // Identity string from usage
    let identity_str = usage_obj
        .and_then(|u| u.get("identity_str"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string());

    Some(TraeUsageSummary {
        identity_str,
        spent_usd,
        total_usd,
        reset_at,
    })
}

#[derive(Debug, Clone, Default)]
#[cfg(not(target_os = "macos"))]
struct CursorTrayUsage {
    total_used_percent: Option<i32>,
    auto_used_percent: Option<i32>,
    api_used_percent: Option<i32>,
    reset_ts: Option<i64>,
    on_demand_text: Option<String>,
}

#[cfg(not(target_os = "macos"))]
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

#[cfg(not(target_os = "macos"))]
fn pick_cursor_number(value: Option<&serde_json::Value>, keys: &[&str]) -> Option<f64> {
    let obj = value?.as_object()?;
    for key in keys {
        let Some(raw) = obj.get(*key) else {
            continue;
        };
        if let Some(v) = raw.as_f64() {
            if v.is_finite() {
                return Some(v);
            }
            continue;
        }
        if let Some(text) = raw.as_str() {
            if let Ok(parsed) = text.trim().parse::<f64>() {
                if parsed.is_finite() {
                    return Some(parsed);
                }
            }
        }
    }
    None
}

#[cfg(not(target_os = "macos"))]
fn pick_cursor_bool(value: Option<&serde_json::Value>, keys: &[&str]) -> Option<bool> {
    let obj = value?.as_object()?;
    for key in keys {
        let Some(raw) = obj.get(*key) else {
            continue;
        };
        if let Some(flag) = raw.as_bool() {
            return Some(flag);
        }
        if let Some(text) = raw.as_str() {
            let normalized = text.trim().to_ascii_lowercase();
            if normalized == "true" {
                return Some(true);
            }
            if normalized == "false" {
                return Some(false);
            }
        }
    }
    None
}

#[cfg(not(target_os = "macos"))]
fn format_cursor_dollars(cents: f64) -> String {
    format!("${:.2}", (cents / 100.0).max(0.0))
}

#[cfg(not(target_os = "macos"))]
fn read_cursor_tray_usage(account: &crate::models::cursor::CursorAccount) -> CursorTrayUsage {
    let Some(raw) = account.cursor_usage_raw.as_ref() else {
        return CursorTrayUsage::default();
    };
    let raw_obj = match raw.as_object() {
        Some(obj) => obj,
        None => return CursorTrayUsage::default(),
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

    let on_demand_effective_used = if on_demand_used.unwrap_or(0.0) > 0.0 {
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
    let on_demand_text = if !has_on_demand_hint {
        None
    } else if let Some(limit) = on_demand_limit {
        if limit > 0.0 {
            let percent = clamp_cursor_percent((on_demand_effective_used / limit) * 100.0);
            Some(format!(
                "{} ({})",
                format_percent_text(percent),
                format_cursor_dollars(on_demand_effective_used)
            ))
        } else {
            None
        }
    } else if on_demand_enabled == Some(true) && !is_team_limit {
        Some("Unlimited".to_string())
    } else {
        Some("Disabled".to_string())
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
    }
}

#[cfg(not(target_os = "macos"))]
fn resolve_github_copilot_current_account(
    accounts: &[crate::models::github_copilot::GitHubCopilotAccount],
) -> Option<crate::models::github_copilot::GitHubCopilotAccount> {
    if let Ok(settings) = crate::modules::github_copilot_instance::load_default_settings() {
        if let Some(bind_id) = settings.bind_account_id {
            let bind_id = bind_id.trim();
            if !bind_id.is_empty() {
                if let Some(account) = accounts.iter().find(|account| account.id == bind_id) {
                    return Some(account.clone());
                }
            }
        }
    }

    accounts
        .iter()
        .max_by_key(|account| account.last_used)
        .cloned()
}

#[cfg(not(target_os = "macos"))]
fn resolve_windsurf_current_account(
    accounts: &[crate::models::windsurf::WindsurfAccount],
) -> Option<crate::models::windsurf::WindsurfAccount> {
    crate::modules::windsurf_account::resolve_current_account_id(accounts).and_then(|account_id| {
        accounts
            .iter()
            .find(|account| account.id == account_id)
            .cloned()
    })
}

#[cfg(not(target_os = "macos"))]
fn resolve_kiro_current_account(
    accounts: &[crate::models::kiro::KiroAccount],
) -> Option<crate::models::kiro::KiroAccount> {
    crate::modules::kiro_account::resolve_current_account_id(accounts).and_then(|account_id| {
        accounts
            .iter()
            .find(|account| account.id == account_id)
            .cloned()
    })
}

#[cfg(not(target_os = "macos"))]
fn resolve_cursor_current_account(
    accounts: &[crate::models::cursor::CursorAccount],
) -> Option<crate::models::cursor::CursorAccount> {
    crate::modules::cursor_account::resolve_current_account_id(accounts).and_then(|account_id| {
        accounts
            .iter()
            .find(|account| account.id == account_id)
            .cloned()
    })
}

#[cfg(not(target_os = "macos"))]
fn resolve_trae_current_account(
    accounts: &[crate::models::trae::TraeAccount],
) -> Option<crate::models::trae::TraeAccount> {
    crate::modules::trae_account::resolve_current_account_id(accounts).and_then(|account_id| {
        accounts
            .iter()
            .find(|account| account.id == account_id)
            .cloned()
    })
}

#[cfg(not(target_os = "macos"))]
fn first_non_empty<'a>(values: &[Option<&'a str>]) -> Option<&'a str> {
    values
        .iter()
        .flatten()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
}

#[cfg(not(target_os = "macos"))]
fn calc_remaining_percent(total: Option<f64>, used: Option<f64>) -> Option<i32> {
    let total = total?;
    if !total.is_finite() || total <= 0.0 {
        return None;
    }

    let used = used.unwrap_or(0.0);
    if !used.is_finite() {
        return None;
    }

    let remaining = (total - used).max(0.0);
    Some(clamp_percent((remaining / total) * 100.0))
}

#[cfg(not(target_os = "macos"))]
fn display_login_email(email: Option<&str>, login: &str) -> String {
    email
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .unwrap_or(login)
        .to_string()
}

#[cfg(not(target_os = "macos"))]
fn format_percent_text(percentage: i32) -> String {
    format!("{}%", percentage.clamp(0, 100))
}

#[cfg(not(target_os = "macos"))]
fn format_quota_line(
    lang: &str,
    label: &str,
    value_text: &str,
    reset_text: Option<&str>,
) -> String {
    let normalized_reset = reset_text
        .map(|text| text.trim())
        .filter(|text| !text.is_empty() && *text != "—");

    if let Some(reset) = normalized_reset {
        format!(
            "{}: {} · {} {}",
            label,
            value_text,
            get_text("reset", lang),
            reset
        )
    } else {
        format!("{}: {}", label, value_text)
    }
}

#[cfg(not(target_os = "macos"))]
fn format_copilot_metric_value(lang: &str, metric: CopilotMetric) -> Option<String> {
    if metric.included {
        return Some(get_text("included", lang));
    }
    metric
        .used_percent
        .map(|percentage| format!("{}%", percentage))
}

#[cfg(not(target_os = "macos"))]
fn build_copilot_quota_lines(lang: &str, usage: CopilotUsage) -> Vec<String> {
    let mut lines = Vec::new();
    let reset_text = format_reset_time_from_ts(lang, usage.reset_ts);

    if let Some(value_text) = format_copilot_metric_value(lang, usage.inline) {
        lines.push(format_quota_line(
            lang,
            &get_text("ghcp_inline", lang),
            &value_text,
            Some(&reset_text),
        ));
    }
    if let Some(value_text) = format_copilot_metric_value(lang, usage.chat) {
        lines.push(format_quota_line(
            lang,
            &get_text("ghcp_chat", lang),
            &value_text,
            Some(&reset_text),
        ));
    }
    let premium_value =
        format_copilot_metric_value(lang, usage.premium).unwrap_or_else(|| "-".to_string());
    lines.push(format_quota_line(
        lang,
        &get_text("ghcp_premium", lang),
        &premium_value,
        None,
    ));

    if lines.is_empty() {
        lines.push(get_text("loading", lang));
    }

    lines
}

#[cfg(not(target_os = "macos"))]
fn build_windsurf_quota_usage_lines(lang: &str, summary: WindsurfQuotaUsageSummary) -> Vec<String> {
    let mut lines = Vec::new();
    let daily_reset_text = format_reset_time_from_ts(lang, summary.daily_reset_ts);
    let weekly_reset_text = format_reset_time_from_ts(lang, summary.weekly_reset_ts);

    if let Some(percentage) = summary.daily_used_percent {
        lines.push(format_quota_line(
            lang,
            &get_text("windsurf_daily_quota_usage", lang),
            &format_percent_text(percentage),
            Some(&daily_reset_text),
        ));
    }
    if let Some(percentage) = summary.weekly_used_percent {
        lines.push(format_quota_line(
            lang,
            &get_text("windsurf_weekly_quota_usage", lang),
            &format_percent_text(percentage),
            Some(&weekly_reset_text),
        ));
    }
    lines.push(format_quota_line(
        lang,
        &get_text("windsurf_extra_usage_balance", lang),
        &format_micros_usd(summary.overage_balance_micros.unwrap_or(0.0)),
        None,
    ));

    if lines.is_empty() {
        lines.push(get_text("loading", lang));
    }

    lines
}

#[cfg(not(target_os = "macos"))]
fn build_windsurf_credit_usage_lines(lang: &str, summary: WindsurfCreditsSummary) -> Vec<String> {
    let mut lines = Vec::new();
    let reset_text = format_reset_time_from_ts(lang, summary.plan_end_ts);

    lines.push(format_quota_line(
        lang,
        &get_text("windsurf_credits_left", lang),
        &summary
            .credits_left
            .map(format_quota_number)
            .unwrap_or_else(|| "-".to_string()),
        Some(&reset_text),
    ));
    let prompt_value = match (summary.prompt_left, summary.prompt_total) {
        (Some(left), Some(total)) if total > 0.0 => {
            format!(
                "{}/{}",
                format_quota_number(left),
                format_quota_number(total)
            )
        }
        (Some(left), _) => format_quota_number(left),
        _ => "-".to_string(),
    };
    lines.push(format_quota_line(
        lang,
        &get_text("windsurf_prompt_credits_left", lang),
        &prompt_value,
        None,
    ));
    lines.push(format_quota_line(
        lang,
        &get_text("windsurf_addon_credits_available", lang),
        &format_quota_number(summary.add_on_left.unwrap_or(0.0)),
        None,
    ));

    if lines.is_empty() {
        lines.push(get_text("loading", lang));
    }

    lines
}

#[cfg(not(target_os = "macos"))]
fn format_quota_number(value: f64) -> String {
    let normalized = if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    };
    if (normalized.fract()).abs() < f64::EPSILON {
        format!("{:.0}", normalized)
    } else {
        format!("{:.2}", normalized)
    }
}

#[cfg(not(target_os = "macos"))]
fn format_micros_usd(value: f64) -> String {
    let normalized = if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    };
    format!("${:.2}", normalized / 1_000_000.0)
}

#[cfg(not(target_os = "macos"))]
fn compute_copilot_usage(
    token: &str,
    plan: Option<&str>,
    limited_quotas: Option<&serde_json::Value>,
    quota_snapshots: Option<&serde_json::Value>,
    limited_reset_ts: Option<i64>,
    quota_reset_date: Option<&str>,
) -> CopilotUsage {
    let token_map = parse_token_map(token);
    let reset_ts = limited_reset_ts
        .or_else(|| parse_reset_date_to_ts(quota_reset_date))
        .or_else(|| {
            parse_token_number(&token_map, "rd")
                .map(|value| value.floor() as i64)
                .filter(|value| *value > 0)
        });
    let sku = token_map
        .get("sku")
        .map(|value| value.to_lowercase())
        .unwrap_or_default();
    let is_free_limited = sku.contains("free_limited")
        || sku.contains("no_auth_limited")
        || plan
            .map(|value| value.to_lowercase().contains("free_limited"))
            .unwrap_or(false);

    let completions_snapshot = get_quota_snapshot(quota_snapshots, "completions");
    let chat_snapshot = get_quota_snapshot(quota_snapshots, "chat");
    let premium_snapshot = get_quota_snapshot(quota_snapshots, "premium_interactions");

    let limited = limited_quotas.and_then(|value| value.as_object());
    let remaining_inline = remaining_from_snapshot(completions_snapshot).or_else(|| {
        limited
            .and_then(|obj| obj.get("completions"))
            .and_then(parse_json_number)
    });
    let remaining_chat = remaining_from_snapshot(chat_snapshot).or_else(|| {
        limited
            .and_then(|obj| obj.get("chat"))
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

#[cfg(not(target_os = "macos"))]
fn get_quota_snapshot<'a>(
    quota_snapshots: Option<&'a serde_json::Value>,
    key: &str,
) -> Option<&'a serde_json::Map<String, serde_json::Value>> {
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

#[cfg(not(target_os = "macos"))]
fn entitlement_from_snapshot(
    snapshot: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Option<f64> {
    snapshot
        .and_then(|data| data.get("entitlement"))
        .and_then(parse_json_number)
        .filter(|value| *value > 0.0)
}

#[cfg(not(target_os = "macos"))]
fn remaining_from_snapshot(
    snapshot: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Option<f64> {
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

#[cfg(not(target_os = "macos"))]
fn is_included_snapshot(snapshot: Option<&serde_json::Map<String, serde_json::Value>>) -> bool {
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

#[cfg(not(target_os = "macos"))]
fn used_percent_from_snapshot(
    snapshot: Option<&serde_json::Map<String, serde_json::Value>>,
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

    let percent_remaining = snapshot
        .and_then(|data| data.get("percent_remaining"))
        .and_then(parse_json_number)
        .map(clamp_percent)?;
    Some(clamp_percent((100 - percent_remaining) as f64))
}

#[cfg(not(target_os = "macos"))]
fn resolve_windsurf_usage_mode(
    account: &crate::models::windsurf::WindsurfAccount,
) -> WindsurfUsageMode {
    match resolve_windsurf_billing_strategy(account).as_deref() {
        Some("quota") => WindsurfUsageMode::Quota,
        _ => WindsurfUsageMode::Credits,
    }
}

#[cfg(not(target_os = "macos"))]
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

    let normalized = raw.trim().to_lowercase();
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

#[cfg(not(target_os = "macos"))]
fn resolve_windsurf_quota_usage_summary(
    account: &crate::models::windsurf::WindsurfAccount,
) -> WindsurfQuotaUsageSummary {
    let plan_status_roots = windsurf_plan_status_roots(account);
    let is_quota_billing = resolve_windsurf_billing_strategy(account).as_deref() == Some("quota");
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

#[cfg(not(target_os = "macos"))]
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
        add_on_left: add_on_left_actual,
        plan_end_ts: resolve_windsurf_plan_end_ts(account),
    }
}

#[cfg(not(target_os = "macos"))]
fn windsurf_plan_status_roots<'a>(
    account: &'a crate::models::windsurf::WindsurfAccount,
) -> Vec<Option<&'a serde_json::Value>> {
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

#[cfg(not(target_os = "macos"))]
fn windsurf_plan_info_roots<'a>(
    account: &'a crate::models::windsurf::WindsurfAccount,
) -> Vec<Option<&'a serde_json::Value>> {
    let direct_plan_status = account.windsurf_plan_status.as_ref();
    let snapshots = account.copilot_quota_snapshots.as_ref();

    vec![
        json_path(direct_plan_status, &["planInfo"]),
        json_path(direct_plan_status, &["plan_info"]),
        json_path(snapshots, &["windsurfPlanInfo"]),
        json_path(snapshots, &["windsurf_plan_info"]),
    ]
}

#[cfg(not(target_os = "macos"))]
fn first_string_from_roots<'a>(
    roots: &[Option<&'a serde_json::Value>],
    paths: &[&[&str]],
) -> Option<&'a str> {
    for root in roots.iter().flatten() {
        for path in paths {
            if let Some(value) = json_path(Some(*root), path).and_then(|v| v.as_str()) {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    return Some(value);
                }
            }
        }
    }
    None
}

#[cfg(not(target_os = "macos"))]
fn first_number_from_roots(roots: &[Option<&serde_json::Value>], paths: &[&[&str]]) -> Option<f64> {
    for root in roots.iter().flatten() {
        for path in paths {
            if let Some(value) = json_path(Some(*root), path).and_then(parse_json_number) {
                return Some(value);
            }
        }
    }
    None
}

#[cfg(not(target_os = "macos"))]
fn first_timestamp_from_roots(
    roots: &[Option<&serde_json::Value>],
    paths: &[&[&str]],
) -> Option<i64> {
    for root in roots.iter().flatten() {
        for path in paths {
            if let Some(value) = json_path(Some(*root), path).and_then(parse_timestamp_like) {
                return Some(value);
            }
        }
    }
    None
}

#[cfg(not(target_os = "macos"))]
fn sum_option_f64(left: Option<f64>, right: Option<f64>) -> Option<f64> {
    match (left, right) {
        (Some(a), Some(b)) => Some(a + b),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

#[cfg(not(target_os = "macos"))]
fn resolve_windsurf_plan_end_ts(account: &crate::models::windsurf::WindsurfAccount) -> Option<i64> {
    let mut candidates: Vec<Option<&serde_json::Value>> = Vec::new();
    let user_status = account.windsurf_user_status.as_ref();
    let snapshots = account.copilot_quota_snapshots.as_ref();

    candidates.push(json_path(
        user_status,
        &["userStatus", "planStatus", "planEnd"],
    ));
    candidates.push(json_path(
        user_status,
        &["userStatus", "planStatus", "plan_end"],
    ));
    candidates.push(json_path(user_status, &["planStatus", "planEnd"]));
    candidates.push(json_path(user_status, &["planStatus", "plan_end"]));
    candidates.push(json_path(snapshots, &["windsurfPlanStatus", "planEnd"]));
    candidates.push(json_path(snapshots, &["windsurfPlanStatus", "plan_end"]));
    candidates.push(json_path(
        snapshots,
        &["windsurfPlanStatus", "planStatus", "planEnd"],
    ));
    candidates.push(json_path(
        snapshots,
        &["windsurfPlanStatus", "planStatus", "plan_end"],
    ));
    candidates.push(json_path(
        snapshots,
        &["windsurfUserStatus", "userStatus", "planStatus", "planEnd"],
    ));
    candidates.push(json_path(
        snapshots,
        &["windsurfUserStatus", "userStatus", "planStatus", "plan_end"],
    ));

    for candidate in candidates.into_iter().flatten() {
        if let Some(ts) = parse_timestamp_like(candidate) {
            return Some(ts);
        }
    }

    None
}

#[cfg(not(target_os = "macos"))]
fn json_path<'a>(
    root: Option<&'a serde_json::Value>,
    path: &[&str],
) -> Option<&'a serde_json::Value> {
    let mut current = root?;
    for key in path {
        current = current.as_object()?.get(*key)?;
    }
    Some(current)
}

#[cfg(not(target_os = "macos"))]
fn parse_timestamp_like(value: &serde_json::Value) -> Option<i64> {
    match value {
        serde_json::Value::Number(num) => parse_timestamp_number(num.as_f64()?),
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return None;
            }
            if let Ok(n) = trimmed.parse::<f64>() {
                return parse_timestamp_number(n);
            }
            chrono::DateTime::parse_from_rfc3339(trimmed)
                .ok()
                .map(|dt| dt.timestamp())
        }
        serde_json::Value::Object(obj) => {
            if let Some(seconds) = obj.get("seconds").and_then(|v| v.as_i64()) {
                return Some(seconds);
            }
            if let Some(seconds) = obj.get("unixSeconds").and_then(|v| v.as_i64()) {
                return Some(seconds);
            }
            if let Some(inner) = obj.get("value") {
                return parse_timestamp_like(inner);
            }
            None
        }
        _ => None,
    }
}

#[cfg(not(target_os = "macos"))]
fn parse_timestamp_number(raw: f64) -> Option<i64> {
    if !raw.is_finite() || raw <= 0.0 {
        return None;
    }
    if raw > 1e12 {
        return Some((raw / 1000.0).floor() as i64);
    }
    Some(raw.floor() as i64)
}

#[cfg(not(target_os = "macos"))]
fn parse_token_map(token: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let prefix = token.split(':').next().unwrap_or(token);
    for item in prefix.split(';') {
        let mut parts = item.splitn(2, '=');
        let key = parts.next().unwrap_or("").trim();
        if key.is_empty() {
            continue;
        }
        let value = parts.next().unwrap_or("").trim();
        map.insert(key.to_string(), value.to_string());
    }
    map
}

#[cfg(not(target_os = "macos"))]
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

#[cfg(not(target_os = "macos"))]
fn parse_json_number(value: &serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Number(num) => num.as_f64(),
        serde_json::Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|value| value.is_finite())
}

#[cfg(not(target_os = "macos"))]
fn calc_used_percent(total: Option<f64>, remaining: Option<f64>) -> Option<i32> {
    let total = total?;
    let remaining = remaining?;
    if total <= 0.0 {
        return None;
    }

    let used = (total - remaining).max(0.0);
    Some(clamp_percent((used / total) * 100.0))
}

#[cfg(not(target_os = "macos"))]
fn parse_reset_date_to_ts(reset_date: Option<&str>) -> Option<i64> {
    let reset_date = reset_date?.trim();
    if reset_date.is_empty() {
        return None;
    }
    chrono::DateTime::parse_from_rfc3339(reset_date)
        .ok()
        .map(|value| value.timestamp())
}

#[cfg(not(target_os = "macos"))]
fn clamp_percent(value: f64) -> i32 {
    value.round().clamp(0.0, 100.0) as i32
}

#[cfg(not(target_os = "macos"))]
fn build_model_quota_lines(lang: &str, models: &[crate::models::quota::ModelQuota]) -> Vec<String> {
    let mut lines = Vec::new();
    for model in models.iter().take(4) {
        let reset_text = format_reset_time(lang, &model.reset_time);
        lines.push(format_quota_line(
            lang,
            &model.name,
            &format_percent_text(model.percentage),
            Some(&reset_text),
        ));
    }
    if lines.is_empty() {
        lines.push("—".to_string());
    }
    lines
}

#[cfg(not(target_os = "macos"))]
fn format_reset_time_from_ts(lang: &str, reset_ts: Option<i64>) -> String {
    let Some(reset_ts) = reset_ts else {
        return "—".to_string();
    };
    let now = chrono::Utc::now().timestamp();
    let remaining_secs = reset_ts - now;
    if remaining_secs <= 0 {
        return get_text("reset_done", lang);
    }
    format_remaining_duration(remaining_secs)
}

#[cfg(not(target_os = "macos"))]
fn format_remaining_duration(remaining_secs: i64) -> String {
    let mut secs = remaining_secs.max(0);
    let days = secs / 86_400;
    secs %= 86_400;
    let hours = secs / 3_600;
    secs %= 3_600;
    let minutes = (secs / 60).max(1);

    if days > 0 {
        format!("{}d {}h {}m", days, hours, minutes)
    } else if hours > 0 {
        format!("{}h {}m", hours, minutes)
    } else {
        format!("{}m", minutes)
    }
}

/// 格式化重置时间
#[cfg(not(target_os = "macos"))]
fn format_reset_time(lang: &str, reset_time: &str) -> String {
    if let Ok(reset) = chrono::DateTime::parse_from_rfc3339(reset_time) {
        let now = chrono::Utc::now();
        let duration = reset.signed_duration_since(now);

        if duration.num_seconds() <= 0 {
            return get_text("reset_done", lang);
        }

        let hours = duration.num_hours();
        let minutes = duration.num_minutes() % 60;

        if hours > 0 {
            format!("{}h {}m", hours, minutes)
        } else {
            format!("{}m", minutes)
        }
    } else {
        reset_time.to_string()
    }
}

/// 处理菜单事件
fn handle_menu_event<R: Runtime>(app: &tauri::AppHandle<R>, event: tauri::menu::MenuEvent) {
    let id = event.id().as_ref();
    logger::log_info(&format!("[Tray] 菜单点击: {}", id));

    match id {
        menu_ids::SHOW_WINDOW => {
            if let Err(err) = crate::modules::floating_card_window::show_main_window(app) {
                logger::log_warn(&format!("[Tray] 显示主窗口失败: {}", err));
            }
        }
        menu_ids::REFRESH_QUOTA => {
            let _ = app.emit("tray:refresh_quota", ());
        }
        menu_ids::SHOW_FLOATING_CARD => {
            let _ = crate::modules::floating_card_window::show_floating_card_window(app, true);
        }
        menu_ids::SETTINGS => {
            if let Err(err) =
                crate::modules::floating_card_window::show_main_window_and_navigate(app, "settings")
            {
                logger::log_warn(&format!("[Tray] 打开设置页失败: {}", err));
            }
        }
        menu_ids::QUIT => {
            info!("[Tray] 用户选择退出应用");
            app.exit(0);
        }
        _ => {
            if let Some(platform) = parse_platform_from_menu_id(id) {
                if let Err(err) =
                    crate::modules::floating_card_window::show_main_window_and_navigate(
                        app,
                        platform.nav_target(),
                    )
                {
                    logger::log_warn(&format!(
                        "[Tray] 打开平台页面失败: platform={}, err={}",
                        platform.as_str(),
                        err
                    ));
                }
            } else if id.starts_with("ag_") {
                if let Err(err) =
                    crate::modules::floating_card_window::show_main_window_and_navigate(
                        app, "overview",
                    )
                {
                    logger::log_warn(&format!("[Tray] 打开 Antigravity 总览失败: {}", err));
                }
            } else if id.starts_with("codex_") {
                if let Err(err) =
                    crate::modules::floating_card_window::show_main_window_and_navigate(
                        app, "codex",
                    )
                {
                    logger::log_warn(&format!("[Tray] 打开 Codex 页面失败: {}", err));
                }
            }
        }
    }
}

fn parse_platform_from_menu_id(id: &str) -> Option<PlatformId> {
    let mut parts = id.split(':');
    if parts.next()? != "platform" {
        return None;
    }
    PlatformId::from_str(parts.next()?)
}

/// 处理托盘图标事件
fn handle_tray_event<R: Runtime>(tray: &TrayIcon<R>, event: TrayIconEvent) {
    match event {
        TrayIconEvent::Click {
            button,
            button_state,
            rect: _rect,
            ..
        } => {
            #[cfg(target_os = "macos")]
            {
                if button == MouseButton::Left && button_state == MouseButtonState::Up {
                    if let Err(err) =
                        crate::modules::floating_card_window::show_main_window(tray.app_handle())
                    {
                        logger::log_warn(&format!("[Tray] 左键恢复主窗口失败: {}", err));
                    }
                    return;
                }

                if button == MouseButton::Right && button_state == MouseButtonState::Down {
                    let app = tray.app_handle().clone();
                    let app_for_menu = app.clone();
                    let _ = app.run_on_main_thread(move || {
                        crate::modules::macos_native_menu::toggle_tray_menu(&app_for_menu, _rect);
                    });
                }
            }

            #[cfg(not(target_os = "macos"))]
            if button == MouseButton::Left && button_state == MouseButtonState::Up {
                if let Err(err) =
                    crate::modules::floating_card_window::show_main_window(tray.app_handle())
                {
                    logger::log_warn(&format!("[Tray] 左键恢复主窗口失败: {}", err));
                }
            }
        }
        TrayIconEvent::DoubleClick {
            button: MouseButton::Left,
            ..
        } => {
            #[cfg(target_os = "macos")]
            {
                return;
            }

            #[cfg(not(target_os = "macos"))]
            if let Err(err) =
                crate::modules::floating_card_window::show_main_window(tray.app_handle())
            {
                logger::log_warn(&format!("[Tray] 双击恢复主窗口失败: {}", err));
            }
        }
        _ => {}
    }
}

/// 更新托盘菜单
pub fn update_tray_menu<R: Runtime>(app: &tauri::AppHandle<R>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let _ = app;
        if !MACOS_TRAY_SKIP_LOGGED.swap(true, Ordering::Relaxed) {
            logger::log_info("[Tray] macOS 原生菜单模式，跳过 Tauri 托盘菜单更新");
        }
    }

    #[cfg(not(target_os = "macos"))]
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let menu = build_tray_menu(app).map_err(|e| e.to_string())?;
        tray.set_menu(Some(menu)).map_err(|e| e.to_string())?;
        logger::log_info("[Tray] 托盘菜单已更新");
    }
    Ok(())
}

/// 获取本地化文本
#[cfg(not(target_os = "macos"))]
fn get_text(key: &str, lang: &str) -> String {
    match (key, lang) {
        // 简体中文
        ("show_window", "zh-cn") => "显示主窗口".to_string(),
        ("show_floating_card", "zh-cn") => "显示悬浮卡片".to_string(),
        ("refresh_quota", "zh-cn") => "🔄 刷新配额".to_string(),
        ("settings", "zh-cn") => "⚙️ 设置...".to_string(),
        ("quit", "zh-cn") => "❌ 退出".to_string(),
        ("not_logged_in", "zh-cn") => "未登录".to_string(),
        ("loading", "zh-cn") => "加载中...".to_string(),
        ("reset", "zh-cn") => "重置".to_string(),
        ("reset_done", "zh-cn") => "已重置".to_string(),
        ("reset_unknown", "zh-cn") => "重置时间未知".to_string(),
        ("left", "zh-cn") => "剩余".to_string(),
        ("usage_status", "zh-cn") => "用量状态".to_string(),
        ("plan", "zh-cn") => "订阅".to_string(),
        ("token_spend", "zh-cn") => "Token 消耗".to_string(),
        ("edit_predictions", "zh-cn") => "编辑预测".to_string(),
        ("overdue_field", "zh-cn") => "是否欠费".to_string(),
        ("overdue_yes", "zh-cn") => "是".to_string(),
        ("overdue_no", "zh-cn") => "否".to_string(),
        ("status_normal_short", "zh-cn") => "正常".to_string(),
        ("included", "zh-cn") => "包含".to_string(),
        ("ghcp_inline", "zh-cn") => "Inline".to_string(),
        ("ghcp_chat", "zh-cn") => "Chat".to_string(),
        ("ghcp_premium", "zh-cn") => "Premium".to_string(),
        ("windsurf_daily_quota_usage", "zh-cn") => "每日额度用量".to_string(),
        ("windsurf_weekly_quota_usage", "zh-cn") => "每周额度用量".to_string(),
        ("windsurf_extra_usage_balance", "zh-cn") => "额外用量余额".to_string(),
        ("windsurf_credits_left", "zh-cn") => "剩余积分".to_string(),
        ("windsurf_prompt_credits_left", "zh-cn") => "Prompt Credits".to_string(),
        ("windsurf_addon_credits_available", "zh-cn") => "附加积分".to_string(),
        ("subscription_reset", "zh-cn") => "订阅重置".to_string(),
        ("more_platforms", "zh-cn") => "更多平台".to_string(),
        ("no_platform_selected", "zh-cn") => "未选择托盘平台".to_string(),

        // 繁体中文
        ("show_window", "zh-tw") => "顯示主視窗".to_string(),
        ("show_floating_card", "zh-tw") => "顯示懸浮卡片".to_string(),
        ("refresh_quota", "zh-tw") => "🔄 重新整理配額".to_string(),
        ("settings", "zh-tw") => "⚙️ 設定...".to_string(),
        ("quit", "zh-tw") => "❌ 結束".to_string(),
        ("not_logged_in", "zh-tw") => "未登入".to_string(),
        ("loading", "zh-tw") => "載入中...".to_string(),
        ("reset", "zh-tw") => "重置".to_string(),
        ("reset_done", "zh-tw") => "已重置".to_string(),
        ("reset_unknown", "zh-tw") => "重置時間未知".to_string(),
        ("left", "zh-tw") => "剩餘".to_string(),
        ("usage_status", "zh-tw") => "用量狀態".to_string(),
        ("plan", "zh-tw") => "訂閱".to_string(),
        ("token_spend", "zh-tw") => "Token 消耗".to_string(),
        ("edit_predictions", "zh-tw") => "編輯預測".to_string(),
        ("overdue_field", "zh-tw") => "是否欠費".to_string(),
        ("overdue_yes", "zh-tw") => "是".to_string(),
        ("overdue_no", "zh-tw") => "否".to_string(),
        ("status_normal_short", "zh-tw") => "正常".to_string(),
        ("included", "zh-tw") => "已包含".to_string(),
        ("ghcp_inline", "zh-tw") => "Inline".to_string(),
        ("ghcp_chat", "zh-tw") => "Chat".to_string(),
        ("ghcp_premium", "zh-tw") => "Premium".to_string(),
        ("windsurf_daily_quota_usage", "zh-tw") => "每日額度用量".to_string(),
        ("windsurf_weekly_quota_usage", "zh-tw") => "每週額度用量".to_string(),
        ("windsurf_extra_usage_balance", "zh-tw") => "額外用量餘額".to_string(),
        ("windsurf_credits_left", "zh-tw") => "剩餘積分".to_string(),
        ("windsurf_prompt_credits_left", "zh-tw") => "Prompt Credits".to_string(),
        ("windsurf_addon_credits_available", "zh-tw") => "附加積分".to_string(),
        ("subscription_reset", "zh-tw") => "訂閱重置".to_string(),
        ("more_platforms", "zh-tw") => "更多平台".to_string(),
        ("no_platform_selected", "zh-tw") => "未選擇托盤平台".to_string(),

        // 英文
        ("show_window", "en") => "Show Window".to_string(),
        ("show_floating_card", "en") => "Show Floating Card".to_string(),
        ("refresh_quota", "en") => "🔄 Refresh Quota".to_string(),
        ("settings", "en") => "⚙️ Settings...".to_string(),
        ("quit", "en") => "❌ Quit".to_string(),
        ("not_logged_in", "en") => "Not logged in".to_string(),
        ("loading", "en") => "Loading...".to_string(),
        ("reset", "en") => "Reset".to_string(),
        ("reset_done", "en") => "Reset done".to_string(),
        ("reset_unknown", "en") => "Reset time unknown".to_string(),
        ("left", "en") => "left".to_string(),
        ("usage_status", "en") => "Usage Status".to_string(),
        ("plan", "en") => "Plan".to_string(),
        ("token_spend", "en") => "Token Spend".to_string(),
        ("edit_predictions", "en") => "Edit Predictions".to_string(),
        ("overdue_field", "en") => "Overdue".to_string(),
        ("overdue_yes", "en") => "Yes".to_string(),
        ("overdue_no", "en") => "No".to_string(),
        ("status_normal_short", "en") => "Normal".to_string(),
        ("included", "en") => "Included".to_string(),
        ("ghcp_inline", "en") => "Inline".to_string(),
        ("ghcp_chat", "en") => "Chat".to_string(),
        ("ghcp_premium", "en") => "Premium".to_string(),
        ("windsurf_daily_quota_usage", "en") => "Daily quota usage".to_string(),
        ("windsurf_weekly_quota_usage", "en") => "Weekly quota usage".to_string(),
        ("windsurf_extra_usage_balance", "en") => "Extra usage balance".to_string(),
        ("windsurf_credits_left", "en") => "Credits left".to_string(),
        ("windsurf_prompt_credits_left", "en") => "Prompt credits left".to_string(),
        ("windsurf_addon_credits_available", "en") => "Add-on credits available".to_string(),
        ("subscription_reset", "en") => "Subscription reset".to_string(),
        ("more_platforms", "en") => "More platforms".to_string(),
        ("no_platform_selected", "en") => "No tray platforms selected".to_string(),

        // 日语
        ("show_window", "ja") => "ウィンドウを表示".to_string(),
        ("show_floating_card", "ja") => "フローティングカードを表示".to_string(),
        ("refresh_quota", "ja") => "🔄 クォータを更新".to_string(),
        ("settings", "ja") => "⚙️ 設定...".to_string(),
        ("quit", "ja") => "❌ 終了".to_string(),
        ("not_logged_in", "ja") => "未ログイン".to_string(),
        ("loading", "ja") => "読み込み中...".to_string(),
        ("reset", "ja") => "リセット".to_string(),
        ("reset_done", "ja") => "リセット済み".to_string(),
        ("reset_unknown", "ja") => "リセット時間不明".to_string(),
        ("left", "ja") => "残り".to_string(),
        ("usage_status", "ja") => "利用状況".to_string(),
        ("plan", "ja") => "プラン".to_string(),
        ("token_spend", "ja") => "Token Spend".to_string(),
        ("edit_predictions", "ja") => "Edit Predictions".to_string(),
        ("overdue_field", "ja") => "延滞有無".to_string(),
        ("overdue_yes", "ja") => "はい".to_string(),
        ("overdue_no", "ja") => "いいえ".to_string(),
        ("status_normal_short", "ja") => "正常".to_string(),
        ("included", "ja") => "含まれる".to_string(),
        ("ghcp_inline", "ja") => "Inline".to_string(),
        ("ghcp_chat", "ja") => "Chat".to_string(),
        ("ghcp_premium", "ja") => "Premium".to_string(),
        ("windsurf_daily_quota_usage", "ja") => "日次クォータ使用量".to_string(),
        ("windsurf_weekly_quota_usage", "ja") => "週次クォータ使用量".to_string(),
        ("windsurf_extra_usage_balance", "ja") => "追加使用残高".to_string(),
        ("windsurf_credits_left", "ja") => "残りクレジット".to_string(),
        ("windsurf_prompt_credits_left", "ja") => "Prompt Credits".to_string(),
        ("windsurf_addon_credits_available", "ja") => "追加クレジット".to_string(),
        ("subscription_reset", "ja") => "サブスクリプションリセット".to_string(),
        ("more_platforms", "ja") => "その他のプラットフォーム".to_string(),
        ("no_platform_selected", "ja") => {
            "トレイに表示するプラットフォームがありません".to_string()
        }

        // 俄语
        ("show_window", "ru") => "Показать окно".to_string(),
        ("show_floating_card", "ru") => "Показать плавающую карточку".to_string(),
        ("refresh_quota", "ru") => "🔄 Обновить квоту".to_string(),
        ("settings", "ru") => "⚙️ Настройки...".to_string(),
        ("quit", "ru") => "❌ Выход".to_string(),
        ("not_logged_in", "ru") => "Не авторизован".to_string(),
        ("loading", "ru") => "Загрузка...".to_string(),
        ("reset", "ru") => "Сброс".to_string(),
        ("reset_done", "ru") => "Сброс выполнен".to_string(),
        ("reset_unknown", "ru") => "Время сброса неизвестно".to_string(),
        ("left", "ru") => "осталось".to_string(),
        ("usage_status", "ru") => "Статус использования".to_string(),
        ("plan", "ru") => "План".to_string(),
        ("token_spend", "ru") => "Token Spend".to_string(),
        ("edit_predictions", "ru") => "Edit Predictions".to_string(),
        ("overdue_field", "ru") => "Есть задолженность".to_string(),
        ("overdue_yes", "ru") => "Да".to_string(),
        ("overdue_no", "ru") => "Нет".to_string(),
        ("status_normal_short", "ru") => "Норма".to_string(),
        ("included", "ru") => "Включено".to_string(),
        ("ghcp_inline", "ru") => "Inline".to_string(),
        ("ghcp_chat", "ru") => "Chat".to_string(),
        ("ghcp_premium", "ru") => "Premium".to_string(),
        ("windsurf_daily_quota_usage", "ru") => "Дневная квота".to_string(),
        ("windsurf_weekly_quota_usage", "ru") => "Недельная квота".to_string(),
        ("windsurf_extra_usage_balance", "ru") => "Баланс доп. использования".to_string(),
        ("windsurf_credits_left", "ru") => "Остаток кредитов".to_string(),
        ("windsurf_prompt_credits_left", "ru") => "Prompt credits".to_string(),
        ("windsurf_addon_credits_available", "ru") => "Доп. кредиты".to_string(),
        ("subscription_reset", "ru") => "Сброс подписки".to_string(),
        ("more_platforms", "ru") => "Другие платформы".to_string(),
        ("no_platform_selected", "ru") => "Платформы для трея не выбраны".to_string(),

        // 默认英文
        ("show_window", _) => "Show Window".to_string(),
        ("show_floating_card", _) => "Show Floating Card".to_string(),
        ("refresh_quota", _) => "🔄 Refresh Quota".to_string(),
        ("settings", _) => "⚙️ Settings...".to_string(),
        ("quit", _) => "❌ Quit".to_string(),
        ("not_logged_in", _) => "Not logged in".to_string(),
        ("loading", _) => "Loading...".to_string(),
        ("reset", _) => "Reset".to_string(),
        ("reset_done", _) => "Reset done".to_string(),
        ("reset_unknown", _) => "Reset time unknown".to_string(),
        ("left", _) => "left".to_string(),
        ("usage_status", _) => "Usage Status".to_string(),
        ("plan", _) => "Plan".to_string(),
        ("token_spend", _) => "Token Spend".to_string(),
        ("edit_predictions", _) => "Edit Predictions".to_string(),
        ("overdue_field", _) => "Overdue".to_string(),
        ("overdue_yes", _) => "Yes".to_string(),
        ("overdue_no", _) => "No".to_string(),
        ("status_normal_short", _) => "Normal".to_string(),
        ("included", _) => "Included".to_string(),
        ("ghcp_inline", _) => "Inline".to_string(),
        ("ghcp_chat", _) => "Chat".to_string(),
        ("ghcp_premium", _) => "Premium".to_string(),
        ("windsurf_daily_quota_usage", _) => "Daily quota usage".to_string(),
        ("windsurf_weekly_quota_usage", _) => "Weekly quota usage".to_string(),
        ("windsurf_extra_usage_balance", _) => "Extra usage balance".to_string(),
        ("windsurf_credits_left", _) => "Credits left".to_string(),
        ("windsurf_prompt_credits_left", _) => "Prompt credits left".to_string(),
        ("windsurf_addon_credits_available", _) => "Add-on credits available".to_string(),
        ("subscription_reset", _) => "Subscription reset".to_string(),
        ("more_platforms", _) => "More platforms".to_string(),
        ("no_platform_selected", _) => "No tray platforms selected".to_string(),

        _ => key.to_string(),
    }
}
