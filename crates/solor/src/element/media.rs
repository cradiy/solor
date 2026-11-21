use super::{AnyElement, Draw, Element, IntoElement, PaintContext, element};
use crate::{
    Fill, ImageAsset, ImageFill, Pixels, Style, StyleRefinement, Styled, SvgAsset,
    bitmap::Placement,
    layout::{
        self, LayoutContext, LayoutNode, LayoutOutput, MeasureContext, Metrics, Origin, Query, Size,
    },
    length::UnitResolver,
    px,
    style::{Corners, ObjectFit},
};
use anyhow::{Context as _, Result};

/// Draws a bitmap at its intrinsic size unless constrained by layout.
pub fn image(asset: impl Into<ImageSource>) -> impl Styled + IntoElement {
    Image {
        asset: asset.into(),
        style: StyleRefinement::default(),
    }
}

/// Bitmap content supplied directly or by a reactive source.
pub enum ImageSource {
    /// Draw the same decoded image throughout the attachment.
    Fixed(ImageAsset),
    /// Update the painted pixels and remeasure if the intrinsic dimensions change.
    Bound(crate::Signal<ImageAsset>),
}
impl From<ImageAsset> for ImageSource {
    fn from(value: ImageAsset) -> Self {
        Self::Fixed(value)
    }
}
impl From<crate::Signal<ImageAsset>> for ImageSource {
    fn from(value: crate::Signal<ImageAsset>) -> Self {
        Self::Bound(value)
    }
}

struct Image {
    asset: ImageSource,
    style: StyleRefinement,
}
impl Styled for Image {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
impl IntoElement for Image {
    fn into_element(self) -> AnyElement {
        AnyElement::new(false, move |cx, units| {
            let (asset, source) = match self.asset {
                ImageSource::Fixed(asset) => (asset, None),
                ImageSource::Bound(signal) => (cx.read(signal)?.clone(), Some(signal)),
            };
            let mut node = MediaBody::node(Media::Bitmap(asset), self.style, units);
            if let Some(signal) = source {
                node.watches.push(super::content_watch(
                    signal,
                    crate::ContentUpdate::Image(signal),
                ));
            }
            Ok(node)
        })
    }
}

pub(super) enum Media {
    Bitmap(ImageAsset),
    Svg(SvgAsset, bool),
}
impl Media {
    fn size(&self) -> Size {
        match self {
            Self::Bitmap(asset) => asset.dimensions().at_unit_scale(),
            Self::Svg(asset, _) => asset.size(),
        }
    }
}
pub(super) struct MediaBody {
    media: Media,
    units: UnitResolver,
    placement: Placement,
    radii: Corners<Pixels>,
}
impl MediaBody {
    pub(super) fn node(
        media: Media,
        style: StyleRefinement,
        units: UnitResolver,
    ) -> super::Node<Self> {
        let mut node = element(Self {
            media,
            units,
            placement: Placement::centered(ObjectFit::Contain),
            radii: Default::default(),
        });
        node.style = style;
        node
    }
}

impl crate::Instance {
    pub(crate) fn update_image(&mut self, id: crate::NodeId, asset: &ImageAsset) -> Result<()> {
        type Body = crate::StyledBody<MediaBody>;
        let body = self.scene.body::<Body>(id)?;
        let Media::Bitmap(current) = &body.content.media else {
            anyhow::bail!("image binding requires a bitmap element")
        };
        if current == asset {
            return Ok(());
        }
        let replace = |body: &mut Body| body.content.media = Media::Bitmap(asset.clone());
        if current.dimensions() == asset.dimensions() {
            replace(self.scene.body_mut::<Body>(id)?);
        } else {
            self.scene.edit_layout::<Body>(id, replace)?;
        }
        if self.bodies[&id].visible {
            self.repaints.borrow_mut().invalidate(id);
        }
        Ok(())
    }
}
impl LayoutNode for MediaBody {
    fn measure(
        &mut self,
        cx: &mut MeasureContext<'_, '_>,
        query: Query,
    ) -> layout::Result<Metrics> {
        let intrinsic = self.media.size();
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

impl Element for MediaBody {
    fn prepare_content(&mut self, style: &Style) -> Result<()> {
        self.placement = Placement::resolve(style, self.units, ObjectFit::Contain)?;
        self.radii = style.corner_radii;
        Ok(())
    }
    fn paint(&self, cx: &mut PaintContext) -> Result<()> {
        if cx.size.width.get() == 0.0 || cx.size.height.get() == 0.0 {
            return Ok(());
        }
        match &self.media {
            Media::Bitmap(asset) => {
                cx.record_fill(
                    cx.origin,
                    cx.size,
                    Fill::Image(ImageFill::new(asset.clone(), self.placement)),
                    self.radii.fit_to(cx.size),
                );
                Ok(())
            }
            Media::Svg(asset, preserve_colors) => {
                let (offset, size) = self.placement.bounds(asset.size(), cx.size)?;
                cx.with_clip(Origin::default(), cx.size, |cx| {
                    let origin = cx
                        .origin
                        .checked_add(offset)
                        .context("invalid SVG position")?;
                    cx.commands.push(Draw::Svg {
                        origin,
                        size,
                        clip: cx.clip.clone(),
                        asset: asset.clone(),
                        tint: if *preserve_colors {
                            None
                        } else {
                            Some(cx.text_color()?)
                        },
                    });
                    Ok(())
                })
            }
        }
    }
}
