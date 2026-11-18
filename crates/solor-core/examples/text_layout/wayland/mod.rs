mod buffer;
mod pointer;

use std::{error::Error, io, time::Instant};

use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop,
    globals::registry_queue_init,
    protocol::{wl_buffer, wl_callback, wl_compositor, wl_shm, wl_shm_pool, wl_surface},
};
use wayland_protocols::wp::{
    fractional_scale::v1::client::{wp_fractional_scale_manager_v1, wp_fractional_scale_v1},
    viewporter::client::{wp_viewport, wp_viewporter},
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::{
    damage::RetainedPixels,
    glyph_masks::GlyphMasks,
    layout::{LayoutError, Size},
    pixels::{Clip, RasterScale, Surface},
    window_content::Content,
};
use buffer::Buffer;

struct Window {
    viewport: wp_viewport::WpViewport,
    scale_120: u32,
    surface: wl_surface::WlSurface,
    shm: wl_shm::WlShm,
    slots: [Option<Buffer>; 2],
    next_slot: usize,
    view: Content,
    seats: Vec<pointer::Seat>,
    pixels: Option<RetainedPixels>,
    masks: GlyphMasks,
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
    previous: Option<Surface>,
}

impl DamageAudit {
    fn check(&mut self, current: Surface, damage: Option<Clip>) -> io::Result<()> {
        let canvas = Clip::canvas(current.size);
        if damage.is_some_and(|area| area.empty() || area.intersect(canvas) != area) {
            return Err(io::Error::other("damage outside the submitted buffer"));
        }
        if damage != Some(canvas) {
            let previous = self
                .previous
                .as_ref()
                .filter(|old| old.size == current.size)
                .ok_or_else(|| io::Error::other("new target requires full damage"))?;
            let stride = current.size.width as usize * 4;
            for (y, (old, new)) in previous
                .rgba
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
        self.previous = Some(current);
        Ok(())
    }
}

fn buffer_size(scale_120: u32, logical: Size) -> io::Result<Size> {
    let dimension = |n: u32| {
        u32::try_from(((u64::from(n) * u64::from(scale_120) + 60) / 120).max(1))
            .map_err(|_| io::Error::other("scaled dimension out of range"))
    };
    let size = Size {
        width: dimension(logical.width)?,
        height: dimension(logical.height)?,
    };
    if scale_120 == 0 || u64::from(size.width) * u64::from(size.height) > 4 * 1024 * 1024 {
        return Err(io::Error::other(
            "scale or buffer size outside raster budget",
        ));
    }
    Ok(size)
}

fn layout_error(error: LayoutError) -> io::Error {
    io::Error::other(format!("fixture layout: {error:?}"))
}

pub(super) fn run(title: &str, app_id: &str, view: Content) -> Result<(), Box<dyn Error>> {
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
    top.set_title(title.into());
    top.set_app_id(app_id.into());
    surface.commit();

    let mut window = Window {
        viewport,
        scale_120: 120,
        surface,
        shm,
        slots: [None, None],
        next_slot: 0,
        view,
        seats: Vec::new(),
        pixels: None,
        masks: GlyphMasks::new(64, 32 * 1024),
        size: Size {
            width: 320,
            height: 100,
        },
        pending_size: None,
        configured: false,
        dirty: true,
        waiting: false,
        closed: false,
        commits: 0,
        audit: std::env::var_os("SOLOR_VERIFY_SHM").map(|_| DamageAudit::default()),
        start,
    };
    if window.view.accepts_pointer() {
        for global in globals.contents().clone_list() {
            if global.interface == "wl_seat" {
                window.add_seat(globals.registry(), global.name, global.version, &qh);
            }
        }
    }
    loop {
        queue.blocking_dispatch(&mut window)?;
        if window.closed {
            break;
        }
        window.present(&qh)?;
    }
    top.destroy();
    role.destroy();
    fractional.destroy();
    window.viewport.destroy();
    window.surface.destroy();
    shell.destroy();
    connection.flush()?;
    eprintln!("closed commits={}", window.commits);
    Ok(())
}

impl Window {
    fn present(&mut self, qh: &QueueHandle<Self>) -> io::Result<()> {
        if !self.configured || !self.dirty || self.waiting {
            return Ok(());
        }
        let Some(index) = [self.next_slot, 1 - self.next_slot]
            .into_iter()
            .find(|&i| self.slots[i].as_ref().is_none_or(|slot| !slot.busy))
        else {
            return Ok(());
        };
        let size = buffer_size(self.scale_120, self.size)?;
        let scale = RasterScale(f64::from(self.scale_120) / 120.0);
        let frame = self.view.frame(self.size).map_err(layout_error)?;
        let clip = Clip::canvas(size);
        let damage = match &mut self.pixels {
            Some(pixels) => pixels.update(frame.clone(), size, clip, &mut self.masks)?,
            None => {
                self.pixels = Some(RetainedPixels::new_scaled(
                    frame.clone(),
                    size,
                    clip,
                    scale,
                    &mut self.masks,
                )?);
                Some(clip)
            }
        };

        let oracle = || Surface::raster(&frame, size, clip, (0, 0), scale, None);
        let callback = self.view.wants_frame_callback(damage.is_some());
        let Some(area) = damage else {
            if let Some(audit) = &mut self.audit {
                audit.check(oracle()?, None)?;
            }
            if callback {
                // A frame request takes effect at commit, even with no new buffer.
                self.surface.frame(qh, ());
                self.surface.commit();
                self.waiting = true;
            }
            self.dirty = false;
            return Ok(());
        };
        for slot in self.slots.iter_mut().flatten().filter(|s| s.size == size) {
            slot.invalidate(area);
        }
        let reuse = self.slots[index]
            .as_ref()
            .is_some_and(|slot| slot.size == size);
        if !reuse {
            self.slots[index] = Some(Buffer::new(size, index, &self.shm, qh)?);
        }
        let slot = self.slots[index]
            .as_mut()
            .expect("initialized free shm slot");
        let upload_start = Instant::now();
        let pixels = self.pixels.as_ref().expect("prepared retained pixels");
        let uploaded = slot.upload(pixels.surface())?;
        let upload_us = upload_start.elapsed().as_micros();
        if let Some(audit) = &mut self.audit {
            let oracle = oracle()?;
            slot.verify(&oracle)?;
            audit.check(oracle, damage)?;
        }
        self.viewport
            .set_destination(self.size.width as i32, self.size.height as i32);
        self.surface.attach(Some(&slot.handle), 0, 0);
        self.surface.damage_buffer(
            area.left as i32,
            area.top as i32,
            (area.right - area.left) as i32,
            (area.bottom - area.top) as i32,
        );
        if callback {
            self.surface.frame(qh, ());
            self.waiting = true;
        }
        self.surface.commit();
        slot.busy = true;
        self.next_slot = 1 - index;
        self.dirty = false;
        self.commits += 1;
        eprintln!(
            "commit={} edit={} size={}x{} buffer={}x{} scale_120={} slot={} reuse={} audited={} rows={} scroll_y={} upload_bytes={} upload_us={} elapsed_us={}",
            self.commits,
            self.view.edit(),
            self.size.width,
            self.size.height,
            size.width,
            size.height,
            self.scale_120,
            index,
            reuse,
            self.audit.is_some(),
            frame.draws.len(),
            frame.scroll_y,
            uploaded,
            upload_us,
            self.start.elapsed().as_micros()
        );
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
            state.pixels = None;
            state.dirty = true;
        }
    }
}

delegate_noop!(Window: ignore wl_compositor::WlCompositor);
delegate_noop!(Window: ignore wl_surface::WlSurface);
delegate_noop!(Window: ignore wl_shm::WlShm);
delegate_noop!(Window: ignore wl_shm_pool::WlShmPool);

impl Dispatch<wl_callback::WlCallback, ()> for Window {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.waiting = false;
        state.dirty |= state.view.frame_ready();
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
            xdg_toplevel::Event::Configure { width, height, .. } => {
                state.pending_size = Some(Size {
                    width: if width > 0 {
                        width as u32
                    } else {
                        state.size.width
                    },
                    height: if height > 0 {
                        height as u32
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
            if let Some(size) = state.pending_size.take()
                && size != state.size
            {
                state.size = size;
                state.pixels = None;
                state.dirty = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_preserves_pixels_outside_the_surface_damage() {
        let size = Size {
            width: 5,
            height: 3,
        };
        let image = |edits: &[(usize, usize)]| {
            let mut rgba = vec![255; 5 * 3 * 4];
            for &(x, y) in edits {
                rgba[(5 * y + x) * 4] = 0;
            }
            Surface { size, rgba }
        };
        let region = Clip {
            left: 1,
            top: 1,
            right: 4,
            bottom: 2,
        };
        let mut audit = DamageAudit::default();
        assert!(audit.check(image(&[]), None).is_err());
        audit.check(image(&[]), Some(Clip::canvas(size))).unwrap();
        for outside in [(0, 1), (4, 1), (2, 0), (2, 2)] {
            assert!(audit.check(image(&[outside]), Some(region)).is_err());
        }
        let inside = [(1, 1), (3, 1)];
        audit.check(image(&inside), Some(region)).unwrap();
        audit.check(image(&inside), None).unwrap();
        assert!(audit.check(image(&[]), None).is_err());
    }
}
