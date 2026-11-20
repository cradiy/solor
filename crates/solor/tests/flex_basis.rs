//! Main-axis basis in single-line flex layout.
use anyhow::Result;
use solor::{
    Draw, Length, Styled, Ui, component, div,
    layout::Size,
    percent, px, rem,
    style::{BoxSizing, FlexDirection::*},
    text,
};

#[test]
fn row_basis_controls_wrapping_and_reserved_line_height_after_resize() -> Result<()> {
    let font = include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf");
    for direction in [Row, RowReverse] {
        let mut ui = Ui::with_font(font.to_vec())?;
        let view = ui.mount(component(|_| {
            div()
                .items_stretch()
                .child(
                    div()
                        .flex_direction(direction)
                        .gap(px(3.5))
                        .child(
                            text("Audio settings and output device")
                                .box_sizing(BoxSizing::ContentBox)
                                .w(px(9.0))
                                .flex_basis(percent(50.0))
                                .min_w(rem(2.0))
                                .max_w(px(90.0))
                                .px(px(2.25))
                                .line_height(px(20.0)),
                        )
                        .child(div().size(px(8.0))),
                )
                .child(div().size(px(4.0)))
        }))?;
        let mut lines = Vec::new();
        for (width, inner) in [(240.0, 90.0), (96.0, 48.0), (56.0, 32.0), (240.0, 90.0)] {
            ui.layout(
                view,
                Size {
                    width: px(width),
                    height: px(300.0),
                },
            )?;
            let mut rows = ui.children(view, ui.root(view)?)?;
            let row = rows.next().unwrap();
            let marker = ui.geometry(view, rows.next().unwrap()).unwrap();
            let mut items = ui.children(view, row)?;
            let label = items.next().unwrap();
            let action = ui.geometry(view, items.next().unwrap()).unwrap();
            let label_box = ui.geometry(view, label).unwrap();
            let outer = inner + 4.5;
            assert_eq!(label_box.metrics.size.width, px(outer));
            assert_eq!(
                action.origin.x,
                px(if direction == Row {
                    outer + 3.5
                } else {
                    width - outer - 3.5 - 8.0
                })
            );
            let mut draw = Vec::new();
            ui.paint(view, label, &mut draw)?;
            let [
                Draw::Text {
                    size, paragraph, ..
                },
            ] = &draw[..]
            else {
                panic!("expected shaped text")
            };
            assert_eq!(size.width, px(inner));
            assert_eq!(marker.origin.y, px(paragraph.lines.len() as f32 * 20.0));
            lines.push(paragraph.lines.len());
        }
        assert!(lines[2] > lines[0]);
        assert_eq!(lines[0], lines[3]);
    }
    Ok(())
}

#[test]
fn column_basis_precedes_height_and_resolves_inside_padding_before_justification() -> Result<()> {
    for direction in [Column, ColumnReverse] {
        let mut ui = Ui::new();
        let view = ui.mount(component(|_| {
            div()
                .flex_direction(direction)
                .justify_end()
                .p(px(2.0))
                .gap(px(3.5))
                .child(
                    div()
                        .box_sizing(BoxSizing::ContentBox)
                        .py(px(2.5))
                        .h(px(3.0))
                        .flex_basis(percent(50.0))
                        .max_h(rem(3.0))
                        .child(div().h(percent(50.0))),
                )
                .child(div().h(px(8.0)).flex_basis(Length::Auto))
        }))?;
        for (height, inner) in [(144.0, 48.0), (68.0, 32.0)] {
            ui.layout(
                view,
                Size {
                    width: px(128.0),
                    height: px(height),
                },
            )?;
            let mut items = ui.children(view, ui.root(view)?)?;
            let card = items.next().unwrap();
            let child = ui
                .geometry(view, ui.children(view, card)?.next().unwrap())
                .unwrap();
            let card = ui.geometry(view, card).unwrap();
            let last = ui.geometry(view, items.next().unwrap()).unwrap();
            assert_eq!(child.metrics.size.height, px(inner / 2.0));
            assert_eq!(card.metrics.size.height, px(inner + 5.0));
            assert_eq!(last.metrics.size.height, px(8.0));
            assert_eq!(
                last.origin.y,
                px(if direction == Column {
                    height - 10.0
                } else {
                    2.0
                })
            );
            assert_eq!(
                card.origin.y,
                px(if direction == Column {
                    height - 18.5 - inner
                } else {
                    13.5
                })
            );
        }
    }
    Ok(())
}
