// Copyright 2022-2022 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! Accelerators describe keyboard shortcuts for menu items.
//!
//! [`Accelerator`s](crate::accelerator::Accelerator) are used to define a keyboard shortcut consisting
//! of an optional combination of modifier keys (provided by [`Modifiers`]) and
//! one key ([`Code`]).
//!
//! # Examples
//! They can be created directly
//! ```no_run
//! # use muda::accelerator::{Accelerator, Modifiers, Code};
//! let accelerator = Accelerator::new(Some(Modifiers::SHIFT), Code::KeyQ);
//! let accelerator_without_mods = Accelerator::new(None, Code::KeyQ);
//! ```
//! or from `&str`, note that all modifiers
//! have to be listed before the non-modifier key, `shift+alt+KeyQ` is legal,
//! whereas `shift+q+alt` is not.
//! ```no_run
//! # use muda::accelerator::{Accelerator};
//! let accelerator: Accelerator = "shift+alt+KeyQ".parse().unwrap();
//! # // This assert exists to ensure a test breaks once the
//! # // statement above about ordering is no longer valid.
//! # assert!("shift+KeyQ+alt".parse::<Accelerator>().is_err());
//! ```
//!

pub use keyboard_types::{Code, Modifiers};
use std::{borrow::Borrow, hash::Hash, str::FromStr};

#[cfg(target_os = "macos")]
pub const CMD_OR_CTRL: Modifiers = Modifiers::SUPER;
#[cfg(not(target_os = "macos"))]
pub const CMD_OR_CTRL: Modifiers = Modifiers::CONTROL;

#[derive(thiserror::Error, Debug)]
pub enum AcceleratorParseError {
    #[error("Couldn't recognize \"{0}\" as a valid key for accelerator, if you feel like it should be, please report this to https://github.com/tauri-apps/muda")]
    UnsupportedKey(String),
    #[error("Found empty token while parsing accelerator: {0}")]
    EmptyToken(String),
    #[error("Invalid accelerator format: \"{0}\", an accelerator should have the modifiers first and only one main key, for example: \"Shift + Alt + K\"")]
    InvalidFormat(String),
}

/// A keyboard shortcut that consists of an optional combination
/// of modifier keys (provided by [`Modifiers`] and
/// one key ([`Code`]).
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Accelerator {
    pub(crate) mods: Modifiers,
    pub(crate) key: Code,
    id: u32,
}

impl Accelerator {
    /// Creates a new accelerator to define keyboard shortcuts throughout your application.
    /// Only [`Modifiers::ALT`], [`Modifiers::SHIFT`], [`Modifiers::CONTROL`], and [`Modifiers::SUPER`]
    pub fn new(mods: Option<Modifiers>, key: Code) -> Self {
        let mut mods = mods.unwrap_or_else(Modifiers::empty);
        if mods.contains(Modifiers::META) {
            mods.remove(Modifiers::META);
            mods.insert(Modifiers::SUPER);
        }

        let id = Self::generate_hash(mods, key);

        Self { mods, key, id }
    }

    fn generate_hash(mods: Modifiers, key: Code) -> u32 {
        let mut accelerator_str = String::new();
        if mods.contains(Modifiers::SHIFT) {
            accelerator_str.push_str("shift+")
        }
        if mods.contains(Modifiers::CONTROL) {
            accelerator_str.push_str("control+")
        }
        if mods.contains(Modifiers::ALT) {
            accelerator_str.push_str("alt+")
        }
        if mods.contains(Modifiers::SUPER) {
            accelerator_str.push_str("super+")
        }
        accelerator_str.push_str(&key.to_string());

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        accelerator_str.hash(&mut hasher);
        std::hash::Hasher::finish(&hasher) as u32
    }

    /// Returns the id associated with this accelerator
    /// which is a hash of the string representation of modifiers and key within this accelerator.
    pub fn id(&self) -> u32 {
        self.id
    }

    /// Returns the modifier for this accelerator
    pub fn modifiers(&self) -> Modifiers {
        self.mods
    }

    /// Returns the code for this accelerator
    pub fn key(&self) -> Code {
        self.key
    }

    /// Returns `true` if this [`Code`] and [`Modifiers`] matches this `Accelerator`.
    pub fn matches(&self, modifiers: impl Borrow<Modifiers>, key: impl Borrow<Code>) -> bool {
        // Should be a const but const bit_or doesn't work here.
        let base_mods = Modifiers::SHIFT | Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER;
        let modifiers = modifiers.borrow();
        let key = key.borrow();
        self.mods == *modifiers & base_mods && self.key == *key
    }
}

impl FromStr for Accelerator {
    type Err = AcceleratorParseError;
    fn from_str(accelerator_string: &str) -> Result<Self, Self::Err> {
        parse_accelerator(accelerator_string)
    }
}

impl TryFrom<&str> for Accelerator {
    type Error = AcceleratorParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        parse_accelerator(value)
    }
}

impl TryFrom<String> for Accelerator {
    type Error = AcceleratorParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        parse_accelerator(&value)
    }
}

fn parse_accelerator(accelerator: &str) -> Result<Accelerator, AcceleratorParseError> {
    let tokens = accelerator.split('+').collect::<Vec<&str>>();

    let mut mods = Modifiers::empty();
    let mut key = None;

    match tokens.len() {
        // single key accelerator
        1 => {
            key = Some(parse_key(tokens[0])?);
        }

        // modifiers and key comobo accelerator
        _ => {
            for raw in tokens {
                let token = raw.trim();

                if token.is_empty() {
                    return Err(AcceleratorParseError::EmptyToken(accelerator.to_string()));
                }

                if key.is_some() {
                    // At this point we have parsed the modifiers and a main key, so by reaching
                    // this code, the function either received more than one main key or
                    //  the accelerator is not in the right order
                    // examples:
                    // 1. "Ctrl+Shift+C+A" => only one main key should be allowd.
                    // 2. "Ctrl+C+Shift" => wrong order
                    return Err(AcceleratorParseError::InvalidFormat(
                        accelerator.to_string(),
                    ));
                }

                match token.to_uppercase().as_str() {
                    "OPTION" | "ALT" => {
                        mods |= Modifiers::ALT;
                    }
                    "CONTROL" | "CTRL" => {
                        mods |= Modifiers::CONTROL;
                    }
                    "COMMAND" | "CMD" | "SUPER" => {
                        mods |= Modifiers::META;
                    }
                    "SHIFT" => {
                        mods |= Modifiers::SHIFT;
                    }
                    #[cfg(target_os = "macos")]
                    "COMMANDORCONTROL" | "COMMANDORCTRL" | "CMDORCTRL" | "CMDORCONTROL" => {
                        mods |= Modifiers::SUPER;
                    }
                    #[cfg(not(target_os = "macos"))]
                    "COMMANDORCONTROL" | "COMMANDORCTRL" | "CMDORCTRL" | "CMDORCONTROL" => {
                        mods |= Modifiers::CONTROL;
                    }
                    _ => {
                        key = Some(parse_key(token)?);
                    }
                }
            }
        }
    }

    let key = key.ok_or_else(|| AcceleratorParseError::InvalidFormat(accelerator.to_string()))?;
    Ok(Accelerator::new(Some(mods), key))
}

fn parse_key(key: &str) -> Result<Code, AcceleratorParseError> {
    use Code::*;
    match key.to_uppercase().as_str() {
        "BACKQUOTE" | "`" => Ok(Backquote),
        "BACKSLASH" | "\\" => Ok(Backslash),
        "BRACKETLEFT" | "[" => Ok(BracketLeft),
        "BRACKETRIGHT" | "]" => Ok(BracketRight),
        "COMMA" | "," => Ok(Comma),
        "DIGIT0" | "0" => Ok(Digit0),
        "DIGIT1" | "1" => Ok(Digit1),
        "DIGIT2" | "2" => Ok(Digit2),
        "DIGIT3" | "3" => Ok(Digit3),
        "DIGIT4" | "4" => Ok(Digit4),
        "DIGIT5" | "5" => Ok(Digit5),
        "DIGIT6" | "6" => Ok(Digit6),
        "DIGIT7" | "7" => Ok(Digit7),
        "DIGIT8" | "8" => Ok(Digit8),
        "DIGIT9" | "9" => Ok(Digit9),
        "EQUAL" | "=" => Ok(Equal),
        "KEYA" | "A" => Ok(KeyA),
        "KEYB" | "B" => Ok(KeyB),
        "KEYC" | "C" => Ok(KeyC),
        "KEYD" | "D" => Ok(KeyD),
        "KEYE" | "E" => Ok(KeyE),
        "KEYF" | "F" => Ok(KeyF),
        "KEYG" | "G" => Ok(KeyG),
        "KEYH" | "H" => Ok(KeyH),
        "KEYI" | "I" => Ok(KeyI),
        "KEYJ" | "J" => Ok(KeyJ),
        "KEYK" | "K" => Ok(KeyK),
        "KEYL" | "L" => Ok(KeyL),
        "KEYM" | "M" => Ok(KeyM),
        "KEYN" | "N" => Ok(KeyN),
        "KEYO" | "O" => Ok(KeyO),
        "KEYP" | "P" => Ok(KeyP),
        "KEYQ" | "Q" => Ok(KeyQ),
        "KEYR" | "R" => Ok(KeyR),
        "KEYS" | "S" => Ok(KeyS),
        "KEYT" | "T" => Ok(KeyT),
        "KEYU" | "U" => Ok(KeyU),
        "KEYV" | "V" => Ok(KeyV),
        "KEYW" | "W" => Ok(KeyW),
        "KEYX" | "X" => Ok(KeyX),
        "KEYY" | "Y" => Ok(KeyY),
        "KEYZ" | "Z" => Ok(KeyZ),
        "MINUS" | "-" => Ok(Minus),
        "PERIOD" | "." => Ok(Period),
        "QUOTE" | "'" => Ok(Quote),
        "SEMICOLON" | ";" => Ok(Semicolon),
        "SLASH" | "/" => Ok(Slash),
        "BACKSPACE" => Ok(Backspace),
        "CAPSLOCK" => Ok(CapsLock),
        "ENTER" => Ok(Enter),
        "SPACE" => Ok(Space),
        "TAB" => Ok(Tab),
        "DELETE" => Ok(Delete),
        "END" => Ok(End),
        "HOME" => Ok(Home),
        "INSERT" => Ok(Insert),
        "PAGEDOWN" => Ok(PageDown),
        "PAGEUP" => Ok(PageUp),
        "PRINTSCREEN" => Ok(PrintScreen),
        "SCROLLLOCK" => Ok(ScrollLock),
        "ARROWDOWN" | "DOWN" => Ok(ArrowDown),
        "ARROWLEFT" | "LEFT" => Ok(ArrowLeft),
        "ARROWRIGHT" | "RIGHT" => Ok(ArrowRight),
        "ARROWUP" | "UP" => Ok(ArrowUp),
        "NUMLOCK" => Ok(NumLock),
        "NUMPAD0" | "NUM0" => Ok(Numpad0),
        "NUMPAD1" | "NUM1" => Ok(Numpad1),
        "NUMPAD2" | "NUM2" => Ok(Numpad2),
        "NUMPAD3" | "NUM3" => Ok(Numpad3),
        "NUMPAD4" | "NUM4" => Ok(Numpad4),
        "NUMPAD5" | "NUM5" => Ok(Numpad5),
        "NUMPAD6" | "NUM6" => Ok(Numpad6),
        "NUMPAD7" | "NUM7" => Ok(Numpad7),
        "NUMPAD8" | "NUM8" => Ok(Numpad8),
        "NUMPAD9" | "NUM9" => Ok(Numpad9),
        "NUMPADADD" | "NUMADD" | "NUMPADPLUS" | "NUMPLUS" => Ok(NumpadAdd),
        "NUMPADDECIMAL" | "NUMDECIMAL" => Ok(NumpadDecimal),
        "NUMPADDIVIDE" | "NUMDIVIDE" => Ok(NumpadDivide),
        "NUMPADENTER" | "NUMENTER" => Ok(NumpadEnter),
        "NUMPADEQUAL" | "NUMEQUAL" => Ok(NumpadEqual),
        "NUMPADMULTIPLY" | "NUMMULTIPLY" => Ok(NumpadMultiply),
        "NUMPADSUBTRACT" | "NUMSUBTRACT" => Ok(NumpadSubtract),
        "ESCAPE" | "ESC" => Ok(Escape),
        "F1" => Ok(F1),
        "F2" => Ok(F2),
        "F3" => Ok(F3),
        "F4" => Ok(F4),
        "F5" => Ok(F5),
        "F6" => Ok(F6),
        "F7" => Ok(F7),
        "F8" => Ok(F8),
        "F9" => Ok(F9),
        "F10" => Ok(F10),
        "F11" => Ok(F11),
        "F12" => Ok(F12),
        "AUDIOVOLUMEDOWN" | "VOLUMEDOWN" => Ok(AudioVolumeDown),
        "AUDIOVOLUMEUP" | "VOLUMEUP" => Ok(AudioVolumeUp),
        "AUDIOVOLUMEMUTE" | "VOLUMEMUTE" => Ok(AudioVolumeMute),
        "F13" => Ok(F13),
        "F14" => Ok(F14),
        "F15" => Ok(F15),
        "F16" => Ok(F16),
        "F17" => Ok(F17),
        "F18" => Ok(F18),
        "F19" => Ok(F19),
        "F20" => Ok(F20),
        "F21" => Ok(F21),
        "F22" => Ok(F22),
        "F23" => Ok(F23),
        "F24" => Ok(F24),

        _ => Err(AcceleratorParseError::UnsupportedKey(key.to_string())),
    }
}

#[test]
fn test_parse_accelerator() {
    macro_rules! assert_parse_accelerator {
        ($key:literal, $lrh:expr) => {
            let r = parse_accelerator($key).unwrap();
            let l = $lrh;
            assert_eq!(r.mods, l.mods);
            assert_eq!(r.key, l.key);
        };
    }

    assert_parse_accelerator!(
        "KeyX",
        Accelerator {
            mods: Modifiers::empty(),
            key: Code::KeyX,
            id: 0,
        }
    );

    assert_parse_accelerator!(
        "CTRL+KeyX",
        Accelerator {
            mods: Modifiers::CONTROL,
            key: Code::KeyX,
            id: 0,
        }
    );

    assert_parse_accelerator!(
        "SHIFT+KeyC",
        Accelerator {
            mods: Modifiers::SHIFT,
            key: Code::KeyC,
            id: 0,
        }
    );

    assert_parse_accelerator!(
        "SHIFT+KeyC",
        Accelerator {
            mods: Modifiers::SHIFT,
            key: Code::KeyC,
            id: 0,
        }
    );

    assert_parse_accelerator!(
        "super+ctrl+SHIFT+alt+ArrowUp",
        Accelerator {
            mods: Modifiers::SUPER | Modifiers::CONTROL | Modifiers::SHIFT | Modifiers::ALT,
            key: Code::ArrowUp,
            id: 0,
        }
    );
    assert_parse_accelerator!(
        "Digit5",
        Accelerator {
            mods: Modifiers::empty(),
            key: Code::Digit5,
            id: 0,
        }
    );
    assert_parse_accelerator!(
        "KeyG",
        Accelerator {
            mods: Modifiers::empty(),
            key: Code::KeyG,
            id: 0,
        }
    );

    assert_parse_accelerator!(
        "SHiFT+F12",
        Accelerator {
            mods: Modifiers::SHIFT,
            key: Code::F12,
            id: 0,
        }
    );

    assert_parse_accelerator!(
        "CmdOrCtrl+Space",
        Accelerator {
            #[cfg(target_os = "macos")]
            mods: Modifiers::SUPER,
            #[cfg(not(target_os = "macos"))]
            mods: Modifiers::CONTROL,
            key: Code::Space,
            id: 0,
        }
    );
}

#[test]
fn test_equality() {
    let h1 = parse_accelerator("Shift+KeyR").unwrap();
    let h2 = parse_accelerator("Shift+KeyR").unwrap();
    let h3 = Accelerator::new(Some(Modifiers::SHIFT), Code::KeyR);
    let h4 = parse_accelerator("Alt+KeyR").unwrap();
    let h5 = parse_accelerator("Alt+KeyR").unwrap();
    let h6 = parse_accelerator("KeyR").unwrap();

    assert!(h1 == h2 && h2 == h3 && h3 != h4 && h4 == h5 && h5 != h6);
    assert!(
        h1.id() == h2.id()
            && h2.id() == h3.id()
            && h3.id() != h4.id()
            && h4.id() == h5.id()
            && h5.id() != h6.id()
    );
}
