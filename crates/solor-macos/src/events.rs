//! Translation between AppKit `NSEvent` inputs and Solor pointer/keyboard types.
use objc2_app_kit::{
    NSDeleteFunctionKey, NSDownArrowFunctionKey, NSEndFunctionKey, NSEvent, NSEventModifierFlags,
    NSF1FunctionKey, NSF2FunctionKey, NSF3FunctionKey, NSF4FunctionKey, NSF5FunctionKey,
    NSF6FunctionKey, NSF7FunctionKey, NSF8FunctionKey, NSF9FunctionKey, NSF10FunctionKey,
    NSF11FunctionKey, NSF12FunctionKey, NSF13FunctionKey, NSF14FunctionKey, NSF15FunctionKey,
    NSF16FunctionKey, NSF17FunctionKey, NSF18FunctionKey, NSF19FunctionKey, NSF20FunctionKey,
    NSF21FunctionKey, NSF22FunctionKey, NSF23FunctionKey, NSF24FunctionKey, NSHelpFunctionKey,
    NSHomeFunctionKey, NSLeftArrowFunctionKey, NSPageDownFunctionKey, NSPageUpFunctionKey,
    NSRightArrowFunctionKey, NSUpArrowFunctionKey,
};
use objc2_foundation::NSPoint;
use solor::{
    keyboard::{Key, KeyEvent, KeyState, Modifiers, NamedKey},
    layout::Origin,
    pointer::{PointerButton, PointerId, PointerKind, PointerSample, ScrollDelta},
    px,
};

const BACKSPACE_KEY: u32 = 0x7f;
const SPACE_KEY: u32 = b' ' as u32;
const ENTER_KEY: u32 = 0x0d;
const NUMPAD_ENTER_KEY: u32 = 0x03;
const ESCAPE_KEY: u32 = 0x1b;
const TAB_KEY: u32 = 0x09;
const SHIFT_TAB_KEY: u32 = 0x19;
const LINE_SCROLL_PIXELS: f64 = 20.0;

/// Primary mouse pointer identifier used for AppKit mouse/trackpad events.
pub const MOUSE_POINTER_ID: PointerId = PointerId(0);

/// Converts a bottom-left AppKit view point to Solor top-left logical coordinates.
pub fn view_point_to_solor(point: NSPoint, view_height: f64) -> Origin {
    Origin {
        x: px(point.x as f32),
        y: px((view_height - point.y) as f32),
    }
}

/// Extracts Solor modifier flags from `NSEventModifierFlags`.
pub fn modifiers_from_flags(flags: NSEventModifierFlags) -> Modifiers {
    Modifiers {
        shift: flags.contains(NSEventModifierFlags::Shift),
        control: flags.contains(NSEventModifierFlags::Control),
        alt: flags.contains(NSEventModifierFlags::Option),
        meta: flags.contains(NSEventModifierFlags::Command),
    }
}

/// Converts raw AppKit scroll wheel deltas to Solor's positive-right/positive-down convention.
pub fn scroll_delta_from_raw(raw_x: f64, raw_y: f64, precise: bool) -> ScrollDelta {
    let scale = if precise { 1.0 } else { LINE_SCROLL_PIXELS };
    ScrollDelta {
        x: px((-raw_x * scale) as f32),
        y: px((-raw_y * scale) as f32),
    }
}

/// Maps an AppKit mouse button number to `PointerButton`.
pub fn pointer_button_from_number(button_number: isize) -> Option<PointerButton> {
    match button_number {
        0 => Some(PointerButton::Primary),
        1 => Some(PointerButton::Secondary),
        2 => Some(PointerButton::Middle),
        n if n > 2 => u16::try_from(n).ok().map(PointerButton::Other),
        _ => None,
    }
}

/// Builds a `PointerSample` from an `NSEvent` and a view-local `NSPoint`.
pub fn pointer_sample_from_event_in_view(
    event: &NSEvent,
    local_point: NSPoint,
    view_height: f64,
) -> PointerSample {
    PointerSample {
        id: MOUSE_POINTER_ID,
        kind: PointerKind::Mouse,
        position: view_point_to_solor(local_point, view_height),
        modifiers: modifiers_from_flags(event.modifierFlags()),
    }
}

/// Converts a key-down or key-up `NSEvent` into a `KeyEvent`.
pub fn key_event_from_ns_event(event: &NSEvent, state: KeyState) -> Option<KeyEvent> {
    let modifiers = modifiers_from_flags(event.modifierFlags());
    let chars = event
        .characters()
        .map(|s| s.to_string())
        .unwrap_or_default();
    let chars_unmodified = event
        .charactersIgnoringModifiers()
        .map(|s| s.to_string())
        .unwrap_or_default();
    let key = resolve_key(&chars, &chars_unmodified, modifiers)?;
    let repeat = state == KeyState::Down && event.isARepeat();
    Some(KeyEvent {
        key,
        state,
        repeat,
        modifiers,
    })
}

/// Resolves a Solor `Key` from AppKit `characters` and `charactersIgnoringModifiers`.
pub fn resolve_key(chars: &str, chars_unmodified: &str, modifiers: Modifiers) -> Option<Key> {
    let unmodified_first = chars_unmodified.chars().next().map(|ch| ch as u32);
    if let Some(named) = unmodified_first.and_then(named_key_from_code) {
        return Some(Key::Named(named));
    }
    let char_first = chars.chars().next().map(|ch| ch as u32);
    if let Some(named) = char_first.and_then(named_key_from_code) {
        return Some(Key::Named(named));
    }

    let source = if modifiers.control || modifiers.meta {
        chars_unmodified
    } else {
        let has_printable = !chars.is_empty() && !chars.chars().any(char::is_control);
        if has_printable {
            chars
        } else {
            chars_unmodified
        }
    };

    let mut iter = source.chars().filter(|ch| !ch.is_control());
    let first = iter.next()?;
    if iter.next().is_none() {
        Some(Key::Character(first))
    } else {
        Some(Key::Text(source.into()))
    }
}

/// Computes a `KeyEvent` from a change in `NSEventModifierFlags` (`flagsChanged:`).
pub fn modifier_key_transition(
    previous: NSEventModifierFlags,
    current: NSEventModifierFlags,
) -> Option<KeyEvent> {
    const CHECKS: [(NSEventModifierFlags, NamedKey); 5] = [
        (NSEventModifierFlags::Shift, NamedKey::Shift),
        (NSEventModifierFlags::Control, NamedKey::Control),
        (NSEventModifierFlags::Option, NamedKey::Alt),
        (NSEventModifierFlags::Command, NamedKey::Meta),
        (NSEventModifierFlags::CapsLock, NamedKey::CapsLock),
    ];
    let modifiers = modifiers_from_flags(current);
    for (flag, named) in CHECKS {
        let was_down = previous.contains(flag);
        let is_down = current.contains(flag);
        if was_down != is_down {
            return Some(KeyEvent {
                key: Key::Named(named),
                state: if is_down {
                    KeyState::Down
                } else {
                    KeyState::Up
                },
                repeat: false,
                modifiers,
            });
        }
    }
    None
}

fn named_key_from_code(code: u32) -> Option<NamedKey> {
    #[allow(non_upper_case_globals)]
    Some(match code {
        SPACE_KEY => return None,
        TAB_KEY | SHIFT_TAB_KEY => NamedKey::Tab,
        ENTER_KEY | NUMPAD_ENTER_KEY => NamedKey::Enter,
        BACKSPACE_KEY => NamedKey::Backspace,
        ESCAPE_KEY => NamedKey::Escape,
        NSUpArrowFunctionKey => NamedKey::ArrowUp,
        NSDownArrowFunctionKey => NamedKey::ArrowDown,
        NSLeftArrowFunctionKey => NamedKey::ArrowLeft,
        NSRightArrowFunctionKey => NamedKey::ArrowRight,
        NSPageUpFunctionKey => NamedKey::PageUp,
        NSPageDownFunctionKey => NamedKey::PageDown,
        NSHomeFunctionKey => NamedKey::Home,
        NSEndFunctionKey => NamedKey::End,
        NSDeleteFunctionKey => NamedKey::Delete,
        NSHelpFunctionKey => NamedKey::Insert,
        NSF1FunctionKey => NamedKey::F1,
        NSF2FunctionKey => NamedKey::F2,
        NSF3FunctionKey => NamedKey::F3,
        NSF4FunctionKey => NamedKey::F4,
        NSF5FunctionKey => NamedKey::F5,
        NSF6FunctionKey => NamedKey::F6,
        NSF7FunctionKey => NamedKey::F7,
        NSF8FunctionKey => NamedKey::F8,
        NSF9FunctionKey => NamedKey::F9,
        NSF10FunctionKey => NamedKey::F10,
        NSF11FunctionKey => NamedKey::F11,
        NSF12FunctionKey => NamedKey::F12,
        NSF13FunctionKey => NamedKey::F13,
        NSF14FunctionKey => NamedKey::F14,
        NSF15FunctionKey => NamedKey::F15,
        NSF16FunctionKey => NamedKey::F16,
        NSF17FunctionKey => NamedKey::F17,
        NSF18FunctionKey => NamedKey::F18,
        NSF19FunctionKey => NamedKey::F19,
        NSF20FunctionKey => NamedKey::F20,
        NSF21FunctionKey => NamedKey::F21,
        NSF22FunctionKey => NamedKey::F22,
        NSF23FunctionKey => NamedKey::F23,
        NSF24FunctionKey => NamedKey::F24,
        _ => return None,
    })
}

#[cfg(test)]
#[path = "../tests/unit/events.rs"]
mod tests;
