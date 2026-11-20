//! Height limits during measurement, baseline alignment and placement.
use anyhow::Result;
use solor::{
    Draw, SharedString, Styled, Ui, component, div, layout::Size, percent, px, rem,
    style::BoxSizing, text,
};

const FONT: &[u8] = include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf");

#[test]
fn capped_label_reserves_its_used_height_and_preserves_paragraph_overflow() -> Result<()> {
    let mut ui = Ui::with_font(FONT.to_vec(), "zh-CN")?;
    let caption = ui.context().signal(SharedString::from_static("A"));
    let view = ui.mount(component(|_| {
        div()
            .gap(px(2.5))
            .child(
                text(caption)
                    .box_sizing(BoxSizing::ContentBox)
                    .max_h(percent(20.0))
                    .p(px(2.0))
                    .line_height(px(20.0)),
            )
            .child(div().size(px(4.0)))
    }))?;
    let root = ui.root(view)?;
    for (value, viewport, used, lines) in [
        ("A\nB\nC", 120.0, 24.0, 3),
        ("A", 120.0, 20.0, 1),
        ("A\nB\nC", 15.0, 3.0, 3),
        ("A\nB\nC", 120.0, 24.0, 3),
    ] {
        ui.context()
            .set(caption, SharedString::from_static(value))?;
        ui.flush()?;
        ui.layout(
            view,
            Size {
                width: px(180.0),
                height: px(viewport),
            },
        )?;
        let mut rows = ui.children(view, root)?;
        let label = rows.next().unwrap();
        let marker = ui.geometry(view, rows.next().unwrap()).unwrap();
        let geometry = ui.geometry(view, label).unwrap();
        assert_eq!(geometry.metrics.size.height, px(used + 4.0));
        assert_eq!(geometry.content.height, px(lines as f32 * 20.0 + 4.0));
        assert_eq!(marker.origin.y, geometry.metrics.size.height + px(2.5));
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
        assert_eq!(size.height, px(used));
        assert_eq!(paragraph.lines.len(), lines);
        assert_eq!(
            geometry.metrics.baseline,
            paragraph
                .baseline
                .map(|b| b + px(2.0))
                .filter(|b| *b <= geometry.metrics.size.height)
        );
    }
    Ok(())
}

#[test]
fn row_stretch_clamps_the_content_box_before_laying_out_percentage_children() -> Result<()> {
    use solor::style::FlexDirection::{Row, RowReverse};
    for direction in [Row, RowReverse] {
        let mut ui = Ui::new();
        let view = ui.mount(component(|_| {
            div()
                .flex_direction(direction)
                .items_stretch()
                .p(px(2.0))
                .child(
                    div()
                        .box_sizing(BoxSizing::ContentBox)
                        .min_h(rem(3.0))
                        .max_h(percent(50.0))
                        .py(px(2.5))
                        .mt(px(1.25))
                        .mb(px(2.75))
                        .child(div().h(percent(50.0))),
                )
                .child(div().h(px(150.0)).max_h(px(19.5)))
        }))?;
        let root = ui.root(view)?;
        for (parent_height, inner) in [(200.0, 100.0), (32.0, 48.0), (200.0, 100.0)] {
            ui.layout(
                view,
                Size {
                    width: px(100.0),
                    height: px(parent_height + 4.0),
                },
            )?;
            let mut items = ui.children(view, root)?;
            let group = items.next().unwrap();
            let fixed = ui.geometry(view, items.next().unwrap()).unwrap();
            let child = ui
                .geometry(view, ui.children(view, group)?.next().unwrap())
                .unwrap();
            let group = ui.geometry(view, group).unwrap();
            assert_eq!(group.origin.y, px(3.25));
            assert_eq!(group.metrics.size.height, px(inner + 5.0));
            assert_eq!(child.origin.y, px(2.5));
            assert_eq!(child.metrics.size.height, px(inner * 0.5));
            assert_eq!(fixed.metrics.size.height, px(19.5));
        }
    }
    Ok(())
}

#[test]
fn minimum_column_height_updates_its_preferred_baseline_and_the_following_row() -> Result<()> {
    let mut ui = Ui::with_font(FONT.to_vec(), "zh-CN")?;
    let view = ui.mount(component(|_| {
        div()
            .child(
                div()
                    .flex_row()
                    .items_baseline()
                    .child(text("A").line_height(px(20.0)).pb(px(10.0)))
                    .child(
                        div()
                            .min_h(px(60.0))
                            .justify_end()
                            .child(text("B").line_height(px(20.0))),
                    ),
            )
            .child(div().size(px(4.0)))
    }))?;
    ui.layout(
        view,
        Size {
            width: px(200.0),
            height: px(120.0),
        },
    )?;
    let mut rows = ui.children(view, ui.root(view)?)?;
    let row = rows.next().unwrap();
    let marker = ui.geometry(view, rows.next().unwrap()).unwrap();
    let mut cells = ui.children(view, row)?;
    let left = ui.geometry(view, cells.next().unwrap()).unwrap();
    let group = cells.next().unwrap();
    let label = ui
        .geometry(view, ui.children(view, group)?.next().unwrap())
        .unwrap();
    let group = ui.geometry(view, group).unwrap();
    assert_eq!(group.metrics.size.height, px(60.0));
    assert_eq!(label.origin.y, px(40.0));
    assert_eq!(
        left.origin.y + left.metrics.baseline.unwrap(),
        group.origin.y + label.origin.y + label.metrics.baseline.unwrap()
    );
    assert_eq!(
        ui.geometry(view, row).unwrap().metrics.size.height,
        px(70.0)
    );
    assert_eq!(marker.origin.y, px(70.0));
    Ok(())
}
