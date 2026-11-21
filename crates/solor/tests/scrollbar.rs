//! Scrollbar geometry and pointer routing through submitted frames.
use anyhow::Result;
use solor::{
    Draw, ListScrollHandle, ScrollbarThumbStyle, Styled, Ui, ViewId, component, div,
    layout::{Origin, Size},
    pointer::{PointerButton, PointerId, PointerInput, PointerKind, PointerSample, ScrollDelta},
    px, rgb, scrollbar, uniform_list,
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
    ui.accept_pointer_frame(frame.into_pointer_frame())?;
    Ok(())
}

fn point(x: f32, y: f32) -> PointerSample {
    PointerSample {
        id: PointerId(1),
        kind: PointerKind::Mouse,
        position: Origin { x: px(x), y: px(y) },
        modifiers: Default::default(),
    }
}

#[test]
fn custom_thumb_pages_drags_and_releases_capture_on_cancel() -> Result<()> {
    let mut ui = Ui::new();
    let list = ListScrollHandle::new();
    let handle = list.clone();
    let count = ui.context().signal(30usize);
    let view = ui.mount(component(move |_| {
        div()
            .flex_row()
            .items_stretch()
            .child(
                uniform_list(count, px(10.0), |_| div())
                    .scroll_handle(&handle)
                    .flex_1(),
            )
            .child(
                scrollbar(&handle)
                    .w(px(20.0))
                    .p(px(0.0))
                    .bg(rgb(0xffeeef))
                    .thumb_style(ScrollbarThumbStyle {
                        fill: rgb(0x112233).into(),
                        hover_fill: rgb(0x223344).into(),
                        drag_fill: rgb(0x334455).into(),
                        idle_width: px(20.0),
                        radius: px(3.0),
                        min_length: px(20.0),
                    }),
            )
    }))?;
    let thumb = |ui: &Ui, color| -> Result<_> {
        Ok(ui
            .record_frame(view)?
            .draws()
            .iter()
            .find_map(|draw| match draw {
                Draw::Fill {
                    origin,
                    size,
                    fill,
                    corner_radii,
                    ..
                } if *fill == rgb(color).into() => {
                    Some((origin.y, size.width, size.height, corner_radii.top_left))
                }
                _ => None,
            }))
    };
    submit(&mut ui, view, 120.0)?;
    assert_eq!(
        thumb(&ui, 0x112233)?,
        Some((px(0.0), px(20.0), px(48.0), px(3.0)))
    );
    let page = point(110.0, 100.0);
    ui.dispatch_pointer(view, PointerInput::Move(page))?;
    ui.dispatch_pointer(view, PointerInput::Down(page, PointerButton::Primary))?;
    ui.dispatch_pointer(view, PointerInput::Up(page, PointerButton::Primary))?;
    submit(&mut ui, view, 120.0)?;
    submit(&mut ui, view, 120.0)?;
    assert_eq!(list.position().unwrap().offset_px, 120.0);
    assert_eq!(thumb(&ui, 0x223344)?.unwrap().0, px(48.0));
    // Pick up the thumb near its lower edge and move vertically outside the track.
    ui.dispatch_pointer(
        view,
        PointerInput::Down(point(110.0, 90.0), PointerButton::Primary),
    )?;
    ui.dispatch_pointer(view, PointerInput::Move(point(50.0, 81.0)))?;
    submit(&mut ui, view, 120.0)?;
    assert_eq!(list.position().unwrap().offset_px, 97.5);
    assert!(thumb(&ui, 0x334455)?.is_some());
    ui.dispatch_pointer(view, PointerInput::Cancel(PointerId(1)))?;
    ui.dispatch_pointer(view, PointerInput::Move(point(50.0, 5.0)))?;
    submit(&mut ui, view, 120.0)?;
    assert_eq!(list.position().unwrap().offset_px, 97.5);
    assert!(thumb(&ui, 0x334455)?.is_none());
    let wheel = PointerInput::Scroll(
        point(110.0, 60.0),
        ScrollDelta {
            x: px(0.0),
            y: px(4.5),
        },
    );
    ui.dispatch_pointer(
        view,
        PointerInput::Scroll(
            point(40.0, 60.0),
            ScrollDelta {
                x: px(0.0),
                y: px(4.5),
            },
        ),
    )?;
    ui.dispatch_pointer(view, wheel)?;
    submit(&mut ui, view, 120.0)?;
    assert_eq!(list.position().unwrap().offset_px, 106.5);
    list.scroll_to_percent(100.0)?;
    submit(&mut ui, view, 60.0)?;
    submit(&mut ui, view, 60.0)?;
    let (_, _, length, _) = thumb(&ui, 0x112233)?.or(thumb(&ui, 0x223344)?).unwrap();
    assert_eq!(length, px(20.0));
    ui.context().set(count, 2)?;
    submit(&mut ui, view, 60.0)?;
    submit(&mut ui, view, 60.0)?;
    assert!(thumb(&ui, 0x112233)?.or(thumb(&ui, 0x223344)?).is_none());
    assert!(ui.next_redraw(view).is_none());
    Ok(())
}

#[test]
fn measured_list_and_short_track_share_wheel_distance_with_the_parent() -> Result<()> {
    use std::{cell::Cell, rc::Rc};
    let mut ui = Ui::new();
    let list = ListScrollHandle::new();
    let handle = list.clone();
    let unused = Rc::new(Cell::new(None));
    let observed = unused.clone();
    let view = ui.mount(component(move |_| {
        div()
            .flex_row()
            .items_stretch()
            .on_scroll(move |event, _| {
                observed.set(Some(event.delta));
                Ok(())
            })
            .child(
                solor::variable_list(3usize, px(50.0), |i: usize| {
                    div().h(px([40.0, 50.0, 60.0][i]))
                })
                .scroll_handle(&handle)
                .flex_1(),
            )
            .child(scrollbar(&handle).h(px(60.0)).p(px(0.0)))
    }))?;
    submit(&mut ui, view, 120.0)?;
    list.scroll_to_percent(100.0)?;
    submit(&mut ui, view, 120.0)?;
    submit(&mut ui, view, 120.0)?;
    let pos = list.position().unwrap();
    assert_eq!(pos.offset_px, 30.0);
    assert!(!pos.is_estimated);
    let expected = (60.0_f64 * 120.0 / 150.0) as f32;
    let thumb_height = ui
        .record_frame(view)?
        .draws()
        .iter()
        .find_map(|cmd| match cmd {
            Draw::Fill { size, fill, .. } if *fill == rgb(0x999999).into() => Some(size.height),
            _ => None,
        });
    assert_eq!(thumb_height, Some(px(expected)));
    let delta = ScrollDelta {
        x: px(2.5),
        y: px(-60.0),
    };
    ui.dispatch_pointer(view, PointerInput::Scroll(point(115.0, 20.0), delta))?;
    assert_eq!(
        unused.get(),
        Some(ScrollDelta {
            x: px(2.5),
            y: px(-30.0)
        })
    );
    submit(&mut ui, view, 120.0)?;
    assert_eq!(list.position().unwrap().offset_px, 0.0);
    Ok(())
}

#[test]
fn slim_thumb_and_auto_hide_share_a_stable_drag_target() -> Result<()> {
    for auto_hide in [false, true] {
        let mut ui = Ui::new();
        let handle = ListScrollHandle::new();
        let view = ui.mount(component(move |_| {
            div()
                .flex_row()
                .items_stretch()
                .child(
                    uniform_list(30usize, px(10.0), |_| div())
                        .scroll_handle(&handle)
                        .flex_1(),
                )
                .child(scrollbar(&handle).auto_hide(auto_hide))
        }))?;
        let fills = |ui: &Ui| -> Result<Vec<_>> {
            Ok(ui
                .record_frame(view)?
                .draws()
                .iter()
                .filter_map(|draw| match draw {
                    Draw::Fill { origin, size, .. } => Some((origin.x, size.width)),
                    _ => None,
                })
                .collect())
        };
        submit(&mut ui, view, 120.0)?;
        let resting = if auto_hide {
            vec![]
        } else {
            vec![(px(115.0), px(3.0))]
        };
        assert_eq!(fills(&ui)?, resting);
        // Enter the transparent part of the target, outside the resting thumb.
        let grab = point(111.0, 12.0);
        ui.dispatch_pointer(view, PointerInput::Move(grab))?;
        submit(&mut ui, view, 120.0)?;
        let expanded = vec![(px(110.0), px(8.0)); 2];
        assert_eq!(fills(&ui)?, expanded);
        ui.dispatch_pointer(view, PointerInput::Down(grab, PointerButton::Primary))?;
        let outside = point(60.0, 48.0);
        ui.dispatch_pointer(view, PointerInput::Move(outside))?;
        submit(&mut ui, view, 120.0)?;
        assert_eq!(fills(&ui)?, expanded);
        ui.dispatch_pointer(view, PointerInput::Up(outside, PointerButton::Primary))?;
        submit(&mut ui, view, 120.0)?;
        assert_eq!(fills(&ui)?, resting);
        assert!(ui.next_redraw(view).is_none());
    }
    Ok(())
}

#[test]
fn custom_painter_uses_application_visibility_and_the_interactive_thumb_interval() -> Result<()> {
    let mut ui = Ui::new();
    let handle = ListScrollHandle::new();
    let visible = ui.context().signal(false);
    let view = ui.mount(component(move |_| {
        div()
            .flex_row()
            .items_stretch()
            .child(
                uniform_list(30usize, px(10.0), |_| div())
                    .scroll_handle(&handle)
                    .flex_1(),
            )
            .child(
                scrollbar(&handle)
                    .w(px(20.0))
                    .p(px(0.0))
                    .paint_with(move |cx, bar| {
                        if *cx.read(visible)? {
                            let radius = px(if bar.dragging { 4.0 } else { 2.0 });
                            cx.fill_circle(
                                Origin {
                                    x: cx.size.width / 2.0,
                                    y: bar.thumb_top + bar.thumb_length / 2.0,
                                },
                                radius,
                                rgb(0x123456),
                            )?;
                        }
                        Ok(())
                    }),
            )
    }))?;
    submit(&mut ui, view, 120.0)?;
    assert!(ui.record_frame(view)?.draws().is_empty());
    ui.context().set(visible, true)?;
    submit(&mut ui, view, 120.0)?;
    let dot = |ui: &Ui| -> Result<_> {
        let frame = ui.record_frame(view)?;
        let [
            Draw::Fill {
                origin, size, fill, ..
            },
        ] = frame.draws()
        else {
            anyhow::bail!("expected the custom dot alone");
        };
        assert_eq!(*fill, rgb(0x123456).into());
        Ok((origin.y + size.height / 2.0, size.width))
    };
    assert_eq!(dot(&ui)?, (px(24.0), px(4.0)));
    ui.dispatch_pointer(
        view,
        PointerInput::Down(point(110.0, 24.0), PointerButton::Primary),
    )?;
    ui.dispatch_pointer(view, PointerInput::Move(point(70.0, 54.0)))?;
    submit(&mut ui, view, 120.0)?;
    submit(&mut ui, view, 120.0)?;
    assert_eq!(dot(&ui)?, (px(54.0), px(8.0)));
    Ok(())
}
