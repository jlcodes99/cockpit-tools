use tauri_plugin_autostart::ManagerExt as _;

use super::system::lock_general_config_transaction;
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
        "grok" => modules::grok_instance::load_instance_store(),
        "pi" => modules::pi_instance::load_instance_store(),
        "codebuddy" => modules::codebuddy_instance::load_instance_store(),
        "codebuddy_cn" => modules::codebuddy_cn_instance::load_instance_store(),
        "qoder" => modules::qoder_instance::load_instance_store(),
        "zcode" => modules::zcode_instance::load_instance_store(),
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
        "grok" => modules::grok_instance::save_instance_store(store),
        "pi" => modules::pi_instance::save_instance_store(store),
        "codebuddy" => modules::codebuddy_instance::save_instance_store(store),
        "codebuddy_cn" => modules::codebuddy_cn_instance::save_instance_store(store),
        "qoder" => modules::qoder_instance::save_instance_store(store),
        "zcode" => modules::zcode_instance::save_instance_store(store),
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
    let _transaction_guard = lock_general_config_transaction()?;
    let mut next_config = config;
    let desired_auto_launch = next_config.app_auto_launch_enabled;
    let cached_auto_launch = config::get_user_config().app_auto_launch_enabled;
    let previous_auto_launch = get_app_auto_launch_enabled(&app).unwrap_or(cached_auto_launch);
    let app_auto_launch_changed = previous_auto_launch != desired_auto_launch;
    if app_auto_launch_changed {
        apply_app_auto_launch_enabled(&app, desired_auto_launch)?;
    }

    let mut needs_restart = false;
    let mut language_changed = false;
    #[cfg(target_os = "macos")]
    let mut hide_dock_icon_changed = false;
    #[cfg(target_os = "macos")]
    let mut tray_icon_style_changed = false;

    let patch_result = config::patch_user_config(|current| {
        // 恢复备份配置时，以提交瞬间的值保留 WebDAV 配置和同步历史。
        next_config.webdav_sync_enabled = current.webdav_sync_enabled;
        next_config.webdav_sync_url = current.webdav_sync_url.clone();
        next_config.webdav_sync_username = current.webdav_sync_username.clone();
        next_config.webdav_sync_password = current.webdav_sync_password.clone();
        next_config.webdav_sync_remote_dir = current.webdav_sync_remote_dir.clone();
        next_config.webdav_sync_retention_days = current.webdav_sync_retention_days;
        next_config.webdav_sync_last_upload_at = current.webdav_sync_last_upload_at.clone();
        next_config.webdav_sync_last_upload_file_name =
            current.webdav_sync_last_upload_file_name.clone();
        next_config.webdav_sync_last_download_at = current.webdav_sync_last_download_at.clone();
        next_config.webdav_sync_last_download_file_name =
            current.webdav_sync_last_download_file_name.clone();

        needs_restart = current.ws_port != next_config.ws_port
            || current.ws_enabled != next_config.ws_enabled
            || current.report_enabled != next_config.report_enabled
            || current.report_port != next_config.report_port
            || current.report_token != next_config.report_token;
        language_changed = current.language != next_config.language;
        #[cfg(target_os = "macos")]
        {
            hide_dock_icon_changed = current.hide_dock_icon != next_config.hide_dock_icon;
            tray_icon_style_changed = current.tray_icon_style != next_config.tray_icon_style;
        }
        *current = next_config.clone();
        Ok(())
    });
    let next_config = match patch_result {
        Ok(config) => config,
        Err(error) => {
            if app_auto_launch_changed {
                if let Err(rollback_error) =
                    apply_app_auto_launch_enabled(&app, previous_auto_launch)
                {
                    modules::logger::log_error(&format!(
                        "[DataTransfer] 配置导入失败后回滚应用自启动状态失败: {}",
                        rollback_error
                    ));
                }
            }
            return Err(error);
        }
    };

    if let Err(err) = modules::floating_card_window::apply_floating_card_always_on_top(&app) {
        modules::logger::log_warn(&format!("[DataTransfer] 应用悬浮卡片置顶状态失败: {}", err));
    }

    #[cfg(target_os = "macos")]
    if hide_dock_icon_changed {
        crate::apply_macos_activation_policy(&app);
    }

    #[cfg(target_os = "macos")]
    if tray_icon_style_changed {
        if let Err(err) = modules::tray::apply_tray_icon_style(&app) {
            modules::logger::log_warn(&format!(
                "[DataTransfer] 应用 macOS 菜单栏图标样式失败: {}",
                err
            ));
        }
    }

    if language_changed {
        let normalized_language = next_config.language.clone();
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
