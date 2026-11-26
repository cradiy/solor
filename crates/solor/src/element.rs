//! Composition, conversion at attachment, and retained element behavior.
pub(crate) mod image;
mod outline;
mod shadow;
mod style_binding;
pub(crate) mod svg;

use crate::{
    ContentClip,
    clip::{NodeClips, OverflowClip},
};
use anyhow::Context as _;
use std::{cell::RefCell, rc::Rc, sync::Arc};

use crate::layout::{LayoutNode, Origin, Query, Scene, Size};
use crate::state::{Runtime, Signal, SignalId};
use crate::tree::NodeId;

use super::{Attachment, Context, Repaints, Result, SharedString, ViewId};
use crate::layout::{FlexDirection, FlexLayout};
use crate::length::UnitResolver;
use crate::pointer::Handlers;
use crate::style::{Align, Background, BorderPaint, Corners, Foreground, TextStyleRefinement};
use crate::style::{PointerEvents, Visibility};
use crate::text::{Line, Paragraph, Text, TextStyle};
use crate::{Fill, Pixels, Style, StyleRefinement, Styled, StyledBody, px};

/// Builds an element declaration when a component is attached.
pub trait Render {
    /// Consumes the component and declares its retained element tree.
    fn render(self, cx: &mut Context<'_>) -> impl IntoElement;
}

/// Converts a component, primitive or text value into a single-use declaration.
/// Strings become text leaves. A `Signal<SharedString>` updates the same leaf on flush.
pub trait IntoElement {
    /// Erases the declaration's concrete type for heterogeneous composition.
    fn into_element(self) -> AnyElement;
}

impl<R: Render + 'static> IntoElement for R {
    fn into_element(self) -> AnyElement {
        AnyElement(Box::new(move |target, cx, units| {
            let item = self.render(cx).into_element();
            (item.0)(target, cx, units)
        }))
    }
}

/// Layout and local drawing behavior of a retained primitive.
pub trait Element: LayoutNode {
    /// Intrinsic defaults used before inherited typography and local refinements.
    fn base_style(&self) -> Style {
        Style::default()
    }

    /// Content-specific inputs such as font metrics. The framework handles the box and paint.
    fn prepare_content(&mut self, _style: &Style) -> Result<()> {
        Ok(())
    }

    /// Records this body's content after its background.
    fn paint(&self, _cx: &mut PaintContext) -> Result<()> {
        Ok(())
    }
}

impl Element for FlexLayout {
    fn base_style(&self) -> Style {
        Style {
            row_gap: self.gap,
            column_gap: self.gap,
            ..Style::default()
        }
    }

    fn prepare_content(&mut self, style: &Style) -> Result<()> {
        if let Some(layout) = &style.layout {
            self.direction = layout.flex_direction.unwrap_or(self.direction);
            self.justify_content = layout.justify_content.unwrap_or(self.justify_content);
            self.align_items = layout.align_items.unwrap_or(self.align_items);
        }
        self.gap = match self.direction {
            FlexDirection::Row | FlexDirection::RowReverse => style.column_gap,
            FlexDirection::Column | FlexDirection::ColumnReverse => style.row_gap,
        };
        Ok(())
    }
}

impl Element for Text {
    fn base_style(&self) -> Style {
        let mut style = Style::default();
        style.text.size = self.style().size;
        style.text.line_height = self.style().line_height;
        style
    }
    fn prepare_content(&mut self, style: &Style) -> Result<()> {
        self.set_style(TextStyle {
            size: style.text.size,
            line_height: style.text.line_height,
        })?;
        self.set_paragraph_style(style.text.attributes.as_deref(), style.text.letter_spacing);
        if let Some(input) = &mut self.input {
            input.selection_color = style
                .text
                .attributes
                .as_ref()
                .and_then(|a| a.selection_color);
        }
        self.set_decorations(&style.text)?;
        Ok(())
    }
    fn paint(&self, cx: &mut PaintContext) -> Result<()> {
        let paragraph = self.resolve(Query::Exact(cx.size.width))?;
        if let Some(input) = &self.input {
            let (caret, _) = cx.state.read(input.source)?.label_caret();
            let left = self.scroll_for_caret(&paragraph, cx.size.width, caret);
            if left > px(0.) {
                return cx.with_clip(Origin::default(), cx.size, |cx| {
                    cx.origin.x = cx.origin.x - left;
                    cx.size.width += left;
                    self.paint_paragraph(cx, paragraph)
                });
            }
        }
        self.paint_paragraph(cx, paragraph)
    }
}

impl Text {
    fn paint_paragraph(
        &self,
        cx: &mut PaintContext,
        paragraph: Rc<crate::text::Paragraph>,
    ) -> Result<()> {
        let color = cx.text_color()?;
        let decorations = self.decorations.as_deref();
        if let Some(background) = decorations.and_then(|d| d.background.as_ref()) {
            cx.text_line_fills(&paragraph, background, |line| {
                (line.top, line.top + line.height)
            })?;
        }
        if let Some((stroke, kind)) = decorations.and_then(|d| d.underline.as_ref()) {
            if *kind == crate::style::UnderlineStyle::Wavy {
                cx.wavy_underline(&paragraph, stroke, &color)?;
            } else {
                cx.text_stroke(&paragraph, stroke, &color)?;
            }
        }
        cx.draw_paragraph(paragraph.clone(), cx.size, color.clone());
        if let Some(stroke) = decorations.and_then(|d| d.strike.as_ref()) {
            cx.text_stroke(&paragraph, stroke, &color)?;
        }
        Ok(())
    }
}

/// Determines which regions of a closed contour are filled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FillRule {
    /// A point is inside when a ray from it crosses the contour an odd number of times.
    #[default]
    EvenOdd,
    /// A point is inside when the net winding around it is nonzero.
    NonZero,
}

/// A recorded drawing operation. Origins are local to the recording target.
#[derive(Clone, Debug, PartialEq)]
pub enum Draw {
    /// Composites the enclosed commands as a single layer over the destination.
    Group {
        /// Optional luminance mapping applied before group opacity.
        gradient_map: Option<crate::GradientMap>,
        /// Group opacity, with 255 representing full coverage.
        alpha: u8,
        /// Commands in paint order, in the same coordinates as the enclosing frame.
        draws: Vec<Draw>,
    },
    /// Draws one sinusoidal underline across a shaped line.
    WavyUnderline {
        /// Effective clip, including the label's content box.
        clip: Option<Arc<ContentClip>>,
        /// Upper-left corner and horizontal phase origin of the wave.
        origin: Origin,
        /// Line width and wave band height. The stroke is one third of the band height.
        size: Size,
        /// Foreground paint across the wave.
        color: crate::Ink,
    },
    /// Filters previously painted pixels under a rounded box in the current layer.
    BackdropBlur {
        /// Ancestor clip on the filter output.
        clip: Option<Arc<ContentClip>>,
        /// Upper-left corner of the box.
        origin: Origin,
        /// Size of the filtered region.
        size: Size,
        /// Sampling radius in logical pixels.
        radius: Pixels,
        /// Fitted radii of the output mask.
        corner_radii: Corners<Pixels>,
    },
    /// Fills a rounded rectangle.
    Fill {
        /// Effective clip in the same coordinates as the origin.
        clip: Option<Arc<ContentClip>>,
        /// Upper-left corner of the fill.
        origin: Origin,
        /// Extent of the rectangle.
        size: Size,
        /// Color or gradient covering the rectangle.
        fill: Fill,
        /// Circular radii in logical pixels, fitted to the fill bounds.
        corner_radii: Corners<Pixels>,
    },
    /// Paints a single inner or outer shadow of a rounded border box.
    Shadow {
        /// Ancestor clip in the recording coordinate space.
        clip: Option<Arc<ContentClip>>,
        /// Upper-left corner of the unshifted box.
        origin: Origin,
        /// Extent of the unshifted box.
        size: Size,
        /// Fitted corner radii of the unshifted box.
        corner_radii: Corners<Pixels>,
        /// Layer parameters in logical pixels.
        shadow: crate::style::BoxShadow<Pixels>,
    },
    /// Paints the ring between a box and its inner border edges.
    Border {
        /// Effective clip in view coordinates.
        clip: Option<Arc<ContentClip>>,
        /// Upper-left corner of the border box.
        origin: Origin,
        /// Border-box extent.
        size: Size,
        /// Per-edge widths and fills, with outer radii fitted at draw time.
        paint: Rc<crate::style::BorderPaint>,
    },
    /// Fills closed contours as one shape.
    Polygon {
        /// Effective clip in view coordinates.
        clip: Option<Arc<ContentClip>>,
        /// Content origin for the local vertices.
        origin: Origin,
        /// Local vertices grouped into independently closed contours.
        contours: smallvec::SmallVec<[Rc<[Origin]>; 1]>,
        /// Rule used to determine the interior.
        rule: FillRule,
        /// Paint mapped to the complete polygon bounds.
        fill: Fill,
    },
    /// Strokes connected segments with round joins and round ends.
    Polyline {
        /// Effective clip in view coordinates.
        clip: Option<Arc<ContentClip>>,
        /// Content origin for the local points.
        origin: Origin,
        /// Ordered vertices of each contour in local logical pixels.
        contours: smallvec::SmallVec<[Rc<[Origin]>; 1]>,
        /// Whether each contour connects its last vertex to its first.
        closed: bool,
        /// Full stroke width in logical pixels.
        width: Pixels,
        /// Foreground mapped to the full stroke bounds, including caps.
        ink: crate::Ink,
    },
    /// Draws a fitted SVG viewport.
    Svg {
        /// Content box intersected with the ancestor clip.
        clip: Option<Arc<ContentClip>>,
        /// Rounded content boundary before positioning and fitting the artwork.
        rounded_clip: Option<Rc<crate::RoundedClip>>,
        /// Upper-left corner of the fitted viewport.
        origin: Origin,
        /// Fitted viewport in logical pixels.
        size: Size,
        /// Parsed artwork retained through submission.
        asset: crate::SvgAsset,
        /// Foreground applied to the alpha mask, or source colors when absent.
        tint: Option<crate::Ink>,
    },
    /// Draws a shaped paragraph.
    Text {
        /// Effective clip in the same coordinates as the origin.
        clip: Option<Arc<ContentClip>>,
        /// Upper-left corner of the paragraph.
        origin: Origin,
        /// Assigned content box used to clip glyphs.
        size: Size,
        /// Shaped text retained for this command.
        paragraph: Rc<Paragraph>,
        /// Foreground applied to the glyphs.
        color: crate::Ink,
    },
}

impl Draw {
    /// Effective overflow clip for this primitive. Group children carry their own clips.
    pub fn content_clip(&self) -> Option<&ContentClip> {
        match self {
            Self::Group { .. } => None,
            Self::WavyUnderline { clip, .. }
            | Self::BackdropBlur { clip, .. }
            | Self::Shadow { clip, .. }
            | Self::Border { clip, .. }
            | Self::Fill { clip, .. }
            | Self::Polygon { clip, .. }
            | Self::Polyline { clip, .. }
            | Self::Svg { clip, .. }
            | Self::Text { clip, .. } => clip.as_deref(),
        }
    }
}

/// Local content bounds, inherited color and state access during recording.
pub struct PaintContext<'a> {
    #[cfg(any(feature = "gif", feature = "webp"))]
    image_owner: (&'a crate::Instance, crate::NodeId),
    origin: Origin,
    // Position of this node's border box in the clip coordinate system.
    frame_offset: Origin,
    units: UnitResolver,
    /// Assigned content area after removing padding.
    pub size: Size,
    corner_radii: Corners<Pixels>,
    text_color: &'a Foreground,
    commands: &'a mut Vec<Draw>,
    clip: Option<Arc<ContentClip>>,
    next_frame: bool,
    state: &'a Runtime<()>,
    reads: &'a RefCell<Vec<SignalId>>,
}

fn collect_contours(
    contours: impl IntoIterator<Item = impl Into<Rc<[Origin]>>>,
    minimum: usize,
) -> Result<smallvec::SmallVec<[Rc<[Origin]>; 1]>> {
    let mut rings = smallvec::SmallVec::new();
    for contour in contours {
        let points = contour.into();
        validate_points(&points)?;
        if points.len() >= minimum {
            rings.push(points);
        }
    }
    Ok(rings)
}

fn normalized_opacity(value: f32) -> Result<f32> {
    anyhow::ensure!(value.is_finite(), "opacity must be finite");
    Ok(value.clamp(0.0, 1.0))
}

fn opacity_alpha(value: f32) -> u8 {
    (value * 255.0).round() as u8
}

fn validate_points(points: &[Origin]) -> Result<()> {
    anyhow::ensure!(
        points
            .iter()
            .all(|p| p.x.get().is_finite() && p.y.get().is_finite()),
        "invalid path point"
    );
    Ok(())
}

impl<'paint> PaintContext<'paint> {
    /// Draws shaped glyphs at the local origin. The size bounds the ink and clips the glyphs.
    pub fn draw_paragraph(
        &mut self,
        paragraph: Rc<Paragraph>,
        size: Size,
        color: impl Into<crate::Ink>,
    ) {
        self.commands.push(Draw::Text {
            clip: self.clip.clone(),
            origin: self.origin,
            size,
            paragraph,
            color: color.into(),
        });
    }

    /// The element's current corner radii fitted to the local drawing size.
    pub fn corner_radii(&self) -> Corners<Pixels> {
        self.corner_radii.fit_to(self.size)
    }

    /// Reads the inherited foreground and tracks a bound color for repaint.
    pub fn text_color(&self) -> Result<crate::Ink> {
        match self.text_color {
            Foreground::Fixed(ink) => Ok(ink.clone()),
            Foreground::Bound(signal) => Ok((*self.read(*signal)?).into()),
            Foreground::Gradient(signal) => Ok(self.read(*signal)?.clone().into()),
            Foreground::Ink(signal) => Ok(self.read(*signal)?.clone()),
            #[cfg(feature = "shader")]
            Foreground::ScalarShader(fill) => fill.sample(self),
            #[cfg(feature = "shader")]
            Foreground::VectorShader(fill) => fill.sample(self),
        }
    }

    /// Samples a signal and subscribes this element to changes after successful paint.
    /// The value can stay borrowed while recording drawing commands.
    pub fn read<T: 'static>(&self, signal: Signal<T>) -> Result<&'paint T> {
        let value = self.state.read(signal)?;
        let mut reads = self.reads.borrow_mut();
        if !reads.contains(&signal.id()) {
            reads.push(signal.id());
        }
        Ok(value)
    }

    /// Requests another host-paced frame after this view frame is submitted.
    /// Call on each paint while animation is active. Requests from siblings coalesce.
    pub fn request_animation_frame(&mut self) {
        self.next_frame = true;
    }

    /// Composites the callback's drawing as one layer at the given opacity.
    /// Finite values are clamped to 0–1. Opacity zero still runs the callback.
    pub fn with_opacity<R>(
        &mut self,
        opacity: f32,
        draw: impl FnOnce(&mut PaintContext<'_>) -> Result<R>,
    ) -> Result<R> {
        self.with_group(opacity_alpha(normalized_opacity(opacity)?), None, draw)
    }

    /// Recolors the callback's composited pixels by sampling the map at their luminance.
    pub fn with_gradient_map<R>(
        &mut self,
        map: &crate::GradientMap,
        draw: impl FnOnce(&mut PaintContext<'_>) -> Result<R>,
    ) -> Result<R> {
        self.with_group(255, (map.strength() > 0.).then_some(map), draw)
    }

    fn with_group<R>(
        &mut self,
        alpha: u8,
        map: Option<&crate::GradientMap>,
        draw: impl FnOnce(&mut PaintContext<'_>) -> Result<R>,
    ) -> Result<R> {
        let start = self.commands.len();
        let value = self.scoped(self.origin, self.clip.clone(), draw)?;
        crate::frame::wrap_group(self.commands, start, alpha, map);
        Ok(value)
    }

    /// Draws relative to a translated origin. Nested offsets add together.
    /// Existing clips stay in place and clips created inside use the translated coordinates.
    pub fn with_offset<R>(
        &mut self,
        offset: Origin,
        draw: impl FnOnce(&mut PaintContext<'_>) -> Result<R>,
    ) -> Result<R> {
        let origin = self
            .origin
            .checked_add(offset)
            .context("invalid paint offset")?;
        self.scoped(origin, self.clip.clone(), draw)
    }

    /// Clips drawing inside the callback to a rectangle in content-local coordinates.
    /// Nested clips intersect. The drawing origin and content size stay the same.
    /// An error discards the callback's commands and frame request.
    pub fn with_clip<R>(
        &mut self,
        offset: Origin,
        size: Size,
        draw: impl FnOnce(&mut PaintContext<'_>) -> Result<R>,
    ) -> Result<R> {
        let origin = self
            .origin
            .checked_add(offset)
            .and_then(|local| self.frame_offset.checked_add(local))
            .context("invalid clip origin")?;
        let clip = self
            .clip
            .as_deref()
            .cloned()
            .unwrap_or_default()
            .intersect_rect(origin, size)?;
        self.scoped(self.origin, Some(Arc::new(clip)), draw)
    }

    fn scoped<R>(
        &mut self,
        origin: Origin,
        clip: Option<Arc<ContentClip>>,
        draw: impl FnOnce(&mut PaintContext<'_>) -> Result<R>,
    ) -> Result<R> {
        let start = self.commands.len();
        let mut scoped = PaintContext {
            commands: self.commands,
            origin,
            clip,
            next_frame: false,
            ..*self
        };
        match draw(&mut scoped) {
            Ok(value) => {
                self.next_frame |= scoped.next_frame;
                Ok(value)
            }
            Err(error) => {
                self.commands.truncate(start);
                Err(error)
            }
        }
    }

    fn text_stroke(
        &mut self,
        paragraph: &Paragraph,
        stroke: &crate::text::Stroke,
        foreground: &crate::Ink,
    ) -> Result<()> {
        let color = stroke.color.as_ref().unwrap_or(foreground);
        let height = stroke.thickness.get();
        self.text_line_fills(paragraph, color, |line| {
            let center = line.baseline - stroke.position.get();
            (center - height * 0.5, center + height * 0.5)
        })
    }

    fn wavy_underline(
        &mut self,
        paragraph: &Paragraph,
        stroke: &crate::text::Stroke,
        foreground: &crate::Ink,
    ) -> Result<()> {
        let color = stroke.color.as_ref().unwrap_or(foreground);
        if color.is_clear_solid() {
            return Ok(());
        }
        let height = stroke.thickness * 3.0;
        anyhow::ensure!(height.is_extent(), "invalid underline extent");
        self.with_clip(Origin::default(), self.size, |cx| {
            for line in &paragraph.lines {
                let (left, right) = line.advance_bounds();
                let center = px(line.baseline) - stroke.position;
                let top = center - height * 0.5;
                if right > left
                    && right > 0.
                    && left < cx.size.width.get()
                    && top < cx.size.height
                    && center + height * 0.5 > px(0.)
                {
                    cx.commands.push(Draw::WavyUnderline {
                        origin: Origin {
                            x: cx.origin.x + px(left),
                            y: cx.origin.y + top,
                        },
                        size: Size {
                            width: px(right - left),
                            height,
                        },
                        color: color.clone(),
                        clip: cx.clip.clone(),
                    });
                }
            }
            Ok(())
        })
    }

    fn text_line_fills(
        &mut self,
        paragraph: &Paragraph,
        color: &crate::Ink,
        vertical_bounds: impl Fn(&Line) -> (f32, f32),
    ) -> Result<()> {
        if color.is_clear_solid() {
            return Ok(());
        }
        let fill = Fill::from(color.clone());
        let spatial = !matches!(color, crate::Ink::Solid(_));
        for line in &paragraph.lines {
            let (left, right) = line.advance_bounds();
            let (top, bottom) = vertical_bounds(line);
            let (x0, x1) = (left.max(0.), right.min(self.size.width.get()));
            let (y0, y1) = (top.max(0.), bottom.min(self.size.height.get()));
            if x1 <= x0 || y1 <= y0 {
                continue;
            }
            let needs_clip = spatial && (left, right, top, bottom) != (x0, x1, y0, y1);
            let (left, right, top, bottom) = if needs_clip {
                (left, right, top, bottom)
            } else {
                (x0, x1, y0, y1)
            };
            let record = |cx: &mut PaintContext<'_>| {
                cx.record_fill(
                    Origin {
                        x: cx.origin.x + px(left),
                        y: cx.origin.y + px(top),
                    },
                    Size {
                        width: px(right - left),
                        height: px(bottom - top),
                    },
                    fill.clone(),
                    Corners::default(),
                );
                Ok(())
            };
            if needs_clip {
                // Sample across the full line when only part of its ink is visible.
                self.with_clip(Origin::default(), self.size, record)?;
            } else {
                record(self)?;
            }
        }
        Ok(())
    }

    /// Fills a polygon in local logical pixels, implicitly closing the contour.
    /// The even-odd rule determines the interior, including at self-intersections.
    /// Gradients use the polygon's unclipped bounding box.
    pub fn fill_polygon(
        &mut self,
        points: impl Into<Rc<[Origin]>>,
        fill: impl Into<Fill>,
    ) -> Result<()> {
        self.fill_polygon_with_rule(points, fill, FillRule::EvenOdd)
    }

    /// Fills a closed polygon in local logical pixels using the given winding rule.
    pub fn fill_polygon_with_rule(
        &mut self,
        points: impl Into<Rc<[Origin]>>,
        fill: impl Into<Fill>,
        rule: FillRule,
    ) -> Result<()> {
        self.fill_contours([points], fill, rule)
    }

    /// Fills straight-sided contours as one shape using a shared paint box.
    /// Each contour closes independently. Contours with fewer than three vertices are skipped.
    pub fn fill_contours(
        &mut self,
        contours: impl IntoIterator<Item = impl Into<Rc<[Origin]>>>,
        fill: impl Into<Fill>,
        rule: FillRule,
    ) -> Result<()> {
        let rings = collect_contours(contours, 3)?;
        if !rings.is_empty() {
            self.commands.push(Draw::Polygon {
                origin: self.origin,
                clip: self.clip.clone(),
                contours: rings,
                rule,
                fill: fill.into(),
            });
        }
        Ok(())
    }

    /// Strokes consecutive local points with round caps and joins.
    /// Gradients span the unclipped bounds of the stroke, including its width.
    /// Fewer than two points or a zero-width stroke produces no drawing.
    pub fn stroke_polyline(
        &mut self,
        points: impl Into<Rc<[Origin]>>,
        width: Pixels,
        ink: impl Into<crate::Ink>,
    ) -> Result<()> {
        self.record_stroke([points], width, ink.into(), false)
    }

    /// Strokes a polygon with round joins, connecting the final vertex to the first.
    /// At least three vertices are required to draw the outline.
    pub fn stroke_polygon(
        &mut self,
        points: impl Into<Rc<[Origin]>>,
        width: Pixels,
        ink: impl Into<crate::Ink>,
    ) -> Result<()> {
        self.stroke_contours([points], width, ink)
    }

    /// Strokes independently closed contours with round joins and one shared ink.
    /// Overlapping outlines composite once. Each contour needs at least three vertices.
    pub fn stroke_contours(
        &mut self,
        contours: impl IntoIterator<Item = impl Into<Rc<[Origin]>>>,
        width: Pixels,
        ink: impl Into<crate::Ink>,
    ) -> Result<()> {
        self.record_stroke(contours, width, ink.into(), true)
    }

    fn record_stroke(
        &mut self,
        contours: impl IntoIterator<Item = impl Into<Rc<[Origin]>>>,
        width: Pixels,
        ink: crate::Ink,
        closed: bool,
    ) -> Result<()> {
        anyhow::ensure!(width.is_extent(), "invalid stroke width");
        let contours = collect_contours(contours, if closed { 3 } else { 2 })?;
        if !contours.is_empty() && width.get() > 0.0 && !ink.is_clear_solid() {
            self.commands.push(Draw::Polyline {
                origin: self.origin,
                clip: self.clip.clone(),
                contours,
                closed,
                width,
                ink,
            });
        }
        Ok(())
    }

    /// Fills a rectangle at an offset from the content origin in logical pixels.
    pub fn fill_rect(&mut self, offset: Origin, size: Size, fill: impl Into<Fill>) -> Result<()> {
        self.fill_rounded_rect(offset, size, px(0.0), fill)
    }

    /// Draws a border inside a content-local rectangle, leaving its interior untouched.
    pub fn stroke_rect(
        &mut self,
        offset: Origin,
        size: Size,
        border: impl Into<Rc<BorderPaint>>,
    ) -> Result<()> {
        size.validate()?;
        let origin = self
            .origin
            .checked_add(offset)
            .context("invalid border origin")?;
        let paint = border.into();
        let w = paint.widths;
        anyhow::ensure!(
            [w.top, w.right, w.bottom, w.left]
                .into_iter()
                .all(Pixels::is_extent),
            "invalid border width"
        );
        anyhow::ensure!(paint.radii.valid_radii(), "invalid corner radius");
        if size.width > px(0.)
            && size.height > px(0.)
            && paint.visible_fills().iter().any(Option::is_some)
        {
            self.commands.push(Draw::Border {
                origin,
                size,
                clip: self.clip.clone(),
                paint,
            });
        }
        Ok(())
    }

    /// Fills a content-local rectangle with a shared radius or individual corner radii.
    /// Oversized radii shrink proportionally to fit the edges.
    pub fn fill_rounded_rect(
        &mut self,
        offset: Origin,
        size: Size,
        radii: impl Into<Corners<Pixels>>,
        fill: impl Into<Fill>,
    ) -> Result<()> {
        let radii = radii.into();
        anyhow::ensure!(radii.valid_radii(), "invalid corner radius");
        anyhow::ensure!(
            size.width.is_extent() && size.height.is_extent(),
            "invalid fill extent"
        );
        let origin = self
            .origin
            .checked_add(offset)
            .context("invalid fill origin")?;
        self.record_fill(origin, size, fill.into(), radii.fit_to(size));
        Ok(())
    }

    /// Fills a circle in content-local logical pixels, mapping the fill to its diameter.
    /// A zero radius records no drawing.
    pub fn fill_circle(
        &mut self,
        center: Origin,
        radius: Pixels,
        fill: impl Into<Fill>,
    ) -> Result<()> {
        anyhow::ensure!(radius.is_extent(), "invalid circle radius");
        let diameter = radius
            .checked_add(radius)
            .context("circle diameter outside layout range")?;
        let offset = Origin {
            x: center.x - radius,
            y: center.y - radius,
        };
        let origin = self
            .origin
            .checked_add(offset)
            .context("invalid circle origin")?;
        if radius.get() > 0.0 {
            self.record_fill(
                origin,
                Size {
                    width: diameter,
                    height: diameter,
                },
                fill.into(),
                Corners::all(radius),
            );
        }
        Ok(())
    }

    /// Records a rectangle at the content origin.
    pub fn fill(&mut self, size: Size, fill: impl Into<Fill>) {
        self.record_fill(self.origin, size, fill.into(), Corners::default());
    }

    fn record_fill(
        &mut self,
        origin: Origin,
        size: Size,
        fill: Fill,
        corner_radii: Corners<Pixels>,
    ) {
        self.commands.push(Draw::Fill {
            clip: self.clip.clone(),
            origin,
            size,
            fill,
            corner_radii,
        });
    }
}

pub(super) type Click = Box<dyn Fn(&mut Context<'_>) -> Result<()>>;

struct MountedBackground {
    placement: crate::bitmap::Placement,
    source: Background,
}

impl MountedBackground {
    fn resolve(style: &Style, units: UnitResolver) -> Result<Option<Box<Self>>> {
        let Some(source) = &style.background else {
            return Ok(None);
        };
        if let Background::GradientColor(color) = source
            && !(0.0..=1.0).contains(&color.position)
        {
            return Err(crate::GradientError::InvalidOffset.into());
        }
        use crate::{bitmap::Placement, style::ObjectFit};
        let placement = match source {
            #[cfg(feature = "image")]
            Background::Fixed(Fill::ImageFile(_)) => {
                Placement::resolve(style, units, ObjectFit::Cover)?
            }
            Background::Fixed(Fill::Image(_) | Fill::CustomImage(_))
            | Background::Bound(_)
            | Background::Image(_)
            | Background::ImageRegion(_)
            | Background::CustomImage(_) => Placement::resolve(style, units, ObjectFit::Cover)?,
            _ => Placement::centered(ObjectFit::Cover),
        };
        Ok(Some(Box::new(Self {
            source: source.clone(),
            placement,
        })))
    }
}
/// When an element records new drawing commands.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RefreshMode {
    /// Reuse commands until a paint dependency or layout changes.
    #[default]
    OnDemand,
    /// Sample on every host frame, including frames with identical inputs.
    Continuous,
}

/// A fixed refresh mode or a runtime-selected mode.
pub enum RefreshPolicy {
    /// Use this mode throughout the element's attachment.
    Fixed(RefreshMode),
    /// Follow the current value of the signal.
    Bound(Signal<RefreshMode>),
}

impl From<RefreshMode> for RefreshPolicy {
    fn from(mode: RefreshMode) -> Self {
        Self::Fixed(mode)
    }
}

impl From<Signal<RefreshMode>> for RefreshPolicy {
    fn from(mode: Signal<RefreshMode>) -> Self {
        Self::Bound(mode)
    }
}

#[derive(Clone, Default)]
pub(super) struct InheritedText {
    animated_color: bool,
    fixed: TextStyleRefinement,
    bindings: crate::style::Bindings,
}

type Record = Box<dyn Fn(&Scene, NodeId, &mut PaintContext) -> Result<()>>;

type IndicatorPaint = Box<dyn Fn(&mut PaintContext<'_>) -> Result<()>>;

pub(crate) struct FocusIndicator {
    paint: IndicatorPaint,
    pub(crate) visible: bool,
}

fn record_content<E: Element>(scene: &Scene, id: NodeId, cx: &mut PaintContext) -> Result<()> {
    let body = scene.body::<StyledBody<E>>(id)?;
    cx.size = body.content_size(cx.size);
    cx.origin = body.origin();
    body.content.paint(cx)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FramePaint {
    Empty,
    FixedBackground,
    FixedLabel,
    Recorded,
}

impl FramePaint {
    pub(super) fn is_fixed_draw(self) -> bool {
        matches!(self, Self::FixedBackground | Self::FixedLabel)
    }
}

pub(super) struct MountedNode {
    pub(super) id: NodeId,
    pub(super) offset: usize,
    pub(super) frame_paint: FramePaint,
    pub(super) visible: bool,
    pub(super) local_visible: bool,
    pub(super) focusable: bool,
    paint_mode: RefreshPolicy,
    pub(super) text_style: InheritedText,
    text_color: Foreground,
    pub(crate) corner_radii: Corners<Pixels>,
    backdrop_blur: Pixels,
    background: Option<Box<MountedBackground>>,
    border: Option<Rc<crate::style::BorderPaint>>,
    outline: Option<Box<outline::MountedOutline>>,
    shadows: Option<Box<shadow::Shadows>>,
    pub(super) binding_target: Option<NodeId>,
    record: Option<Record>,
    pub(super) owned: crate::Owned,
    pub(crate) handlers: Option<Box<Handlers>>,
    pub(super) opacity: f32,
    pub(super) gradient_map: Option<crate::GradientMap>,
    pub(super) z_index: i32,
    pub(crate) cursor: Option<crate::style::Cursor>,
    pub(crate) hit_testable: bool,
    pub(crate) overflow_clip: OverflowClip,
    pub(crate) scroll_axes: crate::pointer::ScrollAxes,
}

impl MountedNode {
    pub(super) fn set_border_widths(&mut self, widths: crate::Edges) {
        if let Some(border) = &mut self.border
            && border.widths != widths
        {
            Rc::make_mut(border).widths = widths;
        }
    }

    pub(super) fn set_corner_radii(&mut self, radii: Corners<Pixels>) -> bool {
        if self.corner_radii == radii {
            return false;
        }
        self.corner_radii = radii;
        if let Some(border) = &mut self.border {
            Rc::make_mut(border).radii = radii;
        }
        if let Some(outline) = &mut self.outline {
            outline.set_radii(radii);
        }
        true
    }

    fn transition_color(&self, property: crate::TransitionProperty) -> Option<crate::Rgba> {
        match property {
            crate::TransitionProperty::BackgroundColor => match &self.background.as_ref()?.source {
                Background::Fixed(Fill::Solid(color)) => Some(*color),
                _ => None,
            },
            crate::TransitionProperty::OutlineColor => self.outline.as_ref()?.color(),
            edge => self.border.as_ref()?.solid_color(edge.border_edge()?),
        }
    }

    pub(super) fn set_transition_color(
        &mut self,
        property: crate::TransitionProperty,
        color: crate::Rgba,
    ) -> bool {
        match property {
            crate::TransitionProperty::BackgroundColor => {
                let Some(Background::Fixed(Fill::Solid(current))) =
                    self.background.as_mut().map(|bg| &mut bg.source)
                else {
                    return false;
                };
                if *current == color {
                    return false;
                }
                *current = color;
            }
            crate::TransitionProperty::OutlineColor => {
                let Some(outline) = &mut self.outline else {
                    return false;
                };
                if outline.color() == Some(color) {
                    return false;
                }
                outline.set_color(color);
            }
            edge => {
                let Some(side) = edge.border_edge() else {
                    return false;
                };
                let Some(border) = &mut self.border else {
                    return false;
                };
                if border
                    .solid_color(side)
                    .is_none_or(|current| current == color)
                {
                    return false;
                }
                let fills = &mut Rc::make_mut(border).fills;
                *[
                    &mut fills.top,
                    &mut fills.right,
                    &mut fills.bottom,
                    &mut fills.left,
                ][side] = Some(Fill::Solid(color));
            }
        }
        true
    }

    pub(super) fn set_transition_length(
        &mut self,
        property: crate::TransitionProperty,
        value: Pixels,
    ) -> bool {
        if property == crate::TransitionProperty::BackdropBlur {
            return std::mem::replace(&mut self.backdrop_blur, value) != value;
        }
        self.outline
            .as_deref_mut()
            .is_some_and(|outline| outline.set_length(property, value))
    }

    pub(super) fn set_palette_position(&mut self, value: f32) -> bool {
        if let Some(paint) = &mut self.background
            && let Background::GradientColor(color) = &mut paint.source
            && color.position != value
        {
            color.position = value;
            return true;
        }
        false
    }

    pub(super) fn alpha(&self) -> u8 {
        opacity_alpha(self.opacity)
    }

    pub(super) fn is_group(&self) -> bool {
        self.alpha() < 255 || self.gradient_map.is_some()
    }

    fn new(
        id: NodeId,
        handlers: Option<Box<Handlers>>,
        focusable: bool,
        mounted: style_binding::PreparedStyle,
        fixed_label: bool,
        paint_mode: RefreshPolicy,
        record: Option<Record>,
    ) -> Self {
        let style = mounted.style;
        let frame_paint = if handlers
            .as_deref()
            .is_some_and(|h| h.focus_indicator.is_some())
            || mounted.backdrop_blur > px(0.)
            || mounted.border.is_some()
            || mounted.outline.is_some()
            || mounted.shadows.is_some()
            || !mounted.fixed
            || !matches!(paint_mode, RefreshPolicy::Fixed(RefreshMode::OnDemand))
        {
            FramePaint::Recorded
        } else if record.is_none() {
            match &style.background {
                None => FramePaint::Empty,
                Some(Background::Fixed(_) | Background::GradientColor(_)) => {
                    FramePaint::FixedBackground
                }
                Some(_) => FramePaint::Recorded,
            }
        } else if fixed_label
            && style.background.is_none()
            && !style.text.attributes.as_deref().is_some_and(|a| {
                a.underline
                    .is_some_and(|s| s != crate::style::UnderlineStyle::None)
                    || a.strikethrough == Some(true)
                    || a.text_background
                        .as_ref()
                        .is_some_and(|ink| !ink.is_clear_solid())
            })
            && matches!(style.text.color, Foreground::Fixed(_))
        {
            FramePaint::FixedLabel
        } else {
            FramePaint::Recorded
        };
        Self {
            id,
            offset: 0,
            local_visible: mounted.visible,
            focusable,
            visible: mounted.visible,
            frame_paint,
            paint_mode,
            binding_target: None,
            handlers,
            opacity: mounted.opacity,
            gradient_map: style_binding::gradient_map(&style),
            z_index: style
                .appearance
                .as_deref()
                .and_then(|a| a.z_index)
                .unwrap_or_default(),
            overflow_clip: OverflowClip::from_layout(
                style.layout.as_deref(),
                mounted.scroll_axes.gutter,
            ),
            scroll_axes: mounted.scroll_axes,
            cursor: style.appearance.as_deref().and_then(|a| a.cursor),
            hit_testable: style.appearance.as_deref().and_then(|a| a.pointer_events)
                != Some(PointerEvents::None),
            text_style: mounted.text,
            text_color: style.text.color,
            corner_radii: style.corner_radii,
            backdrop_blur: mounted.backdrop_blur,
            background: mounted.background,
            border: mounted.border,
            outline: mounted.outline,
            shadows: mounted.shadows,
            owned: Default::default(),
            record,
        }
    }

    pub(super) fn paint(
        &self,
        instance: &crate::Instance,
        state: &Runtime<()>,
        frame_clips: Option<(&NodeClips, Origin)>,
        commands: &mut Vec<Draw>,
    ) -> Result<bool> {
        let scene = &instance.scene;
        if scene.is_pending() {
            anyhow::bail!("layout pending");
        }
        instance.paint_reads.borrow_mut().clear();
        if !self.visible {
            return Ok(false);
        }
        let geometry = scene.geometry(self.id).context("missing layout geometry")?;
        let start = commands.len();
        let size = geometry.metrics.size;
        let local_clips;
        let (clips, frame_offset) = match frame_clips {
            Some(frame) => frame,
            None => {
                local_clips = self.overflow_clip.resolve(
                    None,
                    Origin::default(),
                    self.scroll_axes.viewport(size),
                    size,
                    self.corner_radii,
                )?;
                (&local_clips, Origin::default())
            }
        };
        let mut cx = PaintContext {
            #[cfg(any(feature = "gif", feature = "webp"))]
            image_owner: (instance, self.id),
            clip: clips.content.clone(),
            frame_offset,
            units: instance.units,
            origin: Origin::default(),
            size,
            text_color: &self.text_color,
            corner_radii: self.corner_radii,
            commands,
            next_frame: false,
            state,
            reads: &instance.paint_reads,
        };
        let mode = match self.paint_mode {
            RefreshPolicy::Fixed(mode) => mode,
            RefreshPolicy::Bound(source) => *cx.read(source)?,
        };
        cx.next_frame = mode == RefreshMode::Continuous;
        if self.backdrop_blur > px(0.) {
            cx.commands.push(Draw::BackdropBlur {
                clip: clips.inherited.clone(),
                origin: Origin::default(),
                size,
                radius: self.backdrop_blur,
                corner_radii: self.corner_radii.fit_to(size),
            });
        }
        if let Some(shadows) = &self.shadows {
            shadows.record(
                false,
                self.corner_radii,
                size,
                &clips.inherited,
                cx.commands,
            );
        }
        if let Some(paint) = &self.background {
            let mut fill = paint.source.sample(&cx)?;
            if let Fill::Image(image) = &mut fill {
                image.placement = paint.placement;
            }
            if let Fill::CustomImage(image) = &mut fill {
                image.placement = paint.placement;
            }
            #[cfg(feature = "image")]
            if let Fill::ImageFile(file) = &mut fill {
                file.placement = paint.placement;
            }
            cx.commands.push(Draw::Fill {
                clip: clips.inherited.clone(),
                origin: Origin::default(),
                size,
                fill,
                corner_radii: self.corner_radii.fit_to(size),
            });
        }
        if let Some(shadows) = &self.shadows {
            shadows.record(true, self.corner_radii, size, &clips.inherited, cx.commands);
        }
        if let Some(paint) = &self.border
            && paint.visible_fills().iter().any(Option::is_some)
        {
            cx.commands.push(Draw::Border {
                clip: clips.inherited.clone(),
                origin: Origin::default(),
                size,
                paint: Rc::clone(paint),
            });
        }
        if let Some(indicator) = self
            .handlers
            .as_deref()
            .and_then(|h| h.focus_indicator.as_ref())
            && indicator.visible
        {
            if let Err(error) = (indicator.paint)(&mut cx) {
                commands.truncate(start);
                return Err(error);
            }
            // A painter can customize its own bounds without changing the content pass.
            cx.size = size;
        }
        if let Some(record) = &self.record
            && let Err(error) = record(scene, self.id, &mut cx)
        {
            commands.truncate(start);
            return Err(error);
        }
        if let Some(outline) = &self.outline
            && let Some(draw) = outline.draw(size, &clips.inherited)?
        {
            cx.commands.push(draw);
        }
        Ok(cx.next_frame)
    }
}

type FrameCallback = Box<dyn FnMut(std::time::Duration, &mut Context<'_>) -> Result<()>>;

pub(super) struct FrameHandler {
    enabled: Option<Signal<bool>>,
    pub callback: FrameCallback,
}

impl FrameHandler {
    pub fn is_enabled(&self, state: &Runtime<()>) -> Result<bool> {
        match self.enabled {
            Some(source) => Ok(*state.read(source)?),
            None => Ok(true),
        }
    }
}

pub(super) struct Installation {
    pub scroll_handle: Option<crate::ScrollHandle>,
    pub on_frame: Option<FrameHandler>,
    pub node: MountedNode,
    pub children: Vec<Child>,
    pub watches: Vec<Watch>,
}

pub(super) struct PaintSubscription {
    repaints: Repaints,
    node: NodeId,
}

impl PaintSubscription {
    pub(super) fn observe(
        state: &mut Runtime<()>,
        source: SignalId,
        target: NodeId,
        node: NodeId,
        repaints: &Repaints,
    ) -> Result<crate::state::BindingId> {
        let subscription = Self {
            repaints: repaints.clone(),
            node,
        };
        Ok(state.observe(source, target, move || subscription.invalidate())?)
    }

    fn invalidate(&self) {
        self.repaints.borrow_mut().invalidate(self.node);
    }
}

impl Drop for PaintSubscription {
    fn drop(&mut self) {
        // A retired source can no longer invalidate commands through its binding.
        self.repaints.borrow_mut().commands.remove(&self.node);
    }
}

pub(super) type Watch =
    Box<dyn FnOnce(&mut Attachment<'_>, NodeId, NodeId, &[usize], Option<&Repaints>) -> Result<()>>;
type MountResources<'a> = (UnitResolver, &'a crate::assets::Resources);
type Install =
    Box<dyn FnOnce(Target<'_>, &mut Context<'_>, MountResources<'_>) -> Result<Installation>>;

enum Target<'a> {
    Root(&'a mut Option<Scene>),
    Child {
        scene: &'a mut Scene,
        parent: NodeId,
        index: usize,
        inherited: InheritedText,
    },
}

/// Opaque, single-use declaration. Only the host can attach it.
pub struct AnyElement(Install);

impl AnyElement {
    fn new<E: Element>(
        fixed_label: bool,
        make: impl FnOnce(&mut Context<'_>, MountResources<'_>) -> Result<Node<E>> + 'static,
    ) -> Self {
        Self(Box::new(move |target, cx, resources| {
            let (units, _) = resources;
            let mut node = make(cx, resources)?;
            if let Some(input) = node.handlers.as_deref().and_then(|h| h.text_input) {
                cx.read(input)?;
            }
            if let Some(source) = node.on_frame.as_ref().and_then(|handler| handler.enabled) {
                node.watches.push(Box::new(move |cx, target, _, _, _| {
                    // Wake an idle host. Frame scheduling reads the live gate directly.
                    cx.state.observe(source.id(), target, || {})?;
                    Ok(())
                }));
            }
            let inherited = match &target {
                Target::Root(_) => &InheritedText::default(),
                Target::Child { inherited, .. } => inherited,
            };
            let mounted = style_binding::prepare(&mut node, inherited, cx, units)?;
            let style = &mounted.style;
            let item = style_binding::item_style(style, mounted.scroll_axes.gutter, units)?;
            if let Some(tip) = node
                .handlers
                .as_deref()
                .and_then(|handlers| handlers.tooltip.as_ref())
            {
                Text::validate_content(tip.text.as_str())?;
                tip.resolve(&crate::TooltipStyle::default()).validate()?;
            }
            node.body.prepare_content(style)?;
            let body = StyledBody::new(node.body, style, units, mounted.scroll_axes.gutter)?;
            let item_sizing = body.bounds.item_sizing(style, units, false)?;
            if let Some(handlers) = node.handlers.as_mut() {
                handlers.padding = body.bounds.content_insets();
            }
            let id = match target {
                Target::Root(slot) => slot
                    .insert(Scene::with_root_style(body, item, item_sizing))
                    .root(),
                Target::Child {
                    scene,
                    parent,
                    index,
                    ..
                } => scene.mount_entry(parent, Some(index), body, item, item_sizing)?,
            };
            Ok(Installation {
                scroll_handle: node.scroll_handle,
                on_frame: node.on_frame,
                node: MountedNode::new(
                    id,
                    node.handlers,
                    node.focusable,
                    mounted,
                    fixed_label,
                    node.paint_mode,
                    node.record,
                ),
                children: node.children.0,
                watches: node.watches,
            })
        }))
    }

    pub(super) fn root(
        self,
        cx: &mut Context<'_>,
        resources: MountResources<'_>,
    ) -> Result<(Scene, Installation)> {
        let mut scene = None;
        let installed = (self.0)(Target::Root(&mut scene), cx, resources)?;
        Ok((
            scene.expect("successful root attachment creates a scene"),
            installed,
        ))
    }

    pub(super) fn attach(
        self,
        scene: &mut Scene,
        parent: NodeId,
        index: usize,
        inherited: InheritedText,
        resources: MountResources<'_>,
        cx: &mut Context<'_>,
    ) -> Result<Installation> {
        (self.0)(
            Target::Child {
                scene,
                parent,
                index,
                inherited,
            },
            cx,
            resources,
        )
    }
}

impl IntoElement for AnyElement {
    fn into_element(self) -> Self {
        self
    }
}

pub(super) type Factory = Box<dyn Fn() -> AnyElement>;

pub(super) enum Child {
    List(crate::uniform_list::UpdateList),
    Fixed(AnyElement),
    When(Signal<bool>, Factory),
}

/// Ordered child declarations owned by a component until it places them in a container.
#[derive(Default)]
pub struct Children(pub(super) Vec<Child>);

impl<I: IntoIterator<Item: IntoElement>> From<I> for Children {
    fn from(elements: I) -> Self {
        Self(
            elements
                .into_iter()
                .map(|element| Child::Fixed(element.into_element()))
                .collect(),
        )
    }
}

/// A builder that accepts child declarations in order.
pub trait ParentElement: Sized {
    /// Returns the slot that receives this component's children.
    fn children_mut(&mut self) -> &mut Children;

    /// Appends a fixed child.
    fn child(mut self, child: impl IntoElement) -> Self {
        self.children_mut()
            .0
            .push(Child::Fixed(child.into_element()));
        self
    }

    /// Appends elements from an iterator or a component's child slot in declaration order.
    fn children(mut self, children: impl Into<Children>) -> Self {
        let children = children.into();
        let slot = self.children_mut();
        if slot.0.is_empty() {
            *slot = children;
        } else {
            slot.0.extend(children.0);
        }
        self
    }

    /// Mounts a fresh child each time the signal becomes true.
    fn child_when<T: IntoElement>(
        mut self,
        open: Signal<bool>,
        make: impl Fn() -> T + 'static,
    ) -> Self {
        self.children_mut()
            .0
            .push(Child::When(open, Box::new(move || make().into_element())));
        self
    }
}

/// A primitive and the children, handlers and styles to attach with it.
pub struct Node<E> {
    focusable: bool,
    scroll_handle: Option<crate::ScrollHandle>,
    record: Option<Record>,
    on_frame: Option<FrameHandler>,
    paint_mode: RefreshPolicy,
    style: StyleRefinement,
    pub(super) watches: Vec<Watch>,
    body: E,
    pub(super) children: Children,
    pub(crate) handlers: Option<Box<Handlers>>,
}

/// Wraps a custom layout body in the common element builder.
pub fn element<E: Element>(body: E) -> Node<E> {
    node(body, Some(Box::new(record_content::<E>)))
}

fn node<E>(body: E, record: Option<Record>) -> Node<E> {
    Node {
        focusable: false,
        scroll_handle: None,
        record,
        on_frame: None,
        paint_mode: RefreshMode::OnDemand.into(),
        style: StyleRefinement::default(),
        watches: Vec::new(),
        body,
        children: Children::default(),
        handlers: None,
    }
}

impl<E> Styled for Node<E> {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl<E: Element> IntoElement for Node<E> {
    fn into_element(self) -> AnyElement {
        AnyElement::new(false, move |_, _| Ok(self))
    }
}

impl<E: Element> ParentElement for Node<E> {
    fn children_mut(&mut self) -> &mut Children {
        &mut self.children
    }
}

impl<E: Element> Node<E> {
    /// Allows focus by primary press or through `Ui::focus`. Tab navigation is enabled by default.
    pub fn focusable(mut self) -> Self {
        self.focusable = true;
        self
    }

    /// Makes this element focusable and exposes its editing state to the host while focused.
    /// The first visible label bound to the same state supplies the insertion point.
    pub fn text_input(mut self, state: crate::Signal<crate::text::TextInputState>) -> Self {
        self.handlers
            .get_or_insert_with(Default::default)
            .text_input = Some(state);
        self.focusable()
    }

    /// Makes this element focusable and controls whether Tab and Shift+Tab can land on it.
    /// A signal is sampled on navigation. Changing it preserves the current focus.
    pub fn tab_stop(mut self, enabled: impl Into<crate::style::Property<bool>>) -> Self {
        self.handlers.get_or_insert_with(Default::default).tab_stop = Some(enabled.into());
        self.focusable()
    }

    /// Makes this element focusable and orders it within its Tab group or view.
    /// Lower indices come first. The default is zero, with ties in tree order.
    pub fn tab_index(mut self, index: impl Into<crate::style::Property<i32>>) -> Self {
        self.handlers.get_or_insert_with(Default::default).tab_index = Some(index.into());
        self.focusable()
    }

    /// Keeps this subtree together at `index` in the enclosing Tab sequence.
    /// A focusable group container uses its own Tab index inside the group.
    pub fn tab_group(mut self, index: impl Into<crate::style::Property<i32>>) -> Self {
        self.handlers.get_or_insert_with(Default::default).tab_group = Some(index.into());
        self
    }

    /// Makes this element focusable and paints its keyboard focus indicator over the
    /// background, before its content. The painter receives border-box coordinates.
    pub fn focus_indicator(
        mut self,
        paint: impl Fn(&mut PaintContext) -> Result<()> + 'static,
    ) -> Self {
        let handlers = self.handlers.get_or_insert_with(Default::default);
        handlers.focus_indicator = Some(FocusIndicator {
            paint: Box::new(paint),
            visible: false,
        });
        self.focusable()
    }

    /// Appends a child in declaration order.
    pub fn child(self, child: impl IntoElement) -> Self {
        ParentElement::child(self, child)
    }

    /// Appends elements from an iterator or a component's child slot in declaration order.
    pub fn children(self, children: impl Into<Children>) -> Self {
        ParentElement::children(self, children)
    }

    /// Registers the handler invoked on a direct click.
    pub fn on_click(mut self, click: impl Fn(&mut Context<'_>) -> Result<()> + 'static) -> Self {
        self.handlers.get_or_insert_with(Default::default).click = Some(Box::new(click));
        self
    }

    /// Requests successive host frame opportunities while this node is visible.
    /// The timestamp is the window’s monotonic animation time. Signal writes apply before the next layout.
    pub fn on_frame(
        mut self,
        handler: impl FnMut(std::time::Duration, &mut Context<'_>) -> Result<()> + 'static,
    ) -> Self {
        self.on_frame = Some(FrameHandler {
            enabled: None,
            callback: Box::new(handler),
        });
        self
    }

    /// Requests host frames while this node is visible and `enabled` is true.
    /// Disabling the signal skips pending callbacks and retains the handler's state.
    pub fn on_frame_when(
        mut self,
        enabled: Signal<bool>,
        handler: impl FnMut(std::time::Duration, &mut Context<'_>) -> Result<()> + 'static,
    ) -> Self {
        self.on_frame = Some(FrameHandler {
            enabled: Some(enabled),
            callback: Box::new(handler),
        });
        self
    }

    /// Selects a fixed or signal-driven refresh mode. The default is on demand.
    pub fn refresh_mode(mut self, mode: impl Into<RefreshPolicy>) -> Self {
        self.paint_mode = mode.into();
        self
    }

    /// Mounts the generated child while its boolean signal is true.
    pub fn child_when<T: IntoElement>(
        self,
        open: Signal<bool>,
        make: impl Fn() -> T + 'static,
    ) -> Self {
        ParentElement::child_when(self, open, make)
    }
}

/// A fixed label or a retained label bound to application state.
pub enum TextContent {
    /// Text supplied at attachment.
    Fixed(SharedString),
    /// Text updated from the signal at flush.
    Bound(Signal<SharedString>),
    /// Displays the editing buffer including preedit, within the text leaf's byte budget around the caret.
    Input(Signal<crate::text::TextInputState>),
}

impl From<Signal<crate::text::TextInputState>> for TextContent {
    fn from(input: Signal<crate::text::TextInputState>) -> Self {
        Self::Input(input)
    }
}

impl<T: Into<SharedString>> From<T> for TextContent {
    fn from(value: T) -> Self {
        Self::Fixed(value.into())
    }
}

impl From<Signal<SharedString>> for TextContent {
    fn from(value: Signal<SharedString>) -> Self {
        Self::Bound(value)
    }
}

struct Label {
    content: TextContent,
    style: StyleRefinement,
}

impl Styled for Label {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

/// Declares a text leaf that uses the view's font engine.
pub fn text(content: impl Into<TextContent>) -> impl IntoElement + Styled {
    Label {
        content: content.into(),
        style: StyleRefinement::default(),
    }
}

macro_rules! text_elements {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl IntoElement for $ty {
                fn into_element(self) -> AnyElement {
                    text(self).into_element()
                }
            }
        )+
    };
}

text_elements!(
    &str,
    String,
    &String,
    SharedString,
    &SharedString,
    Signal<SharedString>,
    TextContent
);

fn content_watch<T: 'static>(signal: Signal<T>, update: crate::ContentUpdate) -> Watch {
    Box::new(move |cx, target, id, _, repaints| {
        let pending = Rc::clone(cx.content_updates);
        let view = ViewId(cx.scope);
        cx.state.bind(signal, target, move |_, _| {
            pending.borrow_mut().insert((view, id), update);
        })?;
        if let Some(repaints) = repaints {
            repaints.borrow_mut().invalidate(id);
        }
        Ok(())
    })
}

impl IntoElement for Label {
    fn into_element(self) -> AnyElement {
        let fixed = matches!(self.content, TextContent::Fixed(_));
        AnyElement::new(fixed, move |cx, _| {
            let input = match &self.content {
                TextContent::Input(s) => Some(*s),
                _ => None,
            };
            let (initial, watch) = match self.content {
                TextContent::Fixed(value) => (value, None),
                TextContent::Bound(signal) => (
                    cx.read(signal)?.clone(),
                    Some(content_watch(signal, crate::ContentUpdate::Text(signal))),
                ),
                TextContent::Input(signal) => (
                    cx.read(signal)?.label_content(),
                    Some(content_watch(
                        signal,
                        crate::ContentUpdate::InputText(signal),
                    )),
                ),
            };
            let mut leaf = Text::new(
                Rc::clone(cx.engine.engine()),
                initial,
                TextStyle::default(),
                true,
            )?;
            if let Some(input) = input {
                leaf.bind_input(input);
            }
            let mut node = element(leaf);
            node.style = self.style;
            if let Some(watch) = watch {
                node.watches.push(watch);
            }
            Ok(node)
        })
    }
}

/// A general-purpose container with children, styles and event handlers.
pub type Div = Node<FlexLayout>;

impl Div {
    /// Tracks this container's overflow and connects it to a scrollbar.
    /// Enable scrolling with an overflow style such as `.overflow_y_scroll()`.
    pub fn track_scroll(mut self, handle: &crate::ScrollHandle) -> Self {
        self.scroll_handle = Some(handle.clone());
        self
    }

    /// Sets the drawing callback for this container's content box.
    /// It runs after the background and before the children. Signal reads track repaint.
    pub fn paint(mut self, draw: impl Fn(&mut PaintContext) -> Result<()> + 'static) -> Self {
        self.record = Some(Box::new(move |scene, id, cx| {
            record_content::<FlexLayout>(scene, id, cx)?;
            draw(cx)
        }));
        self
    }
}

/// Creates a container that stacks children vertically by default.
pub fn div() -> Div {
    node(
        FlexLayout {
            direction: FlexDirection::Column,
            justify_content: crate::style::Distribution::Start,
            align_items: Align::Start,
            gap: px(0.0),
        },
        None,
    )
}
