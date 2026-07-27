mod commands;
pub mod error;
mod models;
mod modules;
mod utils;

use modules::config::CloseWindowBehavior;
use modules::logger;
use std::sync::OnceLock;
#[cfg(target_os = "macos")]
use tauri::ActivationPolicy;
use tauri::RunEvent;
use tauri::WindowEvent;
use tauri::{Emitter, Manager};
use tauri_plugin_deep_link::DeepLinkExt;
use tracing::info;

/// 全局 AppHandle 存储
static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();

/// 获取全局 AppHandle
pub fn get_app_handle() -> Option<&'static tauri::AppHandle> {
    APP_HANDLE.get()
}

#[cfg(test)]
mod tests {
    use super::should_hide_startup_minimized_window;
    use crate::modules::config::UserConfig;

    #[test]
    fn startup_minimized_does_not_hide_when_disabled() {
        let mut config = UserConfig::default();
        config.startup_minimized = false;
        config.hide_dock_icon = true;

        assert!(!should_hide_startup_minimized_window(&config, true));
    }

    #[test]
    fn startup_minimized_hides_on_macos_when_dock_icon_is_hidden() {
        let mut config = UserConfig::default();
        config.startup_minimized = true;
        config.hide_dock_icon = true;

        assert!(should_hide_startup_minimized_window(&config, true));
    }

    #[test]
    fn startup_minimized_does_not_hide_when_dock_icon_is_available() {
        let mut config = UserConfig::default();
        config.startup_minimized = true;
        config.hide_dock_icon = false;

        assert!(!should_hide_startup_minimized_window(&config, true));
    }

    #[test]
    fn startup_minimized_does_not_wait_before_hiding_window() {
        let source = include_str!("lib.rs");
        let delayed_startup_hide = concat!(
            "std::thread::sleep",
            "(std::time::Duration::from_millis(300))"
        );

        assert!(!source.contains(delayed_startup_hide));
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn raise_process_file_descriptor_limit() {
    const TARGET_NOFILE_LIMIT: libc::rlim_t = 4096;

    unsafe {
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) != 0 {
            logger::log_warn(&format!(
                "[Startup] 读取进程文件句柄上限失败: {}",
                std::io::Error::last_os_error()
            ));
            return;
        }

        let target = if limit.rlim_max == libc::RLIM_INFINITY {
            TARGET_NOFILE_LIMIT
        } else {
            TARGET_NOFILE_LIMIT.min(limit.rlim_max)
        };
        if target <= limit.rlim_cur || target == 0 {
            return;
        }

        let previous = limit.rlim_cur;
        limit.rlim_cur = target;
        if libc::setrlimit(libc::RLIMIT_NOFILE, &limit) == 0 {
            logger::log_info(&format!(
                "[Startup] 已提升进程文件句柄软限制: {} -> {}",
                previous, target
            ));
        } else {
            logger::log_warn(&format!(
                "[Startup] 提升进程文件句柄软限制失败: {} -> {}, error={}",
                previous,
                target,
                std::io::Error::last_os_error()
            ));
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn raise_process_file_descriptor_limit() {}

fn should_hide_startup_minimized_window(
    config: &modules::config::UserConfig,
    is_macos: bool,
) -> bool {
    config.startup_minimized && is_macos && config.hide_dock_icon
}

fn apply_startup_minimized(app: &tauri::AppHandle) {
    let config = modules::config::get_user_config();
    if !config.startup_minimized {
        return;
    }

    let should_hide = should_hide_startup_minimized_window(&config, cfg!(target_os = "macos"));
    let Some(window) = app.get_webview_window("main") else {
        logger::log_warn("[Window] 启动后自动最小化失败: main window not found");
        return;
    };

    let (result, action_label) = if should_hide {
        (window.hide(), "隐藏")
    } else {
        (window.minimize(), "最小化")
    };

    match result {
        Ok(()) => logger::log_info(&format!("[Window] 启动后已自动{}主窗口", action_label)),
        Err(err) => logger::log_warn(&format!("[Window] 启动后自动最小化失败: {}", err)),
    }
}

#[cfg(target_os = "macos")]
fn apply_macos_activation_policy(app: &tauri::AppHandle) {
    let config = modules::config::get_user_config();
    let (policy, dock_visible, policy_label) = if config.hide_dock_icon {
        (ActivationPolicy::Accessory, false, "hidden")
    } else {
        (ActivationPolicy::Regular, true, "visible")
    };

    if let Err(err) = app.set_activation_policy(policy) {
        logger::log_warn(&format!("[Window] 设置 macOS 激活策略失败: {}", err));
        return;
    }

    if let Err(err) = app.set_dock_visibility(dock_visible) {
        logger::log_warn(&format!("[Window] 设置 macOS Dock 可见性失败: {}", err));
    }

    if dock_visible {
        let _ = app.show();
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
        }
    }

    info!("[Window] 已应用 macOS Dock 图标策略: {}", policy_label);
}

fn handle_zcode_oauth_deep_links(args: &[String]) -> bool {
    let callbacks: Vec<String> = args
        .iter()
        .filter(|value| value.trim().to_ascii_lowercase().starts_with("zcode://"))
        .cloned()
        .collect();
    if callbacks.is_empty() {
        return false;
    }
    for callback in callbacks {
        tauri::async_runtime::spawn(async move {
            modules::zcode_oauth::handle_deep_link(&callback).await;
        });
    }
    true
}

fn summarize_deep_link_args(args: &[String]) -> Vec<String> {
    args.iter()
        .map(|value| {
            if value.trim().to_ascii_lowercase().starts_with("zcode://") {
                "zcode://<oauth-callback>".to_string()
            } else {
                value.clone()
            }
        })
        .collect()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    logger::init_logger();
    modules::diagnostics::install_panic_hook();
    modules::diagnostics::start_frontend_ready_watchdog();
    raise_process_file_descriptor_limit();
    // 启动时先加载一次配置，确保进程级代理环境与用户设置同步。
    let _ = modules::config::get_user_config();

    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
            logger::log_info("[Linux] 设置 WEBKIT_DISABLE_DMABUF_RENDERER=1");
        }
    }

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            logger::log_info(&format!(
                "[SingleInstance] 收到唤起请求: arg_count={}",
                args.len()
            ));
            let zcode_oauth_handled = handle_zcode_oauth_deep_links(&args);
            let handled = zcode_oauth_handled
                || modules::external_import::handle_external_import_args(
                    app,
                    &args,
                    "single-instance",
                );
            logger::log_info(&format!(
                "[SingleInstance] 外部导入处理结果: handled={}",
                handled
            ));
            if handled {
                return;
            }
            if let Err(err) = modules::floating_card_window::show_main_window(app) {
                logger::log_warn(&format!("[Window] 单实例唤起恢复主窗口失败: {}", err));
            }
        }))
        .setup(|app| {
            info!("Cockpit Tools 启动...");
            let current_exe = std::env::current_exe()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|err| format!("unknown: {}", err));
            let build_mode = if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            };
            logger::log_info(&format!(
                "[Startup] 启动诊断: marker=tray-diagnostics-v1, version={}, mode={}, exe={}",
                env!("CARGO_PKG_VERSION"),
                build_mode,
                current_exe
            ));

            // 存储全局 AppHandle
            let _ = APP_HANDLE.set(app.handle().clone());

            if let Err(err) = modules::app_lifecycle::install_system_shutdown_listener() {
                logger::log_warn(&format!("[Lifecycle] 安装系统关机监听失败: {}", err));
            }

            // 启动时清理 WebKit LocalStorage WAL，防止无限膨胀
            std::thread::spawn(|| {
                modules::webkit_cache_maintenance::checkpoint_webkit_localstorage();
            });

            // 当前主线不再使用 platform-packages；启动时回收旧版本遗留的孤儿 adapter。
            std::thread::spawn(|| {
                match modules::process::close_orphaned_legacy_platform_adapter_processes(5) {
                    Ok(0) => {}
                    Ok(count) => logger::log_info(&format!(
                        "[LegacyAdapterCleanup] 已清理旧平台 adapter 进程: count={}",
                        count
                    )),
                    Err(err) => logger::log_warn(&format!(
                        "[LegacyAdapterCleanup] 清理旧平台 adapter 进程失败: {}",
                        err
                    )),
                }
            });

            // 初始化 Updater 插件
            #[cfg(desktop)]
            {
                app.handle()
                    .plugin(tauri_plugin_updater::Builder::new().build())?;
                app.handle().plugin(tauri_plugin_process::init())?;
                app.handle().plugin(tauri_plugin_autostart::init(
                    tauri_plugin_autostart::MacosLauncher::LaunchAgent,
                    None::<Vec<&'static str>>,
                ))?;
                info!("[Updater] Tauri Updater + Process 插件已初始化");
            }

            // 启动时同步设置合并（移至后台线程，不阻塞窗口显示）
            std::thread::spawn(|| {
                let current_config = modules::config::get_user_config();
                if let Some(merged_language) = modules::sync_settings::merge_setting_on_startup(
                    "language",
                    &current_config.language,
                    None,
                ) {
                    info!(
                        "[SyncSettings] 启动时合并语言设置: {} -> {}",
                        current_config.language, merged_language
                    );
                    if let Err(e) = modules::config::patch_user_config(|config| {
                        config.language = merged_language;
                        Ok(())
                    }) {
                        logger::log_error(&format!("[SyncSettings] 保存合并后的配置失败: {}", e));
                    }
                }
            });

            // 启动 WebSocket 服务（使用 Tauri 的 async runtime）
            tauri::async_runtime::spawn(async {
                modules::websocket::start_server().await;
            });

            // 启动网页查询服务（网络服务配置中的独立模块）
            tauri::async_runtime::spawn(async {
                modules::web_report::start_server().await;
            });

            tauri::async_runtime::spawn(async {
                modules::codex_local_access::restore_local_access_gateway().await;
            });

            {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    modules::codex_oauth::restore_pending_oauth_listener(app_handle);
                    modules::windsurf_oauth::restore_pending_oauth_listener();
                    modules::kiro_oauth::restore_pending_oauth_listener();
                    modules::trae_oauth::restore_pending_oauth_listener();
                    modules::zed_oauth::restore_pending_oauth_listener();
                });
            }

            modules::provider_token_keeper::ensure_started(app.handle().clone());
            modules::auto_local_import::ensure_started(app.handle().clone());

            // Wakeup restore/start and Deep Link registration/read can hit disk or OS
            // APIs — never block setup (window + skeleton tray first).
            {
                let app_handle = app.handle().clone();
                std::thread::spawn(move || {
                    modules::wakeup_scheduler::restore_state_from_disk();
                    modules::wakeup_scheduler::ensure_started(app_handle.clone());
                    modules::codex_wakeup_scheduler::ensure_started(app_handle.clone());
                    modules::codex_wakeup_scheduler::trigger_startup_tasks_if_needed(app_handle);
                });
            }

            #[cfg(target_os = "macos")]
            apply_macos_activation_policy(&app.handle());

            #[cfg(any(windows, target_os = "linux"))]
            {
                let app_handle = app.handle().clone();
                std::thread::spawn(move || {
                    if let Err(err) = app_handle.deep_link().register_all() {
                        logger::log_warn(&format!("[DeepLink] register_all 失败: {}", err));
                    } else {
                        logger::log_info("[DeepLink] register_all 已完成");
                    }
                });
            }

            {
                let app_handle = app.handle().clone();
                app.deep_link().on_open_url(move |event| {
                    let urls = event.urls();
                    let args: Vec<String> = urls.iter().map(|url| url.to_string()).collect();
                    logger::log_info(&format!(
                        "[DeepLink] 收到 on_open_url 事件: url_count={}, urls={:?}",
                        args.len(),
                        summarize_deep_link_args(&args)
                    ));
                    let zcode_oauth_handled = handle_zcode_oauth_deep_links(&args);
                    let handled = zcode_oauth_handled
                        || modules::external_import::handle_external_import_args(
                            &app_handle,
                            &args,
                            "deep-link-open-url",
                        );
                    logger::log_info(&format!(
                        "[DeepLink] on_open_url 外部导入处理结果: handled={}",
                        handled
                    ));
                });
            }

            {
                let app_handle = app.handle().clone();
                std::thread::spawn(move || match app_handle.deep_link().get_current() {
                    Ok(Some(urls)) => {
                        let args: Vec<String> = urls.iter().map(|url| url.to_string()).collect();
                        logger::log_info(&format!(
                            "[DeepLink] 启动时 get_current 命中: url_count={}, urls={:?}",
                            args.len(),
                            summarize_deep_link_args(&args)
                        ));
                        let zcode_oauth_handled = handle_zcode_oauth_deep_links(&args);
                        let handled = zcode_oauth_handled
                            || modules::external_import::handle_external_import_args(
                                &app_handle,
                                &args,
                                "deep-link-current",
                            );
                        logger::log_info(&format!(
                            "[DeepLink] get_current 外部导入处理结果: handled={}",
                            handled
                        ));
                    }
                    Ok(None) => {
                        logger::log_info("[DeepLink] 启动时 get_current: empty");
                    }
                    Err(err) => {
                        logger::log_warn(&format!("[DeepLink] get_current 失败: {}", err));
                    }
                });
            }

            // 创建骨架托盘（无账号文件 I/O，秒出）
            if let Err(e) = modules::tray::create_tray_skeleton(app.handle()) {
                logger::log_error(&format!("[Tray] 创建骨架托盘失败: {}", e));
            }

            #[cfg(target_os = "macos")]
            {
                let tray_app_handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(800));
                    if let Err(err) = modules::tray::apply_tray_icon_style(&tray_app_handle) {
                        logger::log_warn(&format!(
                            "[Tray] macOS 启动后重应用菜单栏图标样式失败: {}",
                            err
                        ));
                    }
                });
            }

            // 后台线程加载完整托盘菜单（含账号数据）
            let tray_app_handle = app.handle().clone();
            std::thread::spawn(move || {
                if let Err(e) = modules::tray::update_tray_menu(&tray_app_handle) {
                    logger::log_error(&format!("[Tray] 后台更新托盘菜单失败: {}", e));
                }
            });

            if let Err(err) =
                modules::floating_card_window::show_floating_card_window_on_startup(&app.handle())
            {
                logger::log_warn(&format!("[FloatingCard] 启动时显示悬浮卡片失败: {}", err));
            }

            let startup_args: Vec<String> = std::env::args().collect();
            logger::log_info(&format!("[Startup] 启动参数数量: {}", startup_args.len()));
            let startup_external_import_handled =
                modules::external_import::handle_external_import_args(
                    &app.handle(),
                    &startup_args,
                    "startup",
                );
            logger::log_info(&format!(
                "[Startup] 外部导入处理结果: handled={}",
                startup_external_import_handled
            ));

            // Restore last main-window size/position before optional startup minimize (#948 / #1132).
            if let Some(main) = app.get_webview_window("main") {
                modules::main_window_state::restore_to_window(&main);
            }

            apply_startup_minimized(&app.handle());

            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                if window.label() != "main" {
                    return;
                }
                // Always snapshot geometry before close / tray-destroy / quit.
                modules::main_window_state::capture_and_save_from_window_handle(window);
                let config = modules::config::get_user_config();

                match config.close_behavior {
                    CloseWindowBehavior::Minimize => {
                        api.prevent_close();
                        // Full #686 behavior: destroy main WebView, keep tray process alive.
                        if let Err(err) =
                            modules::floating_card_window::destroy_main_window_to_tray(window)
                        {
                            modules::logger::log_warn(&format!(
                                "[Window] 销毁主窗口 WebView 失败，回退为隐藏: {}",
                                err
                            ));
                            let _ = window.hide();
                            modules::process_memory::trim_idle_process_memory();
                        }
                        info!("[Window] 窗口已关闭到托盘");
                    }
                    CloseWindowBehavior::Quit => {
                        modules::floating_card_window::request_app_exit();
                        info!("[Window] 用户选择退出应用");
                        window.app_handle().exit(0);
                    }
                    CloseWindowBehavior::Ask => {
                        api.prevent_close();
                        let _ = window.emit("window:close_requested", ());
                        info!("[Window] 等待用户选择关闭行为");
                    }
                }
            }
            WindowEvent::Resized(_) | WindowEvent::Moved(_) => {
                if window.label() == "main" {
                    modules::main_window_state::capture_and_save_from_window_handle_debounced(
                        window,
                    );
                }
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            // Account Commands
            commands::account::list_accounts,
            commands::account::add_account,
            commands::account::delete_account,
            commands::account::delete_accounts,
            commands::account::reorder_accounts,
            commands::account::get_current_account,
            commands::account::set_current_account,
            commands::account::fetch_account_quota,
            commands::account::refresh_all_quotas,
            commands::account::refresh_current_quota,
            commands::account::switch_account,
            commands::account::load_antigravity_switch_history,
            commands::account::clear_antigravity_switch_history,
            commands::account::update_account_tags,
            commands::account::update_account_notes,
            commands::account::load_account_groups,
            commands::account::save_account_groups,
            commands::account::sync_current_from_client,
            commands::account::sync_from_extension,
            // Device Commands
            // OAuth Commands
            commands::oauth::start_oauth_login,
            commands::oauth::prepare_oauth_url,
            commands::oauth::complete_oauth_login,
            commands::oauth::submit_oauth_callback_url,
            commands::oauth::cancel_oauth_login,
            // Import/Export Commands
            commands::import::import_from_old_tools,
            commands::import::import_from_local,
            commands::import::import_from_json,
            commands::import::import_from_files,
            commands::import::export_accounts,
            commands::data_transfer::data_transfer_get_user_config,
            commands::data_transfer::data_transfer_apply_user_config,
            commands::data_transfer::data_transfer_get_instance_store,
            commands::data_transfer::data_transfer_replace_instance_store,
            commands::provider_current::get_provider_current_account_id,
            // Claude Commands
            commands::claude::list_claude_accounts,
            commands::claude::delete_claude_account,
            commands::claude::delete_claude_accounts,
            commands::claude::import_claude_from_json,
            commands::claude::import_claude_api_key,
            commands::claude::import_claude_desktop_gateway,
            commands::claude::update_claude_desktop_gateway,
            commands::claude::claude_desktop_gateway_list_models,
            commands::claude::claude_oauth_login_prepare,
            commands::claude::claude_oauth_login_start,
            commands::claude::claude_oauth_login_complete,
            commands::claude::claude_oauth_login_cancel,
            commands::claude::import_claude_cli_from_local,
            commands::claude::claude_desktop_login_start,
            commands::claude::claude_desktop_login_complete,
            commands::claude::claude_desktop_login_cancel,
            commands::claude::claude_open_verification_window,
            commands::claude::export_claude_accounts,
            commands::claude::refresh_claude_quota,
            commands::claude::refresh_all_claude_quotas,
            commands::claude::update_claude_account_tags,
            commands::claude::update_claude_account_plan,
            commands::claude::update_claude_account_note,
            commands::claude::get_claude_accounts_index_path,
            commands::claude::claude_get_cli_launch_command,
            commands::claude::claude_execute_cli_launch_command,
            commands::claude::claude_launch_cli,
            commands::claude::switch_claude_account,
            // Claude Instance Commands
            commands::claude_instance::claude_get_instance_defaults,
            commands::claude_instance::claude_list_instances,
            commands::claude_instance::claude_create_instance,
            commands::claude_instance::claude_update_instance,
            commands::claude_instance::claude_delete_instance,
            commands::claude_instance::claude_start_instance,
            commands::claude_instance::claude_stop_instance,
            commands::claude_instance::claude_open_instance_window,
            commands::claude_instance::claude_close_all_instances,
            commands::claude_instance::claude_get_instance_launch_command,
            commands::claude_instance::claude_execute_instance_launch_command,
            // System Commands
            commands::system::open_data_folder,
            commands::system::open_local_path,
            commands::system::save_text_file,
            commands::system::get_downloads_dir,
            commands::system::get_auto_backup_settings,
            commands::system::save_auto_backup_settings,
            commands::system::update_auto_backup_last_run,
            commands::system::write_auto_backup_file,
            commands::system::read_auto_backup_file,
            commands::system::copy_auto_backup_file,
            commands::system::list_auto_backup_files,
            commands::system::delete_auto_backup_file,
            commands::system::cleanup_auto_backup_files,
            commands::system::open_auto_backup_dir,
            commands::system::get_webdav_sync_settings,
            commands::system::save_webdav_sync_settings,
            commands::system::test_webdav_sync_connection,
            commands::system::upload_auto_backup_to_webdav,
            commands::system::list_webdav_backup_files,
            commands::system::read_webdav_backup_file,
            commands::system::delete_webdav_backup_file,
            commands::system::get_network_config,
            commands::system::save_network_config,
            commands::system::get_diagnostics_config,
            commands::system::save_diagnostics_config,
            commands::system::diagnostics_frontend_stage,
            commands::system::diagnostics_frontend_ready,
            commands::system::diagnostics_capture_event,
            commands::system::get_general_config,
            commands::system::get_available_terminals,
            commands::system::patch_general_config,
            commands::system::scan_auto_local_import,
            commands::system::codex_ssh_list_servers,
            commands::system::codex_ssh_upsert_server,
            commands::system::codex_ssh_delete_server,
            commands::system::codex_ssh_select_server,
            commands::system::codex_ssh_test_connection,
            commands::system::codex_ssh_sync_current,
            commands::system::codex_managed_lb_provider_id,
            commands::system::codebuddy_list_local_session_files,
            commands::system::save_refresh_interval_config,
            commands::system::save_tray_platform_layout,
            commands::system::set_app_path,
            commands::system::set_claude_app_scan_roots,
            commands::system::set_trae_app_scan_roots,
            commands::system::set_codex_launch_on_switch,
            commands::system::set_codex_local_access_entry_visible,
            commands::system::detect_app_path,
            commands::system::scan_claude_desktop_launch_targets,
            commands::system::scan_app_launch_targets,
            commands::system::get_antigravity_installed_version_info,
            commands::system::set_wakeup_override,
            commands::system::handle_window_close,
            commands::system::main_window_take_pending_navigation,
            commands::system::show_floating_card_window,
            commands::system::show_instance_floating_card_window,
            commands::system::get_floating_card_context,
            commands::system::hide_floating_card_window,
            commands::system::hide_current_floating_card_window,
            commands::system::set_floating_card_always_on_top,
            commands::system::set_current_floating_card_window_always_on_top,
            commands::system::set_floating_card_confirm_on_close,
            commands::system::save_floating_card_position,
            commands::system::show_main_window_and_navigate,
            commands::system::external_import_take_pending,
            commands::system::external_import_fetch_import_url,
            commands::system::open_folder,
            commands::system::delete_corrupted_file,
            // Logs Commands
            commands::logs::logs_get_snapshot,
            commands::logs::logs_open_log_directory,
            // Wakeup Commands
            commands::wakeup::wakeup_ensure_runtime_ready,
            commands::wakeup::wakeup_set_official_ls_version_mode,
            commands::wakeup::trigger_wakeup,
            commands::wakeup::fetch_available_models,
            commands::wakeup::wakeup_validate_crontab,
            commands::wakeup::wakeup_sync_state,
            commands::wakeup::wakeup_run_enabled_tasks,
            commands::wakeup::wakeup_load_history,
            commands::wakeup::wakeup_add_history,
            commands::wakeup::wakeup_clear_history,
            commands::wakeup::wakeup_verification_load_state,
            commands::wakeup::wakeup_verification_load_history,
            commands::wakeup::wakeup_verification_delete_history,
            commands::wakeup::wakeup_verification_run_batch,
            commands::wakeup::confirm_wakeup_task,
            commands::wakeup::cancel_wakeup_task,
            commands::wakeup::check_wakeup_timeouts,
            // Update Commands
            commands::update::should_check_updates,
            commands::update::update_last_check_time,
            commands::update::get_update_settings,
            commands::update::patch_update_settings,
            commands::update::save_pending_update_notes,
            commands::update::check_version_jump,
            commands::update::get_release_history,
            commands::update::update_log,
            commands::update::get_update_runtime_info,
            commands::update::install_linux_update,
            // Announcement Commands
            commands::announcement::announcement_get_state,
            commands::announcement::announcement_mark_as_read,
            commands::announcement::announcement_mark_all_as_read,
            commands::announcement::announcement_force_refresh,
            commands::announcement::announcement_get_top_right_ad,
            commands::announcement::announcement_force_refresh_top_right_ad,
            commands::announcement::announcement_get_sponsor_module,
            commands::announcement::announcement_force_refresh_sponsor_module,
            commands::remote_config::remote_config_get_state,
            commands::remote_config::remote_config_force_refresh,
            // Group Commands
            commands::group::get_group_settings,
            commands::group::save_group_settings,
            commands::group::set_model_group,
            commands::group::remove_model_group,
            commands::group::set_group_name,
            commands::group::delete_group,
            commands::group::update_group_order,
            commands::group::get_display_groups,
            // Codex Commands
            commands::codex::list_codex_accounts,
            commands::codex::get_current_codex_account,
            commands::codex::get_codex_config_toml_path,
            commands::codex::open_codex_config_toml,
            commands::codex::get_codex_quick_config,
            commands::codex::save_codex_quick_config,
            commands::codex::get_codex_app_speed_config,
            commands::codex::save_codex_app_speed,
            commands::codex::get_codex_api_service_app_speed_config,
            commands::codex::save_codex_api_service_app_speed,
            commands::codex::update_codex_account_app_speed,
            commands::codex::refresh_codex_account_profile,
            commands::codex::switch_codex_account,
            commands::codex::delete_codex_account,
            commands::codex::delete_codex_accounts,
            commands::codex::start_codex_batch_delete,
            commands::codex::get_codex_batch_delete,
            commands::codex::resume_codex_batch_delete,
            commands::codex::pause_codex_batch_delete,
            commands::codex::retry_failed_codex_batch_delete,
            commands::codex::clear_codex_batch_delete,
            commands::codex::import_codex_access_token_account,
            commands::codex::import_codex_from_local,
            commands::codex::import_codex_from_json,
            commands::codex::export_codex_accounts,
            commands::codex::import_codex_from_files,
            commands::codex::start_codex_batch_import_from_files,
            commands::codex::cancel_codex_batch_import,
            commands::codex::resume_codex_batch_import,
            commands::codex::get_codex_batch_import_preview,
            commands::codex::confirm_codex_batch_import,
            commands::codex::refresh_codex_quota,
            commands::codex::refresh_codex_quotas_batch,
            commands::codex::get_codex_reset_credits,
            commands::codex::consume_codex_reset_credit,
            commands::codex::refresh_codex_subscription_info,
            commands::codex::refresh_all_codex_quotas,
            commands::codex::refresh_current_codex_quota,
            commands::codex::codex_oauth_login_start,
            commands::codex::codex_oauth_open_incognito_window,
            commands::codex::codex_oauth_login_completed,
            commands::codex::codex_oauth_submit_callback_url,
            commands::codex::codex_oauth_login_cancel,
            commands::codex::add_codex_account_with_token,
            commands::codex::add_codex_account_with_api_key,
            commands::codex::update_codex_account_name,
            commands::codex::update_codex_api_key_credentials,
            commands::codex::sync_codex_api_key_provider_accounts,
            commands::codex::update_codex_api_key_bound_oauth_account,
            commands::codex::is_codex_oauth_port_in_use,
            commands::codex::close_codex_oauth_port,
            commands::codex::update_codex_account_tags,
            commands::codex::update_codex_account_note,
            commands::codex::create_pending_codex_oauth_account,
            commands::codex::fetch_codex_account_note_mail_url,
            commands::codex::codex_wakeup_get_cli_status,
            commands::codex::codex_wakeup_update_runtime_config,
            commands::codex::codex_wakeup_get_overview,
            commands::codex::codex_wakeup_get_state,
            commands::codex::codex_wakeup_save_state,
            commands::codex::codex_wakeup_load_history,
            commands::codex::codex_wakeup_clear_history,
            commands::codex::codex_wakeup_cancel_scope,
            commands::codex::codex_wakeup_release_scope,
            commands::codex::codex_wakeup_test,
            commands::codex::codex_wakeup_run_task,
            commands::codex::codex_wakeup_run_enabled_tasks,
            commands::codex::load_codex_account_groups,
            commands::codex::save_codex_account_groups,
            commands::codex::load_codex_model_providers,
            commands::codex::save_codex_model_providers,
            commands::codex::codex_test_model_provider_connection,
            commands::codex::codex_model_provider_chat_test_batch,
            commands::codex::codex_cancel_model_provider_chat_test,
            commands::codex::codex_list_model_provider_models,
            commands::codex::codex_query_model_provider_usage,
            commands::codex::codex_local_access_get_state,
            commands::codex::codex_local_access_save_accounts,
            commands::codex::codex_local_access_append_accounts,
            commands::codex::codex_local_access_remove_account,
            commands::codex::codex_local_access_recover_accounts,
            commands::codex::codex_local_access_rotate_api_key,
            commands::codex::codex_local_access_update_bound_oauth_account,
            commands::codex::codex_local_access_clear_stats,
            commands::codex::codex_local_access_query_stats,
            commands::codex::codex_local_access_query_request_logs,
            commands::codex::codex_local_access_prepare_restart,
            commands::codex::codex_local_access_kill_port,
            commands::codex::codex_local_access_update_port,
            commands::codex::codex_local_access_update_routing_strategy,
            commands::codex::codex_local_access_update_custom_routing,
            commands::codex::codex_local_access_update_account_model_rules,
            commands::codex::codex_local_access_update_model_rules,
            commands::codex::codex_local_access_update_model_pricings,
            commands::codex::codex_local_access_reprice_request_logs,
            commands::codex::codex_local_access_update_routing_options,
            commands::codex::codex_local_access_update_timeouts,
            commands::codex::codex_local_access_update_timeout_presets,
            commands::codex::codex_local_access_update_upstream_proxy_config,
            commands::codex::codex_local_access_update_gateway_mode,
            commands::codex::codex_local_access_update_debug_logs,
            commands::codex::codex_local_access_update_access_scope,
            commands::codex::codex_local_access_update_client_base_url_host,
            commands::codex::codex_local_access_create_api_key,
            commands::codex::codex_local_access_update_api_key,
            commands::codex::codex_local_access_set_api_key_account_priority,
            commands::codex::codex_local_access_rotate_named_api_key,
            commands::codex::codex_local_access_delete_api_key,
            commands::codex::codex_local_access_set_enabled,
            commands::codex::codex_local_access_activate,
            commands::codex::codex_local_access_test,
            commands::codex::codex_local_access_chat_test,
            commands::codex::codex_local_access_chat_test_stream,
            // GitHub Copilot Commands
            commands::github_copilot::list_github_copilot_accounts,
            commands::github_copilot::delete_github_copilot_account,
            commands::github_copilot::delete_github_copilot_accounts,
            commands::github_copilot::import_github_copilot_from_json,
            commands::github_copilot::import_github_copilot_from_local,
            commands::github_copilot::export_github_copilot_accounts,
            commands::github_copilot::refresh_github_copilot_token,
            commands::github_copilot::refresh_all_github_copilot_tokens,
            commands::github_copilot::github_copilot_oauth_login_start,
            commands::github_copilot::github_copilot_oauth_login_complete,
            commands::github_copilot::github_copilot_oauth_login_cancel,
            commands::github_copilot::add_github_copilot_account_with_token,
            commands::github_copilot::update_github_copilot_account_tags,
            commands::github_copilot::get_github_copilot_accounts_index_path,
            commands::github_copilot::inject_github_copilot_to_vscode,
            // GitHub Copilot Instance Commands
            commands::github_copilot_instance::github_copilot_get_instance_defaults,
            commands::github_copilot_instance::github_copilot_list_instances,
            commands::github_copilot_instance::github_copilot_create_instance,
            commands::github_copilot_instance::github_copilot_update_instance,
            commands::github_copilot_instance::github_copilot_delete_instance,
            commands::github_copilot_instance::github_copilot_start_instance,
            commands::github_copilot_instance::github_copilot_stop_instance,
            commands::github_copilot_instance::github_copilot_open_instance_window,
            commands::github_copilot_instance::github_copilot_close_all_instances,
            // Windsurf Commands
            commands::windsurf::list_windsurf_accounts,
            commands::windsurf::delete_windsurf_account,
            commands::windsurf::delete_windsurf_accounts,
            commands::windsurf::import_windsurf_from_json,
            commands::windsurf::import_windsurf_from_local,
            commands::windsurf::export_windsurf_accounts,
            commands::windsurf::refresh_windsurf_token,
            commands::windsurf::refresh_all_windsurf_tokens,
            commands::windsurf::windsurf_oauth_login_start,
            commands::windsurf::windsurf_oauth_login_complete,
            commands::windsurf::windsurf_oauth_submit_callback_url,
            commands::windsurf::windsurf_oauth_login_cancel,
            commands::windsurf::add_windsurf_account_with_token,
            commands::windsurf::add_windsurf_account_with_password,
            commands::windsurf::add_windsurf_accounts_with_password,
            commands::windsurf::update_windsurf_account_tags,
            commands::windsurf::get_windsurf_accounts_index_path,
            commands::windsurf::inject_windsurf_to_vscode,
            // Kiro Commands
            commands::kiro::list_kiro_accounts,
            commands::kiro::delete_kiro_account,
            commands::kiro::delete_kiro_accounts,
            commands::kiro::import_kiro_from_json,
            commands::kiro::import_kiro_from_local,
            commands::kiro::export_kiro_accounts,
            commands::kiro::refresh_kiro_token,
            commands::kiro::refresh_all_kiro_tokens,
            commands::kiro::kiro_oauth_login_start,
            commands::kiro::kiro_oauth_login_complete,
            commands::kiro::kiro_oauth_submit_callback_url,
            commands::kiro::kiro_oauth_login_cancel,
            commands::kiro::add_kiro_account_with_token,
            commands::kiro::update_kiro_account_tags,
            commands::kiro::get_kiro_accounts_index_path,
            commands::kiro::inject_kiro_to_vscode,
            // CodeBuddy Commands
            commands::codebuddy::list_codebuddy_accounts,
            commands::codebuddy::delete_codebuddy_account,
            commands::codebuddy::delete_codebuddy_accounts,
            commands::codebuddy::import_codebuddy_from_json,
            commands::codebuddy::import_codebuddy_from_local,
            commands::codebuddy::export_codebuddy_accounts,
            commands::codebuddy::refresh_codebuddy_token,
            commands::codebuddy::refresh_all_codebuddy_tokens,
            commands::codebuddy::codebuddy_oauth_login_start,
            commands::codebuddy::codebuddy_oauth_login_complete,
            commands::codebuddy::codebuddy_oauth_login_cancel,
            commands::codebuddy::add_codebuddy_account_with_token,
            commands::codebuddy::update_codebuddy_account_tags,
            commands::codebuddy::get_codebuddy_accounts_index_path,
            commands::codebuddy::inject_codebuddy_to_vscode,
            commands::codebuddy_session::codebuddy_list_sessions,
            commands::ssh_server::list_ssh_servers,
            commands::ssh_server::upsert_ssh_server,
            commands::ssh_server::delete_ssh_server,
            commands::ssh_server::select_ssh_server,
            commands::ssh_server::test_ssh_server_connection,
            commands::ssh_server::sync_current_codex_account_to_ssh_server,
            // CodeBuddy CN Commands
            commands::codebuddy_cn::list_codebuddy_cn_accounts,
            commands::codebuddy_cn::delete_codebuddy_cn_account,
            commands::codebuddy_cn::delete_codebuddy_cn_accounts,
            commands::codebuddy_cn::import_codebuddy_cn_from_json,
            commands::codebuddy_cn::import_codebuddy_cn_from_local,
            commands::codebuddy_cn::export_codebuddy_cn_accounts,
            commands::codebuddy_cn::refresh_codebuddy_cn_token,
            commands::codebuddy_cn::refresh_all_codebuddy_cn_tokens,
            commands::codebuddy_cn::codebuddy_cn_oauth_login_start,
            commands::codebuddy_cn::codebuddy_cn_oauth_login_complete,
            commands::codebuddy_cn::codebuddy_cn_oauth_login_cancel,
            commands::codebuddy_cn::add_codebuddy_cn_account_with_token,
            commands::codebuddy_cn::update_codebuddy_cn_account_tags,
            commands::codebuddy_cn::get_codebuddy_cn_accounts_index_path,
            commands::codebuddy_cn::inject_codebuddy_cn_to_vscode,
            commands::codebuddy_cn::sync_codebuddy_cn_to_workbuddy,
            // WorkBuddy Commands
            commands::workbuddy::list_workbuddy_accounts,
            commands::workbuddy::delete_workbuddy_account,
            commands::workbuddy::delete_workbuddy_accounts,
            commands::workbuddy::import_workbuddy_from_json,
            commands::workbuddy::import_workbuddy_from_local,
            commands::workbuddy::export_workbuddy_accounts,
            commands::workbuddy::refresh_workbuddy_token,
            commands::workbuddy::refresh_all_workbuddy_tokens,
            commands::workbuddy::workbuddy_oauth_login_start,
            commands::workbuddy::workbuddy_oauth_login_complete,
            commands::workbuddy::workbuddy_oauth_login_cancel,
            commands::workbuddy::add_workbuddy_account_with_token,
            commands::workbuddy::update_workbuddy_account_tags,
            commands::workbuddy::get_workbuddy_accounts_index_path,
            commands::workbuddy::inject_workbuddy_to_vscode,
            commands::workbuddy::sync_workbuddy_to_codebuddy_cn,
            commands::workbuddy::get_checkin_status_workbuddy,
            commands::workbuddy::checkin_workbuddy,
            // WorkBuddy Instance Commands
            commands::workbuddy_instance::workbuddy_get_instance_defaults,
            commands::workbuddy_instance::workbuddy_list_instances,
            commands::workbuddy_instance::workbuddy_create_instance,
            commands::workbuddy_instance::workbuddy_update_instance,
            commands::workbuddy_instance::workbuddy_delete_instance,
            commands::workbuddy_instance::workbuddy_start_instance,
            commands::workbuddy_instance::workbuddy_stop_instance,
            commands::workbuddy_instance::workbuddy_open_instance_window,
            commands::workbuddy_instance::workbuddy_close_all_instances,
            // CodeBuddy Instance Commands
            commands::codebuddy_instance::codebuddy_get_instance_defaults,
            commands::codebuddy_instance::codebuddy_list_instances,
            commands::codebuddy_instance::codebuddy_create_instance,
            commands::codebuddy_instance::codebuddy_update_instance,
            commands::codebuddy_instance::codebuddy_delete_instance,
            commands::codebuddy_instance::codebuddy_start_instance,
            commands::codebuddy_instance::codebuddy_stop_instance,
            commands::codebuddy_instance::codebuddy_open_instance_window,
            commands::codebuddy_instance::codebuddy_close_all_instances,
            // CodeBuddy CN Instance Commands
            commands::codebuddy_cn_instance::codebuddy_cn_get_instance_defaults,
            commands::codebuddy_cn_instance::codebuddy_cn_list_instances,
            commands::codebuddy_cn_instance::codebuddy_cn_create_instance,
            commands::codebuddy_cn_instance::codebuddy_cn_update_instance,
            commands::codebuddy_cn_instance::codebuddy_cn_delete_instance,
            commands::codebuddy_cn_instance::codebuddy_cn_start_instance,
            commands::codebuddy_cn_instance::codebuddy_cn_stop_instance,
            commands::codebuddy_cn_instance::codebuddy_cn_open_instance_window,
            commands::codebuddy_cn_instance::codebuddy_cn_close_all_instances,
            // Qoder Commands
            commands::qoder::list_qoder_accounts,
            commands::qoder::delete_qoder_account,
            commands::qoder::delete_qoder_accounts,
            commands::qoder::import_qoder_from_json,
            commands::qoder::import_qoder_from_local,
            commands::qoder::qoder_oauth_login_start,
            commands::qoder::qoder_oauth_login_peek,
            commands::qoder::qoder_oauth_login_complete,
            commands::qoder::qoder_oauth_login_cancel,
            commands::qoder::export_qoder_accounts,
            commands::qoder::refresh_qoder_token,
            commands::qoder::refresh_all_qoder_tokens,
            commands::qoder::inject_qoder_account,
            commands::qoder::update_qoder_account_tags,
            commands::qoder::get_qoder_accounts_index_path,
            // Zed Commands
            commands::zed::list_zed_accounts,
            commands::zed::delete_zed_account,
            commands::zed::delete_zed_accounts,
            commands::zed::import_zed_from_json,
            commands::zed::import_zed_from_local,
            commands::zed::export_zed_accounts,
            commands::zed::refresh_zed_token,
            commands::zed::refresh_all_zed_tokens,
            commands::zed::update_zed_account_tags,
            commands::zed::zed_oauth_login_start,
            commands::zed::zed_oauth_login_peek,
            commands::zed::zed_oauth_login_complete,
            commands::zed::zed_oauth_login_cancel,
            commands::zed::zed_oauth_submit_callback_url,
            commands::zed::inject_zed_account,
            commands::zed::zed_logout_current_account,
            commands::zed::zed_get_runtime_status,
            commands::zed::zed_start_default_session,
            commands::zed::zed_stop_default_session,
            commands::zed::zed_restart_default_session,
            commands::zed::zed_focus_default_session,
            // ZCode Commands
            commands::zcode::list_zcode_accounts,
            commands::zcode::delete_zcode_account,
            commands::zcode::delete_zcode_accounts,
            commands::zcode::import_zcode_from_json,
            commands::zcode::import_zcode_from_local,
            commands::zcode::import_zcode_api_key,
            commands::zcode::export_zcode_accounts,
            commands::zcode::zcode_oauth_login_start,
            commands::zcode::zcode_oauth_login_complete,
            commands::zcode::zcode_oauth_submit_callback_url,
            commands::zcode::zcode_oauth_open_window,
            commands::zcode::zcode_oauth_login_cancel,
            commands::zcode::refresh_zcode_account,
            commands::zcode::refresh_all_zcode_accounts,
            commands::zcode::inject_zcode_account,
            commands::zcode::update_zcode_account_tags,
            commands::zcode::get_zcode_current_account_id,
            commands::zcode::get_zcode_accounts_index_path,
            // ZCode Instance Commands
            commands::zcode_instance::zcode_get_instance_defaults,
            commands::zcode_instance::zcode_list_instances,
            commands::zcode_instance::zcode_create_instance,
            commands::zcode_instance::zcode_update_instance,
            commands::zcode_instance::zcode_delete_instance,
            commands::zcode_instance::zcode_start_instance,
            commands::zcode_instance::zcode_stop_instance,
            commands::zcode_instance::zcode_open_instance_window,
            commands::zcode_instance::zcode_close_all_instances,
            // Qoder Instance Commands
            commands::qoder_instance::qoder_get_instance_defaults,
            commands::qoder_instance::qoder_list_instances,
            commands::qoder_instance::qoder_create_instance,
            commands::qoder_instance::qoder_update_instance,
            commands::qoder_instance::qoder_delete_instance,
            commands::qoder_instance::qoder_start_instance,
            commands::qoder_instance::qoder_stop_instance,
            commands::qoder_instance::qoder_open_instance_window,
            commands::qoder_instance::qoder_close_all_instances,
            // Trae Commands
            commands::trae::list_trae_accounts,
            commands::trae::delete_trae_account,
            commands::trae::delete_trae_accounts,
            commands::trae::import_trae_from_json,
            commands::trae::import_trae_from_local,
            commands::trae::trae_oauth_login_start,
            commands::trae::trae_oauth_login_complete,
            commands::trae::trae_oauth_submit_callback_url,
            commands::trae::trae_oauth_login_cancel,
            commands::trae::export_trae_accounts,
            commands::trae::refresh_trae_token,
            commands::trae::refresh_all_trae_tokens,
            commands::trae::refresh_trae_tokens_for_platform,
            commands::trae::add_trae_account_with_token,
            commands::trae::update_trae_account_tags,
            commands::trae::get_trae_accounts_index_path,
            commands::trae::inject_trae_account,
            // Trae Instance Commands
            commands::trae_instance::trae_get_instance_defaults,
            commands::trae_instance::trae_list_instances,
            commands::trae_instance::trae_create_instance,
            commands::trae_instance::trae_update_instance,
            commands::trae_instance::trae_delete_instance,
            commands::trae_instance::trae_start_instance,
            commands::trae_instance::trae_stop_instance,
            commands::trae_instance::trae_open_instance_window,
            commands::trae_instance::trae_close_all_instances,
            // Cursor Commands
            commands::cursor::list_cursor_accounts,
            commands::cursor::delete_cursor_account,
            commands::cursor::delete_cursor_accounts,
            commands::cursor::import_cursor_from_json,
            commands::cursor::import_cursor_from_local,
            commands::cursor::export_cursor_accounts,
            commands::cursor::refresh_cursor_token,
            commands::cursor::refresh_all_cursor_tokens,
            commands::cursor::add_cursor_account_with_token,
            commands::cursor::update_cursor_account_tags,
            commands::cursor::get_cursor_accounts_index_path,
            commands::cursor::cursor_oauth_login_start,
            commands::cursor::cursor_oauth_login_complete,
            commands::cursor::cursor_oauth_login_cancel,
            commands::cursor::inject_cursor_account,
            // Grok Commands
            commands::grok::grok_get_cli_status,
            commands::grok::grok_execute_cli_install_command,
            commands::grok::grok_update_cli_runtime_config,
            commands::grok::list_grok_accounts,
            commands::grok::delete_grok_account,
            commands::grok::delete_grok_accounts,
            commands::grok::import_grok_from_json,
            commands::grok::add_grok_account_with_api_key,
            commands::grok::import_grok_from_local,
            commands::grok::export_grok_accounts,
            commands::grok::grok_oauth_login_start,
            commands::grok::grok_oauth_login_complete,
            commands::grok::grok_oauth_login_cancel,
            commands::grok::refresh_grok_account,
            commands::grok::force_refresh_grok_account,
            commands::grok::refresh_all_grok_accounts,
            commands::grok::switch_grok_account,
            commands::grok::update_grok_account_tags,
            commands::grok::update_grok_account_working_dir,
            commands::grok::get_grok_current_account_id,
            commands::grok::get_grok_accounts_index_path,
            // Grok Instance Commands
            commands::grok_instance::grok_get_instance_defaults,
            commands::grok_instance::grok_list_instances,
            commands::grok_instance::grok_create_instance,
            commands::grok_instance::grok_update_instance,
            commands::grok_instance::grok_delete_instance,
            commands::grok_instance::grok_start_instance,
            commands::grok_instance::grok_stop_instance,
            commands::grok_instance::grok_close_all_instances,
            commands::grok_instance::grok_open_instance_window,
            commands::grok_instance::grok_get_instance_launch_command,
            commands::grok_instance::grok_execute_instance_launch_command,
            // Cursor Instance Commands
            commands::cursor_instance::cursor_get_instance_defaults,
            commands::cursor_instance::cursor_list_instances,
            commands::cursor_instance::cursor_create_instance,
            commands::cursor_instance::cursor_update_instance,
            commands::cursor_instance::cursor_delete_instance,
            commands::cursor_instance::cursor_start_instance,
            commands::cursor_instance::cursor_stop_instance,
            commands::cursor_instance::cursor_open_instance_window,
            commands::cursor_instance::cursor_close_all_instances,
            // Windsurf Instance Commands
            commands::windsurf_instance::windsurf_get_instance_defaults,
            commands::windsurf_instance::windsurf_list_instances,
            commands::windsurf_instance::windsurf_create_instance,
            commands::windsurf_instance::windsurf_update_instance,
            commands::windsurf_instance::windsurf_delete_instance,
            commands::windsurf_instance::windsurf_start_instance,
            commands::windsurf_instance::windsurf_stop_instance,
            commands::windsurf_instance::windsurf_open_instance_window,
            commands::windsurf_instance::windsurf_close_all_instances,
            // Kiro Instance Commands
            commands::kiro_instance::kiro_get_instance_defaults,
            commands::kiro_instance::kiro_list_instances,
            commands::kiro_instance::kiro_create_instance,
            commands::kiro_instance::kiro_update_instance,
            commands::kiro_instance::kiro_delete_instance,
            commands::kiro_instance::kiro_start_instance,
            commands::kiro_instance::kiro_stop_instance,
            commands::kiro_instance::kiro_open_instance_window,
            commands::kiro_instance::kiro_close_all_instances,
            // Codex Instance Commands
            commands::codex_instance::codex_get_instance_defaults,
            commands::codex_instance::codex_list_instances,
            commands::codex_instance::codex_get_instance_quick_config,
            commands::codex_instance::codex_save_instance_quick_config,
            commands::codex_instance::codex_open_instance_config_toml,
            commands::codex_instance::codex_sync_threads_across_instances,
            commands::codex_instance::codex_sync_sessions_to_instance,
            commands::codex_instance::codex_repair_session_visibility_across_instances,
            commands::codex_instance::codex_list_session_visibility_repair_providers,
            commands::codex_instance::codex_list_session_visibility_repair_instances,
            commands::codex_instance::codex_list_sessions_across_instances,
            commands::codex_instance::codex_get_session_token_stats_across_instances,
            commands::codex_instance::codex_move_sessions_to_trash_across_instances,
            commands::codex_instance::codex_list_trashed_sessions_across_instances,
            commands::codex_instance::codex_restore_sessions_from_trash_across_instances,
            commands::codex_instance::codex_delete_trashed_sessions_across_instances,
            commands::codex_instance::codex_empty_session_trash_across_instances,
            commands::codex_instance::codex_preview_session_export,
            commands::codex_instance::codex_export_sessions,
            commands::codex_instance::codex_preview_session_import,
            commands::codex_instance::codex_import_sessions,
            commands::codex_instance::codex_open_session_location,
            commands::codex_instance::codex_open_session_rollout,
            commands::codex_instance::codex_create_instance,
            commands::codex_instance::codex_update_instance,
            commands::codex_instance::codex_delete_instance,
            commands::codex_instance::codex_start_instance,
            commands::codex_instance::codex_stop_instance,
            commands::codex_instance::codex_open_instance_window,
            commands::codex_instance::codex_close_all_instances,
            commands::codex_instance::codex_preview_instance_launch_command,
            commands::codex_instance::codex_get_instance_launch_command,
            commands::codex_instance::codex_execute_instance_launch_command,
            // Instance Commands
            commands::instance::get_instance_defaults,
            commands::instance::list_instances,
            commands::instance::create_instance,
            commands::instance::update_instance,
            commands::instance::delete_instance,
            commands::instance::start_instance,
            commands::instance::stop_instance,
            commands::instance::open_instance_window,
            commands::instance::close_all_instances,
            commands::antigravity_legacy_instance::antigravity_legacy_get_instance_defaults,
            commands::antigravity_legacy_instance::antigravity_legacy_list_instances,
            commands::antigravity_legacy_instance::antigravity_legacy_create_instance,
            commands::antigravity_legacy_instance::antigravity_legacy_update_instance,
            commands::antigravity_legacy_instance::antigravity_legacy_delete_instance,
            commands::antigravity_legacy_instance::antigravity_legacy_start_instance,
            commands::antigravity_legacy_instance::antigravity_legacy_stop_instance,
            commands::antigravity_legacy_instance::antigravity_legacy_open_instance_window,
            commands::antigravity_legacy_instance::antigravity_legacy_close_all_instances,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app_handle, event| {
        match &event {
            RunEvent::ExitRequested { api, .. } => {
                if modules::floating_card_window::should_keep_alive_after_main_window_destroyed()
                    && !modules::app_lifecycle::is_shutdown_started()
                {
                    api.prevent_exit();
                    modules::logger::log_info("[Window] 主窗口已销毁，应用继续在托盘运行");
                } else {
                    modules::app_lifecycle::begin_shutdown();
                    modules::codex_app_injection::stop_all();
                    tauri::async_runtime::spawn(async {
                        modules::codex_local_access::shutdown_local_access_gateway_for_app_exit()
                            .await;
                    });
                }
            }
            RunEvent::Exit => {
                modules::app_lifecycle::begin_shutdown();
                modules::codex_app_injection::stop_all();
                tauri::async_runtime::spawn(async {
                    modules::codex_local_access::shutdown_local_access_gateway_for_app_exit().await;
                });
            }
            _ => {}
        }

        #[cfg(target_os = "macos")]
        {
            match event {
                RunEvent::Reopen { .. } => {
                    if let Err(err) = modules::floating_card_window::show_main_window(app_handle) {
                        logger::log_warn(&format!("[Window] Dock 重新打开主窗口失败: {}", err));
                    }
                }
                RunEvent::Opened { urls } => {
                    let args: Vec<String> = urls.iter().map(|url| url.to_string()).collect();
                    logger::log_info(&format!(
                        "[RunEvent] 收到 Opened 事件: url_count={}, urls={:?}",
                        args.len(),
                        summarize_deep_link_args(&args)
                    ));
                    let zcode_oauth_handled = handle_zcode_oauth_deep_links(&args);
                    let handled = zcode_oauth_handled
                        || modules::external_import::handle_external_import_args(
                            app_handle,
                            &args,
                            "run-event-opened",
                        );
                    logger::log_info(&format!(
                        "[RunEvent] Opened 外部导入处理结果: handled={}",
                        handled
                    ));
                }
                _ => {}
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (app_handle, event);
        }
    });
}
