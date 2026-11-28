use std::{
    fs::File,
    os::{fd::OwnedFd, unix::fs::FileExt},
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, ensure};
use solor::{
    keyboard::{Key, KeyEvent, KeyState, Modifiers, NamedKey},
    platform::{PlatformWindow, WindowEvent},
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum, protocol::wl_keyboard};
use xkbcommon::xkb::{self, Keysym};

use super::Desktop;
use wayland_client::backend::ObjectId;

pub(super) struct Keyboard {
    device: wl_keyboard::WlKeyboard,
    layout: Option<xkb::State>,
    pub(super) focus: Option<ObjectId>,
    repeat: Repeat,
    pub(super) modifiers: Modifiers,
}

impl Keyboard {
    pub(super) fn new(device: wl_keyboard::WlKeyboard) -> Self {
        Self {
            device,
            layout: None,
            focus: None,
            repeat: Repeat::default(),
            modifiers: Modifiers::default(),
        }
    }
    pub(super) fn leave(&mut self, id: &ObjectId) {
        if self.focus.as_ref() == Some(id) {
            self.focus = None;
            self.repeat.held = None;
            self.modifiers = Modifiers::default();
            if let Some(layout) = &mut self.layout {
                layout.update_mask(0, 0, 0, 0, 0, 0);
            }
        }
    }
}

struct Repeat {
    delay: Duration,
    interval: Option<Duration>,
    held: Option<(u32, Instant)>,
}

impl Default for Repeat {
    fn default() -> Self {
        // Seats older than version 4 do not advertise repeat settings.
        Self {
            delay: Duration::from_millis(500),
            interval: Some(Duration::from_millis(40)),
            held: None,
        }
    }
}

impl Repeat {
    fn configure(&mut self, rate: i32, delay: i32, now: Instant) {
        self.interval =
            (rate > 0).then(|| Duration::from_nanos((1_000_000_000 / rate as u64).max(1)));
        self.delay = Duration::from_millis(delay.max(0) as u64);
        if let Some((code, _)) = self.held {
            self.press(code, now);
        }
    }

    fn press(&mut self, code: u32, now: Instant) {
        self.held = self.interval.map(|_| (code, now + self.delay));
    }

    fn release(&mut self, code: u32) {
        if self.held.is_some_and(|(held, _)| held == code) {
            self.held = None;
        }
    }

    fn due(&mut self, now: Instant) -> Option<u32> {
        let (code, deadline) = self.held.as_mut()?;
        if now < *deadline {
            return None;
        }
        let interval = self.interval?;
        let next = *deadline + interval;
        *deadline = if next > now { next } else { now + interval };
        Some(*code)
    }
}

pub(super) fn repeat_keys(
    seats: &mut [super::pointer::Seat],
    surface: &ObjectId,
    window: &mut super::Window,
    now: Instant,
) -> Option<Instant> {
    let mut next = None;
    for keyboard in seats.iter_mut().filter_map(|s| s.keyboard.as_mut()) {
        if keyboard.repeat.held.is_none() || keyboard.focus.as_ref() != Some(surface) {
            continue;
        }
        if window.mapping != super::Mapping::Mapped
            || window.life.blocked()
            || !window.life.is_open()
        {
            keyboard.repeat.held = None;
            continue;
        }
        if let Some(code) = keyboard.repeat.due(now)
            && let Some(key) = keyboard
                .layout
                .as_ref()
                .and_then(|layout| key_for_code(layout, code))
        {
            window.event(WindowEvent::Key(KeyEvent {
                key,
                state: KeyState::Down,
                repeat: true,
                modifiers: keyboard.modifiers,
            }));
        }
        if let Some((_, deadline)) = keyboard.repeat.held {
            next = Some(next.map_or(deadline, |old: Instant| old.min(deadline)));
        }
    }
    next
}

impl Drop for Keyboard {
    fn drop(&mut self) {
        if self.device.version() >= 3 {
            self.device.release();
        }
    }
}

fn read_layout(fd: OwnedFd, size: u32) -> Result<xkb::State> {
    // The advertised range starts at offset zero and may include a trailing NUL.
    // Bound the allocation even for a malformed size.
    ensure!(
        (1..=16 * 1024 * 1024).contains(&size),
        "invalid keymap size ({size} bytes)"
    );
    let mut bytes = vec![0; size as usize];
    File::from(fd).read_exact_at(&mut bytes, 0)?;
    if bytes.last() == Some(&0) {
        bytes.pop();
    }
    let keymap = xkb::Keymap::new_from_string(
        &xkb::Context::new(xkb::CONTEXT_NO_FLAGS),
        String::from_utf8(bytes)?,
        xkb::KEYMAP_FORMAT_TEXT_V1,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .context("XKB could not compile the keymap")?;
    Ok(xkb::State::new(&keymap))
}

fn key_for_code(layout: &xkb::State, evdev_code: u32) -> Option<Key> {
    // The Wayland XKB v1 keymap numbers keys eight above their evdev codes.
    let code = xkb::Keycode::new(evdev_code.checked_add(8)?);
    Some(logical_key(layout.key_get_one_sym(code)))
}

fn logical_key(sym: Keysym) -> Key {
    if let Some(ch) = char::from_u32(xkb::keysym_to_utf32(sym)).filter(|ch| !ch.is_control()) {
        return Key::from(ch);
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

impl Desktop {
    pub(super) fn keyboard_focus_changed(&mut self, id: &ObjectId, entered: bool) {
        let seats = self
            .seats
            .iter()
            .filter_map(|s| s.keyboard.as_ref())
            .filter(|k| k.focus.as_ref() == Some(id))
            .count();
        if seats == usize::from(entered)
            && let Some(window) = self.windows.get_mut(id)
        {
            window.event(WindowEvent::KeyboardFocus(entered));
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, u32> for Desktop {
    fn event(
        desktop: &mut Self,
        device: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some((seat, keyboard)) = desktop
            .seats
            .iter_mut()
            .filter_map(|seat| Some((&seat.seat, seat.keyboard.as_mut()?)))
            .find(|(_, keyboard)| &keyboard.device == device)
        else {
            return;
        };
        match event {
            wl_keyboard::Event::Keymap { format, fd, size } => {
                keyboard.repeat.held = None;
                keyboard.layout = match format {
                    WEnum::Value(wl_keyboard::KeymapFormat::XkbV1) => read_layout(fd, size)
                        .inspect_err(|err| {
                            log::warn!("failed to load Wayland keyboard keymap: {err:#}")
                        })
                        .ok(),
                    _ => None,
                };
                keyboard.modifiers = Modifiers::default();
            }
            wl_keyboard::Event::Enter { surface, .. } => {
                keyboard.repeat.held = None;
                let id = surface.id();
                let previous = keyboard.focus.replace(id.clone());
                if previous.as_ref() == Some(&id) {
                    return;
                }
                if let Some(c) = &desktop.clipboard {
                    c.focus(*name, Some(id.clone()));
                }
                if let Some(previous) = previous {
                    desktop.keyboard_focus_changed(&previous, false);
                }
                desktop.keyboard_focus_changed(&id, true);
            }
            wl_keyboard::Event::RepeatInfo { rate, delay } => {
                keyboard.repeat.configure(rate, delay, Instant::now());
            }
            wl_keyboard::Event::Leave { surface, .. } => {
                let id = surface.id();
                if keyboard.focus.as_ref() != Some(&id) {
                    return;
                }
                keyboard.leave(&id);
                if let Some(c) = &desktop.clipboard {
                    c.focus(*name, None);
                }
                desktop.keyboard_focus_changed(&id, false);
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
                serial,
                key,
                state: WEnum::Value(state),
                ..
            } => {
                if state == wl_keyboard::KeyState::Released {
                    keyboard.repeat.release(key);
                }
                let Some(window) = keyboard
                    .focus
                    .as_ref()
                    .and_then(|id| desktop.windows.get_mut(id))
                    .filter(|window| {
                        window.mapping == super::Mapping::Mapped && !window.life.blocked()
                    })
                else {
                    return;
                };
                let state = match state {
                    wl_keyboard::KeyState::Pressed => KeyState::Down,
                    wl_keyboard::KeyState::Released => KeyState::Up,
                    _ => return,
                };
                let Some(layout) = &keyboard.layout else {
                    return;
                };
                if state == KeyState::Down
                    && let Some(code) = key.checked_add(8)
                    && layout.get_keymap().key_repeats(xkb::Keycode::new(code))
                {
                    keyboard.repeat.press(key, Instant::now());
                }
                let Some(key) = key_for_code(layout, key) else {
                    return;
                };
                let event = KeyEvent {
                    state,
                    key,
                    repeat: false,
                    modifiers: keyboard.modifiers,
                };
                if let Some(c) = &desktop.clipboard {
                    c.input(*name, &window.surface.id(), serial);
                }
                let _input = desktop
                    .activation
                    .as_ref()
                    .map(|activation| activation.input(&window.surface, seat, serial));
                let _menu_input =
                    (state == KeyState::Down).then(|| desktop.menus.input(window, seat, serial));
                window.event(WindowEvent::Key(event));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/keyboard.rs"]
mod tests;
