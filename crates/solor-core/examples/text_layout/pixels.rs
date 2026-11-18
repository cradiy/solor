use std::io::{Error, Result};

use swash::FontRef;
use swash::scale::image::Image;
use swash::scale::{Render, ScaleContext, Source};

use crate::frame::Frame;
use crate::glyph_masks::GlyphMasks;
use crate::layout::Size;

/// Half-open destination coordinates, in device pixels at scale 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clip {
    pub left: i64,
    pub top: i64,
    pub right: i64,
    pub bottom: i64,
}

impl Clip {
    pub fn canvas(size: Size) -> Self {
        Self {
            left: 0,
            top: 0,
            right: size.width.into(),
            bottom: size.height.into(),
        }
    }

    pub fn intersect(self, other: Self) -> Self {
        Self {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        }
    }

    pub fn empty(self) -> bool {
        self.left >= self.right || self.top >= self.bottom
    }
}

pub struct Surface {
    pub size: Size,
    pub rgba: Vec<u8>,
}

impl Surface {
    pub fn draw(frame: &Frame, size: Size, clip: Clip) -> Result<Self> {
        Self::raster(frame, size, clip, (0, 0), None)
    }

    pub fn draw_cached(
        frame: &Frame,
        size: Size,
        clip: Clip,
        cache: &mut GlyphMasks,
    ) -> Result<Self> {
        Self::raster(frame, size, clip, (0, 0), Some(cache))
    }

    /// The caller supplies a nonempty region inside the validated destination.
    pub(super) fn patch(frame: &Frame, area: Clip, cache: &mut GlyphMasks) -> Result<Self> {
        let size = Size {
            width: (area.right - area.left) as u32,
            height: (area.bottom - area.top) as u32,
        };
        Self::raster(
            frame,
            size,
            Clip::canvas(size),
            (area.left, area.top),
            Some(cache),
        )
    }

    fn raster(
        frame: &Frame,
        size: Size,
        clip: Clip,
        origin: (i64, i64),
        mut cache: Option<&mut GlyphMasks>,
    ) -> Result<Self> {
        let pixels = u64::from(size.width) * u64::from(size.height);
        if pixels == 0 || pixels > 4 * 1024 * 1024 {
            return Err(Error::other("surface requires 1 to 4194304 pixels"));
        }
        let mut surface = Surface {
            size,
            rgba: vec![255; pixels as usize * 4],
        };
        let font =
            FontRef::from_index(frame.font, 0).ok_or_else(|| Error::other("invalid frame font"))?;
        if let Some(cache) = cache.as_deref_mut() {
            cache.bind(frame.font);
        }
        let mut context = ScaleContext::new();
        let mut scratch = Image::new();
        let renderer = Render::new(&[Source::Outline]);
        let viewport = Clip::canvas(size).intersect(clip);
        for draw in &frame.draws {
            let left = i64::from(draw.origin.x) - origin.0;
            let top = i64::from(draw.origin.y) - origin.1;
            let bounds = viewport.intersect(Clip {
                left,
                top,
                right: left + i64::from(draw.allocated.width),
                bottom: top + i64::from(draw.allocated.height),
            });
            if bounds.empty() || draw.color[3] == 0 {
                continue;
            }
            let em = f32::from(draw.paragraph.style.size);
            let mut scaler = context.builder(font).size(em).hint(false).build();
            for line in &draw.paragraph.lines {
                for glyph in &line.glyphs {
                    let mut rasterize = |mask: &mut Image| {
                        mask.clear();
                        if renderer.render_into(&mut scaler, glyph.id, mask) {
                            Ok(())
                        } else {
                            Err(Error::other("glyph outline unavailable"))
                        }
                    };
                    let mask = match cache.as_deref_mut() {
                        Some(cache) => cache.resolve(
                            draw.paragraph.style.size,
                            glyph.id,
                            &mut scratch,
                            rasterize,
                        )?,
                        None => {
                            rasterize(&mut scratch)?;
                            &scratch
                        }
                    };
                    let x = left
                        + (glyph.x + em * glyph.x_offset).round() as i64
                        + i64::from(mask.placement.left);
                    let y = top + (line.baseline + glyph.y - em * glyph.y_offset).round() as i64
                        - i64::from(mask.placement.top);
                    surface.composite(mask, x, y, bounds, draw.color);
                }
            }
        }
        Ok(surface)
    }

    fn composite(&mut self, mask: &Image, x: i64, y: i64, clip: Clip, color: [u8; 4]) {
        let pitch = mask.placement.width as usize;
        let area = clip.intersect(Clip {
            left: x,
            top: y,
            right: x + i64::from(mask.placement.width),
            bottom: y + i64::from(mask.placement.height),
        });
        if area.empty() {
            return;
        }
        for py in area.top..area.bottom {
            let offset = (py - y) as usize * pitch + (area.left - x) as usize;
            let len = (area.right - area.left) as usize;
            let dst = (py as usize * self.size.width as usize + area.left as usize) * 4;
            for (pixel, &coverage) in self.rgba[dst..dst + len * 4]
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(&mask.data[offset..offset + len])
            {
                let alpha = (u32::from(coverage) * u32::from(color[3]) + 127) / 255;
                for (channel, src) in pixel[..3].iter_mut().zip(color) {
                    *channel = ((u32::from(src) * alpha
                        + u32::from(*channel) * (255 - alpha)
                        + 127)
                        / 255) as u8;
                }
            }
        }
    }
}
