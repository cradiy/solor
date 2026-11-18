use std::io::{Error, Result};

use swash::FontRef;
use swash::scale::image::Image;
use swash::scale::{Render, ScaleContext, Source};

use crate::frame::{Frame, TextDraw};
use crate::glyph_masks::GlyphMasks;
use crate::layout::Size;

/// Half-open destination coordinates, in device pixels.
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

/// Device pixels per logical unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RasterScale(pub f64);

impl RasterScale {
    pub const ONE: Self = Self(1.0);

    fn edge(self, logical: f64) -> i64 {
        (logical * self.0).round() as i64
    }

    pub fn bounds(self, draw: &TextDraw) -> Clip {
        let x = f64::from(draw.origin.x);
        let y = f64::from(draw.origin.y);
        Clip {
            left: self.edge(x),
            top: self.edge(y),
            right: self.edge(x + f64::from(draw.allocated.width)),
            bottom: self.edge(y + f64::from(draw.allocated.height)),
        }
    }
}

pub struct Surface {
    pub size: Size,
    pub rgba: Vec<u8>,
}

impl Surface {
    pub fn draw(frame: &Frame, size: Size, clip: Clip) -> Result<Self> {
        Self::raster(frame, size, clip, (0, 0), RasterScale::ONE, None)
    }

    pub fn draw_cached(
        frame: &Frame,
        size: Size,
        clip: Clip,
        cache: &mut GlyphMasks,
    ) -> Result<Self> {
        Self::raster(frame, size, clip, (0, 0), RasterScale::ONE, Some(cache))
    }

    /// The caller supplies a nonempty region inside the validated destination.
    pub(super) fn patch(
        frame: &Frame,
        area: Clip,
        scale: RasterScale,
        cache: &mut GlyphMasks,
    ) -> Result<Self> {
        let size = Size {
            width: (area.right - area.left) as u32,
            height: (area.bottom - area.top) as u32,
        };
        Self::raster(
            frame,
            size,
            Clip::canvas(size),
            (area.left, area.top),
            scale,
            Some(cache),
        )
    }

    pub(super) fn raster(
        frame: &Frame,
        size: Size,
        clip: Clip,
        origin: (i64, i64),
        scale: RasterScale,
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
            let device = scale.bounds(draw);
            let bounds = viewport.intersect(Clip {
                left: device.left - origin.0,
                top: device.top - origin.1,
                right: device.right - origin.0,
                bottom: device.bottom - origin.1,
            });
            if bounds.empty() || draw.color[3] == 0 {
                continue;
            }
            let em = f32::from(draw.paragraph.style.size);
            let factor = scale.0 as f32;
            let device_em = em * factor;
            let mut scaler = context.builder(font).size(device_em).hint(false).build();
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
                        Some(cache) => {
                            cache.resolve(device_em.to_bits(), glyph.id, &mut scratch, rasterize)?
                        }
                        None => {
                            rasterize(&mut scratch)?;
                            &scratch
                        }
                    };
                    let x = scale
                        .edge(f64::from(draw.origin.x) + f64::from(glyph.x + em * glyph.x_offset))
                        - origin.0
                        + i64::from(mask.placement.left);
                    let y = scale.edge(
                        f64::from(draw.origin.y)
                            + f64::from(line.baseline + glyph.y - em * glyph.y_offset),
                    ) - origin.1
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
