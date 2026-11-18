use std::{fs::File, io, ops::Range, os::fd::AsFd, os::unix::fs::FileExt};

use rustix::fs::{MemfdFlags, memfd_create};
use wayland_client::{
    QueueHandle,
    protocol::{wl_buffer, wl_shm},
};

use super::Window;
use crate::{
    layout::Size,
    pixels::{Clip, Surface},
};

pub(super) struct Buffer {
    pub handle: wl_buffer::WlBuffer,
    pub size: Size,
    pub busy: bool,
    file: File,
    pending: PendingRows,
}

impl Buffer {
    // Size has already passed the rasterizer's pixel budget.
    pub fn new(
        size: Size,
        slot: usize,
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
            wl_shm::Format::Xrgb8888,
            qh,
            slot,
        );
        pool.destroy();
        Ok(Self {
            handle,
            size,
            busy: false,
            file,
            pending: PendingRows(0..size.height as usize),
        })
    }

    pub fn invalidate(&mut self, area: Clip) {
        self.pending
            .include(area.top as usize..area.bottom as usize);
    }

    pub fn upload(&mut self, pixels: &Surface) -> io::Result<usize> {
        self.pending.write(&self.file, pixels)
    }

    pub fn verify(&self, oracle: &Surface) -> io::Result<()> {
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
                let [x, r, g, b] = u32::from_ne_bytes(*word).to_be_bytes();
                if [r, g, b, x] != *rgba {
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

/// Physical scanlines changed since the last successful write to this file.
struct PendingRows(Range<usize>);

impl PendingRows {
    fn include(&mut self, rows: Range<usize>) {
        self.0 = if self.0.is_empty() {
            rows
        } else {
            self.0.start.min(rows.start)..self.0.end.max(rows.end)
        };
    }

    fn write(&mut self, file: &File, pixels: &Surface) -> io::Result<usize> {
        let mut staging = [0u8; 16384];
        let stride = pixels.size.width as usize * 4;
        let bytes = self.0.start * stride..self.0.end * stride;
        let mut offset = bytes.start as u64;
        for rgba in pixels.rgba[bytes.clone()].chunks(staging.len()) {
            for (src, dst) in rgba
                .as_chunks::<4>()
                .0
                .iter()
                .zip(staging.as_chunks_mut::<4>().0)
            {
                let value = u32::from_be_bytes([255, src[0], src[1], src[2]]);
                dst.copy_from_slice(&value.to_ne_bytes());
            }
            file.write_all_at(&staging[..rgba.len()], offset)?;
            offset += rgba.len() as u64;
        }
        self.0 = 0..0;
        Ok(bytes.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delayed_slot_repairs_all_missed_rows_and_retries_after_io_error() {
        let size = Size {
            width: 3,
            height: 6,
        };
        let mut pixels = Surface {
            size,
            rgba: vec![255; 3 * 6 * 4],
        };
        let file = File::from(memfd_create("solor-test", MemfdFlags::CLOEXEC).unwrap());
        let mut dirty = PendingRows(0..6);
        assert_eq!(dirty.write(&file, &pixels).unwrap(), 72);

        // The slot misses three updates, including one out of row order.
        for y in [4, 1, 3] {
            pixels.rgba[y * 12..y * 12 + 4].copy_from_slice(&[y as u8, 20, 80, 255]);
            dirty.include(y..y + 1);
        }
        let read_only = File::open("/dev/null").unwrap();
        assert!(dirty.write(&read_only, &pixels).is_err());
        assert_eq!(dirty.write(&file, &pixels).unwrap(), 48);
        assert_eq!(dirty.write(&file, &pixels).unwrap(), 0);

        let mut data = [0u8; 72];
        file.read_exact_at(&mut data, 0).unwrap();
        for (word, rgba) in data
            .as_chunks::<4>()
            .0
            .iter()
            .zip(pixels.rgba.as_chunks::<4>().0)
        {
            let [x, r, g, b] = u32::from_ne_bytes(*word).to_be_bytes();
            assert_eq!([r, g, b, x], *rgba);
        }
    }
}
