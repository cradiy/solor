//! Shared bitmap pixels and placement within a painted box.
mod custom;
#[cfg(feature = "network")]
pub(crate) mod remote;
pub use custom::{CustomImage, ImageRegion, ImageRenderer};

use crate::{
    Fill, Style,
    layout::{ComputedLength, Origin, Size},
    length::UnitResolver,
    px,
    raster::DeviceSize,
    style::ObjectFit,
};
use anyhow::{Result, ensure};
use std::{fmt, sync::Arc};

/// Immutable premultiplied RGBA pixels, shared by elements, backgrounds and recorded frames.
#[derive(Clone)]
pub struct ImageAsset(Arc<Bitmap>);

struct Bitmap {
    dimensions: DeviceSize,
    rgba: Vec<u8>,
}

#[cfg(feature = "image")]
mod decode;
#[cfg(feature = "image")]
pub use decode::ImageFile;

impl ImageAsset {
    /// Takes ownership of tightly packed, straight-alpha RGBA8 pixels in top-to-bottom row order.
    pub fn from_rgba8(width: u32, height: u32, mut bytes: Vec<u8>) -> Result<Self> {
        let len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4));
        ensure!(
            width > 0 && height > 0 && len == Some(bytes.len()),
            "invalid RGBA8 image dimensions or byte count"
        );
        for [r, g, b, a] in bytes.as_chunks_mut::<4>().0 {
            if *a != 255 {
                for channel in [r, g, b] {
                    *channel = ((u16::from(*channel) * u16::from(*a) + 127) / 255) as u8;
                }
            }
        }
        Ok(Self(Arc::new(Bitmap {
            dimensions: DeviceSize { width, height },
            rgba: bytes,
        })))
    }

    /// Returns the source width and height in pixels.
    pub fn dimensions(&self) -> DeviceSize {
        self.0.dimensions
    }

    /// Premultiplied sRGB RGBA8 pixels in top-to-bottom row order.
    pub fn pixels(&self) -> &[[u8; 4]] {
        self.0.rgba.as_chunks::<4>().0
    }
}

impl fmt::Debug for ImageAsset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ImageAsset")
            .field(&self.dimensions())
            .finish()
    }
}
impl PartialEq for ImageAsset {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// A shared bitmap with its fit and alignment in the painted box.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageFill {
    asset: ImageAsset,
    pub(crate) placement: Placement,
}

impl ImageFill {
    pub(crate) fn new(asset: ImageAsset, placement: Placement) -> Self {
        Self { asset, placement }
    }
    /// Returns the shared source pixels.
    pub fn asset(&self) -> &ImageAsset {
        &self.asset
    }
    /// Returns the fitted origin and extent relative to the painted box.
    pub fn bounds(&self, target: Size) -> Result<(Origin, Size)> {
        self.placement
            .bounds(self.asset.dimensions().at_unit_scale(), target)
    }
}

impl From<ImageAsset> for Fill {
    fn from(asset: ImageAsset) -> Self {
        Self::Image(ImageFill::new(asset, Placement::centered(ObjectFit::Cover)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placement {
    fit: ObjectFit,
    position: [ComputedLength; 2],
}
impl Placement {
    pub(crate) fn centered(fit: ObjectFit) -> Self {
        Self {
            fit,
            position: [ComputedLength::Fraction(0.5); 2],
        }
    }
    pub(crate) fn resolve(style: &Style, units: UnitResolver, default: ObjectFit) -> Result<Self> {
        let Some(a) = style.appearance.as_ref() else {
            return Ok(Self::centered(default));
        };
        let position = |value| -> Result<_> {
            Ok(units
                .offset(value)?
                .unwrap_or(ComputedLength::Fraction(0.5)))
        };
        Ok(Self {
            fit: a.object_fit.unwrap_or(default),
            position: [
                position(a.object_position_x.unwrap_or_default())?,
                position(a.object_position_y.unwrap_or_default())?,
            ],
        })
    }
    pub(crate) fn bounds(&self, intrinsic: Size, target: Size) -> Result<(Origin, Size)> {
        target.validate()?;
        let x = f64::from(target.width) / f64::from(intrinsic.width);
        let y = f64::from(target.height) / f64::from(intrinsic.height);
        let scaled = |factor| Size {
            width: px((f64::from(intrinsic.width) * factor) as f32),
            height: px((f64::from(intrinsic.height) * factor) as f32),
        };
        let size = match self.fit {
            ObjectFit::Fill => target,
            ObjectFit::None => intrinsic,
            ObjectFit::Contain => scaled(x.min(y)),
            ObjectFit::Cover => scaled(x.max(y)),
            ObjectFit::ScaleDown => scaled(x.min(y).min(1.0)),
        };
        size.validate()?;
        let origin = Origin {
            x: self.position[0]
                .resolve(Some(target.width - size.width))
                .unwrap(),
            y: self.position[1]
                .resolve(Some(target.height - size.height))
                .unwrap(),
        };
        ensure!(
            origin.x.get().is_finite() && origin.y.get().is_finite(),
            "invalid image position"
        );
        Ok((origin, size))
    }
}

/// Window-owned decoded file images. Each cache retains at most 64 MiB and 128 entries.
#[derive(Default)]
pub struct ImageCache {
    #[cfg(feature = "image")]
    entries: std::collections::VecDeque<(Arc<std::path::Path>, ImageAsset)>,
    #[cfg(feature = "image")]
    bytes: usize,
}

impl ImageCache {
    /// Resolves file pixels for a visible paint command, keeping its placement.
    pub fn resolve<'a>(&mut self, fill: &'a Fill) -> Result<std::borrow::Cow<'a, Fill>> {
        #[cfg(feature = "image")]
        if let Fill::ImageFile(file) = fill {
            let asset = self.load(file)?;
            return Ok(std::borrow::Cow::Owned(Fill::Image(ImageFill::new(
                asset,
                file.placement,
            ))));
        }
        Ok(std::borrow::Cow::Borrowed(fill))
    }

    #[cfg(feature = "image")]
    fn load(&mut self, file: &ImageFile) -> Result<ImageAsset> {
        if let Some(i) = self.entries.iter().position(|(path, _)| path == &file.path) {
            let entry = self.entries.remove(i).expect("index found in cache");
            let asset = entry.1.clone();
            self.entries.push_back(entry);
            return Ok(asset);
        }

        const BUDGET: usize = 64 * 1024 * 1024;
        let asset = file.decode()?;
        let bytes = asset.0.rgba.capacity();
        if bytes <= BUDGET {
            while self.bytes + bytes > BUDGET || self.entries.len() >= 128 {
                let (_, oldest) = self.entries.pop_front().expect("nonempty image cache");
                self.bytes -= oldest.0.rgba.capacity();
            }
            self.entries.push_back((file.path.clone(), asset.clone()));
            self.bytes += bytes;
        }
        Ok(asset)
    }
}
