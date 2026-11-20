//! Single-line deficit allocation and the geometry of reflowed content.
use anyhow::Result;
use solor::{
    Draw, Styled, Ui, component, div,
    layout::Size,
    px,
    style::FlexDirection::{Column, ColumnReverse, Row, RowReverse},
    text,
};

#[test]
fn shrink_uses_content_weights_and_preserves_minima_on_both_axes() -> Result<()> {
    for direction in [Row, RowReverse, Column, ColumnReverse] {
        let row = matches!(direction, Row | RowReverse);
        let reversed = matches!(direction, RowReverse | ColumnReverse);
        let mut ui = Ui::new();
        let view = ui.mount(component(|_| {
            let a = div().flex_basis(px(90.0)).flex_shrink(1.0);
            let b = div().flex_basis(px(40.0)).flex_shrink(0.5);
            div()
                .flex_direction(direction)
                .gap(px(3.5))
                .justify_end()
                .child(if row {
                    a.px(px(5.0)).min_w(px(70.0))
                } else {
                    a.py(px(5.0)).min_h(px(70.0))
                })
                .child(b)
                .child(div().flex_none().size(px(8.0)))
        }))?;
        // Content weights are 80 and 20. After A hits 70, B can use at
        // most half the initial deficit. Any excess stays aligned to the end.
        for (main, a, b, start) in [
            (145.0, 90.0, 40.0, 0.0),
            (132.5, 80.0, 37.5, 0.0),
            (113.0, 70.0, 28.0, 0.0),
            (85.0, 70.0, 10.0, -10.0),
            (55.0, 70.0, 0.0, -30.0),
        ] {
            let (w, h) = if row { (main, 200.0) } else { (200.0, main) };
            ui.layout(
                view,
                Size {
                    width: px(w),
                    height: px(h),
                },
            )?;
            let mut cursor = start;
            for (node, expected) in ui.children(view, ui.root(view)?)?.zip([a, b, 8.0]) {
                let g = ui.geometry(view, node).unwrap();
                let (size, origin) = if row {
                    (g.metrics.size.width, g.origin.x)
                } else {
                    (g.metrics.size.height, g.origin.y)
                };
                assert_eq!(size, px(expected), "{direction:?} at {main}");
                assert_eq!(
                    origin,
                    px(if reversed {
                        main - cursor - expected
                    } else {
                        cursor
                    })
                );
                cursor += expected + 3.5;
            }
        }
    }
    Ok(())
}

#[test]
fn preferred_widths_larger_than_the_container_keep_their_relative_weights() -> Result<()> {
    let mut ui = Ui::new();
    let view = ui.mount(component(|_| {
        div()
            .flex_row()
            .child(div().w(px(200.0)).flex_shrink(1.0))
            .child(div().w(px(50.0)).flex_shrink(1.0))
    }))?;
    ui.layout(
        view,
        Size {
            width: px(100.0),
            height: px(20.0),
        },
    )?;
    let widths: Vec<_> = ui
        .children(view, ui.root(view)?)?
        .map(|node| ui.geometry(view, node).unwrap().metrics.size.width)
        .collect();
    assert_eq!(widths, [px(80.0), px(20.0)]);
    Ok(())
}

#[test]
fn auto_basis_label_reflows_between_fixed_controls_after_resize() -> Result<()> {
    let font = include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf");
    let mut ui = Ui::with_font(font.to_vec(), "zh-CN")?;
    let view = ui.mount(component(|_| {
        div()
            .child(
                div()
                    .flex_row()
                    .gap(px(4.0))
                    .child(div().size(px(16.0)))
                    .child(
                        text("Audio settings and output device selection and advanced controls")
                            .flex_shrink(1.0)
                            .line_height(px(20.0)),
                    )
                    .child(div().size(px(24.0))),
            )
            .child(div().h(px(4.0)))
    }))?;
    let mut lines = Vec::new();
    for w in [160.0, 88.0, 160.0] {
        ui.layout(
            view,
            Size {
                width: px(w),
                height: px(800.0),
            },
        )?;
        let mut rows = ui.children(view, ui.root(view)?)?;
        let mut items = ui.children(view, rows.next().unwrap())?;
        let label = items.nth(1).unwrap();
        let mut draw = Vec::new();
        ui.paint(view, label, &mut draw)?;
        let [
            Draw::Text {
                size, paragraph, ..
            },
        ] = &draw[..]
        else {
            panic!("expected a paragraph")
        };
        assert_eq!(size.width, px(w - 48.0));
        assert_eq!(
            ui.geometry(view, items.next().unwrap()).unwrap().origin.x,
            px(w - 24.0)
        );
        assert_eq!(
            ui.geometry(view, rows.next().unwrap()).unwrap().origin.y,
            px(20.0 * paragraph.lines.len() as f32)
        );
        lines.push(paragraph.lines.len());
    }
    assert!(lines[1] > lines[0]);
    assert_eq!(lines[0], lines[2]);
    Ok(())
}

#[test]
fn invalid_shrink_factors_reject_the_declaration() {
    for factor in [-1.0, f32::NAN, f32::INFINITY] {
        let mut ui = Ui::new();
        assert!(
            ui.mount(component(move |_| div().child(div().flex_shrink(factor))))
                .is_err()
        );
    }
}

#[test]
fn a_frozen_item_stays_at_its_minimum_when_remaining_factors_are_fractional() -> Result<()> {
    let mut ui = Ui::new();
    let view = ui.mount(component(|_| {
        div()
            .flex_row()
            .justify_end()
            .child(div().w(px(10.0)).min_w(px(8.0)).flex_shrink(0.875))
            .child(div().w(px(100.0)).flex_shrink(0.125))
    }))?;
    ui.layout(
        view,
        Size {
            width: px(102.0),
            height: px(20.0),
        },
    )?;
    let mut nodes = ui.children(view, ui.root(view)?)?;
    let first = ui.geometry(view, nodes.next().unwrap()).unwrap();
    let second = ui.geometry(view, nodes.next().unwrap()).unwrap();
    assert_eq!(first.metrics.size.width, px(8.0));
    assert_eq!(first.origin.x, px(-5.0));
    assert_eq!(second.metrics.size.width, px(99.0));
    Ok(())
}

#[test]
fn a_padded_item_keeps_its_insets_when_the_line_runs_out_of_space() -> Result<()> {
    let mut ui = Ui::new();
    let view = ui.mount(component(|_| {
        div()
            .flex_row()
            .child(div().flex_1().flex_basis(px(24.0)).px(px(3.5)))
            .child(div().w(px(20.0)).flex_none())
    }))?;
    ui.layout(
        view,
        Size {
            width: px(22.0),
            height: px(20.0),
        },
    )?;
    let mut nodes = ui.children(view, ui.root(view)?)?;
    assert_eq!(
        ui.geometry(view, nodes.next().unwrap())
            .unwrap()
            .metrics
            .size
            .width,
        px(7.0)
    );
    assert_eq!(
        ui.geometry(view, nodes.next().unwrap()).unwrap().origin.x,
        px(7.0)
    );
    Ok(())
}
