use crate::{Context, Element, Instance, Node, state::Runtime};
use anyhow::{Context as _, Result};
use std::{
    num::NonZeroU64,
    sync::atomic::{AtomicU64, Ordering},
};

/// Identifies a focus target without retaining its mounted element.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FocusHandle(NonZeroU64);

impl FocusHandle {
    /// Creates an independent handle for use with `track_focus`.
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .expect("focus handle IDs exhausted");
        Self(NonZeroU64::new(id).expect("focus handle IDs start at one"))
    }
}

impl Default for FocusHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: Element> Node<E> {
    /// Runs when this element receives keyboard focus, including when its window regains it.
    pub fn on_focus(mut self, handler: impl Fn(&mut Context<'_>) -> Result<()> + 'static) -> Self {
        self.handlers.get_or_insert_with(Default::default).focus = Some(Box::new(handler));
        self
    }

    /// Runs when keyboard focus leaves this element or its window.
    pub fn on_blur(mut self, handler: impl Fn(&mut Context<'_>) -> Result<()> + 'static) -> Self {
        self.handlers.get_or_insert_with(Default::default).blur = Some(Box::new(handler));
        self
    }

    /// Associates this focusable element with a handle. A view can mount one target per handle.
    pub fn track_focus(mut self, handle: FocusHandle) -> Self {
        self.handlers
            .get_or_insert_with(Default::default)
            .focus_handle = Some(handle);
        self.focusable()
    }
}

pub(crate) struct CallbackFocus<'a> {
    pub(crate) request: &'a mut Option<FocusHandle>,
    pub(crate) target: Option<(crate::NodeId, &'a crate::pointer::PointerFrame)>,
}

impl Context<'_> {
    /// Returns the current focus target's border box in view-local logical pixels.
    /// Returns `None` outside a view callback or when the accepted frame has no box for that target.
    pub fn focused_bounds(&self) -> Option<crate::platform::SurfaceRect> {
        let (node, frame) = self.focus.as_ref()?.target?;
        frame.bounds(node)
    }

    /// Requests logical focus from a view callback. The last request in a dispatch takes
    /// effect if its target is still mounted and visible in that view.
    pub fn focus(&mut self, handle: FocusHandle) -> Result<()> {
        *self
            .focus
            .as_mut()
            .context("focus requires a view callback")?
            .request = Some(handle);
        Ok(())
    }
}

impl Instance {
    pub(super) fn focus_target(&self, handle: FocusHandle) -> Option<crate::NodeId> {
        let id = *self.focus_targets.get(&handle)?;
        self.bodies.get(&id)?.visible.then_some(id)
    }

    pub(crate) fn apply_focus_request(
        &mut self,
        request: Option<FocusHandle>,
        state: &mut Runtime<()>,
        engine: &crate::TextSystem,
        tasks: &crate::Tasks,
    ) -> bool {
        if let Some(id) = request.and_then(|handle| self.focus_target(handle)) {
            self.set_focus(Some(id), true, state, engine, tasks);
            return true;
        }
        false
    }
}
