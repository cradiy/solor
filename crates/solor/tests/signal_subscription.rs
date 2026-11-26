//! Application and component subscriptions to published signal values.
use anyhow::Result;
use solor::{Ui, component, div};
use std::{cell::RefCell, rc::Rc};

#[test]
fn subscribers_receive_batched_changes_and_can_cancel_pending_delivery() -> Result<()> {
    let mut ui = Ui::new();
    let value = ui.context().signal(0);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let out = seen.clone();
    let subscription = ui
        .context()
        .subscribe(value, move |n| out.borrow_mut().push(*n))?;

    ui.context().set(value, 0)?;
    ui.flush()?;
    assert!(seen.borrow().is_empty());
    ui.context().set(value, 1)?;
    ui.context().update(value, |n| *n += 1)?;
    ui.flush()?;
    ui.context().notify(value)?;
    ui.flush()?;
    ui.context().set(value, 3)?;
    ui.context().unsubscribe(subscription)?;
    ui.flush()?;
    assert_eq!(&*seen.borrow(), &[2, 2]);
    Ok(())
}

#[test]
fn unmount_releases_the_listener_on_a_borrowed_nonclone_signal() -> Result<()> {
    struct Model(u32);
    let mut ui = Ui::new();
    let model = ui.context().signal(Model(0));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let out = seen.clone();
    let view = ui.mount(component(move |cx| {
        cx.subscribe(model, move |model| out.borrow_mut().push(model.0))
            .unwrap();
        div()
    }))?;
    ui.context().update_in_place(model, |model| model.0 = 7)?;
    ui.flush()?;
    ui.context().update_in_place(model, |model| model.0 = 8)?;
    ui.unmount(view)?;
    assert_eq!(Rc::strong_count(&seen), 1);
    ui.flush()?;
    assert_eq!(&*seen.borrow(), &[7]);
    Ok(())
}
