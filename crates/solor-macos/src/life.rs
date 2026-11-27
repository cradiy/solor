//! Window lifecycle, modal hierarchy, and host-state listeners.
use solor::platform::{PlatformWindow, SurfaceRect, WindowHandle, WindowState, WindowSubscription};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

type Listener<T> = RefCell<Box<dyn FnMut(T)>>;
type ListenerList<T> = RefCell<Vec<Rc<Listener<T>>>>;

/// Subscriber list for UI-thread window state notifications.
#[derive(Default)]
pub struct WindowListeners<T>(std::cell::OnceCell<Rc<ListenerList<T>>>);

impl<T: Copy + 'static> WindowListeners<T> {
    /// Registers a listener and returns a subscription that unregisters on drop.
    pub fn observe(&self, listener: Box<dyn FnMut(T)>) -> WindowSubscription {
        let listeners = self.0.get_or_init(Rc::default);
        let callback = Rc::new(RefCell::new(listener));
        let target = Rc::downgrade(&callback);
        listeners.borrow_mut().push(callback);
        let list = Rc::downgrade(listeners);
        WindowSubscription::new(move || {
            if let Some(list) = list.upgrade() {
                let removed = {
                    let mut entries = list.borrow_mut();
                    entries
                        .iter()
                        .position(|entry| std::ptr::eq(Rc::as_ptr(entry), target.as_ptr()))
                        .map(|index| entries.remove(index))
                };
                drop(removed);
            }
        })
    }

    /// Invokes every live listener with the new value.
    pub fn notify(&self, value: T) {
        if let Some(list) = self.0.get() {
            let pending: smallvec::SmallVec<[_; 4]> =
                list.borrow().iter().map(Rc::downgrade).collect();
            for entry in pending {
                if let Some(listener) = entry.upgrade() {
                    listener.borrow_mut()(value);
                }
            }
        }
    }

    /// Drops all registered listeners.
    pub fn clear(&self) {
        if let Some(list) = self.0.get() {
            drop(list.take());
        }
    }
}

/// Shared UI-thread lifecycle and modal tracking for a macOS window.
pub struct WindowLife {
    open: Cell<bool>,
    should_close: RefCell<Option<Rc<dyn Fn() -> bool>>>,
    state: Cell<Option<WindowState>>,
    state_listeners: WindowListeners<WindowState>,
    client_decorations: Cell<bool>,
    decoration_listeners: WindowListeners<bool>,
    modal_parent: Weak<Self>,
    modal_children: RefCell<Vec<Weak<Self>>>,
    cancel_pointer: Cell<bool>,
    activate_requested: Cell<bool>,
    blur_region: RefCell<Option<Vec<SurfaceRect>>>,
    revisit: Rc<Cell<bool>>,
    on_wake: Rc<dyn Fn()>,
}

impl WindowLife {
    /// Creates a new open window lifecycle node linked to the event loop wakeup flag.
    pub fn new(
        revisit: Rc<Cell<bool>>,
        modal_parent: Weak<Self>,
        client_decorations: bool,
        on_wake: Rc<dyn Fn()>,
    ) -> Rc<Self> {
        Rc::new(Self {
            open: Cell::new(true),
            should_close: RefCell::default(),
            state: Cell::default(),
            state_listeners: WindowListeners::default(),
            client_decorations: Cell::new(client_decorations),
            decoration_listeners: WindowListeners::default(),
            modal_parent,
            modal_children: RefCell::default(),
            cancel_pointer: Cell::default(),
            activate_requested: Cell::default(),
            blur_region: RefCell::default(),
            revisit,
            on_wake,
        })
    }

    /// Returns a weak handle referencing this window.
    pub fn handle(self: &Rc<Self>) -> WindowHandle {
        let weak = Rc::downgrade(self);
        WindowHandle::new(weak)
    }

    /// Evaluates the close guard for a user-initiated close request and closes if permitted.
    pub fn request_close(&self) -> bool {
        if !self.is_open() || self.blocked() {
            return false;
        }
        let handler = self.should_close.borrow().clone();
        if handler.is_none_or(|allow| allow()) {
            self.close();
            return true;
        }
        false
    }

    /// Records the confirmed top-level state and notifies subscribers when it changed.
    pub fn confirm_state(&self, state: WindowState) -> bool {
        if self.state.replace(Some(state)) == Some(state) {
            return false;
        }
        self.state_listeners.notify(state);
        true
    }

    /// Updates whether client-side decorations are active and notifies subscribers on change.
    pub fn set_client_decorations(&self, enabled: bool) -> bool {
        if self.client_decorations.replace(enabled) == enabled {
            return false;
        }
        self.decoration_listeners.notify(enabled);
        self.mark_revisit();
        true
    }

    /// Links this window as a modal child of `parent`, cancelling active parent input.
    pub fn attach_modal(self: &Rc<Self>, parent: &Self) {
        if !parent.is_open() {
            self.close();
            return;
        }
        if self.is_open() {
            let mut children = parent.modal_children.borrow_mut();
            if children.is_empty() {
                parent.cancel_pointer.set(true);
            }
            children.push(Rc::downgrade(self));
            parent.mark_revisit();
        }
    }

    /// Returns the newest open modal descendant that should receive activation instead.
    pub fn modal_activation_target(&self) -> Option<Rc<Self>> {
        let mut window = self.modal_children.borrow().last()?.upgrade()?;
        loop {
            let next = window
                .modal_children
                .borrow()
                .last()
                .and_then(Weak::upgrade);
            match next {
                Some(child) => window = child,
                None => return Some(window),
            }
        }
    }

    /// Returns true while at least one modal child is open.
    pub fn blocked(&self) -> bool {
        !self.modal_children.borrow().is_empty()
    }

    /// Consumes a pending pointer cancellation caused by opening a modal child.
    pub fn take_pointer_cancel(&self) -> bool {
        self.cancel_pointer.replace(false)
    }

    /// Consumes a pending programmatic activation request.
    pub fn take_activate_request(&self) -> bool {
        self.activate_requested.replace(false)
    }

    /// Consumes pending background blur rectangles requested for this window.
    pub fn take_blur_region(&self) -> Option<Vec<SurfaceRect>> {
        self.blur_region.borrow_mut().take()
    }

    fn mark_revisit(&self) {
        self.revisit.set(true);
        (self.on_wake)();
    }
}

impl PlatformWindow for WindowLife {
    fn is_open(&self) -> bool {
        self.open.get()
    }

    fn activate(&self) {
        if let Some(modal) = self.modal_activation_target() {
            if modal.is_open() {
                modal.activate_requested.set(true);
                modal.mark_revisit();
            }
        } else if self.is_open() {
            self.activate_requested.set(true);
            self.mark_revisit();
        }
    }

    fn set_background_blur_region(&self, rects: &[SurfaceRect]) -> std::io::Result<()> {
        if self.is_open() {
            *self.blur_region.borrow_mut() = Some(rects.to_vec());
            self.mark_revisit();
        }
        Ok(())
    }

    fn client_decorations(&self) -> bool {
        self.client_decorations.get()
    }

    fn observe_client_decorations(&self, listener: Box<dyn FnMut(bool)>) -> WindowSubscription {
        self.decoration_listeners.observe(listener)
    }

    fn state(&self) -> Option<WindowState> {
        self.state.get()
    }

    fn observe_state(&self, listener: Box<dyn FnMut(WindowState)>) -> WindowSubscription {
        self.state_listeners.observe(listener)
    }

    fn set_close_guard(&self, handler: Rc<dyn Fn() -> bool>) {
        self.should_close.replace(Some(handler));
    }

    fn close(&self) {
        if self.open.replace(false) {
            self.state_listeners.clear();
            self.decoration_listeners.clear();
            if let Some(parent) = self.modal_parent.upgrade() {
                parent
                    .modal_children
                    .borrow_mut()
                    .retain(|child| !std::ptr::eq(child.as_ptr(), self));
            }
            let children = self.modal_children.take();
            for child in children.into_iter().filter_map(|child| child.upgrade()) {
                child.close();
            }
            self.mark_revisit();
        }
    }
}

impl Drop for WindowLife {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
#[path = "../tests/unit/life.rs"]
mod tests;
