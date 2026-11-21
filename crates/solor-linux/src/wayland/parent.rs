//! Application-local toplevels available as native parents.
use std::{cell::RefCell, collections::HashMap, rc::Rc};

use anyhow::{Result, ensure};
use solor::platform::WindowHandle;
use wayland_protocols::xdg::shell::client::xdg_toplevel::XdgToplevel;

#[derive(Clone, Default)]
pub(super) struct Toplevels(Rc<RefCell<HashMap<WindowHandle, Parent>>>);

struct Parent {
    top: XdgToplevel,
    mapped: bool,
}

impl Toplevels {
    pub(super) fn check_parent(&self, parent: &WindowHandle) -> Result<()> {
        ensure!(
            parent.is_open() && self.0.borrow().contains_key(parent),
            "parent must be an open toplevel in this application"
        );
        Ok(())
    }

    pub(super) fn register(
        &self,
        handle: WindowHandle,
        top: &XdgToplevel,
        parent: Option<&WindowHandle>,
    ) -> Registration {
        self.0.borrow_mut().insert(
            handle.clone(),
            Parent {
                top: top.clone(),
                mapped: false,
            },
        );
        Registration {
            handle,
            windows: self.clone(),
            parent: parent.cloned(),
        }
    }
}

pub(super) struct Registration {
    handle: WindowHandle,
    windows: Toplevels,
    parent: Option<WindowHandle>,
}

impl Registration {
    /// A parent must have committed a buffer before xdg-shell will honor set_parent.
    pub(super) fn prepare_map(&mut self) -> bool {
        let Some(handle) = &self.parent else {
            return true;
        };
        let windows = self.windows.0.borrow();
        if handle.is_open()
            && let Some(parent) = windows.get(handle)
        {
            if !parent.mapped {
                return false;
            }
            windows[&self.handle].top.set_parent(Some(&parent.top));
        }
        self.parent = None;
        true
    }

    pub(super) fn mapped(&self) {
        self.windows
            .0
            .borrow_mut()
            .get_mut(&self.handle)
            .expect("registered toplevel")
            .mapped = true;
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.windows.0.borrow_mut().remove(&self.handle);
    }
}
