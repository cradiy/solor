//! Unclipped device-space geometry for recorded paths.
use solor::{
    layout::Origin,
    raster::{Clip, RasterScale},
};
use std::io::{Error, Result};

pub(super) fn device(point: Origin, origin: Origin, scale: RasterScale) -> (f64, f64) {
    (
        (f64::from(point.x) + f64::from(origin.x)) * scale.0,
        (f64::from(point.y) + f64::from(origin.y)) * scale.0,
    )
}

pub(super) struct PathBounds {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl PathBounds {
    pub fn new(origin: Origin, points: &[Origin], scale: RasterScale) -> Result<Self> {
        let mut b = Self {
            left: f64::INFINITY,
            top: f64::INFINITY,
            right: f64::NEG_INFINITY,
            bottom: f64::NEG_INFINITY,
        };
        for &point in points {
            let (x, y) = device(point, origin, scale);
            b.left = b.left.min(x);
            b.right = b.right.max(x);
            b.top = b.top.min(y);
            b.bottom = b.bottom.max(y);
        }
        if !(b.right - b.left).hypot(b.bottom - b.top).is_finite() {
            return Err(Error::other("path exceeds device coordinate range"));
        }
        Ok(b)
    }

    pub fn clip(&self, margin: f64) -> Result<Clip> {
        if !margin.is_finite() {
            return Err(Error::other("invalid path margin"));
        }
        Ok(Clip {
            left: (self.left - margin).floor() as i64,
            top: (self.top - margin).floor() as i64,
            right: (self.right + margin).ceil() as i64,
            bottom: (self.bottom + margin).ceil() as i64,
        })
    }
}
