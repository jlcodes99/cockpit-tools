#[tauri::command]
pub fn save_refresh_interval_config(
    auto_refresh_minutes: Option<i32>,
    codex_auto_refresh_minutes: Option<i32>,
) -> Result<(), String> {
    config::patch_user_config(|current| {
        if let Some(value) = auto_refresh_minutes {
            current.auto_refresh_minutes = value;
        }
        if let Some(value) = codex_auto_refresh_minutes {
            current.codex_auto_refresh_minutes = value;
        }
        Ok(())
    })?;
    Ok(())
}

#[tauri::command]
pub fn save_tray_platform_layout(
    app: tauri::AppHandle,
    sort_mode: String,
    ordered_platform_ids: Vec<String>,
    tray_platform_ids: Vec<String>,
    ordered_entry_ids: Option<Vec<String>>,
    platform_groups: Option<Vec<modules::tray_layout::TrayLayoutGroup>>,
) -> Result<(), String> {
    modules::tray_layout::save_tray_layout(
        sort_mode,
        ordered_platform_ids,
        tray_platform_ids,
        ordered_entry_ids,
        platform_groups,
    )?;
    modules::tray::update_tray_menu(&app)?;
    Ok(())
}

#[tauri::command]
pub fn set_app_path(app: String, path: String) -> Result<(), String> {
    let normalized_path = modules::process::normalize_windows_user_facing_path(&path);
    config::patch_user_config(move |current| {
        match app.as_str() {
            "antigravity" | "antigravity_ide" | "antigravity_legacy" => {
                current.antigravity_app_path = normalized_path
            }
            "codex" => current.codex_app_path = normalized_path,
            "claude" => current.claude_app_path = normalized_path,
            "zed" => current.zed_app_path = normalized_path,
            "vscode" => current.vscode_app_path = normalized_path,
            "windsurf" => current.windsurf_app_path = normalized_path,
            "kiro" => current.kiro_app_path = normalized_path,
            "cursor" => current.cursor_app_path = normalized_path,
            "codebuddy" => current.codebuddy_app_path = normalized_path,
            "codebuddy_cn" => current.codebuddy_cn_app_path = normalized_path,
            "qoder" => current.qoder_app_path = normalized_path,
            "zcode" => current.zcode_app_path = normalized_path,
            "trae" => current.trae_app_path = normalized_path,
            "trae_solo" => current.trae_solo_app_path = normalized_path,
            "trae_cn" => current.trae_cn_app_path = normalized_path,
            "trae_solo_cn" => current.trae_solo_cn_app_path = normalized_path,
            "workbuddy" => current.workbuddy_app_path = normalized_path,
            "opencode" => current.opencode_app_path = normalized_path,
            _ => return Err("未知应用类型".to_string()),
        }
        Ok(())
    })?;
    Ok(())
}

#[tauri::command]
pub fn set_claude_app_scan_roots(scan_roots: String) -> Result<(), String> {
    let normalized = scan_roots.trim().to_string();
    config::patch_user_config(move |current| {
        current.claude_app_scan_roots = normalized;
        Ok(())
    })?;
    Ok(())
}

#[tauri::command]
pub fn set_trae_app_scan_roots(app: Option<String>, scan_roots: String) -> Result<(), String> {
    let normalized = scan_roots.trim().to_string();
    let target = app
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("trae")
        .to_string();
    config::patch_user_config(move |current| {
        match target.as_str() {
            "trae" => current.trae_app_scan_roots = normalized,
            "trae_solo" => current.trae_solo_app_scan_roots = normalized,
            "trae_cn" => current.trae_cn_app_scan_roots = normalized,
            "trae_solo_cn" => current.trae_solo_cn_app_scan_roots = normalized,
            _ => return Err("鏈煡搴旂敤绫诲瀷".to_string()),
        }
        Ok(())
    })?;
    Ok(())
}

#[tauri::command]
pub fn set_codex_launch_on_switch(enabled: bool) -> Result<(), String> {
    config::patch_user_config(|current| {
        current.codex_launch_on_switch = enabled;
        Ok(())
    })?;
    Ok(())
}

#[tauri::command]
pub fn set_codex_local_access_entry_visible(enabled: bool) -> Result<(), String> {
    config::patch_user_config(|current| {
        current.codex_local_access_entry_visible = enabled;
        Ok(())
    })?;
    Ok(())
}

#[tauri::command]
pub fn detect_app_path(app: String, force: Option<bool>) -> Result<Option<String>, String> {
    let force = force.unwrap_or(false);
    match app.as_str() {
        "windsurf" => Ok(modules::windsurf_instance::detect_and_save_windsurf_launch_path(force)),
        "kiro" => Ok(modules::kiro_instance::detect_and_save_kiro_launch_path(
            force,
        )),
        "cursor" => Ok(modules::cursor_instance::detect_and_save_cursor_launch_path(force)),
        "claude" => Ok(modules::claude_instance::detect_and_save_claude_launch_path(force)),
        "antigravity" | "antigravity_ide" | "antigravity_legacy" | "codex" | "zed" | "vscode"
        | "codebuddy" | "codebuddy_cn" | "qoder" | "zcode" | "trae" | "trae_solo" | "trae_cn"
        | "trae_solo_cn" | "opencode" | "workbuddy" => Ok(
            modules::process::detect_and_save_app_path(app.as_str(), force),
        ),
        _ => Err("未知应用类型".to_string()),
    }
}

#[tauri::command]
pub async fn scan_claude_desktop_launch_targets(
    scan_roots: Option<String>,
) -> Result<Vec<modules::claude_instance::ClaudeDesktopLaunchCandidate>, String> {
    #[cfg(target_os = "windows")]
    {
        let _ = scan_roots;
        let task = tauri::async_runtime::spawn_blocking(|| {
            modules::process::scan_app_launch_targets("claude", None)
        });
        let candidates = match tokio::time::timeout(Duration::from_secs(2), task).await {
            Ok(Ok(result)) => result?,
            Ok(Err(error)) => return Err(format!("检测运行中的 Claude 任务失败: {error}")),
            Err(_) => return Err("检测运行中的 Claude 超时，请重试".to_string()),
        };
        return Ok(candidates
            .into_iter()
            .map(
                |candidate| modules::claude_instance::ClaudeDesktopLaunchCandidate {
                    target_type: candidate.target_type,
                    label: candidate.label,
                    target: candidate.target,
                    source: candidate.source,
                    supports_multi_instance: candidate.supports_multi_instance,
                },
            )
            .collect());
    }

    #[cfg(not(target_os = "windows"))]
    {
        let roots = scan_roots
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        Ok(modules::claude_instance::scan_claude_desktop_launch_targets(roots))
    }
}

#[tauri::command]
pub async fn scan_app_launch_targets(
    app: String,
    scan_roots: Option<String>,
) -> Result<Vec<modules::process::AppLaunchCandidate>, String> {
    match app.as_str() {
        "antigravity" | "antigravity_ide" | "antigravity_legacy" | "codex" | "claude"
        | "vscode" | "windsurf" | "kiro" | "cursor" | "codebuddy" | "codebuddy_cn" | "qoder"
        | "zcode" | "trae" | "trae_solo" | "trae_cn" | "trae_solo_cn" | "workbuddy" | "zed"
        | "opencode" => {}
        _ => return Err("未知应用类型".to_string()),
    }
    let _ = scan_roots;

    let task = tauri::async_runtime::spawn_blocking(move || {
        modules::process::scan_app_launch_targets(app.as_str(), None)
    });
    match tokio::time::timeout(Duration::from_secs(2), task).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => Err(format!("检测运行中的应用任务失败: {error}")),
        Err(_) => Err("检测运行中的应用超时，请重试".to_string()),
    }
}

#[tauri::command]
pub async fn get_antigravity_installed_version_info(
    target: Option<String>,
    scan_mode: Option<String>,
) -> Result<Option<AntigravityInstalledVersionInfo>, String> {
    let scan_mode = normalize_antigravity_version_scan_mode(scan_mode.as_deref());
    let timeout_ms = match scan_mode {
        AntigravityVersionScanMode::Quick => ANTIGRAVITY_VERSION_BADGE_TIMEOUT_MS,
        AntigravityVersionScanMode::Full => ANTIGRAVITY_VERSION_FULL_SCAN_TIMEOUT_MS,
    };
    let target_for_task = target.clone();

    let task = tauri::async_runtime::spawn_blocking(move || match scan_mode {
        AntigravityVersionScanMode::Quick => {
            resolve_antigravity_installed_version_info_quick_for_target(target_for_task.as_deref())
        }
        AntigravityVersionScanMode::Full => {
            resolve_antigravity_installed_version_info_for_target(target_for_task.as_deref())
        }
    });

    match tokio::time::timeout(Duration::from_millis(timeout_ms), task).await {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(error)) => Err(format!("Antigravity 版本检测任务失败: {}", error)),
        Err(_) => Ok(None),
    }
}

/// 通知插件关闭/开启唤醒功能（互斥）
#[tauri::command]
pub fn set_wakeup_override(enabled: bool) -> Result<(), String> {
    websocket::broadcast_wakeup_override(enabled);
    Ok(())
}

/// 执行窗口关闭操作
/// action: "minimize" | "quit"
/// remember: 是否记住选择
#[tauri::command]
pub fn handle_window_close(
    window: tauri::Window,
    action: String,
    remember: bool,
) -> Result<(), String> {
    modules::logger::log_info(&format!(
        "[Window] 用户选择: action={}, remember={}",
        action, remember
    ));

    // 如果需要记住选择，更新配置
    if remember {
        let close_behavior = match action.as_str() {
            "minimize" => CloseWindowBehavior::Minimize,
            "quit" => CloseWindowBehavior::Quit,
            _ => CloseWindowBehavior::Ask,
        };
        config::patch_user_config(move |current| {
            current.close_behavior = close_behavior;
            Ok(())
        })?;
        modules::logger::log_info(&format!("[Window] 已保存关闭行为设置: {}", action));
    }

    // 执行操作
    match action.as_str() {
        "minimize" => {
            if let Err(err) = modules::floating_card_window::destroy_main_window_to_tray(&window) {
                modules::logger::log_warn(&format!("[Window] 销毁主窗口失败，回退隐藏: {}", err));
                let _ = window.hide();
                modules::process_memory::trim_idle_process_memory();
            }
            modules::logger::log_info("[Window] 窗口已关闭到托盘");
        }
        "quit" => {
            modules::floating_card_window::request_app_exit();
            window.app_handle().exit(0);
        }
        _ => {
            return Err("无效的操作".to_string());
        }
    }

    Ok(())
}

#[tauri::command]
pub fn main_window_take_pending_navigation() -> Result<Option<String>, String> {
    modules::floating_card_window::take_pending_main_window_navigation()
}

#[tauri::command]
pub fn show_floating_card_window(app: tauri::AppHandle) -> Result<(), String> {
    modules::floating_card_window::show_floating_card_window(&app, true)
}

#[tauri::command]
pub fn show_instance_floating_card_window(
    app: tauri::AppHandle,
    context: modules::floating_card_window::FloatingCardInstanceContext,
) -> Result<(), String> {
    modules::floating_card_window::show_instance_floating_card_window(&app, context, true)
}

#[tauri::command]
pub fn get_floating_card_context(
    window_label: String,
) -> Result<Option<modules::floating_card_window::FloatingCardInstanceContext>, String> {
    modules::floating_card_window::get_floating_card_context(&window_label)
}

#[tauri::command]
pub fn hide_floating_card_window(app: tauri::AppHandle) -> Result<(), String> {
    modules::floating_card_window::hide_floating_card_window(&app, false)
}

#[tauri::command]
pub fn hide_current_floating_card_window(window: tauri::Window) -> Result<(), String> {
    window.hide().map_err(|err| err.to_string())
}

#[tauri::command]
pub fn set_floating_card_always_on_top(
    app: tauri::AppHandle,
    always_on_top: bool,
) -> Result<(), String> {
    config::patch_user_config(|current| {
        current.floating_card_always_on_top = always_on_top;
        Ok(())
    })?;
    modules::floating_card_window::apply_floating_card_always_on_top(&app)
}

#[tauri::command]
pub fn set_current_floating_card_window_always_on_top(
    window: tauri::Window,
    always_on_top: bool,
) -> Result<(), String> {
    window
        .set_always_on_top(always_on_top)
        .map_err(|err| err.to_string())
}

#[tauri::command]
pub fn set_floating_card_confirm_on_close(confirm_on_close: bool) -> Result<(), String> {
    config::patch_user_config(|current| {
        current.floating_card_confirm_on_close = confirm_on_close;
        Ok(())
    })?;
    Ok(())
}

#[tauri::command]
pub fn save_floating_card_position(x: i32, y: i32) -> Result<(), String> {
    config::patch_user_config(|current| {
        current.floating_card_position_x = Some(x);
        current.floating_card_position_y = Some(y);
        Ok(())
    })?;
    Ok(())
}

/// Must run window recreate on the UI/main thread. Sync invoke handlers run on a
/// worker pool; building a WebView there hangs on Windows after tray destroy.
#[tauri::command]
pub async fn show_main_window_and_navigate(
    app: tauri::AppHandle,
    page: String,
) -> Result<(), String> {
    modules::floating_card_window::show_main_window_and_navigate_async(app, page).await
}

#[tauri::command]
pub fn external_import_take_pending(
) -> Option<modules::external_import::ExternalProviderImportPayload> {
    modules::external_import::take_pending_external_import()
}

#[tauri::command]
pub async fn external_import_fetch_import_url(import_url: String) -> Result<String, String> {
    const MAX_IMPORT_BUNDLE_BYTES: usize = 8 * 1024 * 1024;

    let import_url = import_url.trim();
    if import_url.is_empty() {
        return Err("导入包地址为空".to_string());
    }

    let parsed = Url::parse(import_url).map_err(|err| format!("导入包地址无效: {}", err))?;
    if !matches!(parsed.scheme(), "https" | "http") {
        return Err("导入包地址仅支持 http/https".to_string());
    }

    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|err| format!("创建网络客户端失败: {}", err))?
        .get(parsed)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|err| format!("拉取导入包失败: {}", err))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("拉取导入包失败: HTTP {}", status.as_u16()));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|err| format!("读取导入包失败: {}", err))?;
    if bytes.len() > MAX_IMPORT_BUNDLE_BYTES {
        return Err("导入包过大".to_string());
    }

    String::from_utf8(bytes.to_vec()).map_err(|_| "导入包不是有效 UTF-8 文本".to_string())
}

/// 打开指定文件夹（如不存在则创建）
#[tauri::command]
pub async fn open_folder(path: String) -> Result<(), String> {
    let folder_path = std::path::Path::new(&path);

    // 如果目录不存在则创建
    if !folder_path.exists() {
        std::fs::create_dir_all(folder_path).map_err(|e| format!("创建文件夹失败: {}", e))?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("打开文件夹失败: {}", e))?;
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("打开文件夹失败: {}", e))?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("打开文件夹失败: {}", e))?;
    }

    Ok(())
}

/// 删除损坏的文件（会先备份）
#[tauri::command]
pub async fn delete_corrupted_file(path: String) -> Result<(), String> {
    let file_path = std::path::Path::new(&path);

    if !file_path.exists() {
        // 文件不存在，直接返回成功
        return Ok(());
    }

    // 创建备份文件名
    let timestamp = chrono::Utc::now().timestamp();
    let backup_name = format!("{}.corrupted.{}", path, timestamp);

    // 备份文件
    std::fs::rename(&path, &backup_name).map_err(|e| format!("备份损坏文件失败: {}", e))?;

    modules::logger::log_info(&format!(
        "已备份并删除损坏文件: {} -> {}",
        path, backup_name
    ));

    Ok(())
}

#[tauri::command]
pub fn load_user_memory() -> Result<modules::user_memory::UserMemory, String> {
    modules::user_memory::load_user_memory()
}

#[tauri::command]
pub fn mark_user_memory_dismissed(id: String) -> Result<modules::user_memory::UserMemory, String> {
    modules::user_memory::mark_user_memory_dismissed(&id)
}

#[tauri::command]
pub fn save_user_memory_list(
    id: String,
    items: Vec<String>,
) -> Result<modules::user_memory::UserMemory, String> {
    modules::user_memory::save_user_memory_list(&id, items)
}
