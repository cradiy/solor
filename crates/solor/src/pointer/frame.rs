//! Immutable hit geometry paired with a host submission.
use std::{collections::HashMap, sync::Arc};

use anyhow::{Context as _, ensure};

use super::{PointerButton, PointerEvent, PointerSample};
use crate::clip::NodeClips;
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

#[derive(Clone, Copy, Default)]
pub(super) struct Recipients {
    pub pointer: Option<NodeId>,
    pub scroll: Option<NodeId>,
}

struct Entry {
    node: NodeId,
    area: HitBox,
    recipients: Recipients,
    testable: bool,
    clips: NodeClips,
}

/// Immutable input geometry prepared alongside a view's pending presentation.
pub struct PointerFrame {
    pub(super) view: ViewId,
    geometry: Arc<HitGeometry>,
}

pub(super) struct HitGeometry {
    frame_listeners: Vec<NodeId>,
    entries: Vec<Entry>,
    interactive: HashMap<NodeId, HitBox>,
}

impl PointerFrame {
    pub(crate) fn frame_listeners(&self) -> &[NodeId] {
        &self.geometry.frame_listeners
    }

    pub(crate) fn paint_order(&self) -> impl Iterator<Item = (NodeId, Origin, &NodeClips)> + '_ {
        self.geometry
            .entries
            .iter()
            .map(|entry| (entry.node, entry.area.origin, &entry.clips))
    }

    pub(super) fn has_recipient(&self, node: NodeId) -> bool {
        self.geometry.interactive.contains_key(&node)
    }

    pub(super) fn hit(&self, p: Origin) -> Option<(NodeId, Recipients)> {
        if !self.geometry.entries.first()?.area.contains(p) {
            return None;
        }
        let hit = self.geometry.entries.iter().rev().find(|entry| {
            entry.testable
                && entry.area.contains(p)
                && entry
                    .clips
                    .inherited
                    .as_deref()
                    .is_none_or(|clip| clip.contains(p))
        })?;
        Some((hit.node, hit.recipients))
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
            frame_listeners: Vec::new(),
            entries: Vec::with_capacity(instance.visible_nodes),
            interactive: HashMap::new(),
        };
        let root = &instance.bodies[&instance.scene.root()];
        if !root.visible {
            return Ok(frame);
        }
        let mut pending = vec![(root, Origin::default(), Recipients::default(), None)];
        while let Some((body, offset, inherited, inherited_clip)) = pending.pop() {
            let id = body.id;
            if instance.frame_handlers.contains_key(&id) {
                frame.frame_listeners.push(id);
            }
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
            let mut recipients = inherited;
            if body.hit_testable
                && let Some(handlers) = body.handlers.as_deref()
            {
                frame.interactive.insert(id, area);
                if handlers.handles_pointer() {
                    recipients.pointer = Some(id);
                }
                if handlers.scroll.is_some() {
                    recipients.scroll = Some(id);
                }
            }
            let clips = body
                .overflow_clip
                .resolve(inherited_clip, origin, area.size)?;
            let child_clip = clips.content.clone();
            frame.entries.push(Entry {
                clips,
                node: id,
                area,
                recipients,
                testable: body.hit_testable,
            });
            let start = pending.len();
            for child in instance.scene.children(id)? {
                let child = &instance.bodies[&child];
                if child.visible {
                    pending.push((child, origin, recipients, child_clip.clone()));
                }
            }
            let siblings = &mut pending[start..];
            if !siblings.is_sorted_by_key(|(child, ..)| child.z_index) {
                siblings.sort_by_key(|(child, ..)| child.z_index);
            }
            // Pop lower layers first, visiting each entire subtree before the next sibling.
            siblings.reverse();
        }
        Ok(frame)
    }
}
