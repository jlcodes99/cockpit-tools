// Copyright 2022-2022 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use gtk::gdk;
use keyboard_types::{Code, Modifiers};

use crate::accelerator::{Accelerator, AcceleratorParseError};

pub fn to_gtk_mnemonic<S: AsRef<str>>(string: S) -> String {
    string
        .as_ref()
        .replace("_", "__")
        .replace("&&", "[~~]")
        .replace('&', "_")
        .replace("[~~]", "&")
}

pub fn from_gtk_mnemonic<S: AsRef<str>>(string: S) -> String {
    string
        .as_ref()
        .replace("&", "&&")
        .replace("__", "[~~]")
        .replace('_', "&")
        .replace("[~~]", "_")
}

pub fn parse_accelerator(
    accelerator: &Accelerator,
) -> Result<(gdk::ModifierType, u32), AcceleratorParseError> {
    let key = match &accelerator.key {
        Code::KeyA => 'A' as u32,
        Code::KeyB => 'B' as u32,
        Code::KeyC => 'C' as u32,
        Code::KeyD => 'D' as u32,
        Code::KeyE => 'E' as u32,
        Code::KeyF => 'F' as u32,
        Code::KeyG => 'G' as u32,
        Code::KeyH => 'H' as u32,
        Code::KeyI => 'I' as u32,
        Code::KeyJ => 'J' as u32,
        Code::KeyK => 'K' as u32,
        Code::KeyL => 'L' as u32,
        Code::KeyM => 'M' as u32,
        Code::KeyN => 'N' as u32,
        Code::KeyO => 'O' as u32,
        Code::KeyP => 'P' as u32,
        Code::KeyQ => 'Q' as u32,
        Code::KeyR => 'R' as u32,
        Code::KeyS => 'S' as u32,
        Code::KeyT => 'T' as u32,
        Code::KeyU => 'U' as u32,
        Code::KeyV => 'V' as u32,
        Code::KeyW => 'W' as u32,
        Code::KeyX => 'X' as u32,
        Code::KeyY => 'Y' as u32,
        Code::KeyZ => 'Z' as u32,
        Code::Digit0 => '0' as u32,
        Code::Digit1 => '1' as u32,
        Code::Digit2 => '2' as u32,
        Code::Digit3 => '3' as u32,
        Code::Digit4 => '4' as u32,
        Code::Digit5 => '5' as u32,
        Code::Digit6 => '6' as u32,
        Code::Digit7 => '7' as u32,
        Code::Digit8 => '8' as u32,
        Code::Digit9 => '9' as u32,
        Code::Comma => ',' as u32,
        Code::Minus => '-' as u32,
        Code::Period => '.' as u32,
        Code::Space => ' ' as u32,
        Code::Equal => '=' as u32,
        Code::Semicolon => ';' as u32,
        Code::Slash => '/' as u32,
        Code::Backslash => '\\' as u32,
        Code::Quote => '\'' as u32,
        Code::Backquote => '`' as u32,
        Code::BracketLeft => '[' as u32,
        Code::BracketRight => ']' as u32,
        key => {
            if let Some(gdk_key) = key_to_raw_key(key) {
                *gdk_key
            } else {
                return Err(AcceleratorParseError::UnsupportedKey(key.to_string()));
            }
        }
    };

    Ok((modifiers_to_gdk_modifier_type(accelerator.mods), key))
}

fn modifiers_to_gdk_modifier_type(modifiers: Modifiers) -> gdk::ModifierType {
    let mut result = gdk::ModifierType::empty();

    result.set(
        gdk::ModifierType::MOD1_MASK,
        modifiers.contains(Modifiers::ALT),
    );
    result.set(
        gdk::ModifierType::CONTROL_MASK,
        modifiers.contains(Modifiers::CONTROL),
    );
    result.set(
        gdk::ModifierType::SHIFT_MASK,
        modifiers.contains(Modifiers::SHIFT),
    );
    result.set(
        gdk::ModifierType::META_MASK,
        modifiers.contains(Modifiers::SUPER),
    );

    result
}

fn key_to_raw_key(src: &Code) -> Option<gdk::keys::Key> {
    use gdk::keys::constants::*;
    Some(match src {
        Code::Escape => Escape,
        Code::Backspace => BackSpace,

        Code::Tab => Tab,
        Code::Enter => Return,

        Code::ControlLeft => Control_L,
        Code::AltLeft => Alt_L,
        Code::ShiftLeft => Shift_L,
        Code::MetaLeft => Super_L,

        Code::ControlRight => Control_R,
        Code::AltRight => Alt_R,
        Code::ShiftRight => Shift_R,
        Code::MetaRight => Super_R,

        Code::CapsLock => Caps_Lock,
        Code::F1 => F1,
        Code::F2 => F2,
        Code::F3 => F3,
        Code::F4 => F4,
        Code::F5 => F5,
        Code::F6 => F6,
        Code::F7 => F7,
        Code::F8 => F8,
        Code::F9 => F9,
        Code::F10 => F10,
        Code::F11 => F11,
        Code::F12 => F12,
        Code::F13 => F13,
        Code::F14 => F14,
        Code::F15 => F15,
        Code::F16 => F16,
        Code::F17 => F17,
        Code::F18 => F18,
        Code::F19 => F19,
        Code::F20 => F20,
        Code::F21 => F21,
        Code::F22 => F22,
        Code::F23 => F23,
        Code::F24 => F24,

        Code::PrintScreen => Print,
        Code::ScrollLock => Scroll_Lock,
        // Pause/Break not audio.
        Code::Pause => Pause,

        Code::Insert => Insert,
        Code::Delete => Delete,
        Code::Home => Home,
        Code::End => End,
        Code::PageUp => Page_Up,
        Code::PageDown => Page_Down,

        Code::NumLock => Num_Lock,

        Code::ArrowUp => Up,
        Code::ArrowDown => Down,
        Code::ArrowLeft => Left,
        Code::ArrowRight => Right,

        Code::ContextMenu => Menu,
        Code::WakeUp => WakeUp,
        _ => return None,
    })
}
