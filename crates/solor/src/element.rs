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
use crate::style::{Align, Background, Corners, Foreground, TextStyleRefinement};
use crate::style::{PointerEvents, Visibility};
use crate::text::{Line, Paragraph, Text, TextStyle};
use crate::{Fill, Pixels, Rgba, Style, StyleRefinement, Styled, StyledBody, px};

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
        self.set_decorations(&style.text)?;
        Ok(())
    }
    fn paint(&self, cx: &mut PaintContext) -> Result<()> {
        let paragraph = self.resolve(Query::Exact(cx.size.width))?;
        let color = cx.text_color()?;
        let decorations = self.decorations.as_deref();
        let stroke_color = decorations.and_then(|d| d.color).map(crate::Ink::from);
        let stroke_color = stroke_color.as_ref().unwrap_or(&color);
        if let Some(background) = decorations.and_then(|d| d.background) {
            cx.text_line_fills(&paragraph, &background.into(), |line| {
                (line.top, line.top + line.height)
            });
        }
        if let Some(d) = decorations
            && let Some((stroke, kind)) = d.underline
        {
            if kind == crate::style::UnderlineStyle::Wavy {
                cx.wavy_underline(&paragraph, stroke, d.thickness, stroke_color)?;
            } else {
                cx.text_stroke(&paragraph, stroke, d.thickness, stroke_color);
            }
        }
        cx.commands.push(Draw::Text {
            clip: cx.clip.clone(),
            origin: cx.origin,
            size: cx.size,
            paragraph: Rc::clone(&paragraph),
            color: color.clone(),
        });
        if let Some(d) = decorations
            && let Some(stroke) = d.strike
        {
            cx.text_stroke(&paragraph, stroke, d.thickness, stroke_color);
        }
        Ok(())
    }
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
    /// Fills a closed contour using the even-odd rule.
    Polygon {
        /// Effective clip in view coordinates.
        clip: Option<Arc<ContentClip>>,
        /// Content origin for the local vertices.
        origin: Origin,
        /// Vertices in contour order. The last vertex connects to the first.
        points: Rc<[Origin]>,
        /// Paint mapped to the complete polygon bounds.
        fill: Fill,
    },
    /// Strokes an open polyline with round caps and joins.
    Polyline {
        /// Effective clip in view coordinates.
        clip: Option<Arc<ContentClip>>,
        /// Content origin for the local points.
        origin: Origin,
        /// Ordered points in logical pixels relative to the origin.
        points: Rc<[Origin]>,
        /// Full stroke width in logical pixels.
        width: Pixels,
        /// Stroke color.
        color: Rgba,
    },
    /// Draws a fitted SVG viewport.
    Svg {
        /// Content box intersected with the ancestor clip.
        clip: Option<Arc<ContentClip>>,
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

/// Local content bounds, inherited color and state access during recording.
pub struct PaintContext<'a> {
    origin: Origin,
    // Position of this node's border box in the clip coordinate system.
    frame_offset: Origin,
    /// Assigned content area after removing padding.
    pub size: Size,
    text_color: &'a Foreground,
    commands: &'a mut Vec<Draw>,
    clip: Option<Arc<ContentClip>>,
    next_frame: bool,
    state: &'a Runtime<()>,
    reads: &'a RefCell<Vec<SignalId>>,
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
    /// Reads the inherited foreground and tracks a bound color for repaint.
    pub fn text_color(&self) -> Result<crate::Ink> {
        match self.text_color {
            Foreground::Fixed(ink) => Ok(ink.clone()),
            Foreground::Bound(signal) => Ok((*self.read(*signal)?).into()),
            Foreground::Gradient(signal) => Ok(self.read(*signal)?.clone().into()),
            Foreground::Ink(signal) => Ok(self.read(*signal)?.clone()),
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
        let start = self.commands.len();
        let mut scoped = PaintContext {
            commands: self.commands,
            clip: Some(Arc::new(clip)),
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
        stroke: crate::text::Stroke,
        thickness: Option<Pixels>,
        color: &crate::Ink,
    ) {
        let em = paragraph.style.size.get();
        let height = thickness.map_or(stroke.thickness * em, Pixels::get);
        self.text_line_fills(paragraph, color, |line| {
            let center = line.baseline - stroke.position * em;
            (center - height * 0.5, center + height * 0.5)
        });
    }

    fn wavy_underline(
        &mut self,
        paragraph: &Paragraph,
        stroke: crate::text::Stroke,
        thickness: Option<Pixels>,
        color: &crate::Ink,
    ) -> Result<()> {
        if matches!(color, crate::Ink::Solid(c) if c.alpha == 0) {
            return Ok(());
        }
        let em = paragraph.style.size.get();
        let height = thickness.unwrap_or(px(stroke.thickness * em)) * 3.0;
        anyhow::ensure!(height.is_extent(), "invalid underline extent");
        self.with_clip(Origin::default(), self.size, |cx| {
            for line in &paragraph.lines {
                let (left, right) = line.advance_bounds();
                let center = px(line.baseline - stroke.position * em);
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
    ) {
        if matches!(color, crate::Ink::Solid(c) if c.alpha == 0) {
            return;
        }
        for line in &paragraph.lines {
            let (top, bottom) = vertical_bounds(line);
            let top = top.max(0.0);
            let bottom = bottom.min(self.size.height.get());
            let (left, right) = line.advance_bounds();
            let left = left.max(0.0);
            let right = right.min(self.size.width.get());
            if right > left && bottom > top {
                self.commands.push(Draw::Fill {
                    origin: Origin {
                        x: self.origin.x + px(left),
                        y: self.origin.y + px(top),
                    },
                    size: Size {
                        width: px(right - left),
                        height: px(bottom - top),
                    },
                    fill: color.clone().into(),
                    clip: self.clip.clone(),
                    corner_radii: Corners::default(),
                });
            }
        }
    }

    /// Fills a polygon in local logical pixels, implicitly closing the contour.
    /// The even-odd rule determines the interior, including at self-intersections.
    /// Gradients use the polygon's unclipped bounding box.
    pub fn fill_polygon(
        &mut self,
        points: impl Into<Rc<[Origin]>>,
        fill: impl Into<Fill>,
    ) -> Result<()> {
        let points = points.into();
        validate_points(&points)?;
        if points.len() >= 3 {
            self.commands.push(Draw::Polygon {
                origin: self.origin,
                clip: self.clip.clone(),
                points,
                fill: fill.into(),
            });
        }
        Ok(())
    }

    /// Strokes consecutive local points with a solid color and round caps and joins.
    /// Fewer than two points or a zero-width stroke produces no drawing.
    pub fn stroke_polyline(
        &mut self,
        points: impl Into<Rc<[Origin]>>,
        width: Pixels,
        color: Rgba,
    ) -> Result<()> {
        let points = points.into();
        anyhow::ensure!(width.is_extent(), "invalid stroke width");
        validate_points(&points)?;
        if points.len() >= 2 && width.get() > 0.0 && color.alpha != 0 {
            self.commands.push(Draw::Polyline {
                origin: self.origin,
                clip: self.clip.clone(),
                points,
                width,
                color,
            });
        }
        Ok(())
    }

    /// Fills a rectangle at an offset from the content origin in logical pixels.
    pub fn fill_rect(&mut self, offset: Origin, size: Size, fill: impl Into<Fill>) -> Result<()> {
        self.fill_rounded_rect(offset, size, px(0.0), fill)
    }

    /// Fills a content-local rectangle with a uniform corner radius in logical pixels.
    pub fn fill_rounded_rect(
        &mut self,
        offset: Origin,
        size: Size,
        radius: Pixels,
        fill: impl Into<Fill>,
    ) -> Result<()> {
        anyhow::ensure!(radius.is_extent(), "invalid corner radius");
        anyhow::ensure!(
            size.width.is_extent() && size.height.is_extent(),
            "invalid fill extent"
        );
        let origin = self
            .origin
            .checked_add(offset)
            .context("invalid fill origin")?;
        self.record_fill(
            origin,
            size,
            fill.into(),
            Corners::all(radius.min(size.width / 2.0).min(size.height / 2.0)),
        );
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
    corner_radii: Corners<Pixels>,
}

impl MountedBackground {
    fn resolve(style: &Style, units: UnitResolver) -> Result<Option<Box<Self>>> {
        let Some(source) = &style.background else {
            return Ok(None);
        };
        use crate::{bitmap::Placement, style::ObjectFit};
        let placement = match source {
            #[cfg(feature = "image")]
            Background::Fixed(Fill::ImageFile(_)) => {
                Placement::resolve(style, units, ObjectFit::Cover)?
            }
            Background::Fixed(Fill::Image(_) | Fill::CustomImage(_))
            | Background::Bound(_)
            | Background::Image(_)
            | Background::CustomImage(_) => Placement::resolve(style, units, ObjectFit::Cover)?,
            _ => Placement::centered(ObjectFit::Cover),
        };
        Ok(Some(Box::new(Self {
            source: source.clone(),
            corner_radii: style.corner_radii,
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
    background: Option<Box<MountedBackground>>,
    border: Option<Rc<crate::style::BorderPaint>>,
    outline: Option<Box<outline::MountedOutline>>,
    shadows: Option<Box<shadow::Shadows>>,
    pub(super) binding_target: Option<NodeId>,
    record: Option<Record>,
    pub(super) owned: crate::Owned,
    pub(crate) handlers: Option<Box<Handlers>>,
    pub(super) opacity: u8,
    pub(super) gradient_map: Option<crate::GradientMap>,
    pub(super) z_index: i32,
    pub(crate) cursor: Option<crate::style::Cursor>,
    pub(crate) hit_testable: bool,
    pub(crate) overflow_clip: OverflowClip,
    pub(crate) scroll_axes: crate::pointer::ScrollAxes,
}

impl MountedNode {
    pub(super) fn is_group(&self) -> bool {
        self.opacity < 255 || self.gradient_map.is_some()
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
                Some(Background::Fixed(_)) => FramePaint::FixedBackground,
                Some(_) => FramePaint::Recorded,
            }
        } else if fixed_label
            && style.background.is_none()
            && !style.text.attributes.as_deref().is_some_and(|a| {
                a.underline
                    .is_some_and(|s| s != crate::style::UnderlineStyle::None)
                    || a.strikethrough == Some(true)
                    || a.text_background.is_some_and(|color| color.alpha != 0)
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
        scene: &Scene,
        state: &Runtime<()>,
        reads: &RefCell<Vec<SignalId>>,
        frame_clips: Option<(&NodeClips, Origin)>,
        commands: &mut Vec<Draw>,
    ) -> Result<bool> {
        if scene.is_pending() {
            anyhow::bail!("layout pending");
        }
        reads.borrow_mut().clear();
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
                )?;
                (&local_clips, Origin::default())
            }
        };
        let mut cx = PaintContext {
            clip: clips.content.clone(),
            frame_offset,
            origin: Origin::default(),
            size,
            text_color: &self.text_color,
            commands,
            next_frame: false,
            state,
            reads,
        };
        let mode = match self.paint_mode {
            RefreshPolicy::Fixed(mode) => mode,
            RefreshPolicy::Bound(source) => *cx.read(source)?,
        };
        cx.next_frame = mode == RefreshMode::Continuous;
        if let Some(shadows) = &self.shadows {
            shadows.record(false, size, &clips.inherited, cx.commands);
        }
        if let Some(paint) = &self.background {
            let mut fill = match &paint.source {
                Background::Fixed(fill) => fill.clone(),
                Background::Bound(signal) => cx.read(*signal)?.clone(),
                Background::Color(signal) => Fill::from(*cx.read(*signal)?),
                Background::Image(signal) => Fill::from(cx.read(*signal)?.clone()),
                Background::CustomImage(signal) => Fill::from(cx.read(*signal)?.clone()),
            };
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
                corner_radii: paint.corner_radii.fit_to(size),
            });
        }
        if let Some(shadows) = &self.shadows {
            shadows.record(true, size, &clips.inherited, cx.commands);
        }
        if let Some(paint) = &self.border {
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
            let item = style_binding::item_style(style, mounted.scroll_axes.gutter)?;
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
            let item_sizing = body.bounds.item_sizing(style, units)?;
            if let Some(handlers) = node.handlers.as_mut() {
                handlers.padding = body.bounds.content_insets();
            }
            let id = match target {
                Target::Root(slot) => slot.insert(Scene::with_root_style(body, item)).root(),
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
    /// Includes this element in Tab navigation and allows focus by primary press or through `Ui::focus`.
    pub fn focusable(mut self) -> Self {
        self.focusable = true;
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
    /// The timestamp is the monotonic dispatch time. Signal writes apply before the next layout.
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
            let (initial, source) = match self.content {
                TextContent::Fixed(value) => (value, None),
                TextContent::Bound(signal) => (cx.read(signal)?.clone(), Some(signal)),
            };
            let mut node = element(Text::new(
                Rc::clone(cx.engine.engine()),
                initial,
                TextStyle::default(),
                true,
            )?);
            node.style = self.style;
            if let Some(signal) = source {
                node.watches
                    .push(content_watch(signal, crate::ContentUpdate::Text(signal)));
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
