use crate::pixels::{blend, blend_solid_span};
use solor::{
    Ink, Rgba,
    layout::{Origin, Size},
    raster::RasterScale,
};
use std::io::{Error, Result};

pub(super) trait MaskPainter {
    fn paint_row(&mut self, pixels: &mut [[u8; 4]], mask: &[u8], x: i64, y: i64) -> Result<()>;
}

enum Brush<F> {
    Solid(Rgba),
    Gradient(F),
}

impl<F: FnMut(i64, i64) -> Result<Rgba>> MaskPainter for Brush<F> {
    fn paint_row(&mut self, pixels: &mut [[u8; 4]], mask: &[u8], x: i64, y: i64) -> Result<()> {
        match self {
            Self::Solid(color) => tint_row(pixels, mask, *color),
            Self::Gradient(sample) => {
                for (i, (dst, &coverage)) in pixels.iter_mut().zip(mask).enumerate() {
                    if coverage != 0 {
                        blend(dst, sample(x + i as i64, y)?, coverage);
                    }
                }
            }
        }
        Ok(())
    }
}

/// Device-space mask rows share one sampler anchored to the complete paint box.
pub(super) fn painter(
    ink: &Ink,
    at: Origin,
    size: Size,
    scale: RasterScale,
    offset: (i64, i64),
) -> Result<impl MaskPainter + '_> {
    match ink {
        Ink::Solid(color) => Ok(Brush::Solid(*color)),
        Ink::Gradient(g) => {
            let mut sample = g.sampler(size).map_err(Error::other)?;
            let local = move |device: i64, crop: i64, origin: solor::Pixels| {
                ((device as f64 + crop as f64 + 0.5) / scale.0 - f64::from(origin)) as f32
            };
            Ok(Brush::Gradient(move |x, y| {
                sample(local(x, offset.0, at.x), local(y, offset.1, at.y)).map_err(Error::other)
            }))
        }
    }
}

fn tint_row(row: &mut [[u8; 4]], mask: &[u8], ink: Rgba) {
    // Solid interiors and empty margins cover many consecutive pixels.
    let (blocks, tail) = row.as_chunks_mut::<8>();
    let (masks, tail_mask) = mask.as_chunks::<8>();
    for (pixels, coverage) in blocks.iter_mut().zip(masks) {
        match u64::from_ne_bytes(*coverage) {
            0 => {}
            u64::MAX => blend_solid_span(pixels, ink, 255),
            _ => tint_pixels(pixels, coverage, ink),
        }
    }
    tint_pixels(tail, tail_mask, ink);
}

fn tint_pixels(row: &mut [[u8; 4]], mask: &[u8], ink: Rgba) {
    for (dst, &coverage) in row.iter_mut().zip(mask) {
        match coverage {
            0 => {}
            255 if ink.alpha == 255 => *dst = [ink.red, ink.green, ink.blue, 255],
            _ => blend(dst, ink, coverage),
        }
    }
}
