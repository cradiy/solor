use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
    globals::GlobalListContents,
    protocol::{wl_pointer, wl_registry, wl_seat},
};

use super::Window;
use crate::input::{Point, PointerEvent};

pub(super) struct Seat {
    name: u32,
    seat: wl_seat::WlSeat,
    pointer: Option<wl_pointer::WlPointer>,
    position: Option<Point>,
    pending_axis_y: f64,
}

impl Seat {
    fn release_pointer(&mut self) {
        self.position = None;
        self.pending_axis_y = 0.0;
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
                pending_axis_y: 0.0,
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
        if !state.view.accepts_pointer() {
            return;
        }
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == "wl_seat" => state.add_seat(registry, name, version, qh),
            wl_registry::Event::GlobalRemove { name } => {
                if let Some(index) = state.seats.iter().position(|s| s.name == name) {
                    let mut seat = state.seats.swap_remove(index);
                    state.view.pointer(PointerEvent::Cancel(name.into()));
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
                state.view.pointer(PointerEvent::Cancel((*name).into()));
                input.release_pointer();
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
        let id = u64::from(*name);
        let event = match event {
            wl_pointer::Event::Enter {
                surface,
                surface_x,
                surface_y,
                ..
            } if surface == state.surface => {
                seat.position = Some(Point {
                    x: surface_x,
                    y: surface_y,
                });
                return;
            }
            wl_pointer::Event::Leave { .. } => {
                seat.position = None;
                seat.pending_axis_y = 0.0;
                PointerEvent::Cancel(id)
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } if seat.position.is_some() => {
                let p = Point {
                    x: surface_x,
                    y: surface_y,
                };
                seat.position = Some(p);
                PointerEvent::Move(id, p)
            }
            wl_pointer::Event::Axis {
                axis: WEnum::Value(wl_pointer::Axis::VerticalScroll),
                value,
                ..
            } if seat.position.is_some() => {
                if pointer.version() >= 5 {
                    seat.pending_axis_y += value;
                } else {
                    state.dirty |= state.view.scroll_by(value);
                }
                return;
            }
            wl_pointer::Event::Frame => {
                let delta = std::mem::take(&mut seat.pending_axis_y);
                if delta != 0.0 {
                    state.dirty |= state.view.scroll_by(delta);
                }
                return;
            }
            // Linux evdev BTN_LEFT. Wayland surface coordinates are already logical.
            wl_pointer::Event::Button {
                button: 0x110,
                state: WEnum::Value(button),
                ..
            } => {
                let Some(p) = seat.position else {
                    return;
                };
                match button {
                    wl_pointer::ButtonState::Pressed => PointerEvent::Down(id, p),
                    wl_pointer::ButtonState::Released => PointerEvent::Up(id, p),
                    _ => return,
                }
            }
            _ => return,
        };
        state.dirty |= state.view.pointer(event);
    }
}
