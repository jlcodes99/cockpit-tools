//! Persist and restore the main window size/position (#948 / #1132).
//!
//! Independent of general user config so frequent resize writes stay lightweight.
//! Only applies to the `main` window — floating card / OAuth windows are ignored.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, PhysicalPosition, PhysicalSize, Position,
    Runtime, Size, WebviewWindow, Window,
};

use crate::modules::{atomic_write, config, logger};

const STATE_FILE: &str = "main_window_state.json";
const MIN_WIDTH: f64 = 900.0;
const MIN_HEIGHT: f64 = 600.0;
const DEFAULT_WIDTH: f64 = 1280.0;
const DEFAULT_HEIGHT: f64 = 800.0;
const MIN_VISIBLE_WIDTH: f64 = 64.0;
const MIN_VISIBLE_HEIGHT: f64 = 48.0;
const SAVE_DEBOUNCE: Duration = Duration::from_millis(250);
const RECONCILE_GEOMETRY: u8 = 1 << 0;
const REFRESH_WEBVIEW_VIEWPORT: u8 = 1 << 1;
const SAVE_WINDOW_STATE_DEBOUNCED: u8 = 1 << 2;
const SAVE_WINDOW_STATE_FORCED: u8 = 1 << 3;
const CONTAIN_WINDOW_IN_WORK_AREA: u8 = 1 << 4;
const CHECK_DISPLAY_CONTEXT: u8 = 1 << 5;

static LAST_SAVE_AT: Mutex<Option<Instant>> = Mutex::new(None);
static PENDING_MAIN_WINDOW_RECONCILE: AtomicU8 = AtomicU8::new(0);
static LAST_MAIN_WINDOW_DISPLAY_CONTEXT: Mutex<Option<DisplayContext>> = Mutex::new(None);

#[derive(Debug, Clone, Copy)]
struct LogicalRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct PixelRect {
    x: i64,
    y: i64,
    width: u64,
    height: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct MonitorContext {
    bounds: PixelRect,
    work_area: PixelRect,
    scale_factor_bits: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DisplayContext {
    monitors: Vec<MonitorContext>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PixelWindowGeometry {
    x: i64,
    y: i64,
    outer_width: u64,
    outer_height: u64,
    inner_width: u64,
    inner_height: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FittedWindowGeometry {
    x: i64,
    y: i64,
    inner_width: u64,
    inner_height: u64,
    outer_width: u64,
    outer_height: u64,
    resized: bool,
    repositioned: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct ReconcileOutcome {
    geometry_changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReconcileMode {
    Skip,
    ViewportOnly,
    Full,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MainWindowState {
    pub width: f64,
    pub height: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    #[serde(default)]
    pub maximized: bool,
}

impl Default for MainWindowState {
    fn default() -> Self {
        Self {
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            x: None,
            y: None,
            maximized: false,
        }
    }
}

fn state_path() -> Result<PathBuf, String> {
    let data_dir = config::get_data_dir()?;
    Ok(data_dir.join(STATE_FILE))
}

fn clamp_size(width: f64, height: f64) -> (f64, f64) {
    let w = if width.is_finite() && width > 0.0 {
        width.max(MIN_WIDTH)
    } else {
        DEFAULT_WIDTH
    };
    let h = if height.is_finite() && height > 0.0 {
        height.max(MIN_HEIGHT)
    } else {
        DEFAULT_HEIGHT
    };
    (w, h)
}

fn remember_main_window_state_enabled() -> bool {
    config::get_user_config().remember_main_window_state
}

fn has_visible_overlap(window: LogicalRect, monitor: LogicalRect) -> bool {
    let overlap_width =
        (window.x + window.width).min(monitor.x + monitor.width) - window.x.max(monitor.x);
    let overlap_height =
        (window.y + window.height).min(monitor.y + monitor.height) - window.y.max(monitor.y);
    overlap_width >= MIN_VISIBLE_WIDTH && overlap_height >= MIN_VISIBLE_HEIGHT
}

fn rect_end(start: i64, length: u64) -> i64 {
    start.saturating_add(i64::try_from(length).unwrap_or(i64::MAX))
}

fn overlap_extent(a_start: i64, a_length: u64, b_start: i64, b_length: u64) -> u64 {
    let start = a_start.max(b_start);
    let end = rect_end(a_start, a_length).min(rect_end(b_start, b_length));
    u64::try_from(end.saturating_sub(start).max(0)).unwrap_or(u64::MAX)
}

fn clamp_axis_to_work_area(
    position: i64,
    window_length: u64,
    work_area_start: i64,
    work_area_length: u64,
) -> i64 {
    let max_position = rect_end(work_area_start, work_area_length)
        .saturating_sub(i64::try_from(window_length).unwrap_or(i64::MAX))
        .max(work_area_start);
    position.clamp(work_area_start, max_position)
}

fn fit_window_to_work_area(
    window: PixelWindowGeometry,
    work_area: PixelRect,
    minimum_inner_width: u64,
    minimum_inner_height: u64,
    minimum_visible_width: u64,
    minimum_visible_height: u64,
    force_containment: bool,
) -> FittedWindowGeometry {
    let frame_width = window.outer_width.saturating_sub(window.inner_width);
    let frame_height = window.outer_height.saturating_sub(window.inner_height);
    let fitting_inner_width = work_area.width.saturating_sub(frame_width).max(1);
    let fitting_inner_height = work_area.height.saturating_sub(frame_height).max(1);
    let allowed_inner_width = fitting_inner_width.max(minimum_inner_width.max(1));
    let allowed_inner_height = fitting_inner_height.max(minimum_inner_height.max(1));

    // Display changes may require shrinking, but returning to a larger display must not
    // unexpectedly grow a window the user deliberately kept small.
    let inner_width = window.inner_width.max(1).min(allowed_inner_width);
    let inner_height = window.inner_height.max(1).min(allowed_inner_height);
    let outer_width = inner_width.saturating_add(frame_width);
    let outer_height = inner_height.saturating_add(frame_height);
    let resized = inner_width != window.inner_width || inner_height != window.inner_height;

    let overlap_width = overlap_extent(window.x, outer_width, work_area.x, work_area.width);
    let overlap_height = overlap_extent(window.y, outer_height, work_area.y, work_area.height);
    let contain_window = force_containment || resized;
    let x = if outer_width > work_area.width {
        work_area.x
    } else if contain_window || overlap_width < minimum_visible_width {
        clamp_axis_to_work_area(window.x, outer_width, work_area.x, work_area.width)
    } else {
        window.x
    };
    let y = if outer_height > work_area.height {
        work_area.y
    } else if contain_window || overlap_height < minimum_visible_height {
        clamp_axis_to_work_area(window.y, outer_height, work_area.y, work_area.height)
    } else {
        window.y
    };

    FittedWindowGeometry {
        x,
        y,
        inner_width,
        inner_height,
        outer_width,
        outer_height,
        resized,
        repositioned: x != window.x || y != window.y,
    }
}

fn normalize_display_context(mut monitors: Vec<MonitorContext>) -> DisplayContext {
    monitors.sort_unstable();
    DisplayContext { monitors }
}

fn has_display_context_changed(context: &DisplayContext) -> bool {
    let Ok(previous) = LAST_MAIN_WINDOW_DISPLAY_CONTEXT.lock() else {
        return true;
    };
    previous.as_ref() != Some(context)
}

fn remember_display_context(context: DisplayContext) {
    if let Ok(mut previous) = LAST_MAIN_WINDOW_DISPLAY_CONTEXT.lock() {
        *previous = Some(context);
    }
}

fn reconcile_mode(is_minimized: bool, is_maximized: bool) -> ReconcileMode {
    if is_minimized {
        ReconcileMode::Skip
    } else if is_maximized {
        ReconcileMode::ViewportOnly
    } else {
        ReconcileMode::Full
    }
}

fn should_reconcile_geometry(
    explicitly_requested: bool,
    check_display_context: bool,
    display_context_changed: bool,
) -> bool {
    explicitly_requested || (check_display_context && display_context_changed)
}

fn logical_pixels_to_physical(logical: f64, scale_factor: f64) -> u64 {
    let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    let physical = (logical * scale).round();
    if !physical.is_finite() || physical <= 1.0 {
        1
    } else {
        physical.min(u32::MAX as f64) as u64
    }
}

fn state_position_is_visible<R: Runtime>(
    window: &WebviewWindow<R>,
    state: &MainWindowState,
) -> bool {
    let (Some(x), Some(y)) = (state.x, state.y) else {
        return false;
    };
    let saved_window = LogicalRect {
        x,
        y,
        width: state.width,
        height: state.height,
    };
    let Ok(monitors) = window.available_monitors() else {
        return false;
    };

    monitors.iter().any(|monitor| {
        let scale = monitor.scale_factor().max(0.1);
        let position = monitor.position();
        let size = monitor.size();
        let monitor_rect = LogicalRect {
            x: position.x as f64 / scale,
            y: position.y as f64 / scale,
            width: size.width as f64 / scale,
            height: size.height as f64 / scale,
        };
        has_visible_overlap(saved_window, monitor_rect)
    })
}

pub fn load_main_window_state() -> Option<MainWindowState> {
    let path = state_path().ok()?;
    if !path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(&path).ok()?;
    let mut state: MainWindowState = serde_json::from_str(&content).ok()?;
    let (width, height) = clamp_size(state.width, state.height);
    state.width = width;
    state.height = height;
    if let Some(x) = state.x {
        if !x.is_finite() {
            state.x = None;
        }
    }
    if let Some(y) = state.y {
        if !y.is_finite() {
            state.y = None;
        }
    }
    Some(state)
}

pub fn save_main_window_state(state: &MainWindowState) -> Result<(), String> {
    let (width, height) = clamp_size(state.width, state.height);
    let normalized = MainWindowState {
        width,
        height,
        x: state.x.filter(|v| v.is_finite()),
        y: state.y.filter(|v| v.is_finite()),
        maximized: state.maximized,
    };
    let path = state_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            format!(
                "创建窗口状态目录失败: path={}, error={}",
                parent.display(),
                e
            )
        })?;
    }
    let json = serde_json::to_string_pretty(&normalized)
        .map_err(|e| format!("序列化窗口状态失败: {}", e))?;
    atomic_write::write_string_atomic(&path, &json)
}

/// Capture logical size/position from a live window.
pub fn capture_main_window_state<R: Runtime>(
    window: &WebviewWindow<R>,
) -> Result<MainWindowState, String> {
    let scale = window.scale_factor().unwrap_or(1.0).max(0.1);
    let physical = window
        .inner_size()
        .map_err(|e| format!("读取窗口尺寸失败: {}", e))?;
    let width = physical.width as f64 / scale;
    let height = physical.height as f64 / scale;
    let (width, height) = clamp_size(width, height);

    let maximized = window.is_maximized().unwrap_or(false);
    let (x, y) = if maximized {
        // Keep last known non-maximized position if we already saved one.
        load_main_window_state()
            .map(|s| (s.x, s.y))
            .unwrap_or((None, None))
    } else {
        match window.outer_position() {
            Ok(pos) => {
                let lx = pos.x as f64 / scale;
                let ly = pos.y as f64 / scale;
                (
                    if lx.is_finite() { Some(lx) } else { None },
                    if ly.is_finite() { Some(ly) } else { None },
                )
            }
            Err(_) => (None, None),
        }
    };

    Ok(MainWindowState {
        width,
        height,
        x,
        y,
        maximized,
    })
}

pub fn capture_and_save_main_window<R: Runtime>(window: &WebviewWindow<R>) {
    if !remember_main_window_state_enabled() || window.is_minimized().unwrap_or(false) {
        return;
    }
    match capture_main_window_state(window) {
        Ok(state) => {
            if let Err(err) = save_main_window_state(&state) {
                logger::log_warn(&format!("[Window] 保存主窗口尺寸失败: {}", err));
            }
        }
        Err(err) => {
            logger::log_warn(&format!("[Window] 采集主窗口尺寸失败: {}", err));
        }
    }
}

/// Debounced save for continuous resize/move events.
/// Skips mid-drag thrashing; CloseRequested / tray destroy always force-save.
pub fn capture_and_save_main_window_debounced<R: Runtime>(window: &WebviewWindow<R>) {
    {
        let mut last = match LAST_SAVE_AT.lock() {
            Ok(guard) => guard,
            Err(_) => {
                capture_and_save_main_window(window);
                return;
            }
        };
        let now = Instant::now();
        if let Some(prev) = *last {
            if now.duration_since(prev) < SAVE_DEBOUNCE {
                return;
            }
        }
        *last = Some(now);
    }
    capture_and_save_main_window(window);
}

/// Schedule a work-area check and force the WebView bounds to be reapplied.
///
/// Reapplying the physical client size is intentional: remote-desktop and DPI transitions can
/// leave WebView2 with stale bounds until the user manually resizes the native window.
pub fn request_main_window_viewport_refresh() {
    PENDING_MAIN_WINDOW_RECONCILE.fetch_or(
        CHECK_DISPLAY_CONTEXT | REFRESH_WEBVIEW_VIEWPORT,
        Ordering::Release,
    );
}

/// Refresh after startup/tray restoration and ensure legacy saved coordinates fit this display.
pub fn request_main_window_restore_refresh() {
    PENDING_MAIN_WINDOW_RECONCILE.fetch_or(
        RECONCILE_GEOMETRY | REFRESH_WEBVIEW_VIEWPORT | CONTAIN_WINDOW_IN_WORK_AREA,
        Ordering::Release,
    );
}

pub fn request_main_window_resized() {
    PENDING_MAIN_WINDOW_RECONCILE.fetch_or(
        CHECK_DISPLAY_CONTEXT | REFRESH_WEBVIEW_VIEWPORT | SAVE_WINDOW_STATE_DEBOUNCED,
        Ordering::Release,
    );
}

pub fn request_main_window_moved() {
    PENDING_MAIN_WINDOW_RECONCILE.fetch_or(
        CHECK_DISPLAY_CONTEXT | SAVE_WINDOW_STATE_DEBOUNCED,
        Ordering::Release,
    );
}

fn sync_webview_to_inner_size<R: Runtime>(
    window: &WebviewWindow<R>,
    inner_size: PhysicalSize<u32>,
    force: bool,
) -> Result<bool, String> {
    let webview: &tauri::Webview<R> = window.as_ref();
    let current_size = webview
        .size()
        .map_err(|err| format!("读取主 WebView 尺寸失败: {}", err))?;
    if !force && current_size == inner_size {
        return Ok(false);
    }

    webview
        .set_size(Size::Physical(inner_size))
        .map_err(|err| format!("同步主 WebView 尺寸失败: {}", err))?;
    Ok(true)
}

fn reconcile_main_window_geometry<R: Runtime>(
    window: &WebviewWindow<R>,
    force_webview_refresh: bool,
    force_work_area_containment: bool,
    explicitly_requested: bool,
    check_display_context: bool,
) -> Result<ReconcileOutcome, String> {
    let mode = reconcile_mode(
        window.is_minimized().unwrap_or(false),
        window.is_maximized().unwrap_or(false),
    );
    if mode == ReconcileMode::Skip {
        return Ok(ReconcileOutcome::default());
    }

    let current_inner_size = window
        .inner_size()
        .map_err(|err| format!("读取主窗口内部尺寸失败: {}", err))?;

    if mode == ReconcileMode::ViewportOnly {
        sync_webview_to_inner_size(window, current_inner_size, force_webview_refresh)?;
        return Ok(ReconcileOutcome::default());
    }

    let monitor = match window.current_monitor() {
        Ok(Some(monitor)) => Some(monitor),
        Ok(None) | Err(_) => window
            .primary_monitor()
            .map_err(|err| format!("读取主显示器失败: {}", err))?,
    };
    let Some(monitor) = monitor else {
        let refreshed = sync_webview_to_inner_size(
            window,
            current_inner_size,
            force_webview_refresh,
        )?;
        logger::log_warn(if refreshed {
            "[Window] 未找到可用显示器；已仅刷新主 WebView 尺寸"
        } else {
            "[Window] 未找到可用显示器"
        });
        return Ok(ReconcileOutcome::default());
    };

    let current_outer_position = window
        .outer_position()
        .map_err(|err| format!("读取主窗口位置失败: {}", err))?;
    let current_outer_size = window
        .outer_size()
        .map_err(|err| format!("读取主窗口外部尺寸失败: {}", err))?;
    let scale_factor = monitor.scale_factor();
    let work_area = monitor.work_area();
    let work_area_rect = PixelRect {
        x: i64::from(work_area.position.x),
        y: i64::from(work_area.position.y),
        width: u64::from(work_area.size.width),
        height: u64::from(work_area.size.height),
    };
    // Compare the complete monitor topology rather than only `current_monitor()`. The latter
    // changes during a normal cross-monitor drag and must not snap the window to either screen.
    let display_context = window.available_monitors().ok().and_then(|monitors| {
        if monitors.is_empty() {
            return None;
        }
        Some(normalize_display_context(
            monitors
                .iter()
                .map(|available_monitor| {
                    let position = available_monitor.position();
                    let size = available_monitor.size();
                    let available_work_area = available_monitor.work_area();
                    MonitorContext {
                        bounds: PixelRect {
                            x: i64::from(position.x),
                            y: i64::from(position.y),
                            width: u64::from(size.width),
                            height: u64::from(size.height),
                        },
                        work_area: PixelRect {
                            x: i64::from(available_work_area.position.x),
                            y: i64::from(available_work_area.position.y),
                            width: u64::from(available_work_area.size.width),
                            height: u64::from(available_work_area.size.height),
                        },
                        scale_factor_bits: available_monitor.scale_factor().to_bits(),
                    }
                })
                .collect(),
        ))
    });
    let context_changed = display_context
        .as_ref()
        .map_or(false, has_display_context_changed);
    if !should_reconcile_geometry(
        explicitly_requested || force_work_area_containment,
        check_display_context,
        context_changed,
    ) {
        sync_webview_to_inner_size(
            window,
            current_inner_size,
            force_webview_refresh || context_changed,
        )?;
        if let Some(display_context) = display_context {
            remember_display_context(display_context);
        }
        return Ok(ReconcileOutcome::default());
    }

    let fitted = fit_window_to_work_area(
        PixelWindowGeometry {
            x: i64::from(current_outer_position.x),
            y: i64::from(current_outer_position.y),
            outer_width: u64::from(current_outer_size.width),
            outer_height: u64::from(current_outer_size.height),
            inner_width: u64::from(current_inner_size.width),
            inner_height: u64::from(current_inner_size.height),
        },
        work_area_rect,
        logical_pixels_to_physical(MIN_WIDTH, scale_factor),
        logical_pixels_to_physical(MIN_HEIGHT, scale_factor),
        logical_pixels_to_physical(MIN_VISIBLE_WIDTH, scale_factor),
        logical_pixels_to_physical(MIN_VISIBLE_HEIGHT, scale_factor),
        force_work_area_containment || context_changed,
    );

    let target_inner_size = PhysicalSize::new(
        u32::try_from(fitted.inner_width).unwrap_or(u32::MAX),
        u32::try_from(fitted.inner_height).unwrap_or(u32::MAX),
    );
    if fitted.resized {
        window
            .set_size(Size::Physical(target_inner_size))
            .map_err(|err| format!("按显示器工作区调整主窗口尺寸失败: {}", err))?;
    }

    if fitted.repositioned {
        let target_position = PhysicalPosition::new(
            fitted.x.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            fitted.y.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        );
        window
            .set_position(Position::Physical(target_position))
            .map_err(|err| format!("按显示器工作区调整主窗口位置失败: {}", err))?;
    }

    let geometry_changed = fitted.resized || fitted.repositioned;
    let actual_inner_size = window
        .inner_size()
        .map_err(|err| format!("读取校正后的主窗口内部尺寸失败: {}", err))?;
    sync_webview_to_inner_size(
        window,
        actual_inner_size,
        force_webview_refresh || fitted.resized || context_changed,
    )?;
    if let Some(display_context) = display_context {
        remember_display_context(display_context);
    }

    if geometry_changed {
        logger::log_info(&format!(
            "[Window] 已按当前显示器工作区校正主窗口: position=({}, {}), outer={}x{}, inner={}x{}",
            fitted.x,
            fitted.y,
            fitted.outer_width,
            fitted.outer_height,
            fitted.inner_width,
            fitted.inner_height
        ));
    }

    Ok(ReconcileOutcome { geometry_changed })
}

/// Flush a pending display-environment change from the main event loop.
pub fn flush_pending_main_window_reconcile<R: Runtime>(app: &AppHandle<R>) {
    let pending = PENDING_MAIN_WINDOW_RECONCILE.swap(0, Ordering::AcqRel);
    if pending == 0 {
        return;
    }

    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let force_webview_refresh = pending & REFRESH_WEBVIEW_VIEWPORT != 0;
    let force_work_area_containment = pending & CONTAIN_WINDOW_IN_WORK_AREA != 0;
    let explicitly_requested = pending & RECONCILE_GEOMETRY != 0;
    let check_display_context = pending & CHECK_DISPLAY_CONTEXT != 0;
    match reconcile_main_window_geometry(
        &window,
        force_webview_refresh,
        force_work_area_containment,
        explicitly_requested,
        check_display_context,
    ) {
        Ok(outcome) => {
            let is_follow_up = pending & SAVE_WINDOW_STATE_FORCED != 0;
            if outcome.geometry_changed && !is_follow_up {
                // Native geometry setters can settle on the next event-loop turn. Reconcile once
                // more and only then force-save the final accepted geometry.
                PENDING_MAIN_WINDOW_RECONCILE.fetch_or(
                    RECONCILE_GEOMETRY
                        | REFRESH_WEBVIEW_VIEWPORT
                        | SAVE_WINDOW_STATE_FORCED
                        | CONTAIN_WINDOW_IN_WORK_AREA,
                    Ordering::Release,
                );
            } else if is_follow_up {
                if outcome.geometry_changed {
                    logger::log_warn(
                        "[Window] 主窗口二次校正后仍受原生约束；停止自动重试并保存实际几何",
                    );
                }
                capture_and_save_main_window(&window);
            } else if pending & SAVE_WINDOW_STATE_DEBOUNCED != 0 {
                capture_and_save_main_window_debounced(&window);
            }
        }
        Err(err) => logger::log_warn(&format!("[Window] 主窗口显示环境同步失败: {}", err)),
    }
}

pub fn apply_state_to_window_config(config: &mut tauri::utils::config::WindowConfig) {
    if !remember_main_window_state_enabled() {
        return;
    }
    let Some(state) = load_main_window_state() else {
        return;
    };
    config.width = state.width;
    config.height = state.height;
    // Position is restored only after the window exists and can validate current monitors.
    config.maximized = state.maximized;
}

/// Apply saved geometry to an already-created main window (first launch / recreate).
pub fn restore_to_window<R: Runtime>(window: &WebviewWindow<R>) {
    request_main_window_restore_refresh();
    if !remember_main_window_state_enabled() {
        return;
    }
    let Some(state) = load_main_window_state() else {
        return;
    };

    if state.maximized {
        if let Err(err) = window.maximize() {
            logger::log_warn(&format!("[Window] 恢复最大化失败: {}", err));
        }
        return;
    }

    if let Err(err) = window.set_size(Size::Logical(LogicalSize {
        width: state.width,
        height: state.height,
    })) {
        logger::log_warn(&format!("[Window] 恢复窗口尺寸失败: {}", err));
    }

    if state_position_is_visible(window, &state) {
        let (x, y) = (state.x.unwrap_or_default(), state.y.unwrap_or_default());
        if let Err(err) = window.set_position(Position::Logical(LogicalPosition { x, y })) {
            logger::log_warn(&format!("[Window] 恢复窗口位置失败: {}", err));
        }
    } else {
        if let Err(err) = window.center() {
            logger::log_warn(&format!("[Window] 窗口位置无效，居中失败: {}", err));
        }
        if state.x.is_some() || state.y.is_some() {
            let repaired = MainWindowState {
                x: None,
                y: None,
                ..state
            };
            if let Err(err) = save_main_window_state(&repaired) {
                logger::log_warn(&format!("[Window] 清理无效窗口位置失败: {}", err));
            }
        }
    }
}

/// Helper for events that only give us a Window handle.
pub fn capture_and_save_from_window_handle<R: Runtime>(window: &Window<R>) {
    if window.label() != "main" {
        return;
    }
    let Some(webview) = window.app_handle().get_webview_window("main") else {
        return;
    };
    capture_and_save_main_window(&webview);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_state_memory_is_opt_in_by_default() {
        assert!(!config::UserConfig::default().remember_main_window_state);
    }

    #[test]
    fn clamp_size_enforces_minimum() {
        let (w, h) = clamp_size(100.0, 50.0);
        assert_eq!(w, MIN_WIDTH);
        assert_eq!(h, MIN_HEIGHT);
    }

    #[test]
    fn clamp_size_keeps_valid() {
        let (w, h) = clamp_size(1400.0, 900.0);
        assert_eq!(w, 1400.0);
        assert_eq!(h, 900.0);
    }

    #[test]
    fn visible_overlap_accepts_partially_visible_window() {
        let window = LogicalRect {
            x: 1880.0,
            y: 200.0,
            width: 1280.0,
            height: 800.0,
        };
        let monitor = LogicalRect {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        };
        assert!(!has_visible_overlap(window, monitor));

        let sufficiently_visible = LogicalRect {
            x: 1850.0,
            ..window
        };
        assert!(has_visible_overlap(sufficiently_visible, monitor));
    }

    #[test]
    fn visible_overlap_rejects_windows_minimized_offscreen() {
        let window = LogicalRect {
            x: -32000.0,
            y: -32000.0,
            width: 1280.0,
            height: 800.0,
        };
        let monitor = LogicalRect {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        };
        assert!(!has_visible_overlap(window, monitor));
    }

    #[test]
    fn fit_window_shrinks_outer_bounds_to_smaller_work_area() {
        let fitted = fit_window_to_work_area(
            PixelWindowGeometry {
                x: 100,
                y: 100,
                outer_width: 2020,
                outer_height: 1200,
                inner_width: 2004,
                inner_height: 1161,
            },
            PixelRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1040,
            },
            900,
            600,
            64,
            48,
            false,
        );

        assert_eq!(fitted.inner_width, 1904);
        assert_eq!(fitted.inner_height, 1001);
        assert_eq!(fitted.outer_width, 1920);
        assert_eq!(fitted.outer_height, 1040);
        assert_eq!((fitted.x, fitted.y), (0, 0));
        assert!(fitted.resized);
        assert!(fitted.repositioned);
    }

    #[test]
    fn fit_window_is_idempotent_after_shrinking() {
        let work_area = PixelRect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1040,
        };
        let first = fit_window_to_work_area(
            PixelWindowGeometry {
                x: 100,
                y: 100,
                outer_width: 2020,
                outer_height: 1200,
                inner_width: 2004,
                inner_height: 1161,
            },
            work_area,
            900,
            600,
            64,
            48,
            false,
        );
        let second = fit_window_to_work_area(
            PixelWindowGeometry {
                x: first.x,
                y: first.y,
                outer_width: first.outer_width,
                outer_height: first.outer_height,
                inner_width: first.inner_width,
                inner_height: first.inner_height,
            },
            work_area,
            900,
            600,
            64,
            48,
            false,
        );

        assert!(!second.resized);
        assert!(!second.repositioned);
        assert_eq!(
            (second.inner_width, second.inner_height),
            (first.inner_width, first.inner_height)
        );
    }

    #[test]
    fn fit_window_preserves_position_with_minimum_visible_overlap() {
        let fitted = fit_window_to_work_area(
            PixelWindowGeometry {
                x: 1856,
                y: 992,
                outer_width: 1280,
                outer_height: 800,
                inner_width: 1264,
                inner_height: 761,
            },
            PixelRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1040,
            },
            900,
            600,
            64,
            48,
            false,
        );

        assert_eq!((fitted.x, fitted.y), (1856, 992));
        assert!(!fitted.resized);
        assert!(!fitted.repositioned);
    }

    #[test]
    fn fit_window_recovers_fully_offscreen_position() {
        let fitted = fit_window_to_work_area(
            PixelWindowGeometry {
                x: -32000,
                y: -32000,
                outer_width: 1280,
                outer_height: 800,
                inner_width: 1264,
                inner_height: 761,
            },
            PixelRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1040,
            },
            900,
            600,
            64,
            48,
            false,
        );

        assert_eq!((fitted.x, fitted.y), (0, 0));
        assert!(!fitted.resized);
        assert!(fitted.repositioned);
    }

    #[test]
    fn fit_window_keeps_native_minimum_when_work_area_is_smaller() {
        let fitted = fit_window_to_work_area(
            PixelWindowGeometry {
                x: 100,
                y: 100,
                outer_width: 1280,
                outer_height: 800,
                inner_width: 1264,
                inner_height: 761,
            },
            PixelRect {
                x: 0,
                y: 0,
                width: 800,
                height: 500,
            },
            900,
            600,
            64,
            48,
            false,
        );

        assert_eq!((fitted.inner_width, fitted.inner_height), (900, 600));
        assert_eq!((fitted.outer_width, fitted.outer_height), (916, 639));
        assert_eq!((fitted.x, fitted.y), (0, 0));
        assert!(fitted.resized);
        assert!(fitted.repositioned);
    }

    #[test]
    fn fit_window_supports_negative_monitor_coordinates() {
        let fitted = fit_window_to_work_area(
            PixelWindowGeometry {
                x: -1800,
                y: 100,
                outer_width: 1280,
                outer_height: 800,
                inner_width: 1264,
                inner_height: 761,
            },
            PixelRect {
                x: -1920,
                y: 0,
                width: 1920,
                height: 1040,
            },
            900,
            600,
            64,
            48,
            false,
        );

        assert_eq!((fitted.x, fitted.y), (-1800, 100));
        assert!(!fitted.resized);
        assert!(!fitted.repositioned);
    }

    #[test]
    fn fit_window_does_not_snap_during_cross_monitor_drag() {
        let fitted = fit_window_to_work_area(
            PixelWindowGeometry {
                x: 1280,
                y: 100,
                outer_width: 1280,
                outer_height: 800,
                inner_width: 1264,
                inner_height: 761,
            },
            PixelRect {
                x: 1920,
                y: 0,
                width: 1920,
                height: 1040,
            },
            900,
            600,
            64,
            48,
            false,
        );

        assert_eq!((fitted.x, fitted.y), (1280, 100));
        assert!(!fitted.resized);
        assert!(!fitted.repositioned);
    }

    #[test]
    fn forced_containment_repairs_legacy_mixed_dpi_position() {
        let fitted = fit_window_to_work_area(
            PixelWindowGeometry {
                x: 1280,
                y: 100,
                outer_width: 1280,
                outer_height: 800,
                inner_width: 1264,
                inner_height: 761,
            },
            PixelRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1040,
            },
            900,
            600,
            64,
            48,
            true,
        );

        assert_eq!((fitted.x, fitted.y), (640, 100));
        assert!(!fitted.resized);
        assert!(fitted.repositioned);
    }

    #[test]
    fn display_context_is_stable_when_monitor_enumeration_order_changes() {
        let left = MonitorContext {
            bounds: PixelRect {
                x: -1920,
                y: 0,
                width: 1920,
                height: 1080,
            },
            work_area: PixelRect {
                x: -1920,
                y: 0,
                width: 1920,
                height: 1040,
            },
            scale_factor_bits: 1.0_f64.to_bits(),
        };
        let right = MonitorContext {
            bounds: PixelRect {
                x: 0,
                y: 0,
                width: 2560,
                height: 1440,
            },
            work_area: PixelRect {
                x: 0,
                y: 0,
                width: 2560,
                height: 1400,
            },
            scale_factor_bits: 1.5_f64.to_bits(),
        };

        assert_eq!(
            normalize_display_context(vec![left, right]),
            normalize_display_context(vec![right, left])
        );
    }

    #[test]
    fn logical_thresholds_are_converted_once_per_monitor_scale() {
        assert_eq!(logical_pixels_to_physical(900.0, 1.5), 1350);
        assert_eq!(logical_pixels_to_physical(64.0, 2.0), 128);
        assert_eq!(logical_pixels_to_physical(48.0, f64::NAN), 48);
    }

    #[test]
    fn reconcile_mode_skips_minimized_and_preserves_maximized_geometry() {
        assert_eq!(reconcile_mode(true, false), ReconcileMode::Skip);
        assert_eq!(reconcile_mode(true, true), ReconcileMode::Skip);
        assert_eq!(
            reconcile_mode(false, true),
            ReconcileMode::ViewportOnly
        );
        assert_eq!(reconcile_mode(false, false), ReconcileMode::Full);
    }

    #[test]
    fn ordinary_window_events_only_reconcile_after_topology_changes() {
        assert!(!should_reconcile_geometry(false, true, false));
        assert!(should_reconcile_geometry(false, true, true));
        assert!(should_reconcile_geometry(true, false, false));
    }
}
