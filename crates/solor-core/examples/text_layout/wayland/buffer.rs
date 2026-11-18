use std::{fs::File, io, os::fd::AsFd, os::unix::fs::FileExt};

use rustix::fs::{MemfdFlags, memfd_create};
use wayland_client::{
    QueueHandle,
    protocol::{wl_buffer, wl_shm},
};

use super::Window;
use crate::{layout::Size, pixels::Surface};

pub(super) struct Buffer {
    pub handle: wl_buffer::WlBuffer,
    pub size: Size,
    pub busy: bool,
    file: File,
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
        })
    }

    pub fn upload(&self, pixels: &Surface) -> io::Result<()> {
        let mut staging = [0u8; 16384];
        let mut offset = 0;
        for rgba in pixels.rgba.chunks(staging.len()) {
            for (src, dst) in rgba
                .as_chunks::<4>()
                .0
                .iter()
                .zip(staging.as_chunks_mut::<4>().0)
            {
                let value = u32::from_be_bytes([255, src[0], src[1], src[2]]);
                dst.copy_from_slice(&value.to_ne_bytes());
            }
            self.file.write_all_at(&staging[..rgba.len()], offset)?;
            offset += rgba.len() as u64;
        }
        Ok(())
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
