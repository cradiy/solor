use super::{Preedit, TextInputState};
use crate::{Instance, Signal, Ui, ViewId, state::Runtime};
use anyhow::Result;

/// Text edits delivered by the host to a view's focused input.
#[derive(Debug)]
pub enum TextInputEvent {
    /// Replace the committed selection and end composition.
    Commit(String),
    /// Remove UTF-8 bytes on either side of the selection.
    DeleteSurrounding {
        /// Bytes preceding the selection.
        before: usize,
        /// Bytes following the selection.
        after: usize,
    },
    /// Update provisional text. Empty text cancels composition.
    Preedit(Preedit),
}

impl Instance {
    pub(crate) fn text_input_signal(&self) -> Option<Signal<TextInputState>> {
        self.bodies
            .get(&self.focused?)?
            .handlers
            .as_ref()?
            .text_input
    }

    pub(crate) fn cancel_focused_preedit(&self, state: &mut Runtime<()>) {
        if let Some(input) = self.text_input_signal()
            && state.read(input).is_ok_and(|s| s.preedit().is_some())
        {
            state
                .update_in_place(input, TextInputState::cancel_preedit)
                .expect("borrowed live text input");
        }
    }
}

impl Ui {
    /// Identifies the uninterrupted focus session of the view's current text editor.
    pub fn text_input_session(&self, view: ViewId) -> Result<Option<u64>> {
        let instance = self.instance(view)?;
        Ok(instance
            .text_input_signal()
            .map(|_| instance.focus_generation))
    }

    /// Borrows the current editing state for a synchronous host query.
    pub fn focused_text_input(&mut self, view: ViewId) -> Result<Option<&TextInputState>> {
        let (instance, state) = self.instance_and_state(view)?;
        Ok(instance
            .text_input_signal()
            .map(|s| state.read(s))
            .transpose()?)
    }

    /// Applies an edit to the focused input. Returns false when the view has no input target.
    pub fn dispatch_text_input(&mut self, view: ViewId, event: TextInputEvent) -> Result<bool> {
        let (instance, state) = self.instance_and_state(view)?;
        let Some(input) = instance.text_input_signal() else {
            return Ok(false);
        };
        match event {
            TextInputEvent::DeleteSurrounding { before, after } => {
                state.update_in_place(input, |s| s.delete_surrounding(before, after))??
            }
            TextInputEvent::Commit(text) => state.update_in_place(input, |s| s.commit(&text))?,
            TextInputEvent::Preedit(preedit) => {
                if let Some(selection) = preedit.selection {
                    selection.validate(&preedit.text)?;
                }
                state.update_in_place(input, |s| s.replace_preedit(preedit))?;
            }
        }
        Ok(true)
    }
}
