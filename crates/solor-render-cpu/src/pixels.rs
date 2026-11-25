use std::io::{Error, Result};

use solor::raster::glyphs::{GlyphMask as Image, GlyphRasterizer};

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
                &draw.color.into(),
                viewport,
            )?;
        }
        Ok(surface)
    }

    pub(super) fn cleared(size: DeviceSize, scale: RasterScale, clear_byte: u8) -> Result<Self> {
        Ok(Self {
            size,
            rgba: vec![clear_byte; Self::byte_len(size, scale)?],
        })
    }

    fn byte_len(size: DeviceSize, scale: RasterScale) -> Result<usize> {
        if !scale.0.is_finite() || scale.0 <= 0.0 {
            return Err(Error::other("invalid raster scale"));
        }
        let pixels = u64::from(size.width) * u64::from(size.height);
        if pixels == 0 || pixels > 4 * 1024 * 1024 {
            return Err(Error::other("surface requires 1 to 4194304 pixels"));
        }
        Ok(pixels as usize * 4)
    }

    pub(super) fn reset(
        &mut self,
        size: DeviceSize,
        scale: RasterScale,
        clear_byte: u8,
    ) -> Result<()> {
        let bytes = Self::byte_len(size, scale)?;
        // Every pixel will be repainted. Resize the allocation without preserving old rows.
        self.rgba.clear();
        if bytes < self.rgba.capacity() {
            self.rgba.shrink_to(bytes);
        } else {
            self.rgba.try_reserve_exact(bytes).map_err(Error::other)?;
        }
        self.rgba.resize(bytes, clear_byte);
        self.size = size;
        Ok(())
    }

    fn composite(
        &mut self,
        mask: &Image,
        x: i64,
        y: i64,
        clip: Clip,
        paint: &mut impl crate::mask::MaskPainter,
    ) -> Result<()> {
        let pitch = mask.placement.width as usize;
        let area = clip.intersect(Clip {
            left: x,
            top: y,
            right: x.saturating_add(i64::from(mask.placement.width)),
            bottom: y.saturating_add(i64::from(mask.placement.height)),
        });
        if area.empty() {
            return Ok(());
        }
        for py in area.top..area.bottom {
            let offset = (py - y) as usize * pitch + (area.left - x) as usize;
            let len = (area.right - area.left) as usize;
            let dst = (py as usize * self.size.width as usize + area.left as usize) * 4;
            paint.paint_row(
                self.rgba[dst..dst + len * 4].as_chunks_mut::<4>().0,
                &mask.data[offset..offset + len],
                area.left,
                py,
            )?;
        }
        Ok(())
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
    glyphs: GlyphRasterizer,
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
            glyphs: GlyphRasterizer::default(),
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
        color: &solor::Ink,
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
        if bounds.empty() || matches!(color, solor::Ink::Solid(c) if c.alpha == 0) {
            return Ok(());
        }
        let mut paint = crate::mask::painter(color, at, allocated, scale, origin, bounds)?;
        let document_clip = Clip {
            left: bounds.left.saturating_add(origin.0),
            right: bounds.right.saturating_add(origin.0),
            top: bounds.top.saturating_add(origin.1),
            bottom: bounds.bottom.saturating_add(origin.1),
        };
        self.glyphs.visit(
            paragraph,
            at,
            scale,
            document_clip,
            |key, [x, y], rasterize| {
                let mask = match self.cache.as_deref_mut() {
                    Some(cache) => cache.resolve(
                        key.face,
                        key.size_bits,
                        key.glyph,
                        key.y_phase,
                        &mut self.scratch,
                        rasterize,
                    )?,
                    None => {
                        rasterize(&mut self.scratch)?;
                        &self.scratch
                    }
                };
                surface.composite(
                    mask,
                    x.saturating_sub(origin.0)
                        .saturating_add(i64::from(mask.placement.left)),
                    y.saturating_sub(origin.1)
                        .saturating_sub(i64::from(mask.placement.top)),
                    bounds,
                    &mut paint,
                )
            },
        )
    }
}

#[cfg(test)]
#[path = "../tests/unit/blend.rs"]
mod tests;
