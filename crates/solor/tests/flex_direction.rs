//! Container direction, local spacing and published child geometry.
use anyhow::Result;
use solor::{
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

#[test]
fn justify_distributes_space_inside_padding_in_both_directions() -> Result<()> {
    use solor::style::{Distribution::*, FlexDirection::*};

    for direction in [Row, RowReverse, Column, ColumnReverse] {
        for (justify, first, second) in [
            (Start, 0.0, 14.5),
            (End, 20.0, 34.5),
            (Center, 10.0, 24.5),
            (SpaceBetween, 0.0, 34.5),
            (SpaceAround, 5.0, 29.5),
            (SpaceEvenly, 20.0 / 3.0, 14.5 + 40.0 / 3.0),
        ] {
            let mut ui = Ui::new();
            let view = ui.mount(component(|_| {
                div()
                    .flex_direction(direction)
                    .justify_content(justify)
                    .p(px(2.0))
                    .gap(px(4.5))
                    .child(div().size(px(10.0)))
                    .child(div().size(px(10.0)))
            }))?;
            ui.layout(
                view,
                Size {
                    width: px(48.5),
                    height: px(48.5),
                },
            )?;
            for (id, offset) in ui.children(view, ui.root(view)?)?.zip([first, second]) {
                let origin = ui.geometry(view, id).unwrap().origin;
                let (actual, cross) = match direction {
                    Row | RowReverse => (origin.x, origin.y),
                    Column | ColumnReverse => (origin.y, origin.x),
                };
                let expected = match direction {
                    Row | Column => 2.0 + offset,
                    RowReverse | ColumnReverse => 36.5 - offset,
                };
                assert!(
                    (actual.get() - expected).abs() < 0.0001,
                    "{direction:?} {justify:?}"
                );
                assert_eq!(cross, px(2.0));
            }
        }
    }
    Ok(())
}

#[test]
fn column_auto_margin_precedes_justification_until_the_line_overflows() -> Result<()> {
    use solor::Length;
    let mut ui = Ui::new();
    let visible = ui.context().signal(false);
    let view = ui.mount(component(|_| {
        div()
            .flex_col()
            .justify_center()
            .gap(px(3.0))
            .child(div().size(px(10.0)).mt(Length::Auto))
            .child_when(visible, || div().size(px(12.0)))
    }))?;
    for (height, show, first_y) in [(44.0, false, 34.0), (44.0, true, 19.0), (20.0, true, -2.5)] {
        ui.context().set(visible, show)?;
        ui.flush()?;
        ui.layout(
            view,
            Size {
                width: px(32.0),
                height: px(height),
            },
        )?;
        let mut items = ui.children(view, ui.root(view)?)?;
        assert_eq!(
            ui.geometry(view, items.next().unwrap()).unwrap().origin.y,
            px(first_y)
        );
        if show {
            assert_eq!(
                ui.geometry(view, items.next().unwrap()).unwrap().origin.y,
                px(first_y + 13.0)
            );
        }
    }
    Ok(())
}

#[test]
fn cross_alignment_uses_the_margin_box_and_child_overrides() -> Result<()> {
    use solor::{Length, style::FlexDirection::*};

    for direction in [Row, RowReverse, Column, ColumnReverse] {
        let row = matches!(direction, Row | RowReverse);
        let mut ui = Ui::new();
        let shown = ui.context().signal(false);
        let item = move || {
            let item = div().size(px(8.0));
            if row {
                item.mt(px(3.5)).mb(px(1.5))
            } else {
                item.ml(px(3.5)).mr(px(1.5))
            }
        };
        let view = ui.mount(component(|_| {
            div()
                .p(px(2.0))
                .flex_direction(direction)
                .items_center()
                .child(item())
                .child(item().self_end())
                .child_when(shown, move || item().self_start())
                .child(if row {
                    item().mb(Length::Auto)
                } else {
                    item().mr(Length::Auto)
                })
        }))?;
        for (cross, visible) in [(33.0, false), (23.0, true)] {
            ui.context().set(shown, visible)?;
            ui.flush()?;
            let (width, height) = if row { (90.0, cross) } else { (cross, 90.0) };
            ui.layout(
                view,
                Size {
                    width: px(width),
                    height: px(height),
                },
            )?;
            let positions: Vec<_> = ui
                .children(view, ui.root(view)?)?
                .map(|id| {
                    let origin = ui.geometry(view, id).unwrap().origin;
                    if row { origin.y } else { origin.x }
                })
                .collect();
            let center = px(5.5 + (cross - 17.0) / 2.0);
            let end = px(cross - 11.5);
            if visible {
                assert_eq!(positions, [center, end, px(5.5), px(5.5)]);
            } else {
                assert_eq!(positions, [center, end, px(5.5)]);
            }
        }
    }
    Ok(())
}

#[test]
fn stretch_respects_cross_sizes_and_margins_in_each_direction() -> Result<()> {
    use solor::{Length, style::FlexDirection::*};

    for direction in [Row, RowReverse, Column, ColumnReverse] {
        let row = matches!(direction, Row | RowReverse);
        let mut ui = Ui::new();
        let item = || div().child(div().size(px(3.0)));
        let view = ui.mount(component(|_| {
            div()
                .flex_direction(direction)
                .p(px(2.0))
                .items_stretch()
                .child(if row {
                    item().mt(px(1.25)).mb(px(2.5))
                } else {
                    item().ml(px(1.25)).mr(px(2.5))
                })
                .child(if row {
                    item().h(percent(25.0))
                } else {
                    item().w(percent(25.0))
                })
                .child(if row {
                    item().h(px(0.0))
                } else {
                    item().w(px(0.0))
                })
                .child(if row {
                    item().mt(Length::Auto)
                } else {
                    item().ml(Length::Auto)
                })
                .child(item().self_center())
        }))?;
        for extent in [32.0, 48.0] {
            ui.layout(
                view,
                Size {
                    width: px(extent + 4.0),
                    height: px(extent + 4.0),
                },
            )?;
            let geometry: Vec<_> = ui
                .children(view, ui.root(view)?)?
                .map(|id| {
                    let g = ui.geometry(view, id).unwrap();
                    if row {
                        (g.origin.y.get(), g.metrics.size.height.get())
                    } else {
                        (g.origin.x.get(), g.metrics.size.width.get())
                    }
                })
                .collect();
            assert_eq!(
                geometry,
                [
                    (3.25, extent - 3.75),
                    (2.0, extent * 0.25),
                    (2.0, 0.0),
                    (extent - 1.0, 3.0),
                    (2.0 + (extent - 3.0) * 0.5, 3.0)
                ],
                "{direction:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn stretched_column_remeasures_percentage_label_before_placing_siblings() -> Result<()> {
    use solor::text;

    let font = include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf");
    let mut ui = Ui::with_font(font.to_vec(), "zh-CN")?;
    let view = ui.mount(component(|_| {
        div()
            .items_start()
            .justify_center()
            .child(
                div()
                    .self_stretch()
                    .child(text("Audio settings and output device").w(percent(50.0))),
            )
            .child(div().size(px(6.0)))
    }))?;
    let root = ui.root(view)?;
    let (group, marker) = {
        let mut items = ui.children(view, root)?;
        (items.next().unwrap(), items.next().unwrap())
    };
    let label = ui.children(view, group)?.next().unwrap();
    let mut heights = Vec::new();
    for width in [240.0, 80.0, 240.0] {
        ui.layout(
            view,
            Size {
                width: px(width),
                height: px(250.0),
            },
        )?;
        let group = ui.geometry(view, group).unwrap();
        let label = ui.geometry(view, label).unwrap();
        let marker = ui.geometry(view, marker).unwrap();
        let height = label.metrics.size.height;
        assert_eq!(label.metrics.size.width, px(width * 0.5));
        assert_eq!(
            group.metrics.size,
            Size {
                width: px(width),
                height
            }
        );
        assert_eq!(group.origin.y, (px(244.0) - height) * 0.5);
        assert_eq!(marker.origin.y, group.origin.y + height);
        heights.push(height);
    }
    assert!(heights[1] > heights[0]);
    assert_eq!(heights[0], heights[2]);
    Ok(())
}

#[test]
fn baseline_group_sizes_a_nested_row_and_follows_conditional_membership() -> Result<()> {
    use solor::{Length, style::FlexDirection::*, text};

    let font = include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf");
    for direction in [Row, RowReverse] {
        let mut ui = Ui::with_font(font.to_vec(), "zh-CN")?;
        let shown = ui.context().signal(false);
        let view = ui.mount(component(|_| {
            div()
                .flex_direction(direction)
                .items_baseline()
                .child(text("A").text_size(px(28.0)).line_height(px(36.0)))
                .child(
                    div()
                        .flex_direction(direction)
                        .items_start()
                        .child(div().size(px(2.0)))
                        .child(
                            text("B")
                                .self_baseline()
                                .text_size(px(12.0))
                                .line_height(px(18.0))
                                .mt(px(1.5))
                                .mb(px(9.25)),
                        )
                        .child_when(shown, || div().size(px(32.0)).mt(px(2.5)).self_baseline()),
                )
                .child(div().size(px(4.0)).mt(Length::Auto))
        }))?;
        let root = ui.root(view)?;
        for visible in [false, true, false] {
            ui.context().set(shown, visible)?;
            ui.flush()?;
            ui.layout(
                view,
                Size {
                    width: px(180.0),
                    height: px(80.0),
                },
            )?;
            let mut items = ui.children(view, root)?;
            let large = ui.geometry(view, items.next().unwrap()).unwrap();
            let nested_id = items.next().unwrap();
            let nested = ui.geometry(view, nested_id).unwrap();
            let automatic = ui.geometry(view, items.next().unwrap()).unwrap();
            let mut group = ui.children(view, nested_id)?;
            let leading = ui.geometry(view, group.next().unwrap()).unwrap();
            let small = ui.geometry(view, group.next().unwrap()).unwrap();
            let local = small.metrics.baseline.unwrap();
            let mut expected = px(1.5) + local;
            if visible {
                let icon = ui.geometry(view, group.next().unwrap()).unwrap();
                expected = expected.max(px(2.5) + icon.metrics.size.height);
                assert_eq!(icon.origin.y + icon.metrics.size.height, expected);
            }
            assert_eq!(leading.origin.y, px(0.0));
            assert_eq!(small.origin.y + local, expected);
            assert_eq!(nested.metrics.baseline, Some(expected));
            assert_eq!(
                nested.metrics.size.height,
                expected + (small.metrics.size.height - local + px(9.25))
            );
            let shared = nested.origin.y + expected;
            assert_eq!(shared, large.origin.y + large.metrics.baseline.unwrap());
            assert_eq!(
                ui.geometry(view, root).unwrap().metrics.baseline,
                Some(shared)
            );
            assert_eq!(automatic.origin.y, px(76.0));
        }
    }
    Ok(())
}

#[test]
fn alignment_uses_the_baseline_after_a_nested_column_justifies_its_content() -> Result<()> {
    use solor::text;

    let font = include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf");
    let mut ui = Ui::with_font(font.to_vec(), "zh-CN")?;
    let view = ui.mount(component(|_| {
        div()
            .child(
                div()
                    .flex_row()
                    .items_baseline()
                    .child(text("A").line_height(px(20.0)).pb(px(10.0)))
                    .child(
                        div()
                            .h(px(60.0))
                            .justify_end()
                            .child(text("B").line_height(px(20.0))),
                    ),
            )
            .child(div().size(px(4.0)))
    }))?;
    ui.layout(
        view,
        Size {
            width: px(180.0),
            height: px(80.0),
        },
    )?;
    let mut rows = ui.children(view, ui.root(view)?)?;
    let row = rows.next().unwrap();
    assert_eq!(
        ui.geometry(view, row).unwrap().metrics.size.height,
        px(70.0)
    );
    assert_eq!(
        ui.geometry(view, rows.next().unwrap()).unwrap().origin.y,
        px(70.0)
    );
    let mut items = ui.children(view, row)?;
    let left = ui.geometry(view, items.next().unwrap()).unwrap();
    let column = items.next().unwrap();
    let nested = ui
        .geometry(view, ui.children(view, column)?.next().unwrap())
        .unwrap();
    let column = ui.geometry(view, column).unwrap();
    assert_eq!(nested.origin.y, px(40.0));
    assert_eq!(
        left.origin.y + left.metrics.baseline.unwrap(),
        column.origin.y + nested.origin.y + nested.metrics.baseline.unwrap()
    );
    Ok(())
}
