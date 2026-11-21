use std::{
    fs::File,
    os::{fd::OwnedFd, unix::fs::FileExt},
};

use solor::{
    keyboard::{Key, KeyEvent, KeyState, Modifiers, NamedKey},
    platform::WindowEvent,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum, protocol::wl_keyboard};
use xkbcommon::xkb::{self, Keysym};

use super::Window;

pub(super) struct Keyboard {
    device: wl_keyboard::WlKeyboard,
    layout: Option<xkb::State>,
    entered: bool,
    pub(super) modifiers: Modifiers,
}

impl Keyboard {
    pub(super) fn new(device: wl_keyboard::WlKeyboard) -> Self {
        Self {
            device,
            layout: None,
            entered: false,
            modifiers: Modifiers::default(),
        }
    }
}

impl Drop for Keyboard {
    fn drop(&mut self) {
        if self.device.version() >= 3 {
            self.device.release();
        }
    }
}

fn read_layout(fd: OwnedFd, size: u32) -> Option<xkb::State> {
    // The advertised range starts at offset zero and may include a trailing NUL.
    // Bound the allocation even for a malformed size.
    if !(1..=16 * 1024 * 1024).contains(&size) {
        return None;
    }
    let mut bytes = vec![0; size as usize];
    File::from(fd).read_exact_at(&mut bytes, 0).ok()?;
    if bytes.last() == Some(&0) {
        bytes.pop();
    }
    let keymap = xkb::Keymap::new_from_string(
        &xkb::Context::new(xkb::CONTEXT_NO_FLAGS),
        String::from_utf8(bytes).ok()?,
        xkb::KEYMAP_FORMAT_TEXT_V1,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )?;
    Some(xkb::State::new(&keymap))
}

fn key_for_code(layout: &xkb::State, evdev_code: u32) -> Option<Key> {
    // The Wayland XKB v1 keymap numbers keys eight above their evdev codes.
    let code = xkb::Keycode::new(evdev_code.checked_add(8)?);
    Some(logical_key(layout.key_get_one_sym(code)))
}

fn logical_key(sym: Keysym) -> Key {
    if let Some(ch) = char::from_u32(xkb::keysym_to_utf32(sym)).filter(|ch| !ch.is_control()) {
        return Key::Character(ch.to_string());
    }
    let named = match sym {
        Keysym::Return | Keysym::KP_Enter | Keysym::ISO_Enter => NamedKey::Enter,
        Keysym::Tab | Keysym::KP_Tab | Keysym::ISO_Left_Tab => NamedKey::Tab,
        Keysym::BackSpace => NamedKey::Backspace,
        Keysym::Left | Keysym::KP_Left => NamedKey::ArrowLeft,
        Keysym::Right | Keysym::KP_Right => NamedKey::ArrowRight,
        Keysym::Up | Keysym::KP_Up => NamedKey::ArrowUp,
        Keysym::Down | Keysym::KP_Down => NamedKey::ArrowDown,
        Keysym::Prior | Keysym::KP_Prior => NamedKey::PageUp,
        Keysym::Next | Keysym::KP_Next => NamedKey::PageDown,
        Keysym::KP_Home => NamedKey::Home,
        Keysym::KP_End => NamedKey::End,
        Keysym::KP_Insert => NamedKey::Insert,
        Keysym::KP_Delete => NamedKey::Delete,
        Keysym::Shift_L | Keysym::Shift_R => NamedKey::Shift,
        Keysym::Control_L | Keysym::Control_R => NamedKey::Control,
        Keysym::Alt_L | Keysym::Alt_R => NamedKey::Alt,
        Keysym::Super_L | Keysym::Super_R | Keysym::Meta_L | Keysym::Meta_R => NamedKey::Meta,
        Keysym::ISO_Level3_Shift => NamedKey::AltGraph,
        Keysym::Caps_Lock => NamedKey::CapsLock,
        Keysym::Num_Lock => NamedKey::NumLock,
        _ => {
            let name = xkb::keysym_get_name(sym);
            if name.starts_with("dead_") {
                NamedKey::Dead
            } else {
                name.parse().unwrap_or(NamedKey::Unidentified)
            }
        }
    };
    Key::Named(named)
}

impl Dispatch<wl_keyboard::WlKeyboard, u32> for Window {
    fn event(
        window: &mut Self,
        device: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(keyboard) = window
            .seats
            .iter_mut()
            .filter_map(|seat| seat.keyboard.as_mut())
            .find(|keyboard| &keyboard.device == device)
        else {
            return;
        };
        match event {
            wl_keyboard::Event::Keymap { format, fd, size } => {
                keyboard.layout = match format {
                    WEnum::Value(wl_keyboard::KeymapFormat::XkbV1) => read_layout(fd, size),
                    _ => None,
                };
                keyboard.modifiers = Modifiers::default();
            }
            wl_keyboard::Event::Enter { surface, .. } => {
                keyboard.entered = surface == window.surface
            }
            wl_keyboard::Event::Leave { .. } => {
                keyboard.entered = false;
                keyboard.modifiers = Modifiers::default();
                if let Some(layout) = &mut keyboard.layout {
                    layout.update_mask(0, 0, 0, 0, 0, 0);
                }
            }
            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                let Some(layout) = &mut keyboard.layout else {
                    return;
                };
                layout.update_mask(mods_depressed, mods_latched, mods_locked, 0, 0, group);
                let active = |name| layout.mod_name_is_active(name, xkb::STATE_MODS_EFFECTIVE);
                keyboard.modifiers = Modifiers {
                    shift: active(xkb::MOD_NAME_SHIFT),
                    control: active(xkb::MOD_NAME_CTRL),
                    alt: active(xkb::MOD_NAME_ALT),
                    meta: active(xkb::MOD_NAME_LOGO),
                };
            }
            wl_keyboard::Event::Key {
                key,
                state: WEnum::Value(state),
                ..
            } if keyboard.entered => {
                let state = match state {
                    wl_keyboard::KeyState::Pressed => KeyState::Down,
                    wl_keyboard::KeyState::Released => KeyState::Up,
                    _ => return,
                };
                let Some(layout) = &keyboard.layout else {
                    return;
                };
                let Some(key) = key_for_code(layout, key) else {
                    return;
                };
                let event = KeyEvent {
                    state,
                    key,
                    modifiers: keyboard.modifiers,
                };
                window.event(WindowEvent::Key(event));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/keyboard.rs"]
mod tests;
