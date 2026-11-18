use std::{fs::File, io, os::fd::AsFd, os::unix::fs::FileExt};

use rustix::fs::{MemfdFlags, memfd_create};
use wayland_client::{
    QueueHandle,
    protocol::{wl_buffer, wl_shm},
};

use super::Window;
use crate::{
    pixels::DeviceSize,
    pixels::{Clip, Surface},
};

pub(super) struct Buffer {
    pub handle: wl_buffer::WlBuffer,
    pub size: DeviceSize,
    pub busy: bool,
    file: File,
    pending: PendingDamage,
}

impl Buffer {
    // DeviceSize has already passed the rasterizer's pixel budget.
    pub fn new(
        size: DeviceSize,
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
            pending: PendingDamage(Some(Clip::canvas(size))),
        })
    }

    pub fn invalidate(&mut self, area: Clip) {
        self.pending.include(area);
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

/// Bounds of all changes since this file was last brought up to date.
struct PendingDamage(Option<Clip>);

impl PendingDamage {
    fn include(&mut self, area: Clip) {
        self.0 = Some(self.0.map_or(area, |old| old.union(area)));
    }

    fn write(&mut self, file: &File, pixels: &Surface) -> io::Result<usize> {
        let Some(area) = self.0 else { return Ok(0) };
        let stride = pixels.size.width as usize * 4;
        let rows = area.top as usize..area.bottom as usize;
        let left = area.left as usize * 4;
        let right = area.right as usize * 4;
        let mut staging = [0u8; 16384];
        let mut write_span = |start: usize, end: usize| -> io::Result<()> {
            let mut offset = start as u64;
            for rgba in pixels.rgba[start..end].chunks(staging.len()) {
                for (src, dst) in rgba
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(staging.as_chunks_mut::<4>().0)
                {
                    dst.copy_from_slice(
                        &u32::from_be_bytes([255, src[0], src[1], src[2]]).to_ne_bytes(),
                    );
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
mod tests {
    use super::*;

    #[test]
    fn delayed_slot_repairs_disjoint_edits_and_retries_after_io_error() {
        for (width, expected_bytes) in [(5, 68), (1024, 8 * 4)] {
            let size = DeviceSize { width, height: 6 };
            let stride = width as usize * 4;
            let mut pixels = Surface {
                size,
                rgba: vec![255; stride * 6],
            };
            let file = File::from(memfd_create("solor-test", MemfdFlags::CLOEXEC).unwrap());
            let mut dirty = PendingDamage(Some(Clip::canvas(size)));
            assert_eq!(dirty.write(&file, &pixels).unwrap(), pixels.rgba.len());

            // The slot misses three edits in different rows and columns.
            for (x, y) in [(2, 4), (1, 1), (2, 3)] {
                let start = y * stride + x * 4;
                pixels.rgba[start..start + 4].copy_from_slice(&[y as u8, 20, 80, 255]);
                dirty.include(Clip {
                    left: x as i64,
                    top: y as i64,
                    right: x as i64 + 1,
                    bottom: y as i64 + 1,
                });
            }
            let read_only = File::open("/dev/null").unwrap();
            assert!(dirty.write(&read_only, &pixels).is_err());
            assert_eq!(dirty.write(&file, &pixels).unwrap(), expected_bytes);
            assert_eq!(dirty.write(&file, &pixels).unwrap(), 0);

            let mut data = vec![0; pixels.rgba.len()];
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
}
