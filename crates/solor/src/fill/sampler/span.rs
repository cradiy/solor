use super::{Gradient, GradientError, GradientKind, LinearProjection, OpaqueRamp, OpaqueSegment};
use crate::layout::{Origin, Size};
use crate::raster::RasterScale;

/// Samples fully covered spans of an opaque linear sRGB gradient into RGBA8 pixels.
pub struct OpaqueLinearGradient<'a> {
    gradient: &'a Gradient,
    projection: LinearProjection,
    size: Size,
    left: f64,
    top: f64,
    scale: RasterScale,
}

impl<'a> OpaqueLinearGradient<'a> {
    /// Prepares a draw in device coordinates if the gradient supports span interpolation.
    pub fn new(
        gradient: &'a Gradient,
        origin: Origin,
        size: Size,
        scale: RasterScale,
    ) -> Option<Self> {
        if gradient.kind != GradientKind::Linear
            || !OpaqueRamp::supports(gradient)
            || !size.width.is_extent()
            || !size.height.is_extent()
            || size.width.get() == 0.0
            || size.height.get() == 0.0
            || !scale.0.is_finite()
            || scale.0 <= 0.0
            || !gradient.stops().iter().all(|s| s.color.alpha == 255)
        {
            return None;
        }
        let (sin, cos) = (gradient.angle - 90.0).to_radians().sin_cos();
        let projection = LinearProjection::new(size.width.get(), size.height.get(), sin, cos);
        if !projection.length.is_finite() || projection.length <= 0.0 {
            return None;
        }
        Some(Self {
            gradient,
            projection,
            size,
            scale,
            left: f64::from(origin.x) * scale.0,
            top: f64::from(origin.y) * scale.0,
        })
    }

    /// Writes consecutive pixel centers starting at device coordinate `(x, y)`.
    pub fn write_span(&self, x: u32, y: u32, pixels: &mut [[u8; 4]]) -> Result<(), GradientError> {
        if pixels.is_empty() {
            return Ok(());
        }
        let local_y = ((f64::from(y) - self.top + 0.5) / self.scale.0) as f32;
        let row_offset = self.projection.dy * (local_y - self.size.height.get() * 0.5);
        let local_x =
            |i: usize| ((f64::from(x) + i as f64 - self.left + 0.5) / self.scale.0) as f32;
        if !local_y.is_finite() || !local_x(0).is_finite() || !local_x(pixels.len() - 1).is_finite()
        {
            return Err(GradientError::InvalidParameter);
        }
        let position = |i| {
            self.projection
                .position(local_x(i) - self.size.width.get() * 0.5, row_offset)
                .clamp(0.0, 1.0)
        };
        let last = position(pixels.len() - 1);
        if position(0).is_nan() || last.is_nan() {
            return Err(GradientError::InvalidOffset);
        }
        let stops = self.gradient.stops();
        let mut start = 0;
        while start < pixels.len() {
            let first = position(start);
            let upper = stops.partition_point(|stop| stop.offset <= first);
            let left = upper.checked_sub(1).map(|i| stops[i]);
            let right = stops.get(upper).copied();
            // Projection is monotonic along a scanline, including at coincident stops.
            let in_segment =
                |p| left.is_none_or(|s| s.offset <= p) && right.is_none_or(|s| p < s.offset);
            let mut end = pixels.len();
            if !in_segment(last) {
                let mut lo = start + 1;
                while lo < end {
                    let mid = lo + (end - lo) / 2;
                    if in_segment(position(mid)) {
                        lo = mid + 1;
                    } else {
                        end = mid;
                    }
                }
            }
            match (left, right) {
                (Some(a), Some(b)) => {
                    let segment = OpaqueSegment::new(a, b);
                    for (i, pixel) in (start..end).zip(&mut pixels[start..end]) {
                        let c = segment.color(position(i));
                        *pixel = [c.red, c.green, c.blue, 255];
                    }
                }
                _ => {
                    let c = stops[upper.saturating_sub(1)].color;
                    pixels[start..end].fill([c.red, c.green, c.blue, 255]);
                }
            }
            start = end;
        }
        Ok(())
    }
}
