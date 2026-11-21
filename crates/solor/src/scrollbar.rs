//! A retained vertical scrollbar for scrollable viewports.
use crate::{
    Context, Div, Fill, IntoElement, ListScrollHandle, ListScrollPosition, PaintContext, Pixels,
    Render, ScrollHandle, StyleRefinement, Styled, div,
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
    /// Width of the resting thumb, aligned to the right edge of the track content.
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
    /// Vertical scroll position and viewport extent of the bound content.
    pub position: ListScrollPosition,
    /// Top of the draggable interval, relative to the track's content origin.
    pub thumb_top: Pixels,
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
    Container(ScrollHandle),
}

impl ScrollSource {
    fn position(&self) -> Option<ListScrollPosition> {
        match self {
            Self::List(handle) => handle.position(),
            Self::Container(handle) => handle.vertical_position(),
        }
    }

    fn observe(&self, cx: &mut Context<'_>) -> Rc<crate::scroll_position::PositionSignal> {
        match self {
            Self::List(handle) => handle.observe(cx),
            Self::Container(handle) => handle.observe(cx),
        }
    }

    fn scroll_to_percent(&self, percent: f64) -> anyhow::Result<()> {
        match self {
            Self::List(handle) => handle.scroll_to_percent(percent),
            Self::Container(handle) => {
                handle.seek_percent(percent);
                Ok(())
            }
        }
    }

    fn scroll_by(&self, delta: f64, cx: &mut Context<'_>) -> anyhow::Result<f64> {
        match self {
            Self::List(handle) => handle.scroll_by(delta, cx),
            Self::Container(handle) => Ok(handle.scroll_by(delta)),
        }
    }
}

/// A vertical scrollbar. Use the shared style builders for its track.
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

impl From<&ListScrollHandle> for Scrollbar {
    fn from(handle: &ListScrollHandle) -> Self {
        Self::new(ScrollSource::List(handle.clone()))
    }
}

impl From<&ScrollHandle> for Scrollbar {
    fn from(handle: &ScrollHandle) -> Self {
        Self::new(ScrollSource::Container(handle.clone()))
    }
}

impl Scrollbar {
    fn new(source: ScrollSource) -> Self {
        Self {
            source,
            track: div().w(px(12.0)).p(px(2.0)).flex_shrink(0.0),
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
    fn resolve(position: ListScrollPosition, height: Pixels, minimum: Pixels) -> Option<Self> {
        let track = f64::from(height);
        if track <= 0.0 || position.max_offset_px <= 0.0 {
            return None;
        }
        let viewport = f64::from(position.viewport_height);
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
                .and_then(|p| Thumb::resolve(p, event.content_size().height, minimum))
                && thumb.travel > 0.0
            {
                let top = f64::from(event.content_position().y) - fraction * thumb.length;
                self.source.scroll_to_percent(100.0 * top / thumb.travel)?;
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
                let Some(thumb) = Thumb::resolve(position, cx.size.height, minimum) else {
                    return Ok(());
                };
                let thumb_top = px(thumb.start as f32);
                let thumb_length = px(thumb.length as f32);
                if let Some(paint) = &self.painter {
                    return paint(
                        cx,
                        ScrollbarPaintState {
                            position,
                            thumb_top,
                            thumb_length,
                            hovered,
                            dragging,
                        },
                    );
                }
                let width = if active {
                    cx.fill_rounded_rect(
                        Origin::default(),
                        cx.size,
                        self.thumb.radius,
                        self.hover_track.clone(),
                    )?;
                    cx.size.width
                } else {
                    self.thumb.idle_width.min(cx.size.width)
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
                        x: cx.size.width - width,
                        y: thumb_top,
                    },
                    Size {
                        width,
                        height: thumb_length,
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
                let Some(thumb) = Thumb::resolve(pos, event.content_size().height, minimum) else {
                    return Ok(());
                };
                let y = f64::from(event.content_position().y);
                if (thumb.start..=thumb.start + thumb.length).contains(&y) {
                    cx.capture_pointer();
                    cx.set(
                        drag,
                        Some((event.pointer_id(), (y - thumb.start) / thumb.length)),
                    )?;
                } else {
                    let page = f64::from(pos.viewport_height);
                    press.scroll_by(if y < thumb.start { -page } else { page }, cx)?;
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
                let delta = f64::from(event.delta.y);
                let used = wheel.scroll_by(delta, cx)?;
                cx.propagate_remaining(ScrollDelta {
                    x: event.delta.x,
                    y: px((delta - used) as f32),
                })
            })
    }
}
