#![cfg(windows)]

use std::ptr::{null, null_mut};

use muda::{ContextMenu, Menu, MenuEvent, MenuItem, Submenu};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetMenuItemID, GetSubMenu, SendMessageW, WM_COMMAND,
    WS_OVERLAPPED,
};

struct HiddenWindow(HWND);

impl HiddenWindow {
    fn new() -> Self {
        let class: Vec<u16> = "STATIC".encode_utf16().chain([0]).collect();
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                null(),
                WS_OVERLAPPED,
                0,
                0,
                1,
                1,
                null_mut(),
                null_mut(),
                null_mut(),
                null(),
            )
        };
        assert!(!hwnd.is_null(), "{}", std::io::Error::last_os_error());
        Self(hwnd)
    }
}

impl Drop for HiddenWindow {
    fn drop(&mut self) {
        unsafe { DestroyWindow(self.0) };
    }
}

#[test]
fn quit_menu_event_survives_more_than_70_000_menu_item_creations() {
    let window = HiddenWindow::new();
    let mut created = 0;
    let mut failed = Vec::new();
    for target in [0, 70_000, 140_000, 210_000] {
        for _ in created..target {
            drop(MenuItem::new("Quota", true, None));
        }
        created = target;

        let quit = MenuItem::with_id("tray_quit", "Quit", true, None);
        let submenu = Submenu::with_items("Cockpit Tools", true, &[&quit]).unwrap();
        let menu = Menu::with_items(&[&submenu]).unwrap();
        let popup = unsafe { GetSubMenu(menu.hpopupmenu() as _, 0) };
        assert!(!popup.is_null());
        let command = unsafe { GetMenuItemID(popup, 0) };
        assert_ne!(command, u32::MAX);

        // tray-icon attaches the same subclass to its window. Win32 places the
        // menu command in WM_COMMAND's low word with a zero notification code.
        unsafe {
            menu.attach_menu_subclass_for_hwnd(window.0 as _);
            SendMessageW(window.0, WM_COMMAND, command as u16 as usize, 0);
            menu.detach_menu_subclass_from_hwnd(window.0 as _);
        }
        let event = MenuEvent::receiver().try_recv();
        let received_quit = matches!(event, Ok(event) if event.id() == quit.id());
        println!("{created:>6} items: command {command:>6}, quit event {received_quit}");
        if !received_quit || !(1000..=0xefff).contains(&command) {
            failed.push(created);
        }
        assert!(
            MenuEvent::receiver().try_recv().is_err(),
            "duplicate menu event"
        );
    }
    assert!(
        failed.is_empty(),
        "quit dispatch failed at {failed:?} items"
    );
}
