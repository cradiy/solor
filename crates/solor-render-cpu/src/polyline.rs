//! Device-space coverage for a solid stroke with round caps and joins.
use crate::pixels::{Surface, blend};
use solor::{
    Pixels, Rgba,
    layout::Origin,
    raster::{Clip, RasterScale},
};

fn device(point: Origin, origin: Origin, scale: RasterScale) -> (f64, f64) {
    (
        (f64::from(point.x) + f64::from(origin.x)) * scale.0,
        (f64::from(point.y) + f64::from(origin.y)) * scale.0,
    )
}

pub(super) fn bounds(
    origin: Origin,
    points: &[Origin],
    width: Pixels,
    scale: RasterScale,
) -> std::io::Result<Clip> {
    let margin = f64::from(width) * scale.0 * 0.5 + 0.5;
    let (mut left, mut top) = (f64::INFINITY, f64::INFINITY);
    let (mut right, mut bottom) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for &point in points {
        let (x, y) = device(point, origin, scale);
        left = left.min(x);
        top = top.min(y);
        right = right.max(x);
        bottom = bottom.max(y);
    }
    if !margin.is_finite() || !(right - left).hypot(bottom - top).is_finite() {
        return Err(std::io::Error::other(
            "stroke exceeds device coordinate range",
        ));
    }
    Ok(Clip {
        left: (left - margin).floor() as i64,
        top: (top - margin).floor() as i64,
        right: (right + margin).ceil() as i64,
        bottom: (bottom + margin).ceil() as i64,
    })
}

impl Surface {
    pub(super) fn stroke_polyline(
        &mut self,
        origin: Origin,
        points: &[Origin],
        width: Pixels,
        color: Rgba,
        scale: RasterScale,
        clip: Clip,
    ) {
        let pitch = (clip.right - clip.left) as usize;
        let radius = f64::from(width) * scale.0 * 0.5;
        let reach = radius + 0.5;
        // A single mask composites joins, retraced segments and crossings exactly once.
        let mut mask = vec![0u8; pitch * (clip.bottom - clip.top) as usize];
        for edge in points.windows(2) {
            let (ax, ay) = device(edge[0], origin, scale);
            let (bx, by) = device(edge[1], origin, scale);
            let (dx, dy) = (bx - ax, by - ay);
            let length = dx.hypot(dy);
            let (ux, uy) = if length > 0.0 {
                (dx / length, dy / length)
            } else {
                (0.0, 0.0)
            };
            let top = clip.top.max((ay.min(by) - reach).floor() as i64);
            let bottom = clip.bottom.min((ay.max(by) + reach).ceil() as i64);
            for row in top..bottom {
                let y = row as f64 + 0.5 - ay;
                // Only visit the section of the segment that can reach this scanline.
                let (left, right) = if dy != 0.0 {
                    let x0 = ax + dx * ((y - reach) / dy).clamp(0.0, 1.0);
                    let x1 = ax + dx * ((y + reach) / dy).clamp(0.0, 1.0);
                    (x0.min(x1), x0.max(x1))
                } else {
                    (ax.min(bx), ax.max(bx))
                };
                let first = clip.left.max((left - reach).floor() as i64);
                let end = clip.right.min((right + reach).ceil() as i64);
                let row_start = (row - clip.top) as usize * pitch;
                for col in first..end {
                    let x = col as f64 + 0.5 - ax;
                    let along = (x * ux + y * uy).clamp(0.0, length);
                    let (ex, ey) = (x - along * ux, y - along * uy);
                    let distance = (ex * ex + ey * ey).sqrt();
                    let coverage = ((reach - distance).clamp(0.0, (2.0 * radius).min(1.0)) * 255.0)
                        .round() as u8;
                    let cell = &mut mask[row_start + (col - clip.left) as usize];
                    *cell = (*cell).max(coverage);
                }
            }
        }
        let stride = self.size.width as usize * 4;
        for (row, mask) in mask.chunks_exact(pitch).enumerate() {
            let start = (clip.top as usize + row) * stride + clip.left as usize * 4;
            let pixels = &mut self.rgba[start..start + pitch * 4];
            for (pixel, &coverage) in pixels.as_chunks_mut::<4>().0.iter_mut().zip(mask) {
                if coverage != 0 {
                    blend(pixel, color, coverage);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/polyline.rs"]
mod tests;
