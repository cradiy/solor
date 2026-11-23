//! Retained vector artwork.
use super::{
    AnyElement, Draw, IntoElement, PaintContext,
    image::{ImageBody, ImageContent},
};
use crate::{
    Ink, RoundedClip, StyleRefinement, Styled,
    bitmap::Placement,
    layout::{Origin, Size},
    px,
    style::ObjectFit,
};
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

impl PaintContext<'_> {
    /// Centers the SVG's silhouette in a content-local box, preserving its aspect
    /// ratio. The ink can be a solid color or a gradient.
    pub fn paint_svg(
        &mut self,
        offset: Origin,
        size: Size,
        asset: &SvgAsset,
        ink: impl Into<Ink>,
    ) -> Result<()> {
        self.paint_contained_svg(offset, size, asset, Some(ink.into()))
    }

    /// Centers the SVG in a content-local box, preserving its aspect ratio and source colors.
    pub fn paint_color_svg(&mut self, offset: Origin, size: Size, asset: &SvgAsset) -> Result<()> {
        self.paint_contained_svg(offset, size, asset, None)
    }

    fn paint_contained_svg(
        &mut self,
        offset: Origin,
        size: Size,
        asset: &SvgAsset,
        tint: Option<Ink>,
    ) -> Result<()> {
        let (inset, fitted) = Placement::centered(ObjectFit::Contain).bounds(asset.size(), size)?;
        let position = offset.checked_add(inset).context("invalid SVG position")?;
        self.record_svg(position, fitted, asset, tint, None)
    }

    pub(super) fn record_svg(
        &mut self,
        offset: Origin,
        size: Size,
        asset: &SvgAsset,
        tint: Option<Ink>,
        rounded_clip: Option<Rc<RoundedClip>>,
    ) -> Result<()> {
        let origin = self
            .origin
            .checked_add(offset)
            .context("invalid SVG position")?;
        if size.width > px(0.) && size.height > px(0.) {
            self.commands.push(Draw::Svg {
                origin,
                size,
                clip: self.clip.clone(),
                rounded_clip,
                asset: asset.clone(),
                tint,
            });
        }
        Ok(())
    }
}

/// Vector artwork supplied by a parsed asset, signal, resource name, bytes or file.
pub enum SvgSource {
    /// Reuse this parsed SVG.
    Asset(SvgAsset),
    /// Follow the current parsed artwork throughout the attachment.
    Bound(crate::Signal<SvgAsset>),
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

impl From<crate::Signal<SvgAsset>> for SvgSource {
    fn from(signal: crate::Signal<SvgAsset>) -> Self {
        Self::Bound(signal)
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
        AnyElement::new(false, move |cx, (units, assets)| {
            let mut watch = None;
            let asset = match self.asset {
                SvgSource::Asset(asset) => asset,
                SvgSource::Bound(signal) => {
                    watch = Some(super::content_watch(
                        signal,
                        crate::ContentUpdate::Svg(signal, self.preserve_colors),
                    ));
                    cx.read(signal)?.clone()
                }
                SvgSource::Name(name) => assets.named.svg(&name)?,
                SvgSource::Bytes(bytes) => SvgAsset::from_bytes(&bytes)?,
                SvgSource::Path(path) => SvgAsset::from_path(path)?,
            };
            let mut node = ImageBody::node(
                ImageContent::Svg(asset, self.preserve_colors),
                self.style,
                units,
            );
            if let Some(watch) = watch {
                node.watches.push(watch);
            }
            Ok(node)
        })
    }
}
