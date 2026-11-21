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
    /// Premultiplied RGBA bytes in top-to-bottom row order.
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
        cache: Option<&mut GlyphMasks>,
    ) -> Result<Self> {
        let mut surface = Self::cleared(size, scale, 255)?;
        // Sample a device-pixel crop of the document at the current scale.
        let origin = (
            origin.0,
            origin
                .1
                .saturating_add(scale.edge(f64::from(frame.scroll_y))),
        );
        let viewport = Clip::canvas(size).intersect(clip);
        let mut text = TextRaster::new(scale, origin, cache);
        for draw in &frame.draws {
            text.draw(
                &mut surface,
                draw.origin,
                draw.allocated,
                &draw.paragraph,
                draw.color,
                viewport,
            )?;
        }
        Ok(surface)
    }

    pub(super) fn cleared(size: DeviceSize, scale: RasterScale, clear_byte: u8) -> Result<Self> {
        if !scale.0.is_finite() || scale.0 <= 0.0 {
            return Err(Error::other("invalid raster scale"));
        }
        let pixels = u64::from(size.width) * u64::from(size.height);
        if pixels == 0 || pixels > 4 * 1024 * 1024 {
            return Err(Error::other("surface requires 1 to 4194304 pixels"));
        }
        Ok(Self {
            size,
            rgba: vec![clear_byte; pixels as usize * 4],
        })
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
                blend(pixel, color, coverage);
            }
        }
    }
}

pub(super) fn blend(pixel: &mut [u8; 4], color: solor::Rgba, coverage: u8) {
    let alpha = (u32::from(coverage) * u32::from(color.alpha) + 127) / 255;
    for (channel, src) in pixel
        .iter_mut()
        .zip([color.red, color.green, color.blue, 255])
    {
        *channel =
            ((u32::from(src) * alpha + u32::from(*channel) * (255 - alpha) + 127) / 255) as u8;
    }
}

pub(super) fn blend_premultiplied(dst: &mut [u8; 4], src: [u8; 4]) {
    match src[3] {
        0 => {}
        255 => *dst = src,
        alpha => {
            let inverse = u16::from(255 - alpha);
            for (dst, src) in dst.iter_mut().zip(src) {
                *dst = src + ((u16::from(*dst) * inverse + 127) / 255) as u8;
            }
        }
    }
}

// Solid spans share one effective alpha across all their destination pixels.
pub(super) fn blend_solid_span(pixels: &mut [[u8; 4]], color: solor::Rgba, coverage: u8) {
    let alpha = (u16::from(coverage) * u16::from(color.alpha) + 127) / 255;
    if alpha == 0 {
        return;
    }
    if alpha == 255 {
        pixels.fill([color.red, color.green, color.blue, 255]);
        return;
    }

    let inverse = 255 - alpha;
    // Source-over uses the same weights for premultiplied RGB and destination alpha.
    let source = [color.red, color.green, color.blue, 255].map(|c| u16::from(c) * alpha + 127);
    for pixel in pixels {
        for (channel, source) in pixel.iter_mut().zip(source) {
            *channel = ((source + u16::from(*channel) * inverse) / 255) as u8;
        }
    }
}

pub(super) struct TextRaster<'a> {
    context: ScaleContext,
    scratch: Image,
    scale: RasterScale,
    origin: (i64, i64),
    cache: Option<&'a mut GlyphMasks>,
}

impl<'a> TextRaster<'a> {
    pub(super) fn new(
        scale: RasterScale,
        origin: (i64, i64),
        cache: Option<&'a mut GlyphMasks>,
    ) -> Self {
        Self {
            context: ScaleContext::new(),
            scratch: Image::new(),
            scale,
            origin,
            cache,
        }
    }

    pub(super) fn draw(
        &mut self,
        surface: &mut Surface,
        at: solor::layout::Origin,
        allocated: solor::layout::Size,
        paragraph: &solor::text::Paragraph,
        color: solor::Rgba,
        viewport: Clip,
    ) -> Result<()> {
        let scale = self.scale;
        let origin = self.origin;
        let device = scale.bounds(at, allocated);
        let bounds = viewport.intersect(Clip {
            left: device.left.saturating_sub(origin.0),
            top: device.top.saturating_sub(origin.1),
            right: device.right.saturating_sub(origin.0),
            bottom: device.bottom.saturating_sub(origin.1),
        });
        if bounds.empty() || color.alpha == 0 {
            return Ok(());
        }
        let renderer = Render::new(&[Source::Outline]);
        let em = paragraph.style.size.get();
        let factor = scale.0 as f32;
        let device_em = em * factor;
        for line in &paragraph.lines {
            for run in line.glyphs.chunk_by(|a, b| a.face == b.face) {
                let face = &paragraph.faces[run[0].face];
                let font = FontRef::from_index(face.data(), face.index() as usize)
                    .ok_or_else(|| Error::other("invalid paragraph font"))?;
                let mut scaler = self
                    .context
                    .builder_with_id(font, [face.cache_key(), 0])
                    .size(device_em)
                    .variations([("wght", f32::from(face.weight()))])
                    .hint(false)
                    .build();
                for glyph in run {
                    let mut rasterize = |mask: &mut Image| {
                        mask.clear();
                        if renderer.render_into(&mut scaler, glyph.id, mask) {
                            Ok(())
                        } else {
                            Err(Error::other("glyph outline unavailable"))
                        }
                    };
                    let mask = match self.cache.as_deref_mut() {
                        Some(cache) => cache.resolve(
                            face.cache_key(),
                            device_em.to_bits(),
                            glyph.id,
                            &mut self.scratch,
                            rasterize,
                        )?,
                        None => {
                            rasterize(&mut self.scratch)?;
                            &self.scratch
                        }
                    };
                    let x = scale
                        .edge(f64::from(at.x) + f64::from(glyph.x + em * glyph.x_offset))
                        .saturating_sub(origin.0)
                        .saturating_add(i64::from(mask.placement.left));
                    let y = scale
                        .edge(
                            f64::from(at.y)
                                + f64::from(line.baseline + glyph.y - em * glyph.y_offset),
                        )
                        .saturating_sub(origin.1)
                        .saturating_sub(i64::from(mask.placement.top));
                    surface.composite(mask, x, y, bounds, color);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "../tests/unit/blend.rs"]
mod tests;
