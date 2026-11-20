//! Input against host-accepted frames and the live mounted tree.
use std::{cell::Cell, rc::Rc};

use anyhow::Result;
use solor::{
    Styled, Ui, ViewId, component, div,
    layout::{Origin, Size},
    pointer::{
        PointerButton::{Primary, Secondary},
        PointerId,
        PointerInput::{Cancel, Down, Leave, Move, Up},
        PointerKind, PointerSample,
    },
    px, text,
};

fn sample(id: u64, x: f32, y: f32) -> PointerSample {
    PointerSample {
        id: PointerId(id),
        kind: PointerKind::Mouse,
        position: Origin { x: px(x), y: px(y) },
        modifiers: Default::default(),
    }
}

fn accept(ui: &mut Ui, view: ViewId, width: f32) -> Result<()> {
    ui.flush()?;
    ui.layout(
        view,
        Size {
            width: px(width),
            height: px(40.0),
        },
    )?;
    let frame = ui.prepare_pointer_frame(view)?;
    ui.accept_pointer_frame(frame)
}

#[test]
fn label_hits_button_up_precedes_click_and_bindings_wait_for_flush() -> Result<()> {
    let mut ui = Ui::with_font(
        include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf").to_vec(),
        "en",
    )?;
    let visible = ui.context().signal(false);
    let released = ui.context().signal(false);
    let view = ui.mount(component(|_| {
        div()
            .child(
                div()
                    .w(px(40.0))
                    .h(px(20.0))
                    .child(text("A"))
                    .on_pointer_up(move |_, cx| cx.set(released, true))
                    .on_click(move |cx| {
                        assert!(*cx.read(released)?);
                        cx.set(visible, true)
                    }),
            )
            .child_when(visible, div)
    }))?;
    let root = ui.root(view)?;
    let inside = sample(1, 1.0, 1.0);
    let outside = sample(1, 45.0, 1.0);
    accept(&mut ui, view, 60.0)?;
    ui.dispatch_pointer(view, Down(inside, Primary))?;
    ui.dispatch_pointer(view, Move(outside))?;
    ui.dispatch_pointer(view, Move(inside))?;
    ui.dispatch_pointer(view, Up(inside, Primary))?;
    assert!(!*ui.context().read(visible)?);
    ui.context().set(released, false)?;
    ui.dispatch_pointer(view, Down(inside, Primary))?;
    ui.dispatch_pointer(view, Up(sample(2, 1.0, 1.0), Primary))?;
    assert!(!*ui.context().read(released)?);
    ui.dispatch_pointer(view, Up(inside, Primary))?;
    assert!(*ui.context().read(visible)?);
    assert_eq!(ui.children(view, root)?.len(), 1);
    ui.flush()?;
    assert_eq!(ui.children(view, root)?.len(), 2);
    Ok(())
}

#[test]
fn capture_uses_accepted_bounds_and_ends_on_release_error_or_cancel() -> Result<()> {
    let mut ui = Ui::new();
    let owner = ui.context().signal(None);
    let value = ui.context().signal(0.0);
    let canceled = ui.context().signal(0);
    let view = ui.mount(component(|_| {
        div().p(px(5.0)).child(
            div()
                .w_full()
                .h(px(10.0))
                .on_pointer_down(move |event, cx| {
                    if event.is_primary_action() && cx.read(owner)?.is_none() {
                        cx.capture_pointer();
                        cx.set(owner, Some(event.pointer_id()))?;
                    }
                    Ok(())
                })
                .on_pointer_move(move |event, cx| {
                    if *cx.read(owner)? == Some(event.pointer_id()) {
                        cx.set(
                            value,
                            event.local_position().x.get() / event.bounds().width.get(),
                        )?;
                    }
                    Ok(())
                })
                .on_pointer_up(move |event, cx| {
                    if event.is_primary_action() {
                        cx.set(owner, None)?;
                        anyhow::bail!("release failed");
                    }
                    Ok(())
                })
                .on_pointer_cancel(move |_, cx| {
                    cx.set(owner, None)?;
                    cx.update(canceled, |v| *v += 1)
                })
                .on_click(|_| panic!("captured press activated")),
        )
    }))?;
    accept(&mut ui, view, 50.0)?;
    let original = ui.prepare_pointer_frame(view)?;
    let inside = sample(1, 10.0, 8.0);
    let outside = sample(1, 65.0, 25.0);
    ui.dispatch_pointer(view, Down(inside, Primary))?;
    ui.dispatch_pointer(view, Leave(PointerId(1)))?;
    ui.dispatch_pointer(view, Move(outside))?;
    assert_eq!(*ui.context().read(value)?, 1.5);
    ui.dispatch_pointer(view, Down(sample(2, 10.0, 8.0), Primary))?;
    ui.dispatch_pointer(view, Move(sample(2, 25.0, 8.0)))?;
    assert_eq!(*ui.context().read(value)?, 1.5);
    ui.layout(
        view,
        Size {
            width: px(90.0),
            height: px(40.0),
        },
    )?;
    let resized = ui.prepare_pointer_frame(view)?;
    ui.dispatch_pointer(view, Move(outside))?;
    assert_eq!(*ui.context().read(value)?, 1.5);
    ui.accept_pointer_frame(resized)?;
    ui.dispatch_pointer(view, Move(outside))?;
    assert_eq!(*ui.context().read(value)?, 0.75);
    // A delayed submission may present the old geometry after the resize.
    ui.accept_pointer_frame(original)?;
    ui.dispatch_pointer(view, Move(outside))?;
    assert_eq!(*ui.context().read(value)?, 1.5);
    let current = ui.record_frame(view)?.into_pointer_frame();
    ui.accept_pointer_frame(current)?;
    ui.dispatch_pointer(view, Move(outside))?;
    assert_eq!(*ui.context().read(value)?, 0.75);
    ui.dispatch_pointer(view, Down(outside, Secondary))?;
    ui.dispatch_pointer(view, Up(outside, Secondary))?;
    assert!(ui.dispatch_pointer(view, Up(outside, Primary)).is_err());
    ui.dispatch_pointer(view, Cancel(PointerId(1)))?;
    assert_eq!(*ui.context().read(canceled)?, 0);
    ui.dispatch_pointer(view, Down(inside, Primary))?;
    ui.dispatch_pointer(view, Cancel(PointerId(1)))?;
    ui.dispatch_pointer(view, Cancel(PointerId(1)))?;
    assert_eq!(*ui.context().read(canceled)?, 1);
    Ok(())
}

#[test]
fn retired_press_and_stale_frontmost_hit_never_retarget() -> Result<()> {
    let mut ui = Ui::new();
    let open = ui.context().signal(true);
    let behind = ui.context().signal(0);
    let downs = Rc::new(Cell::new(0));
    let ups = Rc::new(Cell::new(0));
    let view = ui.mount(component(|_| {
        let downs = downs.clone();
        let ups = ups.clone();
        div()
            .child(
                div()
                    .size(px(20.0))
                    .on_click(move |cx| cx.update(behind, |v| *v += 1)),
            )
            .child_when(open, move || {
                let downs = downs.clone();
                let ups = ups.clone();
                div()
                    .size(px(20.0))
                    .mt(px(-20.0))
                    .on_pointer_down(move |_, cx| {
                        cx.capture_pointer();
                        downs.set(downs.get() + 1);
                        Ok(())
                    })
                    .on_pointer_up(move |_, _| {
                        ups.set(ups.get() + 1);
                        Ok(())
                    })
            })
    }))?;
    accept(&mut ui, view, 50.0)?;
    let p = sample(1, 1.0, 1.0);
    ui.dispatch_pointer(view, Down(p, Primary))?;
    assert_eq!(downs.get(), 1);
    ui.context().set(open, false)?;
    ui.flush()?;
    assert!(ui.prepare_pointer_frame(view).is_err());
    ui.layout(
        view,
        Size {
            width: px(50.0),
            height: px(40.0),
        },
    )?;
    // Simulate a failed host submission by discarding its input geometry.
    drop(ui.prepare_pointer_frame(view)?);
    ui.dispatch_pointer(view, Up(p, Primary))?;
    ui.dispatch_pointer(view, Down(p, Primary))?;
    ui.dispatch_pointer(view, Up(p, Primary))?;
    assert_eq!(*ui.context().read(behind)?, 0);
    let uncovered = ui.record_frame(view)?.into_pointer_frame();
    ui.accept_pointer_frame(uncovered)?;
    ui.dispatch_pointer(view, Down(p, Primary))?;
    ui.dispatch_pointer(view, Up(p, Primary))?;
    assert_eq!(*ui.context().read(behind)?, 1);
    ui.context().set(open, true)?;
    accept(&mut ui, view, 50.0)?;
    ui.dispatch_pointer(view, Up(p, Primary))?;
    assert_eq!(ups.get(), 0);
    ui.dispatch_pointer(view, Down(p, Primary))?;
    ui.dispatch_pointer(view, Up(p, Primary))?;
    assert_eq!((downs.get(), ups.get()), (2, 1));
    Ok(())
}

#[test]
fn deactivation_cancels_all_contacts_even_on_error_and_is_scoped_to_one_view() -> Result<()> {
    let mut ui = Ui::new();
    let cancels = ui.context().signal(0);
    let releases = ui.context().signal(0);
    let mount = || {
        component(move |_| {
            div()
                .on_pointer_down(|_, cx| {
                    cx.capture_pointer();
                    Ok(())
                })
                .on_pointer_up(move |_, cx| cx.update(releases, |v| *v += 1))
                .on_pointer_cancel(move |id, cx| {
                    cx.update(cancels, |v| *v += 1)?;
                    anyhow::ensure!(id != PointerId(2), "cancel failed");
                    Ok(())
                })
        })
    };
    let a = ui.mount(mount())?;
    let b = ui.mount(mount())?;
    for view in [a, b] {
        accept(&mut ui, view, 50.0)?;
        ui.dispatch_pointer(view, Down(sample(1, 5.0, 5.0), Primary))?;
    }
    ui.dispatch_pointer(a, Down(sample(2, 5.0, 5.0), Primary))?;
    ui.dispatch_pointer(a, Down(sample(2, 5.0, 5.0), Secondary))?;
    assert!(ui.cancel_pointers(a).is_err());
    ui.cancel_pointers(a)?;
    assert_eq!(*ui.context().read(cancels)?, 2);
    for id in [1, 2] {
        ui.dispatch_pointer(a, Up(sample(id, 5.0, 5.0), Primary))?;
    }
    ui.dispatch_pointer(b, Up(sample(1, 60.0, 5.0), Primary))?;
    assert_eq!(*ui.context().read(releases)?, 1);
    Ok(())
}

#[test]
fn relative_subtree_draws_and_clicks_follow_the_accepted_frame_across_resize() -> Result<()> {
    use solor::{Draw, Rgba, percent};

    let mut ui = Ui::new();
    let clicks = ui.context().signal(0);
    let view = ui.mount(component(|_| {
        div().child(
            div()
                .relative()
                .left(percent(25.0))
                .size(px(20.0))
                .p(px(2.0))
                .child(div().size(px(6.0)).bg(Rgba::new(20, 90, 200, 255)))
                .on_click(move |cx| cx.update(clicks, |count| *count += 1)),
        )
    }))?;
    let record = |ui: &mut Ui, width, expected_x| -> Result<_> {
        ui.layout(
            view,
            Size {
                width: px(width),
                height: px(40.0),
            },
        )?;
        let frame = ui.record_frame(view)?;
        let [Draw::Fill { origin, .. }] = frame.draws() else {
            panic!("expected one fill")
        };
        assert_eq!(
            *origin,
            Origin {
                x: px(expected_x),
                y: px(2.0)
            }
        );
        Ok(frame.into_pointer_frame())
    };
    let click = |ui: &mut Ui, x| -> Result<()> {
        let p = sample(1, x, 3.0);
        ui.dispatch_pointer(view, Down(p, Primary))?;
        ui.dispatch_pointer(view, Up(p, Primary))?;
        Ok(())
    };
    let first = record(&mut ui, 40.0, 12.0)?;
    ui.accept_pointer_frame(first)?;
    let next = record(&mut ui, 160.0, 42.0)?;
    click(&mut ui, 3.0)?;
    click(&mut ui, 43.0)?;
    assert_eq!(*ui.context().read(clicks)?, 0);
    click(&mut ui, 13.0)?;
    assert_eq!(*ui.context().read(clicks)?, 1);
    ui.accept_pointer_frame(next)?;
    click(&mut ui, 13.0)?;
    assert_eq!(*ui.context().read(clicks)?, 1);
    click(&mut ui, 43.0)?;
    assert_eq!(*ui.context().read(clicks)?, 2);
    Ok(())
}

#[test]
fn hiding_a_captured_target_takes_effect_when_the_frame_is_accepted() -> Result<()> {
    let mut ui = Ui::new();
    let show = ui.context().signal(true);
    let released = ui.context().signal(0);
    let canceled = ui.context().signal(0);
    let clicks = ui.context().signal(0);
    let view = ui.mount(component(|_| {
        div()
            .child(
                div()
                    .size(px(20.0))
                    .on_click(move |cx| cx.update(clicks, |n| *n += 1)),
            )
            .child(
                div()
                    .visible_when(show)
                    .mt(px(-20.0))
                    .size(px(20.0))
                    .on_pointer_down(|_, cx| {
                        cx.capture_pointer();
                        Ok(())
                    })
                    .on_pointer_up(move |_, cx| cx.update(released, |n| *n += 1))
                    .on_pointer_cancel(move |_, cx| cx.update(canceled, |n| *n += 1)),
            )
    }))?;
    accept(&mut ui, view, 40.0)?;
    let p = sample(1, 5.0, 5.0);
    ui.dispatch_pointer(view, Down(p, Primary))?;
    ui.context().set(show, false)?;
    ui.flush()?;
    let hidden = ui.record_frame(view)?.into_pointer_frame();
    assert_eq!(*ui.context().read(canceled)?, 0);
    ui.dispatch_pointer(view, Up(p, Primary))?;
    assert_eq!(*ui.context().read(released)?, 1);
    assert_eq!(*ui.context().read(clicks)?, 0);

    ui.dispatch_pointer(view, Down(p, Primary))?;
    ui.accept_pointer_frame(hidden)?;
    assert_eq!(*ui.context().read(canceled)?, 1);
    ui.dispatch_pointer(view, Up(p, Primary))?;
    ui.dispatch_pointer(view, Down(p, Primary))?;
    ui.dispatch_pointer(view, Up(p, Primary))?;
    assert_eq!(*ui.context().read(clicks)?, 1);
    Ok(())
}
