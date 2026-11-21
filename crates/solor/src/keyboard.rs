//! Layout-resolved keyboard input delivered to a view's focused element.

use crate::{Context, Ui, ViewId, tree::TreeError};
use anyhow::Result;

pub use crate::pointer::Modifiers;
pub use keyboard_types::{Key, KeyState, NamedKey};

/// A logical key transition as interpreted by the active keyboard layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    /// Character or named key produced by the current layout and modifiers.
    pub key: Key,
    /// Whether the key was pressed or released.
    pub state: KeyState,
    /// Effective modifiers when the host received this transition.
    pub modifiers: Modifiers,
}

pub(crate) type KeyHandler = Box<dyn Fn(&KeyEvent, &mut Context<'_>) -> Result<()>>;

impl Ui {
    /// Delivers a key transition directly to the currently focused element.
    /// Returns true when the corresponding callback ran.
    pub fn dispatch_key(&mut self, view: ViewId, event: &KeyEvent) -> Result<bool> {
        let instance = self
            .instances
            .get_mut(&view)
            .ok_or(TreeError::InvalidNode(view.0))?;
        let Some(node) = instance.focused.and_then(|id| instance.bodies.get_mut(&id)) else {
            return Ok(false);
        };
        let handler = node
            .handlers
            .as_deref()
            .and_then(|handlers| match event.state {
                KeyState::Down => handlers.key_down.as_ref(),
                KeyState::Up => handlers.key_up.as_ref(),
            });
        let Some(handler) = handler else {
            return Ok(false);
        };
        handler(
            event,
            &mut Context {
                state: self.state.get_mut(),
                engine: &self.engine,
                owned: Some(&mut node.owned),
            },
        )?;
        Ok(true)
    }
}
