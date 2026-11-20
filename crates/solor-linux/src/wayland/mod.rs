use solor::px;
mod buffer;
mod decorations;
mod pointer;
mod presentation;

use std::{io, time::Instant};

use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop,
    globals::registry_queue_init,
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
    layout::Size,
    platform::{Platform, WindowControl, WindowEvent, WindowHandler, WindowOptions},
    raster::{Clip, DeviceSize, RasterFrame, RasterScale, Rgba8},
};

/// Wayland platform using XDG shell and shared-memory window buffers.
#[derive(Default)]
pub struct Wayland {
    /// Verifies uploaded shared memory and damage on every submission.
    pub verify_shm: bool,
    /// Logs frame callbacks, buffer availability, commits and presentation feedback to stderr.
    pub trace_submissions: bool,
}

struct Window {
    top: xdg_toplevel::XdgToplevel,
    decorations: Option<decorations::Decorations>,
    maximized: bool,
    pending_maximized: bool,
    presentation: Option<wp_presentation::WpPresentation>,
    viewport: wp_viewport::WpViewport,
    scale_120: u32,
    surface: wl_surface::WlSurface,
    shm: wl_shm::WlShm,
    pixel_order: PixelOrder,
    slots: [Option<Buffer>; 2],
    next_slot: usize,
    view: Box<dyn WindowHandler<Rgba8>>,
    seats: Vec<pointer::Seat>,
    full_damage: bool,
    failure: Option<anyhow::Error>,
    trace: bool,
    size: Size,
    pending_size: Option<Size>,
    configured: bool,
    dirty: bool,
    waiting: bool,
    closed: bool,
    commits: usize,
    audit: Option<DamageAudit>,
    start: Instant,
}

/// The compositor may keep any pixel outside the reported buffer damage.
#[derive(Default)]
struct DamageAudit {
    previous: Option<(DeviceSize, Vec<u8>)>,
}

impl DamageAudit {
    fn check(&mut self, current: &RasterFrame<'_>) -> io::Result<()> {
        let damage = current.damage;
        let canvas = Clip::canvas(current.size);
        if damage.is_some_and(|area| area.empty() || area.intersect(canvas) != area) {
            return Err(io::Error::other("damage outside the submitted buffer"));
        }
        if damage != Some(canvas) {
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
                let matches = match damage {
                    Some(area) if (area.top..area.bottom).contains(&(y as i64)) => {
                        let left = area.left as usize * 4;
                        let right = area.right as usize * 4;
                        old[..left] == new[..left] && old[right..] == new[right..]
                    }
                    _ => old == new,
                };
                if !matches {
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
        let WindowOptions {
            title,
            native_decorations,
            app_id,
            size,
        } = options;
        anyhow::ensure!(
            size.width.is_extent()
                && size.height.is_extent()
                && size.width.get() >= 1.0
                && size.height.get() >= 1.0,
            "invalid initial window size"
        );
        // XDG configure and viewporter destinations use whole logical pixels.
        let size = Size {
            width: px(size.width.get().round()),
            height: px(size.height.get().round()),
        };
        buffer_size(120, size)?;
        let start = Instant::now();
        let connection = Connection::connect_to_env()?;
        let (globals, mut queue) = registry_queue_init::<Window>(&connection)?;
        let qh = queue.handle();
        let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 4..=4, ())?;
        let shm = globals.bind(&qh, 1..=1, ())?;
        let shell: xdg_wm_base::XdgWmBase = globals.bind(&qh, 1..=1, ())?;
        let surface = compositor.create_surface(&qh, ());
        let viewporter: wp_viewporter::WpViewporter = globals.bind(&qh, 1..=1, ())?;
        let manager: wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1 =
            globals.bind(&qh, 1..=1, ())?;
        let viewport = viewporter.get_viewport(&surface, &qh, ());
        let fractional = manager.get_fractional_scale(&surface, &qh, ());
        let role = shell.get_xdg_surface(&surface, &qh, ());
        let top = role.get_toplevel(&qh, ());
        let decorations = decorations::Decorations::bind(&globals, &qh, &top, native_decorations)?;
        top.set_title(title.into());
        if let Some(app_id) = app_id {
            top.set_app_id(app_id.into());
        }
        surface.commit();

        let presentation = if self.trace_submissions {
            globals.bind(&qh, 1..=1, ()).ok()
        } else {
            None
        };
        let mut window = Window {
            top,
            decorations,
            maximized: false,
            pending_maximized: false,
            presentation,
            viewport,
            scale_120: 120,
            surface,
            shm,
            pixel_order: PixelOrder::Bgr,
            slots: [None, None],
            next_slot: 0,
            view,
            seats: Vec::new(),
            full_damage: true,
            failure: None,
            trace: self.trace_submissions,
            size,
            pending_size: None,
            configured: false,
            dirty: true,
            waiting: false,
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
            queue.blocking_dispatch(&mut window)?;
            if let Some(error) = window.failure.take() {
                return Err(error);
            }
            if window.closed {
                break;
            }
            window.present(&qh)?;
        }
        drop(window.decorations.take());
        window.top.destroy();
        role.destroy();
        fractional.destroy();
        window.viewport.destroy();
        window.surface.destroy();
        shell.destroy();
        connection.flush()?;
        if self.trace_submissions {
            eprintln!("closed commits={}", window.commits);
        }
        Ok(())
    }
}

impl Window {
    fn event(&mut self, event: WindowEvent) {
        self.dispatch(event, None);
    }

    fn dispatch(&mut self, event: WindowEvent, press: Option<(wl_seat::WlSeat, u32)>) {
        if self.failure.is_some() || self.closed {
            return;
        }
        match self.view.event(event) {
            Ok(response) => {
                self.dirty |= response.redraw;
                match response.window_control {
                    Some(WindowControl::Drag) => {
                        if let Some((seat, serial)) = press {
                            self.top._move(&seat, serial);
                        }
                    }
                    Some(WindowControl::ToggleDecorations) => {
                        if let Some(decorations) = &mut self.decorations {
                            decorations.toggle();
                        }
                    }
                    Some(WindowControl::Close) => self.closed = true,
                    Some(WindowControl::Minimize) => self.top.set_minimized(),
                    Some(WindowControl::ToggleMaximize) if self.maximized => {
                        self.top.unset_maximized()
                    }
                    Some(WindowControl::ToggleMaximize) => self.top.set_maximized(),
                    None => {}
                }
            }
            Err(error) => self.failure = Some(error),
        }
    }

    fn present(&mut self, qh: &QueueHandle<Self>) -> anyhow::Result<()> {
        if !self.configured || !self.dirty || self.waiting {
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
            frame.damage = Some(Clip::canvas(size));
        }
        if let Some(audit) = &mut self.audit {
            audit.check(&frame)?;
        }
        let callback = frame.request_callback;
        let Some(area) = frame.damage else {
            if callback {
                // A frame request takes effect at commit, even with no new buffer.
                self.surface.frame(qh, None);
                self.surface.commit();
                self.waiting = true;
            }
            self.dirty = false;
            self.view.frame_submitted()?;
            return Ok(());
        };
        for slot in self.slots.iter_mut().flatten().filter(|s| s.size == size) {
            slot.invalidate(area);
        }
        let reuse = self.slots[index]
            .as_ref()
            .is_some_and(|slot| slot.size == size);
        if !reuse {
            self.slots[index] = Some(Buffer::new(size, index, self.pixel_order, &self.shm, qh)?);
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
        self.surface.attach(Some(&slot.handle), 0, 0);
        self.surface.damage_buffer(
            area.left as i32,
            area.top as i32,
            (area.right - area.left) as i32,
            (area.bottom - area.top) as i32,
        );
        if callback {
            self.surface.frame(qh, Some(self.commits + 1));
            self.waiting = true;
        }
        if let Some(presentation) = &self.presentation {
            presentation.feedback(&self.surface, qh, self.commits + 1);
        }
        self.surface.commit();
        let queued_ns = self.trace.then(presentation::monotonic_ns);
        slot.busy = true;
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
            format: wayland_client::WEnum::Value(wl_shm::Format::Xbgr8888),
        } = event
        {
            state.pixel_order = PixelOrder::Rgb;
        }
    }
}
delegate_noop!(Window: ignore wl_shm_pool::WlShmPool);

impl Dispatch<wl_callback::WlCallback, Option<usize>> for Window {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        buffer_commit: &Option<usize>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if state.trace {
            let dispatched_ns = presentation::monotonic_ns();
            match buffer_commit {
                Some(commit) => eprintln!("frame_done={commit} dispatched_ns={dispatched_ns}"),
                None => eprintln!("frame_done=callback_only dispatched_ns={dispatched_ns}"),
            }
        }
        state.waiting = false;
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
            state.configured = true;
            state.maximized = state.pending_maximized;
            let decorations_configured = state
                .decorations
                .as_mut()
                .is_some_and(|d| d.take_configure());
            if let Some(size) = state.pending_size.take()
                && size != state.size
            {
                state.size = size;
                state.full_damage = true;
                state.dirty = true;
            } else if decorations_configured && state.commits != 0 && !state.full_damage {
                // Apply a decoration-only configure even when the retained view is idle.
                state.surface.commit();
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/damage.rs"]
mod tests;
