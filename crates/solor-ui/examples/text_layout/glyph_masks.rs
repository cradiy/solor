//! A FIFO of grayscale outline masks for a single immutable face at a time.
use std::collections::VecDeque;
use std::io::Result;

use swash::scale::image::Image;

struct Entry {
    size_bits: u32,
    glyph: u16,
    mask: Image,
}

pub struct GlyphMasks {
    font: Option<&'static [u8]>,
    entries: VecDeque<Entry>,
    bytes: usize,
    max_entries: usize,
    max_bytes: usize,
    pub hits: usize,
    pub misses: usize,
}

impl GlyphMasks {
    pub fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            font: None,
            entries: VecDeque::new(),
            bytes: 0,
            max_entries,
            max_bytes,
            hits: 0,
            misses: 0,
        }
    }

    pub fn release(&mut self) {
        self.entries = VecDeque::new();
        self.bytes = 0;
        self.font = None;
    }

    pub fn bind(&mut self, font: &'static [u8]) {
        if !self.font.is_some_and(|old| std::ptr::eq(old, font)) {
            self.release();
            self.font = Some(font);
        }
    }

    pub fn usage(&self) -> (usize, usize) {
        (self.entries.len(), self.bytes)
    }

    pub fn resolve<'a>(
        &'a mut self,
        size_bits: u32,
        glyph: u16,
        scratch: &'a mut Image,
        render: impl FnOnce(&mut Image) -> Result<()>,
    ) -> Result<&'a Image> {
        if let Some(index) = self
            .entries
            .iter()
            .position(|e| e.size_bits == size_bits && e.glyph == glyph)
        {
            self.hits += 1;
            return Ok(&self.entries[index].mask);
        }
        self.misses += 1;
        render(scratch)?;
        // Move the backing allocation into the cache and charge its actual capacity.
        if self.max_entries > 0
            && scratch.data.len() <= self.max_bytes
            && scratch.data.capacity() > self.max_bytes
        {
            // A previous oversized glyph may have grown this temporary buffer.
            scratch.data.shrink_to_fit();
        }
        let bytes = scratch.data.capacity();
        if self.max_entries == 0 || bytes > self.max_bytes {
            return Ok(scratch);
        }
        while self.entries.len() >= self.max_entries || self.bytes > self.max_bytes - bytes {
            let oldest = self
                .entries
                .pop_front()
                .expect("a full budget has a resident entry");
            self.bytes -= oldest.mask.data.capacity();
        }
        self.bytes += bytes;
        self.entries.push_back(Entry {
            size_bits,
            glyph,
            mask: std::mem::take(scratch),
        });
        Ok(&self.entries.back().unwrap().mask)
    }
}
