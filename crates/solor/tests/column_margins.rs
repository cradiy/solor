//! Outer spacing as observed in published UI geometry.
use anyhow::Result;
use solor::{
    Length, Styled, Ui, component, div,
    layout::{Origin, Size},
    percent, px, rem,
};

#[test]
fn nested_column_resolves_margins_against_its_padded_content_on_resize() -> Result<()> {
    let mut ui = Ui::new();
    let view = ui.mount(component(|_| {
        div().child(
            div()
                .w_full()
                .p(px(5.0))
                .gap(px(4.0))
                .child(
                    div()
                        .w(px(20.5))
                        .h(px(8.0))
                        .mx_auto()
                        .mt(percent(10.0))
                        .mb(px(2.25)),
                )
                .child(
                    div()
                        .w_full()
                        .h(px(6.0))
                        .ml(px(-3.5))
                        .mr(rem(0.5))
                        .mt(px(-3.0)),
                ),
        )
    }))?;
    let card = ui.children(view, ui.root(view)?)?.next().unwrap();
    let rows: Vec<_> = ui.children(view, card)?.collect();
    for (width, first_x, first_y, second_width, card_height) in [
        (120.0, 49.75, 16.0, 105.5, 38.25),
        (80.0, 29.75, 12.0, 65.5, 34.25),
        (120.0, 49.75, 16.0, 105.5, 38.25),
    ] {
        ui.layout(
            view,
            Size {
                width: px(width),
                height: px(100.0),
            },
        )?;
        let first = ui.geometry(view, rows[0]).unwrap();
        let second = ui.geometry(view, rows[1]).unwrap();
        assert_eq!(
            first.origin,
            Origin {
                x: px(first_x),
                y: px(first_y)
            }
        );
        assert_eq!(first.metrics.size.width, px(20.5));
        assert_eq!(
            second.origin,
            Origin {
                x: px(1.5),
                y: px(first_y + 11.25)
            }
        );
        assert_eq!(second.metrics.size.width, px(second_width));
        assert_eq!(
            ui.geometry(view, card).unwrap().metrics.size.height,
            px(card_height)
        );
    }
    Ok(())
}

#[test]
fn automatic_margin_shares_only_positive_free_space() -> Result<()> {
    let mut ui = Ui::new();
    let view = ui.mount(component(|_| {
        div()
            .child(div().w(px(15.25)).h(px(2.0)).m(Length::Auto).mr(px(3.0)))
            .child(div().w_full().h(px(2.0)).ml(px(-5.0)).mr(px(35.0)))
    }))?;
    let root = ui.root(view)?;
    let rows: Vec<_> = ui.children(view, root)?.collect();
    ui.layout(
        view,
        Size {
            width: px(20.0),
            height: px(50.0),
        },
    )?;
    assert_eq!(
        ui.geometry(view, rows[0]).unwrap().origin,
        Origin {
            x: px(1.75),
            y: px(23.0)
        }
    );
    let second = ui.geometry(view, rows[1]).unwrap();
    assert_eq!(
        second.origin,
        Origin {
            x: px(-5.0),
            y: px(48.0)
        }
    );
    assert_eq!(second.metrics.size.width, px(0.0));
    assert_eq!(ui.geometry(view, root).unwrap().content.width, px(30.0));
    Ok(())
}

#[test]
fn margin_transition_resolves_against_the_live_parent_and_applies_auto_per_edge() -> Result<()> {
    use solor::{Transition, TransitionProperty};
    use std::time::Duration;

    let mut ui = Ui::new();
    let left = ui.context().signal(Length::from(px(-8.)));
    let right = ui.context().signal(Length::Auto);
    let view = ui.mount_with_rem_size(
        component(move |_| {
            div()
                .flex_row()
                .child(div().size(px(20.)).ml(left).mr(right).transition(
                    TransitionProperty::Margin,
                    Transition::new(Duration::from_millis(100)),
                ))
                .child(div().size(px(10.)))
        }),
        px(16.),
    )?;
    let items: Vec<_> = ui.children(view, ui.root(view)?)?.collect();
    ui.context().set(left, percent(25.))?;
    ui.context().set(right, Length::from(rem(0.5)))?;
    for (ms, width, x, auto) in [
        (0, 200., -8., false),
        (50, 200., 21., false),
        (50, 120., 11., false),
        (100, 120., 30., false),
        (100, 120., 82., true),
    ] {
        if auto {
            ui.context().set(left, Length::Auto)?;
        }
        ui.dispatch_frame(view, Duration::from_millis(ms))?;
        ui.layout(
            view,
            Size {
                width: px(width),
                height: px(50.),
            },
        )?;
        assert_eq!(ui.geometry(view, items[0]).unwrap().origin.x, px(x));
        assert_eq!(ui.geometry(view, items[1]).unwrap().origin.x, px(x + 28.));
        assert_eq!(ui.record_frame(view)?.requests_animation_frame(), ms < 100);
    }
    Ok(())
}
