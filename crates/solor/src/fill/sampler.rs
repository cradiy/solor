use super::{ColorSpace, ColorStop, Gradient, GradientError, GradientKind, Rgba};
use crate::layout::Size;

mod span;
pub use span::OpaqueLinearGradient;

impl Gradient {
    /// Prepares repeated lookups in one fill box, retaining its geometry and current color segment.
    pub fn sampler(
        &self,
        size: Size,
    ) -> Result<impl FnMut(f32, f32) -> Result<Rgba, GradientError> + '_, GradientError> {
        let project = self.projector(size)?;
        let mut sample = self.ramp_sampler();
        Ok(move |x, y| sample(project(x, y)?))
    }

    /// Samples successive normalized positions, reusing the current stop interval.
    pub fn ramp_sampler(&self) -> impl FnMut(f32) -> Result<Rgba, GradientError> + '_ {
        let mut ramp = OpaqueRamp::new(self);
        move |position| {
            if !(0.0..=1.0).contains(&position) {
                return Err(GradientError::InvalidOffset);
            }
            match &mut ramp {
                Some(ramp) => ramp.sample(position),
                None => self.sample(position),
            }
        }
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
        let linear = LinearProjection::new(w, h, sin, cos);
        Ok(move |x: f32, y: f32| {
            if !x.is_finite() || !y.is_finite() {
                return Err(GradientError::InvalidParameter);
            }
            let (x, y) = (x - w * 0.5, y - h * 0.5);
            let (qx, qy) = (2.0 * x / w, 2.0 * y / h);
            let t = match self.kind {
                GradientKind::Linear => linear.position(x, linear.dy * y),
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

struct LinearProjection {
    dx: f32,
    dy: f32,
    length: f32,
}

impl LinearProjection {
    fn new(w: f32, h: f32, sin: f32, cos: f32) -> Self {
        let (dx, dy) = (cos * w / w.max(h), sin * h / w.max(h));
        let extent = if dx.abs() > dy.abs() { w } else { h };
        Self {
            dx,
            dy,
            length: dx.hypot(dy) * extent,
        }
    }

    fn position(&self, centered_x: f32, row_offset: f32) -> f32 {
        0.5 + (self.dx * centered_x + row_offset) / self.length
    }
}

struct OpaqueSegment {
    start: f32,
    width: f32,
    base: [f32; 3],
    delta: [f32; 3],
}

impl OpaqueSegment {
    fn new(left: ColorStop, right: ColorStop) -> Self {
        let channels = |c: Rgba| [c.red, c.green, c.blue].map(|v| f32::from(v) * (1.0 / 255.0));
        let base = channels(left.color);
        let rhs = channels(right.color);
        Self {
            start: left.offset,
            width: right.offset - left.offset,
            base,
            delta: std::array::from_fn(|i| rhs[i] - base[i]),
        }
    }

    fn color(&self, position: f32) -> Rgba {
        let t = (position - self.start) / self.width;
        let channel = |i: usize| ((self.base[i] + t * self.delta[i]) * 255.0 + 0.5) as u8;
        Rgba::new(channel(0), channel(1), channel(2), 255)
    }
}

// For an opaque sRGB ramp with ordinary midpoints, premultiplied interpolation
// reduces to three linear channels. Reuse the interval across adjacent pixels.
struct OpaqueRamp<'a> {
    gradient: &'a Gradient,
    segment: Option<OpaqueSegment>,
    start: f32,
    end: f32,
}

impl<'a> OpaqueRamp<'a> {
    fn supports(gradient: &Gradient) -> bool {
        gradient.space == ColorSpace::Srgb
            && gradient.opacity == 1.0
            && gradient.phase.is_none()
            && gradient.midpoints.is_none()
            && (gradient.kind != GradientKind::Angular || gradient.seam_width == 0.0)
    }

    fn new(gradient: &'a Gradient) -> Option<Self> {
        Self::supports(gradient).then_some(Self {
            gradient,
            segment: None,
            start: 0.0,
            end: 0.0,
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
            self.start = left.offset;
            self.end = right.offset;
            self.segment = (left.color.alpha == 255 && right.color.alpha == 255)
                .then(|| OpaqueSegment::new(left, *right));
        }
        match &self.segment {
            Some(segment) => Ok(segment.color(position)),
            None => self.gradient.sample(position),
        }
    }
}
