use super::Shortcut;
use crate::{
    Context, Element, Instance, Node, SharedString, Tasks, TextSystem, Ui, ViewId, state::Runtime,
    tree::TreeError,
};
use anyhow::Result;
use smallvec::SmallVec;
use std::any::{Any, TypeId};

/// Associates a shortcut with an application-defined action value.
pub struct KeyBinding {
    shortcut: Shortcut,
    action: BoundAction,
}

impl KeyBinding {
    /// Binds one logical key or key combination to an action.
    pub fn new(shortcut: impl Into<Shortcut>, action: impl Any) -> Self {
        Self {
            shortcut: shortcut.into(),
            action: BoundAction {
                value: Box::new(action),
                context: None,
                sequence: 0,
            },
        }
    }

    /// Enables this binding when the focused element or an ancestor has this context name.
    pub fn in_context(mut self, name: impl Into<SharedString>) -> Self {
        self.action.context = Some(name.into());
        self
    }
}

pub(crate) struct BoundAction {
    pub(super) value: Box<dyn Any>,
    context: Option<SharedString>,
    sequence: u64,
}

impl Instance {
    fn find_on_focus_path<T>(
        &self,
        find: impl Fn(&[ActionHandler]) -> Option<T>,
    ) -> Result<Option<T>> {
        let mut target = self.focused;
        while let Some(id) = target {
            let node = self.bodies.get(&id).ok_or(TreeError::InvalidNode(id))?;
            if let Some(handlers) = node.handlers.as_deref()
                && let Some(found) = find(&handlers.actions)
            {
                return Ok(Some(found));
            }
            target = self.scene.parent(id)?;
        }
        Ok(None)
    }

    pub(super) fn route_action<'a>(
        &mut self,
        tasks: &Tasks,
        engine: &TextSystem,
        state: &mut Runtime<()>,
        request: &mut Option<crate::FocusHandle>,
        resolve: impl for<'h> Fn(&'h [ActionHandler]) -> Option<(&'h Callback, &'a dyn Any)>,
    ) -> Result<bool> {
        let mut target = self.focused;
        while let Some(id) = target {
            let node = self.bodies.get_mut(&id).ok_or(TreeError::InvalidNode(id))?;
            if let Some(handlers) = node.handlers.as_deref()
                && let Some((handler, action)) = resolve(&handlers.actions)
            {
                handler(
                    action,
                    &mut Context {
                        focus: Some(self.pointer.callback_focus(self.focused, request)),
                        tasks,
                        engine,
                        state,
                        owned: Some(&mut node.owned),
                    },
                )?;
                return Ok(true);
            }
            target = self.scene.parent(id)?;
        }
        Ok(false)
    }

    pub(super) fn key_contexts(
        &self,
        bindings: &[BoundAction],
    ) -> Result<SmallVec<[SharedString; 2]>> {
        let mut contexts = SmallVec::new();
        if bindings.iter().any(|binding| binding.context.is_some()) {
            let mut target = self.focused;
            while let Some(id) = target {
                if let Some(name) = self.bodies[&id]
                    .handlers
                    .as_ref()
                    .and_then(|h| h.key_context.as_ref())
                {
                    contexts.push(name.clone());
                }
                target = self.scene.parent(id)?;
            }
        }
        Ok(contexts)
    }
}

impl Ui {
    /// Whether the focused element or one of its ancestors handles this action type.
    pub fn is_action_available(&self, view: ViewId, action: &dyn Any) -> Result<bool> {
        let instance = self.instance(view)?;
        Ok(instance
            .find_on_focus_path(|handlers| ActionHandler::for_value(handlers, action).map(|_| ()))?
            .is_some())
    }

    /// Returns the most recently registered shortcut that resolves to this action value
    /// on the current focus path. Key-down handlers can still consume a press.
    pub fn shortcut_for_action<A: Any + PartialEq>(
        &self,
        view: ViewId,
        action: &A,
    ) -> Result<Option<&Shortcut>> {
        let instance = self.instance(view)?;
        let mut latest = None;
        for (shortcut, bindings) in &self.keymap {
            if !bindings
                .iter()
                .any(|b| b.value.downcast_ref::<A>() == Some(action))
            {
                continue;
            }
            let contexts = instance.key_contexts(bindings)?;
            let binding = instance.find_on_focus_path(|handlers| {
                ActionHandler::find(handlers, bindings, &contexts).map(|(_, binding)| binding)
            })?;
            if let Some(binding) = binding
                && binding.value.downcast_ref::<A>() == Some(action)
                && latest.is_none_or(|(sequence, _)| binding.sequence > sequence)
            {
                latest = Some((binding.sequence, shortcut));
            }
        }
        Ok(latest.map(|(_, shortcut)| shortcut))
    }

    /// Sends an action to the nearest matching handler on the view's focus path.
    /// Returns true if a handler ran.
    pub fn dispatch_action(&mut self, view: ViewId, action: &dyn Any) -> Result<bool> {
        let instance = self
            .instances
            .get_mut(&view)
            .ok_or(TreeError::InvalidNode(view.0))?;
        let mut request = None;
        let delivered = instance.route_action(
            &self.tasks,
            &self.engine,
            self.state.get_mut(),
            &mut request,
            |handlers| ActionHandler::for_value(handlers, action),
        )?;
        instance.apply_focus_request(request, self.state.get_mut(), &self.engine, &self.tasks);
        Ok(delivered)
    }

    /// Replaces all shortcut bindings for this UI in registration order.
    pub fn set_key_bindings(&mut self, bindings: impl IntoIterator<Item = KeyBinding>) {
        self.keymap.clear();
        self.binding_sequence = 0;
        self.bind_keys(bindings);
    }

    /// Registers bindings for this UI. Registering the same shortcut and action type
    /// again in the same context replaces its value.
    pub fn bind_keys(&mut self, bindings: impl IntoIterator<Item = KeyBinding>) {
        for KeyBinding {
            shortcut,
            mut action,
        } in bindings
        {
            self.binding_sequence += 1;
            action.sequence = self.binding_sequence;
            let actions = self.keymap.entry(shortcut).or_default();
            actions.retain(|old| {
                old.context != action.context
                    || old.value.as_ref().type_id() != action.value.as_ref().type_id()
            });
            actions.push(action);
        }
    }
}

type Callback = Box<dyn Fn(&dyn Any, &mut Context<'_>) -> Result<()>>;

pub(crate) struct ActionHandler {
    action: TypeId,
    callback: Callback,
}

impl ActionHandler {
    fn for_value<'h, 'a>(
        handlers: &'h [Self],
        action: &'a dyn Any,
    ) -> Option<(&'h Callback, &'a dyn Any)> {
        let handler = handlers.iter().find(|h| h.action == action.type_id())?;
        Some((&handler.callback, action))
    }

    pub(super) fn find<'h, 'a>(
        handlers: &'h [Self],
        actions: &'a [BoundAction],
        contexts: &[SharedString],
    ) -> Option<(&'h Callback, &'a BoundAction)> {
        if handlers.is_empty() {
            return None;
        }
        // At a given handler, try contexts from the focus outwards, then global bindings.
        for context in contexts.iter().map(Some).chain(std::iter::once(None)) {
            for binding in actions
                .iter()
                .rev()
                .filter(|b| b.context.as_ref() == context)
            {
                if let Some((handler, _)) = Self::for_value(handlers, binding.value.as_ref()) {
                    return Some((handler, binding));
                }
            }
        }
        None
    }
}

impl<E: Element> Node<E> {
    /// Names the shortcut context shared by this element and its focused descendants.
    pub fn key_context(mut self, name: impl Into<SharedString>) -> Self {
        self.handlers
            .get_or_insert_with(Default::default)
            .key_context = Some(name.into());
        self
    }

    /// Handles an action while focus is on this element or a descendant.
    /// The nearest handler on the focus path receives the action.
    pub fn on_action<A: Any>(
        mut self,
        handler: impl Fn(&A, &mut Context<'_>) -> Result<()> + 'static,
    ) -> Self {
        let handlers = &mut self.handlers.get_or_insert_with(Default::default).actions;
        let action = TypeId::of::<A>();
        handlers.retain(|h| h.action != action);
        handlers.push(ActionHandler {
            action,
            callback: Box::new(move |value, cx| {
                handler(value.downcast_ref().expect("matched action type"), cx)
            }),
        });
        self
    }
}
