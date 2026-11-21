//! Activation of an application window through xdg-activation-v1.
use std::{cell::RefCell, rc::Rc};

use solor::platform::PlatformWindow;

use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle,
    backend::ObjectId,
    delegate_noop,
    globals::GlobalList,
    protocol::{wl_seat::WlSeat, wl_surface::WlSurface},
};
use wayland_protocols::xdg::activation::v1::client::{
    xdg_activation_token_v1::{self, XdgActivationTokenV1},
    xdg_activation_v1::XdgActivationV1,
};

use super::{Desktop, Mapping};

pub(super) struct Activation {
    manager: XdgActivationV1,
    queue: QueueHandle<Desktop>,
    input: RefCell<Option<Input>>,
    pending: RefCell<Option<(ObjectId, Token)>>,
}

struct Input {
    surface: WlSurface,
    seat: WlSeat,
    serial: u32,
}

enum Token {
    Requested(XdgActivationTokenV1),
    Granted(String),
}

impl Drop for Token {
    fn drop(&mut self) {
        if let Self::Requested(token) = self {
            token.destroy();
        }
    }
}

impl Activation {
    pub(super) fn bind(globals: &GlobalList, queue: &QueueHandle<Desktop>) -> Option<Rc<Self>> {
        Some(Rc::new(Self {
            manager: globals.bind(queue, 1..=1, ()).ok()?,
            queue: queue.clone(),
            input: RefCell::default(),
            pending: RefCell::default(),
        }))
    }

    pub(super) fn input(&self, surface: &WlSurface, seat: &WlSeat, serial: u32) -> impl Drop + '_ {
        struct Scope<'a>(&'a Activation);
        impl Drop for Scope<'_> {
            fn drop(&mut self) {
                self.0.input.take();
            }
        }
        self.input.replace(Some(Input {
            surface: surface.clone(),
            seat: seat.clone(),
            serial,
        }));
        Scope(self)
    }

    fn request(&self, id: &ObjectId, app_id: Option<&str>) {
        let token = self.manager.get_activation_token(&self.queue, ());
        if let Some(input) = self.input.borrow().as_ref() {
            token.set_surface(&input.surface);
            token.set_serial(input.serial, &input.seat);
        }
        if let Some(app_id) = app_id {
            token.set_app_id(app_id.to_owned());
        }
        token.commit();
        self.pending
            .replace(Some((id.clone(), Token::Requested(token))));
    }

    // The grant and the target's first buffer commit can arrive in either order.
    pub(super) fn apply(&self, surface: &WlSurface) {
        let mut pending = self.pending.borrow_mut();
        if let Some((id, Token::Granted(token))) = pending.as_mut()
            && *id == surface.id()
        {
            self.manager.activate(std::mem::take(token), surface);
            pending.take();
        }
    }
}

pub(super) struct ActivationTarget {
    pub(super) activation: Rc<Activation>,
    pub(super) id: ObjectId,
    pub(super) app_id: Option<String>,
}

impl ActivationTarget {
    pub(super) fn request(&self) {
        self.activation.request(&self.id, self.app_id.as_deref());
    }

    pub(super) fn cancel(&self) {
        let mut pending = self.activation.pending.borrow_mut();
        if pending.as_ref().is_some_and(|(id, _)| *id == self.id) {
            pending.take();
        }
    }
}

delegate_noop!(Desktop: ignore XdgActivationV1);

impl Dispatch<XdgActivationTokenV1, ()> for Desktop {
    fn event(
        desktop: &mut Self,
        proxy: &XdgActivationTokenV1,
        event: xdg_activation_token_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let (Some(activation), xdg_activation_token_v1::Event::Done { token }) =
            (&desktop.activation, event)
        else {
            return;
        };
        let mut pending = activation.pending.borrow_mut();
        let Some((target, state)) = pending.as_mut() else {
            return;
        };
        if !matches!(state, Token::Requested(current) if current == proxy) {
            return;
        }
        let window = desktop.windows.get(target);
        *state = Token::Granted(token);
        drop(pending);
        if let Some(window) = window
            && window.mapping == Mapping::Mapped
            && window.life.is_open()
        {
            activation.apply(&window.surface);
        }
    }
}
