//! Container scrolling through the normal CPU window event and presentation boundary.
use std::{cell::Cell, rc::Rc};

use anyhow::Result;
use solor::{
    Styled, Ui, component, div,
    layout::{Origin, Size},
    platform::{WindowEvent, WindowHandler},
    pointer::{PointerButton, PointerId, PointerInput, PointerKind, PointerSample, ScrollDelta},
    px,
    raster::RasterScale,
    rgb,
    style::Overflow,
};
use solor_render_cpu::CpuWindow;

fn mouse(x: f32, y: f32) -> PointerSample {
    PointerSample {
        id: PointerId(1),
        kind: PointerKind::Mouse,
        position: Origin { x: px(x), y: px(y) },
        modifiers: Default::default(),
    }
}

fn wheel(window: &mut CpuWindow, p: PointerSample, x: f32, y: f32) -> Result<bool> {
    Ok(window
        .event(WindowEvent::Pointer(PointerInput::Scroll(
            p,
            ScrollDelta { x: px(x), y: px(y) },
        )))?
        .redraw)
}

fn click(window: &mut CpuWindow, p: PointerSample) -> Result<()> {
    for input in [
        PointerInput::Down(p, PointerButton::Primary),
        PointerInput::Up(p, PointerButton::Primary),
    ] {
        window.event(WindowEvent::Pointer(input))?;
    }
    Ok(())
}

#[test]
fn nested_lists_scroll_across_one_boundary_and_publish_hits_with_the_pixels() -> Result<()> {
    let selected = Rc::new(Cell::new(0));
    let observed = selected.clone();
    let mut window = CpuWindow::mount(
        Ui::new(),
        component(move |cx| {
            let extra = cx.signal(true);
            let row = move |id, color| {
                let observed = observed.clone();
                div()
                    .w_full()
                    .min_h(px(8.0))
                    .h(px(8.0))
                    .bg(rgb(color))
                    .on_click(move |cx| {
                        observed.set(id);
                        if id > 1 {
                            cx.set(extra, id == 2)?;
                        }
                        Ok(())
                    })
            };
            let first = row(1, 0xff0000);
            let second = row(2, 0x00ff00);
            div()
                .items_stretch()
                .overflow_y(Overflow::Auto)
                .child(
                    div()
                        .items_stretch()
                        .h(px(12.0))
                        .overflow_y(Overflow::Scroll)
                        .child(first)
                        .child(second)
                        .child_when(extra, move || row(3, 0x0000ff)),
                )
                .child(div().min_h(px(16.0)).bg(rgb(0xffffff)))
        }),
    )?;
    let size = Size {
        width: px(20.0),
        height: px(16.0),
    };
    let prepare = |window: &mut CpuWindow| -> Result<[u8; 4]> {
        let frame = window.frame(size, RasterScale::ONE)?;
        let offset = (4 * frame.size.width as usize + 2) * 4;
        Ok(frame.rgba[offset..offset + 4].try_into().unwrap())
    };
    assert_eq!(prepare(&mut window)?, [255, 0, 0, 255]);
    window.frame_submitted()?;
    let p = mouse(2.0, 4.0);
    // A queued frame must not discard input received between recording and submission.
    assert!(wheel(&mut window, p, 0.0, 6.5)?);
    prepare(&mut window)?;
    assert!(wheel(&mut window, p, 0.0, 9.5)?);
    window.frame_submitted()?;
    assert!(window.next_redraw().is_some());
    assert_eq!(prepare(&mut window)?, [0, 0, 255, 255]);
    click(&mut window, p)?;
    assert_eq!(selected.get(), 2);
    window.frame_submitted()?;
    assert!(window.next_redraw().is_none());
    click(&mut window, p)?;
    assert_eq!(selected.get(), 3);

    // Removing the last row clamps the inner offset; restoring it keeps that offset.
    assert_eq!(prepare(&mut window)?, [0, 255, 0, 255]);
    window.frame_submitted()?;
    click(&mut window, p)?;
    assert_eq!(selected.get(), 2);
    assert_eq!(prepare(&mut window)?, [0, 255, 0, 255]);
    window.frame_submitted()?;
    assert!(wheel(&mut window, p, 0.0, -100.0)?);
    assert_eq!(prepare(&mut window)?, [255, 0, 0, 255]);
    window.frame_submitted()?;
    assert!(!wheel(&mut window, p, 0.0, -1.0)?);
    Ok(())
}

#[test]
fn horizontal_strip_forwards_diagonal_remainder_and_resets_a_disabled_axis() -> Result<()> {
    let mut ui = Ui::new();
    let policy = ui.context().signal(Overflow::Auto);
    let forwarded = ui.context().signal(Vec::new());
    let view = ui.mount(component(|_| {
        div()
            .items_stretch()
            .on_scroll(move |event, cx| cx.update(forwarded, |values| values.push(event.delta)))
            .child(
                div()
                    .flex_row()
                    .overflow_x(policy)
                    .h(px(10.0))
                    .child(div().min_w(px(16.0)).bg(rgb(0xff0000)))
                    .child(div().min_w(px(16.0)).bg(rgb(0x00ff00))),
            )
    }))?;
    let size = Size {
        width: px(20.0),
        height: px(10.0),
    };
    let submit = |ui: &mut Ui| -> Result<_> {
        ui.flush()?;
        ui.layout(view, size)?;
        let frame = ui.record_frame(view)?;
        let x = frame
            .draws()
            .iter()
            .find_map(|draw| match draw {
                solor::Draw::Fill { origin, .. } => Some(origin.x),
                _ => None,
            })
            .expect("first strip item");
        ui.accept_pointer_frame(frame.into_pointer_frame())?;
        Ok(x)
    };
    let delta = |x, y| ScrollDelta { x: px(x), y: px(y) };
    let scroll = |d| PointerInput::Scroll(mouse(4.0, 4.0), d);
    assert_eq!(submit(&mut ui)?, px(0.0));
    ui.dispatch_pointer(view, scroll(delta(7.25, -2.5)))?;
    assert_eq!(submit(&mut ui)?, px(-7.25));
    ui.dispatch_pointer(view, scroll(delta(10.0, 0.5)))?;
    assert_eq!(submit(&mut ui)?, px(-12.0));
    assert_eq!(
        *ui.context().read(forwarded)?,
        [delta(0.0, -2.5), delta(5.25, 0.5)]
    );
    ui.context().set(policy, Overflow::Hidden)?;
    assert_eq!(submit(&mut ui)?, px(0.0));
    ui.context().set(policy, Overflow::Scroll)?;
    assert_eq!(submit(&mut ui)?, px(0.0));
    Ok(())
}
