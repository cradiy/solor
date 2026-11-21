use super::*;

#[test]
fn a_retained_handle_cannot_close_a_replacement_window() {
    let revisit = Rc::new(Cell::new(false));
    let create = || {
        let life = WindowLife::new(None, revisit.clone(), Weak::new());
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
    let create = |parent: Option<&Rc<WindowLife>>| {
        let window = WindowLife::new(
            None,
            Rc::default(),
            parent.map(Rc::downgrade).unwrap_or_default(),
        );
        if let Some(parent) = parent {
            window.attach_modal(parent);
        }
        window
    };
    let main = create(None);
    let settings = create(Some(&main));
    let sibling = create(Some(&main));
    let confirmation = create(Some(&settings));
    assert!(main.blocked() && settings.blocked());
    confirmation.close();
    assert!(!settings.blocked() && main.blocked());
    drop(settings);
    assert!(main.blocked());
    sibling.close();
    assert!(!main.blocked());

    let settings = create(Some(&main));
    let confirmation = create(Some(&settings));
    main.close();
    assert!(!settings.is_open() && !confirmation.is_open());
}
