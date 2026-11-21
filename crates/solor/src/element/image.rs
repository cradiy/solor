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
    /// Render visible pixels using the application source.
    Custom(crate::CustomImage),
    /// Download an HTTP or HTTPS image when the element mounts.
    #[cfg(feature = "network")]
    Url(crate::Url),
    /// Look up a bitmap by name in the registered resource pack.
    Name(crate::SharedString),
    /// Decode these bytes at attachment.
    #[cfg(feature = "image")]
    Bytes(std::borrow::Cow<'static, [u8]>),
    /// Read intrinsic dimensions at attachment and decode when visible.
    #[cfg(feature = "image")]
    File(crate::ImageFile),
    /// Update the painted pixels and remeasure if the intrinsic dimensions change.
    Bound(crate::Signal<ImageAsset>),
    /// Replace the rendered snapshot when its signal changes.
    BoundCustom(crate::Signal<crate::CustomImage>),
}
crate::assets::name_inputs!(ImageSource);

#[cfg(feature = "network")]
impl From<crate::Url> for ImageSource {
    fn from(url: crate::Url) -> Self {
        Self::Url(url)
    }
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
        AnyElement::new(false, move |cx, (units, assets)| {
            let (image, watch) = match self.asset {
                ImageSource::Custom(image) => (ImageContent::Custom(image), None),
                ImageSource::Fixed(asset) => (ImageContent::Bitmap(asset), None),
                #[cfg(feature = "image")]
                ImageSource::File(file) => (ImageContent::file(file)?, None),
                #[cfg(feature = "image")]
                ImageSource::Bytes(bytes) => {
                    (ImageContent::Bitmap(ImageAsset::from_bytes(&bytes)?), None)
                }
                #[cfg(feature = "network")]
                ImageSource::Url(url) => {
                    return ImageBody::from_url(url, self.style, cx, (units, assets));
                }
                ImageSource::Name(name) => {
                    let image = match assets.named.image(&name)? {
                        #[cfg(feature = "network")]
                        crate::ImageResource::Url(url) => {
                            return ImageBody::from_url(url, self.style, cx, (units, assets))
                                .with_context(|| format!("opening image {name}"));
                        }
                        crate::ImageResource::Custom(image) => Ok(ImageContent::Custom(image)),
                        #[cfg(not(feature = "image"))]
                        crate::ImageResource::Path(_) | crate::ImageResource::Bytes(_) => {
                            Err(anyhow::anyhow!("encoded image support is disabled"))
                        }
                        #[cfg(feature = "image")]
                        crate::ImageResource::Path(path) => {
                            ImageContent::file(crate::ImageFile::new(path.into_owned()))
                        }
                        #[cfg(feature = "image")]
                        crate::ImageResource::Bytes(bytes) => {
                            ImageAsset::from_bytes(&bytes).map(ImageContent::Bitmap)
                        }
                    }
                    .with_context(|| format!("opening image {name}"))?;
                    (image, None)
                }
                ImageSource::Bound(signal) => (
                    ImageContent::Bitmap(cx.read(signal)?.clone()),
                    Some(super::content_watch(
                        signal,
                        crate::ContentUpdate::Image(signal),
                    )),
                ),
                ImageSource::BoundCustom(signal) => (
                    ImageContent::Custom(cx.read(signal)?.clone()),
                    Some(super::content_watch(
                        signal,
                        crate::ContentUpdate::CustomImage(signal),
                    )),
                ),
            };
            let mut node = ImageBody::node(image, self.style, units);
            if let Some(watch) = watch {
                node.watches.push(watch);
            }
            Ok(node)
        })
    }
}

#[derive(PartialEq)]
pub(super) enum ImageContent {
    #[cfg(feature = "network")]
    Empty,
    Bitmap(ImageAsset),
    Custom(crate::CustomImage),
    Svg(SvgAsset, bool),
    #[cfg(feature = "image")]
    File(crate::ImageFile, Size),
}
impl ImageContent {
    #[cfg(feature = "image")]
    fn file(file: crate::ImageFile) -> Result<Self> {
        let size = file.dimensions()?.at_unit_scale();
        Ok(Self::File(file, size))
    }

    fn size(&self) -> Size {
        match self {
            #[cfg(feature = "network")]
            Self::Empty => Size::default(),
            Self::Custom(image) => image.dimensions().at_unit_scale(),
            Self::Bitmap(asset) => asset.dimensions().at_unit_scale(),
            Self::Svg(asset, _) => asset.size(),
            #[cfg(feature = "image")]
            Self::File(_, size) => *size,
        }
    }
}
pub(super) struct ImageBody {
    #[cfg(feature = "network")]
    remote: Option<Box<crate::bitmap::remote::Subscription>>,
    image: ImageContent,
    units: UnitResolver,
    placement: Placement,
    radii: Corners<Pixels>,
}
impl ImageBody {
    #[cfg(feature = "network")]
    fn from_url(
        url: crate::Url,
        style: StyleRefinement,
        cx: &mut crate::Context<'_>,
        (units, assets): super::MountResources<'_>,
    ) -> Result<super::Node<Self>> {
        let subscription = assets.remote.subscribe(url, cx)?;
        let signal = subscription.signal;
        let content = cx
            .read(signal)?
            .clone()
            .map_or(ImageContent::Empty, ImageContent::Bitmap);
        let mut node = Self::node(content, style, units);
        node.body.remote = Some(Box::new(subscription));
        node.watches.push(super::content_watch(
            signal,
            crate::ContentUpdate::RemoteImage(signal),
        ));
        Ok(node)
    }

    pub(super) fn node(
        image: ImageContent,
        style: StyleRefinement,
        units: UnitResolver,
    ) -> super::Node<Self> {
        let mut node = element(Self {
            #[cfg(feature = "network")]
            remote: None,
            image,
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
        self.replace_image_content(id, ImageContent::Bitmap(asset.clone()))
    }

    pub(crate) fn update_svg(
        &mut self,
        id: crate::NodeId,
        asset: &SvgAsset,
        preserve_colors: bool,
    ) -> Result<()> {
        self.replace_image_content(id, ImageContent::Svg(asset.clone(), preserve_colors))
    }

    pub(crate) fn update_custom_image(
        &mut self,
        id: crate::NodeId,
        image: &crate::CustomImage,
    ) -> Result<()> {
        self.replace_image_content(id, ImageContent::Custom(image.clone()))
    }

    fn replace_image_content(&mut self, id: crate::NodeId, image: ImageContent) -> Result<()> {
        type Body = crate::StyledBody<ImageBody>;
        let current = &self.scene.body::<Body>(id)?.content.image;
        if current == &image {
            return Ok(());
        }
        let same_size = current.size() == image.size();
        let replace = |body: &mut Body| body.content.image = image;
        if same_size {
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
impl LayoutNode for ImageBody {
    fn measure(
        &mut self,
        cx: &mut MeasureContext<'_, '_>,
        query: Query,
    ) -> layout::Result<Metrics> {
        let intrinsic = self.image.size();
        #[cfg(feature = "network")]
        if matches!(self.image, ImageContent::Empty) {
            return Ok(Metrics {
                size: Size {
                    width: query.resolve(px(0.)),
                    height: px(0.),
                },
                baseline: None,
            });
        }
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

impl Element for ImageBody {
    fn prepare_content(&mut self, style: &Style) -> Result<()> {
        self.placement = Placement::resolve(style, self.units, ObjectFit::Contain)?;
        self.radii = style.corner_radii;
        Ok(())
    }
    fn paint(&self, cx: &mut PaintContext) -> Result<()> {
        if cx.size.width.get() == 0.0 || cx.size.height.get() == 0.0 {
            return Ok(());
        }
        match &self.image {
            #[cfg(feature = "network")]
            ImageContent::Empty => Ok(()),
            ImageContent::Custom(image) => {
                let mut image = image.clone();
                image.placement = self.placement;
                cx.record_fill(cx.origin, cx.size, image.into(), self.radii.fit_to(cx.size));
                Ok(())
            }
            ImageContent::Bitmap(asset) => {
                cx.record_fill(
                    cx.origin,
                    cx.size,
                    Fill::Image(ImageFill::new(asset.clone(), self.placement)),
                    self.radii.fit_to(cx.size),
                );
                Ok(())
            }
            #[cfg(feature = "image")]
            ImageContent::File(file, _) => {
                let mut file = file.clone();
                file.placement = self.placement;
                cx.record_fill(
                    cx.origin,
                    cx.size,
                    Fill::ImageFile(file),
                    self.radii.fit_to(cx.size),
                );
                Ok(())
            }
            ImageContent::Svg(asset, preserve_colors) => {
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
