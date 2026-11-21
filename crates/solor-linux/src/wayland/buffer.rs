use std::{fs::File, io, os::fd::AsFd, os::unix::fs::FileExt};

use rustix::fs::{MemfdFlags, memfd_create};
use wayland_client::{
    QueueHandle,
    protocol::{wl_buffer, wl_shm},
};

use super::Window;
use solor::raster::{Clip, Damage, DeviceSize};
use solor::{platform::WindowBackground, raster::RasterFrame};

/// Byte order of the three visible channels in a shared-memory pixel.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PixelOrder {
    Rgb,
    Bgr,
}

impl PixelOrder {
    pub(super) fn format(self, background: WindowBackground) -> wl_shm::Format {
        match (self, background) {
            (Self::Rgb, WindowBackground::Opaque) => wl_shm::Format::Xbgr8888,
            (Self::Bgr, WindowBackground::Opaque) => wl_shm::Format::Xrgb8888,
            (Self::Rgb, WindowBackground::Transparent) => wl_shm::Format::Abgr8888,
            (Self::Bgr, WindowBackground::Transparent) => wl_shm::Format::Argb8888,
        }
    }
}

pub(super) struct Buffer {
    pub handle: wl_buffer::WlBuffer,
    pub size: DeviceSize,
    pub busy: bool,
    order: PixelOrder,
    file: File,
    pending: PendingDamage,
}

impl Buffer {
    // DeviceSize has already passed the rasterizer's pixel budget.
    pub fn new(
        size: DeviceSize,
        slot: usize,
        order: PixelOrder,
        background: WindowBackground,
        shm: &wl_shm::WlShm,
        qh: &QueueHandle<Window>,
    ) -> io::Result<Self> {
        let file = File::from(memfd_create("solor-text", MemfdFlags::CLOEXEC)?);
        let bytes = size.width * size.height * 4;
        file.set_len(bytes.into())?;
        let pool = shm.create_pool(file.as_fd(), bytes as i32, qh, ());
        let handle = pool.create_buffer(
            0,
            size.width as i32,
            size.height as i32,
            (size.width * 4) as i32,
            order.format(background),
            qh,
            slot,
        );
        pool.destroy();
        Ok(Self {
            handle,
            size,
            busy: false,
            order,
            file,
            pending: PendingDamage(Damage::from_rect(Clip::canvas(size))),
        })
    }

    pub fn invalidate(&mut self, area: Clip) {
        self.pending.include(area);
    }

    pub fn upload(&mut self, pixels: &RasterFrame<'_>) -> io::Result<usize> {
        self.pending.write(&self.file, pixels, self.order)
    }

    pub fn verify(&self, oracle: &RasterFrame<'_>) -> io::Result<()> {
        let mut data = [0u8; 16384];
        let mut offset = 0;
        for expected in oracle.rgba.chunks(data.len()) {
            let read = &mut data[..expected.len()];
            self.file.read_exact_at(read, offset)?;
            for (word, rgba) in read
                .as_chunks::<4>()
                .0
                .iter()
                .zip(expected.as_chunks::<4>().0)
            {
                let [first, green, third, alpha] = *word;
                let pixel = match self.order {
                    PixelOrder::Rgb => [first, green, third, alpha],
                    PixelOrder::Bgr => [third, green, first, alpha],
                };
                if pixel != *rgba {
                    return Err(io::Error::other("shm pixel differs from uncached frame"));
                }
            }
            offset += expected.len() as u64;
        }
        Ok(())
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        self.handle.destroy();
    }
}

/// Bounds of all changes since this file was last brought up to date.
struct PendingDamage(Damage);

impl PendingDamage {
    fn include(&mut self, area: Clip) {
        self.0.include(area);
    }

    fn write(
        &mut self,
        file: &File,
        pixels: &RasterFrame<'_>,
        order: PixelOrder,
    ) -> io::Result<usize> {
        let mut rects: smallvec::SmallVec<[_; 16]> = smallvec::SmallVec::from_slice(self.0.rects());
        rects.sort_unstable_by_key(|r| r.left);
        let Some(bounds) = self.0.bounds() else {
            return Ok(0);
        };
        let stride = pixels.size.width as usize * 4;
        let mut staging = [0; 16384];
        let mut span = 0..0;
        let mut written = 0;
        for y in bounds.top..bounds.bottom {
            let row = y as usize * stride;
            for rect in &rects {
                if y < rect.top || y >= rect.bottom {
                    continue;
                }
                let start = row + rect.left as usize * 4;
                let end = row + rect.right as usize * 4;
                // Bridge small untouched gaps in the current complete image to save a syscall.
                if span.is_empty() || start.saturating_sub(span.end) >= 2048 {
                    if !span.is_empty() {
                        write_span(file, pixels, order, span.clone(), &mut staging)?;
                        written += span.len();
                    }
                    span = start..end;
                } else {
                    span.end = span.end.max(end);
                }
            }
        }
        if !span.is_empty() {
            write_span(file, pixels, order, span.clone(), &mut staging)?;
            written += span.len();
        }
        self.0 = Damage::default();
        Ok(written)
    }
}

fn write_span(
    file: &File,
    pixels: &RasterFrame<'_>,
    order: PixelOrder,
    span: std::ops::Range<usize>,
    staging: &mut [u8; 16384],
) -> io::Result<()> {
    let (start, end) = (span.start, span.end);

    if order == PixelOrder::Rgb {
        return file.write_all_at(&pixels.rgba[start..end], start as u64);
    }
    let mut offset = start as u64;
    for rgba in pixels.rgba[start..end].chunks(staging.len()) {
        for (src, dst) in rgba
            .as_chunks::<4>()
            .0
            .iter()
            .zip(staging.as_chunks_mut::<4>().0)
        {
            *dst = [src[2], src[1], src[0], src[3]];
        }
        file.write_all_at(&staging[..rgba.len()], offset)?;
        offset += rgba.len() as u64;
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/unit/buffer.rs"]
mod tests;
