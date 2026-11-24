//! Sampling in the fitted image viewport, independent of the current damage rectangle.
use solor::{
    ImageFill, ImageSampling, Pixels,
    layout::Size,
    raster::{Clip, DeviceSize, RasterScale},
};
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

    fn coordinate(&self, center: f64, sampling: ImageSampling) -> (usize, usize, u32) {
        let source = (center - self.start) * self.density;
        if sampling == ImageSampling::Nearest {
            let i = (source as usize).min(self.texels - 1);
            return (i, i, 0);
        }
        let v = (source - 0.5).clamp(0.0, (self.texels - 1) as f64);
        let i = v as usize;
        (
            i,
            (i + 1).min(self.texels - 1),
            ((v - i as f64) * 256.0).round() as u32,
        )
    }
}

struct Column {
    left: usize,
    right: usize,
    weight: u32,
    coverage: f64,
}

enum RowSampling {
    Scalar,
    Native(Clip),
    Filtered { first: i64, columns: Vec<Column> },
}

pub(super) struct ImageSampler<'a> {
    rows: RowSampling,
    sampling: ImageSampling,
    pixels: &'a [[u8; 4]],
    stride: usize,
    x: Axis,
    y: Axis,
    half_pixel: f64,
}

impl<'a> ImageSampler<'a> {
    pub(super) fn new(fill: &'a ImageFill, target: Size, scale: RasterScale) -> Result<Self> {
        let (offset, size) = fill.bounds(target).map_err(Error::other)?;
        let source = fill.source_bounds();
        let stride = fill.asset().dimensions().width as usize;
        let first = source.top as usize * stride + source.left as usize;
        Ok(Self {
            rows: RowSampling::Scalar,
            sampling: fill.sampling(),
            pixels: &fill.asset().pixels()[first..],
            stride,
            x: Axis::new(
                offset.x,
                size.width,
                target.width,
                (source.right - source.left) as u32,
            ),
            y: Axis::new(
                offset.y,
                size.height,
                target.height,
                (source.bottom - source.top) as u32,
            ),
            half_pixel: 0.5 / scale.0,
        })
    }

    /// Device bounds when a whole source texel lands exactly on each output pixel.
    fn native_bounds(&self, origin: [f64; 2], scale: RasterScale) -> Option<Clip> {
        if self.x.density != scale.0 || self.y.density != scale.0 {
            return None;
        }
        let x = origin[0] + self.x.start * scale.0;
        let y = origin[1] + self.y.start * scale.0;
        if x.fract() != 0.0 || y.fract() != 0.0 {
            return None;
        }
        let left = x as i64;
        let top = y as i64;
        Some(Clip {
            left,
            top,
            right: left.saturating_add(self.x.texels as i64),
            bottom: top.saturating_add(self.y.texels as i64),
        })
    }

    pub(super) fn prepare_rows(&mut self, origin: [f64; 2], scale: RasterScale, clip: Clip) {
        if let Some(rect) = self.native_bounds(origin, scale) {
            self.rows = RowSampling::Native(rect);
            return;
        }
        // Amortize a column table across rows of this repair, with bounded scratch
        // even for an extremely wide, shallow surface.
        if clip.bottom - clip.top < 2 || clip.right - clip.left > 8192 {
            return;
        }
        let columns = (clip.left..clip.right)
            .map(|x| {
                let center = (x as f64 + 0.5 - origin[0]) / scale.0;
                let (left, right, weight) = self.x.coordinate(center, self.sampling);
                Column {
                    left,
                    right,
                    weight,
                    coverage: self.x.coverage(center, self.half_pixel),
                }
            })
            .collect();
        self.rows = RowSampling::Filtered {
            first: clip.left,
            columns,
        };
    }

    pub(super) fn blend_span(
        &self,
        dst: &mut [[u8; 4]],
        x: i64,
        y: i64,
        origin: [f64; 2],
        scale: RasterScale,
    ) {
        let center_y = (y as f64 - origin[1] + 0.5) / scale.0;
        match &self.rows {
            RowSampling::Native(rect) => self.blend_native_span(dst, x, y, *rect),
            RowSampling::Scalar => {
                for (column, pixel) in (x..).zip(dst) {
                    self.blend(
                        pixel,
                        (column as f64 + 0.5 - origin[0]) / scale.0,
                        center_y,
                        255,
                    );
                }
            }
            RowSampling::Filtered { first, columns } => {
                let y_coverage = self.y.coverage(center_y, self.half_pixel);
                if y_coverage == 0.0 {
                    return;
                }
                let (top, bottom, wy) = self.y.coordinate(center_y, self.sampling);
                let (upper, lower) = self.source_rows(top, bottom);
                for (pixel, column) in dst.iter_mut().zip(&columns[(x - first) as usize..]) {
                    let coverage = (255.0 * column.coverage * y_coverage).round() as u8;
                    if coverage != 0 {
                        let src =
                            bilinear(upper, lower, column.left, column.right, column.weight, wy);
                        blend_sample(pixel, src, coverage);
                    }
                }
            }
        }
    }

    fn source_rows(&self, top: usize, bottom: usize) -> (&[[u8; 4]], &[[u8; 4]]) {
        let width = self.x.texels;
        let top = top * self.stride;
        let bottom = bottom * self.stride;
        (
            &self.pixels[top..top + width],
            &self.pixels[bottom..bottom + width],
        )
    }

    /// Composites the visible part of a fully covered row without filtering.
    fn blend_native_span(&self, dst: &mut [[u8; 4]], x: i64, y: i64, image: Clip) {
        let left = x.max(image.left);
        let right = (x + dst.len() as i64).min(image.right);
        if left >= right || !(image.top..image.bottom).contains(&y) {
            return;
        }
        let source = (y - image.top) as usize * self.stride + (left - image.left) as usize;
        let count = (right - left) as usize;
        let output = &mut dst[(left - x) as usize..(right - x) as usize];
        for (pixel, &texel) in output.iter_mut().zip(&self.pixels[source..source + count]) {
            crate::pixels::blend_premultiplied(pixel, texel);
        }
    }

    #[inline]
    pub(super) fn blend(&self, dst: &mut [u8; 4], x: f64, y: f64, coverage: u8) {
        let coverage = (f64::from(coverage)
            * self.x.coverage(x, self.half_pixel)
            * self.y.coverage(y, self.half_pixel))
        .round() as u8;
        if coverage == 0 {
            return;
        }
        let (left, right, wx) = self.x.coordinate(x, self.sampling);
        let (top, bottom, wy) = self.y.coordinate(y, self.sampling);
        let (upper, lower) = self.source_rows(top, bottom);
        let src = bilinear(upper, lower, left, right, wx, wy);
        blend_sample(dst, src, coverage);
    }
}

#[inline]
fn bilinear(
    upper_row: &[[u8; 4]],
    lower_row: &[[u8; 4]],
    left: usize,
    right: usize,
    wx: u32,
    wy: u32,
) -> [u8; 4] {
    let a = upper_row[left];
    if wx == 0 && wy == 0 {
        a
    } else {
        let b = upper_row[right];
        let c = lower_row[left];
        let d = lower_row[right];
        std::array::from_fn(|i| {
            let upper = u32::from(a[i]) * (256 - wx) + u32::from(b[i]) * wx;
            let lower = u32::from(c[i]) * (256 - wx) + u32::from(d[i]) * wx;
            ((upper * (256 - wy) + lower * wy + 32768) >> 16) as u8
        })
    }
}

#[inline]
fn blend_sample(dst: &mut [u8; 4], src: [u8; 4], coverage: u8) {
    let src = if coverage == 255 {
        src
    } else {
        src.map(|v| ((u16::from(v) * u16::from(coverage) + 127) / 255) as u8)
    };
    crate::pixels::blend_premultiplied(dst, src);
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

/// Project source pixel edges using the fitted device-space rectangle used for sampling.
pub(super) fn region_projection(
    image: &solor::CustomImage,
    target: Size,
    origin: solor::layout::Origin,
    scale: RasterScale,
) -> Result<impl Fn(solor::raster::Clip) -> solor::raster::Clip> {
    let (offset, fitted) = image.bounds(target).map_err(Error::other)?;
    let intrinsic = image.dimensions();
    let left = f64::from(origin.x) * scale.0 + f64::from(offset.x) * scale.0;
    let top = f64::from(origin.y) * scale.0 + f64::from(offset.y) * scale.0;
    let dx = f64::from(fitted.width) * scale.0 / f64::from(intrinsic.width);
    let dy = f64::from(fitted.height) * scale.0 / f64::from(intrinsic.height);
    Ok(move |region: solor::raster::Clip| solor::raster::Clip {
        left: (left + region.left as f64 * dx).floor() as i64,
        right: (left + region.right as f64 * dx).ceil() as i64,
        top: (top + region.top as f64 * dy).floor() as i64,
        bottom: (top + region.bottom as f64 * dy).ceil() as i64,
    })
}

impl RegionSampler {
    pub(super) fn new(
        image: &solor::CustomImage,
        origin: [f64; 2],
        target: Size,
        scale: RasterScale,
        clip: solor::raster::Clip,
    ) -> Result<Option<Self>> {
        Self::sample(
            origin,
            target,
            scale,
            clip,
            image.bounds(target).map_err(Error::other)?,
            image.dimensions(),
            |region| image.render(region),
        )
    }

    #[cfg(feature = "shader")]
    pub(super) fn shader(
        fill: &solor::ShaderFill,
        origin: [f64; 2],
        target: Size,
        scale: RasterScale,
        clip: Clip,
    ) -> Result<Option<Self>> {
        Self::sample(
            origin,
            target,
            scale,
            clip,
            (solor::layout::Origin::default(), target),
            DeviceSize {
                width: 1,
                height: 1,
            },
            |region| crate::shader::render(fill, region),
        )
    }

    fn sample(
        origin: [f64; 2],
        target: Size,
        scale: RasterScale,
        clip: Clip,
        placement: (solor::layout::Origin, Size),
        intrinsic: DeviceSize,
        render: impl FnOnce(solor::ImageRegion) -> anyhow::Result<solor::ImageAsset>,
    ) -> Result<Option<Self>> {
        let (offset, fitted) = placement;
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
            pixels: render(region).map_err(Error::other)?,
            bounds,
            left,
            top,
            x: Axis::new(offset.x, fitted.width, target.width, intrinsic.width),
            y: Axis::new(offset.y, fitted.height, target.height, intrinsic.height),
        }))
    }

    pub(super) fn blend_span(&self, dst: &mut [[u8; 4]], x: i64, y: i64, scale: RasterScale) {
        if self.x.start > 0.0
            || self.x.end < self.x.target
            || self.y.start > 0.0
            || self.y.end < self.y.target
        {
            for (x, pixel) in (x..).zip(dst) {
                self.blend(pixel, x, y, scale, 255);
            }
            return;
        }
        let left = x.max(self.bounds.left);
        let right = (x + dst.len() as i64).min(self.bounds.right);
        if left >= right || !(self.bounds.top..self.bounds.bottom).contains(&y) {
            return;
        }
        let dst = &mut dst[(left - x) as usize..(right - x) as usize];
        let first = (y - self.bounds.top) as usize * self.pixels.dimensions().width as usize
            + (left - self.bounds.left) as usize;
        let src = &self.pixels.pixels()[first..first + dst.len()];
        if src.iter().fold(255, |alpha, p| alpha & p[3]) == 255 {
            dst.copy_from_slice(src);
        } else {
            for (pixel, &color) in dst.iter_mut().zip(src) {
                crate::pixels::blend_premultiplied(pixel, color);
            }
        }
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
