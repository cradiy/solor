//! Immutable hit geometry paired with a host submission.
use std::{collections::HashMap, sync::Arc};

use anyhow::{Context as _, ensure};

use super::{PointerButton, PointerEvent, PointerSample};
use crate::clip::{NodeClips, OverflowClip};
use crate::{
    Instance, NodeId, Result, Ui, ViewId,
    element::FramePaint,
    layout::{Origin, Size},
    styled_body::StyledBody,
    text::Text,
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
    pub(super) hover: Option<NodeId>,
    pub(super) cursor: Option<crate::style::Cursor>,
}

struct InteractiveBox {
    border: HitBox,
    padding: crate::styled_body::ContentInsets,
}

struct Entry {
    node: NodeId,
    area: HitBox,
    recipients: Recipients,
    testable: bool,
    paints: bool,
    clips: usize,
}

/// Immutable input geometry prepared alongside a view's pending presentation.
pub struct PointerFrame {
    pub(super) view: ViewId,
    geometry: Arc<HitGeometry>,
}

pub(super) struct HitGeometry {
    frame_listeners: Vec<NodeId>,
    entries: Vec<Entry>,
    clips: Vec<NodeClips>,
    interactive: HashMap<NodeId, InteractiveBox>,
    hover_parents: HashMap<NodeId, Option<NodeId>>,
    scroll_parents: HashMap<NodeId, NodeId>,
    scroll_limits: HashMap<NodeId, Size>,
    has_cursor: bool,
}

impl PointerFrame {
    pub(crate) fn frame_listeners(&self) -> &[NodeId] {
        &self.geometry.frame_listeners
    }

    pub(crate) fn paint_order(&self) -> impl Iterator<Item = (NodeId, Origin, &NodeClips)> + '_ {
        self.geometry
            .entries
            .iter()
            .filter(|entry| entry.paints)
            .map(|entry| {
                (
                    entry.node,
                    entry.area.origin,
                    &self.geometry.clips[entry.clips],
                )
            })
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
                && self.geometry.clips[entry.clips]
                    .inherited
                    .as_deref()
                    .is_none_or(|clip| clip.contains(p))
        })?;
        Some((hit.node, hit.recipients))
    }

    pub(super) fn same_geometry(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.geometry, &other.geometry)
    }

    pub(super) fn mouse_target(&self, at: Origin, live: impl Fn(NodeId) -> bool) -> Recipients {
        if (self.geometry.has_cursor || !self.geometry.hover_parents.is_empty())
            && let Some((leaf, target)) = self.hit(at)
            && live(leaf)
        {
            return target;
        }
        Recipients::default()
    }

    pub(super) fn hover_path(&self, owner: Option<NodeId>) -> impl Iterator<Item = NodeId> + '_ {
        std::iter::successors(owner, |id| self.geometry.hover_parents[id])
    }

    pub(super) fn scroll_limit(&self, owner: NodeId) -> Option<Size> {
        self.geometry.scroll_limits.get(&owner).copied()
    }

    pub(super) fn is_current(&self, router: &super::PointerRouter) -> bool {
        router
            .geometry
            .get()
            .is_some_and(|g| Arc::ptr_eq(g, &self.geometry))
    }

    pub(super) fn scroll_parent(&self, owner: NodeId) -> Option<NodeId> {
        self.geometry.scroll_parents.get(&owner).copied()
    }

    pub(super) fn bounds(&self, node: NodeId) -> Option<(Origin, Size)> {
        self.geometry
            .interactive
            .get(&node)
            .map(|b| (b.border.origin, b.border.size))
    }

    pub(super) fn event(
        &self,
        node: NodeId,
        sample: PointerSample,
        button: Option<PointerButton>,
    ) -> Option<PointerEvent> {
        let target = self.geometry.interactive.get(&node)?;
        let area = target.border;
        Some(PointerEvent {
            sample,
            button,
            size: area.size,
            padding: target.padding,
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
            clips: vec![NodeClips::default()],
            interactive: HashMap::new(),
            hover_parents: HashMap::new(),
            scroll_parents: HashMap::new(),
            scroll_limits: HashMap::new(),
            has_cursor: false,
        };
        let root = &instance.bodies[&instance.scene.root()];
        if !root.visible {
            return Ok(frame);
        }
        let mut pending = vec![(root, Origin::default(), Recipients::default(), 0)];
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
            // A fixed label is a leaf. Omit it only when neither its border hit box
            // nor its padded text command can intersect the inherited clip.
            if body.frame_paint == FramePaint::FixedLabel
                && let Some(clip) = &frame.clips[inherited_clip].content
                && !clip.intersects(origin, area.size)
            {
                let label = instance.scene.body::<StyledBody<Text>>(id)?;
                let text_origin = origin
                    .checked_add(label.origin())
                    .context("paint origin outside layout range")?;
                if !clip.intersects(text_origin, label.content_size(area.size)) {
                    continue;
                }
            }
            let mut recipients = inherited;
            if body.hit_testable && body.cursor.is_some() {
                recipients.cursor = body.cursor;
                frame.has_cursor = true;
            }
            if body.hit_testable
                && let Some(handlers) = body.handlers.as_deref()
            {
                frame.interactive.insert(
                    id,
                    InteractiveBox {
                        border: area,
                        padding: handlers.padding,
                    },
                );
                if handlers.hover.is_some()
                    || handlers.hover_position.is_some()
                    || handlers.tooltip.is_some()
                {
                    frame.hover_parents.insert(id, recipients.hover);
                    recipients.hover = Some(id);
                }
                if handlers.handles_pointer() {
                    recipients.pointer = Some(id);
                    frame.has_cursor |=
                        matches!(handlers.control, Some(crate::WindowControl::Resize(_)));
                }
            }
            let children = instance.scene.children(id)?;
            let mut child_origin = origin;
            let mut scroll_listener = body.handlers.as_deref().is_some_and(|h| h.scroll.is_some());
            if body.scroll_axes != super::ScrollAxes::default() && children.len() != 0 {
                let limit = body.scroll_axes.limit(geometry);
                if limit != Size::default() {
                    if let Some(offset) = instance.pointer.scroll_offsets.get(&id) {
                        child_origin.x = origin.x - offset.x.min(limit.width);
                        child_origin.y = origin.y - offset.y.min(limit.height);
                    }
                    if !scroll_listener && body.hit_testable {
                        frame.scroll_limits.insert(id, limit);
                        scroll_listener = true;
                    }
                }
            }
            if body.hit_testable && scroll_listener {
                if let Some(parent) = inherited.scroll {
                    frame.scroll_parents.insert(id, parent);
                }
                recipients.scroll = Some(id);
            }
            let (clips, child_clip) = frame.resolve_clips(
                inherited_clip,
                body.overflow_clip,
                area,
                children.len() > 0,
            )?;
            frame.entries.push(Entry {
                clips,
                node: id,
                area,
                recipients,
                testable: body.hit_testable,
                paints: body.frame_paint != FramePaint::Empty,
            });
            let start = pending.len();
            for child in children {
                let child = &instance.bodies[&child];
                if child.visible {
                    pending.push((child, child_origin, recipients, child_clip));
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

    fn resolve_clips(
        &mut self,
        inherited: usize,
        overflow: OverflowClip,
        area: HitBox,
        has_children: bool,
    ) -> Result<(usize, usize)> {
        if overflow == OverflowClip::default() {
            return Ok((inherited, inherited));
        }
        let clips = overflow.resolve(
            self.clips[inherited].content.clone(),
            area.origin,
            area.size,
        )?;
        let own = self.clips.len();
        self.clips.push(clips);
        if !has_children {
            return Ok((own, own));
        }
        // Descendants use the viewport's content clip for both their border and content.
        let content = self.clips[own].content.clone();
        let children = self.clips.len();
        self.clips.push(NodeClips {
            inherited: content.clone(),
            content,
        });
        Ok((own, children))
    }
}
