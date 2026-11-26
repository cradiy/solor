use super::*;
use crate::wayland::{Mapping, application::fixture::Host};
use solor::{
    layout::Size,
    platform::{EventResponse, WindowHandler},
    raster::{RasterFrame, RasterScale, Rgba8},
};
use std::{cell::RefCell, rc::Rc};

#[derive(Clone, Default)]
struct Input(Rc<RefCell<Vec<PointerInput>>>);
impl WindowHandler<Rgba8> for Input {
    fn event(&mut self, event: WindowEvent) -> anyhow::Result<EventResponse> {
        if let WindowEvent::Pointer(p) = event {
            self.0.borrow_mut().push(p);
        }
        Ok(Default::default())
    }
    fn frame(&mut self, _: Size, _: RasterScale) -> anyhow::Result<RasterFrame<'_>> {
        unreachable!()
    }
}

#[test]
fn contacts_keep_their_window_until_release_or_device_loss() -> anyhow::Result<()> {
    let mut host = Host::new()?;
    let left = Input::default();
    let right = Input::default();
    let a = host.open(left.clone())?;
    let b = host.open(right.clone())?;
    let seat = host.desktop().seats[0].seat.clone();
    host.deliver(
        &seat,
        wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(wl_seat::Capability::Touch),
        },
    );
    let touch = host.desktop().seats[0]
        .touch
        .as_ref()
        .unwrap()
        .proxy
        .clone();
    for (id, window) in [(1, a), (2, b)] {
        let window = host.desktop().windows.get_mut(&window).unwrap();
        window.mapping = Mapping::Mapped;
        let surface = window.surface.clone();
        host.deliver(
            &touch,
            Event::Down {
                serial: 10 + id as u32,
                time: 10,
                surface,
                id,
                x: 2.,
                y: 3.,
            },
        );
    }
    host.deliver(
        &touch,
        Event::Motion {
            time: 11,
            id: 2,
            x: 20.5,
            y: 12.25,
        },
    );
    host.deliver(
        &touch,
        Event::Up {
            serial: 15,
            time: 12,
            id: 2,
        },
    );
    host.deliver(
        &seat,
        wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(wl_seat::Capability::empty()),
        },
    );
    let l = left.0.borrow();
    let r = right.0.borrow();
    let [
        PointerInput::Down(first, _),
        PointerInput::Cancel(cancelled),
    ] = l.as_slice()
    else {
        panic!("{l:?}")
    };
    let [
        PointerInput::Down(second, _),
        PointerInput::Move(moved),
        PointerInput::Up(released, _),
    ] = r.as_slice()
    else {
        panic!("{r:?}")
    };
    assert_eq!(first.kind, PointerKind::Touch);
    assert_eq!(first.id, *cancelled);
    assert_ne!(first.id, second.id);
    assert_eq!(second.id, released.id);
    assert_eq!(moved, released);
    assert_eq!(
        released.position,
        Origin {
            x: px(20.5),
            y: px(12.25)
        }
    );
    Ok(())
}
