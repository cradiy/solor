mod buffer;

use std::{error::Error, io, time::Instant};

use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_buffer, wl_callback, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface,
    },
};
use wayland_protocols::wp::{
    fractional_scale::v1::client::{wp_fractional_scale_manager_v1, wp_fractional_scale_v1},
    viewporter::client::{wp_viewport, wp_viewporter},
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::{
    damage::RetainedPixels,
    frame::{Mode, Presentation},
    glyph_masks::GlyphMasks,
    layout::{LayoutError, Size},
    pixels::{Clip, RasterScale, Surface},
};
use buffer::Buffer;

const ROWS: [[&str; 2]; 2] = [
    ["Solor 中文排版", "Solor 文字换行与基线。"],
    ["Hello 世界！Rust GUI 123", "Hello 世界！测试界面宽度改变。"],
];
const EDITS: usize = 8;

struct Window {
    viewport: wp_viewport::WpViewport,
    scale: RasterScale,
    surface: wl_surface::WlSurface,
    shm: wl_shm::WlShm,
    slots: [Option<Buffer>; 2],
    next_slot: usize,
    view: Presentation,
    pixels: Option<RetainedPixels>,
    masks: GlyphMasks,
    size: Size,
    pending_size: Option<Size>,
    configured: bool,
    dirty: bool,
    waiting: bool,
    closed: bool,
    advance: bool,
    edit: usize,
    commits: usize,
    audit: bool,
    start: Instant,
}

fn buffer_size(scale: RasterScale, logical: Size) -> io::Result<Size> {
    let dimension = |n: u32| {
        u32::try_from(((u64::from(n) * u64::from(scale.0) + 60) / 120).max(1))
            .map_err(|_| io::Error::other("scaled dimension out of range"))
    };
    let size = Size {
        width: dimension(logical.width)?,
        height: dimension(logical.height)?,
    };
    if scale.0 == 0 || u64::from(size.width) * u64::from(size.height) > 4 * 1024 * 1024 {
        return Err(io::Error::other(
            "scale or buffer size outside raster budget",
        ));
    }
    Ok(size)
}

fn layout_error(error: LayoutError) -> io::Error {
    io::Error::other(format!("fixture layout: {error:?}"))
}

pub(super) fn run() -> Result<(), Box<dyn Error>> {
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
    top.set_title("Solor text window".into());
    top.set_app_id("solor.text-window".into());
    surface.commit();

    let mut view = Presentation::new(2, ROWS[0][0], true);
    view.set_content(1, ROWS[1][0]).map_err(layout_error)?;
    view.set_color(1, [27, 85, 148, 255])
        .map_err(layout_error)?;
    let mut window = Window {
        viewport,
        scale: RasterScale::ONE,
        surface,
        shm,
        slots: [None, None],
        next_slot: 0,
        view,
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
        advance: false,
        edit: 0,
        commits: 0,
        audit: std::env::var_os("SOLOR_VERIFY_SHM").is_some(),
        start,
    };
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
        let size = buffer_size(self.scale, self.size)?;
        if self.advance {
            self.advance = false;
            self.edit += 1;
            let row = (self.edit - 1) % 2;
            self.view
                .set_content(row, ROWS[row][((self.edit - 1) / 2 + 1) % 2])
                .map_err(layout_error)?;
        }
        let frame = self
            .view
            .update(self.size, Mode::Normal)
            .map_err(layout_error)?
            .frame;
        let clip = Clip::canvas(size);
        let damage = match &mut self.pixels {
            Some(pixels) => pixels.update(frame.clone(), size, clip, &mut self.masks)?,
            None => {
                self.pixels = Some(RetainedPixels::new_scaled(
                    frame.clone(),
                    size,
                    clip,
                    self.scale,
                    &mut self.masks,
                )?);
                Some(clip)
            }
        };

        if let Some(area) = damage {
            for slot in self.slots.iter_mut().flatten().filter(|s| s.size == size) {
                slot.invalidate(area);
            }
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
        if self.audit {
            slot.verify(&Surface::raster(
                &frame,
                size,
                clip,
                (0, 0),
                self.scale,
                None,
            )?)?;
        }
        self.viewport
            .set_destination(self.size.width as i32, self.size.height as i32);
        self.surface.attach(Some(&slot.handle), 0, 0);
        self.surface
            .damage_buffer(0, 0, size.width as i32, size.height as i32);
        if self.edit < EDITS {
            self.surface.frame(qh, ());
            self.waiting = true;
        }
        self.surface.commit();
        slot.busy = true;
        self.next_slot = 1 - index;
        self.dirty = false;
        self.commits += 1;
        eprintln!(
            "commit={} edit={} size={}x{} buffer={}x{} scale_120={} slot={} reuse={} audited={} upload_bytes={} upload_us={} elapsed_us={}",
            self.commits,
            self.edit,
            self.size.width,
            self.size.height,
            size.width,
            size.height,
            self.scale.0,
            index,
            reuse,
            self.audit,
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
            && state.scale != RasterScale(scale)
        {
            state.scale = RasterScale(scale);
            state.pixels = None;
            state.dirty = true;
        }
    }
}

delegate_noop!(Window: ignore wl_compositor::WlCompositor);
delegate_noop!(Window: ignore wl_surface::WlSurface);
delegate_noop!(Window: ignore wl_shm::WlShm);
delegate_noop!(Window: ignore wl_shm_pool::WlShmPool);

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Window {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

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
        state.advance = true;
        state.dirty = true;
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
            if let Some(size) = state.pending_size.take() {
                state.dirty |= size != state.size;
                state.size = size;
            }
        }
    }
}
