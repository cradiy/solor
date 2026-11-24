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
fn primary_press_focuses_the_nearest_focusable_ancestor_independently_of_clicks() -> Result<()> {
    let mut ui = Ui::new();
    let clicked = ui.context().signal(false);
    let view = ui.mount(component(|_| {
        div().child(
            div()
                .focusable()
                .size(px(30.0))
                .on_click(move |cx| cx.set(clicked, true))
                .child(div().focusable().size(px(10.0)).child(div().size(px(5.0)))),
        )
    }))?;
    accept(&mut ui, view, 60.0)?;
    let outer = ui.children(view, ui.root(view)?)?.next().unwrap();
    let inner = ui.children(view, outer)?.next().unwrap();
    let inside = sample(1, 2.0, 2.0);
    let edge = sample(1, 20.0, 20.0);
    let background = sample(1, 40.0, 2.0);
    assert_eq!(ui.focused(view)?, None);
    ui.dispatch_pointer(view, Down(inside, Primary))?;
    assert_eq!(ui.focused(view)?, Some(inner));
    ui.dispatch_pointer(view, Up(inside, Primary))?;
    assert!(*ui.context().read(clicked)?);
    ui.dispatch_pointer(view, Down(edge, Secondary))?;
    assert_eq!(ui.focused(view)?, Some(inner));
    ui.dispatch_pointer(view, Up(edge, Secondary))?;
    ui.dispatch_pointer(view, Down(edge, Primary))?;
    assert_eq!(ui.focused(view)?, Some(outer));
    ui.dispatch_pointer(view, Up(edge, Primary))?;
    ui.dispatch_pointer(view, Down(background, Primary))?;
    assert_eq!(ui.focused(view)?, None);
    Ok(())
}

#[test]
fn focus_is_local_to_a_live_visible_mount() -> Result<()> {
    let mut ui = Ui::new();
    let shown = ui.context().signal(true);
    let mounted = ui.context().signal(true);
    let view = ui.mount(component(|_| {
        div()
            .visible_when(shown)
            .child_when(mounted, || div().focusable().size(px(20.0)))
    }))?;
    let other = ui.mount(component(|_| div().focusable()))?;
    let other_root = ui.root(other)?;
    let root = ui.root(view)?;
    let inside = sample(1, 2.0, 2.0);
    accept(&mut ui, view, 60.0)?;
    let child = ui.children(view, root)?.next().unwrap();
    ui.focus(other, other_root)?;
    ui.dispatch_pointer(view, Down(inside, Primary))?;
    assert_eq!(ui.focused(view)?, Some(child));
    assert!(ui.focus(view, root).is_err());
    assert!(ui.focus(view, other_root).is_err());
    assert_eq!(ui.focused(view)?, Some(child));

    ui.context().set(shown, false)?;
    ui.flush()?;
    assert_eq!(ui.focused(view)?, None);
    assert!(ui.focus(view, child).is_err());
    ui.dispatch_pointer(view, Down(inside, Primary))?;
    assert_eq!(ui.focused(view)?, None);
    ui.context().set(shown, true)?;
    ui.flush()?;
    assert_eq!(ui.focused(view)?, None);
    ui.focus(view, child)?;
    ui.context().set(mounted, false)?;
    ui.flush()?;
    assert_eq!(ui.focused(view)?, None);
    ui.context().set(mounted, true)?;
    ui.flush()?;
    let replacement = ui.children(view, root)?.next().unwrap();
    assert_ne!(replacement, child);
    ui.dispatch_pointer(view, Down(inside, Primary))?;
    assert_eq!(ui.focused(view)?, None);
    assert!(ui.focus(view, child).is_err());
    ui.focus(view, replacement)?;
    ui.blur(view)?;
    assert_eq!(ui.focused(view)?, None);
    assert_eq!(ui.focused(other)?, Some(other_root));
    Ok(())
}

#[test]
fn label_hits_button_up_precedes_click_and_bindings_wait_for_flush() -> Result<()> {
    let mut ui = Ui::with_font(
        include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf").to_vec(),
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
fn captured_content_coordinates_follow_the_presented_paint_box() -> Result<()> {
    use solor::{Transition, TransitionProperty};
    use std::time::Duration;
    let mut ui = Ui::new();
    let left = ui.context().signal(px(2.5));
    let last_move = Rc::new(Cell::new(None));
    let view = ui.mount(component(|_| {
        let last_move = last_move.clone();
        div().p(px(3.0)).child(
            div()
                .w(px(20.0))
                .h(px(12.0))
                .pl(left)
                .transition(
                    TransitionProperty::Padding,
                    Transition::new(Duration::from_millis(100)),
                )
                .pr(px(3.0))
                .pt(px(1.25))
                .pb(px(2.75))
                .paint(|cx| {
                    cx.fill(cx.size, solor::rgb(0));
                    Ok(())
                })
                .on_pointer_down(|_, cx| {
                    cx.capture_pointer();
                    Ok(())
                })
                .on_pointer_move(move |event, _| {
                    last_move.set(Some(*event));
                    Ok(())
                }),
        )
    }))?;
    let viewport = Size {
        width: px(40.0),
        height: px(40.0),
    };
    let record = |ui: &mut Ui| -> Result<_> {
        ui.flush()?;
        ui.layout(view, viewport)?;
        let frame = ui.record_frame(view)?;
        let [solor::Draw::Fill { origin, size, .. }] = frame.draws() else {
            panic!("expected the painted content box");
        };
        let painted = (*origin, *size);
        Ok((frame.into_pointer_frame(), painted))
    };
    let (first, mut presented) = record(&mut ui)?;
    ui.accept_pointer_frame(first)?;
    // Press inside the padding, then drag beyond the border box.
    ui.dispatch_pointer(view, Down(sample(1, 3.5, 3.5), Primary))?;
    let outside = sample(1, 30.0, 1.0);
    let check = |ui: &mut Ui, (origin, size): (Origin, Size)| -> Result<()> {
        ui.dispatch_pointer(view, Move(outside))?;
        let event = last_move.take().expect("captured move");
        assert_eq!(event.content_size(), size);
        assert_eq!(
            event.content_position(),
            Origin {
                x: outside.position.x - origin.x,
                y: outside.position.y - origin.y,
            }
        );
        assert_eq!(
            event.local_position(),
            Origin {
                x: px(27.0),
                y: px(-2.0)
            }
        );
        assert_eq!(event.bounds().width, px(20.0));
        Ok(())
    };
    assert_eq!(presented.1.width, px(14.5));
    ui.context().set(left, px(18.0))?;
    for (ms, expected_width) in [(0, 14.5), (50, 6.75), (100, 0.0)] {
        ui.dispatch_frame(view, Duration::from_millis(ms))?;
        let (next, painted) = record(&mut ui)?;
        assert_eq!(painted.1.width, px(expected_width));
        check(&mut ui, presented)?;
        ui.accept_pointer_frame(next)?;
        check(&mut ui, painted)?;
        presented = painted;
    }
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

#[test]
fn scroll_uses_its_nearest_listener_and_preserves_press_and_capture() -> Result<()> {
    use solor::pointer::{PointerInput::Scroll, ScrollDelta};
    let mut ui = Ui::new();
    let clicks = ui.context().signal(0);
    let received = ui.context().signal(Vec::new());
    let view = ui.mount(component(|_| {
        div()
            .p(px(4.0))
            .items_stretch()
            .on_click(move |cx| cx.update(clicks, |n| *n += 1))
            .on_scroll(move |event, cx| {
                cx.update(received, |log| {
                    log.push((0, event.pointer.local_position(), event.delta))
                })
            })
            .child(div().h(px(10.0)).on_pointer_down(|_, cx| {
                cx.capture_pointer();
                Ok(())
            }))
            .child(div().h(px(10.0)).on_scroll(move |event, cx| {
                cx.update(received, |log| {
                    log.push((1, event.pointer.local_position(), event.delta))
                })
            }))
    }))?;
    let a = sample(1, 7.0, 9.0);
    let b = sample(1, 7.0, 17.5);
    let delta = ScrollDelta {
        x: px(-1.25),
        y: px(2.5),
    };
    ui.dispatch_pointer(view, Scroll(b, delta))?;
    accept(&mut ui, view, 80.0)?;
    ui.dispatch_pointer(view, Down(a, Primary))?;
    ui.dispatch_pointer(view, Scroll(a, delta))?;
    ui.dispatch_pointer(view, Scroll(b, delta))?;
    ui.dispatch_pointer(view, Up(b, Primary))?;
    ui.dispatch_pointer(view, Down(b, Primary))?;
    ui.dispatch_pointer(view, Scroll(b, ScrollDelta::default()))?;
    ui.dispatch_pointer(view, Up(b, Primary))?;
    assert_eq!(*ui.context().read(clicks)?, 1);
    assert_eq!(
        ui.context().read(received)?.as_slice(),
        &[
            (0, a.position, delta),
            (
                1,
                Origin {
                    x: px(3.0),
                    y: px(3.5)
                },
                delta
            ),
        ]
    );
    assert!(
        ui.dispatch_pointer(
            view,
            Scroll(
                a,
                ScrollDelta {
                    x: px(f32::INFINITY),
                    ..delta
                }
            )
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn nested_scroll_handoff_uses_each_listeners_accepted_coordinates() -> Result<()> {
    use solor::pointer::{PointerInput::Scroll, ScrollContext, ScrollDelta, ScrollEvent};
    use std::cell::RefCell;

    let mut ui = Ui::new();
    let padding = ui.context().signal(px(2.0));
    let log = Rc::new(RefCell::new(Vec::new()));
    let listen = |name| {
        let log = log.clone();
        move |event: &ScrollEvent, cx: &mut ScrollContext<'_>| {
            log.borrow_mut()
                .push((name, event.pointer.local_position().x, event.delta));
            if event.delta.y > px(0.0) || (name == "inner" && event.delta.y == px(0.0)) {
                cx.propagate();
            }
            anyhow::ensure!(
                name != "middle" || event.delta.x != px(5.0),
                "scroll failure"
            );
            Ok(())
        }
    };
    let view = ui.mount(component(|_| {
        div()
            .p(padding)
            .items_stretch()
            .on_scroll(listen("outer"))
            .child(
                div()
                    .p(px(3.0))
                    .items_stretch()
                    .on_scroll(listen("middle"))
                    .child(div().h(px(12.0)).on_scroll(listen("inner"))),
            )
    }))?;
    accept(&mut ui, view, 80.0)?;
    let mouse = sample(1, 10.5, 10.5);
    let both = ScrollDelta {
        x: px(-1.25),
        y: px(2.5),
    };
    let horizontal = ScrollDelta { y: px(0.0), ..both };
    let up = ScrollDelta {
        y: px(-1.0),
        ..both
    };

    ui.context().set(padding, px(4.0))?;
    ui.flush()?;
    ui.layout(
        view,
        Size {
            width: px(80.0),
            height: px(40.0),
        },
    )?;
    let pending = ui.prepare_pointer_frame(view)?;
    ui.dispatch_pointer(view, Scroll(mouse, both))?;
    assert_eq!(
        log.take(),
        [
            ("inner", px(5.5), both),
            ("middle", px(8.5), both),
            ("outer", px(10.5), both)
        ]
    );

    ui.accept_pointer_frame(pending)?;
    ui.dispatch_pointer(view, Scroll(mouse, horizontal))?;
    assert_eq!(
        log.take(),
        [
            ("inner", px(3.5), horizontal),
            ("middle", px(6.5), horizontal)
        ]
    );
    ui.dispatch_pointer(view, Scroll(mouse, up))?;
    assert_eq!(log.take(), [("inner", px(3.5), up)]);
    let rejected = ScrollDelta { x: px(5.0), ..both };
    assert!(ui.dispatch_pointer(view, Scroll(mouse, rejected)).is_err());
    assert_eq!(
        log.take(),
        [("inner", px(3.5), rejected), ("middle", px(6.5), rejected)]
    );
    Ok(())
}

#[test]
fn each_scroll_listener_can_forward_only_its_available_distance() -> Result<()> {
    use solor::pointer::{PointerInput::Scroll, ScrollDelta};

    let mut ui = Ui::new();
    let request = ui.context().signal(None::<ScrollDelta>);
    let delivered = ui.context().signal(Vec::new());
    let view = ui.mount(component(|_| {
        div()
            .items_stretch()
            .on_scroll(move |event, cx| cx.update(delivered, |log| log.push(event.delta)))
            .child(
                div()
                    .items_stretch()
                    .on_scroll(move |_, cx| {
                        if let Some(part) = *cx.read(request)? {
                            cx.propagate_remaining(part)?;
                        } else {
                            cx.propagate();
                        }
                        Ok(())
                    })
                    .child(div().h(px(16.0)).on_scroll(|event, cx| {
                        cx.propagate_remaining(ScrollDelta {
                            x: px(0.0),
                            y: event.delta.y / 2.0,
                        })
                    })),
            )
    }))?;
    accept(&mut ui, view, 80.0)?;
    let delta = |x, y| ScrollDelta { x: px(x), y: px(y) };
    let input = Scroll(sample(1, 4.0, 4.0), delta(1.5, -8.5));
    for invalid in [
        delta(0.0, -8.5),
        delta(0.0, 1.0),
        delta(0.5, 0.0),
        delta(0.0, f32::NAN),
        delta(0.0, f32::NEG_INFINITY),
    ] {
        ui.context().set(request, Some(invalid))?;
        assert!(ui.dispatch_pointer(view, input).is_err());
    }
    for part in [Some(ScrollDelta::default()), None, Some(delta(0.0, -1.25))] {
        ui.context().set(request, part)?;
        ui.dispatch_pointer(view, input)?;
    }
    assert_eq!(
        ui.context().read(delivered)?.as_slice(),
        &[delta(0.0, -4.25), delta(0.0, -1.25)]
    );
    Ok(())
}

#[test]
fn scroll_respects_clipping_and_keeps_a_retired_leaf_from_exposing_its_parent() -> Result<()> {
    use solor::pointer::{PointerInput::Scroll, ScrollDelta};
    let mut ui = Ui::new();
    let enabled = ui.context().signal(true);
    let parent = ui.context().signal(0);
    let child = ui.context().signal(0);
    let view = ui.mount(component(|_| {
        div()
            .on_scroll(move |_, cx| cx.update(parent, |n| *n += 1))
            .child(
                div()
                    .size(px(15.0))
                    .overflow_hidden()
                    .child_when(enabled, move || {
                        div()
                            .w(px(40.0))
                            .h(px(10.0))
                            .on_scroll(move |_, cx| cx.update(child, |n| *n += 1))
                    }),
            )
    }))?;
    let scroll = |x| {
        Scroll(
            sample(1, x, 2.0),
            ScrollDelta {
                x: px(0.0),
                y: px(2.0),
            },
        )
    };
    accept(&mut ui, view, 64.0)?;
    ui.dispatch_pointer(view, scroll(2.0))?;
    ui.dispatch_pointer(view, scroll(20.0))?;
    ui.context().set(enabled, false)?;
    ui.flush()?;
    ui.dispatch_pointer(view, scroll(2.0))?;
    assert_eq!(*ui.context().read(child)?, 1);
    assert_eq!(*ui.context().read(parent)?, 1);
    accept(&mut ui, view, 64.0)?;
    ui.dispatch_pointer(view, scroll(2.0))?;
    assert_eq!(*ui.context().read(parent)?, 2);
    Ok(())
}

#[test]
fn resize_grip_delegates_to_children_and_respects_pointer_capture() -> Result<()> {
    use solor::{ResizeEdge, WindowControl};

    let mut ui = Ui::new();
    let activated = ui.context().signal(false);
    let capture = ui.context().signal(false);
    let view = ui.mount(component(|_| {
        div()
            .window_resize_region(ResizeEdge::BottomRight)
            .on_pointer_down(move |_, cx| {
                if *cx.read(capture)? {
                    cx.capture_pointer();
                }
                Ok(())
            })
            .child(
                div()
                    .size(px(10.0))
                    .on_click(move |cx| cx.set(activated, true)),
            )
    }))?;
    accept(&mut ui, view, 80.0)?;
    let grip = sample(1, 40.0, 20.0);
    let button = sample(1, 5.0, 5.0);

    for input in [
        Down(button, Primary),
        Up(button, Primary),
        Down(grip, Secondary),
        Up(grip, Secondary),
    ] {
        assert_eq!(ui.dispatch_pointer(view, input)?, None);
    }
    assert!(*ui.context().read(activated)?);
    assert_eq!(
        ui.dispatch_pointer(view, Down(grip, Primary))?,
        Some(WindowControl::Resize(ResizeEdge::BottomRight))
    );
    assert_eq!(ui.dispatch_pointer(view, Up(grip, Primary))?, None);
    ui.context().set(capture, true)?;
    for input in [Down(grip, Primary), Move(button), Up(button, Primary)] {
        assert_eq!(ui.dispatch_pointer(view, input)?, None);
    }
    Ok(())
}

#[test]
fn hover_without_a_tooltip_tracks_mouse_presence_across_accepted_frames() -> Result<()> {
    use std::cell::RefCell;

    let mut ui = Ui::new();
    let show = ui.context().signal(true);
    let log = Rc::new(RefCell::new(Vec::new()));
    let listen = |name| {
        let log = log.clone();
        move |inside, _: &mut solor::Context<'_>| {
            log.borrow_mut().push((name, inside));
            Ok(())
        }
    };
    let view = ui.mount(component(|_| {
        div().on_hover(listen("outer")).child(
            div()
                .size(px(20.0))
                .visible_when(show)
                .on_hover(listen("inner")),
        )
    }))?;
    let one = sample(1, 5.0, 5.0);
    let two = sample(2, 6.0, 6.0);
    ui.dispatch_pointer(view, Move(one))?;
    assert!(log.borrow().is_empty());
    accept(&mut ui, view, 80.0)?;
    assert_eq!(log.take(), [("outer", true), ("inner", true)]);
    ui.dispatch_pointer(view, Move(two))?;
    ui.dispatch_pointer(view, Leave(one.id))?;
    ui.dispatch_pointer(view, Move(two))?;
    assert!(log.borrow().is_empty());
    assert!(ui.next_redraw(view).is_none());
    ui.context().set(show, false)?;
    ui.flush()?;
    ui.layout(
        view,
        Size {
            width: px(80.0),
            height: px(40.0),
        },
    )?;
    let prepared = ui.prepare_pointer_frame(view)?;
    assert!(log.borrow().is_empty());
    ui.accept_pointer_frame(prepared)?;
    assert_eq!(log.take(), [("inner", false)]);
    ui.cancel_pointers(view)?;
    assert_eq!(log.take(), [("outer", false)]);
    ui.dispatch_pointer(
        view,
        Move(PointerSample {
            kind: PointerKind::Touch,
            ..one
        }),
    )?;
    assert!(log.borrow().is_empty());
    Ok(())
}

#[test]
fn hover_position_follows_accepted_padding_and_the_remaining_mouse() -> Result<()> {
    use std::cell::RefCell;

    let mut ui = Ui::new();
    let padding = ui.context().signal(px(2.0));
    let positions = Rc::new(RefCell::new(Vec::new()));
    let moves = Rc::new(Cell::new(0));
    let view = ui.mount(component(|_| {
        let positions = positions.clone();
        let moves = moves.clone();
        div()
            .p(padding)
            .on_hover_position(move |hover, _| {
                positions
                    .borrow_mut()
                    .push(hover.map(|h| (h.position.x, h.size.width)));
                Ok(())
            })
            .child(div().size(px(30.0)).on_pointer_move(move |_, _| {
                moves.set(moves.get() + 1);
                Ok(())
            }))
    }))?;
    let first = sample(7, 12.0, 12.0);
    let second = sample(3, 22.0, 12.0);
    ui.dispatch_pointer(view, Move(first))?;
    accept(&mut ui, view, 80.0)?;
    assert_eq!(positions.take(), [Some((px(10.0), px(76.0)))]);
    ui.dispatch_pointer(view, Move(second))?;
    assert_eq!(positions.take(), [Some((px(20.0), px(76.0)))]);
    ui.context().set(padding, px(5.0))?;
    ui.flush()?;
    ui.layout(
        view,
        Size {
            width: px(80.0),
            height: px(40.0),
        },
    )?;
    let changed = ui.prepare_pointer_frame(view)?;
    assert!(positions.borrow().is_empty());
    ui.accept_pointer_frame(changed)?;
    assert_eq!(positions.take(), [Some((px(17.0), px(70.0)))]);
    accept(&mut ui, view, 80.0)?;
    ui.dispatch_pointer(view, Move(second))?;
    assert!(positions.borrow().is_empty());
    assert_eq!(moves.get(), 2);
    ui.dispatch_pointer(view, Leave(second.id))?;
    assert_eq!(positions.take(), [Some((px(7.0), px(70.0)))]);
    ui.cancel_pointers(view)?;
    assert_eq!(positions.take(), [None]);
    Ok(())
}

#[test]
fn tooltip_without_a_callback_waits_cancels_and_draws_above_the_owner() -> Result<()> {
    use solor::Draw;
    let mut ui = Ui::with_font(
        include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf").to_vec(),
    )?;
    let show = ui.context().signal(true);
    let view = ui.mount(component(|_| {
        div()
            .tooltip("Outer hint")
            .items_end()
            .justify_end()
            .child_when(show, || {
                div().size(px(10.0)).overflow_hidden().tooltip(
                    solor::Tooltip::new("Inner hint")
                        .bg(solor::rgba(0x245569cc))
                        .text_color(solor::rgb(0xffdf72)),
                )
            })
    }))?;
    accept(&mut ui, view, 140.0)?;
    let mouse = sample(1, 135.0, 35.0);
    ui.dispatch_pointer(view, Move(mouse))?;
    let first = ui.next_redraw(view).unwrap();
    ui.dispatch_pointer(view, Move(mouse))?;
    assert_eq!(ui.next_redraw(view), Some(first));
    assert!(ui.record_frame(view)?.draws().is_empty());
    ui.dispatch_pointer(view, Leave(mouse.id))?;
    assert!(ui.next_redraw(view).is_none());
    ui.dispatch_pointer(view, Move(mouse))?;
    let due = ui.next_redraw(view).unwrap();
    std::thread::sleep(due.saturating_duration_since(std::time::Instant::now()));
    let frame = ui.record_frame(view)?;
    let [
        Draw::Fill {
            origin,
            size,
            clip,
            fill,
            ..
        },
        Draw::Text {
            paragraph,
            color: solor::Ink::Solid(color),
            ..
        },
    ] = frame.draws()
    else {
        panic!("expected a tooltip overlay")
    };
    assert_eq!(*fill, solor::Fill::from(solor::rgba(0x245569cc)));
    assert_eq!(*color, solor::rgb(0xffdf72));
    assert_eq!(paragraph.text.as_str(), "Inner hint");
    assert!(clip.is_none() && origin.x >= px(0.0) && origin.x + size.width <= px(140.0));
    assert!(origin.y >= px(0.0) && origin.y + size.height <= px(30.0));
    assert!(ui.next_redraw(view).is_none());
    ui.dispatch_pointer(view, Down(mouse, Primary))?;
    assert!(ui.next_redraw(view).unwrap() <= std::time::Instant::now());
    assert!(ui.record_frame(view)?.draws().is_empty());
    ui.dispatch_pointer(view, Move(mouse))?;
    assert!(ui.next_redraw(view).is_none());
    ui.dispatch_pointer(view, Up(mouse, Primary))?;
    ui.dispatch_pointer(view, Move(mouse))?;
    ui.context().set(show, false)?;
    accept(&mut ui, view, 140.0)?;
    assert!(ui.next_redraw(view).unwrap() > std::time::Instant::now());
    ui.dispatch_pointer(view, Cancel(mouse.id))?;
    assert!(ui.next_redraw(view).is_none());
    Ok(())
}

#[test]
fn cursors_follow_the_accepted_branch_for_each_mouse_without_listeners() -> Result<()> {
    use solor::style::{Cursor, PointerEvents};

    let mut ui = Ui::new();
    let tool = ui.context().signal(Cursor::ZoomIn);
    let show = ui.context().signal(true);
    let view = ui.mount(component(|_| {
        div().cursor_pointer().child(
            div()
                .size(px(30.0))
                .cursor(tool)
                .visible_when(show)
                .child(div().size(px(10.0)).cursor(Cursor::None))
                .child(
                    div()
                        .size(px(10.0))
                        .cursor(Cursor::Grab)
                        .pointer_events(PointerEvents::None),
                ),
        )
    }))?;
    let a = sample(1, 4.0, 4.0);
    let b = sample(2, 4.0, 15.0);
    ui.dispatch_pointer(view, Move(a))?;
    assert_eq!(ui.pointer_cursor(view, a.id), Cursor::Default);
    accept(&mut ui, view, 80.0)?;
    ui.dispatch_pointer(view, Move(b))?;
    assert_eq!(ui.pointer_cursor(view, a.id), Cursor::None);
    assert_eq!(ui.pointer_cursor(view, b.id), Cursor::ZoomIn);
    assert!(ui.next_redraw(view).is_none());

    ui.context().set(tool, Cursor::ZoomOut)?;
    accept(&mut ui, view, 80.0)?;
    assert_eq!(ui.pointer_cursor(view, b.id), Cursor::ZoomOut);
    assert_eq!(ui.pointer_cursor(view, a.id), Cursor::None);

    ui.context().set(show, false)?;
    ui.flush()?;
    ui.layout(
        view,
        Size {
            width: px(80.0),
            height: px(40.0),
        },
    )?;
    let frame = ui.prepare_pointer_frame(view)?;
    assert_eq!(ui.pointer_cursor(view, b.id), Cursor::ZoomOut);
    ui.accept_pointer_frame(frame)?;
    assert_eq!(ui.pointer_cursor(view, b.id), Cursor::Pointer);
    ui.dispatch_pointer(view, Leave(b.id))?;
    assert_eq!(ui.pointer_cursor(view, b.id), Cursor::Default);
    assert_eq!(ui.pointer_cursor(view, a.id), Cursor::Pointer);
    Ok(())
}

#[test]
fn resize_cursor_tracks_the_effective_control_and_respects_an_explicit_style() -> Result<()> {
    use solor::{ResizeEdge, style::Cursor};
    let mut ui = Ui::new();
    let view = ui.mount(component(|_| {
        div()
            .window_resize_region(ResizeEdge::TopLeft)
            .child(div().size(px(10.0)).on_click(|_| Ok(())))
            .child(div().size(px(10.0)).cursor(Cursor::Crosshair))
    }))?;
    accept(&mut ui, view, 80.0)?;
    for (x, y, cursor) in [
        (50.0, 5.0, Cursor::Resize(ResizeEdge::TopLeft)),
        (4.0, 5.0, Cursor::Default),
        (4.0, 15.0, Cursor::Crosshair),
    ] {
        let mouse = sample(1, x, y);
        ui.dispatch_pointer(view, Move(mouse))?;
        assert_eq!(ui.pointer_cursor(view, mouse.id), cursor);
    }
    Ok(())
}

#[test]
fn fixed_window_keeps_element_cursor_styles_and_click_handlers() -> Result<()> {
    use solor::{ResizeEdge, WindowControl, style::Cursor};
    let mut ui = Ui::new();
    let clicks = ui.context().signal(0);
    let view = ui.mount(component(move |_| {
        div().window_resize_region(ResizeEdge::Left).child(
            div()
                .size(px(10.0))
                .window_maximize_button()
                .cursor(Cursor::Resize(ResizeEdge::Right))
                .on_click(move |cx| cx.update(clicks, |n| *n += 1)),
        )
    }))?;
    accept(&mut ui, view, 80.0)?;
    let border = sample(1, 50.0, 5.0);
    let button = sample(2, 5.0, 5.0);
    ui.dispatch_pointer(view, Move(border))?;
    ui.dispatch_pointer(view, Move(button))?;
    ui.set_window_resizable(view, false)?;
    assert_eq!(ui.pointer_cursor(view, border.id), Cursor::Default);
    assert_eq!(
        ui.pointer_cursor(view, button.id),
        Cursor::Resize(ResizeEdge::Right)
    );
    assert_eq!(ui.dispatch_pointer(view, Down(border, Primary))?, None);
    ui.dispatch_pointer(view, Up(border, Primary))?;
    ui.dispatch_pointer(view, Down(button, Primary))?;
    assert_eq!(ui.dispatch_pointer(view, Up(button, Primary))?, None);
    assert_eq!(*ui.context().read(clicks)?, 1);
    ui.set_window_resizable(view, true)?;
    assert_eq!(
        ui.pointer_cursor(view, border.id),
        Cursor::Resize(ResizeEdge::Left)
    );
    assert_eq!(
        ui.dispatch_pointer(view, Down(border, Primary))?,
        Some(WindowControl::Resize(ResizeEdge::Left))
    );
    Ok(())
}

#[test]
fn tooltip_defaults_update_pending_and_visible_hints_and_preserve_local_overrides() -> Result<()> {
    use solor::{
        Draw, Edges, Fill, Tooltip, TooltipAlignment, TooltipPlacement, TooltipStyle, rgb,
        style::Corners, text::TextStyle,
    };
    use std::time::Duration;

    let mut ui = Ui::with_font(
        include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf").to_vec(),
    )?;
    let view = ui.mount(component(|_| {
        div().child(
            div()
                .size(px(12.0))
                .tooltip(Tooltip::new("X").text_color(rgb(0xffdd00)).gap(px(3.5))),
        )
    }))?;
    accept(&mut ui, view, 140.0)?;
    ui.dispatch_pointer(view, Move(sample(1, 5.0, 5.0)))?;
    assert!(ui.record_frame(view)?.draws().is_empty());

    let theme = TooltipStyle {
        delay: Duration::ZERO,
        placement: TooltipPlacement::Right,
        alignment: TooltipAlignment::End,
        padding: Edges {
            left: px(6.5),
            top: px(2.5),
            right: px(4.0),
            bottom: px(2.0),
        },
        corner_radii: Corners::all(px(2.5)),
        typography: TextStyle {
            size: px(12.0),
            line_height: px(16.0),
        },
        background: rgb(0xf0f0f0).into(),
        text_color: rgb(0x202020),
        ..TooltipStyle::default()
    };
    ui.set_tooltip_style(theme.clone())?;
    let frame = ui.record_frame(view)?;
    let [
        Draw::Fill {
            origin,
            fill,
            corner_radii,
            ..
        },
        Draw::Text {
            origin: label,
            color: solor::Ink::Solid(color),
            paragraph,
            ..
        },
    ] = frame.draws()
    else {
        panic!("expected visible tooltip");
    };
    assert_eq!(
        *origin,
        Origin {
            x: px(15.5),
            y: px(0.0)
        }
    );
    assert_eq!(
        *label,
        Origin {
            x: px(22.0),
            y: px(2.5)
        }
    );
    assert_eq!(corner_radii, &theme.corner_radii);
    assert_eq!(fill, &theme.background);
    assert_eq!(*color, rgb(0xffdd00));
    assert_eq!(paragraph.style, theme.typography);
    assert!(ui.next_redraw(view).is_none());

    let typography = TextStyle {
        size: px(18.0),
        line_height: px(22.0),
    };
    ui.set_tooltip_style(TooltipStyle {
        delay: Duration::from_secs(10),
        typography,
        max_width: px(22.0),
        background: rgb(0x303030).into(),
        ..theme.clone()
    })?;
    assert!(ui.next_redraw(view).is_some());
    let frame = ui.record_frame(view)?;
    let [
        Draw::Fill { size, fill, .. },
        Draw::Text {
            paragraph,
            color: solor::Ink::Solid(color),
            ..
        },
    ] = frame.draws()
    else {
        panic!("expected restyled tooltip");
    };
    assert!(size.width <= px(22.0));
    assert_eq!(fill, &Fill::from(rgb(0x303030)));
    assert_eq!(*color, rgb(0xffdd00));
    assert_eq!(paragraph.style, typography);
    assert!(ui.next_redraw(view).is_none());

    let old = ui.tooltip_style().clone();
    assert!(
        ui.set_tooltip_style(TooltipStyle {
            max_width: px(f32::NAN),
            ..theme.clone()
        })
        .is_err()
    );
    assert_eq!(ui.tooltip_style(), &old);
    ui.set_tooltip_style(TooltipStyle {
        padding: Edges::all(px(100.0)),
        ..theme
    })?;
    assert!(ui.record_frame(view)?.draws().is_empty());
    assert!(ui.next_redraw(view).is_none());
    Ok(())
}

#[test]
fn dominant_wheel_axis_uses_each_event_and_chains_at_the_viewport_edge() -> Result<()> {
    use solor::pointer::{PointerInput::Scroll, ScrollDelta};
    let mut ui = Ui::new();
    let dominant = ui.context().signal(false);
    let parent = Rc::new(Cell::new(ScrollDelta::default()));
    let received = parent.clone();
    let view = ui.mount(component(move |_| {
        div()
            .on_scroll(move |event, _| {
                received.set(event.delta);
                Ok(())
            })
            .child(
                div()
                    .size(px(40.0))
                    .overflow_scroll()
                    .dominant_scroll_axis(dominant)
                    .child(div().min_w(px(60.0)).h(px(80.0))),
            )
    }))?;
    accept(&mut ui, view, 60.0)?;
    let pane = ui.children(view, ui.root(view)?)?.next().unwrap();
    let delta = |x, y| ScrollDelta { x: px(x), y: px(y) };
    let wheel = |ui: &mut Ui, x, y| -> Result<(ScrollDelta, ScrollDelta)> {
        ui.dispatch_pointer(view, Scroll(sample(1, 4.0, 4.0), delta(x, y)))?;
        let offset = ui.scroll_metrics(view, pane).unwrap().offset;
        Ok((
            ScrollDelta {
                x: offset.x,
                y: offset.y,
            },
            parent.take(),
        ))
    };
    ui.context().set(dominant, true)?;
    ui.flush()?;
    assert_eq!(
        wheel(&mut ui, 2.0, 3.0)?,
        (delta(2.0, 3.0), delta(0.0, 0.0))
    );
    accept(&mut ui, view, 60.0)?;
    for (input, offset, remainder) in [
        ((8.5, 1.0), (10.5, 3.0), (0.0, 0.0)),
        ((-2.5, 2.5), (10.5, 5.5), (0.0, 0.0)),
        ((-0.5, -4.0), (10.5, 1.5), (0.0, 0.0)),
        ((14.0, 3.0), (20.0, 1.5), (4.5, 0.0)),
        ((5.0, 2.0), (20.0, 1.5), (5.0, 2.0)),
    ] {
        assert_eq!(
            wheel(&mut ui, input.0, input.1)?,
            (delta(offset.0, offset.1), delta(remainder.0, remainder.1))
        );
    }
    Ok(())
}

#[test]
fn horizontal_strip_returns_unused_wheel_distance_on_the_input_axis() -> Result<()> {
    use solor::pointer::{PointerInput::Scroll, ScrollDelta};

    let mut ui = Ui::new();
    let remap = ui.context().signal(false);
    let view = ui.mount(component(|_| {
        div()
            .overflow_y_scroll()
            .child(
                div()
                    .size(px(40.0))
                    .overflow_x_scroll()
                    .vertical_wheel_as_horizontal(remap)
                    .child(div().min_w(px(60.0)).h(px(40.0))),
            )
            .child(div().h(px(80.0)))
    }))?;
    accept(&mut ui, view, 60.0)?;
    let page = ui.root(view)?;
    let strip = ui.children(view, page)?.next().unwrap();
    let wheel = |ui: &mut Ui, x, y| -> Result<()> {
        ui.dispatch_pointer(
            view,
            Scroll(sample(1, 4.0, 4.0), ScrollDelta { x: px(x), y: px(y) }),
        )?;
        Ok(())
    };
    let offsets = |ui: &Ui| {
        (
            ui.scroll_metrics(view, strip).unwrap().offset.x,
            ui.scroll_metrics(view, page).unwrap().offset.y,
        )
    };
    ui.context().set(remap, true)?;
    ui.flush()?;
    wheel(&mut ui, 0.0, 2.0)?;
    assert_eq!(offsets(&ui), (px(0.0), px(2.0)));
    accept(&mut ui, view, 60.0)?;
    wheel(&mut ui, 0.0, 28.5)?;
    assert_eq!(offsets(&ui), (px(20.0), px(10.5)));
    wheel(&mut ui, 0.0, -25.0)?;
    assert_eq!(offsets(&ui), (px(0.0), px(5.5)));
    wheel(&mut ui, 3.25, 4.0)?;
    assert_eq!(offsets(&ui), (px(3.25), px(9.5)));
    Ok(())
}

#[test]
fn container_seek_shares_wheel_offsets_and_switches_hits_on_submission() -> Result<()> {
    use solor::{
        Draw, ScrollMetrics,
        pointer::{PointerInput, ScrollDelta},
        rgb,
        style::Overflow,
    };

    let mut ui = Ui::new();
    let selected = ui.context().signal(None);
    let horizontal = ui.context().signal(Overflow::Scroll);
    let view = ui.mount(component(move |_| {
        div()
            .overflow_x(horizontal)
            .overflow_y_scroll()
            .children((0..3u32).map(|index| {
                div()
                    .min_w(px(100.0))
                    .min_h(px(30.0))
                    .bg(rgb(index + 1))
                    .on_click(move |cx| cx.set(selected, Some(index)))
            }))
    }))?;
    let root = ui.root(view)?;
    let origin = |x, y| Origin { x: px(x), y: px(y) };
    assert!(ui.scroll_metrics(view, root).is_none());
    assert!(ui.set_scroll_offset(view, root, origin(2.0, 1.0)).is_err());
    accept(&mut ui, view, 60.0)?;
    assert_eq!(
        ui.scroll_metrics(view, root),
        Some(ScrollMetrics {
            offset: Origin::default(),
            viewport: Size {
                width: px(60.0),
                height: px(40.0)
            },
            max_offset: Size {
                width: px(40.0),
                height: px(50.0)
            },
        })
    );
    let p = sample(1, 3.0, 2.0);
    ui.dispatch_pointer(
        view,
        PointerInput::Scroll(
            p,
            ScrollDelta {
                x: px(0.5),
                y: px(1.25),
            },
        ),
    )?;
    assert_eq!(
        ui.scroll_metrics(view, root).unwrap().offset,
        origin(0.5, 1.25)
    );
    assert!(ui.set_scroll_offset(view, root, origin(7.75, 31.25))?);
    let frame = ui.record_frame(view)?;
    assert!(frame.draws().iter().any(|d|matches!(d, Draw::Fill {origin: at,fill,..} if *fill == rgb(2).into() && *at == origin(-7.75,-1.25))));
    let click = |ui: &mut Ui| -> Result<()> {
        ui.dispatch_pointer(view, Down(p, Primary))?;
        ui.dispatch_pointer(view, Up(p, Primary))?;
        Ok(())
    };
    click(&mut ui)?;
    assert_eq!(*ui.context().read(selected)?, Some(0));
    ui.accept_pointer_frame(frame.into_pointer_frame())?;
    click(&mut ui)?;
    assert_eq!(*ui.context().read(selected)?, Some(1));
    assert!(!ui.set_scroll_offset(view, root, origin(7.75, 31.25))?);
    assert!(ui.next_redraw(view).is_none());
    assert!(ui.set_scroll_offset(view, root, origin(-100.0, 500.0))?);
    assert!(
        ui.set_scroll_offset(view, root, origin(0.0, f32::NAN))
            .is_err()
    );
    assert_eq!(
        ui.scroll_metrics(view, root).unwrap().offset,
        origin(0.0, 50.0)
    );
    ui.context().set(horizontal, Overflow::Hidden)?;
    accept(&mut ui, view, 60.0)?;
    assert!(!ui.set_scroll_offset(view, root, origin(12.0, 50.0))?);
    let child = ui.children(view, root)?.next().unwrap();
    assert!(ui.scroll_metrics(view, child).is_none());
    ui.unmount(view)?;
    assert!(ui.scroll_metrics(view, root).is_none());
    Ok(())
}

#[test]
fn display_none_removes_the_box_and_restores_the_same_interactive_subtree() -> Result<()> {
    use solor::{Draw, Fill, rgb, style::Display};

    let mut ui = Ui::new();
    let display = ui.context().signal(Display::Flex);
    let sibling_clicks = ui.context().signal(0);
    let view = ui.mount(component(move |_| {
        div()
            .gap(px(3.))
            .child(div().display(display).child(component(|cx| {
                let ink = cx.signal(rgb(0xff0000));
                div()
                    .size(px(12.))
                    .focusable()
                    .bg(ink)
                    .on_click(move |cx| cx.set(ink, rgb(0x0000ff)))
            })))
            .child(
                div()
                    .size(px(12.))
                    .focusable()
                    .on_click(move |cx| cx.update(sibling_clicks, |count| *count += 1)),
            )
    }))?;
    let root = ui.root(view)?;
    let nodes: Vec<_> = ui.children(view, root)?.collect();
    let (panel, sibling) = (nodes[0], nodes[1]);
    let child = ui.children(view, panel)?.next().unwrap();
    let point = sample(1, 4., 4.);
    accept(&mut ui, view, 40.)?;
    assert_eq!(ui.geometry(view, sibling).unwrap().origin.y, px(15.));
    ui.dispatch_pointer(view, Down(point, Primary))?;
    ui.dispatch_pointer(view, Up(point, Primary))?;
    assert_eq!(ui.focused(view)?, Some(child));

    ui.context().set(display, Display::None)?;
    accept(&mut ui, view, 40.)?;
    assert!(ui.geometry(view, panel).is_none() && ui.geometry(view, child).is_none());
    assert_eq!(ui.geometry(view, sibling).unwrap().origin.y, px(0.));
    assert!(ui.record_frame(view)?.draws().is_empty());
    assert_eq!(ui.focused(view)?, None);
    assert_eq!(ui.focus_next(view)?, Some(sibling));
    ui.dispatch_pointer(view, Down(point, Primary))?;
    ui.dispatch_pointer(view, Up(point, Primary))?;
    assert_eq!(*ui.context().read(sibling_clicks)?, 1);

    ui.context().set(display, Display::Flex)?;
    accept(&mut ui, view, 40.)?;
    assert_eq!(ui.focus_next(view)?, Some(child));
    assert_eq!(ui.geometry(view, sibling).unwrap().origin.y, px(15.));
    assert!(
        matches!(ui.record_frame(view)?.draws(), [Draw::Fill {fill: Fill::Solid(c), ..}] if *c == rgb(0x0000ff))
    );
    Ok(())
}

#[test]
fn a_hidden_root_resumes_with_the_latest_child_display_and_visibility() -> Result<()> {
    use solor::{rgb, style::Display};

    let mut ui = Ui::new();
    let root_display = ui.context().signal(Display::None);
    let child_display = ui.context().signal(Display::None);
    let extra = ui.context().signal(false);
    let visible = ui.context().signal(false);
    let view = ui.mount(component(move |_| {
        div()
            .display(root_display)
            .items_start()
            .child(div().hidden().size(px(17.)).bg(rgb(0xffffff)))
            .child_when(extra, || div().size(px(4.)))
            .child(
                div()
                    .display(child_display)
                    .visible_when(visible)
                    .size(px(12.))
                    .bg(rgb(0xff0000)),
            )
    }))?;
    let root = ui.root(view)?;
    let child = ui.children(view, root)?.last().unwrap();
    accept(&mut ui, view, 40.)?;
    assert!(ui.geometry(view, root).is_none());
    ui.context().set(child_display, Display::Flex)?;
    accept(&mut ui, view, 40.)?;
    assert!(ui.geometry(view, child).is_none());
    ui.context().set(root_display, Display::Flex)?;
    accept(&mut ui, view, 40.)?;
    assert_eq!(ui.geometry(view, child).unwrap().origin.y, px(0.));
    assert!(ui.record_frame(view)?.draws().is_empty());
    ui.context().set(visible, true)?;
    accept(&mut ui, view, 40.)?;
    assert_eq!(ui.record_frame(view)?.draws().len(), 1);
    for (mounted, y) in [(true, 4.), (false, 0.)] {
        ui.context().set(extra, mounted)?;
        accept(&mut ui, view, 40.)?;
        assert_eq!(ui.geometry(view, child).unwrap().origin.y, px(y));
    }
    ui.context().set(root_display, Display::None)?;
    accept(&mut ui, view, 40.)?;
    assert!(ui.geometry(view, child).is_none());
    Ok(())
}

#[test]
fn an_anchored_badge_tracks_the_painted_hit_target_across_resize() -> Result<()> {
    use solor::{Draw, rgb};
    let mut ui = Ui::new();
    let target = ui.context().signal(0);
    let view = ui.mount(component(|_| {
        div()
            .child(
                div()
                    .w_full()
                    .h(px(12.))
                    .on_click(move |cx| cx.set(target, 1)),
            )
            .child(
                div()
                    .absolute()
                    .right(px(2.))
                    .top(px(1.))
                    .size(px(8.))
                    .bg(rgb(0x44aa88))
                    .on_click(move |cx| cx.set(target, 2)),
            )
    }))?;
    let click = |ui: &mut Ui, x| -> Result<i32> {
        let p = sample(1, x, 3.);
        ui.dispatch_pointer(view, Down(p, Primary))?;
        ui.dispatch_pointer(view, Up(p, Primary))?;
        Ok(*ui.context().read(target)?)
    };
    accept(&mut ui, view, 40.)?;
    assert_eq!(click(&mut ui, 31.)?, 2);
    ui.layout(
        view,
        Size {
            width: px(60.),
            height: px(24.),
        },
    )?;
    let frame = ui.record_frame(view)?;
    assert!(
        matches!(frame.draws(), [Draw::Fill { origin, .. }] if origin.x == px(50.) && origin.y == px(1.))
    );
    assert_eq!(click(&mut ui, 31.)?, 2);
    ui.accept_pointer_frame(frame.into_pointer_frame())?;
    assert_eq!(click(&mut ui, 31.)?, 1);
    assert_eq!(click(&mut ui, 51.)?, 2);
    Ok(())
}
