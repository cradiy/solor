//! Retained vector artwork.
use super::{
    AnyElement, IntoElement,
    media::{Media, MediaBody},
};
use crate::{StyleRefinement, Styled, layout::Size, px};
use anyhow::{Context as _, Result};
use std::rc::Rc;

/// A parsed SVG document shared by elements and recorded frames.
#[derive(Clone, Debug)]
pub struct SvgAsset(Rc<usvg::Tree>);

impl SvgAsset {
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

/// A styled SVG leaf.
pub struct Svg {
    asset: SvgAsset,
    style: StyleRefinement,
    preserve_colors: bool,
}

/// Draws the asset's silhouette in the inherited foreground color.
pub fn svg(asset: SvgAsset) -> Svg {
    Svg {
        asset,
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
        AnyElement::new(false, move |_, units| {
            Ok(MediaBody::node(
                Media::Svg(self.asset, self.preserve_colors),
                self.style,
                units,
            ))
        })
    }
}
