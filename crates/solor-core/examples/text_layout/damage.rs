use std::{io::Result, rc::Rc};

use crate::frame::{Frame, TextDraw};
use crate::glyph_masks::GlyphMasks;
use crate::layout::Size;
use crate::pixels::{Clip, Surface};

fn bounds(draw: &TextDraw) -> Clip {
    let left = i64::from(draw.origin.x);
    let top = i64::from(draw.origin.y);
    Clip {
        left,
        top,
        right: left + i64::from(draw.allocated.width),
        bottom: top + i64::from(draw.allocated.height),
    }
}

fn damage(old: &Frame, new: &Frame, viewport: Clip) -> Option<Clip> {
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
            let rect = bounds(draw).intersect(viewport);
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
}

impl RetainedPixels {
    pub fn new(frame: Rc<Frame>, size: Size, clip: Clip, cache: &mut GlyphMasks) -> Result<Self> {
        let surface = Surface::draw_cached(&frame, size, clip, cache)?;
        Ok(Self {
            surface,
            frame,
            clip,
        })
    }

    pub fn surface(&self) -> &Surface {
        &self.surface
    }

    /// Returns the number of RGBA pixels prepared for this update.
    pub fn update(
        &mut self,
        frame: Rc<Frame>,
        size: Size,
        clip: Clip,
        cache: &mut GlyphMasks,
    ) -> Result<usize> {
        let canvas = Clip::canvas(size);
        let full = self.surface.size != size
            || self.clip != clip
            || !std::ptr::eq(self.frame.font, frame.font);
        let area = if full {
            Some(canvas)
        } else if Rc::ptr_eq(&self.frame, &frame) {
            None
        } else {
            damage(&self.frame, &frame, canvas.intersect(clip))
        };
        let count = match area {
            None => 0,
            Some(a) if a == canvas => {
                self.surface = Surface::draw_cached(&frame, size, clip, cache)?;
                self.surface.rgba.len() / 4
            }
            Some(a) => {
                let patch = Surface::patch(&frame, a, cache)?;
                let stride = size.width as usize * 4;
                let width = patch.size.width as usize * 4;
                let first = a.top as usize * stride + a.left as usize * 4;
                for (index, row) in patch.rgba.chunks_exact(width).enumerate() {
                    let start = first + index * stride;
                    self.surface.rgba[start..start + width].copy_from_slice(row);
                }
                patch.rgba.len() / 4
            }
        };
        self.frame = frame;
        self.clip = clip;
        Ok(count)
    }
}
