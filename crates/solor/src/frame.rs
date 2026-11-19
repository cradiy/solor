//! Immutable output of recording a mounted view.
use std::sync::Arc;

use anyhow::{Context as _, Result};

use crate::{Draw, Ui, ViewId, pointer::PointerFrame};

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

    /// Takes the hit geometry for acceptance after successful presentation.
    pub fn into_pointer_frame(self) -> PointerFrame {
        self.pointer
    }
}

impl Ui {
    /// Records the whole laid-out view. Repaint requests are acknowledged together
    /// after successful recording. Input continues to use the last accepted frame.
    pub fn record_frame(&self, view: ViewId) -> Result<ViewFrame> {
        let instance = self.instance(view)?;
        let pointer = self.prepare_pointer_frame(view)?;
        let mut draws = Vec::new();
        for (id, offset) in pointer.paint_order() {
            let start = draws.len();
            instance.bodies[&id].paint(&instance.scene, &self.state, &mut draws)?;
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
