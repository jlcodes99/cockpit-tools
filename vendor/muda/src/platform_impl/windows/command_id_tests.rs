use std::ptr::null_mut;

use crate::{Menu, MenuEvent, MenuItem, Submenu};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetMenu, GetMenuItemID, GetSubMenu, SendMessageW, WM_COMMAND,
    WS_OVERLAPPEDWINDOW,
};

#[test]
fn menu_commands_survive_repeated_wraparound() {
    let class: Vec<_> = "STATIC".encode_utf16().chain([0]).collect();
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            100,
            100,
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
        )
    };
    assert!(!hwnd.is_null());

    for round in 0..4 {
        if round > 0 {
            for _ in 0..70_000 {
                let _item = MenuItem::new("Transient", true, None);
            }
        }

        let menu = Menu::new();
        let accounts = Submenu::new("Accounts", true);
        accounts
            .append(&MenuItem::with_id("other", "Other", true, None))
            .unwrap();
        menu.append(&accounts).unwrap();
        menu.append(&MenuItem::with_id("quit", "Quit", true, None))
            .unwrap();
        unsafe { menu.init_for_hwnd(hwnd as isize).unwrap() };
        let native = unsafe { GetMenu(hwnd) };
        for (id, expected_event) in [
            (unsafe { GetMenuItemID(native, 1) }, "quit"),
            (unsafe { GetMenuItemID(GetSubMenu(native, 0), 0) }, "other"),
        ] {
            assert!((1000..0xf000).contains(&id));
            unsafe { SendMessageW(hwnd, WM_COMMAND, id as u16 as usize, 0) };
            let event = MenuEvent::receiver()
                .try_recv()
                .expect("Win32 menu selection must emit an event");
            assert_eq!(event.id().0, expected_event);
        }
    }

    unsafe { DestroyWindow(hwnd) };
}
