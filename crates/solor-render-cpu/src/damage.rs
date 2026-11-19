use std::{io::Result, rc::Rc};

use crate::frame::Frame;
use crate::glyph_masks::GlyphMasks;
use crate::pixels::DeviceSize;
use crate::pixels::{Clip, RasterScale, Surface};

fn damage(old: &Frame, new: &Frame, viewport: Clip, scale: RasterScale) -> Option<Clip> {
    let mut area: Option<Clip> = None;
    let old_top = scale.edge(f64::from(old.scroll_y));
    let new_top = scale.edge(f64::from(new.scroll_y));
    // Compare ordered slots. Insertions, removals and swaps dirty both occupants.
    for index in 0..old.draws.len().max(new.draws.len()) {
        let before = old.draws.get(index);
        let after = new.draws.get(index);
        if let (Some(a), Some(b)) = (before, after)
            && old_top == new_top
            && a.node == b.node
            && a.origin == b.origin
            && a.allocated == b.allocated
            && a.color == b.color
            && Rc::ptr_eq(&a.paragraph, &b.paragraph)
        {
            continue;
        }
        for (draw, offset) in [(before, old_top), (after, new_top)] {
            let Some(draw) = draw else { continue };
            let mut rect = scale.bounds(draw.origin, draw.allocated);
            rect.top = rect.top.saturating_sub(offset);
            rect.bottom = rect.bottom.saturating_sub(offset);
            let rect = rect.intersect(viewport);
            if draw.color.alpha == 0 || rect.empty() {
                continue;
            }
            area = Some(area.map_or(rect, |old| old.union(rect)));
        }
    }
    area
}

/// The surface and its source frame move forward together under exclusive access.
pub struct RetainedPixels {
    surface: Surface,
    frame: Rc<Frame>,
    clip: Clip,
    scale: RasterScale,
}

impl RetainedPixels {
    /// Rasterizes the initial frame at unit scale.
    pub fn new(
        frame: Rc<Frame>,
        size: DeviceSize,
        clip: Clip,
        cache: &mut GlyphMasks,
    ) -> Result<Self> {
        Self::new_scaled(frame, size, clip, RasterScale::ONE, cache)
    }

    /// Rasterizes the initial frame at a fixed device scale.
    pub fn new_scaled(
        frame: Rc<Frame>,
        size: DeviceSize,
        clip: Clip,
        scale: RasterScale,
        cache: &mut GlyphMasks,
    ) -> Result<Self> {
        let surface = Surface::draw_scaled(&frame, size, clip, scale, Some(cache))?;
        Ok(Self {
            scale,
            surface,
            frame,
            clip,
        })
    }

    /// Borrows the complete image after the last successful update.
    pub fn surface(&self) -> &Surface {
        &self.surface
    }

    /// Returns the physical region repaired in the retained surface.
    pub fn update(
        &mut self,
        frame: Rc<Frame>,
        size: DeviceSize,
        clip: Clip,
        cache: &mut GlyphMasks,
    ) -> Result<Option<Clip>> {
        let canvas = Clip::canvas(size);
        let full = self.surface.size != size
            || self.clip != clip
            || !std::ptr::eq(self.frame.font, frame.font);
        let area = if full {
            Some(canvas)
        } else if Rc::ptr_eq(&self.frame, &frame) {
            None
        } else {
            damage(&self.frame, &frame, canvas.intersect(clip), self.scale)
        };
        match area {
            None => {}
            Some(a) if a == canvas => {
                self.surface = Surface::draw_scaled(&frame, size, clip, self.scale, Some(cache))?;
            }
            Some(a) => {
                let patch = Surface::patch(&frame, a, self.scale, cache)?;
                let stride = size.width as usize * 4;
                let width = patch.size.width as usize * 4;
                let first = a.top as usize * stride + a.left as usize * 4;
                for (index, row) in patch.rgba.chunks_exact(width).enumerate() {
                    let start = first + index * stride;
                    self.surface.rgba[start..start + width].copy_from_slice(row);
                }
            }
        };
        self.frame = frame;
        self.clip = clip;
        Ok(area)
    }
}
