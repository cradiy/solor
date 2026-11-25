use std::io;

use solor::{Pixels, platform::SurfaceRect};

#[derive(Debug, Default, PartialEq)]
pub(crate) enum InputRegion {
    #[default]
    Full,
    Rects(Vec<[i32; 4]>),
}

impl InputRegion {
    pub(crate) fn from_rects(rects: Option<&[SurfaceRect]>) -> io::Result<Self> {
        let Some(rects) = rects else {
            return Ok(Self::Full);
        };
        Ok(Self::Rects(region_rectangles(rects)?))
    }
}

pub(crate) fn region_rectangles(rects: &[SurfaceRect]) -> io::Result<Vec<[i32; 4]>> {
    let mut bounds = Vec::with_capacity(rects.len());
    for rect in rects {
        let (x, width) = axis(rect.origin.x, rect.size.width)?;
        let (y, height) = axis(rect.origin.y, rect.size.height)?;
        if width > 0 && height > 0 {
            bounds.push([x, y, width, height]);
        }
    }
    Ok(bounds)
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
            "invalid surface rectangle",
        ));
    }
    Ok((start as i32, length as i32))
}
