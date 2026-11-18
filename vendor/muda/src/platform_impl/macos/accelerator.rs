// Copyright 2022-2022 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use keyboard_types::{Code, Modifiers};
use objc2_app_kit::NSEventModifierFlags;

use crate::accelerator::{Accelerator, AcceleratorParseError};

impl Accelerator {
    /// Return the string value of this hotkey, without modifiers.
    ///
    /// Returns the empty string if no key equivalent is known.
    pub fn key_equivalent(self) -> Result<String, AcceleratorParseError> {
        Ok(match self.key {
            Code::KeyA => "a".into(),
            Code::KeyB => "b".into(),
            Code::KeyC => "c".into(),
            Code::KeyD => "d".into(),
            Code::KeyE => "e".into(),
            Code::KeyF => "f".into(),
            Code::KeyG => "g".into(),
            Code::KeyH => "h".into(),
            Code::KeyI => "i".into(),
            Code::KeyJ => "j".into(),
            Code::KeyK => "k".into(),
            Code::KeyL => "l".into(),
            Code::KeyM => "m".into(),
            Code::KeyN => "n".into(),
            Code::KeyO => "o".into(),
            Code::KeyP => "p".into(),
            Code::KeyQ => "q".into(),
            Code::KeyR => "r".into(),
            Code::KeyS => "s".into(),
            Code::KeyT => "t".into(),
            Code::KeyU => "u".into(),
            Code::KeyV => "v".into(),
            Code::KeyW => "w".into(),
            Code::KeyX => "x".into(),
            Code::KeyY => "y".into(),
            Code::KeyZ => "z".into(),
            Code::Digit0 => "0".into(),
            Code::Digit1 => "1".into(),
            Code::Digit2 => "2".into(),
            Code::Digit3 => "3".into(),
            Code::Digit4 => "4".into(),
            Code::Digit5 => "5".into(),
            Code::Digit6 => "6".into(),
            Code::Digit7 => "7".into(),
            Code::Digit8 => "8".into(),
            Code::Digit9 => "9".into(),
            Code::Comma => ",".into(),
            Code::Minus => "-".into(),
            Code::Period => ".".into(),
            Code::Space => "\u{0020}".into(),
            Code::Equal => "=".into(),
            Code::Semicolon => ";".into(),
            Code::Slash => "/".into(),
            Code::Backslash => "\\".into(),
            Code::Quote => "\'".into(),
            Code::Backquote => "`".into(),
            Code::BracketLeft => "[".into(),
            Code::BracketRight => "]".into(),
            Code::Tab => "⇥".into(),
            Code::Escape => "\u{001b}".into(),
            // from NSText.h
            Code::Enter => "\u{0003}".into(),
            Code::Backspace => "\u{0008}".into(),
            Code::Delete => "\u{007f}".into(),
            // from NSEvent.h
            Code::Insert => "\u{F727}".into(),
            Code::Home => "\u{F729}".into(),
            Code::End => "\u{F72B}".into(),
            Code::PageUp => "\u{F72C}".into(),
            Code::PageDown => "\u{F72D}".into(),
            Code::PrintScreen => "\u{F72E}".into(),
            Code::ScrollLock => "\u{F72F}".into(),
            Code::ArrowUp => "\u{F700}".into(),
            Code::ArrowDown => "\u{F701}".into(),
            Code::ArrowLeft => "\u{F702}".into(),
            Code::ArrowRight => "\u{F703}".into(),
            Code::F1 => "\u{F704}".into(),
            Code::F2 => "\u{F705}".into(),
            Code::F3 => "\u{F706}".into(),
            Code::F4 => "\u{F707}".into(),
            Code::F5 => "\u{F708}".into(),
            Code::F6 => "\u{F709}".into(),
            Code::F7 => "\u{F70A}".into(),
            Code::F8 => "\u{F70B}".into(),
            Code::F9 => "\u{F70C}".into(),
            Code::F10 => "\u{F70D}".into(),
            Code::F11 => "\u{F70E}".into(),
            Code::F12 => "\u{F70F}".into(),
            Code::F13 => "\u{F710}".into(),
            Code::F14 => "\u{F711}".into(),
            Code::F15 => "\u{F712}".into(),
            Code::F16 => "\u{F713}".into(),
            Code::F17 => "\u{F714}".into(),
            Code::F18 => "\u{F715}".into(),
            Code::F19 => "\u{F716}".into(),
            Code::F20 => "\u{F717}".into(),
            Code::F21 => "\u{F718}".into(),
            Code::F22 => "\u{F719}".into(),
            Code::F23 => "\u{F71A}".into(),
            Code::F24 => "\u{F71B}".into(),
            key => return Err(AcceleratorParseError::UnsupportedKey(key.to_string())),
        })
    }

    /// Return the modifiers of this hotkey, as an NSEventModifierFlags bitflag.
    pub fn key_modifier_mask(self) -> NSEventModifierFlags {
        let mods: Modifiers = self.mods;
        let mut flags = NSEventModifierFlags::empty();
        if mods.contains(Modifiers::SHIFT) {
            flags.insert(NSEventModifierFlags::Shift);
        }
        if mods.contains(Modifiers::SUPER) {
            flags.insert(NSEventModifierFlags::Command);
        }
        if mods.contains(Modifiers::ALT) {
            flags.insert(NSEventModifierFlags::Option);
        }
        if mods.contains(Modifiers::CONTROL) {
            flags.insert(NSEventModifierFlags::Control);
        }
        flags
    }
}
