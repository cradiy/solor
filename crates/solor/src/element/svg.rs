//! Retained vector artwork.
use super::{
    AnyElement, IntoElement,
    image::{ImageBody, ImageContent},
};
use crate::{StyleRefinement, Styled, layout::Size, px};
use anyhow::{Context as _, Result};
use std::{path::PathBuf, rc::Rc};

/// A parsed SVG document shared by elements and recorded frames.
#[derive(Clone, Debug)]
pub struct SvgAsset(pub(crate) Rc<usvg::Tree>);

impl SvgAsset {
    /// Reads and parses a self-contained SVG from a local file.
    pub fn from_path(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let path = path.as_ref();
        anyhow::ensure!(
            path.is_absolute(),
            "expected an absolute SVG path: {}",
            path.display()
        );
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        Self::from_bytes(&bytes).with_context(|| format!("parsing SVG from {}", path.display()))
    }

    /// Parses self-contained SVG bytes. External resources are not loaded.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let mut options = usvg::Options::default();
        options.image_href_resolver.resolve_string = Box::new(|_, _| None);
        Ok(Self(Rc::new(
            usvg::Tree::from_data(bytes, &options).context("invalid SVG asset")?,
        )))
    }

    /// Shared vector tree used by rendering backends.
    pub fn tree(&self) -> &Rc<usvg::Tree> {
        &self.0
    }

    /// Intrinsic viewport in logical pixels.
    pub fn size(&self) -> Size {
        let size = self.0.size();
        Size {
            width: px(size.width()),
            height: px(size.height()),
        }
    }
}

impl PartialEq for SvgAsset {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

/// A parsed document or a self-contained SVG on the filesystem.
pub enum SvgSource {
    /// Reuse this parsed SVG.
    Asset(SvgAsset),
    /// Look up a registered SVG by name.
    Name(crate::SharedString),
    /// Parse encoded SVG content at attachment.
    Bytes(std::borrow::Cow<'static, [u8]>),
    /// Read this local file when the element is attached.
    Path(PathBuf),
}

impl From<SvgAsset> for SvgSource {
    fn from(asset: SvgAsset) -> Self {
        Self::Asset(asset)
    }
}

crate::assets::name_inputs!(SvgSource);
crate::assets::source_inputs!(SvgSource, Self::Path);

/// A styled SVG leaf.
pub struct Svg {
    asset: SvgSource,
    style: StyleRefinement,
    preserve_colors: bool,
}

/// Draws the asset's silhouette in the inherited foreground color.
pub fn svg(asset: impl Into<SvgSource>) -> Svg {
    Svg {
        asset: asset.into(),
        style: StyleRefinement::default(),
        preserve_colors: false,
    }
}

impl Svg {
    /// Uses the fills, strokes and gradients from the SVG document.
    pub fn preserve_colors(mut self) -> Self {
        self.preserve_colors = true;
        self
    }
}

impl Styled for Svg {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl IntoElement for Svg {
    fn into_element(self) -> AnyElement {
        AnyElement::new(false, move |_, (units, assets)| {
            let asset = match self.asset {
                SvgSource::Asset(asset) => asset,
                SvgSource::Name(name) => assets.svg(&name)?,
                SvgSource::Bytes(bytes) => SvgAsset::from_bytes(&bytes)?,
                SvgSource::Path(path) => SvgAsset::from_path(path)?,
            };
            Ok(ImageBody::node(
                ImageContent::Svg(asset, self.preserve_colors),
                self.style,
                units,
            ))
        })
    }
}
