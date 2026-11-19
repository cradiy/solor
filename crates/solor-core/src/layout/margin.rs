use crate::Pixels;

/// Outer spacing on one edge after resolving length units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Margin {
    /// The parent distributes free space on this edge.
    Auto,
    /// A signed distance in logical pixels.
    Fixed(Pixels),
}

impl Default for Margin {
    fn default() -> Self {
        Self::Fixed(Pixels::default())
    }
}

impl Margin {
    /// Returns the fixed contribution before free space is distributed.
    pub fn fixed_or_zero(self) -> Pixels {
        match self {
            Self::Fixed(value) => value,
            Self::Auto => Pixels::default(),
        }
    }
}
