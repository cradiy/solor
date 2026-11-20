use super::{ColorSpace, Gradient, GradientError, GradientKind, Rgba};
use crate::layout::Size;

impl Gradient {
    /// Prepares repeated lookups in one fill box, retaining its geometry and current color segment.
    pub fn sampler(
        &self,
        size: Size,
    ) -> Result<impl FnMut(f32, f32) -> Result<Rgba, GradientError> + '_, GradientError> {
        let project = self.projector(size)?;
        let mut ramp = OpaqueRamp::new(self);
        Ok(move |x, y| {
            let position = project(x, y)?;
            match &mut ramp {
                Some(ramp) => ramp.sample(position),
                None => self.sample(position),
            }
        })
    }

    pub(super) fn projector(
        &self,
        size: Size,
    ) -> Result<impl Fn(f32, f32) -> Result<f32, GradientError> + '_, GradientError> {
        let (w, h) = (size.width.get(), size.height.get());
        if !size.width.is_extent() || !size.height.is_extent() || w == 0.0 || h == 0.0 {
            return Err(GradientError::InvalidParameter);
        }
        let angle = (self.angle - 90.0).to_radians();
        let (sin, cos) = angle.sin_cos();
        let (dx, dy) = (cos * w / w.max(h), sin * h / w.max(h));
        let extent = if dx.abs() > dy.abs() { w } else { h };
        let length = dx.hypot(dy) * extent;
        Ok(move |x: f32, y: f32| {
            if !x.is_finite() || !y.is_finite() {
                return Err(GradientError::InvalidParameter);
            }
            let (x, y) = (x - w * 0.5, y - h * 0.5);
            let (qx, qy) = (2.0 * x / w, 2.0 * y / h);
            let t = match self.kind {
                GradientKind::Linear => 0.5 + (dx * x + dy * y) / length,
                GradientKind::Radial => qx.hypot(qy),
                GradientKind::Angular => {
                    ((qy.atan2(qx) - angle) / std::f32::consts::TAU).rem_euclid(1.0)
                }
                GradientKind::Diamond => (qx * cos + qy * sin).abs() + (qy * cos - qx * sin).abs(),
            };
            let position = if self.phase.is_some() {
                t.rem_euclid(1.0)
            } else {
                t.clamp(0.0, 1.0)
            };
            if position.is_nan() {
                return Err(GradientError::InvalidOffset);
            }
            Ok(position)
        })
    }
}

// For an opaque sRGB ramp with ordinary midpoints, premultiplied interpolation
// reduces to three linear channels. Reuse the interval across adjacent pixels.
struct OpaqueRamp<'a> {
    gradient: &'a Gradient,
    opaque: bool,
    start: f32,
    end: f32,
    base: [f32; 3],
    delta: [f32; 3],
}

impl<'a> OpaqueRamp<'a> {
    fn new(gradient: &'a Gradient) -> Option<Self> {
        if gradient.space != ColorSpace::Srgb
            || gradient.opacity != 1.0
            || gradient.phase.is_some()
            || gradient.midpoints.is_some()
            || (gradient.kind == GradientKind::Angular && gradient.seam_width > 0.0)
        {
            return None;
        }
        Some(Self {
            gradient,
            opaque: false,
            start: 0.0,
            end: 0.0,
            base: [0.0; 3],
            delta: [0.0; 3],
        })
    }

    fn sample(&mut self, position: f32) -> Result<Rgba, GradientError> {
        if position < self.start || position >= self.end {
            let stops = self.gradient.stops();
            let end = stops.partition_point(|s| s.offset <= position);
            if end == 0 {
                return Ok(stops[0].color);
            }
            let left = stops[end - 1];
            let Some(right) = stops.get(end) else {
                return Ok(left.color);
            };
            let channels = |c: Rgba| [c.red, c.green, c.blue].map(|v| f32::from(v) * (1.0 / 255.0));
            self.start = left.offset;
            self.end = right.offset;
            self.opaque = left.color.alpha == 255 && right.color.alpha == 255;
            if self.opaque {
                self.base = channels(left.color);
                let rhs = channels(right.color);
                self.delta = std::array::from_fn(|i| rhs[i] - self.base[i]);
            }
        }
        if !self.opaque {
            return self.gradient.sample(position);
        }
        let t = (position - self.start) / (self.end - self.start);
        let channel = |i: usize| ((self.base[i] + t * self.delta[i]) * 255.0 + 0.5) as u8;
        Ok(Rgba::new(channel(0), channel(1), channel(2), 255))
    }
}
