//! Session locks presented on every advertised output.
use super::{Desktop, WaylandApplication, role::Request};
use anyhow::{Context, Result, ensure};
use solor::{
    layout::Size,
    platform::{WindowHandle, WindowHandler},
    px,
    raster::Rgba8,
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::{Rc, Weak},
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle,
    backend::ObjectId,
    delegate_noop,
    globals::GlobalList,
    protocol::{wl_callback, wl_display::WlDisplay, wl_output::WlOutput},
};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1::ExtSessionLockManagerV1,
    ext_session_lock_surface_v1::{self as surface, ExtSessionLockSurfaceV1},
    ext_session_lock_v1::{self as lock, ExtSessionLockV1},
};

/// Compositor-confirmed lifecycle of a session lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionLockState {
    /// Waiting for the compositor to secure all outputs.
    Pending,
    /// The compositor has secured the session.
    Locked,
    /// The lock is being released or dismissed.
    Releasing,
    /// The compositor has processed release or dismissal.
    Finished,
}

/// UI-thread handle for one lock request. Only an explicit unlock releases a confirmed lock.
#[derive(Clone)]
pub struct SessionLock {
    active: Weak<Active>,
    state: Rc<Cell<SessionLockState>>,
}

impl SessionLock {
    /// Returns the current state of this request.
    pub fn state(&self) -> SessionLockState {
        self.state.get()
    }

    /// Requests unlock after the application has authenticated the user.
    pub fn unlock(&self) -> Result<()> {
        let active = self.active.upgrade().context("session lock has ended")?;
        ensure!(
            active.state.get() == SessionLockState::Locked,
            "session lock is not confirmed"
        );
        active.release();
        Ok(())
    }
}

type Build = dyn FnMut(u32, &SessionLock, &WlOutput) -> Result<WindowHandle>;

struct Active {
    proxy: ExtSessionLockV1,
    display: WlDisplay,
    qh: QueueHandle<Desktop>,
    state: Rc<Cell<SessionLockState>>,
    build: RefCell<Box<Build>>,
    notify: RefCell<Box<dyn FnMut(SessionLockState)>>,
    outputs: RefCell<HashMap<u32, WindowHandle>>,
}

impl Active {
    fn release(&self) {
        match self.state.replace(SessionLockState::Releasing) {
            SessionLockState::Locked => self.proxy.unlock_and_destroy(),
            SessionLockState::Pending => self.proxy.destroy(),
            _ => unreachable!("release requires an active lock"),
        }
        for (_, window) in self.outputs.borrow_mut().drain() {
            window.close();
        }
        self.display.sync(&self.qh, Released(self.proxy.id()));
    }
}

#[derive(Default)]
pub(super) struct Locks {
    active: RefCell<Option<Rc<Active>>>,
    pub(super) outputs_dirty: Cell<bool>,
    // wl_output v1/v2 have no release request. Bind each such global once.
    legacy_outputs: RefCell<HashMap<u32, WlOutput>>,
}

impl Locks {
    pub(super) fn active(&self) -> bool {
        self.active.borrow().is_some()
    }

    pub(super) fn remove_global(&self, name: u32) {
        if let Some(output) = self.legacy_outputs.borrow_mut().remove(&name)
            && let Some(backend) = output.backend().upgrade()
        {
            let _ = backend.destroy_object(&output.id());
        }
        self.outputs_dirty.set(true);
    }

    pub(super) fn sync_outputs(&self, globals: &GlobalList) {
        if !self.outputs_dirty.replace(false) {
            return;
        }
        let Some(active) = self.active.borrow().clone() else {
            return;
        };
        let list = globals.contents().clone_list();
        let mut outputs = active.outputs.borrow_mut();
        outputs.retain(|name, window| {
            let keep = list.iter().any(|global| global.name == *name);
            if !keep {
                window.close();
            }
            keep
        });
        drop(outputs);
        let handle = SessionLock {
            active: Rc::downgrade(&active),
            state: active.state.clone(),
        };
        for global in list.iter().filter(|g| g.interface == "wl_output") {
            if matches!(
                active.state.get(),
                SessionLockState::Releasing | SessionLockState::Finished
            ) {
                break;
            }
            if active.outputs.borrow().contains_key(&global.name) {
                continue;
            }
            let bind = || {
                globals
                    .registry()
                    .bind(global.name, global.version.min(3), &active.qh, ())
            };
            let output: WlOutput = if global.version < 3 {
                self.legacy_outputs
                    .borrow_mut()
                    .entry(global.name)
                    .or_insert_with(bind)
                    .clone()
            } else {
                bind()
            };
            let result = (active.build.borrow_mut())(global.name, &handle, &output);
            if output.version() >= 3 {
                output.release();
            }
            let window = match result {
                Ok(window) => window,
                Err(error) => {
                    log::error!(
                        "Could not create lock surface for output {}: {error:#}",
                        global.name
                    );
                    continue;
                }
            };
            if active.state.get() == SessionLockState::Releasing {
                window.close();
                break;
            }
            active.outputs.borrow_mut().insert(global.name, window);
        }
    }
}

impl WaylandApplication {
    /// Requests a lock and builds an opaque view for each output, including hotplugged outputs.
    /// The numeric output ID is its registry name. Notifications report confirmation and completion.
    pub fn lock_session(
        &self,
        mut build: impl FnMut(u32, &SessionLock) -> Result<Box<dyn WindowHandler<Rgba8>>> + 'static,
        notify: impl FnMut(SessionLockState) + 'static,
    ) -> Result<SessionLock> {
        let app = self.clone();
        self.request_lock(
            move |name, lock, output| {
                let active = lock.active.upgrade().context("session lock has ended")?;
                app.factory()?
                    .open(Request::Lock(&active.proxy, output), None, |_| {
                        build(name, lock)
                    })
            },
            notify,
        )
    }

    /// Presents per-output lock views using the application's shared GPU device.
    #[cfg(feature = "gpu")]
    pub fn lock_gpu_session<R: solor::Render>(
        &self,
        mut build: impl FnMut(u32, &SessionLock) -> Result<(solor::Ui, R)> + 'static,
        notify: impl FnMut(SessionLockState) + 'static,
    ) -> Result<SessionLock> {
        let app = self.clone();
        self.request_lock(
            move |name, lock, output| {
                let (ui, root) = build(name, lock)?;
                ensure!(
                    lock.state() != SessionLockState::Releasing,
                    "session lock is ending"
                );
                let active = lock.active.upgrade().context("session lock has ended")?;
                app.build_gpu(
                    Request::Lock(&active.proxy, output),
                    None,
                    solor::platform::WindowBackground::Opaque,
                    ui,
                    |_| root,
                )
            },
            notify,
        )
    }

    fn request_lock(
        &self,
        build: impl FnMut(u32, &SessionLock, &WlOutput) -> Result<WindowHandle> + 'static,
        notify: impl FnMut(SessionLockState) + 'static,
    ) -> Result<SessionLock> {
        let factory = self.factory()?;
        ensure!(!factory.locks.active(), "session lock already requested");
        let globals = factory.globals.as_ref().context("application has exited")?;
        let manager: ExtSessionLockManagerV1 = globals
            .bind(&factory.qh, 1..=1, ())
            .context("compositor does not support session locking")?;
        let display = Connection::from_backend(
            manager
                .backend()
                .upgrade()
                .context("display disconnected")?,
        )
        .display();
        let proxy = manager.lock(&factory.qh, ());
        manager.destroy();
        let active = Rc::new(Active {
            proxy,
            display,
            qh: factory.qh.clone(),
            state: Rc::new(Cell::new(SessionLockState::Pending)),
            build: RefCell::new(Box::new(build)),
            notify: RefCell::new(Box::new(notify)),
            outputs: Default::default(),
        });
        let handle = SessionLock {
            active: Rc::downgrade(&active),
            state: active.state.clone(),
        };
        factory.locks.active.replace(Some(active));
        factory.locks.outputs_dirty.set(true);
        factory.revisit.set(true);
        Ok(handle)
    }
}

delegate_noop!(Desktop: ignore ExtSessionLockManagerV1);
delegate_noop!(Desktop: ignore WlOutput);

struct Released(ObjectId);
impl Dispatch<wl_callback::WlCallback, Released> for Desktop {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        done: &Released,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let active = state.locks.active.borrow().clone();
        if let Some(active) = active.filter(|a| a.proxy.id() == done.0) {
            state.locks.active.take();
            active.state.set(SessionLockState::Finished);
            (active.notify.borrow_mut())(SessionLockState::Finished);
        }
    }
}

impl Dispatch<ExtSessionLockV1, ()> for Desktop {
    fn event(
        state: &mut Self,
        proxy: &ExtSessionLockV1,
        event: lock::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let active = state.locks.active.borrow().clone();
        let Some(active) = active.filter(|a| a.proxy == *proxy) else {
            return;
        };
        match event {
            lock::Event::Locked => {
                active.state.set(SessionLockState::Locked);
                (active.notify.borrow_mut())(SessionLockState::Locked);
            }
            lock::Event::Finished if active.state.get() != SessionLockState::Releasing => {
                active.release()
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtSessionLockSurfaceV1, ObjectId> for Desktop {
    fn event(
        state: &mut Self,
        role: &ExtSessionLockSurfaceV1,
        event: surface::Event,
        id: &ObjectId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let surface::Event::Configure {
            serial,
            width,
            height,
        } = event
            && let Some(window) = state.windows.get_mut(id)
            && window.mapping.configure()
        {
            role.ack_configure(serial);
            window.resize(Size {
                width: px(width as f32),
                height: px(height as f32),
            });
        }
    }
}
