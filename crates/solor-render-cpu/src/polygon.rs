//! Even-odd polygon spans with four vertical samples and fractional horizontal coverage.
use crate::{
    path::{PathBounds, device},
    pixels::{Surface, blend},
};
use solor::{
    Fill,
    layout::{Origin, Size},
    px,
    raster::{Clip, RasterScale},
};
use std::io::{Error, Result};

struct Edge {
    x: f64,
    top: f64,
    bottom: f64,
    dx: f64,
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
    points: &[Origin],
    origin: Origin,
    scale: RasterScale,
    clip: Clip,
    mut emit: impl FnMut(i64, i64, i64, u8) -> Result<()>,
) -> Result<()> {
    let mut edges = Vec::with_capacity(points.len());
    let mut previous = device(points[points.len() - 1], origin, scale);
    for &point in points {
        let current = device(point, origin, scale);
        let (a, b) = if previous.1 <= current.1 {
            (previous, current)
        } else {
            (current, previous)
        };
        previous = current;
        if a.1 < b.1 && a.1 < clip.bottom as f64 && b.1 > clip.top as f64 {
            edges.push(Edge {
                x: a.0,
                top: a.1,
                bottom: b.1,
                dx: b.0 - a.0,
            });
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
                    .map(|e| e.x + (y - e.top) / (e.bottom - e.top) * e.dx),
            );
            crossings.sort_unstable_by(f64::total_cmp);
            for &[left, right] in crossings.as_chunks::<2>().0 {
                let left = left.max(clip.left as f64);
                let right = right.min(clip.right as f64);
                if left < right {
                    boundary(&mut events, left, 1);
                    boundary(&mut events, right, -1);
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
        points: &[Origin],
        fill: &Fill,
        scale: RasterScale,
        clip: Clip,
    ) -> Result<()> {
        match fill {
            Fill::Solid(color) => {
                for_each_span(points, origin, scale, clip, |row, from, to, coverage| {
                    let pixels = self.span_pixels(row, from, to);
                    if color.alpha == 255 && coverage == 255 {
                        pixels.fill([color.red, color.green, color.blue, 255]);
                    } else {
                        for pixel in pixels {
                            blend(pixel, *color, coverage);
                        }
                    }
                    Ok(())
                })
            }
            Fill::Gradient(gradient) => {
                let bounds = PathBounds::new(origin, points, scale)?;
                if bounds.left == bounds.right || bounds.top == bounds.bottom {
                    return Ok(());
                }
                let mut sample = gradient
                    .sampler(Size {
                        width: px(((bounds.right - bounds.left) / scale.0) as f32),
                        height: px(((bounds.bottom - bounds.top) / scale.0) as f32),
                    })
                    .map_err(Error::other)?;
                for_each_span(points, origin, scale, clip, |row, from, to, coverage| {
                    let y = ((row as f64 + 0.5 - bounds.top) / scale.0) as f32;
                    for (x, pixel) in (from..to).zip(self.span_pixels(row, from, to)) {
                        let x = ((x as f64 + 0.5 - bounds.left) / scale.0) as f32;
                        blend(pixel, sample(x, y).map_err(Error::other)?, coverage);
                    }
                    Ok(())
                })
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
