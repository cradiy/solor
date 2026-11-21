//! Mounted container scroll policies and logical scroll distances.
use crate::{
    Instance, NodeId, Pixels, Result,
    layout::{Geometry, Origin, Size},
    style::{LayoutProperties, Overflow},
};

use super::ScrollDelta;

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
    pub(super) fn scroll_children(
        &mut self,
        owner: NodeId,
        accepted: Size,
        delta: ScrollDelta,
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
        let offsets = &mut self.pointer.scroll_offsets;
        let previous = offsets.get(&owner).copied().unwrap_or_default();
        let (next, remaining) = advance(previous, limit, delta);
        if next == previous {
            return Ok(remaining);
        }
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
        Ok(remaining)
    }
}
