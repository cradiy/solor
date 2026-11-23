//! Device pixel coordinates shared by renderers and presentation targets.
pub use crate::fill::OpaqueLinearGradient;
use std::io::{Error, Result};
/// Integer extent of a raster buffer in device pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceSize {
    /// Physical width in pixels.
    pub width: u32,
    /// Physical height in pixels.
    pub height: u32,
}

impl DeviceSize {
    /// Returns the logical size at one device pixel per logical pixel.
    pub const fn at_unit_scale(self) -> crate::layout::Size {
        crate::layout::Size {
            width: crate::px(self.width as f32),
            height: crate::px(self.height as f32),
        }
    }
}

/// A half-open rectangle in pixel coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clip {
    /// Inclusive left edge.
    pub left: i64,
    /// Inclusive top edge.
    pub top: i64,
    /// Exclusive right edge.
    pub right: i64,
    /// Exclusive bottom edge.
    pub bottom: i64,
}

impl Clip {
    /// Covers the entire target.
    pub fn canvas(size: DeviceSize) -> Self {
        Self {
            left: 0,
            top: 0,
            right: size.width.into(),
            bottom: size.height.into(),
        }
    }

    /// Returns the overlap of both rectangles.
    pub fn intersect(self, other: Self) -> Self {
        Self {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        }
    }

    /// Smallest rectangle containing both nonempty input rectangles.
    pub fn union(self, other: Self) -> Self {
        Self {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }

    /// Whether the rectangle covers no pixels.
    pub fn empty(self) -> bool {
        self.left >= self.right || self.top >= self.bottom
    }
}

/// Changed device-pixel rectangles since a buffer was last brought up to date.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Damage {
    rects: smallvec::SmallVec<[Clip; 16]>,
}

impl Damage {
    /// Damage covering a single rectangle. Empty rectangles cover no pixels.
    pub fn from_rect(rect: Clip) -> Self {
        let mut damage = Self::default();
        damage.include(rect);
        damage
    }

    /// Returns the rectangles to repair, which may overlap.
    pub fn rects(&self) -> &[Clip] {
        &self.rects
    }

    /// Whether all previously submitted pixels remain current.
    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    /// Smallest rectangle containing every change.
    pub fn bounds(&self) -> Option<Clip> {
        self.rects.iter().copied().reduce(Clip::union)
    }

    /// Includes every pixel in a rectangle, with bounded storage for scattered changes.
    pub fn include(&mut self, rect: Clip) {
        self.include_with_gap(rect, 128);
    }

    pub(crate) fn include_with_gap(&mut self, rect: Clip, gap_area: u128) {
        if rect.empty() {
            return;
        }
        let area =
            |r: Clip| u128::from(r.right.abs_diff(r.left)) * u128::from(r.bottom.abs_diff(r.top));
        let mut nearest = None;
        for (index, &old) in self.rects.iter().enumerate() {
            let merged = old.union(rect);
            if merged == old {
                return;
            }
            let extra = area(merged) - area(old);
            if nearest.is_none_or(|(_, cost)| extra < cost) {
                nearest = Some((index, extra));
            }
        }
        // Combine nearby edits when their gap costs less than another small repair.
        if let Some((index, cost)) = nearest
            && (cost <= area(rect).saturating_add(gap_area)
                || self.rects.len() == self.rects.inline_size())
        {
            let merged = self.rects[index].union(rect);
            self.rects[index] = merged;
            // A growing chart span may absorb several earlier edits.
            let mut i = self.rects.len();
            while i > 0 {
                i -= 1;
                if i != index && self.rects[i].intersect(merged) == self.rects[i] {
                    self.rects.remove(i);
                }
            }
        } else {
            self.rects.push(rect);
        }
    }
}

/// Device pixels per logical unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RasterScale(pub f64);

impl RasterScale {
    /// One device pixel per logical pixel.
    pub const ONE: Self = Self(1.0);

    /// Rounds each scaled extent to device pixels, with a minimum of one per axis.
    pub fn buffer_size(self, logical: crate::layout::Size) -> Result<DeviceSize> {
        if !self.0.is_finite() || self.0 <= 0.0 {
            return Err(Error::other("invalid raster scale"));
        }
        let dimension = |length: crate::Pixels| {
            let scaled = (f64::from(length) * self.0).round();
            if !length.is_extent() || scaled > f64::from(u32::MAX) {
                return Err(Error::other("scaled dimension out of range"));
            }
            Ok(scaled.max(1.0) as u32)
        };
        Ok(DeviceSize {
            width: dimension(logical.width)?,
            height: dimension(logical.height)?,
        })
    }

    /// Rounds a logical edge to the nearest physical pixel.
    pub fn edge(self, logical: f64) -> i64 {
        (logical * self.0).round() as i64
    }

    /// Converts a logical box to device pixels by rounding its edges.
    pub fn bounds(self, origin: crate::layout::Origin, size: crate::layout::Size) -> Clip {
        let x = f64::from(origin.x);
        let y = f64::from(origin.y);
        Clip {
            left: self.edge(x),
            top: self.edge(y),
            right: self.edge(x + f64::from(size.width)),
            bottom: self.edge(y + f64::from(size.height)),
        }
    }
}

/// Presentation of a complete RGBA8 image with optional partial damage.
pub struct Rgba8;

impl crate::platform::Presentation for Rgba8 {
    type Frame<'a> = RasterFrame<'a>;
}

/// A complete, tightly packed RGBA8 image and its damage since the last submission.
pub struct RasterFrame<'a> {
    /// Physical dimensions of the image.
    pub size: DeviceSize,
    /// Four bytes per pixel in RGBA order. Transparent targets require premultiplied RGB.
    /// Opaque targets ignore the alpha byte.
    pub rgba: &'a [u8],
    /// Physical regions changed since the last submission.
    pub damage: Damage,
    /// Request a frame callback even if the buffer contents are unchanged.
    pub request_callback: bool,
}

impl RasterFrame<'_> {
    /// Checks image length, device extent and damage before upload.
    pub fn validate(&self, expected: DeviceSize) -> Result<()> {
        let canvas = Clip::canvas(expected);
        let len = (expected.width as usize)
            .checked_mul(expected.height as usize)
            .and_then(|n| n.checked_mul(4));
        if self.size != expected || len != Some(self.rgba.len()) {
            return Err(Error::other(
                "frame size differs from the configured target",
            ));
        }
        if self
            .damage
            .rects()
            .iter()
            .any(|&a| a.intersect(canvas) != a)
        {
            return Err(Error::other(
                "invalid frame damage for the configured target",
            ));
        }
        Ok(())
    }
}
