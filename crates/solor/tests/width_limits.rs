//! Width negotiation for retained elements in horizontal and vertical flow.
use anyhow::Result;
use solor::{
    Draw, Styled, Ui, component, div, layout::Size, percent, px, rem, style::BoxSizing, text,
};

#[test]
fn stretched_content_box_respects_limits_and_reflows_its_percentage_label() -> Result<()> {
    let font = include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf");
    let mut ui = Ui::with_font(font.to_vec(), "zh-CN")?;
    let view = ui.mount(component(|_| {
        div()
            .items_stretch()
            .child(
                div()
                    .box_sizing(BoxSizing::ContentBox)
                    .min_w(rem(3.0))
                    .max_w(percent(50.0))
                    .mx(px(5.0))
                    .p(px(2.5))
                    .child(
                        text("Audio settings and output device")
                            .w(percent(50.0))
                            .line_height(px(20.0)),
                    ),
            )
            .child(div().h(px(4.0)))
    }))?;
    let root = ui.root(view)?;
    let mut counts = Vec::new();
    for (viewport, inner) in [(240.0, 120.0), (32.0, 48.0), (240.0, 120.0)] {
        ui.layout(
            view,
            Size {
                width: px(viewport),
                height: px(300.0),
            },
        )?;
        let mut rows = ui.children(view, root)?;
        let card = rows.next().unwrap();
        let marker = ui.geometry(view, rows.next().unwrap()).unwrap();
        let label = ui.children(view, card)?.next().unwrap();
        let card = ui.geometry(view, card).unwrap();
        assert_eq!(card.origin.x, px(5.0));
        assert_eq!(card.metrics.size.width, px(inner + 5.0));
        let mut commands = Vec::new();
        ui.paint(view, label, &mut commands)?;
        let [
            Draw::Text {
                paragraph, size, ..
            },
        ] = &commands[..]
        else {
            panic!("expected a paragraph")
        };
        assert_eq!(size.width, px(inner * 0.5));
        let lines = paragraph.lines.len();
        assert_eq!(card.metrics.size.height, px(lines as f32 * 20.0 + 5.0));
        assert_eq!(marker.origin.y, card.metrics.size.height);
        counts.push(lines);
    }
    assert!(counts[1] > counts[0]);
    assert_eq!(counts[0], counts[2]);
    Ok(())
}

#[test]
fn row_uses_bounded_preferred_widths_even_when_the_minimum_overflows() -> Result<()> {
    let mut ui = Ui::new();
    let view = ui.mount(component(|_| {
        div()
            .flex_row()
            .gap(px(2.0))
            .child(div().w(px(140.0)).max_w(percent(50.0)))
            .child(div().w(px(1.0)).min_w(px(40.25)).max_w(px(12.0)))
            .child(div().size(px(6.0)))
    }))?;
    for width in [100.0, 30.0] {
        ui.layout(
            view,
            Size {
                width: px(width),
                height: px(80.0),
            },
        )?;
        let mut cells = ui.children(view, ui.root(view)?)?;
        let first = ui.geometry(view, cells.next().unwrap()).unwrap();
        let second = ui.geometry(view, cells.next().unwrap()).unwrap();
        let last = ui.geometry(view, cells.next().unwrap()).unwrap();
        assert_eq!(first.metrics.size.width, px(width * 0.5));
        assert_eq!(second.metrics.size.width, px(40.25));
        assert_eq!(second.origin.x, first.metrics.size.width + px(2.0));
        assert_eq!(last.origin.x, second.origin.x + px(42.25));
    }
    Ok(())
}

#[test]
fn intrinsic_container_measures_descendants_again_at_its_used_width() -> Result<()> {
    let font = include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf");
    let mut ui = Ui::with_font(font.to_vec(), "zh-CN")?;
    let view = ui.mount(component(|_| {
        div()
            .child(
                div().max_w(px(96.0)).child(
                    text("Audio settings and output device")
                        .w(percent(25.0))
                        .line_height(px(20.0)),
                ),
            )
            .child(div().h(px(4.0)))
    }))?;
    ui.layout(
        view,
        Size {
            width: px(140.0),
            height: px(400.0),
        },
    )?;
    let mut rows = ui.children(view, ui.root(view)?)?;
    let card = rows.next().unwrap();
    let marker = ui.geometry(view, rows.next().unwrap()).unwrap();
    let label = ui.children(view, card)?.next().unwrap();
    let card = ui.geometry(view, card).unwrap();
    let label = ui.geometry(view, label).unwrap();
    assert!(card.metrics.size.width <= px(96.0));
    assert_eq!(label.metrics.size.width, card.metrics.size.width * 0.25);
    assert!(label.metrics.size.height > px(20.0));
    assert_eq!(marker.origin.y, label.metrics.size.height);
    Ok(())
}
