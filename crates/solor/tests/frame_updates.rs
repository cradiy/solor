//! Host frame opportunities belong to visible nodes in the accepted snapshot.
use anyhow::Result;
use solor::{Styled, Ui, component, div, layout::Size, px};
use std::{cell::Cell, rc::Rc, time::Duration};

#[test]
fn frame_listeners_require_acceptance_and_retire_with_the_mount() -> Result<()> {
    let calls = Rc::new(Cell::new(0));
    let mut ui = Ui::new();
    let open = ui.context().signal(true);
    let visible = ui.context().signal(true);
    let view = ui.mount(component(|_| {
        div().child_when(open, {
            let calls = calls.clone();
            move || {
                let calls = calls.clone();
                div().visible_when(visible).on_frame(move |_, _| {
                    calls.set(calls.get() + 1);
                    Ok(())
                })
            }
        })
    }))?;
    let accept = |ui: &mut Ui, active| -> Result<()> {
        ui.flush()?;
        ui.layout(
            view,
            Size {
                width: px(40.0),
                height: px(40.0),
            },
        )?;
        let frame = ui.record_frame(view)?;
        assert_eq!(frame.requests_animation_frame(), active);
        ui.accept_pointer_frame(frame.into_pointer_frame())
    };
    ui.dispatch_frame(view, Duration::ZERO)?;
    assert_eq!(calls.get(), 0);
    accept(&mut ui, true)?;
    ui.dispatch_frame(view, Duration::ZERO)?;
    assert_eq!(calls.get(), 1);
    ui.context().set(visible, false)?;
    accept(&mut ui, false)?;
    ui.dispatch_frame(view, Duration::ZERO)?;
    assert_eq!(calls.get(), 1);
    ui.context().set(visible, true)?;
    accept(&mut ui, true)?;
    ui.context().set(open, false)?;
    ui.flush()?;
    ui.context().set(open, true)?;
    ui.flush()?;
    ui.dispatch_frame(view, Duration::ZERO)?;
    assert_eq!(calls.get(), 1);
    accept(&mut ui, true)?;
    ui.dispatch_frame(view, Duration::ZERO)?;
    assert_eq!(calls.get(), 2);
    Ok(())
}

#[test]
fn frame_gate_wakes_an_idle_view_and_preserves_the_callback_across_pauses() -> Result<()> {
    let mut ui = Ui::new();
    let running = ui.context().signal(false);
    let count = ui.context().signal(0);
    let view = ui.mount(component(move |_| {
        let mut ticks = 0;
        div().on_frame_when(running, move |_, cx| {
            ticks += 1;
            cx.set(count, ticks)
        })
    }))?;
    let size = Size {
        width: px(40.0),
        height: px(40.0),
    };
    ui.flush()?;
    ui.layout(view, size)?;
    let submit = |ui: &mut Ui, running| -> Result<()> {
        let frame = ui.record_frame(view)?;
        assert_eq!(frame.requests_animation_frame(), running);
        ui.accept_pointer_frame(frame.into_pointer_frame())
    };
    submit(&mut ui, false)?;
    for expected in 1..=2 {
        ui.context().set(running, true)?;
        assert!(ui.has_pending_bindings());
        ui.flush()?;
        assert_eq!(ui.layout(view, size)?, 0);
        submit(&mut ui, true)?;
        ui.dispatch_frame(view, Duration::ZERO)?;
        assert_eq!(*ui.context().read(count)?, expected);

        ui.context().set(running, false)?;
        // An opportunity from the last submission can arrive before the pause is presented.
        ui.dispatch_frame(view, Duration::ZERO)?;
        assert_eq!(*ui.context().read(count)?, expected);
        submit(&mut ui, false)?;
    }
    ui.unmount(view)?;
    ui.context().set(running, true)?;
    assert!(!ui.has_pending_bindings());
    Ok(())
}
