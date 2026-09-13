use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, Runtime, WebviewWindow, WebviewWindowBuilder,
    Window,
};

use crate::modules::{config, i18n, logger, main_window_state, process_memory};

pub const FLOATING_CARD_WINDOW_LABEL: &str = "floating-card";
pub const INSTANCE_FLOATING_CARD_WINDOW_LABEL_PREFIX: &str = "instance-floating-card-";
pub const FLOATING_CARD_CONTEXT_CHANGED_EVENT: &str = "floating-card:context-changed";
const MAIN_WINDOW_LABEL: &str = "main";
const FLOATING_CARD_DEFAULT_MARGIN: i32 = 20;
const INSTANCE_FLOATING_CARD_WINDOW_OFFSET_STEP: i32 = 28;
const FLOATING_CARD_NATIVE_CORNER_RADIUS: f64 = 15.0;
static FLOATING_CARD_INSTANCE_CONTEXTS: LazyLock<
    Mutex<HashMap<String, FloatingCardInstanceContext>>,
> = LazyLock::new(|| Mutex::new(HashMap::new()));
static PENDING_MAIN_WINDOW_NAVIGATION: LazyLock<Mutex<Option<String>>> =
    LazyLock::new(|| Mutex::new(None));
static APP_EXIT_REQUESTED: AtomicBool = AtomicBool::new(false);
static MAIN_WINDOW_DESTROYED_TO_TRAY: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FloatingCardInstanceContext {
    pub platform_id: String,
    pub instance_id: String,
    pub instance_name: String,
    pub bound_account_id: String,
}

fn is_instance_floating_card_window_label(label: &str) -> bool {
    label.starts_with(INSTANCE_FLOATING_CARD_WINDOW_LABEL_PREFIX)
}

fn sanitize_window_label_segment(value: &str) -> String {
    let mut sanitized = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            sanitized.push(ch);
        } else {
            sanitized.push('-');
        }
    }

    let trimmed = sanitized.trim_matches('-').trim_matches('_');
    if trimmed.is_empty() {
        "instance".to_string()
    } else {
        trimmed.to_string()
    }
}

fn build_instance_floating_card_window_label(context: &FloatingCardInstanceContext) -> String {
    format!(
        "{}{}-{}",
        INSTANCE_FLOATING_CARD_WINDOW_LABEL_PREFIX,
        sanitize_window_label_segment(&context.platform_id),
        sanitize_window_label_segment(&context.instance_id)
    )
}

fn emit_floating_card_context_changed<R: Runtime>(
    app: &AppHandle<R>,
    window_label: &str,
    context: Option<FloatingCardInstanceContext>,
) -> Result<(), String> {
    let Some(window) = app.get_webview_window(window_label) else {
        return Ok(());
    };

    window
        .emit(FLOATING_CARD_CONTEXT_CHANGED_EVENT, context)
        .map_err(|err| err.to_string())
}

pub fn get_floating_card_context(
    window_label: &str,
) -> Result<Option<FloatingCardInstanceContext>, String> {
    if !is_instance_floating_card_window_label(window_label) {
        return Ok(None);
    }

    FLOATING_CARD_INSTANCE_CONTEXTS
        .lock()
        .map_err(|_| "floating_card_context_lock_failed".to_string())
        .map(|contexts| contexts.get(window_label).cloned())
}

pub fn set_floating_card_instance_context<R: Runtime>(
    app: &AppHandle<R>,
    window_label: &str,
    context: FloatingCardInstanceContext,
) -> Result<(), String> {
    {
        let mut contexts = FLOATING_CARD_INSTANCE_CONTEXTS
            .lock()
            .map_err(|_| "floating_card_context_lock_failed".to_string())?;
        contexts.insert(window_label.to_string(), context.clone());
    }

    emit_floating_card_context_changed(app, window_label, Some(context))
}

fn floating_card_window_config(
    app: &AppHandle<impl Runtime>,
) -> Result<&tauri::utils::config::WindowConfig, String> {
    app.config()
        .app
        .windows
        .iter()
        .find(|item| item.label == FLOATING_CARD_WINDOW_LABEL)
        .ok_or_else(|| "floating_card_window_config_not_found".to_string())
}

fn clone_floating_card_window_config(
    app: &AppHandle<impl Runtime>,
    label: &str,
) -> Result<tauri::utils::config::WindowConfig, String> {
    let mut config = floating_card_window_config(app)?.clone();
    config.label = label.to_string();
    config.create = false;
    config.visible = false;
    Ok(config)
}

fn ensure_floating_card_window_with_label<R: Runtime>(
    app: &AppHandle<R>,
    label: &str,
) -> Result<(WebviewWindow<R>, bool), String> {
    if let Some(window) = app.get_webview_window(label) {
        apply_native_floating_card_window_shape(&window)?;
        return Ok((window, false));
    }

    let window_config = clone_floating_card_window_config(app, label)?;
    let window = WebviewWindowBuilder::from_config(app, &window_config)
        .map_err(|err| err.to_string())?
        .build()
        .map_err(|err| err.to_string())?;

    apply_native_floating_card_window_shape(&window)?;
    logger::log_info(&format!("[FloatingCard] 悬浮卡片窗口已创建: {}", label));
    Ok((window, true))
}

pub fn ensure_floating_card_window<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<WebviewWindow<R>, String> {
    ensure_floating_card_window_with_label(app, FLOATING_CARD_WINDOW_LABEL)
        .map(|(window, _)| window)
}

#[cfg(not(target_os = "macos"))]
fn apply_native_floating_card_window_shape<R: Runtime>(
    _window: &WebviewWindow<R>,
) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn apply_native_floating_card_window_shape<R: Runtime>(
    window: &WebviewWindow<R>,
) -> Result<(), String> {
    use objc2_foundation::NSThread;
    use std::sync::mpsc;

    let ns_window = window.ns_window().map_err(|err| err.to_string())? as usize;

    if NSThread::isMainThread_class() {
        return unsafe { configure_floating_card_ns_window(ns_window as *mut std::ffi::c_void) };
    }

    let (tx, rx) = mpsc::channel();

    window
        .run_on_main_thread(move || {
            let result =
                unsafe { configure_floating_card_ns_window(ns_window as *mut std::ffi::c_void) };
            let _ = tx.send(result);
        })
        .map_err(|err| err.to_string())?;

    rx.recv()
        .map_err(|_| "floating_card_window_main_thread_channel_closed".to_string())?
}

#[cfg(target_os = "macos")]
unsafe fn configure_floating_card_ns_window(
    ns_window: *mut std::ffi::c_void,
) -> Result<(), String> {
    use objc2_app_kit::{NSColor, NSWindow};

    let window = ns_window
        .cast::<NSWindow>()
        .as_ref()
        .ok_or_else(|| "floating_card_ns_window_not_found".to_string())?;

    window.setOpaque(false);
    let clear_color = NSColor::clearColor();
    window.setBackgroundColor(Some(&clear_color));

    let content_view = window
        .contentView()
        .ok_or_else(|| "floating_card_content_view_not_found".to_string())?;
    apply_corner_mask_to_view(&content_view)?;

    if let Some(frame_view) = content_view.superview() {
        apply_corner_mask_to_view(&frame_view)?;
    }

    window.invalidateShadow();
    Ok(())
}

#[cfg(target_os = "macos")]
fn apply_corner_mask_to_view(view: &objc2_app_kit::NSView) -> Result<(), String> {
    use objc2::{msg_send, runtime::AnyObject};

    view.setWantsLayer(true);
    let layer: *mut AnyObject = unsafe { msg_send![view, layer] };
    if layer.is_null() {
        return Err("floating_card_view_layer_not_found".to_string());
    }

    unsafe {
        let _: () = msg_send![layer, setCornerRadius: FLOATING_CARD_NATIVE_CORNER_RADIUS];
        let _: () = msg_send![layer, setMasksToBounds: true];
    }
    Ok(())
}

pub fn apply_floating_card_always_on_top<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let Some(window) = app.get_webview_window(FLOATING_CARD_WINDOW_LABEL) else {
        return Ok(());
    };

    let config = config::get_user_config();
    window
        .set_always_on_top(config.floating_card_always_on_top)
        .map_err(|err| err.to_string())?;
    Ok(())
}

fn resolve_saved_floating_card_position() -> Option<PhysicalPosition<i32>> {
    let user_config = config::get_user_config();
    match (
        user_config.floating_card_position_x,
        user_config.floating_card_position_y,
    ) {
        (Some(x), Some(y)) => Some(PhysicalPosition::new(x, y)),
        _ => None,
    }
}

fn clamp_position_to_work_area(
    position: PhysicalPosition<i32>,
    work_area: &tauri::PhysicalRect<i32, u32>,
    window: &WebviewWindow<impl Runtime>,
) -> Result<PhysicalPosition<i32>, String> {
    let window_size = window.outer_size().map_err(|err| err.to_string())?;
    let window_width = i32::try_from(window_size.width)
        .map_err(|_| "floating_card_window_width_overflow".to_string())?;
    let window_height = i32::try_from(window_size.height)
        .map_err(|_| "floating_card_window_height_overflow".to_string())?;
    let work_area_width = i32::try_from(work_area.size.width)
        .map_err(|_| "floating_card_monitor_width_overflow".to_string())?;
    let work_area_height = i32::try_from(work_area.size.height)
        .map_err(|_| "floating_card_monitor_height_overflow".to_string())?;

    let min_x = work_area.position.x;
    let min_y = work_area.position.y;
    let max_x = (min_x + work_area_width - window_width).max(min_x);
    let max_y = (min_y + work_area_height - window_height).max(min_y);

    Ok(PhysicalPosition::new(
        position.x.clamp(min_x, max_x),
        position.y.clamp(min_y, max_y),
    ))
}

fn resolve_visible_floating_card_position<R: Runtime>(
    app: &AppHandle<R>,
    window: &WebviewWindow<R>,
) -> Result<Option<PhysicalPosition<i32>>, String> {
    let Some(saved_position) = resolve_saved_floating_card_position() else {
        return Ok(None);
    };

    let Some(monitor) = app
        .monitor_from_point(saved_position.x as f64, saved_position.y as f64)
        .map_err(|err| err.to_string())?
    else {
        return Ok(None);
    };

    clamp_position_to_work_area(saved_position, monitor.work_area(), window).map(Some)
}

fn calculate_default_top_right_position<R: Runtime>(
    app: &AppHandle<R>,
    window: &WebviewWindow<R>,
) -> Result<PhysicalPosition<i32>, String> {
    let Some(monitor) = app.primary_monitor().map_err(|err| err.to_string())? else {
        return Ok(PhysicalPosition::new(
            FLOATING_CARD_DEFAULT_MARGIN,
            FLOATING_CARD_DEFAULT_MARGIN,
        ));
    };
    let work_area = monitor.work_area();
    let window_size = window.outer_size().map_err(|err| err.to_string())?;
    let window_width = i32::try_from(window_size.width)
        .map_err(|_| "floating_card_window_width_overflow".to_string())?;

    let x = work_area.position.x
        + i32::try_from(work_area.size.width)
            .map_err(|_| "floating_card_monitor_width_overflow".to_string())?
        - window_width
        - FLOATING_CARD_DEFAULT_MARGIN;
    let y = work_area.position.y + FLOATING_CARD_DEFAULT_MARGIN;

    Ok(PhysicalPosition::new(x.max(work_area.position.x), y))
}

fn apply_floating_card_position<R: Runtime>(
    app: &AppHandle<R>,
    window: &WebviewWindow<R>,
) -> Result<(), String> {
    let target_position =
        if let Some(saved_position) = resolve_visible_floating_card_position(app, window)? {
            saved_position
        } else {
            calculate_default_top_right_position(app, window)?
        };

    window
        .set_position(target_position)
        .map_err(|err| err.to_string())
}

fn count_visible_instance_floating_card_windows<R: Runtime>(app: &AppHandle<R>) -> usize {
    app.webview_windows()
        .values()
        .filter(|window| {
            is_instance_floating_card_window_label(window.label())
                && window.is_visible().unwrap_or(false)
        })
        .count()
}

fn apply_instance_floating_card_position<R: Runtime>(
    app: &AppHandle<R>,
    window: &WebviewWindow<R>,
) -> Result<(), String> {
    let Some(monitor) = app.primary_monitor().map_err(|err| err.to_string())? else {
        return Ok(());
    };

    let stack_index = count_visible_instance_floating_card_windows(app);
    let base_position = calculate_default_top_right_position(app, window)?;
    let offset = i32::try_from(stack_index)
        .map_err(|_| "floating_card_instance_stack_overflow".to_string())?
        * INSTANCE_FLOATING_CARD_WINDOW_OFFSET_STEP;
    let target_position = PhysicalPosition::new(base_position.x - offset, base_position.y + offset);
    let clamped = clamp_position_to_work_area(target_position, monitor.work_area(), window)?;

    window.set_position(clamped).map_err(|err| err.to_string())
}

pub fn show_floating_card_window<R: Runtime>(
    app: &AppHandle<R>,
    focus: bool,
) -> Result<(), String> {
    let window = ensure_floating_card_window(app)?;
    let config = config::get_user_config();

    apply_floating_card_position(app, &window)?;
    window
        .set_always_on_top(config.floating_card_always_on_top)
        .map_err(|err| err.to_string())?;
    window.show().map_err(|err| err.to_string())?;
    window.unminimize().map_err(|err| err.to_string())?;
    if focus {
        window.set_focus().map_err(|err| err.to_string())?;
    }

    Ok(())
}

pub fn show_instance_floating_card_window<R: Runtime>(
    app: &AppHandle<R>,
    context: FloatingCardInstanceContext,
    focus: bool,
) -> Result<(), String> {
    let window_label = build_instance_floating_card_window_label(&context);
    let (window, created) = ensure_floating_card_window_with_label(app, &window_label)?;

    set_floating_card_instance_context(app, &window_label, context)?;
    if created {
        let config = config::get_user_config();
        apply_instance_floating_card_position(app, &window)?;
        window
            .set_always_on_top(config.floating_card_always_on_top)
            .map_err(|err| err.to_string())?;
    }
    window.show().map_err(|err| err.to_string())?;
    window.unminimize().map_err(|err| err.to_string())?;
    if focus {
        window.set_focus().map_err(|err| err.to_string())?;
    }

    Ok(())
}

pub fn show_floating_card_window_on_startup<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let config = config::get_user_config();
    if !config.floating_card_show_on_startup {
        return Ok(());
    }

    show_floating_card_window(app, false)
}

pub fn hide_floating_card_window<R: Runtime>(
    app: &AppHandle<R>,
    notify: bool,
) -> Result<(), String> {
    let Some(window) = app.get_webview_window(FLOATING_CARD_WINDOW_LABEL) else {
        return Ok(());
    };

    window.hide().map_err(|err| err.to_string())?;
    if notify {
        send_hidden_notification(app);
    }
    Ok(())
}

fn main_window_config(
    app: &AppHandle<impl Runtime>,
) -> Result<&tauri::utils::config::WindowConfig, String> {
    app.config()
        .app
        .windows
        .iter()
        .find(|item| item.label == MAIN_WINDOW_LABEL)
        .ok_or_else(|| "main_window_config_not_found".to_string())
}

fn clone_main_window_config(
    app: &AppHandle<impl Runtime>,
) -> Result<tauri::utils::config::WindowConfig, String> {
    let mut config = main_window_config(app)?.clone();
    config.create = false;
    config.visible = false;
    // Restore last size/position when recreating after tray destroy (#948 / #1132).
    main_window_state::apply_state_to_window_config(&mut config);
    Ok(config)
}

/// After close-to-tray destroy (#686), a stale `main` label can still resolve on
/// Windows/WebView2. Treat `destroyed_to_tray` as authoritative and force recreate.
fn must_recreate_main_window(window_present: bool, destroyed_to_tray: bool) -> bool {
    destroyed_to_tray || !window_present
}

fn wait_until_main_window_gone<R: Runtime>(app: &AppHandle<R>, attempts: u32) {
    for _ in 0..attempts {
        if app.get_webview_window(MAIN_WINDOW_LABEL).is_none() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

fn ensure_main_window<R: Runtime>(app: &AppHandle<R>) -> Result<(WebviewWindow<R>, bool), String> {
    let destroyed_to_tray = MAIN_WINDOW_DESTROYED_TO_TRAY.load(Ordering::SeqCst);

    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        if !must_recreate_main_window(true, destroyed_to_tray) {
            return Ok((window, false));
        }

        logger::log_info("[Window] 托盘销毁后检测到残留主窗口句柄，强制销毁并重建");
        if let Err(err) = window.destroy() {
            logger::log_warn(&format!(
                "[Window] 清理残留主窗口失败（仍将尝试重建）: {}",
                err
            ));
        }
        wait_until_main_window_gone(app, 50);
    }

    logger::log_info(&format!(
        "[Window] 开始重建主窗口 WebView (destroyed_to_tray={})",
        destroyed_to_tray
    ));
    let window_config = clone_main_window_config(app)?;
    let window = WebviewWindowBuilder::from_config(app, &window_config)
        .map_err(|err| err.to_string())?
        .build()
        .map_err(|err| {
            format!(
                "重建主窗口失败（destroyed_to_tray={}）: {}",
                destroyed_to_tray, err
            )
        })?;

    // Builder already got size from config; re-apply for maximized / DPI edge cases.
    main_window_state::restore_to_window(&window);

    logger::log_info("[Window] WebView 主窗口已重新创建");
    Ok((window, true))
}

fn set_pending_main_window_navigation(page: &str) -> Result<(), String> {
    let mut pending = PENDING_MAIN_WINDOW_NAVIGATION
        .lock()
        .map_err(|_| "main_window_navigation_lock_failed".to_string())?;
    *pending = Some(page.to_string());
    Ok(())
}

pub fn take_pending_main_window_navigation() -> Result<Option<String>, String> {
    PENDING_MAIN_WINDOW_NAVIGATION
        .lock()
        .map_err(|_| "main_window_navigation_lock_failed".to_string())
        .map(|mut pending| pending.take())
}

pub fn request_app_exit() {
    APP_EXIT_REQUESTED.store(true, Ordering::SeqCst);
}

pub fn should_keep_alive_after_main_window_destroyed() -> bool {
    MAIN_WINDOW_DESTROYED_TO_TRAY.load(Ordering::SeqCst)
        && !APP_EXIT_REQUESTED.load(Ordering::SeqCst)
}

/// Destroy the main WebView when minimizing to tray (community #686 full behavior).
/// Backend, tray, and background services stay alive; reopen recreates the window.
pub fn destroy_main_window_to_tray<R: Runtime>(window: &Window<R>) -> Result<(), String> {
    // Persist size before destroy so tray reopen can restore geometry (#948 / #1132).
    main_window_state::capture_and_save_from_window_handle(window);
    MAIN_WINDOW_DESTROYED_TO_TRAY.store(true, Ordering::SeqCst);
    if let Err(err) = window.destroy() {
        MAIN_WINDOW_DESTROYED_TO_TRAY.store(false, Ordering::SeqCst);
        return Err(err.to_string());
    }
    // Defer working-set trim so WebView2 can finish teardown before a later recreate.
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(300));
        process_memory::trim_idle_process_memory();
    });
    logger::log_info("[Window] 主窗口 WebView 已销毁并保留托盘进程");
    Ok(())
}

pub fn show_main_window_and_navigate<R: Runtime>(
    app: &AppHandle<R>,
    page: &str,
) -> Result<(), String> {
    let window_present = app.get_webview_window(MAIN_WINDOW_LABEL).is_some();
    let destroyed_to_tray = MAIN_WINDOW_DESTROYED_TO_TRAY.load(Ordering::SeqCst);
    // Recreate (or first create) means the frontend is not ready for tray:navigate yet.
    let should_defer_navigation = must_recreate_main_window(window_present, destroyed_to_tray);
    if should_defer_navigation {
        set_pending_main_window_navigation(page)?;
        logger::log_info(&format!("[Window] 主窗口将重建，延迟导航到: {}", page));
    }
    if let Err(err) = show_main_window_internal(app) {
        if should_defer_navigation {
            let _ = take_pending_main_window_navigation();
        }
        return Err(err);
    }
    if should_defer_navigation {
        // Pending navigation is consumed on main-window mount; emit would race load.
        return Ok(());
    }
    if let Err(err) = app.emit("tray:navigate", page.to_string()) {
        return Err(err.to_string());
    }
    Ok(())
}

/// Invoke from async Tauri commands so WebView recreate runs on the UI thread.
pub async fn show_main_window_and_navigate_async<R: Runtime>(
    app: AppHandle<R>,
    page: String,
) -> Result<(), String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let app_for_task = app.clone();
    app.run_on_main_thread(move || {
        let result = show_main_window_and_navigate(&app_for_task, &page);
        let _ = tx.send(result);
    })
    .map_err(|err| format!("调度主线程重建主窗口失败: {}", err))?;
    rx.await
        .map_err(|_| "主线程重建主窗口任务已取消".to_string())?
}

pub fn show_main_window<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    show_main_window_internal(app).map(|_| ())
}

fn show_main_window_internal<R: Runtime>(app: &AppHandle<R>) -> Result<bool, String> {
    let (window, created) = ensure_main_window(app)?;

    logger::log_info(&format!("[Window] 尝试恢复主窗口 (recreated={})", created));
    #[cfg(target_os = "macos")]
    show_macos_application(app);

    window.show().map_err(|err| err.to_string())?;
    window.unminimize().map_err(|err| err.to_string())?;
    if !created {
        main_window_state::request_main_window_viewport_refresh();
    }

    if let Err(err) = window.set_focus() {
        logger::log_warn(&format!("[Window] WebView 主窗口聚焦失败: {}", err));
    }

    #[cfg(target_os = "macos")]
    activate_macos_application(app);

    #[cfg(target_os = "windows")]
    {
        // Prefer the Tauri main window HWND. Process MainWindowHandle often points at
        // the floating card after the main WebView was destroyed to tray.
        // Never block the UI thread on PowerShell focus.
        match window.hwnd() {
            Ok(hwnd) => {
                let raw = hwnd.0 as isize;
                std::thread::spawn(move || {
                    if let Err(err) = crate::modules::process::focus_window_by_hwnd(raw) {
                        logger::log_warn(&format!(
                            "[Window] Windows 按 HWND 前置主窗口失败: {}",
                            err
                        ));
                    }
                });
            }
            Err(err) => {
                logger::log_warn(&format!(
                    "[Window] 获取主窗口 HWND 失败，回退进程主句柄: {}",
                    err
                ));
                std::thread::spawn(|| {
                    if let Err(err) = crate::modules::process::focus_current_process_main_window() {
                        logger::log_warn(&format!("[Window] Windows 原生前置主窗口失败: {}", err));
                    }
                });
            }
        }
    }

    if created {
        MAIN_WINDOW_DESTROYED_TO_TRAY.store(false, Ordering::SeqCst);
        logger::log_info("[Window] 主窗口恢复完成，前端将重新加载");
    }

    Ok(created)
}

#[cfg(target_os = "macos")]
fn show_macos_application<R: Runtime>(app: &AppHandle<R>) {
    if let Err(err) = app.show() {
        logger::log_warn(&format!("[Window] macOS 应用取消隐藏失败: {}", err));
    }
}

#[cfg(target_os = "macos")]
fn activate_macos_application<R: Runtime>(app: &AppHandle<R>) {
    if let Err(err) = app.run_on_main_thread(|| {
        use objc2_app_kit::NSApplication;
        use objc2_foundation::MainThreadMarker;

        let marker =
            MainThreadMarker::new().unwrap_or_else(|| unsafe { MainThreadMarker::new_unchecked() });
        let ns_app = NSApplication::sharedApplication(marker);
        ns_app.unhide(None);
        #[allow(deprecated)]
        ns_app.activateIgnoringOtherApps(true);
    }) {
        logger::log_warn(&format!("[Window] macOS 应用激活失败: {}", err));
    }
}

#[cfg(not(target_os = "macos"))]
fn send_hidden_notification<R: Runtime>(app: &AppHandle<R>) {
    use tauri_plugin_notification::NotificationExt;

    let locale = config::get_user_config().language;
    let title = i18n::translate(&locale, "floatingCard.hiddenNotification.title", &[]);
    let body = i18n::translate(&locale, "floatingCard.hiddenNotification.body", &[]);

    if let Err(err) = app.notification().builder().title(&title).body(body).show() {
        logger::log_warn(&format!("[FloatingCard] 发送关闭引导通知失败: {}", err));
    }
}

#[cfg(target_os = "macos")]
fn send_hidden_notification<R: Runtime>(app: &AppHandle<R>) {
    let locale = config::get_user_config().language;
    let title = i18n::translate(&locale, "floatingCard.hiddenNotification.title", &[]);
    let body = i18n::translate(&locale, "floatingCard.hiddenNotification.body", &[]);
    let bundle_identifier = app.config().identifier.to_string();

    std::thread::spawn(move || {
        let mut notification = mac_notification_sys::Notification::new();
        notification
            .title(title.as_str())
            .message(body.as_str())
            .wait_for_click(false)
            .asynchronous(true);

        if let Err(err) = mac_notification_sys::set_application(&bundle_identifier) {
            logger::log_warn(&format!("[FloatingCard] 设置通知应用标识失败: {}", err));
        }

        if let Err(err) = notification.send() {
            logger::log_warn(&format!("[FloatingCard] 发送关闭引导通知失败: {}", err));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::must_recreate_main_window;

    #[test]
    fn recreates_when_window_missing() {
        assert!(must_recreate_main_window(false, false));
        assert!(must_recreate_main_window(false, true));
    }

    #[test]
    fn reuses_live_window_when_not_destroyed_to_tray() {
        assert!(!must_recreate_main_window(true, false));
    }

    #[test]
    fn force_recreates_stale_handle_after_tray_destroy() {
        // Windows/WebView2 may still resolve `main` after destroy; flag wins.
        assert!(must_recreate_main_window(true, true));
    }
}
