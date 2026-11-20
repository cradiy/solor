//! Flow geometry and overflow for relatively positioned children.
use anyhow::Result;
use solor::{
    Length, Styled, Ui, component, div,
    layout::{Origin, Size},
    percent, px, rem,
    style::Position,
};

fn size(width: f32, height: f32) -> Size {
    Size {
        width: px(width),
        height: px(height),
    }
}

fn at(x: f32, y: f32) -> Origin {
    Origin { x: px(x), y: px(y) }
}

#[test]
fn relative_offsets_resolve_against_the_parent_content_box_after_flex_placement() -> Result<()> {
    let mut ui = Ui::new();
    let view = ui.mount(component(|_| {
        div()
            .flex_row_reverse()
            .p(px(5.0))
            .gap(px(2.0))
            .child(
                div()
                    .size(px(12.0))
                    .relative()
                    .left(percent(50.0))
                    .right(px(200.0))
                    .top(Length::Auto)
                    .bottom(percent(-25.0)),
            )
            .child(
                div()
                    .size(px(10.0))
                    .position(Position::Static)
                    .left(px(80.0)),
            )
    }))?;
    let root = ui.root(view)?;
    let (moved, sibling) = {
        let mut children = ui.children(view, root)?;
        (children.next().unwrap(), children.next().unwrap())
    };
    for (width, height) in [(100.0, 60.0), (70.0, 30.0), (100.0, 60.0)] {
        ui.layout(view, size(width, height))?;
        let moved = ui.geometry(view, moved).unwrap();
        let x = width - 17.0 + (width - 10.0) * 0.5;
        let y = 5.0 + (height - 10.0) * 0.25;
        assert_eq!(moved.origin, at(x, y));
        assert_eq!(moved.metrics.size, size(12.0, 12.0));
        assert_eq!(
            ui.geometry(view, sibling).unwrap().origin,
            at(width - 29.0, 5.0)
        );
        assert_eq!(ui.geometry(view, root).unwrap().content.width, px(x + 12.0));
    }
    Ok(())
}

#[test]
fn a_shifted_child_keeps_its_flow_slot_and_reports_descendant_overflow() -> Result<()> {
    let mut ui = Ui::new();
    let view = ui.mount(component(|_| {
        div()
            .child(
                div()
                    .relative()
                    .right(rem(0.5))
                    .top(px(7.25))
                    .bottom(px(80.0))
                    .w(px(10.0))
                    .child(div().relative().top(px(20.0)).size(px(8.0))),
            )
            .child(div().size(px(6.0)))
    }))?;
    ui.layout(view, size(100.0, 80.0))?;
    let root = ui.root(view)?;
    let mut children = ui.children(view, root)?;
    let group = children.next().unwrap();
    let parent = ui.geometry(view, group).unwrap();
    let inner = ui.children(view, group)?.next().unwrap();
    assert_eq!(parent.origin, at(-8.0, 7.25));
    assert_eq!(ui.geometry(view, inner).unwrap().origin, at(0.0, 20.0));
    assert_eq!(parent.metrics.size.height, px(8.0));
    assert_eq!(parent.content.height, px(28.0));
    assert_eq!(
        ui.geometry(view, children.next().unwrap()).unwrap().origin,
        at(0.0, 8.0)
    );
    assert_eq!(ui.geometry(view, root).unwrap().content.height, px(35.25));
    Ok(())
}
