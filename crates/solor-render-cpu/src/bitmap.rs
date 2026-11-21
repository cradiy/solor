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

/// A rasterized region aligned to the destination pixel grid. Repairs use the same
/// source transform as a full redraw, independent of their offsets and extents.
pub(super) struct RegionSampler {
    pixels: solor::ImageAsset,
    bounds: solor::raster::Clip,
    x: Axis,
    y: Axis,
    left: f64,
    top: f64,
}

impl RegionSampler {
    pub(super) fn new(
        image: &solor::CustomImage,
        origin: [f64; 2],
        target: Size,
        scale: RasterScale,
        clip: solor::raster::Clip,
    ) -> Result<Option<Self>> {
        let (offset, fitted) = image.bounds(target).map_err(Error::other)?;
        let width = f64::from(fitted.width) * scale.0;
        let height = f64::from(fitted.height) * scale.0;
        if width <= 0.0 || height <= 0.0 {
            return Ok(None);
        }
        let [left, top] = origin;
        let x = f64::from(offset.x) * scale.0;
        let y = f64::from(offset.y) * scale.0;
        let bounds = clip.intersect(solor::raster::Clip {
            left: (left + x).floor() as i64,
            top: (top + y).floor() as i64,
            right: (left + x + width).ceil() as i64,
            bottom: (top + y + height).ceil() as i64,
        });
        if bounds.empty() {
            return Ok(None);
        }
        let intrinsic = image.dimensions();
        let step = [
            f64::from(intrinsic.width) / width,
            f64::from(intrinsic.height) / height,
        ];
        let region = solor::ImageRegion {
            output: solor::raster::DeviceSize {
                width: (bounds.right - bounds.left) as u32,
                height: (bounds.bottom - bounds.top) as u32,
            },
            source_origin: [
                (bounds.left as f64 - left - x) * step[0],
                (bounds.top as f64 - top - y) * step[1],
            ],
            source_step: step,
        };
        Ok(Some(Self {
            pixels: image.render(region).map_err(Error::other)?,
            bounds,
            left,
            top,
            x: Axis::new(offset.x, fitted.width, target.width, intrinsic.width),
            y: Axis::new(offset.y, fitted.height, target.height, intrinsic.height),
        }))
    }

    pub(super) fn blend(
        &self,
        dst: &mut [u8; 4],
        x: i64,
        y: i64,
        scale: RasterScale,
        coverage: u8,
    ) {
        if x < self.bounds.left
            || x >= self.bounds.right
            || y < self.bounds.top
            || y >= self.bounds.bottom
        {
            return;
        }
        let half = 0.5 / scale.0;
        let coverage = (f64::from(coverage)
            * self
                .x
                .coverage((x as f64 + 0.5 - self.left) / scale.0, half)
            * self.y.coverage((y as f64 + 0.5 - self.top) / scale.0, half))
        .round() as u8;
        let index = (y - self.bounds.top) as usize * self.pixels.dimensions().width as usize
            + (x - self.bounds.left) as usize;
        let src = self.pixels.pixels()[index];
        let src = if coverage == 255 {
            src
        } else {
            src.map(|v| ((u16::from(v) * u16::from(coverage) + 127) / 255) as u8)
        };
        crate::pixels::blend_premultiplied(dst, src);
    }
}
