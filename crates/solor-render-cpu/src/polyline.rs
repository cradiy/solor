//! Device-space coverage for a stroke with round caps and joins.
use crate::{
    mask::{self, MaskPainter},
    pixels::Surface,
};
use solor::{
    Ink, Pixels,
    layout::{Origin, Size},
    px,
    raster::{Clip, RasterScale},
};

use crate::path::{PathBounds, device};

use std::rc::Rc;

pub(super) struct StrokeContours<'a> {
    pub rings: &'a [Rc<[Origin]>],
    pub closed: bool,
}

pub(super) fn bounds(
    origin: Origin,
    rings: &[Rc<[Origin]>],
    width: Pixels,
    scale: RasterScale,
) -> std::io::Result<Clip> {
    PathBounds::new(origin, rings, scale)?.clip(f64::from(width) * scale.0 * 0.5 + 0.5)
}

impl Surface {
    pub(super) fn stroke_polyline(
        &mut self,
        origin: Origin,
        contours: StrokeContours<'_>,
        width: Pixels,
        ink: &Ink,
        scale: RasterScale,
        clip: Clip,
    ) -> std::io::Result<()> {
        let rings = contours.rings;
        let (at, size) = match ink {
            Ink::Solid(_) => (origin, Size::default()),
            Ink::Gradient(_) => {
                let b = PathBounds::new(origin, rings, RasterScale::ONE)?;
                let width = f64::from(width);
                (
                    Origin {
                        x: px((b.left - width * 0.5) as f32),
                        y: px((b.top - width * 0.5) as f32),
                    },
                    Size {
                        width: px((b.right - b.left + width) as f32),
                        height: px((b.bottom - b.top + width) as f32),
                    },
                )
            }
        };
        let mut painter = mask::painter(ink, at, size, scale, (0, 0))?;
        let pitch = (clip.right - clip.left) as usize;
        let radius = f64::from(width) * scale.0 * 0.5;
        let reach = radius + 0.5;
        // A single mask composites joins, retraced segments and crossings exactly once.
        let mut mask = vec![0u8; pitch * (clip.bottom - clip.top) as usize];
        let edges = rings.iter().flat_map(|points| {
            let closing_vertex = points.first().filter(|_| contours.closed);
            points
                .iter()
                .zip(points.iter().skip(1).chain(closing_vertex))
        });
        for (&start, &end) in edges {
            let (ax, ay) = device(start, origin, scale);
            let (bx, by) = device(end, origin, scale);
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
            painter.paint_row(
                pixels.as_chunks_mut::<4>().0,
                mask,
                clip.left,
                clip.top + row as i64,
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "../tests/unit/polyline.rs"]
mod tests;
