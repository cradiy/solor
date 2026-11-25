//! Rasterization of a visible region of a scaled SVG.
use super::{Clip, DeviceSize};
use crate::SvgAsset;
use resvg::tiny_skia::{Pixmap, Transform};
use std::{
    hash::{Hash, Hasher},
    io::{Error, Result},
    rc::{Rc, Weak},
};

#[derive(Clone)]
struct Tree(Weak<usvg::Tree>);

impl PartialEq for Tree {
    fn eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }
}
impl Eq for Tree {}
impl Hash for Tree {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.as_ptr().hash(state);
    }
}

/// Identifies pixels of a parsed SVG at one viewport size without retaining its source tree.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SvgRaster {
    tree: Tree,
    viewport: [u64; 2],
    offset: [u64; 2],
    width: u32,
    height: u32,
}

impl SvgRaster {
    /// Selects a nonempty device-pixel rectangle within the scaled viewport.
    pub fn new(asset: &SvgAsset, viewport: Clip, visible: Clip) -> Result<Self> {
        if visible.empty() || viewport.intersect(visible) != visible {
            return Err(Error::other("invalid SVG raster region"));
        }
        let extent = |a: i64, b: i64| u32::try_from(a.abs_diff(b)).map_err(Error::other);
        Ok(Self {
            tree: Tree(Rc::downgrade(asset.tree())),
            viewport: [
                viewport.right.abs_diff(viewport.left),
                viewport.bottom.abs_diff(viewport.top),
            ],
            offset: [
                visible.left.abs_diff(viewport.left),
                visible.top.abs_diff(viewport.top),
            ],
            width: extent(visible.left, visible.right)?,
            height: extent(visible.top, visible.bottom)?,
        })
    }

    /// Size of the visible region in device pixels.
    pub fn size(&self) -> DeviceSize {
        DeviceSize {
            width: self.width,
            height: self.height,
        }
    }

    fn pixmap(&self) -> Result<Pixmap> {
        let tree = self
            .tree
            .0
            .upgrade()
            .ok_or_else(|| Error::other("SVG source released"))?;
        let mut pixmap = Pixmap::new(self.width, self.height)
            .ok_or_else(|| Error::other("invalid SVG raster extent"))?;
        let source = tree.size();
        let transform = Transform::from_row(
            self.viewport[0] as f32 / source.width(),
            0.,
            0.,
            self.viewport[1] as f32 / source.height(),
            -(self.offset[0] as f32),
            -(self.offset[1] as f32),
        );
        resvg::render(&tree, transform, &mut pixmap.as_mut());
        Ok(pixmap)
    }

    /// Rasterizes alpha coverage, one byte per device pixel.
    pub fn mask(&self) -> Result<Vec<u8>> {
        Ok(self.pixmap()?.pixels().iter().map(|p| p.alpha()).collect())
    }

    /// Rasterizes source colors as premultiplied RGBA8.
    pub fn rgba(&self) -> Result<Vec<u8>> {
        Ok(self.pixmap()?.take())
    }
}
