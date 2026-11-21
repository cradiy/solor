use super::Shortcut;
use crate::{Context, Element, Instance, Node, SharedString, Ui};
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
    value: Box<dyn Any>,
    context: Option<SharedString>,
}

impl Instance {
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
    /// Registers bindings for this UI. Registering the same shortcut and action type
    /// again in the same context replaces its value.
    pub fn bind_keys(&mut self, bindings: impl IntoIterator<Item = KeyBinding>) {
        for KeyBinding { shortcut, action } in bindings {
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
    pub(super) fn find<'a>(
        handlers: &'a [Self],
        actions: &'a [BoundAction],
        contexts: &[SharedString],
    ) -> Option<(&'a Callback, &'a dyn Any)> {
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
                let value = binding.value.as_ref();
                if let Some(handler) = handlers.iter().find(|h| h.action == value.type_id()) {
                    return Some((&handler.callback, value));
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
