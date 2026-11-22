use crate::pixels::blend;
use solor::{Checkerboard, Rgba, raster::RasterScale};

pub(crate) struct CheckerBrush {
    color: Rgba,
    left: f64,
    top: f64,
    cell: f64,
}

impl CheckerBrush {
    pub(crate) fn new(pattern: Checkerboard, left: f64, top: f64, scale: RasterScale) -> Self {
        Self {
            color: pattern.color,
            left,
            top,
            cell: f64::from(pattern.cell_size()) * scale.0,
        }
    }

    // The integral of alternating +1/-1 cells is a triangle wave. Its difference
    // across a device pixel filters even cells smaller than that pixel.
    fn integral(&self, position: f64) -> f64 {
        let period = 2.0 * self.cell;
        let phase = position.rem_euclid(period);
        phase.min(period - phase)
    }

    pub(crate) fn blend_span(&self, pixels: &mut [[u8; 4]], from: i64, row: i64, coverage: u8) {
        let y = row as f64 - self.top;
        let vertical = self.integral(y + 1.) - self.integral(y);
        let mut previous = self.integral(from as f64 - self.left);
        for (x, pixel) in (from..).zip(pixels) {
            let next = self.integral((x + 1) as f64 - self.left);
            let fraction = 0.5 * (1.0 - (next - previous) * vertical);
            let covered = (fraction * f64::from(coverage)).round() as u8;
            if covered != 0 {
                blend(pixel, self.color, covered);
            }
            previous = next;
        }
    }
}
