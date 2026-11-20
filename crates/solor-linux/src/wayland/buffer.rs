use std::{fs::File, io, os::fd::AsFd, os::unix::fs::FileExt};

use rustix::fs::{MemfdFlags, memfd_create};
use wayland_client::{
    QueueHandle,
    protocol::{wl_buffer, wl_shm},
};

use super::Window;
use solor::raster::RasterFrame;
use solor::raster::{Clip, DeviceSize};

/// Byte order of the three visible channels in an opaque shared-memory pixel.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PixelOrder {
    Rgb,
    Bgr,
}

impl PixelOrder {
    fn format(self) -> wl_shm::Format {
        match self {
            Self::Rgb => wl_shm::Format::Xbgr8888,
            Self::Bgr => wl_shm::Format::Xrgb8888,
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
            order.format(),
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
            pending: PendingDamage(Some(Clip::canvas(size))),
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
                let [first, green, third, _] = *word;
                let rgb = match self.order {
                    PixelOrder::Rgb => [first, green, third],
                    PixelOrder::Bgr => [third, green, first],
                };
                if rgb != rgba[..3] {
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
struct PendingDamage(Option<Clip>);

impl PendingDamage {
    fn include(&mut self, area: Clip) {
        self.0 = Some(self.0.map_or(area, |old| old.union(area)));
    }

    fn write(
        &mut self,
        file: &File,
        pixels: &RasterFrame<'_>,
        order: PixelOrder,
    ) -> io::Result<usize> {
        let Some(area) = self.0 else { return Ok(0) };
        let stride = pixels.size.width as usize * 4;
        let rows = area.top as usize..area.bottom as usize;
        let left = area.left as usize * 4;
        let right = area.right as usize * 4;
        let mut staging = [0u8; 16384];
        let mut write_span = |start: usize, end: usize| -> io::Result<()> {
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
                    *dst = [src[2], src[1], src[0], 255];
                }
                file.write_all_at(&staging[..rgba.len()], offset)?;
                offset += rgba.len() as u64;
            }
            Ok(())
        };
        // Small gaps cost less to convert and copy than an extra write per row.
        const MIN_GAP_BYTES: usize = 2048;
        let row_bytes = right - left;
        let bytes = if stride - row_bytes < MIN_GAP_BYTES {
            let start = rows.start * stride + left;
            let end = (rows.end - 1) * stride + right;
            write_span(start, end)?;
            end - start
        } else {
            let bytes = rows.len() * row_bytes;
            for y in rows {
                write_span(y * stride + left, y * stride + right)?;
            }
            bytes
        };
        self.0 = None;
        Ok(bytes)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/buffer.rs"]
mod tests;
