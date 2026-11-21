//! One-shot scroll requests through retained layout and accepted pointer frames.
use std::{cell::Cell, rc::Rc};

use anyhow::Result;
use solor::{
    ListScrollHandle,
    ScrollAlignment::{Bottom, Center, Nearest, Top},
    Styled, Ui, ViewId, component, div,
    layout::{Origin, Size},
    pointer::{PointerButton, PointerId, PointerInput, PointerKind, PointerSample, ScrollDelta},
    px, uniform_list, variable_list,
};

fn submit(ui: &mut Ui, view: ViewId, height: f32) -> Result<()> {
    ui.flush()?;
    ui.layout(
        view,
        Size {
            width: px(120.0),
            height: px(height),
        },
    )?;
    let frame = ui.record_frame(view)?;
    ui.accept_pointer_frame(frame.into_pointer_frame())
}

fn mouse(y: f32) -> PointerSample {
    PointerSample {
        id: PointerId(1),
        kind: PointerKind::Mouse,
        position: Origin {
            x: px(2.0),
            y: px(y),
        },
        modifiers: Default::default(),
    }
}

fn click(ui: &mut Ui, view: ViewId, y: f32) -> Result<()> {
    let p = mouse(y);
    for input in [
        PointerInput::Down(p, PointerButton::Primary),
        PointerInput::Up(p, PointerButton::Primary),
    ] {
        ui.dispatch_pointer(view, input)?;
    }
    Ok(())
}

#[test]
fn a_handle_jumps_from_a_row_callback_and_publishes_the_matching_hits() -> Result<()> {
    let mut ui = Ui::new();
    let handle = ListScrollHandle::new();
    let rows = handle.clone();
    let selected = ui.context().signal(None);
    let built = Rc::new(Cell::new(0));
    let builds = built.clone();
    let target = 900_123;
    let view = ui.mount(component(move |_| {
        let actions = rows.clone();
        uniform_list(1_000_000usize, px(20.0), move |key| {
            builds.set(builds.get() + 1);
            let actions = actions.clone();
            div().on_click(move |cx| {
                if key == 0 {
                    actions.scroll_to(target, Center);
                }
                cx.set(selected, Some(key))
            })
        })
        .scroll_handle(&rows)
    }))?;
    submit(&mut ui, view, 100.0)?;
    click(&mut ui, view, 5.0)?;
    assert!(ui.next_redraw(view).is_some());
    submit(&mut ui, view, 100.0)?;
    click(&mut ui, view, 40.25)?;
    assert_eq!(*ui.context().read(selected)?, Some(target));
    assert_eq!(built.get(), 13);
    handle.scroll_to(usize::MAX, Top);
    submit(&mut ui, view, 100.0)?;
    click(&mut ui, view, 99.75)?;
    assert_eq!(*ui.context().read(selected)?, Some(999_999));
    Ok(())
}

#[test]
fn variable_row_alignment_uses_measured_height_and_yields_to_wheel_input() -> Result<()> {
    let mut ui = Ui::new();
    let handle = ListScrollHandle::new();
    let list = handle.clone();
    let target = 800_003;
    let built = Rc::new(Cell::new(0));
    let builds = built.clone();
    let view = ui.mount(component(move |_| {
        variable_list(1_000_000usize, px(18.0), move |i| {
            builds.set(builds.get() + 1);
            div().h(px(if i == target { 123.5 } else { 24.25 }))
        })
        .scroll_handle(&list)
    }))?;
    let origin = |ui: &Ui| -> Result<_> {
        let node = ui
            .children(view, ui.root(view)?)?
            .find(|id| ui.geometry(view, *id).unwrap().metrics.size.height == px(123.5))
            .expect("target mounted");
        Ok(ui.geometry(view, node).unwrap().origin.y)
    };
    // The most recent request before the first allocation is the only destination built.
    handle.scroll_to(350_000, Top);
    handle.scroll_to(target, Bottom);
    submit(&mut ui, view, 100.0)?;
    assert_eq!(origin(&ui)?, px(-23.5));
    assert!(built.get() < 20);
    for (alignment, y) in [(Center, -11.75), (Nearest, -11.75), (Top, 0.0)] {
        handle.scroll_to(target, alignment);
        submit(&mut ui, view, 100.0)?;
        assert_eq!(origin(&ui)?, px(y));
    }
    let wheel = PointerInput::Scroll(
        mouse(10.0),
        ScrollDelta {
            x: px(0.0),
            y: px(4.5),
        },
    );
    handle.scroll_to(0, Top);
    ui.dispatch_pointer(view, wheel)?;
    submit(&mut ui, view, 100.0)?;
    assert_eq!(origin(&ui)?, px(-4.5));
    ui.dispatch_pointer(view, wheel)?;
    submit(&mut ui, view, 100.0)?;
    assert_eq!(origin(&ui)?, px(-9.0));
    for (from, expected) in [(usize::MAX, -23.5), (0, 0.0)] {
        handle.scroll_to(from, Top);
        submit(&mut ui, view, 100.0)?;
        handle.scroll_to(target, Nearest);
        submit(&mut ui, view, 100.0)?;
        assert_eq!(origin(&ui)?, px(expected));
    }
    Ok(())
}

#[test]
fn deferred_requests_survive_empty_viewports_and_handles_can_be_reattached() -> Result<()> {
    let mut ui = Ui::new();
    let handle = ListScrollHandle::new();
    handle.scroll_to(40, Top);
    let count = ui.context().signal(0usize);
    let selected = ui.context().signal(None);
    let mount = |ui: &mut Ui| {
        let list = handle.clone();
        ui.mount(component(move |_| {
            uniform_list(count, px(20.0), move |key| {
                div().on_click(move |cx| cx.set(selected, Some(key)))
            })
            .scroll_handle(&list)
        }))
    };
    let view = mount(&mut ui)?;
    assert!(mount(&mut ui).is_err());
    submit(&mut ui, view, 100.0)?;
    assert!(ui.next_redraw(view).is_none());
    ui.context().set(count, 100)?;
    submit(&mut ui, view, 0.0)?;
    assert!(ui.next_redraw(view).is_none());
    submit(&mut ui, view, 100.0)?;
    click(&mut ui, view, 1.0)?;
    assert_eq!(*ui.context().read(selected)?, Some(40));
    assert_eq!(handle.position().unwrap().offset_px, 800.0);
    ui.unmount(view)?;
    assert!(handle.position().is_none());
    let other = mount(&mut ui)?;
    assert!(handle.position().is_none());
    submit(&mut ui, other, 100.0)?;
    click(&mut ui, other, 1.0)?;
    assert_eq!(*ui.context().read(selected)?, Some(0));
    Ok(())
}

#[test]
fn a_failed_destination_can_be_retried_without_reissuing_the_request() -> Result<()> {
    let mut ui = Ui::new();
    let (handle, position) = ListScrollHandle::with_position_signal(&mut ui.context());
    let list = handle.clone();
    let selected = ui.context().signal(None);
    let height = ui.context().signal(px(0.0));
    let view = ui.mount(component(move |_| {
        variable_list(200usize, px(20.0), move |key| {
            let row = if key == 100 {
                div().h(height)
            } else {
                div().h(px(20.0))
            };
            row.on_click(move |cx| cx.set(selected, Some(key)))
        })
        .scroll_handle(&list)
    }))?;
    submit(&mut ui, view, 100.0)?;
    let before = handle.position().unwrap();
    handle.scroll_to(100, Bottom);
    assert!(submit(&mut ui, view, 100.0).is_err());
    assert_eq!(handle.position(), Some(before));
    assert_eq!(*ui.context().read(position)?, Some(before));
    ui.context().set(height, px(54.0))?;
    submit(&mut ui, view, 100.0)?;
    assert_eq!(*ui.context().read(position)?, handle.position());
    for (y, key) in [(45.75, 99), (46.0, 100), (99.75, 100)] {
        click(&mut ui, view, y)?;
        assert_eq!(*ui.context().read(selected)?, Some(key));
    }
    Ok(())
}

#[test]
fn a_position_subscriber_repaints_for_settled_changes_and_list_removal() -> Result<()> {
    let mut ui = Ui::new();
    let (handle, position) = ListScrollHandle::with_position_signal(&mut ui.context());
    let visible = ui.context().signal(true);
    let list = handle.clone();
    let view = ui.mount(component(move |_| {
        div().child_when(visible, move || {
            uniform_list(100usize, px(20.0), |_| div())
                .scroll_handle(&list)
                .w(px(120.0))
                .h(px(100.0))
        })
    }))?;
    let paints = Rc::new(Cell::new(0));
    let observed = Rc::new(Cell::new(None));
    let count = paints.clone();
    let last = observed.clone();
    let indicator = ui.mount(component(move |_| {
        div().paint(move |cx| {
            last.set(*cx.read(position)?);
            count.set(count.get() + 1);
            Ok(())
        })
    }))?;
    submit(&mut ui, indicator, 10.0)?;
    assert_eq!((paints.get(), observed.get()), (1, None));
    handle.scroll_to_percent(10.0)?;
    handle.scroll_to_percent(75.0)?;
    submit(&mut ui, view, 100.0)?;
    assert!(ui.next_redraw(view).is_none());
    assert!(ui.next_redraw(indicator).is_some());
    submit(&mut ui, indicator, 10.0)?;
    assert_eq!(paints.get(), 2);
    assert_eq!(observed.get().unwrap().percent(), 75.0);
    handle.scroll_to_percent(75.0)?;
    submit(&mut ui, view, 100.0)?;
    submit(&mut ui, indicator, 10.0)?;
    assert_eq!(paints.get(), 2);
    assert!(ui.next_redraw(indicator).is_none());
    ui.context().set(visible, false)?;
    submit(&mut ui, view, 100.0)?;
    submit(&mut ui, indicator, 10.0)?;
    assert_eq!((paints.get(), observed.get()), (3, None));
    ui.context().set(visible, true)?;
    submit(&mut ui, view, 100.0)?;
    submit(&mut ui, indicator, 10.0)?;
    assert_eq!(observed.get().unwrap().percent(), 0.0);
    ui.unmount(view)?;
    submit(&mut ui, indicator, 10.0)?;
    assert_eq!((paints.get(), observed.get()), (5, None));
    Ok(())
}

#[test]
fn percentage_and_pixel_positions_round_trip_through_layout_and_wheel_input() -> Result<()> {
    let mut ui = Ui::new();
    let (handle, position) = ListScrollHandle::with_position_signal(&mut ui.context());
    let list = handle.clone();
    let view = ui.mount(component(move |_| {
        uniform_list(1_000_000usize, px(50.0), |_| div()).scroll_handle(&list)
    }))?;
    handle.scroll_to_percent(75.0)?;
    assert!(handle.position().is_none());
    submit(&mut ui, view, 100.0)?;
    let initial = handle.position().unwrap();
    assert_eq!(initial.max_offset_px, 49_999_900.0);
    assert_eq!(initial.offset_px, initial.max_offset_px * 0.75);
    assert_eq!(initial.percent(), 75.0);
    assert!(!initial.is_estimated);
    handle.scroll_to_offset_px(40_000_000.25)?;
    assert!(handle.scroll_to_percent(f64::NAN).is_err());
    assert!(handle.scroll_to_offset_px(f64::INFINITY).is_err());
    assert_eq!(handle.position(), Some(initial));
    submit(&mut ui, view, 100.0)?;
    assert_eq!(handle.position().unwrap().offset_px, 40_000_000.25);
    let visible = ui.children(view, ui.root(view)?)?.nth(1).unwrap();
    assert_eq!(ui.geometry(view, visible).unwrap().origin.y, px(-0.25));
    ui.dispatch_pointer(
        view,
        PointerInput::Scroll(
            mouse(10.0),
            ScrollDelta {
                x: px(0.0),
                y: px(2.5),
            },
        ),
    )?;
    submit(&mut ui, view, 200.0)?;
    assert_eq!(handle.position().unwrap().offset_px, 40_000_002.75);
    assert_eq!(handle.position().unwrap().max_offset_px, 49_999_800.0);
    assert_eq!(*ui.context().read(position)?, handle.position());
    handle.scroll_to_percent(150.0)?;
    submit(&mut ui, view, 200.0)?;
    assert_eq!(handle.position().unwrap().percent(), 100.0);
    handle.scroll_to_offset_px(px(-4.0))?;
    submit(&mut ui, view, 200.0)?;
    assert_eq!(handle.position().unwrap().percent(), 0.0);
    submit(&mut ui, view, 0.0)?;
    assert!(handle.position().is_none());
    assert!(ui.context().read(position)?.is_none());
    ui.unmount(view)?;
    assert!(handle.position().is_none());
    Ok(())
}

#[test]
fn variable_progress_tracks_measurements_and_handles_empty_content() -> Result<()> {
    let mut ui = Ui::new();
    let handle = ListScrollHandle::new();
    let list = handle.clone();
    let count = ui.context().signal(0usize);
    let view = ui.mount(component(move |_| {
        variable_list(count, px(20.0), |i: usize| {
            div().h(px(if i.is_multiple_of(2) { 40.0 } else { 50.0 }))
        })
        .scroll_handle(&list)
    }))?;
    handle.scroll_to_percent(50.0)?;
    submit(&mut ui, view, 100.0)?;
    let empty = handle.position().unwrap();
    assert_eq!(
        (empty.offset_px, empty.max_offset_px, empty.percent()),
        (0.0, 0.0, 0.0)
    );
    assert!(!empty.is_estimated);
    ui.context().set(count, 3)?;
    submit(&mut ui, view, 100.0)?;
    let exact = handle.position().unwrap();
    assert_eq!(
        (exact.offset_px, exact.max_offset_px, exact.percent()),
        (15.0, 30.0, 50.0)
    );
    assert!(!exact.is_estimated);
    ui.context().set(count, 1_000_000)?;
    handle.scroll_to_percent(100.0)?;
    submit(&mut ui, view, 100.0)?;
    let end = handle.position().unwrap();
    assert!(end.is_estimated);
    assert_eq!(end.offset_px, end.max_offset_px);
    let last = ui.children(view, ui.root(view)?)?.last().unwrap();
    let g = ui.geometry(view, last).unwrap();
    assert_eq!(g.origin.y + g.metrics.size.height, px(100.0));
    handle.scroll_to_percent(40.0)?;
    submit(&mut ui, view, 100.0)?;
    let middle = handle.position().unwrap();
    assert!(middle.is_estimated);
    assert!((middle.percent() - 40.0).abs() < 0.001);
    assert!(ui.children(view, ui.root(view)?)?.len() < 10);
    Ok(())
}
