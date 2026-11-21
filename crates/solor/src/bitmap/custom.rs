//! Regional rendering of application-owned image content.
use super::{ImageAsset, Placement};
use crate::{
    Fill, ImageSource,
    layout::{Origin, Size},
    raster::{Clip, Damage, DeviceSize},
    style::ObjectFit,
};
use anyhow::{Result, ensure};
use std::{
    fmt,
    rc::{Rc, Weak},
};

/// Mapping from a rectangular output buffer to intrinsic image pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageRegion {
    /// Output dimensions in device pixels, in top-to-bottom row order.
    pub output: DeviceSize,
    /// Source coordinates at the upper-left edge of the first output pixel.
    /// Fractional coverage can place this edge just outside the source.
    pub source_origin: [f64; 2],
    /// Source pixels per output pixel along each axis.
    pub source_step: [f64; 2],
}

/// Produces pixels from an immutable image snapshot. The application owns its backing
/// store, sampling and cache. A renderer can be called again for overlapping regions.
pub trait ImageRenderer {
    /// Full source dimensions. Only metadata is needed to lay out the element.
    fn dimensions(&self) -> DeviceSize;

    /// Returns exactly the requested output extent. The call runs synchronously
    /// during rasterization. Sample output pixel centers at origin + (index + 0.5) * step.
    fn render(&self, region: ImageRegion) -> Result<ImageAsset>;
}

/// A shared image whose pixels are supplied by the application during rasterization.
#[derive(Clone)]
pub struct CustomImage {
    renderer: Rc<dyn ImageRenderer>,
    size: DeviceSize,
    change: Option<Rc<ImageChange>>,
    pub(crate) placement: Placement,
}

struct ImageChange {
    previous: Weak<dyn ImageRenderer>,
    regions: smallvec::SmallVec<[Clip; 1]>,
}

impl CustomImage {
    /// Creates a handle to a source with nonzero dimensions, without requesting pixels.
    pub fn new(renderer: impl ImageRenderer + 'static) -> Result<Self> {
        let size = renderer.dimensions();
        ensure!(size.width != 0 && size.height != 0, "empty custom image");
        Ok(Self {
            renderer: Rc::new(renderer),
            size,
            change: None,
            placement: Placement::centered(ObjectFit::Cover),
        })
    }

    /// Publishes a new snapshot of the same size, with changed pixels inside a
    /// half-open rectangle in source coordinates. Include any filtering halo:
    /// requests outside this rectangle must produce the same pixels at every scale.
    pub fn updated_region(
        &self,
        region: Clip,
        renderer: impl ImageRenderer + 'static,
    ) -> Result<Self> {
        self.updated_regions([region], renderer)
    }

    /// Publishes several changed rectangles in one snapshot. Supply at least one
    /// nonempty source rectangle and include any pixels influenced by filtering.
    pub fn updated_regions(
        &self,
        regions: impl IntoIterator<Item = Clip>,
        renderer: impl ImageRenderer + 'static,
    ) -> Result<Self> {
        let mut next = Self::new(renderer)?;
        ensure!(
            next.size == self.size,
            "regional image update changed dimensions"
        );
        let canvas = Clip::canvas(self.size);
        let mut damage = Damage::default();
        for region in regions {
            ensure!(
                !region.empty() && region.intersect(canvas) == region,
                "invalid image update region"
            );
            // The cost of painting gaps depends on the eventual display scale.
            damage.include_with_gap(region, 0);
        }
        ensure!(!damage.is_empty(), "empty image update");
        next.change = Some(Rc::new(ImageChange {
            previous: Rc::downgrade(&self.renderer),
            regions: smallvec::SmallVec::from_slice(damage.rects()),
        }));
        next.placement = self.placement;
        Ok(next)
    }

    /// Returns the bounding rectangle of all changes since the painted snapshot when
    /// its identity and placement match the previous version.
    pub fn changed_region_since(&self, previous: &Self) -> Option<Clip> {
        self.changed_regions_since(previous)?
            .iter()
            .copied()
            .reduce(Clip::union)
    }

    /// Returns source rectangles covering the changes since the direct predecessor
    /// when placement matches. Dense batches may be coalesced.
    pub fn changed_regions_since(&self, previous: &Self) -> Option<&[Clip]> {
        let change = self.change.as_ref()?;
        (self.placement == previous.placement
            && change.previous.ptr_eq(&Rc::downgrade(&previous.renderer)))
        .then_some(&change.regions)
    }

    /// Returns the intrinsic image dimensions.
    pub fn dimensions(&self) -> DeviceSize {
        self.size
    }

    /// Resolves the fitted logical image rectangle within a paint box.
    pub fn bounds(&self, target: Size) -> Result<(Origin, Size)> {
        self.placement.bounds(self.size.at_unit_scale(), target)
    }

    /// Requests the specified pixels and checks the returned buffer dimensions.
    pub fn render(&self, region: ImageRegion) -> Result<ImageAsset> {
        ensure!(
            region.output.width > 0
                && region.output.height > 0
                && region.source_origin.iter().all(|n| n.is_finite())
                && region.source_step.iter().all(|n| n.is_finite() && *n > 0.0),
            "invalid image region"
        );
        let image = self.renderer.render(region)?;
        ensure!(
            image.dimensions() == region.output,
            "custom image returned the wrong output dimensions"
        );
        Ok(image)
    }
}

impl PartialEq for CustomImage {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.renderer, &other.renderer) && self.placement == other.placement
    }
}
impl fmt::Debug for CustomImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CustomImage")
            .field("size", &self.size)
            .field("placement", &self.placement)
            .finish_non_exhaustive()
    }
}
impl From<CustomImage> for Fill {
    fn from(image: CustomImage) -> Self {
        Self::CustomImage(image)
    }
}
impl From<CustomImage> for ImageSource {
    fn from(image: CustomImage) -> Self {
        Self::Custom(image)
    }
}

impl From<crate::Signal<CustomImage>> for ImageSource {
    fn from(image: crate::Signal<CustomImage>) -> Self {
        Self::BoundCustom(image)
    }
}
