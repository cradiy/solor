//! Immutable hit geometry paired with a host submission.
use std::{collections::HashMap, sync::Arc};

use anyhow::{Context as _, ensure};

use super::{PointerButton, PointerEvent, PointerSample};
use crate::{
    Instance, NodeId, Result, Ui, ViewId,
    layout::{Origin, Size},
};

#[derive(Clone, Copy)]
struct HitBox {
    origin: Origin,
    size: Size,
}
impl HitBox {
    fn contains(self, p: Origin) -> bool {
        let x = p.x - self.origin.x;
        let y = p.y - self.origin.y;
        x.is_extent() && y.is_extent() && x < self.size.width && y < self.size.height
    }
}

struct Entry {
    node: NodeId,
    area: HitBox,
    recipient: Option<NodeId>,
    testable: bool,
}

/// Immutable input geometry prepared alongside a view's pending presentation.
pub struct PointerFrame {
    pub(super) view: ViewId,
    geometry: Arc<HitGeometry>,
}

pub(super) struct HitGeometry {
    entries: Vec<Entry>,
    interactive: HashMap<NodeId, HitBox>,
}

impl PointerFrame {
    pub(crate) fn paint_order(&self) -> impl Iterator<Item = (NodeId, Origin)> + '_ {
        self.geometry
            .entries
            .iter()
            .map(|entry| (entry.node, entry.area.origin))
    }

    pub(super) fn has_recipient(&self, node: NodeId) -> bool {
        self.geometry.interactive.contains_key(&node)
    }

    pub(super) fn hit(&self, p: Origin) -> Option<(NodeId, NodeId)> {
        if !self.geometry.entries.first()?.area.contains(p) {
            return None;
        }
        let hit = self
            .geometry
            .entries
            .iter()
            .rev()
            .find(|entry| entry.testable && entry.area.contains(p))?;
        Some((hit.node, hit.recipient?))
    }

    pub(super) fn event(
        &self,
        node: NodeId,
        sample: PointerSample,
        button: Option<PointerButton>,
    ) -> Option<PointerEvent> {
        let area = self.geometry.interactive.get(&node)?;
        Some(PointerEvent {
            sample,
            button,
            size: area.size,
            position: Origin {
                x: sample.position.x - area.origin.x,
                y: sample.position.y - area.origin.y,
            },
        })
    }
}

impl Ui {
    /// Snapshots the laid-out border boxes in paint order for one submission.
    pub fn prepare_pointer_frame(&self, view: ViewId) -> Result<PointerFrame> {
        let instance = self.instance(view)?;
        ensure!(!instance.scene.is_pending(), "layout pending");
        let cache = &instance.pointer.geometry;
        let geometry = match cache.get() {
            Some(geometry) => geometry,
            None => {
                let built = HitGeometry::from_layout(instance)?;
                cache.get_or_init(|| Arc::new(built))
            }
        };
        Ok(PointerFrame {
            view,
            geometry: geometry.clone(),
        })
    }
}

impl HitGeometry {
    fn from_layout(instance: &Instance) -> Result<Self> {
        let mut frame = Self {
            entries: Vec::with_capacity(instance.nodes.len()),
            interactive: HashMap::new(),
        };
        let root = &instance.bodies[&instance.scene.root()];
        let mut pending = vec![(root, Origin::default(), None)];
        while let Some((body, offset, inherited)) = pending.pop() {
            let id = body.id;
            let geometry = instance
                .scene
                .geometry(id)
                .context("missing pointer geometry")?;
            let origin = Origin {
                x: offset
                    .x
                    .checked_add(geometry.origin.x)
                    .context("pointer x outside layout range")?,
                y: offset
                    .y
                    .checked_add(geometry.origin.y)
                    .context("pointer y outside layout range")?,
            };
            let area = HitBox {
                origin,
                size: geometry.metrics.size,
            };
            let recipient = if body.hit_testable && body.handlers.is_some() {
                frame.interactive.insert(id, area);
                Some(id)
            } else {
                inherited
            };
            frame.entries.push(Entry {
                node: id,
                area,
                recipient,
                testable: body.hit_testable,
            });
            let start = pending.len();
            for child in instance.scene.children(id)? {
                pending.push((&instance.bodies[&child], origin, recipient));
            }
            let siblings = &mut pending[start..];
            if !siblings.is_sorted_by_key(|(child, _, _)| child.z_index) {
                siblings.sort_by_key(|(child, _, _)| child.z_index);
            }
            // Pop lower layers first, visiting each entire subtree before the next sibling.
            siblings.reverse();
        }
        Ok(frame)
    }
}
