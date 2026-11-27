use super::*;
use solor::platform::{PlatformWindow, WindowState};

fn make_life(revisit: Rc<Cell<bool>>, parent: Weak<WindowLife>) -> Rc<WindowLife> {
    WindowLife::new(revisit, parent, false, Rc::new(|| {}))
}

#[test]
fn close_guard_and_idempotent_close() {
    let revisit = Rc::new(Cell::new(false));
    let life = make_life(revisit.clone(), Weak::new());
    let handle = life.handle();

    let allow = Rc::new(Cell::new(false));
    let allow_ref = allow.clone();
    handle.set_close_guard(move || allow_ref.get());

    assert!(handle.is_open());
    assert!(!life.request_close());
    assert!(handle.is_open());

    allow.set(true);
    assert!(life.request_close());
    assert!(!handle.is_open());
    assert!(revisit.get());

    // Repeated closes and requests after closing stay harmless.
    assert!(!life.request_close());
    handle.close();
    assert!(!handle.is_open());
}

#[test]
fn modal_blocks_parent_redirects_activation_and_cascades_close() {
    let revisit = Rc::new(Cell::new(false));
    let parent = make_life(revisit.clone(), Weak::new());
    let child = make_life(revisit.clone(), Rc::downgrade(&parent));
    child.attach_modal(&parent);

    assert!(parent.blocked());
    assert!(parent.take_pointer_cancel());
    assert!(!parent.take_pointer_cancel());
    assert!(!parent.request_close());

    parent.activate();
    assert!(!parent.take_activate_request());
    assert!(child.take_activate_request());

    parent.close();
    assert!(!parent.is_open());
    assert!(!child.is_open());
    assert!(!parent.blocked());
}

#[test]
fn state_listeners_deduplicate_and_unsubscribe() {
    let revisit = Rc::new(Cell::new(false));
    let life = make_life(revisit, Weak::new());
    let handle = life.handle();

    let observed_ref = Rc::new(std::cell::RefCell::new(Vec::new()));
    let observed_clone = observed_ref.clone();
    let sub = handle.observe_state(move |state| {
        observed_clone.borrow_mut().push(state);
    });

    let active = WindowState {
        active: true,
        maximized: false,
        fullscreen: false,
    };
    assert!(life.confirm_state(active));
    assert!(!life.confirm_state(active));
    assert_eq!(observed_ref.borrow().len(), 1);
    assert_eq!(handle.state(), Some(active));

    drop(sub);
    let fullscreen = WindowState {
        active: true,
        maximized: false,
        fullscreen: true,
    };
    assert!(life.confirm_state(fullscreen));
    assert_eq!(observed_ref.borrow().len(), 1);
}

#[test]
fn blur_region_updates_mark_revisit_and_drain_once() {
    let revisit = Rc::new(Cell::new(false));
    let life = make_life(revisit.clone(), Weak::new());
    let handle = life.handle();

    let rect = SurfaceRect::new(
        solor::px(12.0),
        solor::px(24.0),
        solor::px(120.0),
        solor::px(80.0),
    );
    handle.set_background_blur_region(&[rect]).unwrap();
    assert!(revisit.get());
    assert_eq!(life.take_blur_region(), Some(vec![rect]));
    assert_eq!(life.take_blur_region(), None);

    handle.set_background_blur_region(&[]).unwrap();
    assert_eq!(life.take_blur_region(), Some(Vec::new()));
}
