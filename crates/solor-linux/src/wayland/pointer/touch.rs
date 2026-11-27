use super::*;
use std::collections::HashMap;
use wayland_client::protocol::wl_touch::{Event, WlTouch};

pub(super) struct Touch {
    proxy: WlTouch,
    contacts: HashMap<i32, Contact>,
}

struct Contact {
    surface: ObjectId,
    sample: PointerSample,
    down_serial: u32,
}

impl Touch {
    pub fn new(proxy: WlTouch) -> Self {
        Self {
            proxy,
            contacts: Default::default(),
        }
    }

    pub fn cancel_window(&mut self, surface: &ObjectId, window: &mut Window) {
        self.contacts.retain(|_, c| {
            if &c.surface == surface {
                window.event(WindowEvent::Pointer(PointerInput::Cancel(c.sample.id)));
                false
            } else {
                true
            }
        });
    }

    pub fn cancel_all<'a>(&mut self, windows: impl Iterator<Item = &'a mut Window>) {
        for window in windows {
            if self.contacts.is_empty() {
                break;
            }
            self.cancel_window(&window.surface.id(), window);
        }
        self.contacts.clear();
    }
}

impl Drop for Touch {
    fn drop(&mut self) {
        if self.proxy.version() >= 3 {
            self.proxy.release();
        }
    }
}

impl Dispatch<WlTouch, u32> for Desktop {
    fn event(
        state: &mut Self,
        proxy: &WlTouch,
        event: Event,
        seat_name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(index) = state
            .seats
            .iter()
            .position(|s| s.touch.as_ref().is_some_and(|t| t.proxy == *proxy))
        else {
            return;
        };
        let touch = state.seats[index].touch.as_mut().unwrap();
        let surface = match &event {
            Event::Cancel => {
                touch.cancel_all(state.windows.values_mut());
                return;
            }
            Event::Down { surface, .. } => surface.id(),
            Event::Up { id, .. } | Event::Motion { id, .. } => {
                let Some(c) = touch.contacts.get(id) else {
                    return;
                };
                c.surface.clone()
            }
            _ => return,
        };
        let Some(window) = state.windows.get_mut(&surface) else {
            return;
        };
        cancel_modal_gestures(&mut state.seats, &surface, window);
        if window.life.blocked() || window.mapping != crate::wayland::Mapping::Mapped {
            return;
        }
        let seat = &mut state.seats[index];
        let modifiers = seat.modifiers();
        let contacts = &mut seat.touch.as_mut().unwrap().contacts;
        let (input, serial, grab_serial) = match event {
            Event::Down {
                id, x, y, serial, ..
            } => {
                let sample = PointerSample {
                    id: PointerId((u64::from(*seat_name) << 32) | u64::from(id as u32)),
                    kind: PointerKind::Touch,
                    position: Origin {
                        x: px(x as f32),
                        y: px(y as f32),
                    },
                    modifiers,
                };
                contacts.insert(
                    id,
                    Contact {
                        surface: surface.clone(),
                        sample,
                        down_serial: serial,
                    },
                );
                (
                    PointerInput::Down(sample, PointerButton::Primary),
                    Some(serial),
                    Some(serial),
                )
            }
            Event::Motion { id, x, y, .. } => {
                let Some(c) = contacts.get_mut(&id) else {
                    return;
                };
                c.sample.position = Origin {
                    x: px(x as f32),
                    y: px(y as f32),
                };
                c.sample.modifiers = modifiers;
                (PointerInput::Move(c.sample), None, None)
            }
            Event::Up { id, serial, .. } => {
                let Some(mut c) = contacts.remove(&id) else {
                    return;
                };
                c.sample.modifiers = modifiers;
                (
                    PointerInput::Up(c.sample, PointerButton::Primary),
                    Some(serial),
                    Some(c.down_serial),
                )
            }
            _ => return,
        };
        let press = serial
            .filter(|_| matches!(input, PointerInput::Down(..)))
            .map(|serial| (seat.seat.clone(), serial));
        let _menu_input = grab_serial.map(|serial| state.menus.input(window, &seat.seat, serial));
        let activation = serial.and_then(|serial| {
            if let Some(c) = &state.clipboard {
                c.input(*seat_name, &surface, serial);
            }
            state
                .activation
                .as_ref()
                .map(|a| a.input(&window.surface, &seat.seat, serial))
        });
        window.dispatch(WindowEvent::Pointer(input), press);
        drop(activation);
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/touch.rs"]
mod tests;
