//! Variable row measurement, reading position and bounded window traversal.
use anyhow::Result;
use solor::{
    IntoElement, NodeId, SharedString, Styled, Ui, ViewId, component, div,
    layout::{Origin, Size},
    pointer::{PointerId, PointerInput, PointerKind, PointerSample, ScrollDelta},
    px, text, variable_list,
};
use std::{cell::Cell, rc::Rc};

fn submit(ui: &mut Ui, view: ViewId, width: f32, height: f32) -> Result<Vec<NodeId>> {
    ui.flush()?;
    ui.layout(
        view,
        Size {
            width: px(width),
            height: px(height),
        },
    )?;
    let frame = ui.record_frame(view)?;
    ui.accept_pointer_frame(frame.into_pointer_frame())?;
    Ok(ui.children(view, ui.root(view)?)?.collect())
}

fn wheel(ui: &mut Ui, view: ViewId, distance: f32) -> Result<()> {
    let mouse = PointerSample {
        id: PointerId(1),
        kind: PointerKind::Mouse,
        position: Origin {
            x: px(1.0),
            y: px(1.0),
        },
        modifiers: Default::default(),
    };
    ui.dispatch_pointer(
        view,
        PointerInput::Scroll(
            mouse,
            ScrollDelta {
                x: px(0.0),
                y: px(distance),
            },
        ),
    )?;
    Ok(())
}

#[test]
fn reflow_and_keyed_edits_keep_the_reading_row_at_its_fractional_offset() -> Result<()> {
    let mut ui = Ui::with_font(
        include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf").to_vec(),
    )?;
    let keys = ui.context().signal((0..60usize).collect::<Vec<_>>());
    let caption = ui.context().signal(SharedString::from_static("A"));
    let view = ui.mount(component(move |_| {
        variable_list(keys, px(20.0), move |i| match i {
            1 => text(caption).line_height(px(20.0)).into_element(),
            2 => div().h(px(120.0)).into_element(),
            _ => div().h(px(20.0)).into_element(),
        })
    }))?;
    let first = submit(&mut ui, view, 200.0, 100.0)?;
    let above = first[1];
    let reading = first[2];
    wheel(&mut ui, view, 90.25)?;
    submit(&mut ui, view, 200.0, 100.0)?;
    let y = |ui: &Ui| ui.geometry(view, reading).unwrap().origin.y;
    assert_eq!(y(&ui), px(-50.25));
    ui.context().set(
        caption,
        SharedString::from_static("A B C D E F G H I J K L M N O P"),
    )?;
    submit(&mut ui, view, 180.0, 100.0)?;
    let wide = ui.geometry(view, above).unwrap().metrics.size.height;
    submit(&mut ui, view, 38.0, 100.0)?;
    assert!(ui.geometry(view, above).unwrap().metrics.size.height > wide);
    assert_eq!(y(&ui), px(-50.25));
    ui.context().update_in_place(keys, |keys| {
        keys.splice(..0, [60, 61]);
    })?;
    submit(&mut ui, view, 38.0, 100.0)?;
    assert_eq!(y(&ui), px(-50.25));
    ui.context()
        .update_in_place(keys, |keys| keys.retain(|&k| k != 2))?;
    let remaining = submit(&mut ui, view, 38.0, 100.0)?;
    assert!(!remaining.contains(&reading));
    assert!(
        remaining
            .iter()
            .any(|&id| ui.geometry(view, id).unwrap().origin.y == px(0.0))
    );
    Ok(())
}

#[test]
fn distant_jumps_only_measure_a_viewport_and_align_the_final_row() -> Result<()> {
    for count in [100_000usize, 1_000_000] {
        let mut ui = Ui::new();
        let selected = ui.context().signal(None);
        let built = Rc::new(Cell::new(0));
        let builds = built.clone();
        let view = ui.mount(component(move |_| {
            variable_list(count, px(20.0), move |i: usize| {
                builds.set(builds.get() + 1);
                div()
                    .h(px(if i.is_multiple_of(2) { 16.5 } else { 63.25 }))
                    .on_click(move |cx| cx.set(selected, Some(i)))
            })
        }))?;
        let top = submit(&mut ui, view, 120.0, 100.0)?;
        assert!(top.len() <= 6);
        wheel(&mut ui, view, 1e9)?;
        let end = submit(&mut ui, view, 120.0, 100.0)?;
        let last = *end.last().unwrap();
        ui.click(view, last)?;
        assert_eq!(*ui.context().read(selected)?, Some(count - 1));
        let geometry = ui.geometry(view, last).unwrap();
        assert_eq!(geometry.origin.y + geometry.metrics.size.height, px(100.0));
        assert_eq!(geometry.metrics.size.height, px(63.25));
        wheel(&mut ui, view, -0.25)?;
        submit(&mut ui, view, 120.0, 100.0)?;
        assert_eq!(
            ui.geometry(view, last).unwrap().origin.y,
            geometry.origin.y + px(0.25)
        );
        wheel(&mut ui, view, -1e9)?;
        let start = submit(&mut ui, view, 120.0, 100.0)?;
        assert_eq!(ui.geometry(view, start[0]).unwrap().origin.y, px(0.0));
        assert!(built.get() <= 24);
        assert_eq!(
            ui.layout(
                view,
                Size {
                    width: px(120.0),
                    height: px(100.0)
                }
            )?,
            0
        );
    }
    Ok(())
}

#[test]
fn replacing_a_zero_height_row_recovers_the_view() -> Result<()> {
    let mut ui = Ui::new();
    let keys = ui.context().signal(vec![1usize, 0]);
    let view = ui.mount(component(move |_| {
        variable_list(keys, px(20.0), |key| div().h(px(key as f32 * 20.0)))
    }))?;
    assert!(submit(&mut ui, view, 100.0, 80.0).is_err());
    ui.context().set(keys, vec![2])?;
    let rows = submit(&mut ui, view, 100.0, 80.0)?;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        ui.geometry(view, rows[0]).unwrap().metrics.size.height,
        px(40.0)
    );
    Ok(())
}

#[test]
fn a_reading_row_follows_its_key_through_splices_and_coalesced_moves() -> Result<()> {
    let mut ui = Ui::new();
    let rows = ui
        .context()
        .signal_vec((0..256).map(|i| px(if i % 2 == 0 { 18.0 } else { 22.0 })));
    let scroll = solor::ListScrollHandle::new();
    let view = ui.mount(component(|_| {
        variable_list(rows, px(20.0), |height| div().h(height)).scroll_handle(&scroll)
    }))?;
    scroll.scroll_to(150, solor::ScrollAlignment::Top);
    let visible = submit(&mut ui, view, 100.0, 80.0)?;
    let reading = visible
        .into_iter()
        .find(|&id| ui.geometry(view, id).unwrap().origin.y == px(0.0))
        .unwrap();
    wheel(&mut ui, view, 6.25)?;
    let check_position = |ui: &mut Ui| -> Result<()> {
        submit(ui, view, 100.0, 80.0)?;
        assert_eq!(ui.geometry(view, reading).unwrap().origin.y, px(-6.25));
        Ok(())
    };
    check_position(&mut ui)?;
    rows.splice(5..5, [px(25.0); 7], &mut ui.context())?;
    check_position(&mut ui)?;
    rows.splice(5..12, [], &mut ui.context())?;
    check_position(&mut ui)?;
    rows.move_item(150, 30, &mut ui.context())?;
    rows.push(px(24.0), &mut ui.context())?;
    check_position(&mut ui)?;
    Ok(())
}
