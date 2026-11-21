use super::{
    DamageAudit, Mapping, Wayland, Window,
    activation::{Activation, ActivationTarget},
    buffer::PixelOrder,
    buffer_size, floating,
    parent::Toplevels,
    pointer,
    role::{Request, Role},
};
use crate::layer_shell::{Binding, LayerShellOptions};
use solor::{
    platform::{PlatformWindow, WindowBackground, WindowHandle, WindowHandler, WindowOptions},
    raster::Rgba8,
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    io,
    rc::{Rc, Weak},
    time::Instant,
};
use wayland_client::{
    Connection, EventQueue, Proxy, QueueHandle,
    backend::ObjectId,
    globals::{GlobalList, registry_queue_init},
    protocol::{wl_compositor, wl_shm, wl_surface},
};
use wayland_protocols::wp::{
    cursor_shape::v1::client::wp_cursor_shape_manager_v1::WpCursorShapeManagerV1,
    fractional_scale::v1::client::wp_fractional_scale_manager_v1,
    presentation_time::client::wp_presentation, viewporter::client::wp_viewporter,
};

#[derive(Default)]
pub(super) struct Desktop {
    pub(super) activation: Option<Rc<Activation>>,
    pub(super) windows: HashMap<ObjectId, Window>,
    pub(super) seats: Vec<pointer::Seat>,
    pub(super) cursor_shapes: Option<WpCursorShapeManagerV1>,
    pub(super) rgb_opaque: bool,
    pub(super) rgb_transparent: bool,
}

impl Desktop {
    fn order(&self, background: WindowBackground) -> PixelOrder {
        match background {
            WindowBackground::Opaque if self.rgb_opaque => PixelOrder::Rgb,
            WindowBackground::Transparent if self.rgb_transparent => PixelOrder::Rgb,
            _ => PixelOrder::Bgr,
        }
    }
}

/// A UI-thread handle for opening windows on a running Wayland connection.
#[derive(Clone)]
pub struct WaylandApplication(Weak<WindowFactory>);

impl WaylandApplication {
    /// Creates an XDG toplevel and schedules it for dispatch after the current callback.
    /// Returns an error if the application has exited or window creation fails.
    pub fn open_window(
        &self,
        options: WindowOptions<'_>,
        view: Box<dyn WindowHandler<Rgba8>>,
    ) -> anyhow::Result<WindowHandle> {
        self.open_window_with(options, || Ok(view))
    }

    /// Builds the handler on the UI thread once the application is available.
    pub fn open_window_with(
        &self,
        options: WindowOptions<'_>,
        build: impl FnOnce() -> anyhow::Result<Box<dyn WindowHandler<Rgba8>>>,
    ) -> anyhow::Result<WindowHandle> {
        self.build_window(options, |_| build())
    }

    /// Builds the window's handler with a weak handle to its newly created toplevel.
    pub fn build_window(
        &self,
        options: WindowOptions<'_>,
        build: impl FnOnce(WindowHandle) -> anyhow::Result<Box<dyn WindowHandler<Rgba8>>>,
    ) -> anyhow::Result<WindowHandle> {
        let windows = self
            .0
            .upgrade()
            .ok_or_else(|| anyhow::anyhow!("application has exited"))?;
        windows
            .open(Request::Toplevel(options), build)
            .map(|(_, handle)| handle)
    }
}

pub(super) struct WindowLife {
    pub(super) activation: Option<ActivationTarget>,
    open: Cell<bool>,
    modal_parent: Weak<Self>,
    modal_children: RefCell<Vec<Weak<Self>>>,
    cancel_pointer: Cell<bool>,
    pub(super) revisit: Rc<Cell<bool>>,
}

impl WindowLife {
    fn new(
        activation: Option<ActivationTarget>,
        revisit: Rc<Cell<bool>>,
        modal_parent: Weak<Self>,
    ) -> Rc<Self> {
        Rc::new(Self {
            activation,
            open: Cell::new(true),
            revisit,
            modal_parent,
            modal_children: RefCell::default(),
            cancel_pointer: Cell::default(),
        })
    }

    fn attach_modal(self: &Rc<Self>, parent: &Self) {
        if !parent.is_open() {
            self.close();
        } else if self.is_open() {
            let mut children = parent.modal_children.borrow_mut();
            if children.is_empty() {
                parent.cancel_pointer.set(true);
            }
            children.push(Rc::downgrade(self));
        }
    }

    pub(super) fn blocked(&self) -> bool {
        !self.modal_children.borrow().is_empty()
    }

    pub(super) fn take_pointer_cancel(&self) -> bool {
        self.cancel_pointer.replace(false)
    }
}

impl PlatformWindow for WindowLife {
    fn is_open(&self) -> bool {
        self.open.get()
    }

    fn activate(&self) {
        if let Some(target) = &self.activation {
            target.request();
        }
    }

    fn close(&self) {
        if self.open.replace(false) {
            if let Some(target) = &self.activation {
                target.cancel();
            }

            if let Some(parent) = self.modal_parent.upgrade() {
                parent
                    .modal_children
                    .borrow_mut()
                    .retain(|child| !std::ptr::eq(child.as_ptr(), self));
            }
            let children = self.modal_children.take();
            for child in children.into_iter().filter_map(|child| child.upgrade()) {
                child.close();
            }
            self.revisit.set(true);
        }
    }
}

impl Drop for WindowLife {
    fn drop(&mut self) {
        self.close();
    }
}

struct EventLoop {
    windows: Rc<WindowFactory>,
    connection: Connection,
    queue: EventQueue<Desktop>,
    desktop: Desktop,
}

struct WindowFactory {
    toplevels: Toplevels,
    activation: Option<Rc<Activation>>,
    qh: QueueHandle<Desktop>,
    pending: RefCell<Vec<Window>>,
    revisit: Rc<Cell<bool>>,
    globals: GlobalList,
    compositor: wl_compositor::WlCompositor,
    shm: wl_shm::WlShm,
    viewporter: wp_viewporter::WpViewporter,
    scale_manager: wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    presentation: Option<wp_presentation::WpPresentation>,
    verify_shm: bool,
    trace_submissions: bool,
}

impl Wayland {
    /// Opens the initial windows, then dispatches events until the last window closes.
    pub fn run(
        &self,
        setup: impl FnOnce(WaylandApplication) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        let mut events = EventLoop::connect(self)?;
        setup(WaylandApplication(Rc::downgrade(&events.windows)))?;
        events.dispatch(None)
    }

    /// Runs a desktop layer surface until the compositor or the application closes it.
    pub fn run_layer_surface(
        &self,
        options: LayerShellOptions<'_>,
        view: Box<dyn WindowHandler<Rgba8>>,
    ) -> anyhow::Result<()> {
        let control = options.handle.map(|h| h.claim()).transpose()?;
        let mut events = EventLoop::connect(self)?;
        let (id, _) = events.windows.open(Request::Layer(options), |_| Ok(view))?;
        events.dispatch(control.map(|control| (id, control)))
    }
}

impl EventLoop {
    fn connect(host: &Wayland) -> anyhow::Result<Self> {
        let connection = Connection::connect_to_env()?;
        let (globals, queue) = registry_queue_init::<Desktop>(&connection)?;
        let qh = queue.handle();
        let mut desktop = Desktop {
            activation: Activation::bind(&globals, &qh),
            cursor_shapes: globals.bind(&qh, 1..=1, ()).ok(),
            ..Desktop::default()
        };
        for global in globals.contents().clone_list() {
            if global.interface == "wl_seat" {
                desktop.add_seat(globals.registry(), global.name, global.version, &qh);
            }
        }
        let windows = WindowFactory {
            toplevels: Toplevels::default(),
            activation: desktop.activation.clone(),
            compositor: globals.bind(&qh, 4..=4, ())?,
            shm: globals.bind(&qh, 1..=1, ())?,
            viewporter: globals.bind(&qh, 1..=1, ())?,
            scale_manager: globals.bind(&qh, 1..=1, ())?,
            presentation: if host.trace_submissions {
                globals.bind(&qh, 1..=1, ()).ok()
            } else {
                None
            },
            trace_submissions: host.trace_submissions,
            verify_shm: host.verify_shm,
            qh,
            globals,
            pending: RefCell::default(),
            revisit: Rc::default(),
        };
        Ok(Self {
            windows: Rc::new(windows),
            connection,
            queue,
            desktop,
        })
    }
}

impl WindowFactory {
    fn open(
        &self,
        request: Request<'_>,
        build: impl FnOnce(WindowHandle) -> anyhow::Result<Box<dyn WindowHandler<Rgba8>>>,
    ) -> anyhow::Result<(ObjectId, WindowHandle)> {
        let modal_parent = match &request {
            Request::Toplevel(options) => {
                anyhow::ensure!(
                    !options.modal || options.parent.is_some(),
                    "a modal window requires a parent"
                );
                options
                    .parent
                    .map(|handle| self.toplevels.parent_life(handle))
                    .transpose()?
                    .filter(|_| options.modal)
            }
            Request::Layer(_) => None,
        };
        let surface = self.compositor.create_surface(&self.qh, ());
        let id = surface.id();
        match self.create_window(
            &surface,
            request,
            modal_parent.as_ref().map(Rc::downgrade).unwrap_or_default(),
            build,
        ) {
            Ok((window, handle)) => {
                if let Some(parent) = modal_parent {
                    window.life.attach_modal(&parent);
                }
                self.pending.borrow_mut().push(window);
                Ok((id, handle))
            }
            Err(error) => {
                surface.destroy();
                Err(error)
            }
        }
    }

    fn create_window(
        &self,
        surface: &wl_surface::WlSurface,
        request: Request<'_>,
        modal_parent: Weak<WindowLife>,
        build: impl FnOnce(WindowHandle) -> anyhow::Result<Box<dyn WindowHandler<Rgba8>>>,
    ) -> anyhow::Result<(Window, WindowHandle)> {
        let qh = &self.qh;
        let background = match &request {
            Request::Layer(options) => options.background,
            Request::Toplevel(options) => options.background,
        };
        let parent = match &request {
            Request::Toplevel(options) => options.parent,
            Request::Layer(_) => None,
        };
        let activation = match (&request, &self.activation) {
            (Request::Toplevel(options), Some(activation)) => Some(ActivationTarget {
                activation: activation.clone(),
                id: surface.id(),
                app_id: options.app_id.map(str::to_owned),
            }),
            _ => None,
        };
        let (role, size) = request.create(&self.globals, qh, surface)?;
        buffer_size(120, size)?;
        let life = WindowLife::new(activation, self.revisit.clone(), modal_parent);
        let target = Rc::downgrade(&life);
        let handle = WindowHandle::new(target);
        let registration = match &role {
            Role::Toplevel { top, .. } => {
                Some(self.toplevels.register(handle.clone(), &life, top, parent))
            }
            Role::Layer(..) => None,
        };
        let mut view = build(handle.clone())?;
        view.set_window_resizable(matches!(
            &role,
            Role::Toplevel {
                resizable: true,
                ..
            }
        ))?;
        let floating = match &role {
            Role::Layer(_, p) if p.movable => Some(floating::FloatingPanel::new(
                &self.globals,
                &self.compositor,
                &self.shm,
                &self.viewporter,
                surface,
                floating::Geometry::new(size, p.anchor, p.margin),
                qh,
            )?),
            _ => None,
        };
        let content = floating.as_ref().map_or(surface, |p| &p.content);
        let viewport = self.viewporter.get_viewport(content, qh, ());

        let window = Window {
            role,
            floating,
            maximized: false,
            pending_maximized: false,
            presentation: self.presentation.clone(),
            fractional: self
                .scale_manager
                .get_fractional_scale(surface, qh, surface.id()),
            viewport,
            scale_120: 120,
            surface: surface.clone(),
            shm: self.shm.clone(),
            background,
            pixel_order: PixelOrder::Bgr,
            slots: [None, None],
            next_slot: 0,
            view,
            full_damage: true,
            failure: None,
            trace: self.trace_submissions,
            size,
            pending_size: None,
            mapping: Mapping::Hidden,
            input_region: Default::default(),
            dirty: true,
            waiting: None,
            life,
            registration,
            commits: 0,
            audit: self.verify_shm.then(DamageAudit::default),
            start: Instant::now(),
        };
        Ok((window, handle))
    }
}

impl EventLoop {
    fn dispatch(&mut self, control: Option<(ObjectId, Binding<'_>)>) -> anyhow::Result<()> {
        let qh = self.queue.handle();
        loop {
            self.windows.revisit.set(false);
            for mut window in self.windows.pending.take() {
                window.pixel_order = self.desktop.order(window.background);
                self.desktop.windows.insert(window.surface.id(), window);
            }
            let mut deadline = None;
            let mut closed = Vec::new();
            for (id, window) in &mut self.desktop.windows {
                if let Some(error) = window.failure.take() {
                    return Err(error);
                }
                let handle = control
                    .as_ref()
                    .filter(|(target, _)| target == id)
                    .map(|(_, h)| h);
                let Some(visible) = handle
                    .map_or(Some(true), Binding::visibility)
                    .filter(|_| window.life.is_open())
                else {
                    closed.push(id.clone());
                    continue;
                };
                pointer::cancel_modal_gestures(&mut self.desktop.seats, id, window);
                if let Some(handle) = handle {
                    window.input_region.update(handle.take_input_region());
                }
                if visible && (handle.is_some() || window.floating.is_some()) {
                    window.apply_input_region(&self.windows.compositor, &qh);
                }
                if !visible && window.mapping == Mapping::Mapped {
                    pointer::cancel_focus(&mut self.desktop.seats, id, window);
                }
                if window.mapping == Mapping::Hidden
                    && let Some(registration) = &mut window.registration
                    && !registration.prepare_map()
                {
                    continue;
                }
                window.set_visible(visible, &self.connection, &qh);
                if visible {
                    let mut next = window.deadline();
                    if next.is_some_and(|time| time <= Instant::now()) {
                        window.dirty = true;
                    }
                    if window.dirty {
                        window.present(&qh)?;
                        next = window.deadline();
                    }
                    if let Some(next) = next {
                        deadline = Some(deadline.map_or(next, |d: Instant| d.min(next)));
                    }
                }
            }
            for id in closed {
                if let Some(mut window) = self.desktop.windows.remove(&id) {
                    pointer::cancel_focus(&mut self.desktop.seats, &id, &mut window);
                    window.destroy();
                }
            }
            self.desktop.sync_cursors();
            // A newly mapped parent or a callback can make another window ready.
            if self.windows.revisit.get() || !self.windows.pending.borrow().is_empty() {
                continue;
            }
            if self.desktop.windows.is_empty() {
                self.connection.flush()?;
                return Ok(());
            }
            dispatch_until(
                &mut self.queue,
                &mut self.desktop,
                deadline,
                control.as_ref().map(|(_, h)| h),
            )?;
        }
    }
}

impl Window {
    fn deadline(&self) -> Option<Instant> {
        if self.mapping == Mapping::Mapped && !self.dirty && self.waiting.is_none() {
            self.view.next_redraw()
        } else {
            None
        }
    }

    fn destroy(self) {
        drop(self.role);
        self.fractional.destroy();
        self.viewport.destroy();
        drop(self.floating);
        self.surface.destroy();
        if self.trace {
            eprintln!("closed commits={}", self.commits);
        }
    }
}
// A pending hint can wake an otherwise idle window. Input queued before the
// deadline gets the opportunity to cancel or replace it before presentation.
fn dispatch_until(
    queue: &mut EventQueue<Desktop>,
    desktop: &mut Desktop,
    deadline: Option<Instant>,
    control: Option<&Binding<'_>>,
) -> anyhow::Result<()> {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    use wayland_client::backend::WaylandError;

    if queue.dispatch_pending(desktop)? != 0 {
        return Ok(());
    }
    queue.flush()?;
    if let Some(guard) = queue.prepare_read() {
        let fd = guard.connection_fd();
        let mut fds: smallvec::SmallVec<[_; 2]> =
            smallvec::smallvec![PollFd::new(&fd, PollFlags::IN)];
        if let Some(control) = control {
            fds.push(PollFd::new(control.fd(), PollFlags::IN));
        }
        let timeout = deadline
            .map(|t| Timespec::try_from(t.saturating_duration_since(Instant::now())))
            .transpose()?;
        match poll(&mut fds, timeout.as_ref()) {
            Ok(0) | Err(rustix::io::Errno::INTR) => return Ok(()),
            Err(error) => return Err(error.into()),
            Ok(_) => {}
        }
        if let Some(control) = control
            && !fds[1].revents().is_empty()
        {
            control.drain()?;
        }
        let readable = !fds[0].revents().is_empty();
        drop(fds);
        if readable {
            match guard.read() {
                Ok(_) => {}
                Err(WaylandError::Io(e)) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    queue.dispatch_pending(desktop)?;
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/unit/window_handle.rs"]
mod tests;
