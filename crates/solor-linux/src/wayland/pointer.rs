use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, WEnum, delegate_noop,
    globals::GlobalListContents,
    protocol::{wl_pointer, wl_registry, wl_seat},
};

use wayland_protocols::wp::cursor_shape::v1::client::{
    wp_cursor_shape_device_v1::{Shape, WpCursorShapeDeviceV1},
    wp_cursor_shape_manager_v1::WpCursorShapeManagerV1,
};

use super::{Window, keyboard::Keyboard};
use solor::{
    ResizeEdge,
    layout::Origin,
    platform::WindowEvent,
    pointer::{
        Modifiers, PointerButton, PointerId, PointerInput, PointerKind, PointerSample, ScrollDelta,
    },
    px,
    style::Cursor,
};
fn sample(id: u32, x: f64, y: f64, modifiers: Modifiers) -> PointerSample {
    PointerSample {
        id: PointerId(id.into()),
        kind: PointerKind::Mouse,
        position: Origin {
            x: px(x as f32),
            y: px(y as f32),
        },
        modifiers,
    }
}

delegate_noop!(Window: ignore WpCursorShapeDeviceV1);
delegate_noop!(Window: ignore WpCursorShapeManagerV1);

fn native_shape(cursor: Cursor) -> Option<Shape> {
    Some(match cursor {
        Cursor::Auto | Cursor::Default => Shape::Default,
        Cursor::Pointer => Shape::Pointer,
        Cursor::Text => Shape::Text,
        Cursor::Crosshair => Shape::Crosshair,
        Cursor::Move => Shape::Move,
        Cursor::Grab => Shape::Grab,
        Cursor::Grabbing => Shape::Grabbing,
        Cursor::NotAllowed => Shape::NotAllowed,
        Cursor::Wait => Shape::Wait,
        Cursor::Progress => Shape::Progress,
        Cursor::ColResize => Shape::ColResize,
        Cursor::RowResize => Shape::RowResize,
        Cursor::Resize(edge) => match edge {
            ResizeEdge::Top => Shape::NResize,
            ResizeEdge::TopRight => Shape::NeResize,
            ResizeEdge::Right => Shape::EResize,
            ResizeEdge::BottomRight => Shape::SeResize,
            ResizeEdge::Bottom => Shape::SResize,
            ResizeEdge::BottomLeft => Shape::SwResize,
            ResizeEdge::Left => Shape::WResize,
            ResizeEdge::TopLeft => Shape::NwResize,
        },
        Cursor::None => return None,
    })
}

pub(super) struct Seat {
    pub(super) keyboard: Option<Keyboard>,
    name: u32,
    seat: wl_seat::WlSeat,
    pointer: Option<wl_pointer::WlPointer>,
    position: Option<PointerSample>,
    shape_device: Option<WpCursorShapeDeviceV1>,
    enter_serial: Option<u32>,
    applied_cursor: Option<Cursor>,
    scroll: Option<(PointerSample, ScrollDelta)>,
}

impl Seat {
    fn modifiers(&self) -> Modifiers {
        self.keyboard
            .as_ref()
            .map_or(Modifiers::default(), |keyboard| keyboard.modifiers)
    }

    fn pointer_sample(&self) -> Option<PointerSample> {
        Some(PointerSample {
            modifiers: self.modifiers(),
            ..self.position?
        })
    }

    fn release_pointer(&mut self) {
        if let Some(device) = self.shape_device.take() {
            device.destroy();
        }
        self.enter_serial = None;
        self.applied_cursor = None;
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
    pub(super) fn cancel_pointers(&mut self) {
        for i in 0..self.seats.len() {
            let seat = &mut self.seats[i];
            seat.enter_serial = None;
            seat.applied_cursor = None;
            seat.scroll = None;
            if seat.position.take().is_some() {
                let id = PointerId(seat.name.into());
                self.event(WindowEvent::Pointer(PointerInput::Cancel(id)));
            }
        }
    }

    pub(super) fn sync_cursors(&mut self) {
        for seat in &mut self.seats {
            let (Some(device), Some(pointer), Some(serial)) =
                (&seat.shape_device, &seat.pointer, seat.enter_serial)
            else {
                continue;
            };
            let cursor = self.view.pointer_cursor(PointerId(seat.name.into()));
            if seat.applied_cursor == Some(cursor) {
                continue;
            }
            match native_shape(cursor) {
                Some(shape) => device.set_shape(serial, shape),
                None => pointer.set_cursor(serial, None, 0, 0),
            }
            seat.applied_cursor = Some(cursor);
        }
    }

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
                keyboard: None,
                position: None,
                shape_device: None,
                enter_serial: None,
                applied_cursor: None,
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
                    seat.keyboard = None;
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
            if caps.contains(wl_seat::Capability::Keyboard) {
                input
                    .keyboard
                    .get_or_insert_with(|| Keyboard::new(seat.get_keyboard(qh, *name)));
            } else {
                input.keyboard = None;
            }
            if caps.contains(wl_seat::Capability::Pointer) {
                if input.pointer.is_none() {
                    let pointer = seat.get_pointer(qh, *name);
                    input.shape_device = state
                        .cursor_shapes
                        .as_ref()
                        .map(|m| m.get_pointer(&pointer, qh, ()));
                    input.pointer = Some(pointer);
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
                serial,
                surface,
                surface_x,
                surface_y,
                ..
            } if surface == state.surface && state.mapping == super::Mapping::Mapped => {
                let p = sample(*name, surface_x, surface_y, seat.modifiers());
                seat.enter_serial = Some(serial);
                seat.applied_cursor = None;
                seat.position = Some(p);
                PointerInput::Move(p)
            }
            wl_pointer::Event::Leave { .. } => {
                seat.enter_serial = None;
                seat.applied_cursor = None;
                seat.position = None;
                seat.scroll = None;
                PointerInput::Cancel(PointerId((*name).into()))
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } if seat.position.is_some() => {
                let p = sample(*name, surface_x, surface_y, seat.modifiers());
                seat.position = Some(p);
                PointerInput::Move(p)
            }
            wl_pointer::Event::Axis {
                axis: WEnum::Value(axis),
                value,
                ..
            } => {
                let Some(p) = seat.pointer_sample() else {
                    return;
                };
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
                let Some(p) = seat.pointer_sample() else {
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
        state.sync_cursors();
    }
}
