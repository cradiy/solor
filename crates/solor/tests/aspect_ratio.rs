//! Proportional boxes under retained layout and flex allocation.
use anyhow::Result;
use solor::{Styled, Ui, component, div, layout::Size, percent, px, style::BoxSizing};

fn size(width: f32, height: f32) -> Size {
    Size {
        width: px(width),
        height: px(height),
    }
}

#[test]
fn a_flexible_preview_reserves_its_height_at_the_final_content_width() -> Result<()> {
    let mut ui = Ui::new();
    let view = ui.mount(component(|_| {
        div()
            .items_stretch()
            .child(
                div()
                    .flex_row()
                    .gap(px(4.0))
                    .child(div().size(px(20.0)))
                    .child(
                        div()
                            .box_sizing(BoxSizing::ContentBox)
                            .flex_basis(px(80.0))
                            .flex_grow(1.0)
                            .flex_shrink(1.0)
                            .aspect_ratio(2.0)
                            .px(px(2.5))
                            .py(px(1.5))
                            .min_h(px(24.0))
                            .max_h(px(56.0))
                            .child(div().w_full().h(percent(50.0))),
                    ),
            )
            .child(div().size(px(3.0)))
    }))?;
    for (width, content_height) in [(125.5, 48.25), (61.0, 24.0), (180.0, 56.0), (125.5, 48.25)] {
        ui.layout(view, size(width, 200.0))?;
        let mut rows = ui.children(view, ui.root(view)?)?;
        let row = rows.next().unwrap();
        let preview = ui.children(view, row)?.nth(1).unwrap();
        let child = ui.children(view, preview)?.next().unwrap();
        let preview_height = content_height + 3.0;
        assert_eq!(
            ui.geometry(view, preview).unwrap().metrics.size,
            size(width - 24.0, preview_height)
        );
        assert_eq!(
            ui.geometry(view, child).unwrap().metrics.size,
            size(width - 29.0, content_height * 0.5)
        );
        assert_eq!(
            ui.geometry(view, rows.next().unwrap()).unwrap().origin.y,
            px(preview_height)
        );
    }
    Ok(())
}

#[test]
fn definite_height_supplies_width_within_the_box_model_and_width_limits() -> Result<()> {
    let mut ui = Ui::new();
    let view = ui.mount(component(|_| {
        div()
            .flex_row()
            .child(div().aspect_square().h(percent(50.0)).p(px(2.5)))
            .child(
                div()
                    .box_sizing(BoxSizing::ContentBox)
                    .aspect_ratio(1.5)
                    .h(percent(50.0))
                    .px(px(3.0))
                    .py(px(1.0))
                    .max_w(px(50.0)),
            )
            .child(div().aspect_square().w(px(14.0)).h(px(9.0)))
    }))?;
    for (height, half, inner_width) in [(48.0, 24.0, 36.0), (100.0, 50.0, 50.0)] {
        ui.layout(view, size(180.0, height))?;
        let mut nodes = ui.children(view, ui.root(view)?)?;
        let square = ui.geometry(view, nodes.next().unwrap()).unwrap();
        let padded = ui.geometry(view, nodes.next().unwrap()).unwrap();
        let fixed = ui.geometry(view, nodes.next().unwrap()).unwrap();
        assert_eq!(square.metrics.size, size(half, half));
        assert_eq!(padded.metrics.size, size(inner_width + 6.0, half + 2.0));
        assert_eq!(fixed.metrics.size, size(14.0, 9.0));
        assert_eq!(fixed.origin.x, px(half + inner_width + 6.0));
    }
    Ok(())
}

#[test]
fn invalid_aspect_ratios_fail_at_attachment() {
    for ratio in [0.0, -1.0, f32::INFINITY, f32::NAN] {
        let mut ui = Ui::new();
        assert!(
            ui.mount(component(move |_| div().aspect_ratio(ratio)))
                .is_err()
        );
    }
}
