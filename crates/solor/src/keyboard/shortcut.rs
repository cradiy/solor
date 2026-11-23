use super::{KeyEvent, Modifiers, NamedKey};
use crate::SharedString;
use std::fmt;

/// A logical key resolved by the active keyboard layout.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// A key identified by function.
    Named(NamedKey),
    /// A single character produced by the layout.
    Character(char),
    /// A logical key whose layout value consists of multiple Unicode scalars.
    Text(SharedString),
}

macro_rules! keys {
    ($($name:ident => $value:literal),*; $($named:ident),*) => {
        #[allow(non_upper_case_globals)]
        impl Key {
            $(#[doc = concat!("The ", stringify!($name), " key.")]
            pub const $name: Self = Self::Character($value);)*
            $(#[doc = concat!("The ", stringify!($named), " key.")]
            pub const $named: Self = Self::Named(NamedKey::$named);)*
        }
    }
}

keys! {
    A => 'a', B => 'b', C => 'c', D => 'd', E => 'e', F => 'f', G => 'g', H => 'h',
    I => 'i', J => 'j', K => 'k', L => 'l', M => 'm', N => 'n', O => 'o', P => 'p',
    Q => 'q', R => 'r', S => 's', T => 't', U => 'u', V => 'v', W => 'w', X => 'x',
    Y => 'y', Z => 'z', Digit0 => '0', Digit1 => '1', Digit2 => '2', Digit3 => '3',
    Digit4 => '4', Digit5 => '5', Digit6 => '6', Digit7 => '7', Digit8 => '8', Digit9 => '9',
    Space => ' ', Minus => '-', Equal => '=', Comma => ',', Period => '.', Slash => '/',
    Backslash => '\\', Semicolon => ';', Quote => '\'', Backtick => '`', BracketLeft => '[',
    BracketRight => ']';
    Enter, Escape, Tab, ArrowUp, ArrowDown, ArrowLeft, ArrowRight, Backspace, Delete, Insert,
    Home, End, PageUp, PageDown, F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
    F13, F14, F15, F16, F17, F18, F19, F20, F21, F22, F23, F24
}

impl From<NamedKey> for Key {
    fn from(key: NamedKey) -> Self {
        Self::Named(key)
    }
}

impl From<char> for Key {
    fn from(ch: char) -> Self {
        Self::Character(ch)
    }
}

impl From<&str> for Key {
    fn from(value: &str) -> Self {
        let mut chars = value.chars();
        match (chars.next(), chars.next()) {
            (Some(ch), None) => Self::Character(ch),
            (None, _) => Self::Named(NamedKey::Unidentified),
            _ => Self::Text(value.into()),
        }
    }
}

impl Key {
    /// Combines this key with a required modifier.
    pub fn with(self, modifier: Modifier) -> Shortcut {
        Shortcut::from(self).with(modifier)
    }

    fn shortcut_key(&self) -> Self {
        match self {
            Self::Character(ch) => Self::Character(ch.to_ascii_lowercase()),
            other => other.clone(),
        }
    }
}

/// A modifier required by a shortcut.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Modifier {
    /// Control.
    Ctrl,
    /// Alt or Option.
    Alt,
    /// Shift.
    Shift,
    /// Command, Super or the Windows key.
    Meta,
}

/// A logical key and the exact set of modifiers required to match it.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Shortcut {
    key: Key,
    modifiers: Modifiers,
}

impl fmt::Display for Shortcut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (active, prefix) in [
            (self.modifiers.control, "Ctrl+"),
            (self.modifiers.alt, "Alt+"),
            (self.modifiers.shift, "Shift+"),
            (self.modifiers.meta, "Meta+"),
        ] {
            if active {
                f.write_str(prefix)?;
            }
        }
        match &self.key {
            Key::Named(key) => key.fmt(f),
            Key::Character(' ') => f.write_str("Space"),
            Key::Character(key) => key.to_ascii_uppercase().fmt(f),
            Key::Text(text) => f.write_str(text),
        }
    }
}

impl From<Key> for Shortcut {
    fn from(key: Key) -> Self {
        Self {
            key: key.shortcut_key(),
            modifiers: Modifiers::default(),
        }
    }
}

impl Shortcut {
    /// Requires an additional modifier.
    pub fn with(mut self, modifier: Modifier) -> Self {
        match modifier {
            Modifier::Ctrl => self.modifiers.control = true,
            Modifier::Alt => self.modifiers.alt = true,
            Modifier::Shift => self.modifiers.shift = true,
            Modifier::Meta => self.modifiers.meta = true,
        }
        self
    }

    pub(super) fn from_event(event: &KeyEvent) -> Self {
        Self {
            key: event.key.shortcut_key(),
            modifiers: event.modifiers,
        }
    }
}
