use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop,
    globals::{BindError, GlobalList},
};
use wayland_protocols::xdg::{
    decoration::zv1::client::{
        zxdg_decoration_manager_v1::ZxdgDecorationManagerV1,
        zxdg_toplevel_decoration_v1::{Event, Mode, ZxdgToplevelDecorationV1},
    },
    shell::client::xdg_toplevel::XdgToplevel,
};

use super::{Role, Window};

pub(super) struct Decorations {
    handle: ZxdgToplevelDecorationV1,
    requested: bool,
    pending_configure: bool,
}

impl Decorations {
    pub(super) fn bind(
        globals: &GlobalList,
        qh: &QueueHandle<Window>,
        top: &XdgToplevel,
        enabled: bool,
    ) -> Result<Option<Self>, BindError> {
        let manager: ZxdgDecorationManagerV1 = match globals.bind(qh, 1..=1, ()) {
            Ok(manager) => manager,
            Err(BindError::NotPresent) => return Ok(None),
            Err(error) => return Err(error),
        };
        // Version 1 requires creating the object before the first buffer attachment.
        let handle = manager.get_toplevel_decoration(top, qh, ());
        manager.destroy();
        let decorations = Self {
            handle,
            requested: enabled,
            pending_configure: false,
        };
        decorations.request_mode();
        Ok(Some(decorations))
    }

    fn request_mode(&self) {
        self.handle.set_mode(if self.requested {
            Mode::ServerSide
        } else {
            Mode::ClientSide
        });
    }

    pub(super) fn toggle(&mut self) {
        self.requested = !self.requested;
        self.request_mode();
    }

    pub(super) fn take_configure(&mut self) -> bool {
        std::mem::take(&mut self.pending_configure)
    }
}

impl Drop for Decorations {
    fn drop(&mut self) {
        self.handle.destroy();
    }
}

delegate_noop!(Window: ignore ZxdgDecorationManagerV1);

impl Dispatch<ZxdgToplevelDecorationV1, ()> for Window {
    fn event(
        state: &mut Self,
        _: &ZxdgToplevelDecorationV1,
        event: Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let Event::Configure { .. } = event
            && let Role::Toplevel {
                decorations: Some(decorations),
                ..
            } = &mut state.role
        {
            decorations.pending_configure = true;
        }
    }
}
