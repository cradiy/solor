//! Fixed and signal-controlled children within the same declaration.
use anyhow::Result;
use solor::{Div, Styled, Ui, component, div, layout::Size, px};

fn item(width: f32) -> Div {
    div().w(px(width)).h(px(1.0))
}

#[test]
fn iterated_siblings_keep_order_and_identity_across_a_conditional_branch() -> Result<()> {
    let mut ui = Ui::new();
    let open = ui.context().signal(false);
    let widths = [2.5, 3.0];
    let view = ui.mount(component(|_| {
        div()
            .when(true, |list| list.flex_row().gap_x(px(0.5)))
            .when_some(Some(1.0), |list, width| list.child(item(width)))
            .children(widths.iter().copied().map(item))
            .child_when(open, || item(4.0))
            .when(false, |list| list.child(item(99.0)))
            .when_some(None, |list, width| list.child(item(width)))
            .children([item(5.0)])
    }))?;
    let root = ui.root(view)?;
    let viewport = Size {
        width: px(40.0),
        height: px(1.0),
    };
    ui.flush()?;
    ui.layout(view, viewport)?;
    let fixed: Vec<_> = ui.children(view, root)?.collect();
    assert_eq!(fixed.len(), 4);
    for (id, (width, x)) in fixed
        .iter()
        .zip([(1.0, 0.0), (2.5, 1.5), (3.0, 4.5), (5.0, 8.0)])
    {
        let geometry = ui.geometry(view, *id).unwrap();
        assert_eq!(geometry.metrics.size.width, px(width));
        assert_eq!(geometry.origin.x, px(x));
    }

    ui.context().set(open, true)?;
    ui.flush()?;
    ui.layout(view, viewport)?;
    let expanded: Vec<_> = ui.children(view, root)?.collect();
    assert_eq!(expanded.len(), 5);
    assert_eq!(expanded[..3], fixed[..3]);
    assert_eq!(expanded[4], fixed[3]);
    assert_eq!(
        ui.geometry(view, expanded[3]).unwrap().metrics.size.width,
        px(4.0)
    );
    assert_eq!(ui.geometry(view, fixed[3]).unwrap().origin.x, px(12.5));

    ui.context().set(open, false)?;
    ui.flush()?;
    assert_eq!(ui.children(view, root)?.collect::<Vec<_>>(), fixed);
    Ok(())
}
