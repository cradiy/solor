//! A retained scrollbar for scrollable viewports.
use crate::{
    Context, Div, Fill, IntoElement, ListScrollHandle, PaintContext, Pixels, Render, ScrollAxis,
    ScrollHandle, ScrollPosition, StyleRefinement, Styled, div,
    layout::{Origin, Size},
    pointer::{PointerEvent, ScrollDelta},
    px, rgb, rgba,
};
use std::rc::Rc;

/// Appearance and minimum target length of the draggable thumb.
#[derive(Clone, Debug, PartialEq)]
pub struct ScrollbarThumbStyle {
    /// Fill while the pointer is outside the track.
    pub fill: Fill,
    /// Fill while the pointer hovers over the track.
    pub hover_fill: Fill,
    /// Fill while a pointer owns the drag.
    pub drag_fill: Fill,
    /// Thickness of the resting thumb, aligned to the right or bottom edge of the track.
    pub idle_width: Pixels,
    /// Nonnegative corner radius, limited to half the rendered dimensions.
    pub radius: Pixels,
    /// Minimum thumb length, limited to half the track on short tracks.
    pub min_length: Pixels,
}

impl Default for ScrollbarThumbStyle {
    fn default() -> Self {
        Self {
            fill: rgb(0x999999).into(),
            hover_fill: rgb(0x808080).into(),
            drag_fill: rgb(0x666666).into(),
            idle_width: px(3.0),
            radius: px(4.0),
            min_length: px(24.0),
        }
    }
}

/// Scroll position, pointer state and content-local geometry for custom scrollbar drawing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollbarPaintState {
    /// Direction of travel along the track.
    pub axis: ScrollAxis,
    /// Position and viewport extent along the track.
    pub position: ScrollPosition,
    /// Start of the draggable interval, relative to the track's content origin.
    pub thumb_start: Pixels,
    /// Length of the draggable interval in logical pixels.
    pub thumb_length: Pixels,
    /// Whether the pointer is over the track.
    pub hovered: bool,
    /// Whether a pointer currently owns the thumb, including outside the track.
    pub dragging: bool,
}

type Painter = Box<dyn Fn(&mut PaintContext<'_>, ScrollbarPaintState) -> anyhow::Result<()>>;

#[derive(Clone)]
enum ScrollSource {
    List(ListScrollHandle),
    Container(ScrollHandle, ScrollAxis),
}

impl ScrollSource {
    fn axis(&self) -> ScrollAxis {
        match self {
            Self::List(_) => ScrollAxis::Vertical,
            Self::Container(_, axis) => *axis,
        }
    }

    fn position(&self) -> Option<ScrollPosition> {
        match self {
            Self::List(handle) => handle.position().map(Into::into),
            Self::Container(handle, axis) => handle.position(*axis),
        }
    }

    fn observe(&self, cx: &mut Context<'_>) -> Rc<crate::scroll_position::PositionSignal> {
        match self {
            Self::List(handle) => handle.observe(cx),
            Self::Container(handle, axis) => handle.observe(*axis, cx),
        }
    }

    fn scroll_to_percent(&self, percent: f64) -> anyhow::Result<()> {
        match self {
            Self::List(handle) => handle.scroll_to_percent(percent),
            Self::Container(handle, axis) => handle.scroll_to_percent_on(*axis, percent),
        }
    }

    fn scroll_by(&self, delta: Pixels, cx: &mut Context<'_>) -> anyhow::Result<f64> {
        match self {
            Self::List(handle) => handle.scroll_by(delta.into(), cx),
            Self::Container(handle, axis) => {
                let mut movement = ScrollDelta::default();
                *axis.select(&mut movement.x, &mut movement.y) = delta;
                let used = handle.scroll_by(movement)?;
                Ok(axis.select(used.x, used.y).into())
            }
        }
    }
}

/// A scrollbar. Use the shared style builders for its track.
pub struct Scrollbar {
    source: ScrollSource,
    track: Div,
    thumb: ScrollbarThumbStyle,
    auto_hide: bool,
    hover_track: Fill,
    painter: Option<Painter>,
}

/// Creates a vertical scrollbar for a virtual list or an overflow container in the same UI.
/// Place it beside the viewport in a stretched flex row.
pub fn scrollbar(handle: impl Into<Scrollbar>) -> Scrollbar {
    handle.into()
}

/// Creates a horizontal scrollbar for a container tracked by this handle.
/// Place it below the viewport in a stretched flex column.
pub fn horizontal_scrollbar(handle: &ScrollHandle) -> Scrollbar {
    Scrollbar::new(ScrollSource::Container(
        handle.clone(),
        ScrollAxis::Horizontal,
    ))
}

impl From<&ListScrollHandle> for Scrollbar {
    fn from(handle: &ListScrollHandle) -> Self {
        Self::new(ScrollSource::List(handle.clone()))
    }
}

impl From<&ScrollHandle> for Scrollbar {
    fn from(handle: &ScrollHandle) -> Self {
        Self::new(ScrollSource::Container(
            handle.clone(),
            ScrollAxis::Vertical,
        ))
    }
}

impl Scrollbar {
    fn new(source: ScrollSource) -> Self {
        let track = div().p(px(2.0)).flex_shrink(0.0);
        let track = match source.axis() {
            ScrollAxis::Vertical => track.w(px(12.0)),
            ScrollAxis::Horizontal => track.h(px(12.0)),
        };
        Self {
            source,
            track,
            thumb: ScrollbarThumbStyle::default(),
            auto_hide: false,
            painter: None,
            hover_track: rgba(0x80808026).into(),
        }
    }

    /// Hides the resting thumb until the pointer enters the track or starts a drag.
    /// Defaults to false. The pointer target keeps its full size in either mode.
    pub fn auto_hide(mut self, enabled: bool) -> Self {
        self.auto_hide = enabled;
        self
    }

    /// Sets the rail fill shown during hover and drag. Use a transparent fill to omit it.
    pub fn hover_track_fill(mut self, fill: impl Into<Fill>) -> Self {
        self.hover_track = fill.into();
        self
    }

    /// Draws the rail and thumb using the supplied painter. Read signals from the paint
    /// context to react to application state. An empty drawing leaves input active.
    /// The painter runs for scrollable content while the auto-hide policy permits it.
    pub fn paint_with(
        mut self,
        paint: impl Fn(&mut PaintContext<'_>, ScrollbarPaintState) -> anyhow::Result<()> + 'static,
    ) -> Self {
        self.painter = Some(Box::new(paint));
        self
    }

    /// Sets the thumb's fills, resting width, corner radius and minimum length.
    pub fn thumb_style(mut self, style: ScrollbarThumbStyle) -> Self {
        self.thumb = style;
        self
    }
}

impl Styled for Scrollbar {
    fn style(&mut self) -> &mut StyleRefinement {
        self.track.style()
    }
}

struct Thumb {
    start: f64,
    length: f64,
    travel: f64,
}

impl Thumb {
    fn resolve(position: ScrollPosition, extent: Pixels, minimum: Pixels) -> Option<Self> {
        let track = f64::from(extent);
        if track <= 0.0 || position.max_offset_px <= 0.0 {
            return None;
        }
        let viewport = f64::from(position.viewport_extent);
        let length = (track * viewport / (viewport + position.max_offset_px))
            .max(f64::from(minimum).min(track / 2.0));
        let travel = track - length;
        Some(Self {
            start: travel * (position.offset_px / position.max_offset_px),
            length,
            travel,
        })
    }
}

impl Render for Scrollbar {
    fn render(self, cx: &mut Context<'_>) -> impl IntoElement {
        let axis = self.source.axis();
        let position = self.source.observe(cx);
        let hover = cx.signal(false);
        let drag = cx.signal(None);
        let minimum = self.thumb.min_length;
        let press = self.source.clone();
        let wheel = self.source.clone();
        let seek = move |event: &PointerEvent, fraction: f64| -> anyhow::Result<()> {
            if let Some(thumb) = self
                .source
                .position()
                .and_then(|p| Thumb::resolve(p, axis.extent(event.content_size()), minimum))
                && thumb.travel > 0.0
            {
                let start =
                    f64::from(axis.coordinate(event.content_position())) - fraction * thumb.length;
                self.source
                    .scroll_to_percent(100.0 * start / thumb.travel)?;
            }
            Ok(())
        };
        let release = seek.clone();
        self.track
            .paint(move |cx| {
                anyhow::ensure!(
                    minimum.is_extent()
                        && self.thumb.radius.is_extent()
                        && self.thumb.idle_width.is_extent(),
                    "invalid scrollbar thumb dimensions"
                );
                let dragging = cx.read(drag)?.is_some();
                let hovered = *cx.read(hover)?;
                let active = dragging || hovered;
                if self.auto_hide && !active {
                    return Ok(());
                }
                let Some(position) = *cx.read(*position)? else {
                    return Ok(());
                };
                let Some(thumb) = Thumb::resolve(position, axis.extent(cx.size), minimum) else {
                    return Ok(());
                };
                let thumb_start = px(thumb.start as f32);
                let thumb_length = px(thumb.length as f32);
                if let Some(paint) = &self.painter {
                    return paint(
                        cx,
                        ScrollbarPaintState {
                            axis,
                            position,
                            thumb_start,
                            thumb_length,
                            hovered,
                            dragging,
                        },
                    );
                }
                let cross_extent = axis.select(cx.size.height, cx.size.width);
                let thickness = if active {
                    cx.fill_rounded_rect(
                        Origin::default(),
                        cx.size,
                        self.thumb.radius,
                        self.hover_track.clone(),
                    )?;
                    cross_extent
                } else {
                    self.thumb.idle_width.min(cross_extent)
                };
                let fill = if dragging {
                    &self.thumb.drag_fill
                } else if active {
                    &self.thumb.hover_fill
                } else {
                    &self.thumb.fill
                };
                cx.fill_rounded_rect(
                    Origin {
                        x: axis.select(thumb_start, cross_extent - thickness),
                        y: axis.select(cross_extent - thickness, thumb_start),
                    },
                    Size {
                        width: axis.select(thumb_length, thickness),
                        height: axis.select(thickness, thumb_length),
                    },
                    self.thumb.radius,
                    fill.clone(),
                )
            })
            .on_hover(move |inside, cx| cx.set(hover, inside))
            .on_pointer_down(move |event, cx| {
                if !event.is_primary_action() || cx.read(drag)?.is_some() {
                    return Ok(());
                }
                let Some(pos) = press.position() else {
                    return Ok(());
                };
                let Some(thumb) = Thumb::resolve(pos, axis.extent(event.content_size()), minimum)
                else {
                    return Ok(());
                };
                let along = f64::from(axis.coordinate(event.content_position()));
                if (thumb.start..=thumb.start + thumb.length).contains(&along) {
                    cx.capture_pointer();
                    cx.set(
                        drag,
                        Some((event.pointer_id(), (along - thumb.start) / thumb.length)),
                    )?;
                } else {
                    let page = pos.viewport_extent;
                    press.scroll_by(
                        if along < thumb.start {
                            page * -1.0
                        } else {
                            page
                        },
                        cx,
                    )?;
                }
                Ok(())
            })
            .on_pointer_move(move |event, cx| {
                if let Some((id, fraction)) = *cx.read(drag)?
                    && id == event.pointer_id()
                {
                    seek(event, fraction)?;
                }
                Ok(())
            })
            .on_pointer_up(move |event, cx| {
                if event.is_primary_action()
                    && let Some((id, fraction)) = *cx.read(drag)?
                    && id == event.pointer_id()
                {
                    release(event, fraction)?;
                    cx.set(drag, None)?;
                }
                Ok(())
            })
            .on_pointer_cancel(move |id, cx| {
                if cx.read(drag)?.is_some_and(|(owner, _)| owner == id) {
                    cx.set(drag, None)?;
                }
                Ok(())
            })
            .on_scroll(move |event, cx| {
                let delta = axis.select(event.delta.x, event.delta.y);
                let used = wheel.scroll_by(delta, cx)?;
                let mut remaining = event.delta;
                *axis.select(&mut remaining.x, &mut remaining.y) =
                    px((f64::from(delta) - used) as f32);
                cx.propagate_remaining(remaining)
            })
    }
}
