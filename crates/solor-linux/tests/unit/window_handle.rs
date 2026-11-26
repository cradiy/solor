use super::*;

#[test]
fn a_retained_handle_cannot_close_a_replacement_window() {
    let revisit = Rc::new(Cell::new(false));
    let create = || {
        let life = WindowLife::new(None, revisit.clone(), Weak::new(), None);
        let target = Rc::downgrade(&life);
        (life, WindowHandle::new(target))
    };
    let (original, handle) = create();
    let copy = handle.clone();

    original.close();
    assert!(!copy.is_open());
    assert!(revisit.replace(false));
    copy.close();
    assert!(!revisit.get());
    drop(original);

    let (replacement, current) = create();
    assert_eq!(handle, copy);
    assert_ne!(handle, current);
    handle.close();
    assert!(current.is_open());
    assert!(!revisit.get());

    drop(replacement);
    assert!(!current.is_open());
}

#[test]
fn modal_children_release_only_their_own_parent_and_close_with_it() {
    let main = open_window(None);
    let settings = open_window(Some(&main));
    let sibling = open_window(Some(&main));
    let confirmation = open_window(Some(&settings));
    assert!(main.blocked() && settings.blocked());
    confirmation.close();
    assert!(!settings.blocked() && main.blocked());
    drop(settings);
    assert!(main.blocked());
    sibling.close();
    assert!(!main.blocked());

    let settings = open_window(Some(&main));
    let confirmation = open_window(Some(&settings));
    main.close();
    assert!(!settings.is_open() && !confirmation.is_open());
}

fn open_window(parent: Option<&Rc<WindowLife>>) -> Rc<WindowLife> {
    let window = WindowLife::new(
        None,
        Rc::default(),
        parent.map(Rc::downgrade).unwrap_or_default(),
        None,
    );
    if let Some(parent) = parent {
        window.attach_modal(parent);
    }
    window
}

#[test]
fn activation_follows_the_newest_modal_branch_until_it_closes() {
    let main = open_window(None);
    let settings = open_window(Some(&main));
    let confirm = open_window(Some(&settings));
    let sibling = open_window(Some(&main));
    let target = || main.modal_activation_window().unwrap();

    assert!(Rc::ptr_eq(&target(), &sibling));
    sibling.close();
    assert!(Rc::ptr_eq(&target(), &confirm));
    confirm.close();
    assert!(Rc::ptr_eq(&target(), &settings));
    settings.close();
    assert!(main.modal_activation_window().is_none());
}

#[test]
fn window_observers_follow_the_mounted_titlebar() -> anyhow::Result<()> {
    use solor::{Ui, component, div};

    let window = open_window(None);
    let handle = window.handle();
    let mut ui = Ui::new();
    let mount = |ui: &mut Ui, events: Rc<RefCell<Vec<bool>>>| {
        let handle = handle.clone();
        ui.mount(component(move |cx| {
            let states = events.clone();
            handle
                .observe_state(move |state| states.borrow_mut().push(state.active))
                .bind_to(cx);
            handle
                .observe_client_decorations(move |enabled| events.borrow_mut().push(enabled))
                .bind_to(cx);
            div()
        }))
    };
    let previous = Rc::new(RefCell::new(Vec::new()));
    let first = mount(&mut ui, previous.clone())?;
    window.confirm_state(WindowState {
        active: true,
        ..Default::default()
    });
    window.set_client_decorations(true);
    ui.unmount(first)?;

    let current = Rc::new(RefCell::new(Vec::new()));
    mount(&mut ui, current.clone())?;
    window.confirm_state(WindowState::default());
    window.set_client_decorations(false);
    assert_eq!(*previous.borrow(), [true, true]);
    assert_eq!(*current.borrow(), [false, false]);
    Ok(())
}

#[test]
fn dropping_a_window_observer_during_dispatch_releases_its_callback() {
    let window = open_window(None);
    let handle = window.handle();
    let next = Rc::new(Cell::new(None));
    let _first = handle.observe_state({
        let next = next.clone();
        move |_| drop(next.take())
    });
    let calls = Rc::new(Cell::new(0));
    next.set(Some(handle.observe_state({
        let calls = calls.clone();
        move |_| calls.set(calls.get() + 1)
    })));
    window.confirm_state(WindowState::default());
    assert_eq!(calls.get(), 0);
    assert_eq!(Rc::strong_count(&calls), 1);
}
