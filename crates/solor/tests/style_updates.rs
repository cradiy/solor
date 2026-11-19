//! Style resolution and retained update behavior.
use anyhow::Result;
use solor::{px, style::Corners};
use std::rc::Rc;

use solor::{
    ColorStop, Draw, Fill, GradientError, Rgba, SharedString, StyleRefinement, Styled, Ui,
    component, div,
    layout::{Origin, Size},
    linear_gradient, text,
};

const FONT: &[u8] = include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf");
const INK: Rgba = Rgba::new(30, 40, 50, 255);
const ACCENT: Rgba = Rgba::new(25, 100, 200, 255);

#[test]
fn conditional_label_inherits_style_and_keeps_identity_when_it_wraps() -> Result<()> {
    let mut ui = Ui::with_font(FONT.to_vec(), "zh-CN")?;
    let name = ui.context().signal(SharedString::from_static("A"));
    let open = ui.context().signal(false);
    let card = StyleRefinement::default()
        .padding(px(4.0))
        .padding_bottom(px(7.0))
        .gap(px(2.0))
        .text_color(INK)
        .text_size(px(17.0))
        .line_height(px(28.0))
        .rounded(px(100.0))
        .bg(ACCENT);
    let view = ui.mount(component(|_| {
        div()
            .flex_col()
            .with_style(&card)
            .padding_left(px(11.0))
            .child(text("Title").text_color(ACCENT))
            .child_when(open, move || text(name))
    }))?;
    let root = ui.root(view)?;
    let size = Size {
        width: px(110.0),
        height: px(36.0),
    };
    ui.flush()?;
    ui.layout(view, size)?;
    let title = ui.children(view, root)?.next().unwrap();
    let mut commands = Vec::new();
    ui.paint(view, root, &mut commands)?;
    ui.paint(view, title, &mut commands)?;
    assert!(
        matches!(&commands[..], [Draw::Fill{fill: Fill::Solid(c), corner_radii, ..}, Draw::Text{color, ..}] if *corner_radii == Corners::all(px(18.0)) && *c == ACCENT && *color == ACCENT)
    );

    ui.context().set(open, true)?;
    ui.flush()?;
    ui.layout(view, size)?;
    let label = ui.children(view, root)?.nth(1).unwrap();
    let before = ui.geometry(view, label).unwrap();
    assert_eq!(
        ui.geometry(view, title).unwrap().origin,
        Origin {
            x: px(11.0),
            y: px(4.0)
        }
    );
    assert_eq!(
        before.origin.y,
        px(4.0) + ui.geometry(view, title).unwrap().metrics.size.height + px(2.0)
    );
    commands.clear();
    ui.paint(view, label, &mut commands)?;
    let [
        Draw::Text {
            paragraph, color, ..
        },
    ] = &commands[..]
    else {
        panic!("expected label")
    };
    assert_eq!(
        (*color, paragraph.style.size, paragraph.style.line_height),
        (INK, px(17.0), px(28.0))
    );
    let old = Rc::clone(paragraph);

    let mut ready = Vec::new();
    ui.context().set(name, "Intermediate".into())?;
    ui.context().set(name, "A".into())?;
    ui.flush()?;
    ui.collect_repaints(view, &mut ready)?;
    assert!(ready.is_empty());
    assert_eq!(ui.layout(view, size)?, 0);
    ui.context()
        .set(name, "Long output device name wraps on this row".into())?;
    ui.flush()?;
    ui.collect_repaints(view, &mut ready)?;
    assert_eq!(ready, [label]);
    assert!(ui.paint(view, label, &mut commands).is_err());
    ui.layout(view, size)?;
    assert!(ui.geometry(view, label).unwrap().metrics.size.height > before.metrics.size.height);
    assert_eq!(
        ui.geometry(view, root).unwrap().content.height,
        before.origin.y + ui.geometry(view, label).unwrap().metrics.size.height + px(7.0)
    );
    commands.clear();
    ui.paint(view, label, &mut commands)?;
    assert_eq!(old.text, "A");
    assert!(
        matches!(&commands[..], [Draw::Text{paragraph, ..}] if paragraph.text == "Long output device name wraps on this row")
    );
    ready.clear();
    ui.collect_repaints(view, &mut ready)?;
    assert!(ready.is_empty());

    ui.context().set(name, "x".repeat(4097).into())?;
    assert!(ui.flush().is_err());
    ui.context().set(open, false)?;
    ui.flush()?;
    assert!(ui.geometry(view, label).is_none());
    ui.collect_repaints(view, &mut ready)?;
    assert!(ready.is_empty());
    Ok(())
}

#[test]
fn gradient_updates_share_all_stops_and_only_queue_the_live_background() -> Result<()> {
    let gradient = linear_gradient(
        90.0,
        (0..4097).map(|i| ColorStop::new(i as f32 / 4096.0, Rgba::new((i % 251) as u8, 2, 3, 255))),
    )?;
    let mut ui = Ui::new();
    let background = ui.context().signal(Fill::from(ACCENT));
    let open = ui.context().signal(true);
    let view = ui.mount(component(|_| {
        div()
            .flex_col()
            .child_when(open, move || div().bg(background).padding(px(4.0)))
            .child(div().padding(px(8.0)))
    }))?;
    ui.flush()?;
    let size = Size {
        width: px(160.0),
        height: px(80.0),
    };
    ui.layout(view, size)?;
    let root = ui.root(view)?;
    let node = ui.children(view, root)?.next().unwrap();
    let mut draws = Vec::new();
    ui.paint(view, node, &mut draws)?;
    for i in 0..999 {
        ui.context()
            .set(background, Rgba::new(i as u8, 0, 0, 255).into())?;
    }
    ui.context().set(background, gradient.clone().into())?;
    ui.flush()?;
    let mut ready = Vec::new();
    ui.collect_repaints(view, &mut ready)?;
    assert_eq!(ready, [node]);
    assert!(ui.geometry(view, node).is_some());
    assert_eq!(ui.layout(view, size)?, 0);
    ui.paint(view, node, &mut draws)?;
    let Draw::Fill {
        fill: Fill::Gradient(held),
        ..
    } = &draws[1]
    else {
        panic!("expected linear fill")
    };
    assert_eq!(held.stops().as_ptr(), gradient.stops().as_ptr());
    assert_eq!(held.stops().len(), 4097);
    ui.context().set(background, INK.into())?;
    ui.context().set(open, false)?;
    ui.flush()?;
    ready.clear();
    ui.collect_repaints(view, &mut ready)?;
    assert!(ready.is_empty());
    assert_eq!(held.sample(1.0)?, gradient.stops()[4096].color);
    Ok(())
}

#[test]
fn gradient_edges_validation_and_premultiplied_interpolation() -> Result<()> {
    let clear = Rgba::new(255, 0, 0, 0);
    let blue = Rgba::new(0, 0, 255, 255);
    let stops = [
        ColorStop::new(0.25, clear),
        ColorStop::new(0.5, blue),
        ColorStop::new(0.5, ACCENT),
        ColorStop::new(0.75, INK),
    ];
    let fill = linear_gradient(-90.0, stops)?;
    assert_eq!(fill.angle_degrees(), 270.0);
    assert_eq!(fill.sample(0.0)?, clear);
    assert_eq!(fill.sample(0.375)?, Rgba::new(0, 0, 255, 128));
    assert_eq!(fill.sample(0.5)?, ACCENT);
    assert_eq!(fill.sample(1.0)?, INK);
    assert_eq!(
        linear_gradient(0.0, [stops[3], stops[0]]),
        Err(GradientError::UnorderedStops)
    );
    assert_eq!(linear_gradient(0.0, []), Err(GradientError::TooFewStops));
    assert_eq!(
        linear_gradient(0.0, [stops[0], ColorStop::new(f32::NAN, blue)]),
        Err(GradientError::InvalidOffset)
    );
    Ok(())
}

#[test]
fn custom_content_gets_the_shared_box_and_gradient_without_style_hooks() -> Result<()> {
    use solor::{
        Element, PaintContext, element, gradient_left_to_right,
        layout::{self, Column},
    };
    struct Badge;
    impl layout::LayoutNode for Badge {
        fn measure(
            &mut self,
            cx: &mut layout::MeasureContext<'_, '_>,
            query: layout::Query,
        ) -> layout::Result<layout::Metrics> {
            Column::new(px(0.0), px(0.0)).measure(cx, query)
        }
        fn layout(
            &mut self,
            cx: &mut layout::LayoutContext<'_, '_>,
            size: Size,
        ) -> layout::Result<layout::LayoutOutput> {
            Column::new(px(0.0), px(0.0)).layout(cx, size)
        }
    }
    impl Element for Badge {
        fn paint(&self, cx: &mut PaintContext) -> Result<()> {
            cx.fill(cx.size, cx.text_color);
            Ok(())
        }
    }

    let gradient = gradient_left_to_right([
        ColorStop::new(0.0, INK),
        ColorStop::new(0.4, ACCENT),
        ColorStop::new(1.0, INK),
    ])?;
    let base = StyleRefinement::default()
        .size(px(40.0))
        .px_2()
        .py(px(3.0))
        .bg(gradient.clone())
        .text_color(ACCENT);
    let mut ui = Ui::with_font(FONT.to_vec(), "zh-CN")?;
    let view = ui.mount(component(|_| {
        div().child(
            element(Badge).with_style(&base).pl(px(10.0)).child(
                text("A")
                    .with_style(&base)
                    .w_auto()
                    .h_auto()
                    .p_0()
                    .clear_bg(),
            ),
        )
    }))?;
    ui.flush()?;
    ui.layout(
        view,
        Size {
            width: px(80.0),
            height: px(60.0),
        },
    )?;
    let badge = ui.children(view, ui.root(view)?)?.next().unwrap();
    let label = ui.children(view, badge)?.next().unwrap();
    let inset = Origin {
        x: px(10.0),
        y: px(3.0),
    };
    assert_eq!(ui.geometry(view, label).unwrap().origin, inset);
    let mut commands = Vec::new();
    ui.paint(view, badge, &mut commands)?;
    ui.paint(view, label, &mut commands)?;
    assert!(matches!(&commands[..], [
        Draw::Fill {size: outer_size, origin, fill: Fill::Gradient(g), ..},
        Draw::Fill {size: inner_size, origin: inner, fill: Fill::Solid(c), ..},
        Draw::Text {color, ..},
    ] if *outer_size == (Size {width:px(40.0),height:px(40.0)}) && *inner_size == (Size {width:px(22.0),height:px(34.0)}) && *origin == Origin::default() && *inner == inset && *color == ACCENT && *c == ACCENT && g == &gradient));
    Ok(())
}

#[test]
fn fractional_style_lengths_survive_cascade_text_shaping_and_paint() -> Result<()> {
    let declaration = StyleRefinement::default()
        .w(px(37.5))
        .h(px(24.75))
        .pl(px(1.25))
        .rounded(px(3.5))
        .text_size(px(14.5))
        .line_height(px(20.25))
        .bg(INK);
    let mut ui = Ui::with_font(FONT.to_vec(), "zh-CN")?;
    let view = ui.mount(component(|_| {
        div().child(text("A").with_style(&declaration))
    }))?;
    ui.flush()?;
    ui.layout(
        view,
        Size {
            width: px(100.0),
            height: px(60.0),
        },
    )?;
    let label = ui.children(view, ui.root(view)?)?.next().unwrap();
    let mut commands = vec![];
    ui.paint(view, label, &mut commands)?;
    let [
        Draw::Fill {
            size, corner_radii, ..
        },
        Draw::Text {
            origin, paragraph, ..
        },
    ] = &commands[..]
    else {
        panic!("expected a background followed by shaped text");
    };
    assert_eq!(
        *size,
        Size {
            width: px(37.5),
            height: px(24.75)
        }
    );
    assert_eq!(origin.x, px(1.25));
    assert_eq!(*corner_radii, Corners::all(px(3.5)));
    assert_eq!(paragraph.style.size, px(14.5));
    assert_eq!(paragraph.lines[0].height, 20.25);
    for invalid in [-0.25, f32::NAN, f32::INFINITY] {
        assert!(ui.mount(component(move |_| div().w(px(invalid)))).is_err());
    }
    Ok(())
}

#[test]
fn percent_sizes_follow_the_parent_content_box_and_rem_uses_the_view_basis() -> Result<()> {
    use solor::{percent, rem};
    let mut ui = Ui::with_font(FONT.to_vec(), "zh-CN")?;
    let open = ui.context().signal(false);
    let make = || {
        component(move |_| {
            div().padding(px(0.5)).child(
                div()
                    .w(percent(50.0))
                    .h(percent(25.0))
                    .p(rem(0.125))
                    .gap(px(0.25))
                    .text_size(px(32.0))
                    .child(div().size(px(0.5)))
                    .child_when(open, || {
                        text("A").text_size(rem(0.75)).line_height(rem(1.25))
                    }),
            )
        })
    };
    let normal = ui.mount(make())?;
    let large = ui.mount_with_rem_size(make(), px(20.0))?;
    ui.flush()?;
    ui.context().set(open, true)?;
    ui.flush()?;
    for (view, inset, font, line) in [(normal, 2.0, 12.0, 20.0), (large, 2.5, 15.0, 25.0)] {
        for height in [101.5, 125.5, 101.5] {
            let viewport = Size {
                width: px(202.5),
                height: px(height),
            };
            ui.layout(view, viewport)?;
            let card = ui.children(view, ui.root(view)?)?.next().unwrap();
            let geometry = ui.geometry(view, card).unwrap();
            assert_eq!(
                geometry.origin,
                Origin {
                    x: px(0.5),
                    y: px(0.5)
                }
            );
            assert_eq!(
                geometry.metrics.size,
                Size {
                    width: px(100.75),
                    height: px((height - 1.0) * 0.25),
                }
            );
            let label = ui.children(view, card)?.nth(1).unwrap();
            assert_eq!(
                ui.geometry(view, label).unwrap().origin,
                Origin {
                    x: px(inset),
                    y: px(inset + 0.75)
                }
            );
            let mut commands = Vec::new();
            ui.paint(view, label, &mut commands)?;
            assert!(matches!(&commands[..], [Draw::Text { paragraph, .. }]
                if paragraph.style.size == px(font) && paragraph.lines[0].height == line));
        }
    }
    Ok(())
}
