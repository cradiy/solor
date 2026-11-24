//! Flexible allocation, remaining space and the geometry of grown content.
use anyhow::Result;
use solor::{
    Draw, Length, Styled, Ui, component, div,
    layout::Size,
    percent, px,
    style::FlexDirection::{Column, ColumnReverse, Row, RowReverse},
    text,
};

#[test]
fn weighted_growth_freezes_capped_items_and_redistributes_the_surplus() -> Result<()> {
    for direction in [Row, RowReverse, Column, ColumnReverse] {
        let row = matches!(direction, Row | RowReverse);
        let reverse = matches!(direction, RowReverse | ColumnReverse);
        let mut ui = Ui::new();
        let visible = ui.context().signal(true);
        let view = ui.mount(component(|_| {
            let first = div().flex_basis(px(10.0)).flex_grow(2.0);
            let second = div().flex_basis(px(10.0)).flex_grow(1.0);
            div()
                .flex_direction(direction)
                .p(px(2.0))
                .gap(px(3.0))
                .justify_end()
                .child(if row {
                    first.max_w(px(18.0))
                } else {
                    first.max_h(px(18.0))
                })
                .child(if row {
                    second.max_w(px(35.0))
                } else {
                    second.max_h(px(35.0))
                })
                .child_when(visible, || div().flex_1())
        }))?;
        // Exercise the initial 2:1:1 split, then one and two binding limits.
        for (main, show, sizes, start) in [
            (42.0, true, vec![16.0, 13.0, 3.0], 0.0),
            (50.0, true, vec![18.0, 16.0, 6.0], 0.0),
            (120.0, true, vec![18.0, 35.0, 57.0], 0.0),
            (120.0, false, vec![18.0, 35.0], 60.0),
        ] {
            ui.context().set(visible, show)?;
            ui.flush()?;
            let (w, h) = if row { (main, 40.0) } else { (40.0, main) };
            ui.layout(
                view,
                Size {
                    width: px(w),
                    height: px(h),
                },
            )?;
            let mut cursor = start;
            let items = ui.children(view, ui.root(view)?)?;
            assert_eq!(items.len(), sizes.len());
            for (node, expected) in items.zip(sizes) {
                let g = ui.geometry(view, node).unwrap();
                let (size, origin) = if row {
                    (g.metrics.size.width, g.origin.x)
                } else {
                    (g.metrics.size.height, g.origin.y)
                };
                assert_eq!(size, px(expected), "{direction:?}");
                let pos = if reverse {
                    main - 2.0 - cursor - expected
                } else {
                    2.0 + cursor
                };
                assert_eq!(origin, px(pos), "{direction:?}");
                cursor += expected + 3.0;
            }
        }
    }
    Ok(())
}

#[test]
fn fractional_growth_leaves_space_for_auto_margins_before_justification() -> Result<()> {
    let mut ui = Ui::new();
    let view = ui.mount(component(|_| {
        div()
            .flex_row()
            .justify_center()
            .child(div().size(px(10.0)).ml(Length::Auto))
            .child(div().flex_basis(px(10.0)).flex_grow(0.25))
    }))?;
    for (width, start, grown) in [(100.0, 60.0, 30.0), (16.0, -2.0, 10.0)] {
        ui.layout(
            view,
            Size {
                width: px(width),
                height: px(20.0),
            },
        )?;
        let mut nodes = ui.children(view, ui.root(view)?)?;
        let fixed = ui.geometry(view, nodes.next().unwrap()).unwrap();
        let flexible = ui.geometry(view, nodes.next().unwrap()).unwrap();
        assert_eq!(fixed.origin.x, px(start));
        assert_eq!(flexible.origin.x, px(start + 10.0));
        assert_eq!(flexible.metrics.size.width, px(grown));
    }
    Ok(())
}

#[test]
fn toolbar_measures_its_label_at_the_width_between_fixed_controls() -> Result<()> {
    let font = include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf");
    for stretch in [true, false] {
        let mut ui = Ui::with_font(font.to_vec())?;
        let view =
            ui.mount(component(|_| {
                div().when(stretch, |d| d.items_stretch()).child(div().flex_row().gap(px(3.0))
                .child(div().size(px(14.0)))
                .child(text("Audio settings and output device selection and advanced controls")
                      .line_height(px(20.0)).flex_1())
                .child(div().size(px(20.0)))).child(div().h(px(4.0)))
            }))?;
        let mut lines = Vec::new();
        for width in [180.0, 78.0, 180.0] {
            ui.layout(
                view,
                Size {
                    width: px(width),
                    height: px(500.0),
                },
            )?;
            let mut rows = ui.children(view, ui.root(view)?)?;
            let row = rows.next().unwrap();
            let next = ui.geometry(view, rows.next().unwrap()).unwrap();
            let row_width = ui.geometry(view, row).unwrap().metrics.size.width;
            let mut items = ui.children(view, row)?;
            let label = items.nth(1).unwrap();
            let action = ui.geometry(view, items.next().unwrap()).unwrap();
            let mut commands = Vec::new();
            ui.paint(view, label, &mut commands)?;
            let [
                Draw::Text {
                    size, paragraph, ..
                },
            ] = &commands[..]
            else {
                panic!("expected a paragraph")
            };
            assert_eq!(size.width, row_width - px(40.0));
            assert_eq!(action.origin.x, row_width - px(20.0));
            assert_eq!(next.origin.y, px(paragraph.lines.len() as f32 * 20.0));
            lines.push(paragraph.lines.len());
        }
        assert!(lines[1] > lines[0]);
        assert_eq!(lines[0], lines[2]);
    }
    Ok(())
}

#[test]
fn column_growth_sets_the_percentage_basis_for_nested_content() -> Result<()> {
    let mut ui = Ui::new();
    let view = ui.mount(component(|_| {
        div().child(div().h(px(10.0))).child(
            div()
                .flex_1()
                .p(px(2.0))
                .justify_end()
                .child(div().h(percent(50.0))),
        )
    }))?;
    for height in [100.0, 60.0] {
        ui.layout(
            view,
            Size {
                width: px(100.0),
                height: px(height),
            },
        )?;
        let group = ui.children(view, ui.root(view)?)?.nth(1).unwrap();
        let child = ui
            .geometry(view, ui.children(view, group)?.next().unwrap())
            .unwrap();
        let half = (height - 14.0) * 0.5;
        assert_eq!(child.metrics.size.height, px(half));
        assert_eq!(child.origin.y, px(2.0 + half));
    }
    Ok(())
}

#[test]
fn animated_growth_redistributes_space_when_retargeted_at_a_width_limit() -> Result<()> {
    use solor::{Transition, TransitionProperty};
    use std::time::Duration;

    let mut ui = Ui::new();
    let grow = ui.context().signal(0.0_f32);
    let view = ui.mount(component(move |_| {
        div()
            .flex_row()
            .child(
                div()
                    .max_w(px(72.))
                    .flex_basis(px(0.))
                    .flex_grow(grow)
                    .transition(
                        TransitionProperty::FlexGrow,
                        Transition::new(Duration::from_millis(100)),
                    ),
            )
            .child(div().flex_1())
    }))?;
    let size = Size {
        width: px(120.),
        height: px(20.),
    };
    ui.layout(view, size)?;
    let nodes: Vec<_> = ui.children(view, ui.root(view)?)?.collect();
    ui.context().set(grow, 4.0_f32)?;
    for (ms, expected, reverse) in [
        (0, 0., false),
        (25, 60., false),
        (50, 72., false),
        (50, 72., true),
        (100, 60., false),
        (150, 0., false),
    ] {
        if reverse {
            ui.context().set(grow, 0.0_f32)?;
        }
        ui.dispatch_frame(view, Duration::from_millis(ms))?;
        ui.layout(view, size)?;
        let first = ui.geometry(view, nodes[0]).unwrap();
        let second = ui.geometry(view, nodes[1]).unwrap();
        assert_eq!(first.metrics.size.width, px(expected));
        assert_eq!(second.origin.x, px(expected));
        assert_eq!(second.metrics.size.width, size.width - px(expected));
        assert_eq!(ui.record_frame(view)?.requests_animation_frame(), ms < 150);
    }
    Ok(())
}
