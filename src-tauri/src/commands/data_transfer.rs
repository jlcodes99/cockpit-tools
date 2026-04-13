use tauri_plugin_autostart::ManagerExt as _;

use crate::models::InstanceStore;
use crate::modules;
use crate::modules::config::{self, UserConfig};
use crate::modules::websocket;

fn get_app_auto_launch_enabled(app: &tauri::AppHandle) -> Result<bool, String> {
    app.autolaunch()
        .is_enabled()
        .map_err(|err| format!("读取应用自启动状态失败: {}", err))
}

fn apply_app_auto_launch_enabled(app: &tauri::AppHandle, enabled: bool) -> Result<(), String> {
    if enabled {
        app.autolaunch()
            .enable()
            .map_err(|err| format!("启用应用自启动失败: {}", err))
    } else {
        app.autolaunch()
            .disable()
            .map_err(|err| format!("停用应用自启动失败: {}", err))
    }
}

fn load_instance_store_by_platform(platform: &str) -> Result<InstanceStore, String> {
    match platform {
        "antigravity" => modules::instance::load_instance_store(),
        "codex" => modules::codex_instance::load_instance_store(),
        "github-copilot" => modules::github_copilot_instance::load_instance_store(),
        "windsurf" => modules::windsurf_instance::load_instance_store(),
        "kiro" => modules::kiro_instance::load_instance_store(),
        "cursor" => modules::cursor_instance::load_instance_store(),
        "gemini" => modules::gemini_instance::load_instance_store(),
        "codebuddy" => modules::codebuddy_instance::load_instance_store(),
        "codebuddy_cn" => modules::codebuddy_cn_instance::load_instance_store(),
        "qoder" => modules::qoder_instance::load_instance_store(),
        "trae" => modules::trae_instance::load_instance_store(),
        "workbuddy" => modules::workbuddy_instance::load_instance_store(),
        _ => Err("不支持的实例平台".to_string()),
    }
}

fn save_instance_store_by_platform(platform: &str, store: &InstanceStore) -> Result<(), String> {
    match platform {
        "antigravity" => modules::instance::save_instance_store(store),
        "codex" => modules::codex_instance::save_instance_store(store),
        "github-copilot" => modules::github_copilot_instance::save_instance_store(store),
        "windsurf" => modules::windsurf_instance::save_instance_store(store),
        "kiro" => modules::kiro_instance::save_instance_store(store),
        "cursor" => modules::cursor_instance::save_instance_store(store),
        "gemini" => modules::gemini_instance::save_instance_store(store),
        "codebuddy" => modules::codebuddy_instance::save_instance_store(store),
        "codebuddy_cn" => modules::codebuddy_cn_instance::save_instance_store(store),
        "qoder" => modules::qoder_instance::save_instance_store(store),
        "trae" => modules::trae_instance::save_instance_store(store),
        "workbuddy" => modules::workbuddy_instance::save_instance_store(store),
        _ => Err("不支持的实例平台".to_string()),
    }
}

fn sanitize_instance_store(store: &InstanceStore) -> InstanceStore {
    let mut next = store.clone();
    next.default_settings.last_pid = None;
    for instance in &mut next.instances {
        instance.last_pid = None;
        instance.last_launched_at = None;
    }
    next
}

#[tauri::command]
pub fn data_transfer_get_user_config() -> Result<UserConfig, String> {
    Ok(config::get_user_config())
}

#[tauri::command]
pub fn data_transfer_apply_user_config(
    app: tauri::AppHandle,
    config: UserConfig,
) -> Result<bool, String> {
    let current = config::get_user_config();
    let current_app_auto_launch_enabled =
        get_app_auto_launch_enabled(&app).unwrap_or(current.app_auto_launch_enabled);

    let needs_restart = current.ws_port != config.ws_port
        || current.ws_enabled != config.ws_enabled
        || current.report_enabled != config.report_enabled
        || current.report_port != config.report_port
        || current.report_token != config.report_token;
    let language_changed = current.language != config.language;
    let app_auto_launch_changed = current_app_auto_launch_enabled != config.app_auto_launch_enabled;

    #[cfg(target_os = "macos")]
    let hide_dock_icon_changed = current.hide_dock_icon != config.hide_dock_icon;

    config::save_user_config(&config)?;

    if app_auto_launch_changed {
        apply_app_auto_launch_enabled(&app, config.app_auto_launch_enabled)?;
    }

    if let Err(err) = modules::floating_card_window::apply_floating_card_always_on_top(&app) {
        modules::logger::log_warn(&format!("[DataTransfer] 应用悬浮卡片置顶状态失败: {}", err));
    }

    #[cfg(target_os = "macos")]
    if hide_dock_icon_changed {
        crate::apply_macos_activation_policy(&app);
    }

    if language_changed {
        let normalized_language = config.language.clone();
        websocket::broadcast_language_changed(&normalized_language, "desktop");
        modules::sync_settings::write_sync_setting("language", &normalized_language);
        if let Err(err) = modules::tray::update_tray_menu(&app) {
            modules::logger::log_warn(&format!("[DataTransfer] 语言变更后刷新托盘失败: {}", err));
        }
    }

    Ok(needs_restart)
}

#[tauri::command]
pub fn data_transfer_get_instance_store(platform: String) -> Result<InstanceStore, String> {
    load_instance_store_by_platform(platform.trim())
}

#[tauri::command]
pub fn data_transfer_replace_instance_store(
    platform: String,
    store: InstanceStore,
) -> Result<(), String> {
    let sanitized = sanitize_instance_store(&store);
    save_instance_store_by_platform(platform.trim(), &sanitized)
}
