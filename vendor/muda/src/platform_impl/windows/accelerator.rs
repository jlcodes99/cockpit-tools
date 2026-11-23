// Copyright 2022-2022 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use std::fmt;

use keyboard_types::{Code, Modifiers};
use windows_sys::Win32::UI::{
    Input::KeyboardAndMouse::*,
    WindowsAndMessaging::{ACCEL, FALT, FCONTROL, FSHIFT, FVIRTKEY},
};

use crate::accelerator::{Accelerator, AcceleratorParseError};

impl Accelerator {
    // Convert a hotkey to an accelerator.
    pub fn to_accel(&self, menu_id: u16) -> crate::Result<ACCEL> {
        let mut virt_key = FVIRTKEY;
        let key_mods: Modifiers = self.mods;
        if key_mods.contains(Modifiers::CONTROL) {
            virt_key |= FCONTROL;
        }
        if key_mods.contains(Modifiers::ALT) {
            virt_key |= FALT;
        }
        if key_mods.contains(Modifiers::SHIFT) {
            virt_key |= FSHIFT;
        }

        let vk_code = key_to_vk(&self.key)?;
        let mod_code = vk_code >> 8;
        if mod_code & 0x1 != 0 {
            virt_key |= FSHIFT;
        }
        if mod_code & 0x02 != 0 {
            virt_key |= FCONTROL;
        }
        if mod_code & 0x04 != 0 {
            virt_key |= FALT;
        }
        let raw_key = vk_code & 0x00ff;

        Ok(ACCEL {
            fVirt: virt_key,
            key: raw_key,
            cmd: menu_id,
        })
    }
}

// used to build accelerators table from Key
fn key_to_vk(key: &Code) -> Result<VIRTUAL_KEY, AcceleratorParseError> {
    Ok(match key {
        Code::KeyA => VK_A,
        Code::KeyB => VK_B,
        Code::KeyC => VK_C,
        Code::KeyD => VK_D,
        Code::KeyE => VK_E,
        Code::KeyF => VK_F,
        Code::KeyG => VK_G,
        Code::KeyH => VK_H,
        Code::KeyI => VK_I,
        Code::KeyJ => VK_J,
        Code::KeyK => VK_K,
        Code::KeyL => VK_L,
        Code::KeyM => VK_M,
        Code::KeyN => VK_N,
        Code::KeyO => VK_O,
        Code::KeyP => VK_P,
        Code::KeyQ => VK_Q,
        Code::KeyR => VK_R,
        Code::KeyS => VK_S,
        Code::KeyT => VK_T,
        Code::KeyU => VK_U,
        Code::KeyV => VK_V,
        Code::KeyW => VK_W,
        Code::KeyX => VK_X,
        Code::KeyY => VK_Y,
        Code::KeyZ => VK_Z,
        Code::Digit0 => VK_0,
        Code::Digit1 => VK_1,
        Code::Digit2 => VK_2,
        Code::Digit3 => VK_3,
        Code::Digit4 => VK_4,
        Code::Digit5 => VK_5,
        Code::Digit6 => VK_6,
        Code::Digit7 => VK_7,
        Code::Digit8 => VK_8,
        Code::Digit9 => VK_9,
        Code::Equal => VK_OEM_PLUS,
        Code::Comma => VK_OEM_COMMA,
        Code::Minus => VK_OEM_MINUS,
        Code::Period => VK_OEM_PERIOD,
        Code::Semicolon => VK_OEM_1,
        Code::Slash => VK_OEM_2,
        Code::Backquote => VK_OEM_3,
        Code::BracketLeft => VK_OEM_4,
        Code::Backslash => VK_OEM_5,
        Code::BracketRight => VK_OEM_6,
        Code::Quote => VK_OEM_7,
        Code::Backspace => VK_BACK,
        Code::Tab => VK_TAB,
        Code::Space => VK_SPACE,
        Code::Enter => VK_RETURN,
        Code::Pause => VK_PAUSE,
        Code::CapsLock => VK_CAPITAL,
        Code::KanaMode => VK_KANA,
        Code::Escape => VK_ESCAPE,
        Code::NonConvert => VK_NONCONVERT,
        Code::PageUp => VK_PRIOR,
        Code::PageDown => VK_NEXT,
        Code::End => VK_END,
        Code::Home => VK_HOME,
        Code::ArrowLeft => VK_LEFT,
        Code::ArrowUp => VK_UP,
        Code::ArrowRight => VK_RIGHT,
        Code::ArrowDown => VK_DOWN,
        Code::PrintScreen => VK_SNAPSHOT,
        Code::Insert => VK_INSERT,
        Code::Delete => VK_DELETE,
        Code::Help => VK_HELP,
        Code::ContextMenu => VK_APPS,
        Code::F1 => VK_F1,
        Code::F2 => VK_F2,
        Code::F3 => VK_F3,
        Code::F4 => VK_F4,
        Code::F5 => VK_F5,
        Code::F6 => VK_F6,
        Code::F7 => VK_F7,
        Code::F8 => VK_F8,
        Code::F9 => VK_F9,
        Code::F10 => VK_F10,
        Code::F11 => VK_F11,
        Code::F12 => VK_F12,
        Code::F13 => VK_F13,
        Code::F14 => VK_F14,
        Code::F15 => VK_F15,
        Code::F16 => VK_F16,
        Code::F17 => VK_F17,
        Code::F18 => VK_F18,
        Code::F19 => VK_F19,
        Code::F20 => VK_F20,
        Code::F21 => VK_F21,
        Code::F22 => VK_F22,
        Code::F23 => VK_F23,
        Code::F24 => VK_F24,
        Code::NumLock => VK_NUMLOCK,
        Code::ScrollLock => VK_SCROLL,
        Code::BrowserBack => VK_BROWSER_BACK,
        Code::BrowserForward => VK_BROWSER_FORWARD,
        Code::BrowserRefresh => VK_BROWSER_REFRESH,
        Code::BrowserStop => VK_BROWSER_STOP,
        Code::BrowserSearch => VK_BROWSER_SEARCH,
        Code::BrowserFavorites => VK_BROWSER_FAVORITES,
        Code::BrowserHome => VK_BROWSER_HOME,
        Code::AudioVolumeMute => VK_VOLUME_MUTE,
        Code::AudioVolumeDown => VK_VOLUME_DOWN,
        Code::AudioVolumeUp => VK_VOLUME_UP,
        Code::MediaTrackNext => VK_MEDIA_NEXT_TRACK,
        Code::MediaTrackPrevious => VK_MEDIA_PREV_TRACK,
        Code::MediaStop => VK_MEDIA_STOP,
        Code::MediaPlayPause => VK_MEDIA_PLAY_PAUSE,
        Code::LaunchMail => VK_LAUNCH_MAIL,
        Code::Convert => VK_CONVERT,
        key => return Err(AcceleratorParseError::UnsupportedKey(key.to_string())),
    })
}

impl fmt::Display for Accelerator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let key_mods: Modifiers = self.mods;
        if key_mods.contains(Modifiers::CONTROL) {
            write!(f, "Ctrl+")?;
        }
        if key_mods.contains(Modifiers::SHIFT) {
            write!(f, "Shift+")?;
        }
        if key_mods.contains(Modifiers::ALT) {
            write!(f, "Alt+")?;
        }
        if key_mods.contains(Modifiers::SUPER) {
            write!(f, "Windows+")?;
        }
        match &self.key {
            Code::KeyA => write!(f, "A"),
            Code::KeyB => write!(f, "B"),
            Code::KeyC => write!(f, "C"),
            Code::KeyD => write!(f, "D"),
            Code::KeyE => write!(f, "E"),
            Code::KeyF => write!(f, "F"),
            Code::KeyG => write!(f, "G"),
            Code::KeyH => write!(f, "H"),
            Code::KeyI => write!(f, "I"),
            Code::KeyJ => write!(f, "J"),
            Code::KeyK => write!(f, "K"),
            Code::KeyL => write!(f, "L"),
            Code::KeyM => write!(f, "M"),
            Code::KeyN => write!(f, "N"),
            Code::KeyO => write!(f, "O"),
            Code::KeyP => write!(f, "P"),
            Code::KeyQ => write!(f, "Q"),
            Code::KeyR => write!(f, "R"),
            Code::KeyS => write!(f, "S"),
            Code::KeyT => write!(f, "T"),
            Code::KeyU => write!(f, "U"),
            Code::KeyV => write!(f, "V"),
            Code::KeyW => write!(f, "W"),
            Code::KeyX => write!(f, "X"),
            Code::KeyY => write!(f, "Y"),
            Code::KeyZ => write!(f, "Z"),
            Code::Digit0 => write!(f, "0"),
            Code::Digit1 => write!(f, "1"),
            Code::Digit2 => write!(f, "2"),
            Code::Digit3 => write!(f, "3"),
            Code::Digit4 => write!(f, "4"),
            Code::Digit5 => write!(f, "5"),
            Code::Digit6 => write!(f, "6"),
            Code::Digit7 => write!(f, "7"),
            Code::Digit8 => write!(f, "8"),
            Code::Digit9 => write!(f, "9"),
            Code::Comma => write!(f, ","),
            Code::Minus => write!(f, "-"),
            Code::Period => write!(f, "."),
            Code::Space => write!(f, "Space"),
            Code::Equal => write!(f, "="),
            Code::Semicolon => write!(f, ";"),
            Code::Slash => write!(f, "/"),
            Code::Backslash => write!(f, "\\"),
            Code::Quote => write!(f, "\'"),
            Code::Backquote => write!(f, "`"),
            Code::BracketLeft => write!(f, "["),
            Code::BracketRight => write!(f, "]"),
            Code::Tab => write!(f, "Tab"),
            Code::Escape => write!(f, "Esc"),
            Code::Delete => write!(f, "Del"),
            Code::Insert => write!(f, "Ins"),
            Code::PageUp => write!(f, "PgUp"),
            Code::PageDown => write!(f, "PgDn"),
            // These names match LibreOffice.
            Code::ArrowLeft => write!(f, "Left"),
            Code::ArrowRight => write!(f, "Right"),
            Code::ArrowUp => write!(f, "Up"),
            Code::ArrowDown => write!(f, "Down"),
            _ => write!(f, "{:?}", self.key),
        }
    }
}
