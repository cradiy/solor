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
                    .items_stretch()
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

#[test]
fn scrollbar_gutters_share_viewport_geometry_with_scrolling_paint_and_hits() -> Result<()> {
    use solor_render_cpu::pixels::Surface;

    let mut ui = Ui::new();
    let gutter = ui.context().signal(px(4.5));
    let policy = ui.context().signal(Overflow::Scroll);
    let clicks = ui.context().signal(0);
    let view = ui.mount(component(|_| {
        div()
            .overflow_x_scroll()
            .overflow_y(policy)
            .scrollbar_width(gutter)
            .child(div().w_full().h(px(32.)).bg(rgb(0x123456)))
            .child(div().absolute().right(px(0.)).bottom(px(0.)).size(px(2.)))
            .child(
                div()
                    .absolute()
                    .left(px(0.))
                    .top(px(0.))
                    .w(px(50.))
                    .h(px(36.))
                    .bg(rgb(0xff0000))
                    .on_click(move |cx| cx.update(clicks, |n| *n += 1)),
            )
    }))?;
    let root = ui.root(view)?;
    let children: Vec<_> = ui.children(view, root)?.collect();
    let size = Size {
        width: px(40.),
        height: px(24.),
    };
    for (width, axis, right) in [
        (4.5, Overflow::Scroll, 4.5),
        (4.5, Overflow::Auto, 0.),
        (6.25, Overflow::Scroll, 6.25),
    ] {
        if width != 4.5 {
            ui.context().set(gutter, px(width))?;
        }
        ui.context().set(policy, axis)?;
        ui.flush()?;
        ui.layout(view, size)?;
        let viewport = Size {
            width: px(40. - right),
            height: px(24. - width),
        };
        let m = ui.scroll_metrics(view, root).unwrap();
        assert_eq!(m.viewport, viewport);
        assert_eq!(
            m.max_offset,
            Size {
                width: px(50.) - viewport.width,
                height: px(36.) - viewport.height
            }
        );
        assert_eq!(
            ui.geometry(view, children[0]).unwrap().metrics.size.width,
            viewport.width
        );
        assert_eq!(
            ui.geometry(view, children[1]).unwrap().origin,
            Origin {
                x: viewport.width - px(2.),
                y: viewport.height - px(2.)
            }
        );
        ui.set_scroll_offset(
            view,
            root,
            Origin {
                x: px(100.),
                y: px(100.),
            },
        )?;
        let frame = ui.record_frame(view)?;
        let scale = RasterScale(1.5);
        let image = Surface::draw_view(&frame, scale.buffer_size(size)?, scale)?;
        let pixel = |x, y| {
            let i: usize = 4 * (y * image.size.width as usize + x);
            &image.rgba[i..i + 3]
        };
        assert_eq!(pixel(10, 10), [255, 0, 0]);
        assert_eq!(pixel(10, 34), [255; 3]);
        assert_eq!(
            pixel(58, 10),
            if right > 0. { [255; 3] } else { [255, 0, 0] }
        );
        ui.accept_pointer_frame(frame.into_pointer_frame())?;
        let before = *ui.context().read(clicks)?;
        for position in [mouse(38., 6.), mouse(6., 23.), mouse(6., 6.)] {
            ui.dispatch_pointer(view, PointerInput::Down(position, PointerButton::Primary))?;
            ui.dispatch_pointer(view, PointerInput::Up(position, PointerButton::Primary))?;
        }
        assert_eq!(
            *ui.context().read(clicks)? - before,
            if right > 0. { 1 } else { 2 }
        );
    }
    Ok(())
}

#[test]
fn a_scrolled_view_uses_the_new_row_bounds_after_a_height_change() -> Result<()> {
    use solor::raster::DeviceSize;
    use solor_render_cpu::pixels::Surface;

    let mut ui = Ui::new();
    let height = ui.context().signal(px(4.0));
    let row = |color| div().w_full().min_h(px(4.0)).bg(rgb(color));
    let view = ui.mount(component(|_| {
        div()
            .overflow_y_scroll()
            .child(div().w_full().min_h(height).bg(rgb(0xff0000)))
            .child(row(0x00ff00))
            .child(row(0x0000ff))
            .child(row(0xff00ff))
    }))?;
    let root = ui.root(view)?;
    let size = DeviceSize {
        width: 8,
        height: 4,
    };
    let top_pixel = |ui: &mut Ui| -> Result<[u8; 4]> {
        ui.flush()?;
        ui.layout(view, size.at_unit_scale())?;
        let frame = ui.record_frame(view)?;
        let image = Surface::draw_view(&frame, size, RasterScale::ONE)?;
        ui.accept_pointer_frame(frame.into_pointer_frame())?;
        Ok(image.rgba[..4].try_into()?)
    };
    assert_eq!(top_pixel(&mut ui)?, [255, 0, 0, 255]);
    ui.set_scroll_offset(
        view,
        root,
        Origin {
            x: px(0.0),
            y: px(8.0),
        },
    )?;
    assert_eq!(top_pixel(&mut ui)?, [0, 0, 255, 255]);
    ui.context().set(height, px(6.5))?;
    assert_eq!(top_pixel(&mut ui)?, [0, 255, 0, 255]);
    ui.set_scroll_offset(
        view,
        root,
        Origin {
            x: px(0.0),
            y: px(14.5),
        },
    )?;
    assert_eq!(top_pixel(&mut ui)?, [255, 0, 255, 255]);
    ui.set_scroll_offset(view, root, Origin::default())?;
    assert_eq!(top_pixel(&mut ui)?, [255, 0, 0, 255]);
    Ok(())
}

#[test]
fn overflowing_row_children_keep_their_pixels_and_inherited_click_targets() -> Result<()> {
    use solor::raster::DeviceSize;
    use solor_render_cpu::pixels::Surface;

    let mut ui = Ui::new();
    let selected = ui.context().signal(None);
    let view = ui.mount(component(|_| {
        div().flex_col().gap(px(16.)).overflow_y_scroll().children(
            [(0., 12., 0xff0000), (6., -11., 0x00ff00)]
                .into_iter()
                .enumerate()
                .map(|(index, (x, y, color))| {
                    div()
                        .relative()
                        .w_full()
                        .min_h(px(8.))
                        .on_click(move |cx| cx.set(selected, Some(index)))
                        .child(
                            div()
                                .absolute()
                                .left(px(x))
                                .top(px(y))
                                .size(px(4.))
                                .bg(rgb(color)),
                        )
                }),
        )
    }))?;
    let size = DeviceSize {
        width: 12,
        height: 8,
    };
    ui.layout(view, size.at_unit_scale())?;
    ui.set_scroll_offset(
        view,
        ui.root(view)?,
        Origin {
            x: px(0.),
            y: px(12.5),
        },
    )?;
    let frame = ui.record_frame(view)?;
    let image = Surface::draw_view(&frame, size, RasterScale::ONE)?;
    ui.accept_pointer_frame(frame.into_pointer_frame())?;
    for (x, index, color) in [(1, 0, [255, 0, 0]), (7, 1, [0, 255, 0])] {
        let offset = (size.width as usize * 2 + x) * 4;
        assert_eq!(image.rgba[offset..offset + 3], color);
        let point = mouse(x as f32, 2.);
        ui.dispatch_pointer(view, PointerInput::Down(point, PointerButton::Primary))?;
        ui.dispatch_pointer(view, PointerInput::Up(point, PointerButton::Primary))?;
        assert_eq!(*ui.context().read(selected)?, Some(index));
    }
    Ok(())
}
