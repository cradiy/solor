//! A FIFO of grayscale outline masks keyed by face, device size, glyph and vertical subpixel position.
use std::collections::VecDeque;
use std::io::Result;

use solor::raster::glyphs::GlyphMask as Image;

struct Entry {
    face: u64,
    size_bits: u32,
    glyph: u16,
    y_phase: u8,
    mask: Image,
}

/// FIFO cache bounded by entry count and allocated mask bytes.
pub struct GlyphMasks {
    entries: VecDeque<Entry>,
    bytes: usize,
    max_entries: usize,
    max_bytes: usize,
    /// Number of lookups served from the cache.
    pub hits: usize,
    /// Number of lookups requiring rasterization.
    pub misses: usize,
}

impl GlyphMasks {
    /// Creates an empty cache with the supplied budgets.
    pub fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            bytes: 0,
            max_entries,
            max_bytes,
            hits: 0,
            misses: 0,
        }
    }

    /// Releases cached masks and their backing storage.
    pub fn release(&mut self) {
        self.entries = VecDeque::new();
        self.bytes = 0;
    }

    /// Returns the number of resident masks and their allocated pixel bytes.
    pub fn usage(&self) -> (usize, usize) {
        (self.entries.len(), self.bytes)
    }

    /// Borrows the resident mask or rasterizes it into the supplied scratch image.
    pub(crate) fn resolve<'a>(
        &'a mut self,
        face: u64,
        size_bits: u32,
        glyph: u16,
        y_phase: u8,
        scratch: &'a mut Image,
        render: impl FnOnce(&mut Image) -> Result<()>,
    ) -> Result<&'a Image> {
        if let Some(index) = self.entries.iter().position(|e| {
            e.face == face && e.size_bits == size_bits && e.glyph == glyph && e.y_phase == y_phase
        }) {
            self.hits = self.hits.saturating_add(1);
            return Ok(&self.entries[index].mask);
        }
        self.misses = self.misses.saturating_add(1);
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
            face,
            size_bits,
            glyph,
            y_phase,
            mask: std::mem::take(scratch),
        });
        Ok(&self.entries.back().unwrap().mask)
    }
}

#[cfg(test)]
#[path = "../tests/unit/glyph_masks.rs"]
mod tests;
