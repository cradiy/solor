use super::Shortcut;
use crate::{Context, Element, Node, Ui};
use anyhow::Result;
use std::any::{Any, TypeId};

/// Associates a shortcut with an application-defined action value.
pub struct KeyBinding {
    shortcut: Shortcut,
    action: Box<dyn Any>,
}

impl KeyBinding {
    /// Binds one logical key or key combination to an action.
    pub fn new(shortcut: impl Into<Shortcut>, action: impl Any) -> Self {
        Self {
            shortcut: shortcut.into(),
            action: Box::new(action),
        }
    }
}

impl Ui {
    /// Registers bindings for this UI. Registering the same shortcut and action type
    /// again replaces its value. Later bindings take priority on the same element.
    pub fn bind_keys(&mut self, bindings: impl IntoIterator<Item = KeyBinding>) {
        for KeyBinding { shortcut, action } in bindings {
            let actions = self.keymap.entry(shortcut).or_default();
            actions.retain(|old| old.as_ref().type_id() != action.as_ref().type_id());
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
        actions: &'a [Box<dyn Any>],
    ) -> Option<(&'a Callback, &'a dyn Any)> {
        actions.iter().rev().find_map(|action| {
            let value = action.as_ref();
            let handler = handlers.iter().find(|h| h.action == value.type_id())?;
            Some((&handler.callback, value))
        })
    }
}

impl<E: Element> Node<E> {
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
