//! Retained vector artwork.
use super::{AnyElement, Draw, Element, IntoElement, PaintContext, element};
use crate::{
    Style, StyleRefinement, Styled,
    layout::{
        self, ComputedLength, LayoutContext, LayoutNode, LayoutOutput, MeasureContext, Metrics,
        Origin, Query, Size,
    },
    length::UnitResolver,
    px,
    style::ObjectFit,
};
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
            let mut node = element(SvgBody {
                asset: self.asset,
                preserve_colors: self.preserve_colors,
                units,
                fit: ObjectFit::Contain,
                position: [ComputedLength::Fraction(0.5); 2],
            });
            node.style = self.style;
            Ok(node)
        })
    }
}

pub(super) struct SvgBody {
    asset: SvgAsset,
    preserve_colors: bool,
    units: UnitResolver,
    fit: ObjectFit,
    position: [ComputedLength; 2],
}

impl LayoutNode for SvgBody {
    fn measure(
        &mut self,
        cx: &mut MeasureContext<'_, '_>,
        query: Query,
    ) -> layout::Result<Metrics> {
        let intrinsic = self.asset.size();
        let ratio = f64::from(intrinsic.width) / f64::from(intrinsic.height);
        let preferred = cx
            .definite_content_height()
            .map_or(intrinsic.width, |height| {
                px((f64::from(height) * ratio) as f32)
            });
        let width = query.resolve(preferred);
        Ok(Metrics {
            size: Size {
                width,
                height: px((f64::from(width) / ratio) as f32),
            },
            baseline: None,
        })
    }

    fn layout(
        &mut self,
        _: &mut LayoutContext<'_, '_>,
        assigned: Size,
    ) -> layout::Result<LayoutOutput> {
        Ok(LayoutOutput {
            content: assigned,
            baseline: None,
        })
    }
}

impl Element for SvgBody {
    fn prepare_content(&mut self, style: &Style) -> Result<()> {
        let appearance = style.appearance.as_deref();
        self.fit = appearance
            .and_then(|a| a.object_fit)
            .unwrap_or(ObjectFit::Contain);
        let position = |value: Option<crate::Length>| -> Result<_> {
            Ok(self
                .units
                .offset(value.unwrap_or_default())?
                .unwrap_or(ComputedLength::Fraction(0.5)))
        };
        self.position = [
            position(appearance.and_then(|a| a.object_position_x))?,
            position(appearance.and_then(|a| a.object_position_y))?,
        ];
        Ok(())
    }

    fn paint(&self, cx: &mut PaintContext) -> Result<()> {
        let intrinsic = self.asset.size();
        let target = cx.size;
        if target.width.get() == 0.0 || target.height.get() == 0.0 {
            return Ok(());
        }
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
        let offset = Origin {
            x: self.position[0]
                .resolve(Some(target.width - size.width))
                .unwrap(),
            y: self.position[1]
                .resolve(Some(target.height - size.height))
                .unwrap(),
        };
        cx.with_clip(Origin::default(), target, |cx| {
            let origin = cx
                .origin
                .checked_add(offset)
                .context("invalid SVG position")?;
            cx.commands.push(Draw::Svg {
                origin,
                size,
                clip: cx.clip.clone(),
                asset: self.asset.clone(),
                tint: if self.preserve_colors {
                    None
                } else {
                    Some(cx.text_color()?)
                },
            });
            Ok(())
        })
    }
}
