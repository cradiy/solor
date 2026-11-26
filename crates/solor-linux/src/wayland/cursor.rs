use super::Desktop;
use cursor_icon::CursorIcon;
use rustix::fs::{MemfdFlags, memfd_create};
use std::{
    collections::HashMap,
    fs::File,
    io::{self, Write},
    os::fd::AsFd,
};
use wayland_client::{
    QueueHandle,
    protocol::{
        wl_buffer::WlBuffer,
        wl_compositor::WlCompositor,
        wl_pointer::WlPointer,
        wl_shm::{Format, WlShm},
        wl_surface::WlSurface,
    },
};
use xcursor::{CursorTheme, parser::parse_xcursor_stream};

pub(super) struct ThemedCursor {
    surface: WlSurface,
    theme: CursorTheme,
    logical_size: u32,
    scale: u32,
    cache: HashMap<CursorIcon, Option<Image>>,
}

struct Image {
    buffer: WlBuffer,
    hotspot: (i32, i32),
}

impl Drop for Image {
    fn drop(&mut self) {
        self.buffer.destroy();
    }
}

impl Drop for ThemedCursor {
    fn drop(&mut self) {
        self.surface.destroy();
    }
}

impl ThemedCursor {
    pub fn new(compositor: &WlCompositor, qh: &QueueHandle<Desktop>) -> Self {
        let name = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "default".into());
        let logical_size = std::env::var("XCURSOR_SIZE")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|n| *n > 0)
            .unwrap_or(24);
        Self {
            surface: compositor.create_surface(qh, ()),
            theme: CursorTheme::load(&name),
            logical_size,
            scale: 0,
            cache: HashMap::new(),
        }
    }

    pub fn apply(
        &mut self,
        icon: CursorIcon,
        scale: u32,
        pointer: &WlPointer,
        serial: u32,
        shm: &WlShm,
        qh: &QueueHandle<Desktop>,
    ) -> io::Result<()> {
        if self.scale != scale {
            self.cache.clear();
            self.scale = scale;
        }
        for key in [icon, CursorIcon::Default] {
            let entry = match self.cache.entry(key) {
                std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(load(&self.theme, key, self.logical_size, scale, shm, qh)?)
                }
            };
            if let Some(image) = entry {
                self.surface.set_buffer_scale(scale as i32);
                pointer.set_cursor(
                    serial,
                    Some(&self.surface),
                    image.hotspot.0,
                    image.hotspot.1,
                );
                self.surface.attach(Some(&image.buffer), 0, 0);
                self.surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
                self.surface.commit();
                return Ok(());
            }
        }
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "cursor theme has no default pointer",
        ))
    }
}

fn load(
    theme: &CursorTheme,
    icon: CursorIcon,
    logical_size: u32,
    scale: u32,
    shm: &WlShm,
    qh: &QueueHandle<Desktop>,
) -> io::Result<Option<Image>> {
    let Some(path) = std::iter::once(icon.name())
        .chain(icon.alt_names().iter().copied())
        .find_map(|name| theme.load_icon(name))
    else {
        return Ok(None);
    };
    let images = parse_xcursor_stream(&mut std::io::BufReader::new(File::open(path)?))?;
    let Some(image) = images
        .into_iter()
        .min_by_key(|i| i.size.abs_diff(logical_size.saturating_mul(scale)))
    else {
        return Ok(None);
    };
    let width = image.width.next_multiple_of(scale);
    let height = image.height.next_multiple_of(scale);
    let bytes = width
        .checked_mul(height)
        .and_then(|n| n.checked_mul(4))
        .and_then(|n| i32::try_from(n).ok())
        .ok_or_else(|| io::Error::other("cursor image too large"))?;
    let mut file = File::from(memfd_create("solor-cursor", MemfdFlags::CLOEXEC)?);
    file.set_len(bytes as u64)?;
    let pixels = image.pixels_rgba;
    #[cfg(target_endian = "big")]
    let mut pixels = pixels;
    #[cfg(target_endian = "big")]
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.reverse();
    }
    if width == image.width {
        file.write_all(&pixels)?;
    } else {
        let padding = vec![0; (width - image.width) as usize * 4];
        for row in pixels.chunks_exact(image.width as usize * 4) {
            file.write_all(row)?;
            file.write_all(&padding)?;
        }
    }
    let pool = shm.create_pool(file.as_fd(), bytes, qh, ());
    let buffer = pool.create_buffer(
        0,
        width as i32,
        height as i32,
        width as i32 * 4,
        Format::Argb8888,
        qh,
        (),
    );
    pool.destroy();
    Ok(Some(Image {
        buffer,
        hotspot: ((image.xhot / scale) as i32, (image.yhot / scale) as i32),
    }))
}
