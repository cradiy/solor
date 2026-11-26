use solor::px;
mod activation;
mod application;
mod background_effect;
mod buffer;
mod channel;
pub use channel::ApplicationSender;
mod clipboard;
mod cursor;
mod decorations;
mod floating;
#[cfg(feature = "gpu")]
mod gpu;
mod window_view;
use window_view::WindowView;
mod input_region;
mod keyboard;
mod menu;
mod output;
mod parent;
mod pointer;
mod presentation;
mod role;
pub mod session_lock;
mod stacking;
mod text_input;
mod wake;
pub use application::WaylandApplication;
use application::{Desktop, WindowLife};

use role::Role;

use std::{io, time::Instant};

use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle,
    backend::ObjectId,
    delegate_noop,
    protocol::{wl_buffer, wl_callback, wl_compositor, wl_seat, wl_shm, wl_shm_pool, wl_surface},
};
use wayland_protocols::wp::{
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
        Platform, PlatformWindow, WindowBackground, WindowControl, WindowEvent, WindowHandler,
        WindowOptions,
    },
    pointer::{PointerButton, PointerInput},
    raster::{Clip, Damage, DeviceSize, RasterFrame, RasterScale, Rgba8},
};

/// Wayland platform for XDG windows and desktop layer surfaces with shared-memory buffers.
#[derive(Default)]
pub struct Wayland {
    /// Allocation preference used when creating the shared GPU device.
    pub gpu_memory_policy: solor::platform::GpuMemoryPolicy,
    /// Verifies uploaded shared memory and damage on every submission.
    pub verify_shm: bool,
    /// Logs frame callbacks, buffer availability, commits and presentation feedback to stderr.
    pub trace_submissions: bool,
}

struct Window {
    popup_parent: Option<std::rc::Rc<menu::Parent>>,
    control: Option<crate::surface_control::Binding>,
    preedit_owner: Option<(u32, u64)>,
    registration: Option<parent::Registration>,
    role: Role,
    floating: Option<floating::FloatingPanel>,
    pending_state: Option<solor::platform::WindowState>,
    presentation: Option<wp_presentation::WpPresentation>,
    frame_clock: presentation::FrameClock,
    viewport: Option<wp_viewport::WpViewport>,
    entered_outputs: smallvec::SmallVec<[u32; 2]>,
    scale_120: u32,
    buffer_size: Option<DeviceSize>,
    surface: wl_surface::WlSurface,
    shm: wl_shm::WlShm,
    background: WindowBackground,
    pixel_order: PixelOrder,
    slots: [Option<Buffer>; 2],
    next_slot: usize,
    view: WindowView,
    fractional: Option<wp_fractional_scale_v1::WpFractionalScaleV1>,
    full_damage: bool,
    failure: Option<anyhow::Error>,
    trace: bool,
    size: Size,
    pending_size: Option<Size>,
    mapping: Mapping,
    input_region: input_region::SurfaceInputRegion,
    blur: Option<Box<background_effect::SurfaceBlur>>,
    dirty: bool,
    #[cfg(feature = "gpu")]
    acquire_retry: Option<Instant>,
    waiting: Option<wl_callback::WlCallback>,
    life: std::rc::Rc<WindowLife>,
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

struct Unmapped(ObjectId);

impl Dispatch<wl_callback::WlCallback, Unmapped> for Desktop {
    fn event(
        desktop: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        unmapped: &Unmapped,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(state) = desktop.windows.get_mut(&unmapped.0) else {
            return;
        };
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

pub(crate) fn buffer_size(scale_120: u32, logical: Size) -> io::Result<DeviceSize> {
    budgeted_buffer_size(scale_120, logical)?
        .ok_or_else(|| io::Error::other("buffer exceeds raster budget"))
}

fn budgeted_buffer_size(scale_120: u32, logical: Size) -> io::Result<Option<DeviceSize>> {
    let size = RasterScale(f64::from(scale_120) / 120.0).buffer_size(logical)?;
    let pixels = u64::from(size.width) * u64::from(size.height);
    Ok((pixels <= 4 * 1024 * 1024).then_some(size))
}

impl Platform for Wayland {
    type Target = Rgba8;

    fn run_window(
        &self,
        options: WindowOptions<'_>,
        view: Box<dyn WindowHandler<Self::Target>>,
    ) -> anyhow::Result<()> {
        self.run(|app| app.open_window(options, view).map(drop))
    }
}

impl Window {
    fn apply_input_region(
        &mut self,
        compositor: &wl_compositor::WlCompositor,
        qh: &QueueHandle<Desktop>,
    ) -> bool {
        let (bounds, moved) = match &mut self.floating {
            Some(panel) if matches!(self.mapping, Mapping::Mapped | Mapping::Ready) => {
                (Some(panel.geometry.bounds()), panel.apply_placement())
            }
            Some(_) => return false,
            None => (None, false),
        };
        if moved
            && self.mapping == Mapping::Mapped
            && let Some(parent) = &self.popup_parent
        {
            parent.bounds.set(bounds);
        }
        self.input_region
            .apply(self.mapping, compositor, &self.surface, qh, bounds, moved)
    }

    fn record_failure(&mut self, error: anyhow::Error) {
        self.failure.get_or_insert(error);
        self.life.revisit.set(true);
    }

    fn event(&mut self, event: WindowEvent) {
        self.dispatch(event, None);
    }

    fn dispatch(&mut self, mut event: WindowEvent, press: Option<(wl_seat::WlSeat, u32)>) {
        if self.failure.is_some() || !self.life.is_open() {
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
                if let Some(control) = response.window_control
                    && !self.life.blocked()
                {
                    if control == WindowControl::Drag
                        && let (Some(panel), Some((id, local))) = (&mut self.floating, panel_press)
                    {
                        panel.geometry.begin(id, local);
                    } else {
                        self.window_control(control, press);
                    }
                }
            }
            Err(error) => self.record_failure(error),
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
            if control == WindowControl::Close {
                self.life.close();
            }
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
            WindowControl::Close => self.life.close(),
            WindowControl::Minimize => top.set_minimized(),
            WindowControl::ToggleMaximize if self.life.state().is_some_and(|s| s.maximized) => {
                top.unset_maximized()
            }
            WindowControl::ToggleMaximize => top.set_maximized(),
        }
    }

    fn resize(&mut self, size: Size) {
        self.configure_raster(size, self.scale_120);
    }

    fn configure_raster(&mut self, size: Size, scale_120: u32) {
        if self.size == size && self.scale_120 == scale_120 {
            return;
        }
        self.size = size;
        self.scale_120 = scale_120;
        self.full_damage = true;
        self.dirty = true;
        let target = if let Role::Lock { raster_scale, .. } = &mut self.role {
            self.view
                .lock_buffer_size(scale_120, size)
                .map(|(scale, target)| {
                    *raster_scale = scale;
                    target
                })
        } else {
            self.view.buffer_size(scale_120, size)
        };
        match target {
            Ok(target) => self.buffer_size = target,
            Err(error) => {
                self.buffer_size = None;
                self.record_failure(error.into());
            }
        }
    }

    fn update_layer(&mut self, changes: crate::layer_shell::LayerChanges, visible: bool) {
        let Role::Layer(layer, placement) = &mut self.role else {
            return;
        };
        let active = matches!(self.mapping, Mapping::AwaitingConfigure | Mapping::Ready)
            || (visible && self.mapping == Mapping::Mapped);
        if !placement.update(changes, active.then_some(layer)) {
            return;
        }
        if active {
            self.surface.commit();
        }
        if let Some(panel) = &mut self.floating
            && let Some((width, height)) = changes.size
            && let Some(size) = panel.set_size(width, height)
        {
            self.resize(size);
        }
    }

    fn set_visible(&mut self, visible: bool, connection: &Connection, qh: &QueueHandle<Desktop>) {
        match (visible, self.mapping) {
            (true, Mapping::Hidden) if matches!(self.role, Role::Popup { .. }) => {
                self.mapping = Mapping::Ready;
            }
            (false, Mapping::Ready) if matches!(self.role, Role::Popup { .. }) => {
                self.mapping = Mapping::Hidden;
            }
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
                if matches!(self.role, Role::Popup { .. }) {
                    self.mapping = Mapping::Hidden;
                } else {
                    connection.display().sync(qh, Unmapped(self.surface.id()));
                    self.mapping = Mapping::Unmapping;
                }
                self.waiting = None;
                self.frame_clock.clear_refresh();
                self.full_damage = true;
                self.dirty = true;
            }
            _ => {}
        }
    }

    fn present(
        &mut self,
        size: DeviceSize,
        compositor: &wl_compositor::WlCompositor,
        qh: &QueueHandle<Desktop>,
    ) -> anyhow::Result<()> {
        if !matches!(self.mapping, Mapping::Ready | Mapping::Mapped)
            || !self.dirty
            || self.waiting.is_some()
        {
            return Ok(());
        }
        let scale_120 = match &self.role {
            Role::Lock { raster_scale, .. } => *raster_scale,
            _ => self.scale_120,
        };
        let scale = RasterScale(f64::from(scale_120) / 120.0);
        #[cfg(feature = "gpu")]
        if let WindowView::Gpu(view, _) = &mut self.view {
            let output = view.frame(self.size, scale)?;
            let Some((queue, texture)) = output else {
                // An unmapped surface may never receive a frame callback.
                self.acquire_retry = Some(Instant::now() + std::time::Duration::from_millis(100));
                return Ok(());
            };
            self.waiting = Some(
                self.surface
                    .frame(qh, (self.surface.id(), Some(self.commits + 1))),
            );
            if let Some(p) = &self.presentation {
                p.feedback(&self.surface, qh, (self.surface.id(), self.commits + 1));
            }
            if matches!(self.role, Role::Menu(_) | Role::Popup { .. })
                && self.input_region.changed()
            {
                self.input_region
                    .apply(self.mapping, compositor, &self.surface, qh, None, false);
            }
            set_surface_size(
                self.viewport.as_ref(),
                self.floating.as_ref().map_or(&self.surface, |p| &p.content),
                self.size,
                scale_120,
            );
            queue.present(texture);
            if let Some(panel) = &self.floating {
                if self.mapping == Mapping::Ready {
                    panel.map(&self.surface);
                }
                self.surface.commit();
            }
            self.mark_mapped();
            self.view.frame_submitted()?;
            self.commits += 1;
            self.dirty = false;
            self.full_damage = false;
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
        let region_changed = matches!(self.role, Role::Popup { .. } | Role::Menu(_))
            && self.apply_input_region(compositor, qh);
        let mut frame = match &mut self.view {
            WindowView::Shm(view) => view.frame(self.size, scale)?,
            #[cfg(feature = "gpu")]
            WindowView::Gpu(..) => unreachable!(),
        };
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
                self.waiting = Some(self.surface.frame(qh, (self.surface.id(), None)));
            }
            if callback || region_changed {
                self.surface.commit();
            }
            self.dirty = false;
            self.view.frame_submitted()?;
            return Ok(());
        }
        for entry in &mut self.slots {
            let Some(slot) = entry else { continue };
            if slot.size == size {
                for &area in frame.damage.rects() {
                    slot.invalidate(area);
                }
            } else if !slot.busy {
                *entry = None;
            }
        }
        let reuse = self.slots[index]
            .as_ref()
            .is_some_and(|slot| slot.size == size);
        if !reuse {
            self.slots[index] = Some(Buffer::new(
                size,
                (self.surface.id(), index),
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
        let content = self.floating.as_ref().map_or(&self.surface, |p| &p.content);
        set_surface_size(self.viewport.as_ref(), content, self.size, scale_120);
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
            self.waiting = Some(
                self.surface
                    .frame(qh, (self.surface.id(), Some(self.commits + 1))),
            );
        }
        if let Some(presentation) = &self.presentation {
            presentation.feedback(&self.surface, qh, (self.surface.id(), self.commits + 1));
        }
        if let Some(panel) = &self.floating {
            content.commit();
            if self.mapping == Mapping::Ready {
                panel.map(&self.surface);
            }
        }
        self.surface.commit();
        let queued_ns = self.trace.then(presentation::monotonic_ns);
        slot.busy = true;
        self.mark_mapped();
        self.next_slot = 1 - index;
        self.dirty = false;
        self.full_damage = false;
        self.view.frame_submitted()?;
        self.commits += 1;
        if let Some(queued_ns) = queued_ns {
            eprintln!(
                "commit={} size={}x{} buffer={}x{} scale_120={} slot={} reuse={} audited={} upload_bytes={} upload_us={} elapsed_us={} queued_ns={queued_ns}",
                self.commits,
                self.size.width.get(),
                self.size.height.get(),
                size.width,
                size.height,
                scale_120,
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
    fn mark_mapped(&mut self) {
        if let Some(parent) = &self.popup_parent {
            parent.bounds.set(Some(self.floating.as_ref().map_or_else(
                || {
                    [
                        0,
                        0,
                        self.size.width.get() as i32,
                        self.size.height.get() as i32,
                    ]
                },
                |panel| panel.geometry.bounds(),
            )));
        }
        if self.mapping != Mapping::Mapped {
            self.mapping = Mapping::Mapped;
            if let Some(registration) = &self.registration {
                registration.mapped();
                self.life.revisit.set(true);
            }
            if let Some(target) = &self.life.activation {
                target.activation.apply(&self.surface);
            }
        }
    }
}

fn set_surface_size(
    viewport: Option<&wp_viewport::WpViewport>,
    surface: &wl_surface::WlSurface,
    size: Size,
    scale_120: u32,
) {
    if let Some(viewport) = viewport {
        viewport.set_destination(size.width.get() as i32, size.height.get() as i32);
    } else {
        surface.set_buffer_scale((scale_120 / 120) as i32);
    }
}

delegate_noop!(Desktop: ignore wp_viewporter::WpViewporter);
delegate_noop!(Desktop: ignore wp_viewport::WpViewport);
delegate_noop!(Desktop: ignore wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1);

impl Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, ObjectId> for Desktop {
    fn event(
        desktop: &mut Self,
        _: &wp_fractional_scale_v1::WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        id: &ObjectId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(state) = desktop.windows.get_mut(id) else {
            return;
        };
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            state.configure_raster(state.size, scale);
        }
    }
}

delegate_noop!(Desktop: ignore wl_compositor::WlCompositor);

impl Dispatch<wl_shm::WlShm, ()> for Desktop {
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
        {
            match format {
                wl_shm::Format::Xbgr8888 => state.rgb_opaque = true,
                wl_shm::Format::Abgr8888 => state.rgb_transparent = true,
                _ => return,
            }
            for window in state.windows.values_mut() {
                if format == PixelOrder::Rgb.format(window.background) {
                    window.pixel_order = PixelOrder::Rgb;
                }
            }
        }
    }
}
delegate_noop!(Desktop: ignore wl_shm_pool::WlShmPool);

impl Dispatch<wl_callback::WlCallback, (ObjectId, Option<usize>)> for Desktop {
    fn event(
        desktop: &mut Self,
        callback: &wl_callback::WlCallback,
        _: wl_callback::Event,
        (id, buffer_commit): &(ObjectId, Option<usize>),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(state) = desktop.windows.get_mut(id) else {
            return;
        };
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
        let time = state.frame_clock.sample(state.start.elapsed());
        state.event(WindowEvent::FrameReady(time));
    }
}

impl Dispatch<wl_buffer::WlBuffer, (ObjectId, usize)> for Desktop {
    fn event(
        desktop: &mut Self,
        _: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        (id, index): &(ObjectId, usize),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(state) = desktop.windows.get_mut(id) else {
            return;
        };
        if let wl_buffer::Event::Release = event
            && let Some(slot) = &mut state.slots[*index]
        {
            if state.buffer_size.is_some_and(|size| size != slot.size) {
                state.slots[*index] = None;
            } else {
                slot.busy = false;
            }
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for Desktop {
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

impl Dispatch<xdg_toplevel::XdgToplevel, ObjectId> for Desktop {
    fn event(
        desktop: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        id: &ObjectId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(state) = desktop.windows.get_mut(id) else {
            return;
        };
        match event {
            xdg_toplevel::Event::Configure {
                width,
                height,
                states,
                ..
            } => {
                let flags = states.as_chunks::<4>().0;
                let has = |s: xdg_toplevel::State| flags.contains(&(s as u32).to_ne_bytes());
                state.pending_state = Some(solor::platform::WindowState {
                    active: has(xdg_toplevel::State::Activated),
                    maximized: has(xdg_toplevel::State::Maximized),
                    fullscreen: has(xdg_toplevel::State::Fullscreen),
                });
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
            xdg_toplevel::Event::Close if !state.life.blocked() => state.life.close(),
            _ => {}
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ObjectId> for Desktop {
    fn event(
        desktop: &mut Self,
        surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        id: &ObjectId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(state) = desktop.windows.get_mut(id) else {
            return;
        };
        if let xdg_surface::Event::Configure { serial } = event {
            if !state.life.is_open() {
                return;
            }
            surface.ack_configure(serial);
            state.mapping.configure();

            let (decorations_configured, decoration_mode) = match &mut state.role {
                Role::Toplevel {
                    decorations: Some(d),
                    ..
                } => d.take_configure(),
                Role::Menu(menu) => {
                    menu.finish_configure();
                    (false, None)
                }
                _ => (false, None),
            };
            if let Some(mode) = decoration_mode {
                let csd = matches!(
                    mode,
                    wayland_protocols::xdg::decoration::zv1::client::zxdg_toplevel_decoration_v1::Mode::ClientSide
                );
                if state.life.set_client_decorations(csd) {
                    state.dirty = true;
                    state.full_damage = true;
                }
            }
            if let Some(size) = state.pending_size.take() {
                state.resize(size);
            }
            if let Some(confirmed) = state.pending_state.take()
                && state.life.confirm_state(confirmed)
            {
                state.event(WindowEvent::StateChanged(confirmed));
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
