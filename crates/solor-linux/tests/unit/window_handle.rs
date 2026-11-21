use super::*;

#[test]
fn a_retained_handle_cannot_close_a_replacement_window() {
    let revisit = Rc::new(Cell::new(false));
    let create = || {
        let life = Rc::new(WindowLife {
            open: Cell::new(true),
            revisit: revisit.clone(),
        });
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
