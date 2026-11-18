//! Floating-point geometry in device-independent logical pixels.
use std::ops::{Add, AddAssign, Div, Mul, Sub};

/// A distance in floating-point logical pixels.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Pixels(f32);

/// Declares a distance in logical pixels.
pub const fn px(value: f32) -> Pixels {
    Pixels(value)
}

impl Pixels {
    /// Returns the floating-point distance in logical pixels.
    pub const fn get(self) -> f32 {
        self.0
    }

    /// Whether the value is a valid nonnegative layout extent.
    pub fn is_extent(self) -> bool {
        self.0.is_finite() && self.0 >= 0.0
    }

    /// Returns the smaller distance.
    pub fn min(self, other: Self) -> Self {
        px(self.0.min(other.0))
    }

    /// Returns the larger distance.
    pub fn max(self, other: Self) -> Self {
        px(self.0.max(other.0))
    }

    /// Adds offsets, returning none when the result is not finite.
    pub fn checked_add(self, other: Self) -> Option<Self> {
        let sum = self + other;
        sum.0.is_finite().then_some(sum)
    }

    /// Removes an inset from an extent, clamping the remainder to zero.
    pub fn saturating_sub(self, inset: Self) -> Self {
        px((self.0 - inset.0).max(0.0))
    }
}
impl Add for Pixels {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        px(self.0 + rhs.0)
    }
}
impl Sub for Pixels {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        px(self.0 - rhs.0)
    }
}
impl From<Pixels> for f64 {
    fn from(value: Pixels) -> Self {
        f64::from(value.0)
    }
}

impl AddAssign for Pixels {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}
impl Mul<f32> for Pixels {
    type Output = Self;
    fn mul(self, rhs: f32) -> Self {
        px(self.0 * rhs)
    }
}
impl Div<f32> for Pixels {
    type Output = Self;
    fn div(self, rhs: f32) -> Self {
        px(self.0 / rhs)
    }
}
