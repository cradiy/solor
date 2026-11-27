use super::*;
use objc2_app_kit::{
    NSEventModifierFlags, NSF1FunctionKey, NSF12FunctionKey, NSLeftArrowFunctionKey,
    NSUpArrowFunctionKey,
};
use objc2_foundation::NSPoint;
use solor::{
    keyboard::{Key, KeyState, Modifiers, NamedKey},
    pointer::PointerButton,
    px,
};

#[test]
fn coordinate_and_scroll_conversions_match_solor_top_left_axes() {
    let origin = view_point_to_solor(NSPoint::new(25.5, 100.0), 400.0);
    assert_eq!(origin.x, px(25.5));
    assert_eq!(origin.y, px(300.0));

    let precise = scroll_delta_from_raw(3.0, -12.5, true);
    assert_eq!(precise.x, px(-3.0));
    assert_eq!(precise.y, px(12.5));

    let wheel = scroll_delta_from_raw(0.0, -1.0, false);
    assert_eq!(wheel.x, px(0.0));
    assert_eq!(wheel.y, px(20.0));
}

#[test]
fn pointer_buttons_and_modifiers_map_cleanly() {
    assert_eq!(pointer_button_from_number(0), Some(PointerButton::Primary));
    assert_eq!(
        pointer_button_from_number(1),
        Some(PointerButton::Secondary)
    );
    assert_eq!(pointer_button_from_number(2), Some(PointerButton::Middle));
    assert_eq!(pointer_button_from_number(4), Some(PointerButton::Other(4)));
    assert_eq!(pointer_button_from_number(-1), None);

    let mods = modifiers_from_flags(
        NSEventModifierFlags::Shift
            | NSEventModifierFlags::Control
            | NSEventModifierFlags::Option
            | NSEventModifierFlags::Command,
    );
    assert_eq!(
        mods,
        Modifiers {
            shift: true,
            control: true,
            alt: true,
            meta: true,
        }
    );
}

#[test]
fn key_resolution_handles_named_shifted_and_shortcut_keys() {
    let up = char::from_u32(NSUpArrowFunctionKey).unwrap().to_string();
    let left = char::from_u32(NSLeftArrowFunctionKey).unwrap().to_string();
    let f1 = char::from_u32(NSF1FunctionKey).unwrap().to_string();
    let f12 = char::from_u32(NSF12FunctionKey).unwrap().to_string();

    assert_eq!(
        resolve_key(&up, &up, Modifiers::default()),
        Some(Key::Named(NamedKey::ArrowUp))
    );
    assert_eq!(
        resolve_key(&left, &left, Modifiers::default()),
        Some(Key::Named(NamedKey::ArrowLeft))
    );
    assert_eq!(
        resolve_key(&f1, &f1, Modifiers::default()),
        Some(Key::Named(NamedKey::F1))
    );
    assert_eq!(
        resolve_key(&f12, &f12, Modifiers::default()),
        Some(Key::Named(NamedKey::F12))
    );
    assert_eq!(
        resolve_key("\r", "\r", Modifiers::default()),
        Some(Key::Named(NamedKey::Enter))
    );
    assert_eq!(
        resolve_key(
            "\x19",
            "\x19",
            Modifiers {
                shift: true,
                ..Default::default()
            }
        ),
        Some(Key::Named(NamedKey::Tab))
    );
    assert_eq!(
        resolve_key("\x7f", "\x7f", Modifiers::default()),
        Some(Key::Named(NamedKey::Backspace))
    );
    assert_eq!(
        resolve_key("\x1b", "\x1b", Modifiers::default()),
        Some(Key::Named(NamedKey::Escape))
    );

    // Shift+1 yields '!' when neither Ctrl nor Cmd is held; Ctrl+A keeps 'a'.
    assert_eq!(
        resolve_key(
            "!",
            "1",
            Modifiers {
                shift: true,
                ..Default::default()
            }
        ),
        Some(Key::Character('!'))
    );
    assert_eq!(
        resolve_key(
            "\x01",
            "a",
            Modifiers {
                control: true,
                ..Default::default()
            }
        ),
        Some(Key::Character('a'))
    );
    assert_eq!(
        resolve_key(
            "c",
            "c",
            Modifiers {
                meta: true,
                ..Default::default()
            }
        ),
        Some(Key::Character('c'))
    );
}

#[test]
fn flags_changed_emits_modifier_key_transitions() {
    let down =
        modifier_key_transition(NSEventModifierFlags::empty(), NSEventModifierFlags::Command)
            .expect("command press");
    assert_eq!(down.key, Key::Named(NamedKey::Meta));
    assert_eq!(down.state, KeyState::Down);
    assert!(down.modifiers.meta);

    let up = modifier_key_transition(NSEventModifierFlags::Command, NSEventModifierFlags::empty())
        .expect("command release");
    assert_eq!(up.key, Key::Named(NamedKey::Meta));
    assert_eq!(up.state, KeyState::Up);
    assert!(!up.modifiers.meta);
}
