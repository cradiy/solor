use super::*;
use rustix::fs::{MemfdFlags, memfd_create};
use std::io::Write;

#[test]
fn shared_keymap_resolves_the_declared_symbol_and_shift_level() {
    let map = br#"xkb_keymap {
        xkb_keycodes "codes" { minimum = 8; maximum = 255; <AC01> = 38; };
        xkb_types "types" {
            type "TWO_LEVEL" {
                modifiers = Shift;
                map[None] = Level1;
                map[Shift] = Level2;
            };
        };
        xkb_compatibility "compat" {};
        xkb_symbols "symbols" { key <AC01> { type="TWO_LEVEL", [ Greek_alpha, Greek_ALPHA ] }; };
    };"#;
    let mut file = File::from(memfd_create("keymap", MemfdFlags::CLOEXEC).unwrap());
    file.write_all(map).unwrap();
    file.write_all(&[0]).unwrap();
    let size = map.len() as u32 + 1;
    assert!(read_layout(file.try_clone().unwrap().into(), size - 1).is_none());
    let mut layout = read_layout(file.into(), size).unwrap();
    assert_eq!(key_for_code(&layout, 30), Some(Key::Character("α".into())));
    let shift = layout.get_keymap().mod_get_index(xkb::MOD_NAME_SHIFT);
    layout.update_mask(1 << shift, 0, 0, 0, 0, 0);
    assert_eq!(key_for_code(&layout, 30), Some(Key::Character("Α".into())));
    for (sym, expected) in [
        (Keysym::ISO_Left_Tab, NamedKey::Tab),
        (Keysym::KP_Left, NamedKey::ArrowLeft),
        (Keysym::Return, NamedKey::Enter),
        (Keysym::dead_acute, NamedKey::Dead),
        (Keysym::F12, NamedKey::F12),
    ] {
        assert_eq!(logical_key(sym), Key::Named(expected));
    }
}
