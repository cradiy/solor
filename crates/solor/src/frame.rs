//! Immutable output of recording a mounted view.
use std::{
    cell::{Cell, OnceCell},
    rc::Rc,
    sync::Arc,
};

use anyhow::{Context as _, Result};

use crate::{
    ContentClip, Draw, Instance, Rgba, Ui, ViewId,
    clip::NodeClips,
    element::MountedNode,
    layout::{Origin, Size},
    pointer::PointerFrame,
    state::Runtime,
    text::Paragraph,
};

#[derive(Default)]
pub(super) struct FixedLabels {
    recorded: Cell<bool>,
    labels: OnceCell<Vec<FixedLabel>>,
}

struct FixedLabel {
    clip: Option<Arc<ContentClip>>,
    index: usize,
    origin: Origin,
    size: Size,
    paragraph: Rc<Paragraph>,
    color: Rgba,
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
            color: *color,
        })
    }

    fn draw(&self) -> Draw {
        Draw::Text {
            clip: self.clip.clone(),
            origin: self.origin,
            size: self.size,
            paragraph: self.paragraph.clone(),
            color: self.color,
        }
    }
}

/// Commands in view-local logical coordinates and the matching pointer geometry.
pub struct ViewFrame {
    draws: Vec<Draw>,
    pointer: PointerFrame,
    next_frame: bool,
}

impl ViewFrame {
    /// Returns commands in paint order, with each parent before its children.
    pub fn draws(&self) -> &[Draw] {
        &self.draws
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
        if !self.fixed_label
            && let Some(saved) = instance.repaints.borrow().commands.get(&self.id)
        {
            draws.extend_from_slice(saved);
            return Ok(false);
        }
        let start = draws.len();
        let next_frame = self.paint(
            &instance.scene,
            state,
            &instance.paint_reads,
            Some((clips, offset)),
            draws,
        )?;
        for draw in &mut draws[start..] {
            let origin = match draw {
                Draw::Fill { origin, .. }
                | Draw::Polygon { origin, .. }
                | Draw::Polyline { origin, .. }
                | Draw::Text { origin, .. } => origin,
            };
            *origin = offset
                .checked_add(*origin)
                .context("paint origin outside layout range")?;
        }
        if !self.fixed_label {
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
        let cache = &instance.fixed_labels;
        let mut state = self.state.try_borrow_mut()?;
        let mut next_frame = false;
        for id in pointer.frame_listeners() {
            if instance.frame_handlers[id].is_enabled(&state)? {
                next_frame = true;
                break;
            }
        }
        let mut draws = Vec::new();
        if let Some(labels) = cache.labels.get() {
            draws.reserve(labels.len());
            let mut labels = labels.iter().peekable();
            for (index, (id, offset, clips)) in pointer.paint_order().enumerate() {
                if let Some(label) = labels.next_if(|label| label.index == index) {
                    draws.push(label.draw());
                } else {
                    next_frame |= instance.bodies[&id]
                        .paint_at(offset, clips, instance, &mut state, &mut draws)?;
                }
            }
        } else if cache.recorded.get() {
            let mut labels = Vec::new();
            for (index, (id, offset, clips)) in pointer.paint_order().enumerate() {
                let start = draws.len();
                let body = &instance.bodies[&id];
                next_frame |= body.paint_at(offset, clips, instance, &mut state, &mut draws)?;
                if body.fixed_label
                    && let Some(label) = FixedLabel::from_commands(index, &draws[start..])
                {
                    labels.push(label);
                }
            }
            cache.labels.get_or_init(|| labels);
        } else {
            // Keep one-off layouts allocation-free with respect to the label cache.
            for (id, offset, clips) in pointer.paint_order() {
                next_frame |= instance.bodies[&id]
                    .paint_at(offset, clips, instance, &mut state, &mut draws)?;
            }
            cache.recorded.set(true);
        }
        instance
            .pointer
            .tooltips
            .paint(&pointer, instance, &self.engine, &mut draws)?;
        instance.repaints.borrow_mut().dirty.clear();
        Ok(ViewFrame {
            draws,
            pointer,
            next_frame,
        })
    }
}
