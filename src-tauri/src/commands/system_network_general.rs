// System commands：Network, terminal, diagnostics and general config commands。
// 通过 include! 保持原 commands::system 作用域和 Tauri command 路径。
/// 获取网络服务配置
#[tauri::command]
pub fn get_network_config() -> Result<NetworkConfig, String> {
    let user_config = config::get_user_config();
    let ws_actual_port = config::get_actual_port();
    let report_actual_port = web_report::get_actual_port();

    Ok(NetworkConfig {
        ws_enabled: user_config.ws_enabled,
        ws_port: user_config.ws_port,
        actual_port: ws_actual_port,
        default_port: DEFAULT_WS_PORT,
        report_enabled: user_config.report_enabled,
        report_port: user_config.report_port,
        report_actual_port,
        report_default_port: DEFAULT_REPORT_PORT,
        report_token: user_config.report_token,
        global_proxy_enabled: user_config.global_proxy_enabled,
        global_proxy_url: user_config.global_proxy_url,
        global_proxy_no_proxy: user_config.global_proxy_no_proxy,
    })
}

/// 保存网络服务配置
#[tauri::command]
pub fn save_network_config(
    ws_enabled: bool,
    ws_port: u16,
    report_enabled: Option<bool>,
    report_port: Option<u16>,
    report_token: Option<String>,
    global_proxy_enabled: Option<bool>,
    global_proxy_url: Option<String>,
    global_proxy_no_proxy: Option<String>,
) -> Result<bool, String> {
    let mut needs_restart = false;
    config::patch_user_config(|current| {
        let next_report_enabled = report_enabled.unwrap_or(current.report_enabled);
        let next_report_port = report_port.unwrap_or(current.report_port);
        let next_report_token = report_token
            .unwrap_or_else(|| current.report_token.clone())
            .trim()
            .to_string();
        let next_global_proxy_enabled =
            global_proxy_enabled.unwrap_or(current.global_proxy_enabled);
        let next_global_proxy_url = global_proxy_url
            .unwrap_or_else(|| current.global_proxy_url.clone())
            .trim()
            .to_string();
        let next_global_proxy_no_proxy = global_proxy_no_proxy
            .unwrap_or_else(|| current.global_proxy_no_proxy.clone())
            .trim()
            .to_string();

        if next_report_enabled && next_report_token.is_empty() {
            return Err("网页查询服务 token 不能为空".to_string());
        }
        if next_global_proxy_enabled && next_global_proxy_url.is_empty() {
            return Err("启用全局代理时，代理地址不能为空".to_string());
        }

        needs_restart = current.ws_port != ws_port
            || current.ws_enabled != ws_enabled
            || current.report_enabled != next_report_enabled
            || current.report_port != next_report_port
            || current.report_token != next_report_token;

        current.ws_enabled = ws_enabled;
        current.ws_port = ws_port;
        current.report_enabled = next_report_enabled;
        current.report_port = next_report_port;
        current.report_token = next_report_token;
        current.global_proxy_enabled = next_global_proxy_enabled;
        current.global_proxy_url = next_global_proxy_url;
        current.global_proxy_no_proxy = next_global_proxy_no_proxy;
        Ok(())
    })?;

    Ok(needs_restart)
}

/// 获取系统可用的终端列表
#[tauri::command]
pub async fn get_available_terminals() -> Result<Vec<String>, String> {
    let mut available = Vec::new();
    available.push("system".to_string());

    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").unwrap_or_default();
        let terminals = [
            (
                "Terminal",
                vec![
                    "/System/Applications/Utilities/Terminal.app".to_string(),
                    "/Applications/Utilities/Terminal.app".to_string(),
                ],
            ),
            (
                "iTerm2",
                vec![
                    "/Applications/iTerm.app".to_string(),
                    "/Applications/iTerm 2.app".to_string(),
                    format!("{}/Applications/iTerm.app", home),
                ],
            ),
            (
                "Warp",
                vec![
                    "/Applications/Warp.app".to_string(),
                    format!("{}/Applications/Warp.app", home),
                ],
            ),
            (
                "Ghostty",
                vec![
                    "/Applications/Ghostty.app".to_string(),
                    format!("{}/Applications/Ghostty.app", home),
                ],
            ),
            (
                "WezTerm",
                vec![
                    "/Applications/WezTerm.app".to_string(),
                    format!("{}/Applications/WezTerm.app", home),
                ],
            ),
            (
                "Kitty",
                vec![
                    "/Applications/Kitty.app".to_string(),
                    format!("{}/Applications/Kitty.app", home),
                ],
            ),
            (
                "Alacritty",
                vec![
                    "/Applications/Alacritty.app".to_string(),
                    format!("{}/Applications/Alacritty.app", home),
                ],
            ),
        ];
        for (name, paths) in terminals {
            for path in paths {
                if !path.is_empty() && std::path::Path::new(&path).exists() {
                    available.push(name.to_string());
                    break;
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        // Windows 下检查可执行文件是否在 PATH 中
        let terminals = ["cmd", "PowerShell", "pwsh", "wt"];
        for name in terminals {
            if is_command_available(name) {
                available.push(name.to_string());
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let terminals = [
            "x-terminal-emulator",
            "gnome-terminal",
            "konsole",
            "xfce4-terminal",
            "xterm",
            "alacritty",
            "kitty",
        ];
        for name in terminals {
            if is_command_available(name) {
                available.push(name.to_string());
            }
        }
    }

    Ok(available)
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn is_command_available(cmd: &str) -> bool {
    #[cfg(target_os = "windows")]
    let check_cmd = "where";
    #[cfg(target_os = "linux")]
    let check_cmd = "which";

    let mut command = std::process::Command::new(check_cmd);
    command
        .arg(cmd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        command.creation_flags(CREATE_NO_WINDOW);
    }

    command.status().map(|s| s.success()).unwrap_or(false)
}

/// 获取诊断上报配置
#[tauri::command]
pub fn get_diagnostics_config() -> modules::diagnostics::DiagnosticsConfig {
    modules::diagnostics::get_diagnostics_config()
}

/// 保存诊断上报配置
#[tauri::command]
pub fn save_diagnostics_config(
    error_reporting_enabled: bool,
    error_reporting_debug: Option<bool>,
) -> Result<(), String> {
    modules::diagnostics::save_diagnostics_config(error_reporting_enabled, error_reporting_debug)
}

/// 记录前端启动阶段，只写本地日志，不触发远端上报
#[tauri::command]
pub fn diagnostics_frontend_stage(stage: String, detail: Option<serde_json::Value>) {
    modules::diagnostics::record_frontend_stage(stage, detail);
}

/// 标记前端已完成启动
#[tauri::command]
pub fn diagnostics_frontend_ready(stage: Option<String>) {
    modules::diagnostics::mark_frontend_ready(stage);
}

/// 捕获前端诊断事件并异步上报
#[tauri::command]
pub fn diagnostics_capture_event(event: modules::diagnostics::DiagnosticsClientEvent) {
    modules::diagnostics::capture_client_event(event);
}

/// 获取通用设置配置
#[tauri::command]
pub fn get_general_config(app: tauri::AppHandle) -> Result<GeneralConfig, String> {
    let started = Instant::now();
    let user_config = config::get_user_config();
    let app_auto_launch_enabled =
        get_app_auto_launch_enabled(&app).unwrap_or(user_config.app_auto_launch_enabled);

    let close_behavior_str = match user_config.close_behavior {
        CloseWindowBehavior::Ask => "ask",
        CloseWindowBehavior::Minimize => "minimize",
        CloseWindowBehavior::Quit => "quit",
    };
    let minimize_behavior_str = match user_config.minimize_behavior {
        MinimizeWindowBehavior::DockAndTray => "dock_and_tray",
        MinimizeWindowBehavior::TrayOnly => "tray_only",
    };

    let result = GeneralConfig {
        language: user_config.language,
        default_terminal: user_config.default_terminal,
        theme: user_config.theme,
        theme_color: config::normalize_theme_color(&user_config.theme_color),
        external_network_enabled: user_config.external_network_enabled,
        webdav_allowed_domains: user_config.webdav_allowed_domains,
        reduced_motion_enabled: user_config.reduced_motion_enabled,
        ui_scale: user_config.ui_scale,
        auto_refresh_minutes: user_config.auto_refresh_minutes,
        codex_auto_refresh_minutes: user_config.codex_auto_refresh_minutes,
        codex_sync_wsl: user_config.codex_sync_wsl,
        codex_app_ui_injection_enabled: user_config.codex_app_ui_injection_enabled,
        codex_oauth_app_version: user_config.codex_oauth_app_version,
        codex_cli_only_allow_app_server_clients: user_config
            .codex_cli_only_allow_app_server_clients,
        codex_wsl_config_dir: user_config.codex_wsl_config_dir,
        zed_auto_refresh_minutes: user_config.zed_auto_refresh_minutes,
        ghcp_auto_refresh_minutes: user_config.ghcp_auto_refresh_minutes,
        windsurf_auto_refresh_minutes: user_config.windsurf_auto_refresh_minutes,
        kiro_auto_refresh_minutes: user_config.kiro_auto_refresh_minutes,
        cursor_auto_refresh_minutes: user_config.cursor_auto_refresh_minutes,
        grok_auto_refresh_minutes: user_config.grok_auto_refresh_minutes,
        grok_sync_official_auth_on_switch: user_config.grok_sync_official_auth_on_switch,
        grok_opencode_sync_on_switch: user_config.grok_opencode_sync_on_switch,
        grok_opencode_auth_overwrite_on_switch: user_config.grok_opencode_auth_overwrite_on_switch,
        claude_auto_refresh_minutes: user_config.claude_auto_refresh_minutes,
        codebuddy_auto_refresh_minutes: user_config.codebuddy_auto_refresh_minutes,
        codebuddy_cn_auto_refresh_minutes: user_config.codebuddy_cn_auto_refresh_minutes,
        workbuddy_auto_refresh_minutes: user_config.workbuddy_auto_refresh_minutes,
        qoder_auto_refresh_minutes: user_config.qoder_auto_refresh_minutes,
        zcode_auto_refresh_minutes: user_config.zcode_auto_refresh_minutes,
        trae_auto_refresh_minutes: user_config.trae_auto_refresh_minutes,
        trae_solo_auto_refresh_minutes: user_config.trae_solo_auto_refresh_minutes,
        trae_cn_auto_refresh_minutes: user_config.trae_cn_auto_refresh_minutes,
        trae_solo_cn_auto_refresh_minutes: user_config.trae_solo_cn_auto_refresh_minutes,
        close_behavior: close_behavior_str.to_string(),
        minimize_behavior: minimize_behavior_str.to_string(),
        hide_dock_icon: user_config.hide_dock_icon,
        tray_icon_style: user_config.tray_icon_style.as_str().to_string(),
        menu_bar_quota_enabled: user_config.menu_bar_quota_enabled,
        menu_bar_show_account_prefix: user_config.menu_bar_show_account_prefix,
        menu_bar_quota_platform: user_config.menu_bar_quota_platform,
        floating_card_show_on_startup: user_config.floating_card_show_on_startup,
        startup_minimized: user_config.startup_minimized,
        remember_main_window_state: user_config.remember_main_window_state,
        startup_page: config::normalize_startup_page(&user_config.startup_page),
        floating_card_always_on_top: user_config.floating_card_always_on_top,
        app_auto_launch_enabled,
        token_keeper_enabled: user_config.token_keeper_enabled,
        auto_import_from_local_enabled: user_config.auto_import_from_local_enabled,
        antigravity_startup_wakeup_enabled: user_config.antigravity_startup_wakeup_enabled,
        antigravity_startup_wakeup_delay_seconds: sanitize_startup_wakeup_delay_seconds(
            user_config.antigravity_startup_wakeup_delay_seconds,
        ),
        codex_startup_wakeup_enabled: user_config.codex_startup_wakeup_enabled,
        codex_startup_wakeup_delay_seconds: sanitize_startup_wakeup_delay_seconds(
            user_config.codex_startup_wakeup_delay_seconds,
        ),
        floating_card_confirm_on_close: user_config.floating_card_confirm_on_close,
        opencode_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.opencode_app_path,
        ),
        antigravity_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.antigravity_app_path,
        ),
        codex_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.codex_app_path,
        ),
        claude_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.claude_app_path,
        ),
        claude_app_scan_roots: user_config.claude_app_scan_roots,
        codex_specified_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.codex_specified_app_path,
        ),
        zed_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.zed_app_path,
        ),
        vscode_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.vscode_app_path,
        ),
        windsurf_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.windsurf_app_path,
        ),
        kiro_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.kiro_app_path,
        ),
        cursor_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.cursor_app_path,
        ),
        codebuddy_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.codebuddy_app_path,
        ),
        codebuddy_share_sessions_on_switch: user_config.codebuddy_share_sessions_on_switch,
        codebuddy_cn_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.codebuddy_cn_app_path,
        ),
        codebuddy_cn_share_sessions_on_switch: user_config.codebuddy_cn_share_sessions_on_switch,
        qoder_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.qoder_app_path,
        ),
        zcode_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.zcode_app_path,
        ),
        trae_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.trae_app_path,
        ),
        trae_solo_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.trae_solo_app_path,
        ),
        trae_cn_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.trae_cn_app_path,
        ),
        trae_solo_cn_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.trae_solo_cn_app_path,
        ),
        trae_share_sessions_on_switch: user_config.trae_share_sessions_on_switch,
        trae_solo_share_sessions_on_switch: user_config.trae_solo_share_sessions_on_switch,
        trae_cn_share_sessions_on_switch: user_config.trae_cn_share_sessions_on_switch,
        trae_solo_cn_share_sessions_on_switch: user_config.trae_solo_cn_share_sessions_on_switch,
        trae_app_scan_roots: user_config.trae_app_scan_roots,
        trae_solo_app_scan_roots: user_config.trae_solo_app_scan_roots,
        trae_cn_app_scan_roots: user_config.trae_cn_app_scan_roots,
        trae_solo_cn_app_scan_roots: user_config.trae_solo_cn_app_scan_roots,
        workbuddy_app_path: modules::process::normalize_windows_user_facing_path(
            &user_config.workbuddy_app_path,
        ),
        workbuddy_share_sessions_on_switch: user_config.workbuddy_share_sessions_on_switch,
        opencode_sync_on_switch: user_config.opencode_sync_on_switch,
        opencode_auth_overwrite_on_switch: user_config.opencode_auth_overwrite_on_switch,
        ghcp_opencode_sync_on_switch: user_config.ghcp_opencode_sync_on_switch,
        ghcp_opencode_auth_overwrite_on_switch: user_config.ghcp_opencode_auth_overwrite_on_switch,
        ghcp_launch_on_switch: user_config.ghcp_launch_on_switch,
        openclaw_auth_overwrite_on_switch: user_config.openclaw_auth_overwrite_on_switch,
        hermes_auth_overwrite_on_switch: user_config.hermes_auth_overwrite_on_switch,
        codex_launch_on_switch: user_config.codex_launch_on_switch,
        antigravity_launch_on_switch: user_config.antigravity_launch_on_switch,
        codex_restart_specified_app_on_switch: user_config.codex_restart_specified_app_on_switch,
        codex_local_access_entry_visible: user_config.codex_local_access_entry_visible,
        codex_hide_relay_quota: user_config.codex_hide_relay_quota,
        top_right_ad_visible: user_config.top_right_ad_visible,
        antigravity_dual_switch_no_restart_enabled: user_config
            .antigravity_dual_switch_no_restart_enabled,
        auto_switch_enabled: user_config.auto_switch_enabled,
        auto_switch_threshold: user_config.auto_switch_threshold,
        auto_switch_credits_enabled: user_config.auto_switch_credits_enabled,
        auto_switch_credits_threshold: user_config.auto_switch_credits_threshold,
        auto_switch_scope_mode: user_config.auto_switch_scope_mode,
        auto_switch_selected_group_ids: user_config.auto_switch_selected_group_ids,
        auto_switch_account_scope_mode: user_config.auto_switch_account_scope_mode,
        auto_switch_selected_account_ids: user_config.auto_switch_selected_account_ids,
        codex_auto_switch_enabled: user_config.codex_auto_switch_enabled,
        codex_auto_switch_primary_threshold: user_config.codex_auto_switch_primary_threshold,
        codex_auto_switch_secondary_threshold: user_config.codex_auto_switch_secondary_threshold,
        codex_auto_switch_account_scope_mode: user_config.codex_auto_switch_account_scope_mode,
        codex_auto_switch_selected_account_ids: user_config.codex_auto_switch_selected_account_ids,
        quota_alert_enabled: user_config.quota_alert_enabled,
        quota_alert_threshold: user_config.quota_alert_threshold,
        codex_quota_alert_enabled: user_config.codex_quota_alert_enabled,
        codex_quota_alert_threshold: user_config.codex_quota_alert_threshold,
        zed_quota_alert_enabled: user_config.zed_quota_alert_enabled,
        zed_quota_alert_threshold: user_config.zed_quota_alert_threshold,
        codex_quota_alert_primary_threshold: user_config.codex_quota_alert_primary_threshold,
        codex_quota_alert_secondary_threshold: user_config.codex_quota_alert_secondary_threshold,
        ghcp_quota_alert_enabled: user_config.ghcp_quota_alert_enabled,
        ghcp_quota_alert_threshold: user_config.ghcp_quota_alert_threshold,
        windsurf_quota_alert_enabled: user_config.windsurf_quota_alert_enabled,
        windsurf_quota_alert_threshold: user_config.windsurf_quota_alert_threshold,
        kiro_quota_alert_enabled: user_config.kiro_quota_alert_enabled,
        kiro_quota_alert_threshold: user_config.kiro_quota_alert_threshold,
        cursor_quota_alert_enabled: user_config.cursor_quota_alert_enabled,
        cursor_quota_alert_threshold: user_config.cursor_quota_alert_threshold,
        grok_quota_alert_enabled: user_config.grok_quota_alert_enabled,
        grok_quota_alert_threshold: user_config.grok_quota_alert_threshold,
        claude_quota_alert_enabled: user_config.claude_quota_alert_enabled,
        claude_quota_alert_threshold: user_config.claude_quota_alert_threshold,
        claude_quota_display_remaining: user_config.claude_quota_display_remaining,
        codebuddy_quota_alert_enabled: user_config.codebuddy_quota_alert_enabled,
        codebuddy_quota_alert_threshold: user_config.codebuddy_quota_alert_threshold,
        codebuddy_cn_quota_alert_enabled: user_config.codebuddy_cn_quota_alert_enabled,
        codebuddy_cn_quota_alert_threshold: user_config.codebuddy_cn_quota_alert_threshold,
        qoder_quota_alert_enabled: user_config.qoder_quota_alert_enabled,
        qoder_quota_alert_threshold: user_config.qoder_quota_alert_threshold,
        trae_quota_alert_enabled: user_config.trae_quota_alert_enabled,
        trae_quota_alert_threshold: user_config.trae_quota_alert_threshold,
        trae_solo_quota_alert_enabled: user_config.trae_solo_quota_alert_enabled,
        trae_solo_quota_alert_threshold: user_config.trae_solo_quota_alert_threshold,
        trae_cn_quota_alert_enabled: user_config.trae_cn_quota_alert_enabled,
        trae_cn_quota_alert_threshold: user_config.trae_cn_quota_alert_threshold,
        trae_solo_cn_quota_alert_enabled: user_config.trae_solo_cn_quota_alert_enabled,
        trae_solo_cn_quota_alert_threshold: user_config.trae_solo_cn_quota_alert_threshold,
        workbuddy_quota_alert_enabled: user_config.workbuddy_quota_alert_enabled,
        workbuddy_quota_alert_threshold: user_config.workbuddy_quota_alert_threshold,
    };

    modules::logger::log_info(&format!(
        "[StartupPerf][SystemCommand] get_general_config completed in {}ms: auto_refresh={}, codex={}, zed={}, ghcp={}, windsurf={}, kiro={}, cursor={}, codebuddy={}, codebuddy_cn={}, workbuddy={}, qoder={}, zcode={}, trae={}, auto_switch={}",
        started.elapsed().as_millis(),
        result.auto_refresh_minutes,
        result.codex_auto_refresh_minutes,
        result.zed_auto_refresh_minutes,
        result.ghcp_auto_refresh_minutes,
        result.windsurf_auto_refresh_minutes,
        result.kiro_auto_refresh_minutes,
        result.cursor_auto_refresh_minutes,
        result.codebuddy_auto_refresh_minutes,
        result.codebuddy_cn_auto_refresh_minutes,
        result.workbuddy_auto_refresh_minutes,
        result.qoder_auto_refresh_minutes,
        result.zcode_auto_refresh_minutes,
        result.trae_auto_refresh_minutes,
        result.auto_switch_enabled
    ));

    Ok(result)
}

/// 按字段保存通用设置配置。
#[tauri::command]
pub fn patch_general_config(
    app: tauri::AppHandle,
    updates: JsonMap<String, JsonValue>,
) -> Result<(), String> {
    let _save_guard = lock_general_config_transaction()?;

    if updates.is_empty() {
        return Ok(());
    }

    // 在修改系统自启动状态前完成字段和类型校验，避免无效请求留下外部副作用。
    let mut preview = config::get_user_config();
    apply_general_config_updates(&mut preview, &updates)?;

    let requested_auto_launch = updates
        .get("app_auto_launch_enabled")
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| "配置字段 app_auto_launch_enabled 必须为布尔值".to_string())
        })
        .transpose()?;
    let previous_auto_launch = requested_auto_launch
        .map(|_| get_app_auto_launch_enabled(&app))
        .transpose()?;
    let auto_launch_os_changed = requested_auto_launch
        .zip(previous_auto_launch)
        .map(|(requested, previous)| requested != previous)
        .unwrap_or(false);
    if auto_launch_os_changed {
        apply_app_auto_launch_enabled(
            &app,
            requested_auto_launch.expect("requested auto launch should exist"),
        )?;
    }

    let mut language_changed = false;
    let codex_client_policy_changed =
        updates.contains_key("codex_cli_only_allow_app_server_clients");
    let mut token_keeper_enabled_changed = false;
    let mut auto_import_from_local_enabled_changed = false;
    let mut floating_always_on_top_changed = false;
    #[cfg(target_os = "macos")]
    let mut hide_dock_icon_changed = false;
    #[cfg(target_os = "macos")]
    let mut tray_icon_style_changed = false;
    #[cfg(target_os = "macos")]
    let mut menu_bar_quota_changed = false;

    let patch_result = config::patch_user_config(|current| {
        let previous_language = current.language.clone();
        let previous_token_keeper_enabled = current.token_keeper_enabled;
        let previous_auto_import_from_local_enabled = current.auto_import_from_local_enabled;
        let previous_floating_always_on_top = current.floating_card_always_on_top;
        #[cfg(target_os = "macos")]
        let previous_hide_dock_icon = current.hide_dock_icon;
        #[cfg(target_os = "macos")]
        let previous_tray_icon_style = current.tray_icon_style;
        #[cfg(target_os = "macos")]
        let previous_menu_bar_quota = (
            current.menu_bar_quota_enabled,
            current.menu_bar_show_account_prefix,
            current.menu_bar_quota_platform.clone(),
        );

        apply_general_config_updates(current, &updates)?;

        language_changed = previous_language != current.language;
        token_keeper_enabled_changed =
            previous_token_keeper_enabled != current.token_keeper_enabled;
        auto_import_from_local_enabled_changed =
            previous_auto_import_from_local_enabled != current.auto_import_from_local_enabled;
        floating_always_on_top_changed =
            previous_floating_always_on_top != current.floating_card_always_on_top;
        #[cfg(target_os = "macos")]
        {
            hide_dock_icon_changed = previous_hide_dock_icon != current.hide_dock_icon;
            tray_icon_style_changed = previous_tray_icon_style != current.tray_icon_style;
            menu_bar_quota_changed = previous_menu_bar_quota
                != (
                    current.menu_bar_quota_enabled,
                    current.menu_bar_show_account_prefix,
                    current.menu_bar_quota_platform.clone(),
                );
        }
        Ok(())
    });

    let new_config = match patch_result {
        Ok(config) => config,
        Err(error) => {
            if auto_launch_os_changed {
                if let Some(previous) = previous_auto_launch {
                    if let Err(rollback_error) = apply_app_auto_launch_enabled(&app, previous) {
                        modules::logger::log_error(&format!(
                            "[SystemConfig] 配置保存失败后回滚应用自启动状态失败: {}",
                            rollback_error
                        ));
                    }
                }
            }
            return Err(error);
        }
    };

    if token_keeper_enabled_changed {
        modules::provider_token_keeper::notify_config_changed(
            app.clone(),
            new_config.token_keeper_enabled,
        );
    }

    if auto_import_from_local_enabled_changed {
        modules::auto_local_import::notify_config_changed(
            new_config.auto_import_from_local_enabled,
        );
    }

    if codex_client_policy_changed {
        modules::codex_local_access::schedule_codex_client_policy_sync();
    }

    if floating_always_on_top_changed {
        if let Err(err) = modules::floating_card_window::apply_floating_card_always_on_top(&app) {
            modules::logger::log_warn(&format!(
                "[FloatingCard] 保存通用设置后应用置顶状态失败: {}",
                err
            ));
        }
    }

    #[cfg(target_os = "macos")]
    if hide_dock_icon_changed {
        crate::apply_macos_activation_policy(&app);
    }

    #[cfg(target_os = "macos")]
    if tray_icon_style_changed {
        if let Err(err) = modules::tray::apply_tray_icon_style(&app) {
            modules::logger::log_warn(&format!("[Tray] 保存通用设置后应用图标样式失败: {}", err));
        }
    }

    #[cfg(target_os = "macos")]
    if menu_bar_quota_changed {
        if let Err(err) = modules::tray::update_tray_menu(&app) {
            modules::logger::log_warn(&format!("[Tray] 保存菜单栏额度设置后刷新失败: {}", err));
        }
    }

    if language_changed {
        websocket::broadcast_language_changed(&new_config.language, "desktop");
        modules::sync_settings::write_sync_setting("language", &new_config.language);
        if let Err(err) = modules::tray::update_tray_menu(&app) {
            modules::logger::log_warn(&format!("[Tray] 语言变更后刷新托盘失败: {}", err));
        }
    }

    Ok(())
}

/// 立即扫描并导入本机当前登录账号（开启「本机账号自动导入」后调用）。
#[tauri::command]
pub async fn scan_auto_local_import(
    app: tauri::AppHandle,
) -> Result<modules::auto_local_import::AutoLocalImportScanResult, String> {
    modules::auto_local_import::scan_now(app).await
}

// --- Codex SSH sync (#1404 vertical slice) ---
#[tauri::command]
pub fn codex_ssh_list_servers() -> Result<modules::codex_ssh::CodexSshListResult, String> {
    let (servers, selected_id) = modules::codex_ssh::list_servers()?;
    Ok(modules::codex_ssh::CodexSshListResult {
        servers,
        selected_id,
    })
}

#[tauri::command]
pub fn codex_ssh_upsert_server(
    server: modules::codex_ssh::CodexSshServer,
) -> Result<modules::codex_ssh::CodexSshServer, String> {
    modules::codex_ssh::upsert_server(server)
}

#[tauri::command]
pub fn codex_ssh_delete_server(id: String) -> Result<(), String> {
    modules::codex_ssh::delete_server(&id)
}

#[tauri::command]
pub fn codex_ssh_select_server(id: String) -> Result<(), String> {
    modules::codex_ssh::select_server(&id)
}

#[tauri::command]
pub fn codex_ssh_test_connection(id: String) -> Result<String, String> {
    modules::codex_ssh::test_connection(&id)
}

#[tauri::command]
pub fn codex_ssh_sync_current(id: String) -> Result<String, String> {
    modules::codex_ssh::sync_current_account(&id)
}

/// Managed provider id for local API LB (#980 vertical slice).
#[tauri::command]
pub fn codex_managed_lb_provider_id() -> String {
    "cockpit-codex-lb".to_string()
}

#[tauri::command]
pub fn codebuddy_list_local_session_files(
    limit: Option<u32>,
) -> Result<Vec<modules::codebuddy_session_list::CodebuddySessionFileEntry>, String> {
    Ok(modules::codebuddy_session_list::list_local_session_files(
        limit.unwrap_or(100) as usize,
    ))
}
