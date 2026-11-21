use super::{
    DamageAudit, Mapping, Wayland, Window,
    buffer::PixelOrder,
    buffer_size, floating, pointer,
    role::{Request, Role},
};
use crate::layer_shell::{Binding, LayerShellOptions};
use solor::{
    platform::{WindowBackground, WindowHandler, WindowOptions},
    raster::Rgba8,
};
use std::{collections::HashMap, io, time::Instant};
use wayland_client::{
    Connection, EventQueue, Proxy,
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

/// Ordinary windows opened on one connection and dispatched on the calling thread.
pub struct WaylandApplication {
    connection: Connection,
    queue: EventQueue<Desktop>,
    desktop: Desktop,
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
        setup: impl FnOnce(&mut WaylandApplication) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        let mut app = WaylandApplication::connect(self)?;
        setup(&mut app)?;
        app.dispatch(None)
    }

    /// Runs a desktop layer surface until the compositor or the application closes it.
    pub fn run_layer_surface(
        &self,
        options: LayerShellOptions<'_>,
        view: Box<dyn WindowHandler<Rgba8>>,
    ) -> anyhow::Result<()> {
        let control = options.handle.map(|h| h.claim()).transpose()?;
        let mut app = WaylandApplication::connect(self)?;
        let id = app.open(Request::Layer(options), view)?;
        app.dispatch(control.map(|control| (id, control)))
    }
}

impl WaylandApplication {
    fn connect(host: &Wayland) -> anyhow::Result<Self> {
        let connection = Connection::connect_to_env()?;
        let (globals, queue) = registry_queue_init::<Desktop>(&connection)?;
        let qh = queue.handle();
        let mut desktop = Desktop {
            cursor_shapes: globals.bind(&qh, 1..=1, ()).ok(),
            ..Desktop::default()
        };
        for global in globals.contents().clone_list() {
            if global.interface == "wl_seat" {
                desktop.add_seat(globals.registry(), global.name, global.version, &qh);
            }
        }
        Ok(Self {
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
            connection,
            globals,
            queue,
            desktop,
        })
    }

    /// Adds an independently sized and rendered XDG toplevel to this application.
    pub fn open_window(
        &mut self,
        options: WindowOptions<'_>,
        view: Box<dyn WindowHandler<Rgba8>>,
    ) -> anyhow::Result<()> {
        self.open(Request::Toplevel(options), view).map(|_| ())
    }

    fn open(
        &mut self,
        request: Request<'_>,
        view: Box<dyn WindowHandler<Rgba8>>,
    ) -> anyhow::Result<ObjectId> {
        let surface = self.compositor.create_surface(&self.queue.handle(), ());
        let id = surface.id();
        match self.create_window(&surface, request, view) {
            Ok(window) => {
                self.desktop.windows.insert(id.clone(), window);
                Ok(id)
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
        mut view: Box<dyn WindowHandler<Rgba8>>,
    ) -> anyhow::Result<Window> {
        let qh = self.queue.handle();
        let background = match &request {
            Request::Layer(options) => options.background,
            Request::Toplevel(options) => options.background,
        };
        let (role, size) = request.create(&self.globals, &qh, surface)?;
        buffer_size(120, size)?;
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
                &qh,
            )?),
            _ => None,
        };
        let content = floating.as_ref().map_or(surface, |p| &p.content);
        let viewport = self.viewporter.get_viewport(content, &qh, ());

        Ok(Window {
            role,
            floating,
            maximized: false,
            pending_maximized: false,
            presentation: self.presentation.clone(),
            fractional: self
                .scale_manager
                .get_fractional_scale(surface, &qh, surface.id()),
            viewport,
            scale_120: 120,
            surface: surface.clone(),
            shm: self.shm.clone(),
            background,
            pixel_order: self.desktop.order(background),
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
            closed: false,
            commits: 0,
            audit: self.verify_shm.then(DamageAudit::default),
            start: Instant::now(),
        })
    }

    fn dispatch(&mut self, control: Option<(ObjectId, Binding<'_>)>) -> anyhow::Result<()> {
        let qh = self.queue.handle();
        loop {
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
                    .filter(|_| !window.closed)
                else {
                    closed.push(id.clone());
                    continue;
                };
                if let Some(handle) = handle {
                    window.input_region.update(handle.take_input_region());
                }
                if visible && (handle.is_some() || window.floating.is_some()) {
                    window.apply_input_region(&self.compositor, &qh);
                }
                if !visible && window.mapping == Mapping::Mapped {
                    pointer::cancel_focus(&mut self.desktop.seats, id, window);
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
            if self.desktop.windows.is_empty() {
                self.connection.flush()?;
                return Ok(());
            }
            self.desktop.sync_cursors();
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
