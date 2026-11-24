use super::{
    DamageAudit, Mapping, Wayland, Window,
    activation::{Activation, ActivationTarget},
    buffer::PixelOrder,
    buffer_size, floating,
    parent::Toplevels,
    pointer,
    role::{Request, Role},
};
use crate::{layer_shell::LayerShellOptions, popup::PopupHandle, surface_control::Binding};
use rustix::event::PollFlags;
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
    backend::{ObjectId, WaylandError},
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
    pub(super) text_inputs: Option<super::text_input::Manager>,
    pub(super) activation: Option<Rc<Activation>>,
    pub(super) popup_only: bool,
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
            if let Some(source) = &parent.activation {
                let modal = self.modal_activation_window();
                if let Some(target) = &modal.as_deref().unwrap_or(self).activation {
                    source.redirect_pending(target);
                }
            }
        }
    }

    /// Follow the newest open modal at each level to a window that accepts input.
    fn modal_activation_window(&self) -> Option<Rc<Self>> {
        let mut window = self.modal_children.borrow().last()?.upgrade()?;
        loop {
            let next = window
                .modal_children
                .borrow()
                .last()
                .and_then(Weak::upgrade);
            match next {
                Some(child) => window = child,
                None => return Some(window),
            }
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
        let modal = self.modal_activation_window();
        if let Some(target) = &modal.as_deref().unwrap_or(self).activation {
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
    globals: Option<GlobalList>,
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
        let mut events = EventLoop::new(self, Connection::connect_to_env()?, false)?;
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
        let mut events = EventLoop::new(self, Connection::connect_to_env()?, false)?;
        let (id, _) = events.windows.open(Request::Layer(options), |_| Ok(view))?;
        events.dispatch(control.map(|control| (id, control)))
    }

    /// Runs a transparent, client-sized popup on the supplied connection. The
    /// host keeps reading the connection on its own thread. The factory assigns
    /// a role that can present without a configure handshake; its return value
    /// owns that role and must destroy it on drop.
    pub fn run_popup<R: 'static>(
        &self,
        connection: Connection,
        handle: &PopupHandle,
        assign: impl FnOnce(&wl_surface::WlSurface) -> anyhow::Result<R>,
        view: Box<dyn WindowHandler<Rgba8>>,
    ) -> anyhow::Result<()> {
        let control = handle.0.claim()?;
        let mut events = EventLoop::new(self, connection, true)?;
        let request = Request::Popup(Box::new(move |surface| Ok(Box::new(assign(surface)?))));
        let (id, _) = events.windows.open(request, |_| Ok(view))?;
        events.dispatch(Some((id, control)))
    }
}

impl EventLoop {
    fn new(host: &Wayland, connection: Connection, guest: bool) -> anyhow::Result<Self> {
        let (globals, queue) = registry_queue_init::<Desktop>(&connection)?;
        let qh = queue.handle();
        let mut desktop = Desktop {
            popup_only: guest,
            text_inputs: if guest {
                None
            } else {
                globals.bind(&qh, 1..=1, ()).ok()
            },
            activation: if guest {
                None
            } else {
                Activation::bind(&globals, &qh)
            },
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
            globals: Some(globals),
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
            Request::Layer(_) | Request::Popup(_) => None,
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
            Request::Popup(_) => WindowBackground::Transparent,
            Request::Layer(options) => options.background,
            Request::Toplevel(options) => options.background,
        };
        let parent = match &request {
            Request::Toplevel(options) => options.parent,
            Request::Layer(_) | Request::Popup(_) => None,
        };
        let activation = match (&request, &self.activation) {
            (Request::Toplevel(options), Some(activation)) => Some(ActivationTarget {
                activation: activation.clone(),
                id: surface.id(),
                app_id: options.app_id.map(str::to_owned),
            }),
            _ => None,
        };
        let globals = self.globals.as_ref().expect("live window factory");
        let (role, size) = request.create(globals, qh, surface)?;
        let buffer_size = buffer_size(120, size)?;
        let life = WindowLife::new(activation, self.revisit.clone(), modal_parent);
        let target = Rc::downgrade(&life);
        let handle = WindowHandle::new(target);
        let registration = match &role {
            Role::Toplevel { top, .. } => {
                Some(self.toplevels.register(handle.clone(), &life, top, parent))
            }
            Role::Layer(..) | Role::Popup { .. } => None,
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
                globals,
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
            preedit_owner: None,
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
            buffer_size: Some(buffer_size),
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
        let wake = super::wake::EventWake::new()?;
        let waker = std::task::Waker::from(wake.clone());
        loop {
            self.windows.revisit.set(false);
            for mut window in self.windows.pending.take() {
                window.pixel_order = self.desktop.order(window.background);
                self.desktop.windows.insert(window.surface.id(), window);
            }
            if self.desktop.popup_only {
                let mut cx = std::task::Context::from_waker(&waker);
                if let std::task::Poll::Ready(result) =
                    self.queue.poll_dispatch_pending(&mut cx, &mut self.desktop)
                {
                    match result? {}
                }
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
                if window.life.is_open() {
                    window.view.poll_tasks(&waker)?;
                }
                let update = handle.map(Binding::take);
                let Some(visible) = update
                    .as_ref()
                    .map_or(Some(true), |u| u.visible)
                    .filter(|_| window.life.is_open())
                else {
                    closed.push(id.clone());
                    continue;
                };
                pointer::cancel_modal_gestures(&mut self.desktop.seats, id, window);
                if let Some(update) = update {
                    if let Some(size) = update.size {
                        window.resize(size);
                    }
                    window.input_region.update(update.region);
                }
                if visible
                    && !self.desktop.popup_only
                    && (handle.is_some() || window.floating.is_some())
                    && window.apply_input_region(&self.windows.compositor, &qh)
                    && window.mapping == Mapping::Mapped
                {
                    window.surface.commit();
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
                    if let Some(next) = super::keyboard::repeat_keys(
                        &mut self.desktop.seats,
                        id,
                        window,
                        Instant::now(),
                    ) {
                        deadline = Some(deadline.map_or(next, |d: Instant| d.min(next)));
                    }
                    if self.desktop.popup_only && window.input_region.changed() {
                        window.dirty = true;
                    }
                    if let Some(next) = window.present_if_due(&self.windows.compositor, &qh)? {
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
            self.desktop.sync_text_inputs();
            // A newly mapped parent or a callback can make another window ready.
            if self.windows.revisit.get() || !self.windows.pending.borrow().is_empty() {
                continue;
            }
            let write_interest = flush_interest(&self.queue)?;
            if self.desktop.windows.is_empty() {
                return Ok(());
            }
            dispatch_until(
                &mut self.queue,
                &mut self.desktop,
                write_interest,
                deadline,
                &wake,
                control.as_ref().map(|(_, h)| h),
            )?;
        }
    }
}

impl Drop for WindowFactory {
    fn drop(&mut self) {
        self.scale_manager.destroy();
        self.viewporter.destroy();
        if let Some(presentation) = self.presentation.take() {
            presentation.destroy();
        }
        let registry = self.globals.take().expect("live window factory");
        let proxy = registry.registry().clone();
        registry.destroy();
        if let Some(backend) = proxy.backend().upgrade() {
            // These bound protocol versions have no destructor request.
            for id in [self.compositor.id(), self.shm.id()] {
                let _ = backend.destroy_object(&id);
            }
            if proxy.is_alive() {
                let _ = backend.destroy_object(&proxy.id());
            }
            let _ = backend.flush();
        }
    }
}

impl Drop for EventLoop {
    fn drop(&mut self) {
        for (_, window) in self.desktop.windows.drain() {
            window.destroy();
        }
        for window in self.windows.pending.take() {
            window.destroy();
        }
        self.desktop.seats.clear();
        if let Some(manager) = self.desktop.cursor_shapes.take() {
            manager.destroy();
        }
    }
}

impl Window {
    fn present_if_due(
        &mut self,
        compositor: &wl_compositor::WlCompositor,
        qh: &QueueHandle<Desktop>,
    ) -> anyhow::Result<Option<Instant>> {
        let Some(size) = self.buffer_size else {
            return Ok(None);
        };
        let next = self.deadline();
        if next.is_some_and(|deadline| deadline <= Instant::now()) {
            self.dirty = true;
        }
        if self.dirty {
            self.present(size, compositor, qh)?;
            return Ok(self.deadline());
        }
        Ok(next)
    }

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

fn flush_interest(queue: &EventQueue<Desktop>) -> Result<PollFlags, WaylandError> {
    match queue.flush() {
        Ok(()) => Ok(PollFlags::empty()),
        Err(WaylandError::Io(e)) if e.kind() == io::ErrorKind::WouldBlock => Ok(PollFlags::OUT),
        Err(error) => Err(error),
    }
}

// A pending hint can wake an otherwise idle window. Input queued before the
// deadline gets the opportunity to cancel or replace it before presentation.
fn dispatch_until(
    queue: &mut EventQueue<Desktop>,
    desktop: &mut Desktop,
    write_interest: PollFlags,
    deadline: Option<Instant>,
    wake: &super::wake::EventWake,
    control: Option<&Binding<'_>>,
) -> anyhow::Result<()> {
    if desktop.popup_only {
        wait_for_events(queue, write_interest, deadline, wake, control)?;
        return Ok(());
    }
    if queue.dispatch_pending(desktop)? != 0 {
        return Ok(());
    }
    if let Some(guard) = queue.prepare_read()
        && wait_for_events(
            queue,
            PollFlags::IN | write_interest,
            deadline,
            wake,
            control,
        )?
    {
        match guard.read() {
            Ok(_) => {}
            Err(WaylandError::Io(e)) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(error.into()),
        }
    }
    queue.dispatch_pending(desktop)?;
    Ok(())
}

fn wait_for_events(
    queue: &EventQueue<Desktop>,
    interest: PollFlags,
    deadline: Option<Instant>,
    wake: &super::wake::EventWake,
    control: Option<&Binding<'_>>,
) -> anyhow::Result<bool> {
    use rustix::event::{PollFd, Timespec, poll};
    let mut fds: smallvec::SmallVec<[_; 3]> = smallvec::smallvec![
        PollFd::new(queue, interest),
        PollFd::new(&wake.0, PollFlags::IN)
    ];
    if let Some(control) = control {
        fds.push(PollFd::new(control.fd(), PollFlags::IN));
    }
    let timeout = deadline
        .map(|t| Timespec::try_from(t.saturating_duration_since(Instant::now())))
        .transpose()?;
    match poll(&mut fds, timeout.as_ref()) {
        Ok(0) | Err(rustix::io::Errno::INTR) => return Ok(false),
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    if !fds[1].revents().is_empty() {
        wake.drain()?;
    }
    if let Some(control) = control
        && !fds[2].revents().is_empty()
    {
        control.drain()?;
    }

    let ready = fds[0].revents();
    anyhow::ensure!(
        !ready.intersects(PollFlags::HUP | PollFlags::ERR | PollFlags::NVAL),
        "Wayland connection closed"
    );
    if ready.contains(PollFlags::OUT) {
        flush_interest(queue)?;
    }
    Ok(ready.contains(PollFlags::IN))
}

#[cfg(test)]
#[path = "../../tests/unit/window_handle.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/unit/dispatch.rs"]
mod dispatch_tests;

#[cfg(test)]
#[path = "../../tests/unit/host.rs"]
pub(super) mod fixture;
