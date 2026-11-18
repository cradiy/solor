use std::{io::Result, rc::Rc};

use crate::frame::Frame;
use crate::glyph_masks::GlyphMasks;
use crate::layout::Size;
use crate::pixels::{Clip, RasterScale, Surface};

fn damage(old: &Frame, new: &Frame, viewport: Clip, scale: RasterScale) -> Option<Clip> {
    let mut area: Option<Clip> = None;
    // Compare ordered slots. Insertions, removals and swaps dirty both occupants.
    for index in 0..old.draws.len().max(new.draws.len()) {
        let before = old.draws.get(index);
        let after = new.draws.get(index);
        if let (Some(a), Some(b)) = (before, after)
            && a.node == b.node
            && a.origin == b.origin
            && a.allocated == b.allocated
            && a.color == b.color
            && Rc::ptr_eq(&a.paragraph, &b.paragraph)
        {
            continue;
        }
        for draw in before.into_iter().chain(after) {
            let rect = scale.bounds(draw).intersect(viewport);
            if draw.color[3] == 0 || rect.empty() {
                continue;
            }
            area = Some(match area {
                None => rect,
                Some(a) => Clip {
                    left: a.left.min(rect.left),
                    top: a.top.min(rect.top),
                    right: a.right.max(rect.right),
                    bottom: a.bottom.max(rect.bottom),
                },
            });
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
    pub fn new(frame: Rc<Frame>, size: Size, clip: Clip, cache: &mut GlyphMasks) -> Result<Self> {
        Self::new_scaled(frame, size, clip, RasterScale::ONE, cache)
    }

    pub fn new_scaled(
        frame: Rc<Frame>,
        size: Size,
        clip: Clip,
        scale: RasterScale,
        cache: &mut GlyphMasks,
    ) -> Result<Self> {
        let surface = Surface::raster(&frame, size, clip, (0, 0), scale, Some(cache))?;
        Ok(Self {
            scale,
            surface,
            frame,
            clip,
        })
    }

    pub fn surface(&self) -> &Surface {
        &self.surface
    }

    /// Returns the physical region repaired in the retained surface.
    pub fn update(
        &mut self,
        frame: Rc<Frame>,
        size: Size,
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
                self.surface =
                    Surface::raster(&frame, size, clip, (0, 0), self.scale, Some(cache))?;
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
