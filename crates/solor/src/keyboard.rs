//! Layout-resolved keyboard input delivered to a view's focused element.

use crate::{Context, Instance, NodeId, Ui, ViewId, tree::TreeError};
use anyhow::Result;

pub use crate::pointer::Modifiers;
mod binding;
mod shortcut;
pub use binding::KeyBinding;
pub(crate) use binding::{ActionHandler, BoundAction};
pub use keyboard_types::{KeyState, NamedKey};
pub use shortcut::{Key, Modifier, Shortcut};

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

pub(crate) type KeyDownHandler = Box<dyn Fn(&KeyEvent, &mut KeyDownContext<'_>) -> Result<()>>;

/// State access and control over default behavior during a key press.
pub struct KeyDownContext<'a> {
    context: Context<'a>,
    prevented: bool,
}

impl KeyDownContext<'_> {
    /// Consumes this press before shortcut lookup and Tab navigation.
    pub fn prevent_default(&mut self) {
        self.prevented = true;
    }
}

impl_event_context!(KeyDownContext);

/// Built on first navigation after a topology or visibility change.
pub(super) struct TabOrder {
    stops: Vec<NodeId>,
    cursor: usize,
}

impl TabOrder {
    fn collect(instance: &Instance) -> Result<Self> {
        let mut pending = vec![instance.scene.root()];
        let mut stops = Vec::new();
        while let Some(id) = pending.pop() {
            let body = &instance.bodies[&id];
            if body.visible {
                if body.focusable {
                    stops.push(id);
                }
                pending.extend(instance.scene.children(id)?.rev());
            }
        }
        Ok(Self { stops, cursor: 0 })
    }

    fn step(&mut self, focused: Option<NodeId>, backwards: bool) -> Option<NodeId> {
        let last = self.stops.len().checked_sub(1)?;
        // Pointer and programmatic focus can reposition us between Tab presses.
        let current = focused.and_then(|id| {
            if self.stops[self.cursor] == id {
                Some(self.cursor)
            } else {
                self.stops.iter().position(|&stop| stop == id)
            }
        });
        self.cursor = match (current, backwards) {
            (None | Some(0), true) => last,
            (Some(i), true) => i - 1,
            (Some(i), false) if i < last => i + 1,
            _ => 0,
        };
        Some(self.stops[self.cursor])
    }
}

impl Instance {
    pub(super) fn set_focus(&mut self, next: Option<NodeId>, show_indicator: bool) {
        if self.focused != next
            && let Some(previous) = self.focused
        {
            self.set_focus_indicator(previous, false);
        }
        self.focused = next;
        if let Some(id) = next {
            self.set_focus_indicator(id, show_indicator);
        }
    }

    fn set_focus_indicator(&mut self, id: NodeId, visible: bool) {
        if let Some(indicator) = self
            .bodies
            .get_mut(&id)
            .and_then(|body| body.handlers.as_deref_mut())
            .and_then(|handlers| handlers.focus_indicator.as_mut())
            && indicator.visible != visible
        {
            indicator.visible = visible;
            self.repaints.borrow_mut().invalidate(id);
        }
    }

    fn advance_focus(&mut self, backwards: bool) -> Result<Option<NodeId>> {
        if self.tab_order.is_none() {
            self.tab_order = Some(TabOrder::collect(self)?);
        }
        let order = self.tab_order.as_mut().expect("collected tab order");
        let next = order.step(self.focused, backwards);
        self.set_focus(next, true);
        Ok(self.focused)
    }
}

impl Ui {
    /// Focuses the next visible, mounted focusable element, wrapping at the end of the view.
    pub fn focus_next(&mut self, view: ViewId) -> Result<Option<NodeId>> {
        self.instance_mut(view)?.advance_focus(false)
    }

    /// Focuses the previous visible, mounted focusable element, wrapping at the start of the view.
    pub fn focus_previous(&mut self, view: ViewId) -> Result<Option<NodeId>> {
        self.instance_mut(view)?.advance_focus(true)
    }

    /// Delivers the transition to the focused element, then resolves bound actions on
    /// its focus path. An unconsumed Tab or Shift+Tab press moves focus through the view.
    /// Returns true when a callback ran or Tab navigation found a focus target.
    pub fn dispatch_key(&mut self, view: ViewId, event: &KeyEvent) -> Result<bool> {
        let instance = self
            .instances
            .get_mut(&view)
            .ok_or(TreeError::InvalidNode(view.0))?;
        let mut delivered = false;
        if let Some(node) = instance.focused.and_then(|id| instance.bodies.get_mut(&id))
            && let Some(handlers) = node.handlers.as_deref()
        {
            let mut context = Context {
                tasks: &self.tasks,
                state: self.state.get_mut(),
                engine: &self.engine,
                owned: Some(&mut node.owned),
            };
            match event.state {
                KeyState::Down => {
                    if let Some(handler) = &handlers.key_down {
                        let mut cx = KeyDownContext {
                            context,
                            prevented: false,
                        };
                        handler(event, &mut cx)?;
                        if cx.prevented {
                            return Ok(true);
                        }
                        delivered = true;
                    }
                }
                KeyState::Up => {
                    if let Some(handler) = &handlers.key_up {
                        handler(event, &mut context)?;
                        delivered = true;
                    }
                }
            }
        }
        if event.state == KeyState::Down
            && !self.keymap.is_empty()
            && let Some(actions) = self.keymap.get(&Shortcut::from_event(event))
        {
            let contexts = instance.key_contexts(actions)?;
            let mut target = instance.focused;
            while let Some(id) = target {
                let node = instance
                    .bodies
                    .get_mut(&id)
                    .ok_or(TreeError::InvalidNode(id))?;
                if let Some(handlers) = node.handlers.as_deref()
                    && let Some((handler, action)) =
                        ActionHandler::find(&handlers.actions, actions, &contexts)
                {
                    handler(
                        action,
                        &mut Context {
                            tasks: &self.tasks,
                            state: self.state.get_mut(),
                            engine: &self.engine,
                            owned: Some(&mut node.owned),
                        },
                    )?;
                    return Ok(true);
                }
                target = instance.scene.parent(id)?;
            }
        }
        let mods = event.modifiers;
        if event.state == KeyState::Down
            && event.key == Key::Tab
            && !mods.alt
            && !mods.meta
            && !mods.control
        {
            return Ok(instance.advance_focus(mods.shift)?.is_some() || delivered);
        }
        Ok(delivered)
    }
}
