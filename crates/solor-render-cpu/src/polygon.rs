//! Polygon spans with four vertical samples and fractional horizontal coverage.
use crate::{
    path::{PathBounds, device},
    pixels::{Surface, blend, blend_solid_span},
};
use solor::{
    Fill, FillRule,
    layout::{Origin, Size},
    px,
    raster::{Clip, RasterScale},
};
use std::{
    io::{Error, Result},
    rc::Rc,
};

struct Edge {
    x: f64,
    top: f64,
    bottom: f64,
    dx: f64,
    direction: i32,
}

// Convert a covered interval's endpoint into changes in pixel coverage.
// Quantize in device coordinates so splitting a repair never changes its pixels.
fn boundary(events: &mut Vec<(i64, i32)>, x: f64, sign: i32) {
    let fixed = (x * 256.0).round() as i64;
    let pixel = fixed / 256;
    let fraction = (fixed % 256) as i32;
    events.push((pixel, sign * (256 - fraction)));
    if fraction != 0 {
        events.push((pixel + 1, sign * fraction));
    }
}

fn for_each_span(
    contours: &[Rc<[Origin]>],
    rule: FillRule,
    origin: Origin,
    scale: RasterScale,
    clip: Clip,
    mut emit: impl FnMut(i64, i64, i64, u8) -> Result<()>,
) -> Result<()> {
    let mut edges = Vec::with_capacity(contours.iter().map(|ring| ring.len()).sum());
    for points in contours {
        let Some(&last) = points.last() else { continue };
        let mut previous = device(last, origin, scale);
        for &point in points.iter() {
            let current = device(point, origin, scale);
            let (a, b, direction) = if previous.1 <= current.1 {
                (previous, current, 1)
            } else {
                (current, previous, -1)
            };
            previous = current;
            if a.1 < b.1 && a.1 < clip.bottom as f64 && b.1 > clip.top as f64 {
                edges.push(Edge {
                    x: a.0,
                    top: a.1,
                    bottom: b.1,
                    dx: b.0 - a.0,
                    direction,
                });
            }
        }
    }
    edges.sort_unstable_by(|a, b| a.top.total_cmp(&b.top));
    let mut pending = edges.iter().peekable();
    let mut active: Vec<&Edge> = Vec::new();
    let mut crossings = Vec::new();
    let mut events = Vec::new();
    for row in clip.top..clip.bottom {
        events.clear();
        for offset in [0.125, 0.375, 0.625, 0.875] {
            let y = row as f64 + offset;
            while let Some(edge) = pending.next_if(|e| e.top <= y) {
                active.push(edge);
            }
            active.retain(|e| y < e.bottom);
            crossings.clear();
            crossings.extend(
                active
                    .iter()
                    .map(|e| (e.x + (y - e.top) / (e.bottom - e.top) * e.dx, e.direction)),
            );
            crossings.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
            let mut winding = 0;
            let mut start = 0.0;
            for &(x, direction) in &crossings {
                if winding == 0 {
                    start = x.max(clip.left as f64);
                }
                winding = match rule {
                    FillRule::EvenOdd => winding ^ 1,
                    FillRule::NonZero => winding + direction,
                };
                if winding == 0 {
                    let end = x.min(clip.right as f64);
                    if start < end {
                        boundary(&mut events, start, 1);
                        boundary(&mut events, end, -1);
                    }
                }
            }
        }
        events.sort_unstable_by_key(|e| e.0);
        let mut from = clip.left;
        let mut cover = 0;
        for &(to, delta) in &events {
            if from < to && cover > 0 {
                let alpha = ((cover * 255 + 512) / 1024) as u8;
                emit(row, from, to, alpha)?;
            }
            cover += delta;
            from = to;
        }
    }
    Ok(())
}

impl Surface {
    fn span_pixels(&mut self, row: i64, from: i64, to: i64) -> &mut [[u8; 4]] {
        let start = (row as usize * self.size.width as usize + from as usize) * 4;
        self.rgba[start..start + (to - from) as usize * 4]
            .as_chunks_mut::<4>()
            .0
    }

    pub(super) fn fill_polygon(
        &mut self,
        origin: Origin,
        contours: &[Rc<[Origin]>],
        fill: &Fill,
        rule: FillRule,
        scale: RasterScale,
        clip: Clip,
    ) -> Result<()> {
        match fill {
            Fill::Solid(color) => for_each_span(
                contours,
                rule,
                origin,
                scale,
                clip,
                |row, from, to, coverage| {
                    blend_solid_span(self.span_pixels(row, from, to), *color, coverage);
                    Ok(())
                },
            ),
            Fill::Checkerboard(pattern) => {
                let bounds = PathBounds::new(origin, contours, scale)?;
                let brush = crate::checkerboard::CheckerBrush::new(
                    *pattern,
                    bounds.left,
                    bounds.top,
                    scale,
                );
                for_each_span(
                    contours,
                    rule,
                    origin,
                    scale,
                    clip,
                    |row, from, to, coverage| {
                        brush.blend_span(self.span_pixels(row, from, to), from, row, coverage);
                        Ok(())
                    },
                )
            }
            Fill::DiagonalStripes(pattern) => {
                let bounds = PathBounds::new(origin, contours, scale)?;
                let brush =
                    crate::stripes::StripeBrush::new(*pattern, bounds.left, bounds.top, scale);
                for_each_span(
                    contours,
                    rule,
                    origin,
                    scale,
                    clip,
                    |row, from, to, coverage| {
                        brush.blend_span(self.span_pixels(row, from, to), from, row, coverage);
                        Ok(())
                    },
                )
            }
            Fill::CustomImage(image) => {
                let bounds = PathBounds::new(origin, contours, scale)?;
                let size = Size {
                    width: px(((bounds.right - bounds.left) / scale.0) as f32),
                    height: px(((bounds.bottom - bounds.top) / scale.0) as f32),
                };
                let Some(sampler) = crate::bitmap::RegionSampler::new(
                    image,
                    [bounds.left, bounds.top],
                    size,
                    scale,
                    clip,
                )?
                else {
                    return Ok(());
                };
                for_each_span(
                    contours,
                    rule,
                    origin,
                    scale,
                    clip,
                    |row, from, to, coverage| {
                        for (x, pixel) in (from..to).zip(self.span_pixels(row, from, to)) {
                            sampler.blend(pixel, x, row, scale, coverage);
                        }
                        Ok(())
                    },
                )
            }
            Fill::Image(image) => {
                let bounds = PathBounds::new(origin, contours, scale)?;
                let size = Size {
                    width: px(((bounds.right - bounds.left) / scale.0) as f32),
                    height: px(((bounds.bottom - bounds.top) / scale.0) as f32),
                };
                let sampler = crate::bitmap::ImageSampler::new(image, size, scale)?;
                for_each_span(
                    contours,
                    rule,
                    origin,
                    scale,
                    clip,
                    |row, from, to, coverage| {
                        let y = (row as f64 + 0.5 - bounds.top) / scale.0;
                        for (x, pixel) in (from..to).zip(self.span_pixels(row, from, to)) {
                            sampler.blend(
                                pixel,
                                (x as f64 + 0.5 - bounds.left) / scale.0,
                                y,
                                coverage,
                            );
                        }
                        Ok(())
                    },
                )
            }
            Fill::Gradient(gradient) => {
                let bounds = PathBounds::new(origin, contours, scale)?;
                if bounds.left == bounds.right || bounds.top == bounds.bottom {
                    return Ok(());
                }
                let mut sample = gradient
                    .sampler(Size {
                        width: px(((bounds.right - bounds.left) / scale.0) as f32),
                        height: px(((bounds.bottom - bounds.top) / scale.0) as f32),
                    })
                    .map_err(Error::other)?;
                for_each_span(
                    contours,
                    rule,
                    origin,
                    scale,
                    clip,
                    |row, from, to, coverage| {
                        let y = ((row as f64 + 0.5 - bounds.top) / scale.0) as f32;
                        for (x, pixel) in (from..to).zip(self.span_pixels(row, from, to)) {
                            let x = ((x as f64 + 0.5 - bounds.left) / scale.0) as f32;
                            blend(pixel, sample(x, y).map_err(Error::other)?, coverage);
                        }
                        Ok(())
                    },
                )
            }
            _ => Err(Error::new(
                std::io::ErrorKind::Unsupported,
                "unsupported CPU fill",
            )),
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/polygon.rs"]
mod tests;
