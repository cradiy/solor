//! Mounted container scroll policies and logical scroll distances.
use crate::{
    Context, Instance, ListScrollPosition, NodeId, Pixels, Result, Ui, ViewId,
    layout::{Geometry, Origin, Size},
    px,
    scroll_position::{PositionObservers, PositionSignal},
    state::Runtime,
    style::{LayoutProperties, Overflow},
};

use super::ScrollDelta;
use anyhow::{Context as _, ensure};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

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

#[derive(Default)]
struct ContainerState {
    metrics: Cell<Option<ScrollMetrics>>,
    pending: RefCell<Weak<Cell<Option<Pixels>>>>,
    observers: PositionObservers,
}

/// Connects one mounted overflow container to a vertical scrollbar.
/// Clones address the same container and can be reused after unmount.
#[derive(Clone, Default)]
pub struct ScrollHandle(Rc<ContainerState>);

impl ScrollHandle {
    /// Creates an unattached handle.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the live container's last published metrics, or none until layout and after unmount.
    pub fn metrics(&self) -> Option<ScrollMetrics> {
        self.0.pending.borrow().upgrade()?;
        self.0.metrics.get()
    }

    pub(crate) fn vertical_position(&self) -> Option<ListScrollPosition> {
        let metrics = self.metrics()?;
        (metrics.viewport.width.get() > 0.0 && metrics.viewport.height.get() > 0.0).then_some(
            ListScrollPosition {
                offset_px: f64::from(metrics.offset.y),
                max_offset_px: f64::from(metrics.max_offset.height),
                viewport_height: metrics.viewport.height,
                is_estimated: false,
            },
        )
    }

    pub(crate) fn observe(&self, cx: &mut Context<'_>) -> Rc<PositionSignal> {
        self.0.observers.observe(cx, self.vertical_position())
    }

    pub(crate) fn seek_percent(&self, percent: f64) {
        if let Some(pending) = self.0.pending.borrow().upgrade()
            && let Some(metrics) = self.0.metrics.get()
        {
            let y = px(
                (f64::from(metrics.max_offset.height) * (percent / 100.0).clamp(0.0, 1.0)) as f32,
            );
            pending.set((y != metrics.offset.y).then_some(y));
        }
    }

    pub(crate) fn scroll_by(&self, delta: f64) -> f64 {
        let Some(pending) = self.0.pending.borrow().upgrade() else {
            return 0.0;
        };
        let Some(metrics) = self.0.metrics.get() else {
            return 0.0;
        };
        let limit = f64::from(metrics.max_offset.height);
        let current = f64::from(pending.get().unwrap_or(metrics.offset.y)).min(limit);
        let used = delta.clamp(-current, limit - current);
        if used != 0.0 {
            let y = px((current + used) as f32);
            pending.set((y != metrics.offset.y).then_some(y));
        }
        used
    }

    fn publish(&self, metrics: Option<ScrollMetrics>, state: &mut Runtime<()>) {
        if self.0.metrics.replace(metrics) != metrics {
            self.0.observers.publish(state, self.vertical_position());
        }
    }
}

pub(crate) struct TrackedScroll {
    handle: ScrollHandle,
    pending: Rc<Cell<Option<Pixels>>>,
}

impl TrackedScroll {
    pub(crate) fn attach(handle: ScrollHandle) -> Result<Self> {
        ensure!(
            handle.0.pending.borrow().upgrade().is_none(),
            "scroll handle already attached"
        );
        let pending = Rc::new(Cell::new(None));
        *handle.0.pending.borrow_mut() = Rc::downgrade(&pending);
        handle.0.metrics.set(None);
        Ok(Self { handle, pending })
    }

    pub(crate) fn has_request(&self) -> bool {
        self.pending.get().is_some()
    }

    pub(crate) fn detach(&self, state: &mut Runtime<()>) {
        self.handle.publish(None, state);
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
        ensure!(
            offset.x.get().is_finite() && offset.y.get().is_finite(),
            "scroll offset must be finite"
        );
        let instance = self
            .instances
            .get_mut(&view)
            .ok_or(crate::tree::TreeError::InvalidNode(view.0))?;
        let metrics = instance
            .scroll_metrics(node)
            .context("expected a visible, laid-out scroll container")?;
        let next = Origin {
            x: offset
                .x
                .max(Pixels::default())
                .min(metrics.max_offset.width),
            y: offset
                .y
                .max(Pixels::default())
                .min(metrics.max_offset.height),
        };
        let moved = instance.apply_scroll_offset(node, metrics.offset, next)?;
        if let Some(tracked) = instance.tracked_scroll.get(&node) {
            tracked.pending.set(None);
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

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ScrollAxes {
    x: bool,
    y: bool,
}

impl ScrollAxes {
    pub(crate) fn from_layout(layout: Option<&LayoutProperties>) -> Self {
        let scrolls = |axis| matches!(axis, Some(Overflow::Scroll | Overflow::Auto));
        Self {
            x: scrolls(layout.and_then(|v| v.overflow_x)),
            y: scrolls(layout.and_then(|v| v.overflow_y)),
        }
    }

    pub(crate) fn limit(self, geometry: Geometry) -> Size {
        let size = geometry.metrics.size;
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

/// Consume the available distance separately on each axis and pass any excess outward.
fn advance(offset: Origin, limit: Size, delta: ScrollDelta) -> (Origin, ScrollDelta) {
    fn axis(offset: Pixels, limit: Pixels, delta: Pixels) -> (Pixels, Pixels) {
        let offset = offset.min(limit);
        let used = delta.max(Pixels::default() - offset).min(limit - offset);
        ((offset + used).min(limit), delta - used)
    }
    let (x, left_x) = axis(offset.x, limit.width, delta.x);
    let (y, left_y) = axis(offset.y, limit.height, delta.y);
    (
        Origin { x, y },
        ScrollDelta {
            x: left_x,
            y: left_y,
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
            viewport: geometry.metrics.size,
        })
    }

    pub(crate) fn update_tracked_scroll(&mut self, state: &mut Runtime<()>) -> Result<()> {
        // Moving an offset mutates the instance's hit geometry and draw caches.
        let handles = std::mem::take(&mut self.tracked_scroll);
        let result = handles
            .iter()
            .try_for_each(|(&node, tracked)| -> Result<()> {
                let mut metrics = self.scroll_metrics(node);
                if let (Some(y), Some(m)) = (tracked.pending.take(), &mut metrics) {
                    let next = Origin {
                        y: y.min(m.max_offset.height),
                        ..m.offset
                    };
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
        accepted: Size,
        delta: ScrollDelta,
        state: &mut Runtime<()>,
    ) -> Result<ScrollDelta> {
        let geometry = self
            .scene
            .geometry(owner)
            .expect("laid-out scroll container");
        let live = self.bodies[&owner].scroll_axes.limit(geometry);
        let limit = Size {
            width: live.width.min(accepted.width),
            height: live.height.min(accepted.height),
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
            .and_then(|tracked| tracked.pending.take())
            .map_or(previous, |y| Origin { y, ..previous });
        let (next, remaining) = advance(start, limit, delta);
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
        self.fixed_labels = Default::default();
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
