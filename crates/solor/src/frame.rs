//! Immutable output of recording a mounted view.
use std::{
    cell::{Cell, OnceCell},
    rc::Rc,
    sync::Arc,
};

use anyhow::{Context as _, Result};

use crate::{
    ContentClip, Draw, Ink, Instance, Ui, ViewId,
    clip::NodeClips,
    element::{FramePaint, MountedNode},
    layout::{Origin, Size},
    pointer::PointerFrame,
    state::Runtime,
    text::Paragraph,
};

#[derive(Default)]
pub(super) struct FixedDraws {
    previous_peak: Cell<Option<usize>>,
    commands: OnceCell<FixedCommands>,
}

#[derive(Default)]
struct FixedCommands {
    labels: Vec<FixedLabel>,
    backgrounds: Vec<(usize, Draw)>,
    edges: Option<Rc<FixedEdges>>,
}

struct FixedEdges {
    prefix: usize,
    suffix: usize,
}

struct FixedLabel {
    clip: Option<Arc<ContentClip>>,
    index: usize,
    origin: Origin,
    size: Size,
    paragraph: Rc<Paragraph>,
    color: Ink,
}

impl FixedLabel {
    fn from_commands(index: usize, commands: &[Draw]) -> Option<Self> {
        let [
            Draw::Text {
                clip,
                origin,
                size,
                paragraph,
                color,
            },
        ] = commands
        else {
            return None;
        };
        Some(Self {
            clip: clip.clone(),
            index,
            origin: *origin,
            size: *size,
            paragraph: paragraph.clone(),
            color: color.clone(),
        })
    }

    fn draw(&self) -> Draw {
        Draw::Text {
            clip: self.clip.clone(),
            origin: self.origin,
            size: self.size,
            paragraph: self.paragraph.clone(),
            color: self.color.clone(),
        }
    }
}

/// Commands in view-local logical coordinates and the matching pointer geometry.
pub struct ViewFrame {
    draws: Vec<Draw>,
    pointer: PointerFrame,
    next_frame: bool,
    fixed_edges: Option<Rc<FixedEdges>>,
    has_overlay: bool,
    caret: Option<crate::text::CaretBounds>,
}

impl ViewFrame {
    /// Returns the visible insertion point for input method placement.
    pub fn caret_bounds(&self) -> Option<crate::text::CaretBounds> {
        self.caret
    }

    /// Returns commands in paint order, with each parent before its children.
    pub fn draws(&self) -> &[Draw] {
        &self.draws
    }

    /// Counts leading and trailing commands guaranteed identical in both recordings.
    /// The two runs do not overlap. Commands between them may need comparison.
    pub fn common_draw_edges(&self, other: &Self) -> (usize, usize) {
        if let (Some(a), Some(b)) = (&self.fixed_edges, &other.fixed_edges)
            && Rc::ptr_eq(a, b)
        {
            let suffix = if self.has_overlay || other.has_overlay {
                0
            } else {
                a.suffix
            };
            (a.prefix, suffix)
        } else {
            (0, 0)
        }
    }

    /// Returns the hit geometry for submission while retaining this recording.
    pub fn pointer_frame(&self) -> &PointerFrame {
        &self.pointer
    }

    /// Whether a painted element requested a subsequent host-paced frame.
    pub fn requests_animation_frame(&self) -> bool {
        self.next_frame
    }

    /// Separates the recorded commands for retention from the hit geometry for submission.
    pub fn into_parts(self) -> (Vec<Draw>, PointerFrame) {
        (self.draws, self.pointer)
    }

    /// Takes the hit geometry for acceptance after successful presentation.
    pub fn into_pointer_frame(self) -> PointerFrame {
        self.pointer
    }
}

/// Wraps completed subtrees after their individual commands have been cached.
struct PaintGroups<'a> {
    events: &'a [(usize, Option<crate::NodeId>)],
    stack: Vec<(usize, crate::NodeId)>,
    peak: usize,
}

impl PaintGroups<'_> {
    fn advance(&mut self, index: usize, instance: &Instance, draws: &mut Vec<Draw>) {
        while let Some((&(at, owner), rest)) = self.events.split_first() {
            if at > index {
                break;
            }
            self.events = rest;
            if let Some(owner) = owner {
                self.stack.push((draws.len(), owner));
            } else {
                let (start, owner) = self.stack.pop().expect("paired subtree boundaries");
                // Closing a group moves its children out of the recording buffer.
                self.peak = self.peak.max(draws.len());
                let node = &instance.bodies[&owner];
                wrap_group(draws, start, node.opacity, node.gradient_map.as_ref());
            }
        }
    }
}

pub(super) fn wrap_group(
    draws: &mut Vec<Draw>,
    start: usize,
    alpha: u8,
    gradient_map: Option<&crate::GradientMap>,
) {
    if alpha == 0 {
        draws.truncate(start);
    } else if (alpha < 255 || gradient_map.is_some()) && start < draws.len() {
        let children = draws.split_off(start);
        draws.push(Draw::Group {
            alpha,
            gradient_map: gradient_map.cloned(),
            draws: children,
        });
    }
}

fn translate_draws(draws: &mut [Draw], offset: Origin) -> Result<()> {
    for draw in draws {
        let origin = match draw {
            Draw::Group { draws, .. } => {
                translate_draws(draws, offset)?;
                continue;
            }
            Draw::WavyUnderline { origin, .. }
            | Draw::Shadow { origin, .. }
            | Draw::Border { origin, .. }
            | Draw::Fill { origin, .. }
            | Draw::Polygon { origin, .. }
            | Draw::Polyline { origin, .. }
            | Draw::Svg { origin, .. }
            | Draw::Text { origin, .. } => origin,
        };
        *origin = offset
            .checked_add(*origin)
            .context("paint origin outside layout range")?;
    }
    Ok(())
}

impl Instance {
    fn replay_commands(&self, id: crate::NodeId, draws: &mut Vec<Draw>) -> bool {
        if let Some(saved) = self.repaints.borrow().commands.get(&id) {
            draws.extend_from_slice(saved);
            true
        } else {
            false
        }
    }
}

impl MountedNode {
    #[inline]
    fn paint_at(
        &self,
        offset: Origin,
        clips: &NodeClips,
        instance: &Instance,
        state: &mut Runtime<()>,
        draws: &mut Vec<Draw>,
    ) -> Result<bool> {
        if !self.frame_paint.is_fixed_draw() && instance.replay_commands(self.id, draws) {
            return Ok(false);
        }
        self.record_at(offset, clips, instance, state, draws)
    }

    fn record_at(
        &self,
        offset: Origin,
        clips: &NodeClips,
        instance: &Instance,
        state: &mut Runtime<()>,
        draws: &mut Vec<Draw>,
    ) -> Result<bool> {
        let start = draws.len();
        let next_frame = self.paint(instance, state, Some((clips, offset)), draws)?;
        translate_draws(&mut draws[start..], offset)?;
        if !self.frame_paint.is_fixed_draw() {
            instance.observe_paint(self.id, state)?;
            let mut saved = instance.repaints.borrow_mut();
            if next_frame {
                saved.commands.remove(&self.id);
            } else {
                saved.commands.insert(self.id, draws[start..].to_vec());
            }
        }
        Ok(next_frame)
    }
}

impl Ui {
    /// Advances the live frame listeners from the last accepted view snapshot.
    /// Pending input updates are flushed first so retired or hidden listeners do not run.
    /// Hosts call this once per delivered frame opportunity, before recording its successor.
    pub fn dispatch_frame(&mut self, view: ViewId, now: std::time::Duration) -> Result<()> {
        if self.has_pending_bindings() {
            self.flush()?;
        }
        let instance = self
            .instances
            .get_mut(&view)
            .context("frame view is no longer mounted")?;
        for id in instance.pointer.frame_listeners() {
            if let Some(handler) = instance.frame_handlers.get_mut(id)
                && let Some(node) = instance.bodies.get_mut(id)
                && node.visible
                && handler.is_enabled(self.state.get_mut())?
            {
                (handler.callback)(
                    now,
                    &mut crate::Context {
                        tasks: &self.tasks,
                        state: self.state.get_mut(),
                        engine: &self.engine,
                        owned: Some(&mut node.owned),
                    },
                )?;
            }
        }
        Ok(())
    }

    /// Records the whole laid-out view. Repaint requests are acknowledged together
    /// after successful recording. Input continues to use the last accepted frame.
    pub fn record_frame(&self, view: ViewId) -> Result<ViewFrame> {
        let instance = self.instance(view)?;
        let pointer = self.prepare_pointer_frame(view)?;
        let cache = &instance.fixed_draws;
        let mut state = self.state.try_borrow_mut()?;
        let mut caret = pointer.focused_caret(instance, &state);
        let mut next_frame = false;
        for id in pointer.frame_listeners() {
            if instance.frame_handlers[id].is_enabled(&state)? {
                next_frame = true;
                break;
            }
        }
        let previous_peak = cache.previous_peak.get();
        let mut draws = Vec::with_capacity(previous_peak.unwrap_or_default());
        let mut groups = PaintGroups {
            events: pointer.group_events(),
            stack: Vec::new(),
            peak: 0,
        };
        if let Some(commands) = cache.commands.get() {
            let mut labels = commands.labels.iter().peekable();
            let mut backgrounds = commands.backgrounds.iter().peekable();
            for (index, (id, offset, clips)) in pointer.paint_order().enumerate() {
                groups.advance(index, instance, &mut draws);
                let start = draws.len();
                if let Some(label) = labels.next_if(|label| label.index == index) {
                    draws.push(label.draw());
                } else if let Some((_, draw)) =
                    backgrounds.next_if(|(position, _)| *position == index)
                {
                    draws.push(draw.clone());
                } else if !instance.replay_commands(id, &mut draws) {
                    next_frame |= instance.bodies[&id]
                        .record_at(offset, clips, instance, &mut state, &mut draws)?;
                }
                if let Some(caret) = &mut caret {
                    caret.record(id, &mut draws, start);
                }
            }
        } else if previous_peak.is_some() {
            let mut fixed = FixedCommands::default();
            let mut prefix = None;
            let mut suffix = 0;
            for (index, (id, offset, clips)) in pointer.paint_order().enumerate() {
                groups.advance(index, instance, &mut draws);
                let start = draws.len();
                let body = &instance.bodies[&id];
                next_frame |= body.paint_at(offset, clips, instance, &mut state, &mut draws)?;
                if body.frame_paint == FramePaint::FixedLabel
                    && let Some(label) = FixedLabel::from_commands(index, &draws[start..])
                {
                    fixed.labels.push(label);
                    suffix += 1;
                } else if body.frame_paint == FramePaint::FixedBackground
                    && let [draw] = &draws[start..]
                {
                    fixed.backgrounds.push((index, draw.clone()));
                    suffix += 1;
                } else {
                    prefix.get_or_insert(index);
                    suffix = 0;
                }
                if let Some(caret) = &mut caret {
                    caret.record(id, &mut draws, start);
                }
            }
            if pointer.group_events().is_empty() {
                let edges = FixedEdges {
                    prefix: prefix.unwrap_or(suffix),
                    suffix: if prefix.is_some() { suffix } else { 0 },
                };
                if edges.prefix != 0 || edges.suffix != 0 {
                    fixed.edges = Some(Rc::new(edges));
                }
            }
            cache.commands.get_or_init(|| fixed);
        } else {
            // Keep one-off layouts allocation-free with respect to the fixed draw cache.
            for (index, (id, offset, clips)) in pointer.paint_order().enumerate() {
                groups.advance(index, instance, &mut draws);
                let start = draws.len();
                next_frame |= instance.bodies[&id]
                    .paint_at(offset, clips, instance, &mut state, &mut draws)?;
                if let Some(caret) = &mut caret {
                    caret.record(id, &mut draws, start);
                }
            }
        }
        groups.advance(usize::MAX, instance, &mut draws);
        let content_len = draws.len();
        instance
            .pointer
            .tooltips
            .paint(&pointer, instance, &self.engine, &mut draws)?;
        cache.previous_peak.set(Some(groups.peak.max(draws.len())));
        instance.repaints.borrow_mut().dirty.clear();
        Ok(ViewFrame {
            fixed_edges: if pointer.group_events().is_empty() {
                cache
                    .commands
                    .get()
                    .and_then(|commands| commands.edges.clone())
            } else {
                None
            },
            caret: caret.and_then(|c| c.bounds),
            has_overlay: draws.len() != content_len,
            draws,
            pointer,
            next_frame,
        })
    }
}
