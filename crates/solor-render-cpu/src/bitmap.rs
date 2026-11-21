//! Sampling in the fitted image viewport, independent of the current damage rectangle.
use solor::{ImageFill, Pixels, layout::Size, raster::RasterScale};
use std::io::{Error, Result};

struct Axis {
    start: f64,
    end: f64,
    target: f64,
    texels: usize,
    density: f64,
}

impl Axis {
    fn new(start: Pixels, fitted: Pixels, target: Pixels, texels: u32) -> Self {
        Self {
            start: start.into(),
            end: f64::from(start) + f64::from(fitted),
            target: target.into(),
            texels: texels as usize,
            density: if fitted.get() > 0.0 {
                f64::from(texels) / f64::from(fitted)
            } else {
                0.0
            },
        }
    }

    // The caller already applies coverage at the outside edge of the box.
    fn coverage(&self, center: f64, half_pixel: f64) -> f64 {
        if self.start <= 0.0 && self.end >= self.target {
            return 1.0;
        }
        let lo = (center - half_pixel).max(0.0);
        let hi = (center + half_pixel).min(self.target);
        if hi <= lo {
            return 0.0;
        }
        ((hi.min(self.end) - lo.max(self.start)) / (hi - lo)).clamp(0.0, 1.0)
    }

    fn coordinate(&self, center: f64) -> (usize, usize, u32) {
        let v = ((center - self.start) * self.density - 0.5).clamp(0.0, (self.texels - 1) as f64);
        let i = v as usize;
        (
            i,
            (i + 1).min(self.texels - 1),
            ((v - i as f64) * 256.0).round() as u32,
        )
    }
}

pub(super) struct ImageSampler<'a> {
    pixels: &'a [[u8; 4]],
    x: Axis,
    y: Axis,
    half_pixel: f64,
}

impl<'a> ImageSampler<'a> {
    pub(super) fn new(fill: &'a ImageFill, target: Size, scale: RasterScale) -> Result<Self> {
        let (offset, size) = fill.bounds(target).map_err(Error::other)?;
        let source = fill.asset().dimensions();
        Ok(Self {
            pixels: fill.asset().pixels(),
            x: Axis::new(offset.x, size.width, target.width, source.width),
            y: Axis::new(offset.y, size.height, target.height, source.height),
            half_pixel: 0.5 / scale.0,
        })
    }
    pub(super) fn blend(&self, dst: &mut [u8; 4], x: f64, y: f64, coverage: u8) {
        let coverage = (f64::from(coverage)
            * self.x.coverage(x, self.half_pixel)
            * self.y.coverage(y, self.half_pixel))
        .round() as u8;
        if coverage == 0 {
            return;
        }
        let (left, right, wx) = self.x.coordinate(x);
        let (top, bottom, wy) = self.y.coordinate(y);
        let stride = self.x.texels;
        let a = self.pixels[top * stride + left];
        let src = if wx == 0 && wy == 0 {
            a
        } else {
            let b = self.pixels[top * stride + right];
            let c = self.pixels[bottom * stride + left];
            let d = self.pixels[bottom * stride + right];
            std::array::from_fn(|i| {
                let upper = u32::from(a[i]) * (256 - wx) + u32::from(b[i]) * wx;
                let lower = u32::from(c[i]) * (256 - wx) + u32::from(d[i]) * wx;
                ((upper * (256 - wy) + lower * wy + 32768) >> 16) as u8
            })
        };
        let src = if coverage == 255 {
            src
        } else {
            src.map(|v| ((u16::from(v) * u16::from(coverage) + 127) / 255) as u8)
        };
        crate::pixels::blend_premultiplied(dst, src);
    }
}
