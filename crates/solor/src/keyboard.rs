//! Layout-resolved keyboard input delivered to a view's focused element.

use crate::{
    Context, Instance, NodeId, Ui, ViewId, state::Runtime, style::Property, tree::TreeError,
};
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

pub(super) struct TabOrder {
    stops: Vec<NodeId>,
    dynamic_indices: Vec<(Property<i32>, i32)>,
    cursor: usize,
}

impl TabOrder {
    fn collect(instance: &Instance, state: &Runtime<()>) -> Result<Self> {
        let mut pending = vec![instance.scene.root()];
        let mut stops = Vec::new();
        let mut dynamic_indices = Vec::new();
        while let Some(id) = pending.pop() {
            let body = &instance.bodies[&id];
            if body.visible {
                if body.focusable {
                    let mut index = 0;
                    if let Some(property) =
                        body.handlers.as_ref().and_then(|h| h.tab_index.as_ref())
                    {
                        index = property.read(state)?;
                        if property.is_signal() {
                            dynamic_indices.push((property.clone(), index));
                        }
                    }
                    stops.push((index, id));
                }
                pending.extend(instance.scene.children(id)?.rev());
            }
        }
        stops.sort_by_key(|&(index, _)| index);
        Ok(Self {
            stops: stops.into_iter().map(|(_, id)| id).collect(),
            dynamic_indices,
            cursor: 0,
        })
    }

    fn indices_changed(&self, state: &Runtime<()>) -> Result<bool> {
        for (property, index) in &self.dynamic_indices {
            if property.read(state)? != *index {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn step(
        &mut self,
        focused: Option<NodeId>,
        backwards: bool,
        eligible: impl Fn(NodeId) -> Result<bool>,
    ) -> Result<Option<NodeId>> {
        let Some(last) = self.stops.len().checked_sub(1) else {
            return Ok(None);
        };
        // Pointer and programmatic focus can reposition us between Tab presses.
        let current = focused.and_then(|id| {
            if self.stops[self.cursor] == id {
                Some(self.cursor)
            } else {
                self.stops.iter().position(|&stop| stop == id)
            }
        });
        let mut index = current.unwrap_or(if backwards { 0 } else { last });
        for _ in 0..self.stops.len() {
            index = match (backwards, index) {
                (true, 0) => last,
                (true, i) => i - 1,
                (false, i) if i < last => i + 1,
                _ => 0,
            };
            let id = self.stops[index];
            if eligible(id)? {
                self.cursor = index;
                return Ok(Some(id));
            }
        }
        Ok(None)
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
}

impl Ui {
    fn advance_focus(&mut self, view: ViewId, backwards: bool) -> Result<Option<NodeId>> {
        let instance = self
            .instances
            .get_mut(&view)
            .ok_or(TreeError::InvalidNode(view.0))?;
        let state = self.state.get_mut();
        if match &instance.tab_order {
            Some(order) => order.indices_changed(state)?,
            None => true,
        } {
            instance.tab_order = Some(TabOrder::collect(instance, state)?);
        }
        let order = instance.tab_order.as_mut().expect("collected tab order");
        let next = order.step(instance.focused, backwards, |id| {
            match instance.bodies[&id]
                .handlers
                .as_deref()
                .and_then(|h| h.tab_stop.as_ref())
            {
                Some(stop) => stop.read(state),
                None => Ok(true),
            }
        })?;
        if next.is_some() {
            instance.set_focus(next, true);
        }
        Ok(next)
    }

    /// Focuses the next eligible Tab stop, wrapping at the end of the view.
    pub fn focus_next(&mut self, view: ViewId) -> Result<Option<NodeId>> {
        self.advance_focus(view, false)
    }

    /// Focuses the previous eligible Tab stop, wrapping at the start of the view.
    pub fn focus_previous(&mut self, view: ViewId) -> Result<Option<NodeId>> {
        self.advance_focus(view, true)
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
            return Ok(self.advance_focus(view, mods.shift)?.is_some() || delivered);
        }
        Ok(delivered)
    }
}
