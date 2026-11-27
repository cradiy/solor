use super::*;
use rustix::fs::{MemfdFlags, memfd_create};
use std::io::Write;

#[test]
fn held_key_uses_seat_timing_and_stops_on_release() {
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    let mut repeat = Repeat::default();
    repeat.configure(20, 300, start);
    repeat.press(30, start);
    assert_eq!(repeat.due(at(299)), None);
    assert_eq!(repeat.due(at(300)), Some(30));
    assert_eq!(repeat.due(at(350)), Some(30));
    repeat.press(14, at(360));
    repeat.release(30);
    assert_eq!(repeat.due(at(400)), None);
    assert_eq!(repeat.due(at(700)), Some(14));
    assert_eq!(repeat.due(at(1000)), Some(14));
    assert_eq!(repeat.due(at(1000)), None);
    repeat.release(14);
    assert_eq!(repeat.due(at(1100)), None);
    repeat.press(30, at(1200));
    repeat.configure(0, 300, at(1250));
    assert_eq!(repeat.due(at(1600)), None);
}

fn keymap_file() -> (File, u32) {
    let map = br#"xkb_keymap {
        xkb_keycodes "codes" { minimum = 8; maximum = 255; <AC01> = 38; <FK10> = 76; };
        xkb_types "types" {
            type "TWO_LEVEL" {
                modifiers = Shift;
                map[None] = Level1;
                map[Shift] = Level2;
            };
        };
        xkb_compatibility "compat" {};
        xkb_symbols "symbols" {
            key <AC01> { type="TWO_LEVEL", [ Greek_alpha, Greek_ALPHA ] };
            key <FK10> { repeat=yes, [ F10 ] };
        };
    };"#;
    let mut file = File::from(memfd_create("keymap", MemfdFlags::CLOEXEC).unwrap());
    file.write_all(map).unwrap();
    file.write_all(&[0]).unwrap();
    (file, map.len() as u32 + 1)
}

#[test]
fn shared_keymap_resolves_the_declared_symbol_and_shift_level() {
    let (file, size) = keymap_file();
    assert!(read_layout(file.try_clone().unwrap().into(), size + 1).is_err());
    let plain = read_layout(file.try_clone().unwrap().into(), size - 1).unwrap();
    assert_eq!(key_for_code(&plain, 30), Some(Key::Character('α')));
    let mut layout = read_layout(file.into(), size).unwrap();
    assert_eq!(key_for_code(&layout, 30), Some(Key::Character('α')));
    let shift = layout.get_keymap().mod_get_index(xkb::MOD_NAME_SHIFT);
    layout.update_mask(1 << shift, 0, 0, 0, 0, 0);
    assert_eq!(key_for_code(&layout, 30), Some(Key::Character('Α')));
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

#[test]
fn a_keyboard_popup_grab_is_scoped_to_the_focused_key_press() -> Result<()> {
    use crate::{popup::PopupOptions, wayland::application::fixture::Host};
    use solor::{
        layout::Size,
        platform::{EventResponse, SurfaceRect, WindowHandler, WindowOptions},
        px,
        raster::{RasterFrame, RasterScale, Rgba8},
    };
    use std::{cell::RefCell, rc::Rc};
    use wayland_client::protocol::wl_seat;

    struct OnKey<F>(F);
    impl<F: FnMut(KeyEvent)> WindowHandler<Rgba8> for OnKey<F> {
        fn event(&mut self, event: WindowEvent) -> Result<EventResponse> {
            if let WindowEvent::Key(key) = event {
                (self.0)(key);
            }
            Ok(EventResponse::default())
        }
        fn frame(&mut self, _: Size, _: RasterScale) -> Result<RasterFrame<'_>> {
            unreachable!()
        }
    }

    let mut host = Host::new()?;
    let app = host.application();
    let options = PopupOptions::new(
        SurfaceRect::new(px(4.), px(4.), px(20.), px(10.)),
        Size {
            width: px(48.),
            height: px(32.),
        },
    );
    let open = |app: &crate::WaylandApplication, options| {
        app.build_popup(options, |_| Ok(Box::new(OnKey(|_| {}))))
    };
    let attempts = Rc::new(RefCell::new(Vec::new()));
    let view = OnKey({
        let app = app.clone();
        let options = options.clone();
        let attempts = attempts.clone();
        move |event: KeyEvent| {
            if event.key == Key::Named(NamedKey::F10) {
                attempts
                    .borrow_mut()
                    .push((event.repeat, open(&app, options.clone()).is_ok()));
            }
        }
    });
    let parent = app.open_window(WindowOptions::new("Menu"), Box::new(view))?;
    let id = host.install_pending();
    let window = host.desktop().windows.get_mut(&id).unwrap();
    window.mark_mapped();
    let surface = window.surface.clone();
    let seat = host.desktop().seats[0].seat.clone();
    host.deliver(
        &seat,
        wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(wl_seat::Capability::Keyboard),
        },
    );
    let keyboard = host.desktop().seats[0].keyboard.as_mut().unwrap();
    let (file, size) = keymap_file();
    keyboard.layout = Some(read_layout(file.into(), size)?);
    let device = keyboard.device.clone();
    host.deliver(
        &device,
        wl_keyboard::Event::Enter {
            serial: 8,
            surface,
            keys: Vec::new(),
        },
    );
    let press = |host: &mut Host, serial, key| {
        host.deliver(
            &device,
            wl_keyboard::Event::Key {
                serial,
                time: 30,
                key,
                state: WEnum::Value(wl_keyboard::KeyState::Pressed),
            },
        )
    };
    press(&mut host, 9, 68);
    let desktop = host.desktop();
    let window = desktop.windows.get_mut(&id).unwrap();
    repeat_keys(
        &mut desktop.seats,
        &id,
        window,
        Instant::now() + Duration::from_secs(1),
    );
    assert_eq!(attempts.borrow().as_slice(), [(false, true), (true, false)]);
    press(&mut host, 10, 30);
    let explicit_parent = PopupOptions {
        parent: Some(parent),
        ..options
    };
    assert!(open(&app, explicit_parent).is_err());
    Ok(())
}
