//! Anchored overlays and in-flow siblings under a shared parent.
use anyhow::Result;
use solor::{
    Styled, Ui, component, div,
    layout::{Origin, Size},
    percent, px, rem,
    style::Position,
};

fn size(w: f32, h: f32) -> Size {
    Size {
        width: px(w),
        height: px(h),
    }
}

fn at(x: f32, y: f32) -> Origin {
    Origin { x: px(x), y: px(y) }
}

#[test]
fn changing_flow_membership_remeasures_the_parent_and_preserves_the_child() -> Result<()> {
    let mut ui = Ui::new();
    let position = ui.context().signal(Position::Absolute);
    let view = ui.mount(component(|_| {
        div().items_start().child(
            div()
                .p(px(4.))
                .gap(px(5.))
                .child(div().size(px(10.)))
                .child(
                    div()
                        .position(position)
                        .right(px(-2.5))
                        .bottom(rem(0.25))
                        .size(px(24.)),
                )
                .child(div().size(px(8.))),
        )
    }))?;
    let group = ui.children(view, ui.root(view)?)?.next().unwrap();
    let children: Vec<_> = ui.children(view, group)?.collect();
    for mode in [Position::Absolute, Position::Static, Position::Absolute] {
        ui.context().set(position, mode)?;
        ui.flush()?;
        ui.layout(view, size(120., 100.))?;
        let overlay = ui.geometry(view, children[1]).unwrap();
        let parent = ui.geometry(view, group).unwrap();
        let last = ui.geometry(view, children[2]).unwrap();
        if mode == Position::Absolute {
            assert_eq!(parent.metrics.size, size(18., 31.));
            assert_eq!(parent.content, size(20.5, 31.));
            assert_eq!(last.origin, at(4., 19.));
            assert_eq!(overlay.origin, at(-3.5, 3.));
        } else {
            assert_eq!(parent.metrics.size, size(32., 60.));
            assert_eq!(last.origin, at(4., 48.));
            assert_eq!(overlay.origin, at(4., 19.));
        }
    }
    Ok(())
}

#[test]
fn opposing_insets_resolve_on_resize_before_laying_out_the_overlay_contents() -> Result<()> {
    let mut ui = Ui::new();
    let view = ui.mount(component(|_| {
        div().p(px(7.)).child(
            div()
                .absolute()
                .left(percent(10.))
                .right(px(5.))
                .top(px(3.))
                .bottom(percent(25.))
                .max_w(px(80.))
                .child(div().absolute().bottom(px(1.)).right(px(1.)).size(px(3.))),
        )
    }))?;
    let overlay = ui.children(view, ui.root(view)?)?.next().unwrap();
    let badge = ui.children(view, overlay)?.next().unwrap();
    for (width, height, w, h) in [(100., 40., 80., 27.), (60., 20., 49., 12.)] {
        ui.layout(view, size(width, height))?;
        let geometry = ui.geometry(view, overlay).unwrap();
        assert_eq!(geometry.origin, at(width * 0.1, 3.));
        assert_eq!(geometry.metrics.size, size(w, h));
        assert_eq!(ui.geometry(view, badge).unwrap().origin, at(w - 4., h - 4.));
    }
    Ok(())
}
