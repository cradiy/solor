//! Scroll-time culling for vertically ordered leaves in an overflow viewport.
use std::{collections::HashMap, collections::hash_map::Entry, ops::Range, sync::Arc};

use anyhow::Context as _;

use crate::{
    Instance, NodeId, Pixels, Result,
    element::FramePaint,
    layout::Origin,
    platform::SurfaceRect,
    styled_body::{ContentInsets, StyledBody},
    text::Text,
};

#[derive(Default)]
pub(super) struct ScrollRows {
    containers: HashMap<NodeId, Option<RowBounds>>,
    indexed: usize,
}

impl ScrollRows {
    pub(super) fn indexed(&self) -> usize {
        self.indexed
    }

    pub(super) fn layout(
        &mut self,
        instance: &Instance,
        owner: NodeId,
    ) -> Result<Option<&RowBounds>> {
        let rows = match self.containers.entry(owner) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let rows = RowBounds::from_layout(instance, owner)?;
                if let Some(rows) = &rows {
                    self.indexed += rows.len;
                }
                entry.insert(rows)
            }
        };
        Ok(rows.as_ref())
    }
}

#[derive(Default)]
pub(super) struct RowBounds {
    targets: Option<Arc<[RowTarget]>>,
    len: usize,
    min_content_y: Pixels,
    max_content_y: Pixels,
    max_height: Pixels,
}

impl RowBounds {
    fn from_layout(instance: &Instance, owner: NodeId) -> Result<Option<Self>> {
        let mut rows = Self::default();
        let children = instance.scene.children(owner)?;
        rows.len = children.len();
        let mut previous = None;
        let mut targets = Vec::new();
        for id in children {
            let node = &instance.bodies[&id];
            if !node.visible
                || instance.frame_handlers.contains_key(&id)
                || !matches!(
                    node.frame_paint,
                    FramePaint::Empty | FramePaint::FixedLabel | FramePaint::FixedBackground
                )
                || instance.scene.children(id)?.len() != 0
            {
                return Ok(None);
            }
            let Some(layout) = instance.scene.geometry(id) else {
                return Ok(None);
            };
            if previous.is_some_and(|last| last > layout.origin.y) {
                return Ok(None);
            }
            let padding = node
                .handlers
                .as_deref()
                .filter(|_| node.hit_testable)
                .map(|h| h.padding);
            if padding.is_some() || node.focusable {
                targets.push(RowTarget {
                    node: id,
                    bounds: SurfaceRect {
                        origin: layout.origin,
                        size: layout.metrics.size,
                    },
                    padding,
                });
            }
            previous = Some(layout.origin.y);
            rows.max_height = rows.max_height.max(layout.metrics.size.height);
            if node.frame_paint == FramePaint::FixedLabel {
                let label = instance.scene.body::<StyledBody<Text>>(id)?;
                let y = label.origin().y;
                rows.min_content_y = rows.min_content_y.min(y);
                rows.max_content_y = rows.max_content_y.max(y);
                rows.max_height = rows
                    .max_height
                    .max(label.content_size(layout.metrics.size).height);
            }
        }
        if !targets.is_empty() {
            targets.sort_unstable_by_key(|target| target.node.sort_key());
            rows.targets = Some(targets.into());
        }
        Ok(Some(rows))
    }

    pub(super) fn targets_at(&self, origin: Origin) -> Option<PlacedRowTargets> {
        Some(PlacedRowTargets {
            origin,
            targets: self.targets.as_ref()?.clone(),
        })
    }

    pub(super) fn visible(
        &self,
        instance: &Instance,
        owner: NodeId,
        origin: Pixels,
        clip: &Range<Pixels>,
    ) -> Result<Range<usize>> {
        let partition = |mut end: usize, before: &dyn Fn(Pixels) -> bool| -> Result<usize> {
            let mut start = 0;
            while start < end {
                let mid = start + (end - start) / 2;
                let row = instance
                    .scene
                    .children(owner)?
                    .nth(mid)
                    .context("missing scroll row")?;
                let layout = instance
                    .scene
                    .geometry(row)
                    .context("missing scroll row geometry")?;
                if before(origin + layout.origin.y) {
                    start = mid + 1;
                } else {
                    end = mid;
                }
            }
            Ok(start)
        };
        // Use the same additions as painting for fractional origins and padded labels.
        let end = partition(self.len, &|top| top + self.min_content_y < clip.end)?;
        let start = partition(end, &|top| {
            f64::from(clip.start) - f64::from(top + self.max_content_y)
                >= f64::from(self.max_height)
        })?;
        Ok(start..end)
    }
}

struct RowTarget {
    node: NodeId,
    bounds: SurfaceRect,
    padding: Option<ContentInsets>,
}

// A submitted frame owns the translation; pure scrolling shares the local boxes.
pub(super) struct PlacedRowTargets {
    origin: Origin,
    targets: Arc<[RowTarget]>,
}

impl PlacedRowTargets {
    pub(super) fn get(&self, node: NodeId) -> Option<(SurfaceRect, Option<ContentInsets>)> {
        let index = self
            .targets
            .binary_search_by_key(&node.sort_key(), |target| target.node.sort_key())
            .ok()?;
        let target = &self.targets[index];
        let bounds = SurfaceRect {
            origin: self.origin.checked_add(target.bounds.origin)?,
            ..target.bounds
        };
        Some((bounds, target.padding))
    }
}
