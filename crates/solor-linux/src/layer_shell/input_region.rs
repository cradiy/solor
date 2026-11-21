use std::io;

use solor::{
    Pixels,
    layout::{Origin, Size},
};

/// A rectangle in surface-local logical pixels that accepts pointer and touch input.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InputRect {
    /// Top-left corner relative to the surface.
    pub origin: Origin,
    /// Nonnegative width and height.
    pub size: Size,
}

impl InputRect {
    /// Creates a rectangle with the given offset and extent in logical pixels.
    pub const fn new(x: Pixels, y: Pixels, width: Pixels, height: Pixels) -> Self {
        Self {
            origin: Origin { x, y },
            size: Size { width, height },
        }
    }
}

#[derive(Debug, Default, PartialEq)]
pub(crate) enum InputRegion {
    #[default]
    Full,
    Rects(Vec<[i32; 4]>),
}

impl InputRegion {
    pub(crate) fn from_rects(rects: Option<&[InputRect]>) -> io::Result<Self> {
        let Some(rects) = rects else {
            return Ok(Self::Full);
        };
        let mut bounds = Vec::with_capacity(rects.len());
        for rect in rects {
            let (x, width) = axis(rect.origin.x, rect.size.width)?;
            let (y, height) = axis(rect.origin.y, rect.size.height)?;
            if width > 0 && height > 0 {
                bounds.push([x, y, width, height]);
            }
        }
        Ok(Self::Rects(bounds))
    }
}

fn axis(origin: Pixels, extent: Pixels) -> io::Result<(i32, i32)> {
    let origin = f64::from(origin);
    let start = origin.floor();
    let length = if extent.get() == 0.0 {
        0.0
    } else {
        ((origin - start) + f64::from(extent)).ceil()
    };
    let range = f64::from(i32::MIN)..=f64::from(i32::MAX);
    if !extent.is_extent()
        || !range.contains(&start)
        || !range.contains(&(start + length))
        || length > f64::from(i32::MAX)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid layer input rectangle",
        ));
    }
    Ok((start as i32, length as i32))
}
