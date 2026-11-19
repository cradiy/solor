use std::io::{Error, Result};

use swash::FontRef;
use swash::scale::image::Image;
use swash::scale::{Render, ScaleContext, Source};

use crate::frame::Frame;
use crate::glyph_masks::GlyphMasks;
pub use solor::raster::{Clip, DeviceSize, RasterScale};

/// A tightly packed RGBA8 image in device pixels.
pub struct Surface {
    /// Physical extent of the image.
    pub size: DeviceSize,
    /// RGBA bytes in top-to-bottom row order.
    pub rgba: Vec<u8>,
}

impl Surface {
    /// Draws the clipped frame at unit scale.
    pub fn draw(frame: &Frame, size: DeviceSize, clip: Clip) -> Result<Self> {
        Self::draw_scaled(frame, size, clip, RasterScale::ONE, None)
    }

    /// Draws the clipped frame at unit scale using cached glyph masks.
    pub fn draw_cached(
        frame: &Frame,
        size: DeviceSize,
        clip: Clip,
        cache: &mut GlyphMasks,
    ) -> Result<Self> {
        Self::draw_scaled(frame, size, clip, RasterScale::ONE, Some(cache))
    }

    /// Rasterizes a clipped frame into a fresh surface at the given device scale.
    pub fn draw_scaled(
        frame: &Frame,
        size: DeviceSize,
        clip: Clip,
        scale: RasterScale,
        cache: Option<&mut GlyphMasks>,
    ) -> Result<Self> {
        Self::raster(frame, size, clip, (0, 0), scale, cache)
    }

    /// The caller supplies a nonempty region inside the validated destination.
    pub(super) fn patch(
        frame: &Frame,
        area: Clip,
        scale: RasterScale,
        cache: &mut GlyphMasks,
    ) -> Result<Self> {
        let size = DeviceSize {
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

    /// Rasterizes a device-pixel crop at the given scale and destination offset.
    fn raster(
        frame: &Frame,
        size: DeviceSize,
        clip: Clip,
        origin: (i64, i64),
        scale: RasterScale,
        mut cache: Option<&mut GlyphMasks>,
    ) -> Result<Self> {
        if !scale.0.is_finite() || scale.0 <= 0.0 {
            return Err(Error::other("invalid raster scale"));
        }
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
        // Sample a device-pixel crop of the document at the current scale.
        let origin = (
            origin.0,
            origin
                .1
                .saturating_add(scale.edge(f64::from(frame.scroll_y))),
        );
        let viewport = Clip::canvas(size).intersect(clip);
        for draw in &frame.draws {
            let device = scale.bounds(draw.origin, draw.allocated);
            let bounds = viewport.intersect(Clip {
                left: device.left.saturating_sub(origin.0),
                top: device.top.saturating_sub(origin.1),
                right: device.right.saturating_sub(origin.0),
                bottom: device.bottom.saturating_sub(origin.1),
            });
            if bounds.empty() || draw.color.alpha == 0 {
                continue;
            }
            let em = draw.paragraph.style.size.get();
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
                        .saturating_sub(origin.0)
                        .saturating_add(i64::from(mask.placement.left));
                    let y = scale
                        .edge(
                            f64::from(draw.origin.y)
                                + f64::from(line.baseline + glyph.y - em * glyph.y_offset),
                        )
                        .saturating_sub(origin.1)
                        .saturating_sub(i64::from(mask.placement.top));
                    surface.composite(mask, x, y, bounds, draw.color);
                }
            }
        }
        Ok(surface)
    }

    fn composite(&mut self, mask: &Image, x: i64, y: i64, clip: Clip, color: solor::Rgba) {
        let pitch = mask.placement.width as usize;
        let area = clip.intersect(Clip {
            left: x,
            top: y,
            right: x.saturating_add(i64::from(mask.placement.width)),
            bottom: y.saturating_add(i64::from(mask.placement.height)),
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
                let alpha = (u32::from(coverage) * u32::from(color.alpha) + 127) / 255;
                for (channel, src) in
                    pixel[..3]
                        .iter_mut()
                        .zip([color.red, color.green, color.blue])
                {
                    *channel = ((u32::from(src) * alpha
                        + u32::from(*channel) * (255 - alpha)
                        + 127)
                        / 255) as u8;
                }
            }
        }
    }
}
