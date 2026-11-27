//! Immutable hit geometry paired with a host submission.
use std::{collections::HashMap, rc::Rc, sync::Arc};

use anyhow::{Context as _, ensure};

use super::{PointerButton, PointerEvent, PointerSample};
use crate::clip::{NodeClips, OverflowClip};
use crate::{
    Instance, NodeId, Result, Ui, ViewId,
    element::{FramePaint, MountedNode},
    layout::{Origin, Size},
    platform::SurfaceRect,
    styled_body::StyledBody,
    text::Text,
};

#[derive(Clone, Copy, Default)]
pub(super) struct Recipients {
    pub focus: Option<NodeId>,
    pub pointer: Option<NodeId>,
    pub scroll: Option<NodeId>,
    pub file_drop: Option<NodeId>,
    pub(super) hover: Option<NodeId>,
    pub(super) cursor: Option<crate::style::Cursor>,
}

struct InteractiveBox {
    border: SurfaceRect,
    pointer_padding: Option<crate::styled_body::ContentInsets>,
}

struct Entry {
    node: NodeId,
    area: SurfaceRect,
    recipients: Recipients,
    testable: bool,
    paints: bool,
    clips: usize,
}

/// Immutable input geometry prepared alongside a view's pending presentation.
#[derive(Clone)]
pub struct PointerFrame {
    pub(super) view: ViewId,
    geometry: Arc<HitGeometry>,
    text_hits: Option<Rc<HashMap<NodeId, TextHit>>>,
}

struct Field {
    label: NodeId,
    source: crate::Signal<crate::text::TextInputState>,
    origin: Origin,
    size: Size,
}

struct TextHit {
    paragraph: Rc<crate::text::Paragraph>,
    origin: Origin,
    start: Option<usize>,
}

#[derive(Clone, Copy)]
pub(super) struct ScrollViewport {
    pub limit: Size,
    pub wheel: super::scroll::WheelPolicy,
}

pub(super) struct HitGeometry {
    fields: HashMap<NodeId, Field>,
    frame_listeners: Vec<NodeId>,
    entries: Vec<Entry>,
    group_events: Vec<(usize, Option<NodeId>)>,
    clips: Vec<NodeClips>,
    interactive: HashMap<NodeId, InteractiveBox>,
    hover_parents: HashMap<NodeId, Option<NodeId>>,
    scroll_parents: HashMap<NodeId, NodeId>,
    scroll_viewports: HashMap<NodeId, ScrollViewport>,
    has_cursor: bool,
}

impl PointerFrame {
    pub(crate) fn focused_caret(
        &self,
        instance: &Instance,
        state: &crate::state::Runtime<()>,
    ) -> Option<crate::text::FrameCaret> {
        let field = self.geometry.fields.get(&instance.focused?)?;
        let label = instance.scene.body::<StyledBody<Text>>(field.label).ok()?;
        let color = label
            .content
            .input
            .as_ref()?
            .selection_color
            .unwrap_or(crate::rgba(0x3684eb60));
        Some(crate::text::FrameCaret::new(
            field.label,
            state.read(field.source).ok()?,
            color,
        ))
    }

    pub(super) fn place_caret(
        &self,
        owner: NodeId,
        point: Origin,
        state: &mut crate::state::Runtime<()>,
    ) -> Result<()> {
        let Some(field) = self.geometry.fields.get(&owner) else {
            return Ok(());
        };
        let Some(hit) = self.text_hits.as_ref().and_then(|hits| hits.get(&owner)) else {
            return Ok(());
        };
        let Ok(input) = state.read(field.source) else {
            return Ok(());
        };
        let Some(start) = hit.start else {
            return Ok(());
        };
        if input.preedit().is_some()
            || input.text().get(start..start + hit.paragraph.text.len())
                != Some(hit.paragraph.text.as_str())
        {
            return Ok(());
        }
        if let Some(byte) = hit.paragraph.closest_caret(Origin {
            x: point.x - hit.origin.x,
            y: point.y - hit.origin.y,
        }) {
            let selection = crate::text::TextSelection::caret(start + byte);
            if input.selection() != selection {
                state.update_in_place(field.source, |s| s.select(selection))??;
            }
        }
        Ok(())
    }

    pub(crate) fn frame_listeners(&self) -> &[NodeId] {
        &self.geometry.frame_listeners
    }

    // Group boundaries use indices in paint_order, including when the group owner
    // has no commands of its own. Siblings are traversed in stacking order.
    pub(crate) fn group_events(&self) -> &[(usize, Option<NodeId>)] {
        &self.geometry.group_events
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
        self.geometry
            .interactive
            .get(&node)
            .is_some_and(|area| area.pointer_padding.is_some())
    }

    pub(super) fn hit(&self, p: Origin) -> Option<(NodeId, Recipients)> {
        if !self.geometry.entries.first()?.area.contains(p) {
            return None;
        }
        let hit = self.geometry.entries.iter().rev().find(|entry| {
            entry.testable && entry.area.contains(p) && self.geometry.clips[entry.clips].contains(p)
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

    pub(super) fn scroll_viewport(&self, owner: NodeId) -> Option<ScrollViewport> {
        self.geometry.scroll_viewports.get(&owner).copied()
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

    pub(crate) fn bounds(&self, node: NodeId) -> Option<SurfaceRect> {
        self.geometry.interactive.get(&node).map(|b| b.border)
    }

    pub(super) fn event(
        &self,
        node: NodeId,
        sample: PointerSample,
        button: Option<PointerButton>,
    ) -> Option<PointerEvent> {
        let target = self.geometry.interactive.get(&node)?;
        Some(PointerEvent {
            sample,
            button,
            bounds: target.border,
            padding: target.pointer_padding?,
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
        let mut frame = PointerFrame {
            view,
            geometry: geometry.clone(),
            text_hits: None,
        };
        if geometry.fields.is_empty() {
            return Ok(frame);
        }
        let state = self.state.try_borrow()?;
        let mut hits = HashMap::with_capacity(geometry.fields.len());
        for (&owner, field) in &geometry.fields {
            let body = instance.scene.body::<StyledBody<Text>>(field.label)?;
            let size = body.content_size(field.size);
            let paragraph = body
                .content
                .resolve(crate::layout::Query::Exact(size.width))?;
            let input = state.read(field.source)?;
            let (caret, _) = input.label_caret();
            let scroll = body.content.scroll_for_caret(&paragraph, size.width, caret);
            let mut origin = field
                .origin
                .checked_add(body.origin())
                .context("invalid text origin")?;
            origin.x = origin.x - scroll;
            hits.insert(
                owner,
                TextHit {
                    paragraph,
                    origin,
                    start: input
                        .preedit()
                        .is_none()
                        .then(|| input.selection().head - caret),
                },
            );
        }
        frame.text_hits = Some(Rc::new(hits));
        Ok(frame)
    }
}

impl HitGeometry {
    fn from_layout(instance: &Instance) -> Result<Self> {
        let mut frame = Self {
            fields: HashMap::new(),
            frame_listeners: Vec::new(),
            group_events: Vec::new(),
            entries: Vec::with_capacity(instance.visible_nodes),
            clips: vec![NodeClips::default()],
            interactive: HashMap::new(),
            hover_parents: HashMap::new(),
            scroll_parents: HashMap::new(),
            scroll_viewports: HashMap::new(),
            has_cursor: false,
        };
        let root = &instance.bodies[&instance.scene.root()];
        if !root.visible {
            return Ok(frame);
        }
        let mut pending = vec![(root, Origin::default(), Recipients::default(), 0)];
        let mut open_groups = Vec::new();
        let mut paint_index = 0;
        loop {
            while open_groups.last() == Some(&pending.len()) {
                open_groups.pop();
                frame.group_events.push((paint_index, None));
            }
            let Some((body, offset, inherited, inherited_clip)) = pending.pop() else {
                break;
            };
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
            let area = SurfaceRect {
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
            if body.hit_testable && body.focusable {
                recipients.focus = Some(id);
            }
            if body.hit_testable && body.cursor.is_some() {
                recipients.cursor = body.cursor;
                frame.has_cursor = true;
            }
            let pointer_handlers = body.handlers.as_deref().filter(|_| body.hit_testable);
            if body.focusable || pointer_handlers.is_some() {
                frame.interactive.insert(
                    id,
                    InteractiveBox {
                        border: area,
                        pointer_padding: pointer_handlers.map(|h| h.padding),
                    },
                );
            }
            if let Some(handlers) = pointer_handlers {
                if handlers.hover.is_some()
                    || handlers.hover_position.is_some()
                    || handlers.tooltip.is_some()
                {
                    frame.hover_parents.insert(id, recipients.hover);
                    recipients.hover = Some(id);
                }
                if handlers.file_drop.is_some() {
                    recipients.file_drop = Some(id);
                }
                if handlers.handles_pointer() {
                    recipients.pointer = Some(id);
                    frame.has_cursor |=
                        matches!(handlers.control, Some(crate::WindowControl::Resize(_)));
                }
            }
            if let Some(owner) = recipients.focus
                && let Some(source) = instance.bodies[&owner]
                    .handlers
                    .as_deref()
                    .and_then(|h| h.text_input)
                && let Ok(body) = instance.scene.body::<StyledBody<Text>>(id)
                && body
                    .content
                    .input
                    .as_ref()
                    .is_some_and(|i| i.source == source)
            {
                frame.fields.entry(owner).or_insert(Field {
                    label: id,
                    source,
                    origin,
                    size: area.size,
                });
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
                        frame.scroll_viewports.insert(
                            id,
                            ScrollViewport {
                                limit,
                                wheel: body.scroll_axes.wheel,
                            },
                        );
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
            let (clips, child_clip) =
                frame.resolve_clips(inherited_clip, body, area, children.len() > 0)?;
            if body.is_group() {
                frame.group_events.push((paint_index, Some(id)));
                open_groups.push(pending.len());
            }
            let paints = body.frame_paint != FramePaint::Empty;
            paint_index += usize::from(paints);
            frame.entries.push(Entry {
                clips,
                node: id,
                area,
                recipients,
                testable: body.hit_testable,
                paints,
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
        body: &MountedNode,
        area: SurfaceRect,
        has_children: bool,
    ) -> Result<(usize, usize)> {
        if body.overflow_clip == OverflowClip::default() {
            return Ok((inherited, inherited));
        }
        let clips = body.overflow_clip.resolve(
            self.clips[inherited].content.clone(),
            area.origin,
            body.scroll_axes.viewport(area.size),
            area.size,
            body.corner_radii,
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
