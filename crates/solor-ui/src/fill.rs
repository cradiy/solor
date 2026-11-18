//! Immutable sRGB colors and fills for local drawing commands.
use std::{error::Error, fmt, sync::Arc};

/// Straight (unpremultiplied) 8-bit sRGB channels with linear alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgba {
    /// Red sRGB channel.
    pub red: u8,
    /// Green sRGB channel.
    pub green: u8,
    /// Blue sRGB channel.
    pub blue: u8,
    /// Linear opacity, from transparent at zero to opaque at 255.
    pub alpha: u8,
}

impl Rgba {
    /// Creates a color from straight sRGB channels and linear alpha.
    pub const fn new(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }
}

impl From<[u8; 4]> for Rgba {
    fn from([red, green, blue, alpha]: [u8; 4]) -> Self {
        Self::new(red, green, blue, alpha)
    }
}

/// A color at a normalized position on a gradient ramp.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorStop {
    /// Position along the ramp, from zero to one.
    pub offset: f32,
    /// Color at this position.
    pub color: Rgba,
}

impl ColorStop {
    /// Places a color at the supplied normalized offset.
    pub const fn new(offset: f32, color: Rgba) -> Self {
        Self { offset, color }
    }
}

/// Construction, editing or sampling failure for a gradient.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradientError {
    /// The angle is infinite or NaN.
    NonFiniteAngle,
    /// The ramp has fewer than two stops.
    TooFewStops,
    /// The stop or sample position falls outside the unit interval.
    InvalidOffset,
    /// Adjacent stops are out of ascending order.
    UnorderedStops,
    /// The requested stop or segment does not exist.
    InvalidSegment,
    /// A sampling or interpolation parameter is outside its supported range.
    InvalidParameter,
}

impl fmt::Display for GradientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NonFiniteAngle => "gradient angle must be finite",
            Self::TooFewStops => "a gradient needs at least two stops",
            Self::InvalidOffset => "gradient offsets must be finite and between zero and one",
            Self::InvalidSegment => "gradient stop or segment index out of bounds",
            Self::InvalidParameter => "invalid gradient sampling parameter",
            Self::UnorderedStops => "gradient stops must be in nondecreasing order",
        })
    }
}
impl Error for GradientError {}

/// Reads packed red, green and blue bytes with full opacity.
pub const fn rgb(hex: u32) -> Rgba {
    Rgba::new((hex >> 16) as u8, (hex >> 8) as u8, hex as u8, 255)
}
/// Reads packed red, green, blue and alpha bytes, with alpha last.
pub const fn rgba(hex: u32) -> Rgba {
    Rgba::new(
        (hex >> 24) as u8,
        (hex >> 16) as u8,
        (hex >> 8) as u8,
        hex as u8,
    )
}
/// Places a color at a normalized position on the ramp.
pub const fn color_stop(color: Rgba, position: f32) -> ColorStop {
    ColorStop::new(position, color)
}

/// Geometry mapping local coordinates to positions along the ramp.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GradientKind {
    #[default]
    /// Parallel bands along the selected direction.
    Linear,
    /// Elliptical distance from the center of the bounds.
    Radial,
    /// Clockwise sweep around the center.
    Angular,
    /// Distance from the center along two rotated axes.
    Diamond,
}
/// Interpolation space for mixing neighboring stop colors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorSpace {
    #[default]
    /// Interpolates encoded sRGB components.
    Srgb,
    /// Interpolates perceptual Oklab components.
    Oklab,
}

/// A shared stop sequence and the parameters used to sample it.
#[derive(Clone, Debug)]
pub struct Gradient {
    kind: GradientKind,
    angle: f32,
    space: ColorSpace,
    phase: Option<f32>,
    seam_width: f32,
    opacity: f32,
    stops: Arc<[ColorStop]>,
    midpoints: Option<Arc<[f32]>>,
}

fn shared_eq<T: PartialEq>(a: &Arc<[T]>, b: &Arc<[T]>) -> bool {
    Arc::ptr_eq(a, b) || a == b
}

impl PartialEq for Gradient {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.angle == other.angle
            && self.space == other.space
            && self.phase == other.phase
            && self.seam_width == other.seam_width
            && self.opacity == other.opacity
            && shared_eq(&self.stops, &other.stops)
            && match (&self.midpoints, &other.midpoints) {
                (Some(a), Some(b)) => shared_eq(a, b),
                (None, None) => true,
                (Some(points), None) | (None, Some(points)) => points.iter().all(|&p| p == 0.5),
            }
    }
}

/// Builds a linear ramp from ordered stops at a clockwise angle in degrees.
pub fn linear_gradient(
    angle: f32,
    stops: impl IntoIterator<Item = ColorStop>,
) -> Result<Gradient, GradientError> {
    Gradient::new(GradientKind::Linear, angle, stops)
}

macro_rules! directional_gradients {
    ($($name:ident => $angle:expr),+ $(,)?) => {$(
        /// Builds a directional linear gradient from ordered stops.
        pub fn $name(stops: impl IntoIterator<Item = ColorStop>) -> Result<Gradient, GradientError> {
            linear_gradient($angle, stops)
        }
    )+};
}
directional_gradients!(
    gradient_left_to_right => 90.0,
    gradient_right_to_left => 270.0,
    gradient_top_to_bottom => 180.0,
    gradient_bottom_to_top => 0.0,
);

impl Gradient {
    /// Ordered stops share storage across clones. Zero degrees points up, increasing clockwise.
    pub fn new(
        kind: GradientKind,
        angle: f32,
        stops: impl IntoIterator<Item = ColorStop>,
    ) -> Result<Self, GradientError> {
        if !angle.is_finite() {
            return Err(GradientError::NonFiniteAngle);
        }
        let stops: Arc<[ColorStop]> = stops.into_iter().collect();
        if stops.len() < 2 {
            return Err(GradientError::TooFewStops);
        }
        if stops.iter().any(|s| !(0.0..=1.0).contains(&s.offset)) {
            return Err(GradientError::InvalidOffset);
        }
        if stops.windows(2).any(|s| s[0].offset > s[1].offset) {
            return Err(GradientError::UnorderedStops);
        }
        Ok(Self {
            kind,
            angle: angle.rem_euclid(360.0),
            space: ColorSpace::Srgb,
            stops,
            midpoints: None,
            phase: None,
            seam_width: 0.0,
            opacity: 1.0,
        })
    }
    /// Returns the geometry used to map points onto the ramp.
    pub fn kind(&self) -> GradientKind {
        self.kind
    }
    /// Returns the color space used between stops.
    pub fn interpolation_space(&self) -> ColorSpace {
        self.space
    }
    /// Returns the repeating phase in turns when periodic sampling is enabled.
    pub fn repeat_phase(&self) -> Option<f32> {
        self.phase
    }
    /// Returns the angular seam blend width in turns.
    pub fn seam_width(&self) -> f32 {
        self.seam_width
    }
    /// Returns the opacity multiplier applied to sampled colors.
    pub fn alpha_multiplier(&self) -> f32 {
        self.opacity
    }
    /// Returns the half-mix position within a segment, if it exists.
    pub fn gradient_midpoint_at(&self, segment: usize) -> Option<f32> {
        (segment < self.stops.len() - 1)
            .then(|| self.midpoints.as_ref().map_or(0.5, |m| m[segment]))
    }
    /// Selects the geometry while retaining the same stop sequence.
    pub fn gradient_kind(mut self, kind: GradientKind) -> Self {
        self.kind = kind;
        self
    }
    /// Selects the interpolation space for the ramp.
    pub fn color_space(mut self, space: ColorSpace) -> Self {
        self.space = space;
        self
    }
    /// Returns the normalized clockwise angle from the upward direction.
    pub fn angle_degrees(&self) -> f32 {
        self.angle
    }
    /// Borrows the ordered stop sequence.
    pub fn stops(&self) -> &[ColorStop] {
        &self.stops
    }

    /// Editing shared stops detaches this gradient, leaving recorded commands intact.
    pub fn set_gradient_stop(
        &mut self,
        index: usize,
        stop: ColorStop,
    ) -> Result<(), GradientError> {
        let stops = self.stops();
        let old = stops.get(index).ok_or(GradientError::InvalidSegment)?;
        if !(0.0..=1.0).contains(&stop.offset) {
            return Err(GradientError::InvalidOffset);
        }
        if (index > 0 && stops[index - 1].offset > stop.offset)
            || stops.get(index + 1).is_some_and(|s| s.offset < stop.offset)
        {
            return Err(GradientError::UnorderedStops);
        }
        if *old != stop {
            Arc::make_mut(&mut self.stops)[index] = stop;
        }
        Ok(())
    }

    /// Places the half-mix point within the specified segment.
    pub fn gradient_midpoint(
        mut self,
        segment: usize,
        midpoint: f32,
    ) -> Result<Self, GradientError> {
        let count = self.stops().len() - 1;
        if segment >= count {
            return Err(GradientError::InvalidSegment);
        }
        if !(midpoint > 0.0 && midpoint < 1.0) {
            return Err(GradientError::InvalidParameter);
        }
        if self
            .midpoints
            .as_ref()
            .map_or(0.5, |points| points[segment])
            != midpoint
        {
            let points = self
                .midpoints
                .get_or_insert_with(|| vec![0.5; count].into());
            Arc::make_mut(points)[segment] = midpoint;
        }
        Ok(self)
    }

    /// Multiplies the sampled alpha by a factor between zero and one.
    pub fn opacity(mut self, factor: f32) -> Result<Self, GradientError> {
        if !(0.0..=1.0).contains(&factor) {
            return Err(GradientError::InvalidParameter);
        }
        self.opacity *= factor;
        Ok(self)
    }

    /// Enables periodic cubic spline sampling and offsets it by a fraction of a turn.
    pub fn phase(mut self, phase: f32) -> Result<Self, GradientError> {
        if !phase.is_finite() {
            return Err(GradientError::InvalidParameter);
        }
        self.phase = Some(phase.rem_euclid(1.0));
        Ok(self)
    }

    /// Sets the blend width around a non-repeating angular ramp's seam, in turns.
    pub fn angular_seam_width(mut self, width: f32) -> Result<Self, GradientError> {
        if !(0.0..=0.5).contains(&width) {
            return Err(GradientError::InvalidParameter);
        }
        self.seam_width = width;
        Ok(self)
    }

    /// Samples local pixel coordinates inside a box. End colors extend outside the stop range.
    pub fn sample_at(
        &self,
        x: f32,
        y: f32,
        size: crate::layout::Size,
    ) -> Result<Rgba, GradientError> {
        if !size.width.is_extent()
            || !size.height.is_extent()
            || size.width.get() == 0.0
            || size.height.get() == 0.0
            || !x.is_finite()
            || !y.is_finite()
        {
            return Err(GradientError::InvalidParameter);
        }
        let (w, h) = (size.width.get(), size.height.get());
        let (x, y) = (x - w * 0.5, y - h * 0.5);
        let (qx, qy) = (2.0 * x / w, 2.0 * y / h);
        let angle = (self.angle - 90.0).to_radians();
        let (sin, cos) = angle.sin_cos();
        let t = match self.kind {
            GradientKind::Linear => {
                let (dx, dy) = (cos * w / w.max(h), sin * h / w.max(h));
                let extent = if dx.abs() > dy.abs() { w } else { h };
                0.5 + (dx * x + dy * y) / (dx.hypot(dy) * extent)
            }
            GradientKind::Radial => qx.hypot(qy),
            GradientKind::Angular => {
                ((qy.atan2(qx) - angle) / std::f32::consts::TAU).rem_euclid(1.0)
            }
            GradientKind::Diamond => (qx * cos + qy * sin).abs() + (qy * cos - qx * sin).abs(),
        };
        if self.phase.is_some() {
            self.sample(t.rem_euclid(1.0))
        } else {
            self.sample(t.clamp(0.0, 1.0))
        }
    }

    /// Samples the one-dimensional gradient ramp.
    pub fn sample(&self, position: f32) -> Result<Rgba, GradientError> {
        if !(0.0..=1.0).contains(&position) {
            return Err(GradientError::InvalidOffset);
        }
        let value = match self.space {
            ColorSpace::Srgb => self.sample_space::<color::Srgb>(position),
            ColorSpace::Oklab => self.sample_space::<color::Oklab>(position),
        };
        Ok(value)
    }

    fn sample_space<C: color::ColorSpace>(&self, position: f32) -> Rgba {
        let stops = self.stops();
        let at = |index: usize| {
            let c = stops[index].color;
            color::AlphaColor::<color::Srgb>::from_rgba8(c.red, c.green, c.blue, c.alpha)
                .convert::<C>()
        };
        let ramp = |p: f32| {
            let end = stops.partition_point(|s| s.offset <= p);
            if end == 0 {
                return at(0);
            }
            if end == stops.len() {
                return at(end - 1);
            }
            let left = end - 1;
            if p == stops[left].offset {
                return at(left);
            }
            let t = (p - stops[left].offset) / (stops[end].offset - stops[left].offset);
            let midpoint = self.midpoints.as_ref().map_or(0.5, |points| points[left]);
            let weight = if midpoint == 0.5 {
                t
            } else {
                t.powf(0.5_f32.ln() / midpoint.ln())
            };
            at(left).lerp_rect(at(end), weight)
        };
        let result = if let Some(phase) = self.phase {
            let p = (position + phase).rem_euclid(1.0);
            let n = stops.len();
            let end = stops.partition_point(|s| s.offset <= p);
            let left = if end == 0 { n - 1 } else { end - 1 };
            let right = end % n;
            let width = stops[right].offset - stops[left].offset + f32::from(right == 0);
            let t = if width > 0.0 {
                (p - stops[left].offset + f32::from(end == 0)) / width
            } else {
                0.0
            };
            let t2 = t * t;
            let t3 = t2 * t;
            let weights = [
                (1.0 - t).powi(3) / 6.0,
                (3.0 * t3 - 6.0 * t2 + 4.0) / 6.0,
                (-3.0 * t3 + 3.0 * t2 + 3.0 * t + 1.0) / 6.0,
                t3 / 6.0,
            ];
            let mut components = [0.0; 4];
            for (index, weight) in [(left + n - 1) % n, left, right, (right + 1) % n]
                .into_iter()
                .zip(weights)
            {
                for (out, v) in components
                    .iter_mut()
                    .zip(at(index).premultiply().components)
                {
                    *out += v * weight;
                }
            }
            // Normalize the finite-precision weights so constant channels stay constant.
            let sum: f32 = weights.iter().sum();
            color::PremulColor::<C>::new(components.map(|channel| channel / sum)).un_premultiply()
        } else {
            let half = self.seam_width / 2.0;
            if self.kind == GradientKind::Angular
                && half > 0.0
                && (position < half || position > 1.0 - half)
            {
                let wrapped = if position < half {
                    position
                } else {
                    position - 1.0
                };
                let t = (wrapped + half) / self.seam_width;
                ramp(1.0 - half).lerp_rect(ramp(half), t * t * (3.0 - 2.0 * t))
            } else {
                ramp(position)
            }
        };
        let color = result.multiply_alpha(self.opacity).to_rgba8();
        Rgba::new(color.r, color.g, color.b, color.a)
    }
}

/// Paint for a bounded region, shared by backgrounds and local drawing.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum Fill {
    /// Fills the region with a single color.
    Solid(Rgba),
    /// Samples a gradient in the bounds of the region.
    Gradient(Gradient),
}
impl From<Rgba> for Fill {
    fn from(color: Rgba) -> Self {
        Self::Solid(color)
    }
}
impl From<[u8; 4]> for Fill {
    fn from(color: [u8; 4]) -> Self {
        Self::Solid(color.into())
    }
}
impl From<Gradient> for Fill {
    fn from(gradient: Gradient) -> Self {
        Self::Gradient(gradient)
    }
}
