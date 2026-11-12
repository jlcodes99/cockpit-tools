use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, PhysicalSize, Runtime,
    WebviewWindow, WebviewWindowBuilder, Window,
};

use crate::modules::{config, i18n, logger, main_window_state, process_memory};

pub const FLOATING_CARD_WINDOW_LABEL: &str = "floating-card";
pub const INSTANCE_FLOATING_CARD_WINDOW_LABEL_PREFIX: &str = "instance-floating-card-";
pub const FLOATING_CARD_CONTEXT_CHANGED_EVENT: &str = "floating-card:context-changed";
pub const FLOATING_CARD_APPEARANCE_CHANGED_EVENT: &str = "floating-card:appearance-changed";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FloatingCardAppearance {
    pub minimal: bool,
    pub background_opacity: f64,
}

impl From<&config::UserConfig> for FloatingCardAppearance {
    fn from(value: &config::UserConfig) -> Self {
        Self { minimal: value.floating_card_minimal, background_opacity: config::normalize_floating_card_background_opacity(value.floating_card_background_opacity) }
    }
}
const MAIN_WINDOW_LABEL: &str = "main";
const FLOATING_CARD_WIDTH: f64 = 250.0;
const FLOATING_CARD_FULL_HEIGHT: f64 = 290.0;
const FLOATING_CARD_MINIMAL_HEIGHT: f64 = 110.0;
const FLOATING_CARD_MAX_HEIGHT: f64 = 520.0;
const FLOATING_CARD_DEFAULT_MARGIN: i32 = 20;
const INSTANCE_FLOATING_CARD_WINDOW_OFFSET_STEP: i32 = 28;
const FLOATING_CARD_NATIVE_CORNER_RADIUS: f64 = 15.0;
static FLOATING_CARD_INSTANCE_CONTEXTS: LazyLock<
    Mutex<HashMap<String, FloatingCardInstanceContext>>,
> = LazyLock::new(|| Mutex::new(HashMap::new()));
static PENDING_MAIN_WINDOW_NAVIGATION: LazyLock<Mutex<Option<String>>> =
    LazyLock::new(|| Mutex::new(None));
static APP_EXIT_REQUESTED: AtomicBool = AtomicBool::new(false);
static MAIN_WINDOW_TRAY_STATE: Mutex<MainWindowTrayState> = Mutex::new(MainWindowTrayState {
    destroyed_to_tray: false,
    restore_requested: false,
});

#[derive(Default)]
struct MainWindowTrayState {
    destroyed_to_tray: bool,
    restore_requested: bool,
}

impl MainWindowTrayState {
    fn defer_restore(&mut self, window_present: bool) -> bool {
        self.restore_requested = self.destroyed_to_tray && window_present;
        self.restore_requested
    }

    fn take_restore_request(&mut self) -> bool {
        std::mem::take(&mut self.restore_requested)
    }
}

fn main_window_tray_state() -> std::sync::MutexGuard<'static, MainWindowTrayState> {
    MAIN_WINDOW_TRAY_STATE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

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
    if crate::modules::config::get_user_config().floating_card_minimal {
        config.height = FLOATING_CARD_MINIMAL_HEIGHT;
        config.min_height = Some(FLOATING_CARD_MINIMAL_HEIGHT);
    }
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

fn floating_card_position_in_work_area(
    saved: Option<PhysicalPosition<i32>>,
    work_area: &tauri::PhysicalRect<i32, u32>,
    window_size: PhysicalSize<u32>,
    stack_offset: i32,
) -> Result<PhysicalPosition<i32>, String> {
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
    let position = saved.unwrap_or_else(|| {
        PhysicalPosition::new(
            max_x - FLOATING_CARD_DEFAULT_MARGIN - stack_offset,
            min_y + FLOATING_CARD_DEFAULT_MARGIN + stack_offset,
        )
    });

    Ok(PhysicalPosition::new(
        position.x.clamp(min_x, max_x),
        position.y.clamp(min_y, max_y),
    ))
}

fn floating_card_min_height(minimal: bool) -> f64 {
    if minimal {
        FLOATING_CARD_MINIMAL_HEIGHT
    } else {
        FLOATING_CARD_FULL_HEIGHT
    }
}

fn floating_card_logical_size(minimal: bool, requested_height: f64) -> LogicalSize<f64> {
    let min_height = floating_card_min_height(minimal);
    let height = if requested_height.is_finite() {
        requested_height.clamp(min_height, FLOATING_CARD_MAX_HEIGHT)
    } else {
        min_height
    };
    LogicalSize::new(FLOATING_CARD_WIDTH, height)
}

pub fn resize_floating_card_window<R: Runtime>(
    window: &Window<R>,
    height: f64,
) -> Result<(), String> {
    let label = window.label();
    if label != FLOATING_CARD_WINDOW_LABEL && !is_instance_floating_card_window_label(label) {
        return Err("invalid_floating_card_window".to_string());
    }
    let minimal = config::get_user_config().floating_card_minimal;
    window
        .set_min_size(Some(LogicalSize::new(
            FLOATING_CARD_WIDTH,
            floating_card_min_height(minimal),
        )))
        .map_err(|err| err.to_string())?;
    window
        .set_size(floating_card_logical_size(minimal, height))
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

fn apply_floating_card_position<R: Runtime>(
    app: &AppHandle<R>,
    window: &WebviewWindow<R>,
    created: bool,
    minimal: bool,
) -> Result<(), String> {
    let instance = is_instance_floating_card_window_label(window.label());
    let saved = if instance {
        None
    } else {
        resolve_saved_floating_card_position()
    };
    let saved_monitor = saved
        .map(|position| app.monitor_from_point(f64::from(position.x), f64::from(position.y)))
        .transpose()
        .map_err(|err| err.to_string())?
        .flatten();
    let (monitor, saved) = match saved_monitor {
        Some(monitor) => (Some(monitor), saved),
        None => (app.primary_monitor().map_err(|err| err.to_string())?, None),
    };
    let Some(monitor) = monitor else {
        if instance {
            return Ok(());
        }
        return window
            .set_position(PhysicalPosition::new(
                FLOATING_CARD_DEFAULT_MARGIN,
                FLOATING_CARD_DEFAULT_MARGIN,
            ))
            .map_err(|err| err.to_string());
    };

    #[cfg(target_os = "windows")]
    let size = {
        let height = if created {
            floating_card_min_height(minimal)
        } else {
            window
                .inner_size()
                .map_err(|err| err.to_string())?
                .to_logical::<f64>(window.scale_factor().map_err(|err| err.to_string())?)
                .height
        };
        // Window creation can take place on a different screen. Use the same
        // destination DPI for clamping and for the physical size before show.
        floating_card_logical_size(minimal, height).to_physical::<u32>(monitor.scale_factor())
    };
    #[cfg(not(target_os = "windows"))]
    let size = {
        let _ = (created, minimal);
        window.outer_size().map_err(|err| err.to_string())?
    };

    let offset = if instance {
        i32::try_from(count_visible_instance_floating_card_windows(app))
            .map_err(|_| "floating_card_instance_stack_overflow".to_string())?
            * INSTANCE_FLOATING_CARD_WINDOW_OFFSET_STEP
    } else {
        0
    };
    let position = floating_card_position_in_work_area(saved, monitor.work_area(), size, offset)?;
    window
        .set_position(position)
        .map_err(|err| err.to_string())?;
    #[cfg(target_os = "windows")]
    window.set_size(size).map_err(|err| err.to_string())?;
    Ok(())
}

pub fn show_floating_card_window<R: Runtime>(
    app: &AppHandle<R>,
    focus: bool,
) -> Result<(), String> {
    let (window, created) =
        ensure_floating_card_window_with_label(app, FLOATING_CARD_WINDOW_LABEL)?;
    let config = config::get_user_config();

    apply_floating_card_position(app, &window, created, config.floating_card_minimal)?;
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
        apply_floating_card_position(app, &window, true, config.floating_card_minimal)?;
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

/// The frontend needs pending navigation for both a missing window and one that
/// has been closed to the tray but has not finished its queued destruction.
fn must_recreate_main_window(window_present: bool, destroyed_to_tray: bool) -> bool {
    destroyed_to_tray || !window_present
}

fn ensure_main_window<R: Runtime>(app: &AppHandle<R>) -> Result<(WebviewWindow<R>, bool), String> {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        return Ok((window, false));
    }

    let destroyed_to_tray = main_window_tray_state().destroyed_to_tray;
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
    main_window_tray_state().destroyed_to_tray
        && !APP_EXIT_REQUESTED.load(Ordering::SeqCst)
}

/// Tauri removes the old window label before dispatching the Destroyed run event.
pub fn resume_requested_main_window_restore<R: Runtime>(app: &AppHandle<R>) {
    let restore_requested = main_window_tray_state().take_restore_request();
    if restore_requested && !APP_EXIT_REQUESTED.load(Ordering::SeqCst) {
        if let Err(err) = show_main_window(app) {
            logger::log_warn(&format!("[Window] 托盘销毁后恢复主窗口失败: {}", err));
        }
    }
}

/// Destroy the main WebView when minimizing to tray (community #686 full behavior).
/// Backend, tray, and background services stay alive; reopen recreates the window.
pub fn destroy_main_window_to_tray<R: Runtime>(window: &Window<R>) -> Result<(), String> {
    // Persist size before destroy so tray reopen can restore geometry (#948 / #1132).
    main_window_state::capture_and_save_from_window_handle(window);
    main_window_tray_state().destroyed_to_tray = true;
    if let Err(err) = window.destroy() {
        *main_window_tray_state() = MainWindowTrayState::default();
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
    let destroyed_to_tray = main_window_tray_state().destroyed_to_tray;
    // Recreate (or first create) means the frontend is not ready for tray:navigate yet.
    let should_defer_navigation = must_recreate_main_window(window_present, destroyed_to_tray);
    if should_defer_navigation {
        set_pending_main_window_navigation(page)?;
        logger::log_info(&format!("[Window] 主窗口将重建，延迟导航到: {}", page));
    }
    if let Err(err) = show_main_window(app) {
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
    let window_present = app.get_webview_window(MAIN_WINDOW_LABEL).is_some();
    if main_window_tray_state().defer_restore(window_present) {
        return Ok(());
    }
    let (window, created) = ensure_main_window(app)?;

    logger::log_info(&format!("[Window] 尝试恢复主窗口 (recreated={})", created));
    #[cfg(target_os = "macos")]
    show_macos_application(app);

    window.show().map_err(|err| err.to_string())?;
    window.unminimize().map_err(|err| err.to_string())?;

    // The window event loop focuses only a visible window. A separate deferred
    // restore could bring the main window back after the user closes it again.
    if let Err(err) = window.set_focus() {
        logger::log_warn(&format!("[Window] WebView 主窗口聚焦失败: {}", err));
    }

    #[cfg(target_os = "macos")]
    activate_macos_application(app);

    if created {
        *main_window_tray_state() = MainWindowTrayState::default();
        logger::log_info("[Window] 主窗口恢复完成，前端将重新加载");
    }

    Ok(())
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
    use super::*;

    #[test]
    fn floating_card_cold_start_clamps_with_destination_monitor_size() {
        let full = floating_card_logical_size(false, FLOATING_CARD_FULL_HEIGHT);
        let primary_size = full.to_physical::<u32>(1.5);
        let secondary_size = full.to_physical::<u32>(1.0);
        assert_eq!(primary_size, PhysicalSize::new(375, 435));
        assert_eq!(secondary_size, PhysicalSize::new(250, 290));

        let secondary_work_area = tauri::PhysicalRect {
            position: PhysicalPosition::new(3840, 0),
            size: PhysicalSize::new(1920, 1040),
        };
        let saved = PhysicalPosition::new(5530, 800);
        assert_eq!(
            floating_card_position_in_work_area(
                Some(saved),
                &secondary_work_area,
                secondary_size,
                0
            )
            .unwrap(),
            PhysicalPosition::new(5510, 750),
        );
    }

    #[test]
    fn floating_card_keeps_fixed_width_and_bounded_content_height_across_scales() {
        assert_eq!(
            floating_card_logical_size(false, f64::NAN),
            LogicalSize::new(250.0, 290.0)
        );
        assert_eq!(
            floating_card_logical_size(true, 85.0).to_physical::<u32>(1.5),
            PhysicalSize::new(375, 165)
        );
        assert_eq!(
            floating_card_logical_size(false, 400.0),
            LogicalSize::new(250.0, 400.0)
        );
        assert_eq!(
            floating_card_logical_size(false, 900.0),
            LogicalSize::new(250.0, 520.0)
        );
        let area = tauri::PhysicalRect {
            position: PhysicalPosition::new(-1920, 0),
            size: PhysicalSize::new(1920, 1040),
        };
        assert_eq!(
            floating_card_position_in_work_area(None, &area, PhysicalSize::new(250, 290), 28)
                .unwrap(),
            PhysicalPosition::new(-298, 48),
        );
    }

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
    fn defers_repeated_tray_restore_requests_until_destruction() {
        assert!(must_recreate_main_window(true, true));
        let mut state = MainWindowTrayState {
            destroyed_to_tray: true,
            ..MainWindowTrayState::default()
        };
        assert!(!state.take_restore_request());
        assert!(state.defer_restore(true));
        assert!(state.defer_restore(true));
        assert!(state.take_restore_request());
        assert!(!state.defer_restore(false));
        assert!(!state.take_restore_request());
    }
}
