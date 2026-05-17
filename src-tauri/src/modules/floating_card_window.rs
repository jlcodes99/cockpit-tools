use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, Runtime, WebviewWindow, WebviewWindowBuilder,
};

use crate::modules::{config, i18n, logger};

pub const FLOATING_CARD_WINDOW_LABEL: &str = "floating-card";
pub const INSTANCE_FLOATING_CARD_WINDOW_LABEL_PREFIX: &str = "instance-floating-card-";
pub const FLOATING_CARD_CONTEXT_CHANGED_EVENT: &str = "floating-card:context-changed";
const FLOATING_CARD_DEFAULT_MARGIN: i32 = 20;
const INSTANCE_FLOATING_CARD_WINDOW_OFFSET_STEP: i32 = 28;
const FLOATING_CARD_NATIVE_CORNER_RADIUS: f64 = 15.0;
static FLOATING_CARD_INSTANCE_CONTEXTS: LazyLock<
    Mutex<HashMap<String, FloatingCardInstanceContext>>,
> = LazyLock::new(|| Mutex::new(HashMap::new()));

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

pub fn show_main_window_and_navigate<R: Runtime>(
    app: &AppHandle<R>,
    page: &str,
) -> Result<(), String> {
    show_main_window(app)?;
    app.emit("tray:navigate", page.to_string())
        .map_err(|err| err.to_string())?;
    Ok(())
}

pub fn show_main_window<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let Some(window) = app.get_webview_window("main") else {
        return Err("main_window_not_found".to_string());
    };

    logger::log_info("[Window] 尝试恢复主窗口");
    #[cfg(target_os = "macos")]
    show_macos_application(app);

    window.show().map_err(|err| err.to_string())?;
    window.unminimize().map_err(|err| err.to_string())?;

    if let Err(err) = window.set_focus() {
        logger::log_warn(&format!("[Window] WebView 主窗口聚焦失败: {}", err));
    }

    #[cfg(target_os = "macos")]
    activate_macos_application(app);

    #[cfg(target_os = "windows")]
    if let Err(err) = crate::modules::process::focus_current_process_main_window() {
        logger::log_warn(&format!("[Window] Windows 原生前置主窗口失败: {}", err));
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
