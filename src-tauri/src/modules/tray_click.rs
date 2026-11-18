use tauri::{
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconEvent},
    Runtime,
};

use crate::modules::{floating_card_window, logger};

#[cfg(any(test, target_os = "macos"))]
#[derive(Debug, PartialEq, Eq)]
enum MacosTrayClickAction {
    OpenMainWindow,
    ToggleQuotaMenu,
}

#[cfg(any(test, target_os = "macos"))]
fn macos_tray_click_action(
    button: MouseButton,
    state: MouseButtonState,
    left_click_opens_main_window: bool,
) -> Option<MacosTrayClickAction> {
    use MacosTrayClickAction::{OpenMainWindow, ToggleQuotaMenu};

    match (button, state) {
        (MouseButton::Left, MouseButtonState::Down) if left_click_opens_main_window => {
            Some(OpenMainWindow)
        }
        (MouseButton::Left | MouseButton::Right, MouseButtonState::Down) => Some(ToggleQuotaMenu),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
pub(super) fn handle_tray_event<R: Runtime>(tray: &TrayIcon<R>, event: TrayIconEvent) {
    let TrayIconEvent::Click {
        button,
        button_state,
        rect,
        ..
    } = event
    else {
        return;
    };
    let left_click_opens_main_window = button == MouseButton::Left
        && crate::modules::config::menu_bar_left_click_opens_main_window();
    match macos_tray_click_action(button, button_state, left_click_opens_main_window) {
        Some(MacosTrayClickAction::OpenMainWindow) => {
            if let Err(error) = floating_card_window::show_main_window(tray.app_handle()) {
                logger::log_warn(&format!("[Tray] 左键恢复主窗口失败: {error}"));
            }
        }
        Some(MacosTrayClickAction::ToggleQuotaMenu) => {
            let app = tray.app_handle().clone();
            let app_for_menu = app.clone();
            let _ = app.run_on_main_thread(move || {
                crate::modules::macos_native_menu::toggle_tray_menu(&app_for_menu, rect);
            });
        }
        None => {}
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn handle_tray_event<R: Runtime>(tray: &TrayIcon<R>, event: TrayIconEvent) {
    let click = match event {
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } => "左键",
        TrayIconEvent::DoubleClick {
            button: MouseButton::Left,
            ..
        } => "双击",
        _ => return,
    };
    if let Err(error) = floating_card_window::show_main_window(tray.app_handle()) {
        logger::log_warn(&format!("[Tray] {click}恢复主窗口失败: {error}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use MacosTrayClickAction::{OpenMainWindow, ToggleQuotaMenu};
    use MouseButton::{Left, Middle, Right};
    use MouseButtonState::{Down, Up};

    #[test]
    fn macos_left_click_follows_preference_and_right_click_keeps_quota_menu() {
        for opens_main_window in [false, true] {
            let expected_left_click = if opens_main_window {
                OpenMainWindow
            } else {
                ToggleQuotaMenu
            };
            assert_eq!(
                macos_tray_click_action(Left, Down, opens_main_window),
                Some(expected_left_click)
            );
            assert_eq!(
                macos_tray_click_action(Right, Down, opens_main_window),
                Some(ToggleQuotaMenu)
            );
            for (button, state) in [(Left, Up), (Right, Up), (Middle, Down)] {
                assert_eq!(
                    macos_tray_click_action(button, state, opens_main_window),
                    None
                );
            }
        }
    }
}
