//! Retained row identity and scroll work through the window and frame boundaries.
use std::{cell::Cell, rc::Rc, time::Instant};

use anyhow::Result;
use solor::{
    IntoElement, Styled, Ui, ViewId, component, div,
    layout::{Origin, Size},
    platform::{WindowEvent, WindowHandler},
    pointer::{PointerButton, PointerId, PointerInput, PointerKind, PointerSample, ScrollDelta},
    px,
    raster::RasterScale,
    rgb, text, uniform_list, variable_list,
};
use solor_render_cpu::CpuWindow;

fn mouse() -> PointerSample {
    PointerSample {
        id: PointerId(1),
        kind: PointerKind::Mouse,
        position: Origin {
            x: px(2.0),
            y: px(2.0),
        },
        modifiers: Default::default(),
    }
}

fn wheel(y: f32) -> PointerInput {
    PointerInput::Scroll(
        mouse(),
        ScrollDelta {
            x: px(0.0),
            y: px(y),
        },
    )
}

fn submit(ui: &mut Ui, view: ViewId, height: f32) -> Result<Vec<solor::NodeId>> {
    ui.flush()?;
    ui.layout(
        view,
        Size {
            width: px(200.0),
            height: px(height),
        },
    )?;
    let frame = ui.record_frame(view)?;
    ui.accept_pointer_frame(frame.into_pointer_frame())?;
    Ok(ui.children(view, ui.root(view)?)?.collect())
}

#[test]
fn viewport_bounds_mounts_and_preserves_fractional_motion_at_the_end() -> Result<()> {
    for count in [100_000usize, 1_000_000] {
        let mut ui = Ui::new();
        let builds = Rc::new(Cell::new(0));
        let built = builds.clone();
        let selected = ui.context().signal(None);
        let length = ui.context().signal(count);
        let view = ui.mount(component(move |_| {
            uniform_list(length, px(51.0), move |key| {
                built.set(built.get() + 1);
                div().on_click(move |cx| cx.set(selected, Some(key)))
            })
        }))?;
        let initial = submit(&mut ui, view, 510.0)?;
        assert_eq!((initial.len(), builds.get()), (11, 11));
        ui.dispatch_pointer(view, wheel(51.0))?;
        let next = submit(&mut ui, view, 510.0)?;
        assert_eq!(&next[..11], initial.as_slice());
        ui.dispatch_pointer(view, wheel(51.0))?;
        let next2 = submit(&mut ui, view, 510.0)?;
        assert_eq!(&next2[..11], &next[1..]);
        assert_eq!(builds.get(), 13);
        ui.dispatch_pointer(view, PointerInput::Down(mouse(), PointerButton::Primary))?;
        ui.dispatch_pointer(view, wheel(1e9))?;
        let end = submit(&mut ui, view, 510.0)?;
        assert_eq!((end.len(), builds.get()), (11, 24));
        ui.dispatch_pointer(view, PointerInput::Up(mouse(), PointerButton::Primary))?;
        assert_eq!(*ui.context().read(selected)?, None);
        ui.dispatch_pointer(view, wheel(-0.25))?;
        let fractional = submit(&mut ui, view, 510.0)?;
        let last = *end.last().unwrap();
        assert_eq!(fractional.last(), Some(&last));
        assert_eq!(ui.geometry(view, last).unwrap().origin.y, px(459.25));
        for input in [
            PointerInput::Down(mouse(), PointerButton::Primary),
            PointerInput::Up(mouse(), PointerButton::Primary),
        ] {
            ui.dispatch_pointer(view, input)?;
        }
        assert_eq!(*ui.context().read(selected)?, Some(count - 10));
        ui.context().set(length, 2)?;
        let short = submit(&mut ui, view, 510.0)?;
        assert_eq!(short.len(), 2);
        assert_eq!(ui.geometry(view, short[0]).unwrap().origin.y, px(0.0));
        assert!(submit(&mut ui, view, 0.0)?.is_empty());
    }
    Ok(())
}

#[test]
fn keyed_reorder_resize_and_failed_update_preserve_retained_rows() -> Result<()> {
    let mut ui = Ui::new();
    let keys = ui.context().signal(vec!["A", "B", "C", "D"]);
    let view = ui.mount(component(move |_| uniform_list(keys, px(10.0), |_| div())))?;
    let before = submit(&mut ui, view, 30.0)?;
    ui.context().update_in_place(keys, |k| k.swap(0, 2))?;
    let reordered = submit(&mut ui, view, 30.0)?;
    assert_eq!(reordered, [before[2], before[1], before[0], before[3]]);
    ui.context().set(keys, vec!["C", "C"])?;
    assert!(submit(&mut ui, view, 30.0).is_err());
    ui.context().set(keys, vec!["D", "C"])?;
    assert_eq!(submit(&mut ui, view, 10.0)?, [before[3], before[2]]);
    ui.context().set(keys, Vec::new())?;
    assert!(submit(&mut ui, view, 10.0)?.is_empty());
    Ok(())
}

#[test]
fn a_failed_row_mount_releases_partial_rows_and_can_be_retried() -> Result<()> {
    let mut ui = Ui::new();
    let keys = ui.context().signal(vec![1usize]);
    let owned = Rc::new(Cell::new(None));
    let mounted = owned.clone();
    let view = ui.mount(component(move |_| {
        uniform_list(keys, px(12.0), move |key| {
            let mounted = mounted.clone();
            component(move |cx| {
                let local = cx.signal(key);
                if key == 2 {
                    mounted.set(Some(local));
                }
                let padding = if key == 3 { px(f32::NAN) } else { px(0.0) };
                div().child(div().padding(padding))
            })
        })
    }))?;
    let initial = submit(&mut ui, view, 48.0)?;
    ui.context().set(keys, vec![1, 2, 3])?;
    assert!(submit(&mut ui, view, 48.0).is_err());
    assert!(ui.context().read(owned.get().unwrap()).is_err());
    assert_eq!(
        ui.children(view, ui.root(view)?)?.collect::<Vec<_>>(),
        initial
    );
    ui.context().set(keys, vec![2, 1])?;
    let rows = submit(&mut ui, view, 48.0)?;
    assert_eq!(rows[1], initial[0]);
    assert_eq!(*ui.context().read(owned.get().unwrap())?, 2);
    Ok(())
}

#[test]
#[ignore = "release-profile offscreen CPU frame measurement"]
fn compare_uniform_list_scroll_frames() -> Result<()> {
    scroll_frames(false)
}

#[test]
#[ignore = "release-profile offscreen CPU frame measurement"]
fn compare_variable_list_scroll_frames() -> Result<()> {
    scroll_frames(true)
}

fn scroll_frames(variable: bool) -> Result<()> {
    let size = Size {
        width: px(320.0),
        height: px(480.0),
    };
    for count in [100_000usize, 1_000_000] {
        let font = include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf");
        let mut window = CpuWindow::mount(
            Ui::with_font(font.to_vec())?,
            component(move |_| {
                let row = |key: usize| {
                    div()
                        .flex_row()
                        .justify_between()
                        .bg(rgb(if key.is_multiple_of(2) {
                            0x19242c
                        } else {
                            0x101922
                        }))
                        .child(text(format!("Item {key:07}")))
                        .child(text("24.00"))
                };
                if variable {
                    variable_list(count, px(32.0), move |key| {
                        row(key).py(px(if key.is_multiple_of(2) { 2.0 } else { 12.0 }))
                    })
                    .into_element()
                } else {
                    uniform_list(count, px(24.0), row).into_element()
                }
            }),
        )?;
        let frame = |window: &mut CpuWindow| -> Result<()> {
            std::hint::black_box(window.frame(size, RasterScale::ONE)?.rgba);
            window.frame_submitted()
        };
        frame(&mut window)?;
        for jump in [false, true] {
            let mut times = Vec::with_capacity(240);
            for i in 0..272 {
                let delta = if jump {
                    if i % 2 == 0 { 1e9 } else { -1e9 }
                } else {
                    24.0
                };
                let start = Instant::now();
                window.event(WindowEvent::Pointer(wheel(delta)))?;
                frame(&mut window)?;
                if i >= 32 {
                    times.push(start.elapsed());
                }
            }
            times.sort_unstable();
            eprintln!(
                "{count} rows, jump={jump}, CPU frame p95 {:?}, p99 {:?}",
                times[227], times[237]
            );
        }
    }
    Ok(())
}
