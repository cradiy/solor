//! Scrollbar geometry and pointer routing through submitted frames.
use anyhow::Result;
use solor::{
    Draw, ListScrollHandle, ScrollbarThumbStyle, Styled, Ui, ViewId, component, div,
    layout::{Origin, Size},
    pointer::{PointerButton, PointerId, PointerInput, PointerKind, PointerSample, ScrollDelta},
    px, rgb, scrollbar, uniform_list,
};

fn submit(ui: &mut Ui, view: ViewId, height: f32) -> Result<Vec<Draw>> {
    ui.flush()?;
    ui.layout(
        view,
        Size {
            width: px(120.0),
            height: px(height),
        },
    )?;
    let (draws, pointer) = ui.record_frame(view)?.into_parts();
    ui.accept_pointer_frame(pointer)?;
    Ok(draws)
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
    let painted = thumb(&ui, 0x112233)?.or(thumb(&ui, 0x223344)?).unwrap();
    assert_eq!(painted.0, px(42.6));
    assert!(
        ui.next_redraw(view).is_none(),
        "settled wheel input scheduled a second frame"
    );
    list.scroll_to_percent(100.0)?;
    submit(&mut ui, view, 60.0)?;
    let (_, _, length, _) = thumb(&ui, 0x112233)?.or(thumb(&ui, 0x223344)?).unwrap();
    assert_eq!(length, px(20.0));
    ui.context().set(count, 2)?;
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
                                    y: bar.thumb_start + bar.thumb_length / 2.0,
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
    assert_eq!(dot(&ui)?, (px(54.0), px(8.0)));
    Ok(())
}

#[test]
fn overflow_container_and_bar_share_one_offset_and_presented_hits() -> Result<()> {
    use solor::{ScrollHandle, style::Overflow};
    let mut ui = Ui::new();
    let handle = ScrollHandle::new();
    let tracked = handle.clone();
    let extra = ui.context().signal(false);
    let selected = ui.context().signal(None);
    let view = ui.mount(component(move |_| {
        let row = move |id| {
            div()
                .min_w(px(180.0))
                .min_h(px(50.0))
                .on_click(move |cx| cx.set(selected, Some(id)))
        };
        div()
            .flex_row()
            .items_stretch()
            .child(
                div()
                    .flex_1()
                    .overflow_x(Overflow::Auto)
                    .overflow_y_scroll()
                    .track_scroll(&tracked)
                    .children((0..4).map(row))
                    .child_when(extra, move || row(4)),
            )
            .child(
                scrollbar(&tracked)
                    .w(px(12.0))
                    .p(px(0.0))
                    .paint_with(|cx, bar| {
                        cx.fill_rect(
                            Origin {
                                x: px(0.0),
                                y: bar.thumb_start,
                            },
                            Size {
                                width: cx.size.width,
                                height: bar.thumb_length,
                            },
                            rgb(0x102030),
                        )
                    }),
            )
    }))?;
    submit(&mut ui, view, 100.0)?;
    let container = ui.children(view, ui.root(view)?)?.next().unwrap();
    let send_wheel = |ui: &mut Ui, x, dx, dy| -> Result<()> {
        ui.dispatch_pointer(
            view,
            PointerInput::Scroll(
                point(x, 16.0),
                ScrollDelta {
                    x: px(dx),
                    y: px(dy),
                },
            ),
        )?;
        Ok(())
    };
    send_wheel(&mut ui, 115.0, 0.0, 10.5)?;
    send_wheel(&mut ui, 10.0, 3.5, 5.25)?;
    send_wheel(&mut ui, 115.0, 0.0, 2.25)?;
    submit(&mut ui, view, 100.0)?;
    let metrics = handle.metrics().unwrap();
    assert_eq!(
        metrics.offset,
        Origin {
            x: px(3.5),
            y: px(18.0)
        }
    );
    assert_eq!(ui.scroll_metrics(view, container), Some(metrics));
    let thumb = |ui: &Ui| -> Result<_> {
        Ok(ui
            .record_frame(view)?
            .draws()
            .iter()
            .find_map(|d| match d {
                Draw::Fill {
                    origin, size, fill, ..
                } if *fill == rgb(0x102030).into() => Some((origin.y, size.height)),
                _ => None,
            })
            .unwrap())
    };
    assert_eq!(thumb(&ui)?, (px(9.0), px(50.0)));
    let grab = point(115.0, 12.0);
    ui.dispatch_pointer(view, PointerInput::Down(grab, PointerButton::Primary))?;
    let end = point(70.0, 160.0);
    ui.dispatch_pointer(view, PointerInput::Move(end))?;
    ui.dispatch_pointer(view, PointerInput::Up(end, PointerButton::Primary))?;
    ui.flush()?;
    ui.layout(
        view,
        Size {
            width: px(120.0),
            height: px(100.0),
        },
    )?;
    let frame = ui.record_frame(view)?;
    let click_row = |ui: &mut Ui| -> Result<()> {
        for input in [
            PointerInput::Down(point(4.0, 2.0), PointerButton::Primary),
            PointerInput::Up(point(4.0, 2.0), PointerButton::Primary),
        ] {
            ui.dispatch_pointer(view, input)?;
        }
        Ok(())
    };
    click_row(&mut ui)?;
    assert_eq!(*ui.context().read(selected)?, Some(0));
    ui.accept_pointer_frame(frame.into_pointer_frame())?;
    click_row(&mut ui)?;
    assert_eq!(*ui.context().read(selected)?, Some(2));
    assert_eq!(
        handle.metrics().unwrap().offset,
        Origin {
            x: px(3.5),
            y: px(100.0)
        }
    );
    ui.context().set(extra, true)?;
    submit(&mut ui, view, 100.0)?;
    assert_eq!(thumb(&ui)?, (px(40.0), px(40.0)));
    assert!(ui.next_redraw(view).is_none());
    send_wheel(&mut ui, 115.0, 0.0, 10.0)?;
    let target = Origin {
        x: px(2.0),
        y: px(25.0),
    };
    assert!(ui.set_scroll_offset(view, container, target)?);
    submit(&mut ui, view, 100.0)?;
    assert_eq!(handle.metrics().unwrap().offset, target);
    assert_eq!(thumb(&ui)?, (px(10.0), px(40.0)));
    Ok(())
}

#[test]
fn container_handle_follows_conditional_mounts_and_rejects_two_live_owners() -> Result<()> {
    use solor::ScrollHandle;
    let mut ui = Ui::new();
    let handle = ScrollHandle::new();
    let visible = ui.context().signal(true);
    let handle_for_bar = handle.clone();
    let tracked = handle.clone();
    let view = ui.mount(component(move |_| {
        div()
            .flex_row()
            .items_stretch()
            .child_when(visible, move || {
                div()
                    .flex_1()
                    .overflow_y_scroll()
                    .track_scroll(&tracked)
                    .child(div().min_h(px(200.0)))
            })
            .child(scrollbar(&handle_for_bar))
    }))?;
    submit(&mut ui, view, 60.0)?;
    assert_eq!(handle.metrics().unwrap().max_offset.height, px(140.0));
    let duplicate = handle.clone();
    assert!(
        ui.mount(component(move |_| div().track_scroll(&duplicate)))
            .is_err()
    );
    assert!(handle.metrics().is_some());
    ui.context().set(visible, false)?;
    submit(&mut ui, view, 60.0)?;
    assert!(handle.metrics().is_none());
    assert!(ui.record_frame(view)?.draws().is_empty());
    ui.context().set(visible, true)?;
    submit(&mut ui, view, 80.0)?;
    assert_eq!(handle.metrics().unwrap().max_offset.height, px(120.0));
    ui.unmount(view)?;
    assert!(handle.metrics().is_none());
    Ok(())
}

#[test]
fn container_seeks_use_the_new_extent_and_keep_fractional_offsets() -> Result<()> {
    use solor::{ScrollHandle, style::Overflow};
    let mut ui = Ui::new();
    let scroll = ScrollHandle::new();
    let container = scroll.clone();
    let button = scroll.clone();
    let expanded = ui.context().signal(false);
    let offset = |x, y| Origin { x: px(x), y: px(y) };
    scroll.scroll_to_offset(offset(8.75, 13.25))?;
    let view = ui.mount(component(move |_| {
        div()
            .flex_row()
            .items_stretch()
            .child(
                div()
                    .flex_1()
                    .overflow(Overflow::Auto)
                    .track_scroll(&container)
                    .child(div().min_w(px(200.0)).min_h(px(160.0)))
                    .child_when(expanded, || div().min_h(px(45.5))),
            )
            .child(div().w(px(20.0)).on_click(move |cx| {
                cx.set(expanded, true)?;
                button.scroll_to_percent(100.0)
            }))
    }))?;
    submit(&mut ui, view, 80.0)?;
    assert_eq!(scroll.metrics().unwrap().offset, offset(8.75, 13.25));
    let click = point(110.0, 5.0);
    ui.dispatch_pointer(view, PointerInput::Down(click, PointerButton::Primary))?;
    ui.dispatch_pointer(view, PointerInput::Up(click, PointerButton::Primary))?;
    submit(&mut ui, view, 80.0)?;
    assert_eq!(scroll.metrics().unwrap().offset, offset(8.75, 125.5));

    scroll.scroll_to_offset(offset(16.5, 2.0))?;
    scroll.scroll_to_percent(50.0)?;
    assert!(scroll.scroll_to_percent(f64::NAN).is_err());
    assert!(scroll.scroll_to_offset(offset(f32::INFINITY, 0.0)).is_err());
    assert!(ui.next_redraw(view).is_some());
    submit(&mut ui, view, 100.0)?;
    assert_eq!(scroll.metrics().unwrap().offset, offset(16.5, 52.75));
    assert!(ui.next_redraw(view).is_none());
    scroll.scroll_to_offset(offset(22.25, 150.0))?;
    ui.dispatch_pointer(
        view,
        PointerInput::Scroll(
            point(10.0, 10.0),
            ScrollDelta {
                x: px(1.5),
                y: px(-5.0),
            },
        ),
    )?;
    submit(&mut ui, view, 100.0)?;
    assert_eq!(scroll.metrics().unwrap().offset, offset(23.75, 100.5));
    Ok(())
}

#[test]
fn container_seek_waits_for_scrollable_layout_and_expires_on_unmount() -> Result<()> {
    use solor::{ScrollHandle, style::Overflow};
    let mut ui = Ui::new();
    let scroll = ScrollHandle::new();
    let overflow = ui.context().signal(Overflow::Hidden);
    let mount = |ui: &mut Ui| {
        let scroll = scroll.clone();
        ui.mount(component(move |_| {
            div()
                .overflow_y(overflow)
                .track_scroll(&scroll)
                .child(div().min_h(px(200.0)))
        }))
    };
    let view = mount(&mut ui)?;
    submit(&mut ui, view, 80.0)?;
    scroll.scroll_to_percent(105.0)?;
    submit(&mut ui, view, 80.0)?;
    assert!(ui.next_redraw(view).is_none());
    ui.context().set(overflow, Overflow::Scroll)?;
    submit(&mut ui, view, 80.0)?;
    assert_eq!(scroll.metrics().unwrap().offset.y, px(120.0));
    scroll.scroll_to_percent(50.0)?;
    ui.unmount(view)?;
    let next = mount(&mut ui)?;
    submit(&mut ui, next, 80.0)?;
    assert_eq!(scroll.metrics().unwrap().offset, Origin::default());
    Ok(())
}

#[test]
fn container_progress_repaints_on_both_axes_and_viewport_changes() -> Result<()> {
    use solor::{ScrollHandle, style::Overflow};
    use std::{cell::Cell, rc::Rc};

    let mut ui = Ui::new();
    let (scroll, metrics) = ScrollHandle::with_metrics_signal(&mut ui.context());
    assert!(ui.context().read(metrics)?.is_none());
    let tracked = scroll.clone();
    let overflow = ui.context().signal(Overflow::Auto);
    let paints = Rc::new(Cell::new(0));
    let count = paints.clone();
    let view = ui.mount(component(move |_| {
        div()
            .flex_row()
            .items_stretch()
            .child(
                div()
                    .flex_1()
                    .overflow(overflow)
                    .track_scroll(&tracked)
                    .child(div().min_w(px(180.0)).min_h(px(200.0))),
            )
            .child(div().w(px(20.0)).paint(move |cx| {
                count.set(count.get() + 1);
                if let Some(m) = *cx.read(metrics)? {
                    let size = Size {
                        width: cx.size.width * (m.horizontal_percent() / 100.0) as f32,
                        height: cx.size.height * (m.vertical_percent() / 100.0) as f32,
                    };
                    cx.fill(size, rgb(0x224466));
                }
                Ok(())
            }))
    }))?;
    let indicator_size = |draws: &[Draw]| {
        draws.iter().find_map(|draw| match draw {
            Draw::Fill { size, .. } => Some((size.width, size.height)),
            _ => None,
        })
    };

    submit(&mut ui, view, 80.0)?;
    scroll.scroll_to_offset(Origin {
        x: px(20.0),
        y: px(30.0),
    })?;
    let draws = submit(&mut ui, view, 80.0)?;
    assert_eq!(indicator_size(&draws), Some((px(5.0), px(20.0))));

    ui.dispatch_pointer(
        view,
        PointerInput::Scroll(
            point(10.0, 10.0),
            ScrollDelta {
                x: px(20.0),
                y: px(0.0),
            },
        ),
    )?;
    let draws = submit(&mut ui, view, 80.0)?;
    assert_eq!(indicator_size(&draws), Some((px(10.0), px(20.0))));

    let draws = submit(&mut ui, view, 120.0)?;
    assert_eq!(indicator_size(&draws), Some((px(10.0), px(45.0))));
    let before = paints.get();
    scroll.scroll_to_percent(37.5)?;
    submit(&mut ui, view, 120.0)?;
    assert_eq!(paints.get(), before);
    assert!(ui.next_redraw(view).is_none());

    submit(&mut ui, view, 240.0)?;
    let m = ui.context().read(metrics)?.unwrap();
    assert_eq!((m.horizontal_percent(), m.vertical_percent()), (50.0, 0.0));
    ui.context().set(overflow, Overflow::Hidden)?;
    let draws = submit(&mut ui, view, 240.0)?;
    assert!(ui.context().read(metrics)?.is_none());
    assert_eq!(indicator_size(&draws), None);
    ui.context().set(overflow, Overflow::Auto)?;
    submit(&mut ui, view, 80.0)?;
    assert_eq!(*ui.context().read(metrics)?, scroll.metrics());
    ui.unmount(view)?;
    assert!(ui.context().read(metrics)?.is_none());
    Ok(())
}

#[test]
fn two_scrollbars_share_a_viewport_and_route_input_by_axis() -> Result<()> {
    use solor::{ScrollHandle, horizontal_scrollbar, style::Overflow};
    use std::{cell::Cell, rc::Rc};

    let mut ui = Ui::new();
    let scroll = ScrollHandle::new();
    let tracked = scroll.clone();
    let remaining = Rc::new(Cell::new(None));
    let bubbled = remaining.clone();
    let width = ui.context().signal(px(324.0));
    let view = ui.mount(component(move |_| {
        div()
            .flex_col()
            .items_stretch()
            .on_scroll(move |event, _| {
                bubbled.set(Some(event.delta));
                Ok(())
            })
            .child(
                div()
                    .flex_row()
                    .flex_1()
                    .items_stretch()
                    .child(
                        div()
                            .flex_1()
                            .overflow(Overflow::Auto)
                            .track_scroll(&tracked)
                            .child(div().min_w(width).min_h(px(200.0))),
                    )
                    .child(scrollbar(&tracked).p(px(0.0))),
            )
            .child(horizontal_scrollbar(&tracked).p(px(0.0)))
    }))?;
    let offset = |x, y| Origin { x: px(x), y: px(y) };
    let delta = |x, y| ScrollDelta { x: px(x), y: px(y) };
    let draws = submit(&mut ui, view, 112.0)?;
    assert!(
        draws
            .iter()
            .any(|d| matches!(d, Draw::Fill { origin, size, .. }
        if *origin == offset(0.0, 109.0) && size.width == px(40.0) && size.height == px(3.0)))
    );
    ui.dispatch_pointer(
        view,
        PointerInput::Scroll(point(10.0, 106.0), delta(10.5, 7.25)),
    )?;
    assert_eq!(remaining.take(), Some(delta(0.0, 7.25)));
    ui.dispatch_pointer(
        view,
        PointerInput::Scroll(point(114.0, 15.0), delta(5.0, 18.25)),
    )?;
    assert_eq!(remaining.take(), Some(delta(5.0, 0.0)));
    submit(&mut ui, view, 112.0)?;
    assert_eq!(scroll.metrics().unwrap().offset, offset(10.5, 18.25));

    let page = point(110.0, 106.0);
    ui.dispatch_pointer(view, PointerInput::Down(page, PointerButton::Primary))?;
    ui.dispatch_pointer(view, PointerInput::Up(page, PointerButton::Primary))?;
    submit(&mut ui, view, 112.0)?;
    assert_eq!(scroll.metrics().unwrap().offset, offset(118.5, 18.25));
    let grab = point(60.0, 106.0);
    ui.dispatch_pointer(view, PointerInput::Down(grab, PointerButton::Primary))?;
    scroll.scroll_to_percent(75.0)?;
    let outside = point(160.0, 50.0);
    ui.dispatch_pointer(view, PointerInput::Move(outside))?;
    ui.dispatch_pointer(view, PointerInput::Up(outside, PointerButton::Primary))?;
    submit(&mut ui, view, 112.0)?;
    assert_eq!(scroll.metrics().unwrap().offset, offset(216.0, 75.0));

    ui.dispatch_pointer(view, PointerInput::Scroll(page, delta(4.25, -2.5)))?;
    assert_eq!(remaining.take(), Some(delta(4.25, -2.5)));
    ui.context().set(width, px(80.0))?;
    let draws = submit(&mut ui, view, 112.0)?;
    assert_eq!(scroll.metrics().unwrap().offset, offset(0.0, 75.0));
    assert!(
        !draws
            .iter()
            .any(|d| matches!(d, Draw::Fill { origin, .. } if origin.y >= px(100.0)))
    );
    assert!(ui.next_redraw(view).is_none());
    Ok(())
}

#[test]
fn horizontal_painter_follows_its_axis_through_resize() -> Result<()> {
    use solor::{ScrollAxis, ScrollHandle, horizontal_scrollbar, style::Overflow};
    use std::{cell::Cell, rc::Rc};

    let mut ui = Ui::new();
    let scroll = ScrollHandle::new();
    let tracked = scroll.clone();
    let content_width = ui.context().signal(px(240.0));
    let last = Rc::new(Cell::new(None));
    let seen = last.clone();
    let view = ui.mount(component(move |_| {
        div()
            .flex_col()
            .items_stretch()
            .child(
                div()
                    .flex_1()
                    .overflow(Overflow::Auto)
                    .track_scroll(&tracked)
                    .child(div().min_w(content_width).min_h(px(200.0))),
            )
            .child(
                horizontal_scrollbar(&tracked)
                    .p(px(0.0))
                    .paint_with(move |_, bar| {
                        seen.set(Some(bar));
                        Ok(())
                    }),
            )
    }))?;
    scroll.scroll_to_percent(25.0)?;
    scroll.scroll_to_percent_on(ScrollAxis::Horizontal, 50.0)?;
    submit(&mut ui, view, 112.0)?;
    let bar = last.take().unwrap();
    assert_eq!(bar.axis, ScrollAxis::Horizontal);
    assert_eq!(bar.position.viewport_extent, px(120.0));
    assert_eq!(bar.position.percent(), 50.0);
    assert_eq!((bar.thumb_start, bar.thumb_length), (px(30.0), px(60.0)));
    assert_eq!(
        scroll.metrics().unwrap().offset,
        Origin {
            x: px(60.0),
            y: px(25.0)
        }
    );

    scroll.scroll_to_percent(75.0)?;
    submit(&mut ui, view, 112.0)?;
    assert!(
        last.get().is_none(),
        "vertical movement repainted the horizontal track"
    );
    ui.context().set(content_width, px(360.0))?;
    scroll.scroll_to_percent_on(ScrollAxis::Horizontal, 75.0)?;
    submit(&mut ui, view, 112.0)?;
    let bar = last.take().unwrap();
    assert_eq!((bar.thumb_start, bar.thumb_length), (px(60.0), px(40.0)));
    assert_eq!(
        scroll.metrics().unwrap().offset,
        Origin {
            x: px(180.0),
            y: px(75.0)
        }
    );
    Ok(())
}

#[test]
fn revealing_a_content_rectangle_moves_each_axis_only_as_far_as_needed() -> Result<()> {
    use solor::ScrollHandle;

    let origin = |x, y| Origin { x: px(x), y: px(y) };
    let size = |w, h| Size {
        width: px(w),
        height: px(h),
    };
    let scroll = ScrollHandle::new();
    let target = origin(211.5, 170.25);
    scroll.scroll_to_reveal(target, size(20.0, 10.0))?;
    assert!(scroll.scroll_to_reveal(target, size(-1.0, 10.0)).is_err());

    let mut ui = Ui::new();
    let tracked = scroll.clone();
    let view = ui.mount(component(move |_| {
        div()
            .overflow_auto()
            .track_scroll(&tracked)
            .child(div().min_w(px(400.0)).min_h(px(300.0)))
    }))?;
    submit(&mut ui, view, 80.0)?;
    let offset = || scroll.metrics().unwrap().offset;
    assert_eq!(offset(), origin(111.5, 100.25));

    // An oversized rectangle already covers the horizontal viewport; the short
    // vertical interval is fully visible. Neither axis needs to move.
    scroll.scroll_to_reveal(origin(80.0, 130.0), size(200.0, 20.0))?;
    submit(&mut ui, view, 80.0)?;
    assert_eq!(offset(), origin(111.5, 100.25));

    scroll.scroll_to_reveal(origin(30.0, 190.0), size(20.0, 100.0))?;
    ui.dispatch_pointer(
        view,
        PointerInput::Scroll(
            point(20.0, 20.0),
            ScrollDelta {
                x: px(2.25),
                y: px(3.5),
            },
        ),
    )?;
    submit(&mut ui, view, 80.0)?;
    assert_eq!(offset(), origin(32.25, 193.5));

    scroll.scroll_to_reveal(origin(-50.0, 500.0), size(10.0, 20.0))?;
    submit(&mut ui, view, 80.0)?;
    assert_eq!(offset(), origin(0.0, 220.0));
    Ok(())
}
