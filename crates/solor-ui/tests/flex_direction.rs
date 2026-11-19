//! Container direction, local spacing and published child geometry.
use anyhow::Result;
use solor_ui::{
    Div, StyleRefinement, Styled, Ui, component, div,
    layout::{Origin, Size},
    percent, px,
};

fn pair(container: Div) -> Div {
    container
        .gap_x(px(2.5))
        .gap_y(px(4.0))
        .child(div().w(px(10.5)).h(px(6.0)))
        .child(div().w(px(5.0)).h(px(7.5)))
}

#[test]
fn direction_composes_across_children_and_style_refinements() -> Result<()> {
    let mut ui = Ui::new();
    let horizontal = StyleRefinement::default().flex_row();
    let view = ui.mount(component(|_| {
        div()
            .child(pair(div().flex_row()))
            .child(pair(div()).with_style(&horizontal))
            .child(pair(div().with_style(&horizontal)).flex_col())
            .child(pair(div()))
    }))?;
    ui.layout(
        view,
        Size {
            width: px(100.0),
            height: px(100.0),
        },
    )?;
    let containers: Vec<_> = ui.children(view, ui.root(view)?)?.collect();
    for (index, container) in containers.into_iter().enumerate() {
        let second = ui.children(view, container)?.nth(1).unwrap();
        let (width, height, x, y) = if index < 2 {
            (18.0, 7.5, 13.0, 0.0)
        } else {
            (10.5, 17.5, 0.0, 10.0)
        };
        assert_eq!(
            ui.geometry(view, container).unwrap().metrics.size,
            Size {
                width: px(width),
                height: px(height)
            }
        );
        assert_eq!(
            ui.geometry(view, second).unwrap().origin,
            Origin { x: px(x), y: px(y) }
        );
    }
    Ok(())
}

#[test]
fn reversed_directions_anchor_to_the_far_edge_without_reordering_children() -> Result<()> {
    let mut ui = Ui::new();
    let view = ui.mount(component(|_| {
        div()
            .child(pair(div().flex_row_reverse()).w(px(40.0)).p(px(1.0)))
            .child(pair(div().flex_col_reverse()).h(px(40.0)).p(px(1.0)))
    }))?;
    ui.layout(
        view,
        Size {
            width: px(100.0),
            height: px(100.0),
        },
    )?;
    let parents: Vec<_> = ui.children(view, ui.root(view)?)?.collect();
    for (parent, offsets) in parents
        .into_iter()
        .zip([[(28.5, 1.0), (21.0, 1.0)], [(1.0, 33.0), (1.0, 21.5)]])
    {
        for (child, (x, y)) in ui.children(view, parent)?.zip(offsets) {
            assert_eq!(
                ui.geometry(view, child).unwrap().origin,
                Origin { x: px(x), y: px(y) }
            );
        }
    }
    Ok(())
}

#[test]
fn horizontal_auto_margins_follow_resize_and_conditional_membership() -> Result<()> {
    let mut ui = Ui::new();
    let shown = ui.context().signal(false);
    let view = ui.mount(component(|_| {
        div()
            .flex_row()
            .gap_x(px(2.0))
            .child(div().w(percent(25.0)).h(px(5.0)).mx_auto())
            .child_when(shown, || div().w(px(10.0)).h(px(3.0)))
    }))?;
    let root = ui.root(view)?;
    for (width, visible, x) in [(80.0, false, 30.0), (80.0, true, 24.0), (40.0, true, 9.0)] {
        ui.context().set(shown, visible)?;
        ui.flush()?;
        ui.layout(
            view,
            Size {
                width: px(width),
                height: px(20.0),
            },
        )?;
        let mut items = ui.children(view, root)?;
        let first = ui.geometry(view, items.next().unwrap()).unwrap();
        assert_eq!(first.metrics.size.width, px(width / 4.0));
        assert_eq!(first.origin.x, px(x));
        if visible {
            assert_eq!(
                ui.geometry(view, items.next().unwrap()).unwrap().origin.x,
                px(width - 10.0)
            );
        }
    }
    Ok(())
}
