//! Mounted container scroll policies and logical scroll distances.
use crate::{
    Context, Instance, NodeId, Pixels, Result, ScrollAxis, ScrollPosition, Signal, Ui, ViewId,
    layout::{Geometry, Origin, Size},
    length::UnitResolver,
    px,
    scroll_position::{PositionObservers, PositionSignal, scroll_percent},
    state::Runtime,
    style::{LayoutProperties, Overflow},
};

use super::ScrollDelta;
use anyhow::{Context as _, ensure};
use std::{cell::Cell, rc::Rc};

/// Scroll distances and clipping viewport of a laid-out overflow container in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollMetrics {
    /// Current distance from the top-left corner, including input since the last presentation.
    pub offset: Origin,
    /// Maximum scrollable distance. An axis with scrolling disabled has a zero limit.
    pub max_offset: Size,
    /// Size of the container's clipping box.
    pub viewport: Size,
}

impl ScrollMetrics {
    /// Horizontal travel as a percentage from 0 to 100. Returns zero when the content fits.
    pub fn horizontal_percent(self) -> f64 {
        scroll_percent(self.offset.x.into(), self.max_offset.width.into())
    }

    /// Vertical travel as a percentage from 0 to 100. Returns zero when the content fits.
    pub fn vertical_percent(self) -> f64 {
        scroll_percent(self.offset.y.into(), self.max_offset.height.into())
    }
}

#[derive(Clone, Copy, Default)]
enum AxisTarget {
    #[default]
    Current,
    Pixels(Pixels),
    Percent(f64),
    Reveal {
        start: Pixels,
        extent: Pixels,
    },
}

impl AxisTarget {
    fn resolve(self, current: Pixels, limit: Pixels, viewport: Pixels) -> Pixels {
        let target = match self {
            Self::Current => f64::from(current),
            Self::Pixels(value) => f64::from(value),
            Self::Percent(percent) => f64::from(limit) * (percent / 100.0),
            Self::Reveal { start, extent } => {
                // Align either edge, keeping the current offset whenever it lies between them.
                let leading = f64::from(start);
                let trailing = leading + f64::from(extent) - f64::from(viewport);
                f64::from(current).clamp(leading.min(trailing), leading.max(trailing))
            }
        };
        px(target.clamp(0.0, f64::from(limit)) as f32)
    }
}

#[derive(Clone, Copy, Default)]
struct ScrollRequest {
    x: AxisTarget,
    y: AxisTarget,
}

impl ScrollRequest {
    fn offset(offset: Origin) -> Result<Self> {
        ensure!(
            offset.x.get().is_finite() && offset.y.get().is_finite(),
            "scroll offset must be finite"
        );
        Ok(Self {
            x: AxisTarget::Pixels(offset.x),
            y: AxisTarget::Pixels(offset.y),
        })
    }

    fn resolve(self, metrics: ScrollMetrics) -> Origin {
        Origin {
            x: self.x.resolve(
                metrics.offset.x,
                metrics.max_offset.width,
                metrics.viewport.width,
            ),
            y: self.y.resolve(
                metrics.offset.y,
                metrics.max_offset.height,
                metrics.viewport.height,
            ),
        }
    }
}

#[derive(Default)]
struct ContainerState {
    metrics: Cell<Option<ScrollMetrics>>,
    metrics_signal: Option<Signal<Option<ScrollMetrics>>>,
    pending: Cell<Option<ScrollRequest>>,
    attached: Cell<bool>,
    horizontal: PositionObservers,
    vertical: PositionObservers,
}

/// Reads and positions one scrollable overflow container.
/// Clones address the same container and can be reused after unmount.
#[derive(Clone, Default)]
pub struct ScrollHandle(Rc<ContainerState>);

impl ScrollHandle {
    /// Creates an unattached handle.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a handle and a context-owned signal that mirrors [Self::metrics].
    /// Offset, scroll range and viewport changes update subscribers before frame recording.
    /// The signal contains `None` until scrollable layout and after the container unmounts.
    pub fn with_metrics_signal(cx: &mut Context<'_>) -> (Self, Signal<Option<ScrollMetrics>>) {
        let signal = cx.signal(None);
        let handle = Self(Rc::new(ContainerState {
            metrics_signal: Some(signal),
            ..ContainerState::default()
        }));
        (handle, signal)
    }

    /// Returns the live container's last published metrics, or none until layout and after unmount.
    pub fn metrics(&self) -> Option<ScrollMetrics> {
        self.0.metrics.get()
    }

    /// Requests a logical-pixel offset from the top left on both axes.
    /// Layout clamps the destination to the available range. Intervening wheel input
    /// continues from the requested position using the current scroll range.
    /// Requests may be made before mount. Unmounting discards any outstanding request.
    pub fn scroll_to_offset(&self, offset: Origin) -> Result<()> {
        self.0.pending.set(Some(ScrollRequest::offset(offset)?));
        Ok(())
    }

    /// Scrolls each axis just far enough to reveal a rectangle in unscrolled content coordinates.
    /// An oversized target moves only far enough to cover the viewport on that axis.
    /// The next scrollable layout resolves the request and clamps it to the available travel.
    /// Origins must be finite and extents must be finite and nonnegative.
    pub fn scroll_to_reveal(&self, origin: Origin, size: Size) -> Result<()> {
        ensure!(
            origin.x.get().is_finite() && origin.y.get().is_finite(),
            "reveal origin must be finite"
        );
        ensure!(
            size.width.is_extent() && size.height.is_extent(),
            "invalid reveal extent"
        );
        self.0.pending.set(Some(ScrollRequest {
            x: AxisTarget::Reveal {
                start: origin.x,
                extent: size.width,
            },
            y: AxisTarget::Reveal {
                start: origin.y,
                extent: size.height,
            },
        }));
        Ok(())
    }

    /// Requests a vertical percentage of scrollable travel, preserving the horizontal destination.
    /// Layout resolves the percentage against its content and viewport. Intervening wheel
    /// input continues from the requested percentage of the current range.
    /// Finite values are clamped to 0..=100.
    pub fn scroll_to_percent(&self, percent: f64) -> Result<()> {
        self.scroll_to_percent_on(ScrollAxis::Vertical, percent)
    }

    /// Requests a percentage of travel on one axis and keeps the other axis's destination.
    /// Finite values are clamped to 0..=100 and resolved against the scrollable range.
    pub fn scroll_to_percent_on(&self, axis: ScrollAxis, percent: f64) -> Result<()> {
        ensure!(percent.is_finite(), "scroll percentage must be finite");
        self.request_axis(axis, AxisTarget::Percent(percent.clamp(0.0, 100.0)));
        Ok(())
    }

    fn request_axis(&self, axis: ScrollAxis, target: AxisTarget) {
        let mut request = self.0.pending.get().unwrap_or_default();
        *axis.select(&mut request.x, &mut request.y) = target;
        self.0.pending.set(Some(request));
    }

    pub(crate) fn position(&self, axis: ScrollAxis) -> Option<ScrollPosition> {
        let metrics = self.metrics()?;
        (metrics.viewport.width.get() > 0.0 && metrics.viewport.height.get() > 0.0).then_some(
            ScrollPosition {
                offset_px: axis.coordinate(metrics.offset).into(),
                max_offset_px: axis.extent(metrics.max_offset).into(),
                viewport_extent: axis.extent(metrics.viewport),
                is_estimated: false,
            },
        )
    }

    fn observers(&self, axis: ScrollAxis) -> &PositionObservers {
        axis.select(&self.0.horizontal, &self.0.vertical)
    }

    pub(crate) fn observe(&self, axis: ScrollAxis, cx: &mut Context<'_>) -> Rc<PositionSignal> {
        self.observers(axis).observe(cx, self.position(axis))
    }

    /// Scrolls relative to the current or queued destination in logical pixels.
    /// Returns the portion of each component within the last published scroll bounds.
    /// Before a nonempty viewport is available, both consumed components are zero.
    pub fn scroll_by(&self, delta: ScrollDelta) -> Result<ScrollDelta> {
        ensure!(
            delta.x.get().is_finite() && delta.y.get().is_finite(),
            "scroll delta must be finite"
        );
        let Some(metrics) = self.metrics() else {
            return Ok(ScrollDelta::default());
        };
        if metrics.viewport.width == px(0.0) || metrics.viewport.height == px(0.0) {
            return Ok(ScrollDelta::default());
        }
        let mut request = self.0.pending.get().unwrap_or_default();
        let start = request.resolve(metrics);
        let (next, consumed) = advance(start, metrics.max_offset, delta);
        if next != start {
            if next.x != start.x {
                request.x = AxisTarget::Pixels(next.x);
            }
            if next.y != start.y {
                request.y = AxisTarget::Pixels(next.y);
            }
            self.0.pending.set(Some(request));
        }
        Ok(consumed)
    }

    fn publish(&self, metrics: Option<ScrollMetrics>, state: &mut Runtime<()>) {
        if self.0.metrics.replace(metrics) != metrics {
            if let Some(signal) = self.0.metrics_signal {
                // The scope owning the signal can unmount before the container.
                let _ = state.publish_layout_value(signal, metrics);
            }
            for axis in [ScrollAxis::Horizontal, ScrollAxis::Vertical] {
                self.observers(axis).publish(state, self.position(axis));
            }
        }
    }
}

pub(crate) struct TrackedScroll {
    handle: ScrollHandle,
}

impl TrackedScroll {
    pub(crate) fn attach(handle: ScrollHandle) -> Result<Self> {
        ensure!(
            !handle.0.attached.replace(true),
            "scroll handle already attached"
        );
        Ok(Self { handle })
    }

    pub(crate) fn has_request(&self) -> bool {
        self.handle.metrics().is_some() && self.handle.0.pending.get().is_some()
    }

    pub(crate) fn detach(&self, state: &mut Runtime<()>) {
        self.handle.publish(None, state);
    }
}

impl Drop for TrackedScroll {
    fn drop(&mut self) {
        self.handle.0.pending.set(None);
        self.handle.0.metrics.set(None);
        self.handle.0.attached.set(false);
    }
}

impl Ui {
    /// Reads a visible container with scroll or auto overflow after layout.
    /// Returns none for an unknown node or view, a pending layout or a non-scrolling element.
    pub fn scroll_metrics(&self, view: ViewId, node: NodeId) -> Option<ScrollMetrics> {
        self.instances.get(&view)?.scroll_metrics(node)
    }

    /// Moves a laid-out overflow container to an absolute offset in logical pixels.
    /// Clamps each axis and returns whether a redraw was requested. Input continues to
    /// hit the previously presented geometry until the host accepts the new frame.
    pub fn set_scroll_offset(
        &mut self,
        view: ViewId,
        node: NodeId,
        offset: Origin,
    ) -> Result<bool> {
        let request = ScrollRequest::offset(offset)?;
        let instance = self
            .instances
            .get_mut(&view)
            .ok_or(crate::tree::TreeError::InvalidNode(view.0))?;
        let metrics = instance
            .scroll_metrics(node)
            .context("expected a visible, laid-out scroll container")?;
        let next = request.resolve(metrics);
        let moved = instance.apply_scroll_offset(node, metrics.offset, next)?;
        if let Some(tracked) = instance.tracked_scroll.get(&node) {
            tracked.handle.0.pending.set(None);
            tracked.handle.publish(
                Some(ScrollMetrics {
                    offset: next,
                    ..metrics
                }),
                self.state.get_mut(),
            );
        }
        Ok(moved)
    }
}

#[derive(Clone, Copy, Default, PartialEq)]
pub(super) enum WheelPolicy {
    #[default]
    BothAxes,
    VerticalAsHorizontal,
    DominantAxis,
}

impl WheelPolicy {
    fn advance(
        self,
        start: Origin,
        limit: Size,
        delta: ScrollDelta,
        vertical_input: bool,
    ) -> (Origin, ScrollDelta) {
        let mut movement = delta;
        let swap = self == Self::VerticalAsHorizontal && vertical_input;
        if swap {
            std::mem::swap(&mut movement.x, &mut movement.y);
        } else if self == Self::DominantAxis && limit.width > px(0.) && limit.height > px(0.) {
            if delta.x.get().abs() > delta.y.get().abs() {
                movement.y = px(0.);
            } else {
                movement.x = px(0.);
            }
        }
        let (next, consumed) = advance(start, limit, movement);
        if consumed == ScrollDelta::default() {
            return (next, delta);
        }
        let mut remaining = ScrollDelta {
            x: movement.x - consumed.x,
            y: movement.y - consumed.y,
        };
        if swap {
            std::mem::swap(&mut remaining.x, &mut remaining.y);
        }
        (next, remaining)
    }
}

#[derive(Clone, Copy, Default, PartialEq)]
pub(crate) struct ScrollAxes {
    x: bool,
    y: bool,
    pub(super) wheel: WheelPolicy,
    pub(crate) gutter: Size,
}

impl ScrollAxes {
    pub(crate) fn from_layout(
        layout: Option<&LayoutProperties>,
        units: UnitResolver,
    ) -> Result<Self> {
        let scrolls = |axis| matches!(axis, Some(Overflow::Scroll | Overflow::Auto));
        let width = layout
            .and_then(|v| v.scrollbar_width)
            .map_or(px(0.), |v| units.absolute(v));
        if !width.is_extent() {
            return Err(crate::layout::LayoutError::InvalidSize.into());
        }
        let x = layout.and_then(|v| v.overflow_x);
        let y = layout.and_then(|v| v.overflow_y);
        Ok(Self {
            x: scrolls(x),
            y: scrolls(y),
            wheel: match (scrolls(x), scrolls(y)) {
                (true, false)
                    if layout.and_then(|v| v.vertical_wheel_as_horizontal) == Some(true) =>
                {
                    WheelPolicy::VerticalAsHorizontal
                }
                (true, true) if layout.and_then(|v| v.dominant_scroll_axis) == Some(true) => {
                    WheelPolicy::DominantAxis
                }
                _ => WheelPolicy::BothAxes,
            },
            gutter: Size {
                width: if y == Some(Overflow::Scroll) {
                    width
                } else {
                    px(0.)
                },
                height: if x == Some(Overflow::Scroll) {
                    width
                } else {
                    px(0.)
                },
            },
        })
    }

    pub(crate) fn viewport(self, size: Size) -> Size {
        Size {
            width: size.width.saturating_sub(self.gutter.width),
            height: size.height.saturating_sub(self.gutter.height),
        }
    }

    pub(crate) fn limit(self, geometry: Geometry) -> Size {
        let size = self.viewport(geometry.metrics.size);
        Size {
            width: if self.x {
                geometry.content.width.saturating_sub(size.width)
            } else {
                Pixels::default()
            },
            height: if self.y {
                geometry.content.height.saturating_sub(size.height)
            } else {
                Pixels::default()
            },
        }
    }

    /// Clamp a stored offset to the live layout and report whether it still needs storage.
    pub(crate) fn clamp_offset(self, offset: &mut Origin, geometry: Geometry) -> bool {
        let limit = self.limit(geometry);
        offset.x = offset.x.min(limit.width);
        offset.y = offset.y.min(limit.height);
        *offset != Origin::default()
    }
}

/// Returns the bounded destination and the portion of the input consumed on each axis.
fn advance(offset: Origin, limit: Size, delta: ScrollDelta) -> (Origin, ScrollDelta) {
    fn axis(offset: Pixels, limit: Pixels, delta: Pixels) -> (Pixels, Pixels) {
        let offset = offset.min(limit);
        let used = delta.max(Pixels::default() - offset).min(limit - offset);
        ((offset + used).min(limit), used)
    }
    let (x, used_x) = axis(offset.x, limit.width, delta.x);
    let (y, used_y) = axis(offset.y, limit.height, delta.y);
    (
        Origin { x, y },
        ScrollDelta {
            x: used_x,
            y: used_y,
        },
    )
}

impl Instance {
    fn scroll_metrics(&self, node: NodeId) -> Option<ScrollMetrics> {
        let body = self.bodies.get(&node)?;
        if self.scene.is_pending() || !body.visible || body.scroll_axes == ScrollAxes::default() {
            return None;
        }
        let geometry = self.scene.geometry(node)?;
        Some(ScrollMetrics {
            offset: self
                .pointer
                .scroll_offsets
                .get(&node)
                .copied()
                .unwrap_or_default(),
            max_offset: body.scroll_axes.limit(geometry),
            viewport: body.scroll_axes.viewport(geometry.metrics.size),
        })
    }

    pub(crate) fn update_tracked_scroll(&mut self, state: &mut Runtime<()>) -> Result<()> {
        // Moving an offset mutates the instance's hit geometry and draw caches.
        let handles = std::mem::take(&mut self.tracked_scroll);
        let result = handles
            .iter()
            .try_for_each(|(&node, tracked)| -> Result<()> {
                let mut metrics = self.scroll_metrics(node);
                if let Some(m) = &mut metrics
                    && let Some(request) = tracked.handle.0.pending.take()
                {
                    let next = request.resolve(*m);
                    self.apply_scroll_offset(node, m.offset, next)?;
                    m.offset = next;
                }
                tracked.handle.publish(metrics, state);
                Ok(())
            });
        self.tracked_scroll = handles;
        result
    }

    pub(super) fn scroll_children(
        &mut self,
        owner: NodeId,
        viewport: super::frame::ScrollViewport,
        delta: ScrollDelta,
        vertical_input: bool,
        state: &mut Runtime<()>,
    ) -> Result<ScrollDelta> {
        let geometry = self
            .scene
            .geometry(owner)
            .expect("laid-out scroll container");
        let axes = self.bodies[&owner].scroll_axes;
        let live = axes.limit(geometry);
        let limit = Size {
            width: live.width.min(viewport.limit.width),
            height: live.height.min(viewport.limit.height),
        };
        let previous = self
            .pointer
            .scroll_offsets
            .get(&owner)
            .copied()
            .unwrap_or_default();
        let start = self
            .tracked_scroll
            .get(&owner)
            .and_then(|tracked| tracked.handle.0.pending.take())
            .map_or(previous, |request| {
                request.resolve(ScrollMetrics {
                    offset: previous,
                    max_offset: limit,
                    viewport: axes.viewport(geometry.metrics.size),
                })
            });
        let (next, remaining) = viewport.wheel.advance(start, limit, delta, vertical_input);
        self.apply_scroll_offset(owner, previous, next)?;
        if let Some(tracked) = self.tracked_scroll.get(&owner) {
            tracked.handle.publish(self.scroll_metrics(owner), state);
        }
        Ok(remaining)
    }

    fn apply_scroll_offset(
        &mut self,
        owner: NodeId,
        previous: Origin,
        next: Origin,
    ) -> Result<bool> {
        if next == previous {
            return Ok(false);
        }
        let offsets = &mut self.pointer.scroll_offsets;
        if next == Origin::default() {
            offsets.remove(&owner);
        } else {
            offsets.insert(owner, next);
        }
        self.pointer.scroll_pending = true;
        self.pointer.invalidate_geometry();
        self.fixed_draws = Default::default();
        // Retained draw commands include absolute origins and inherited clips.
        let mut paint = self.repaints.borrow_mut();
        let mut pending: Vec<_> = self.scene.children(owner)?.collect();
        while let Some(id) = pending.pop() {
            paint.commands.remove(&id);
            pending.extend(self.scene.children(id)?);
        }
        Ok(true)
    }
}
