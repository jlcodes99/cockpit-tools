//! Persist and restore the main window size/position (#948 / #1132).
//!
//! Independent of general user config so frequent resize writes stay lightweight.
//! Only applies to the `main` window — floating card / OAuth windows are ignored.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{
    LogicalPosition, LogicalSize, Manager, Position, Runtime, Size, WebviewWindow, Window,
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

static LAST_SAVE_AT: Mutex<Option<Instant>> = Mutex::new(None);

#[derive(Debug, Clone, Copy)]
struct LogicalRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
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

fn maximized_state(saved: Option<MainWindowState>) -> MainWindowState {
    MainWindowState {
        maximized: true,
        ..saved.unwrap_or_default()
    }
}

fn has_visible_overlap(window: LogicalRect, monitor: LogicalRect) -> bool {
    let overlap_width =
        (window.x + window.width).min(monitor.x + monitor.width) - window.x.max(monitor.x);
    let overlap_height =
        (window.y + window.height).min(monitor.y + monitor.height) - window.y.max(monitor.y);
    overlap_width >= MIN_VISIBLE_WIDTH && overlap_height >= MIN_VISIBLE_HEIGHT
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
    if window.is_maximized().unwrap_or(false) {
        return Ok(maximized_state(load_main_window_state()));
    }
    let scale = window.scale_factor().unwrap_or(1.0).max(0.1);
    let physical = window
        .inner_size()
        .map_err(|e| format!("读取窗口尺寸失败: {}", e))?;
    let width = physical.width as f64 / scale;
    let height = physical.height as f64 / scale;
    let (width, height) = clamp_size(width, height);

    let (x, y) = match window.outer_position() {
        Ok(pos) => (Some(pos.x as f64 / scale), Some(pos.y as f64 / scale)),
        Err(_) => (None, None),
    };

    Ok(MainWindowState {
        width,
        height,
        x,
        y,
        maximized: false,
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

pub fn apply_state_to_window_config(config: &mut tauri::utils::config::WindowConfig) {
    if !remember_main_window_state_enabled() {
        return;
    }
    let Some(state) = load_main_window_state() else {
        return;
    };
    config.width = state.width;
    config.height = state.height;
    // Apply position and maximization after creation so the window has normal restore bounds.
}

/// Apply saved geometry to an already-created main window (first launch / recreate).
pub fn restore_to_window<R: Runtime>(window: &WebviewWindow<R>) {
    if !remember_main_window_state_enabled() {
        return;
    }
    let Some(state) = load_main_window_state() else {
        return;
    };

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

    if state.maximized {
        if let Err(err) = window.maximize() {
            logger::log_warn(&format!("[Window] 恢复最大化失败: {}", err));
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

pub fn capture_and_save_from_window_handle_debounced<R: Runtime>(window: &Window<R>) {
    if window.label() != "main" {
        return;
    }
    let Some(webview) = window.app_handle().get_webview_window("main") else {
        return;
    };
    capture_and_save_main_window_debounced(&webview);
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
    fn maximizing_preserves_saved_normal_window_bounds() {
        let normal = MainWindowState {
            width: 1420.0,
            height: 760.0,
            x: Some(120.0),
            y: Some(70.0),
            maximized: false,
        };
        let maximized = maximized_state(Some(normal.clone()));
        assert_eq!(
            maximized,
            MainWindowState {
                maximized: true,
                ..normal
            }
        );
        assert_eq!(maximized_state(Some(maximized.clone())), maximized);
        assert_eq!(
            maximized_state(None),
            MainWindowState {
                maximized: true,
                ..MainWindowState::default()
            }
        );
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
}
