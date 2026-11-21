use solor::px;
mod buffer;
mod decorations;
mod floating;
mod input_region;
mod keyboard;
mod pointer;
mod presentation;
mod role;
mod stacking;

use crate::layer_shell::{Binding, LayerShellOptions};
use role::{Request, Role};

use std::{io, time::Instant};

use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle, delegate_noop,
    globals::registry_queue_init,
    protocol::{wl_buffer, wl_callback, wl_compositor, wl_seat, wl_shm, wl_shm_pool, wl_surface},
};
use wayland_protocols::wp::{
    cursor_shape::v1::client::wp_cursor_shape_manager_v1::WpCursorShapeManagerV1,
    fractional_scale::v1::client::{wp_fractional_scale_manager_v1, wp_fractional_scale_v1},
    presentation_time::client::wp_presentation,
    viewporter::client::{wp_viewport, wp_viewporter},
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use buffer::{Buffer, PixelOrder};
use solor::{
    ResizeEdge,
    layout::Size,
    platform::{
        Platform, WindowBackground, WindowControl, WindowEvent, WindowHandler, WindowOptions,
    },
    pointer::{PointerButton, PointerInput},
    raster::{Clip, Damage, DeviceSize, RasterFrame, RasterScale, Rgba8},
};

/// Wayland platform for XDG windows and desktop layer surfaces with shared-memory buffers.
#[derive(Default)]
pub struct Wayland {
    /// Verifies uploaded shared memory and damage on every submission.
    pub verify_shm: bool,
    /// Logs frame callbacks, buffer availability, commits and presentation feedback to stderr.
    pub trace_submissions: bool,
}

struct Window {
    role: Role,
    floating: Option<floating::FloatingPanel>,
    maximized: bool,
    pending_maximized: bool,
    presentation: Option<wp_presentation::WpPresentation>,
    viewport: wp_viewport::WpViewport,
    scale_120: u32,
    surface: wl_surface::WlSurface,
    shm: wl_shm::WlShm,
    background: WindowBackground,
    pixel_order: PixelOrder,
    slots: [Option<Buffer>; 2],
    next_slot: usize,
    view: Box<dyn WindowHandler<Rgba8>>,
    seats: Vec<pointer::Seat>,
    cursor_shapes: Option<WpCursorShapeManagerV1>,
    full_damage: bool,
    failure: Option<anyhow::Error>,
    trace: bool,
    size: Size,
    pending_size: Option<Size>,
    mapping: Mapping,
    input_region: input_region::SurfaceInputRegion,
    dirty: bool,
    waiting: Option<wl_callback::WlCallback>,
    closed: bool,
    commits: usize,
    audit: Option<DamageAudit>,
    start: Instant,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mapping {
    Hidden,
    AwaitingConfigure,
    Ready,
    Mapped,
    Unmapping,
}

impl Mapping {
    fn configure(&mut self) -> bool {
        if *self == Self::AwaitingConfigure {
            *self = Self::Ready;
        }
        matches!(self, Self::Ready | Self::Mapped)
    }
}

struct Unmapped;

impl Dispatch<wl_callback::WlCallback, Unmapped> for Window {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        _: &Unmapped,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.mapping = Mapping::Hidden;
    }
}

/// The compositor may keep any pixel outside the reported buffer damage.
#[derive(Default)]
struct DamageAudit {
    previous: Option<(DeviceSize, Vec<u8>)>,
}

impl DamageAudit {
    fn check(&mut self, current: &RasterFrame<'_>) -> io::Result<()> {
        let damage = current.damage.rects();
        let canvas = Clip::canvas(current.size);
        if damage.iter().any(|&area| area.intersect(canvas) != area) {
            return Err(io::Error::other("damage outside the submitted buffer"));
        }
        if !damage.contains(&canvas) {
            let previous = self
                .previous
                .as_ref()
                .filter(|old| old.0 == current.size)
                .ok_or_else(|| io::Error::other("new target requires full damage"))?;
            let stride = current.size.width as usize * 4;
            for (y, (old, new)) in previous
                .1
                .chunks_exact(stride)
                .zip(current.rgba.chunks_exact(stride))
                .enumerate()
            {
                if old != new
                    && old
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .zip(new.as_chunks::<4>().0)
                        .enumerate()
                        .any(|(x, (a, b))| {
                            a != b
                                && !damage.iter().any(|r| {
                                    (r.left..r.right).contains(&(x as i64))
                                        && (r.top..r.bottom).contains(&(y as i64))
                                })
                        })
                {
                    return Err(io::Error::other(
                        "changed pixel outside reported buffer damage",
                    ));
                }
            }
        }
        self.previous = Some((current.size, current.rgba.to_vec()));
        Ok(())
    }
}

fn buffer_size(scale_120: u32, logical: Size) -> io::Result<DeviceSize> {
    let size = RasterScale(f64::from(scale_120) / 120.0).buffer_size(logical)?;
    if scale_120 == 0 || u64::from(size.width) * u64::from(size.height) > 4 * 1024 * 1024 {
        return Err(io::Error::other(
            "scale or buffer size outside raster budget",
        ));
    }
    Ok(size)
}

impl Platform for Wayland {
    type Target = Rgba8;

    fn run_window(
        &self,
        options: WindowOptions<'_>,
        view: Box<dyn WindowHandler<Self::Target>>,
    ) -> anyhow::Result<()> {
        self.run_surface(Request::Toplevel(options), view)
    }
}

impl Wayland {
    /// Runs a desktop layer surface until the compositor or the application closes it.
    pub fn run_layer_surface(
        &self,
        options: LayerShellOptions<'_>,
        view: Box<dyn WindowHandler<Rgba8>>,
    ) -> anyhow::Result<()> {
        self.run_surface(Request::Layer(options), view)
    }

    fn run_surface(
        &self,
        request: Request<'_>,
        view: Box<dyn WindowHandler<Rgba8>>,
    ) -> anyhow::Result<()> {
        let (background, handle) = match &request {
            Request::Layer(options) => (options.background, options.handle),
            Request::Toplevel(options) => (options.background, None),
        };
        let control = handle.map(|handle| handle.claim()).transpose()?;
        let start = Instant::now();
        let connection = Connection::connect_to_env()?;
        let (globals, mut queue) = registry_queue_init::<Window>(&connection)?;
        let qh = queue.handle();
        let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 4..=4, ())?;
        let shm = globals.bind(&qh, 1..=1, ())?;
        let surface = compositor.create_surface(&qh, ());
        let viewporter: wp_viewporter::WpViewporter = globals.bind(&qh, 1..=1, ())?;
        let manager: wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1 =
            globals.bind(&qh, 1..=1, ())?;
        let fractional = manager.get_fractional_scale(&surface, &qh, ());
        let (role, size) = request.create(&globals, &qh, &surface)?;
        buffer_size(120, size)?;
        let floating = match &role {
            Role::Layer(_, p) if p.movable => Some(floating::FloatingPanel::new(
                &globals,
                &compositor,
                &shm,
                &viewporter,
                &surface,
                floating::Geometry::new(size, p.anchor, p.margin),
                &qh,
            )?),
            _ => None,
        };
        let content = floating.as_ref().map_or(&surface, |p| &p.content);
        let viewport = viewporter.get_viewport(content, &qh, ());
        let presentation = if self.trace_submissions {
            globals.bind(&qh, 1..=1, ()).ok()
        } else {
            None
        };
        let mut window = Window {
            role,
            floating,
            maximized: false,
            pending_maximized: false,
            presentation,
            viewport,
            scale_120: 120,
            surface,
            shm,
            background,
            pixel_order: PixelOrder::Bgr,
            slots: [None, None],
            next_slot: 0,
            view,
            seats: Vec::new(),
            cursor_shapes: globals.bind(&qh, 1..=1, ()).ok(),
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
            start,
        };
        for global in globals.contents().clone_list() {
            if global.interface == "wl_seat" {
                window.add_seat(globals.registry(), global.name, global.version, &qh);
            }
        }
        loop {
            if let Some(error) = window.failure.take() {
                return Err(error);
            }
            let Some(visible) = control.as_ref().map_or(Some(true), Binding::visibility) else {
                break;
            };
            if window.closed {
                break;
            }
            if visible
                && window.mapping == Mapping::Hidden
                && let Some(control) = &control
            {
                window.input_region.update(control.take_input_region());
            }
            if visible && (control.is_some() || window.floating.is_some()) {
                window.apply_input_region(&compositor, &qh);
            }
            window.set_visible(visible, &connection, &qh);
            if visible {
                window.present(&qh)?;
            }

            let deadline = if visible
                && window.mapping == Mapping::Mapped
                && !window.dirty
                && window.waiting.is_none()
            {
                window.view.next_redraw()
            } else {
                None
            };
            if deadline.is_some() || control.is_some() {
                dispatch_until(&mut queue, &mut window, deadline, control.as_ref())?;
                if deadline.is_some() {
                    window.dirty |= window
                        .view
                        .next_redraw()
                        .is_some_and(|time| time <= Instant::now());
                }
            } else {
                queue.blocking_dispatch(&mut window)?;
            }
        }
        drop(window.role);
        fractional.destroy();
        window.viewport.destroy();
        drop(window.floating);
        window.surface.destroy();
        connection.flush()?;
        if self.trace_submissions {
            eprintln!("closed commits={}", window.commits);
        }
        Ok(())
    }
}

// A pending hint can wake an otherwise idle window. Input queued before the
// deadline gets the opportunity to cancel or replace it before presentation.
fn dispatch_until(
    queue: &mut EventQueue<Window>,
    window: &mut Window,
    deadline: Option<Instant>,
    control: Option<&Binding<'_>>,
) -> anyhow::Result<()> {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    use wayland_client::backend::WaylandError;

    if queue.dispatch_pending(window)? != 0 {
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
            window.input_region.update(control.take_input_region());
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
    queue.dispatch_pending(window)?;
    Ok(())
}

impl Window {
    fn apply_input_region(
        &mut self,
        compositor: &wl_compositor::WlCompositor,
        qh: &QueueHandle<Self>,
    ) {
        let (bounds, moved) = match &mut self.floating {
            Some(panel) if matches!(self.mapping, Mapping::Mapped | Mapping::Ready) => {
                (Some(panel.geometry.bounds()), panel.apply_placement())
            }
            Some(_) => return,
            None => (None, false),
        };
        self.input_region
            .apply(self.mapping, compositor, &self.surface, qh, bounds, moved);
    }

    fn event(&mut self, event: WindowEvent) {
        self.dispatch(event, None);
    }

    fn dispatch(&mut self, mut event: WindowEvent, press: Option<(wl_seat::WlSeat, u32)>) {
        if self.failure.is_some() || self.closed {
            return;
        }
        let mut panel_press = None;
        if let (Some(panel), WindowEvent::Pointer(input)) = (&mut self.floating, &mut event) {
            panel.pointer(input);
            if let PointerInput::Down(p, PointerButton::Primary) = input {
                panel_press = Some((p.id, p.position));
            }
        }
        match self.view.event(event) {
            Ok(response) => {
                self.dirty |= response.redraw;
                if let Some(control) = response.window_control {
                    if control == WindowControl::Drag
                        && let (Some(panel), Some((id, local))) = (&mut self.floating, panel_press)
                    {
                        panel.geometry.begin(id, local);
                    } else {
                        self.window_control(control, press);
                    }
                }
            }
            Err(error) => self.failure = Some(error),
        }
    }

    fn window_control(&mut self, control: WindowControl, press: Option<(wl_seat::WlSeat, u32)>) {
        let Role::Toplevel {
            top,
            decorations,
            resizable,
            ..
        } = &mut self.role
        else {
            self.closed |= control == WindowControl::Close;
            return;
        };
        match control {
            WindowControl::Resize(_) | WindowControl::ToggleMaximize if !*resizable => {}
            WindowControl::Drag => {
                if let Some((seat, serial)) = press {
                    top._move(&seat, serial);
                }
            }
            WindowControl::Resize(edge) => {
                if let Some((seat, serial)) = press {
                    use xdg_toplevel::ResizeEdge as XdgEdge;
                    let edge = match edge {
                        ResizeEdge::Top => XdgEdge::Top,
                        ResizeEdge::TopRight => XdgEdge::TopRight,
                        ResizeEdge::Right => XdgEdge::Right,
                        ResizeEdge::BottomRight => XdgEdge::BottomRight,
                        ResizeEdge::Bottom => XdgEdge::Bottom,
                        ResizeEdge::BottomLeft => XdgEdge::BottomLeft,
                        ResizeEdge::Left => XdgEdge::Left,
                        ResizeEdge::TopLeft => XdgEdge::TopLeft,
                    };
                    top.resize(&seat, serial, edge);
                }
            }
            WindowControl::ToggleDecorations => {
                if let Some(decorations) = decorations {
                    decorations.toggle();
                }
            }
            WindowControl::Close => self.closed = true,
            WindowControl::Minimize => top.set_minimized(),
            WindowControl::ToggleMaximize if self.maximized => top.unset_maximized(),
            WindowControl::ToggleMaximize => top.set_maximized(),
        }
    }

    fn resize(&mut self, size: Size) {
        if self.size != size {
            self.size = size;
            self.full_damage = true;
            self.dirty = true;
        }
    }

    fn set_visible(&mut self, visible: bool, connection: &Connection, qh: &QueueHandle<Self>) {
        match (visible, self.mapping) {
            (true, Mapping::Hidden) => {
                if let Role::Layer(layer, placement) = &self.role {
                    placement.apply(layer, self.commits != 0);
                }
                self.surface.commit();
                self.mapping = Mapping::AwaitingConfigure;
            }
            (false, Mapping::Mapped) => {
                self.surface.attach(None, 0, 0);
                self.surface.commit();
                // Drain events issued before unmap before accepting a new configure.
                connection.display().sync(qh, Unmapped);
                self.mapping = Mapping::Unmapping;
                self.waiting = None;
                self.full_damage = true;
                self.dirty = true;
                self.cancel_pointers();
            }
            _ => {}
        }
    }

    fn present(&mut self, qh: &QueueHandle<Self>) -> anyhow::Result<()> {
        if !matches!(self.mapping, Mapping::Ready | Mapping::Mapped)
            || !self.dirty
            || self.waiting.is_some()
        {
            return Ok(());
        }
        let Some(index) = [self.next_slot, 1 - self.next_slot]
            .into_iter()
            .find(|&i| self.slots[i].as_ref().is_none_or(|slot| !slot.busy))
        else {
            if self.trace {
                eprintln!("waiting_for_buffer_after={}", self.commits);
            }
            return Ok(());
        };
        let size = buffer_size(self.scale_120, self.size)?;
        let scale = RasterScale(f64::from(self.scale_120) / 120.0);
        let mut frame = self.view.frame(self.size, scale)?;
        frame.validate(size)?;
        if self.full_damage {
            frame.damage = Damage::from_rect(Clip::canvas(size));
        }
        if let Some(audit) = &mut self.audit {
            audit.check(&frame)?;
        }
        let callback = frame.request_callback;
        if frame.damage.is_empty() {
            if callback {
                // A frame request takes effect at commit, even with no new buffer.
                self.waiting = Some(self.surface.frame(qh, None));
                self.surface.commit();
            }
            self.dirty = false;
            self.view.frame_submitted()?;
            self.sync_cursors();
            return Ok(());
        }
        for slot in self.slots.iter_mut().flatten().filter(|s| s.size == size) {
            for &area in frame.damage.rects() {
                slot.invalidate(area);
            }
        }
        let reuse = self.slots[index]
            .as_ref()
            .is_some_and(|slot| slot.size == size);
        if !reuse {
            self.slots[index] = Some(Buffer::new(
                size,
                index,
                self.pixel_order,
                self.background,
                &self.shm,
                qh,
            )?);
        }
        let slot = self.slots[index]
            .as_mut()
            .expect("initialized free shm slot");
        let upload_start = Instant::now();
        let uploaded = slot.upload(&frame)?;
        let upload_us = upload_start.elapsed().as_micros();
        if self.audit.is_some() {
            slot.verify(&frame)?;
        }
        self.viewport
            .set_destination(self.size.width.get() as i32, self.size.height.get() as i32);
        let content = self.floating.as_ref().map_or(&self.surface, |p| &p.content);
        content.attach(Some(&slot.handle), 0, 0);
        for &area in frame.damage.rects() {
            content.damage_buffer(
                area.left as i32,
                area.top as i32,
                (area.right - area.left) as i32,
                (area.bottom - area.top) as i32,
            );
        }
        if callback {
            self.waiting = Some(self.surface.frame(qh, Some(self.commits + 1)));
        }
        if let Some(presentation) = &self.presentation {
            presentation.feedback(&self.surface, qh, self.commits + 1);
        }
        if let Some(panel) = &self.floating {
            content.commit();
            if self.mapping == Mapping::Ready {
                panel.map(&self.surface);
            }
        }
        self.surface.commit();
        self.mapping = Mapping::Mapped;
        let queued_ns = self.trace.then(presentation::monotonic_ns);
        slot.busy = true;
        self.next_slot = 1 - index;
        self.dirty = false;
        self.full_damage = false;
        self.view.frame_submitted()?;
        self.sync_cursors();
        self.commits += 1;
        if let Some(queued_ns) = queued_ns {
            eprintln!(
                "commit={} size={}x{} buffer={}x{} scale_120={} slot={} reuse={} audited={} upload_bytes={} upload_us={} elapsed_us={} queued_ns={queued_ns}",
                self.commits,
                self.size.width.get(),
                self.size.height.get(),
                size.width,
                size.height,
                self.scale_120,
                index,
                reuse,
                self.audit.is_some(),
                uploaded,
                upload_us,
                self.start.elapsed().as_micros()
            );
        }
        Ok(())
    }
}

delegate_noop!(Window: ignore wp_viewporter::WpViewporter);
delegate_noop!(Window: ignore wp_viewport::WpViewport);
delegate_noop!(Window: ignore wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1);

impl Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, ()> for Window {
    fn event(
        state: &mut Self,
        _: &wp_fractional_scale_v1::WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event
            && state.scale_120 != scale
        {
            state.scale_120 = scale;
            state.full_damage = true;
            state.dirty = true;
        }
    }
}

delegate_noop!(Window: ignore wl_compositor::WlCompositor);
delegate_noop!(Window: ignore wl_surface::WlSurface);
impl Dispatch<wl_shm::WlShm, ()> for Window {
    fn event(
        state: &mut Self,
        _: &wl_shm::WlShm,
        event: wl_shm::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_shm::Event::Format {
            format: wayland_client::WEnum::Value(format),
        } = event
            && format == PixelOrder::Rgb.format(state.background)
        {
            state.pixel_order = PixelOrder::Rgb;
        }
    }
}
delegate_noop!(Window: ignore wl_shm_pool::WlShmPool);

impl Dispatch<wl_callback::WlCallback, Option<usize>> for Window {
    fn event(
        state: &mut Self,
        callback: &wl_callback::WlCallback,
        _: wl_callback::Event,
        buffer_commit: &Option<usize>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if state.waiting.as_ref() != Some(callback) {
            return;
        }
        if state.trace {
            let dispatched_ns = presentation::monotonic_ns();
            match buffer_commit {
                Some(commit) => eprintln!("frame_done={commit} dispatched_ns={dispatched_ns}"),
                None => eprintln!("frame_done=callback_only dispatched_ns={dispatched_ns}"),
            }
        }
        state.waiting = None;
        state.event(WindowEvent::FrameReady(state.start.elapsed()));
    }
}

impl Dispatch<wl_buffer::WlBuffer, usize> for Window {
    fn event(
        state: &mut Self,
        _: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event
            && let Some(slot) = &mut state.slots[*index]
        {
            slot.busy = false;
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for Window {
    fn event(
        _: &mut Self,
        shell: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            shell.pong(serial);
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for Window {
    fn event(
        state: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_toplevel::Event::Configure {
                width,
                height,
                states,
                ..
            } => {
                state.pending_maximized = states
                    .as_chunks::<4>()
                    .0
                    .contains(&(xdg_toplevel::State::Maximized as u32).to_ne_bytes());
                state.pending_size = Some(Size {
                    width: if width > 0 {
                        px(width as f32)
                    } else {
                        state.size.width
                    },
                    height: if height > 0 {
                        px(height as f32)
                    } else {
                        state.size.height
                    },
                });
            }
            xdg_toplevel::Event::Close => state.closed = true,
            _ => {}
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for Window {
    fn event(
        state: &mut Self,
        surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            surface.ack_configure(serial);
            state.mapping.configure();
            state.maximized = state.pending_maximized;
            let Role::Toplevel { decorations, .. } = &mut state.role else {
                return;
            };
            let decorations_configured = decorations.as_mut().is_some_and(|d| d.take_configure());
            if let Some(size) = state.pending_size.take() {
                state.resize(size);
            }
            if decorations_configured && state.commits != 0 && !state.full_damage {
                // Apply a decoration-only configure even when the retained view is idle.
                state.surface.commit();
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/damage.rs"]
mod tests;
