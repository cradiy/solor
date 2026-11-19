//! Immutable output of recording a mounted view.
use std::{
    cell::{Cell, OnceCell},
    rc::Rc,
    sync::Arc,
};

use anyhow::{Context as _, Result};

use crate::{
    Draw, Rgba, Ui, ViewId,
    element::MountedNode,
    layout::{Origin, Scene, Size},
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
            index,
            origin: *origin,
            size: *size,
            paragraph: paragraph.clone(),
            color: *color,
        })
    }

    fn draw(&self) -> Draw {
        Draw::Text {
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
    font: Option<Arc<Vec<u8>>>,
    pointer: PointerFrame,
}

impl ViewFrame {
    /// Returns commands in paint order, with each parent before its children.
    pub fn draws(&self) -> &[Draw] {
        &self.draws
    }

    /// Borrows the face used to shape the view's text.
    pub fn font(&self) -> Option<&[u8]> {
        self.font.as_ref().map(|font| font.as_slice())
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
        scene: &Scene,
        state: &Runtime<()>,
        draws: &mut Vec<Draw>,
    ) -> Result<()> {
        let start = draws.len();
        self.paint(scene, state, draws)?;
        for draw in &mut draws[start..] {
            let origin = match draw {
                Draw::Fill { origin, .. } | Draw::Text { origin, .. } => origin,
            };
            origin.x = offset
                .x
                .checked_add(origin.x)
                .context("paint x outside layout range")?;
            origin.y = offset
                .y
                .checked_add(origin.y)
                .context("paint y outside layout range")?;
        }
        Ok(())
    }
}

impl Ui {
    /// Records the whole laid-out view. Repaint requests are acknowledged together
    /// after successful recording. Input continues to use the last accepted frame.
    pub fn record_frame(&self, view: ViewId) -> Result<ViewFrame> {
        let instance = self.instance(view)?;
        let pointer = self.prepare_pointer_frame(view)?;
        let cache = &instance.fixed_labels;
        let scene = &instance.scene;
        let state = &self.state;
        let mut draws = Vec::new();
        if let Some(labels) = cache.labels.get() {
            draws.reserve(labels.len());
            let mut labels = labels.iter().peekable();
            for (index, (id, offset)) in pointer.paint_order().enumerate() {
                if let Some(label) = labels.next_if(|label| label.index == index) {
                    draws.push(label.draw());
                } else {
                    instance.bodies[&id].paint_at(offset, scene, state, &mut draws)?;
                }
            }
        } else if cache.recorded.get() {
            let mut labels = Vec::new();
            for (index, (id, offset)) in pointer.paint_order().enumerate() {
                let start = draws.len();
                let body = &instance.bodies[&id];
                body.paint_at(offset, scene, state, &mut draws)?;
                if body.fixed_label
                    && let Some(label) = FixedLabel::from_commands(index, &draws[start..])
                {
                    labels.push(label);
                }
            }
            cache.labels.get_or_init(|| labels);
        } else {
            // Keep one-off layouts allocation-free with respect to the label cache.
            for (id, offset) in pointer.paint_order() {
                instance.bodies[&id].paint_at(offset, scene, state, &mut draws)?;
            }
            cache.recorded.set(true);
        }
        let font = self
            .engine
            .as_ref()
            .map(|engine| engine.borrow().bytes.clone());
        instance.repaints.borrow_mut().clear();
        Ok(ViewFrame {
            draws,
            font,
            pointer,
        })
    }
}
