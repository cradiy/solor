use crate::pixels::blend;
use solor::{DiagonalStripes, Rgba, raster::RasterScale};

pub(crate) struct StripeBrush {
    color: Rgba,
    origin_sum: f64,
    width: f64,
    period: f64,
    remainder: f64,
    whole_period_coverage: f64,
}

impl StripeBrush {
    pub(crate) fn new(pattern: DiagonalStripes, left: f64, top: f64, scale: RasterScale) -> Self {
        // The stripe coordinate is x + y, so its period along a row is sqrt(2)
        // times the physical spacing measured perpendicular to the lines.
        let axis_scale = scale.0 * std::f64::consts::SQRT_2;
        let width = f64::from(pattern.width()) * axis_scale;
        let period = width + f64::from(pattern.gap()) * axis_scale;
        let remainder = 1.0 % period;
        // Whole periods along either axis contribute their mean ink density.
        // Only the remaining square depends on the pixel's position.
        let whole_period_coverage = (1.0 - remainder * remainder) * (width / period);
        Self {
            color: pattern.color,
            origin_sum: left + top,
            width,
            period,
            remainder,
            whole_period_coverage,
        }
    }

    fn coverage(&self, coordinate: f64) -> f64 {
        let phase = coordinate.rem_euclid(self.period);
        let side = self.remainder;
        let mut area = self.whole_period_coverage;
        let mut start = -phase;
        // The remaining square intersects at most three bands, even for subpixel periods.
        while start < 2.0 * side {
            area += area_below(start + self.width, side) - area_below(start, side);
            start += self.period;
        }
        area
    }

    pub(crate) fn blend_span(&self, pixels: &mut [[u8; 4]], from: i64, row: i64, coverage: u8) {
        for (x, pixel) in (from..).zip(pixels) {
            let area = self.coverage((x + row) as f64 - self.origin_sum);
            let covered = (area * f64::from(coverage)).round() as u8;
            if covered != 0 {
                blend(pixel, self.color, covered);
            }
        }
    }
}

// Area of the square [0, side]² where x + y is below the cutoff.
fn area_below(cutoff: f64, side: f64) -> f64 {
    if cutoff <= side {
        0.5 * cutoff.max(0.0).powi(2)
    } else {
        side * side - 0.5 * (2.0 * side - cutoff).max(0.0).powi(2)
    }
}
