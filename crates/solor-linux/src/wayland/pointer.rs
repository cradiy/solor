use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
    globals::GlobalListContents,
    protocol::{wl_pointer, wl_registry, wl_seat},
};

use super::Window;
use solor::{
    layout::Origin,
    platform::WindowEvent,
    pointer::{PointerButton, PointerId, PointerInput, PointerKind, PointerSample, ScrollDelta},
    px,
};
fn sample(id: u32, x: f64, y: f64) -> PointerSample {
    PointerSample {
        id: PointerId(id.into()),
        kind: PointerKind::Mouse,
        position: Origin {
            x: px(x as f32),
            y: px(y as f32),
        },
        modifiers: Default::default(),
    }
}

pub(super) struct Seat {
    name: u32,
    seat: wl_seat::WlSeat,
    pointer: Option<wl_pointer::WlPointer>,
    position: Option<PointerSample>,
    scroll: Option<(PointerSample, ScrollDelta)>,
}

impl Seat {
    fn release_pointer(&mut self) {
        self.position = None;
        self.scroll = None;
        if let Some(pointer) = self.pointer.take()
            && pointer.version() >= 3
        {
            pointer.release();
        }
    }
}

impl Window {
    pub(super) fn add_seat(
        &mut self,
        registry: &wl_registry::WlRegistry,
        name: u32,
        version: u32,
        qh: &QueueHandle<Self>,
    ) {
        if !self.seats.iter().any(|s| s.name == name) {
            self.seats.push(Seat {
                name,
                seat: registry.bind(name, version.min(5), qh, name),
                pointer: None,
                position: None,
                scroll: None,
            });
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Window {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == "wl_seat" => state.add_seat(registry, name, version, qh),
            wl_registry::Event::GlobalRemove { name } => {
                if let Some(index) = state.seats.iter().position(|s| s.name == name) {
                    let mut seat = state.seats.swap_remove(index);
                    state.event(WindowEvent::Pointer(PointerInput::Cancel(PointerId(
                        name.into(),
                    ))));
                    seat.release_pointer();
                    if seat.seat.version() >= 5 {
                        seat.seat.release();
                    }
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, u32> for Window {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        name: &u32,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let Some(input) = state.seats.iter_mut().find(|s| &s.seat == seat) else {
            return;
        };
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = event
        {
            if caps.contains(wl_seat::Capability::Pointer) {
                if input.pointer.is_none() {
                    input.pointer = Some(seat.get_pointer(qh, *name));
                }
            } else {
                input.release_pointer();
                state.event(WindowEvent::Pointer(PointerInput::Cancel(PointerId(
                    (*name).into(),
                ))));
            }
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, u32> for Window {
    fn event(
        state: &mut Self,
        pointer: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(seat) = state
            .seats
            .iter_mut()
            .find(|s| s.pointer.as_ref() == Some(pointer))
        else {
            return;
        };
        let mut press = None;
        let event = match event {
            wl_pointer::Event::Enter {
                surface,
                surface_x,
                surface_y,
                ..
            } if surface == state.surface => {
                let p = sample(*name, surface_x, surface_y);
                seat.position = Some(p);
                PointerInput::Move(p)
            }
            wl_pointer::Event::Leave { .. } => {
                seat.position = None;
                seat.scroll = None;
                PointerInput::Cancel(PointerId((*name).into()))
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } if seat.position.is_some() => {
                let p = sample(*name, surface_x, surface_y);
                seat.position = Some(p);
                PointerInput::Move(p)
            }
            wl_pointer::Event::Axis {
                axis: WEnum::Value(axis),
                value,
                ..
            } => {
                let Some(p) = seat.position else { return };
                let (sample, delta) = seat.scroll.get_or_insert((p, ScrollDelta::default()));
                match axis {
                    wl_pointer::Axis::HorizontalScroll => delta.x += px(value as f32),
                    wl_pointer::Axis::VerticalScroll => delta.y += px(value as f32),
                    _ => return,
                }
                if pointer.version() >= 5 {
                    return;
                }
                let event = PointerInput::Scroll(*sample, *delta);
                seat.scroll = None;
                event
            }
            wl_pointer::Event::Frame => {
                let Some((p, delta)) = seat.scroll.take() else {
                    return;
                };
                PointerInput::Scroll(p, delta)
            }
            // Linux evdev BTN_LEFT. Wayland surface coordinates are already logical.
            wl_pointer::Event::Button {
                serial,
                button,
                state: WEnum::Value(button_state),
                ..
            } => {
                let Some(p) = seat.position else {
                    return;
                };
                let button = match button {
                    0x110 => PointerButton::Primary,
                    0x111 => PointerButton::Secondary,
                    0x112 => PointerButton::Middle,
                    other => match u16::try_from(other) {
                        Ok(code) => PointerButton::Other(code),
                        Err(_) => return,
                    },
                };
                match button_state {
                    wl_pointer::ButtonState::Pressed => {
                        if button == PointerButton::Primary {
                            press = Some((seat.seat.clone(), serial));
                        }
                        PointerInput::Down(p, button)
                    }
                    wl_pointer::ButtonState::Released => PointerInput::Up(p, button),
                    _ => return,
                }
            }
            _ => return,
        };
        state.dispatch(WindowEvent::Pointer(event), press);
    }
}
