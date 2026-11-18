//! Dimensions in device-independent logical pixels.
use crate::layout::{LayoutError, Result};
use crate::{Pixels, px};

/// A style length that can be resolved without knowing the parent box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AbsoluteLength {
    /// A fixed logical-pixel length.
    Pixels(Pixels),
    /// A multiple of the mounted view's rem basis.
    Rem(f32),
}

/// A multiple of the mounted view's base font size.
pub const fn rem(value: f32) -> AbsoluteLength {
    AbsoluteLength::Rem(value)
}

impl From<Pixels> for AbsoluteLength {
    fn from(value: Pixels) -> Self {
        Self::Pixels(value)
    }
}

/// Sizing policy for one box axis.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Length {
    /// Derive the preferred size from content.
    #[default]
    Auto,
    /// Size in logical pixels or multiples of the view's base font size.
    Absolute(AbsoluteLength),
    /// Percentage of the containing block on this axis, with 100.0 denoting the full extent.
    Percent(f32),
}

/// Declares a percentage of the parent content dimension on the corresponding axis.
pub const fn percent(value: f32) -> Length {
    Length::Percent(value)
}

impl<T: Into<AbsoluteLength>> From<T> for Length {
    fn from(value: T) -> Self {
        Self::Absolute(value.into())
    }
}

#[derive(Clone, Copy)]
pub(crate) struct UnitResolver {
    rem_size: Pixels,
}

impl UnitResolver {
    pub fn new(rem_size: Pixels) -> Result<Self> {
        if !rem_size.get().is_finite() || rem_size.get() <= 0.0 {
            return Err(LayoutError::InvalidSize);
        }
        Ok(Self { rem_size })
    }
    pub fn absolute(self, length: AbsoluteLength) -> Pixels {
        match length {
            AbsoluteLength::Pixels(value) => value,
            AbsoluteLength::Rem(value) => px(value * self.rem_size.get()),
        }
    }
    pub fn dimension(self, length: Length) -> Result<Option<ComputedLength>> {
        match length {
            Length::Auto => Ok(None),
            Length::Absolute(value) => {
                let value = self.absolute(value);
                if !value.is_extent() {
                    return Err(LayoutError::InvalidSize);
                }
                Ok(Some(ComputedLength::Pixels(value)))
            }
            Length::Percent(value) if value.is_finite() && value >= 0.0 => {
                Ok(Some(ComputedLength::Fraction(value / 100.0)))
            }
            _ => Err(LayoutError::InvalidSize),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ComputedLength {
    Pixels(Pixels),
    Fraction(f32),
}

impl ComputedLength {
    pub fn resolve(self, basis: Option<Pixels>) -> Option<Pixels> {
        match self {
            Self::Pixels(value) => Some(value),
            Self::Fraction(fraction) => basis.map(|value| px(value.get() * fraction)),
        }
    }
}
