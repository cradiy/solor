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
fn signed_letter_spacing_uses_the_view_rem_and_changes_wrapping() -> Result<()> {
    const LABEL: &str = "中fi AV";
    let mut ui = Ui::with_font(FONT.to_vec(), "en")?;
    let view = ui.mount_with_rem_size(
        component(|_| {
            div()
                .items_stretch()
                .text_size(px(16.0))
                .letter_spacing(solor::rem(0.125))
                .child(text(LABEL))
                .child(text(LABEL).letter_spacing(px(0.0)))
                .child(text(LABEL).letter_spacing(px(-0.75)))
        }),
        px(20.0),
    )?;
    let size = Size {
        width: px(200.0),
        height: px(120.0),
    };
    ui.layout(view, size)?;
    let paragraphs = |ui: &Ui| -> Result<Vec<_>> {
        Ok(ui
            .record_frame(view)?
            .draws()
            .iter()
            .map(|draw| match draw {
                Draw::Text { paragraph, .. } => paragraph.clone(),
                _ => panic!("expected a text label"),
            })
            .collect())
    };
    let wide = paragraphs(&ui)?;
    let normal = &wide[1].lines[0];
    assert!(normal.glyphs.iter().any(|g| &LABEL[g.start..g.end] == "fi"));
    for (index, spacing) in [(0, 2.5), (2, -0.75)] {
        let line = &wide[index].lines[0];
        assert_eq!(line.glyphs.len(), normal.glyphs.len());
        for (i, (glyph, base)) in line.glyphs.iter().zip(&normal.glyphs).enumerate() {
            assert_eq!(
                (glyph.id, glyph.start, glyph.end),
                (base.id, base.start, base.end)
            );
            assert!((glyph.x - base.x - i as f32 * spacing).abs() < 0.001);
        }
        assert!((line.width - normal.width - line.glyphs.len() as f32 * spacing).abs() < 0.001);
    }
    let width = px((wide[0].lines[0].width + normal.width) * 0.5);
    ui.layout(view, Size { width, ..size })?;
    let narrow = paragraphs(&ui)?;
    assert_eq!(
        narrow.iter().map(|p| p.lines.len()).collect::<Vec<_>>(),
        [2, 1, 1]
    );
    let next = ui.children(view, ui.root(view)?)?.nth(1).unwrap();
    assert_eq!(
        ui.geometry(view, next).unwrap().origin.y,
        narrow[0].content.height
    );
    Ok(())
}

#[test]
fn tracked_clamped_line_starts_at_the_same_position_as_the_full_line() -> Result<()> {
    let mut ui = Ui::with_font(FONT.to_vec(), "en")?;
    let view = ui.mount(component(|_| {
        div()
            .items_stretch()
            .letter_spacing(px(2.25))
            .child(
                text("ABC\nD")
                    .line_clamp(1.try_into().unwrap())
                    .text_ellipsis(),
            )
            .child(text("ABC"))
    }))?;
    ui.layout(
        view,
        Size {
            width: px(160.0),
            height: px(72.0),
        },
    )?;
    let frame = ui.record_frame(view)?;
    let [
        Draw::Text {
            paragraph: preview, ..
        },
        Draw::Text {
            paragraph: full, ..
        },
    ] = frame.draws()
    else {
        panic!("expected two labels");
    };
    let marked = &preview.lines[0].glyphs;
    let unmarked = &full.lines[0].glyphs;
    assert!(marked.len() > unmarked.len());
    assert_eq!(&marked[..unmarked.len()], unmarked);
    Ok(())
}

#[test]
fn inherited_font_features_control_clusters_and_wrapping() -> Result<()> {
    use solor::style::FontFeature;

    const LABEL: &str = "中fi AVAVAVAVAVAV";
    let mut ui = Ui::with_font(FONT.to_vec(), "en")?;
    let view = ui.mount(component(|_| {
        div()
            .items_stretch()
            .font_features([
                FontFeature {
                    tag: *b"liga",
                    value: 0,
                },
                FontFeature {
                    tag: *b"kern",
                    value: 0,
                },
            ])
            .child(text(LABEL))
            .child(text(LABEL).font_features([]))
    }))?;
    let size = Size {
        width: px(400.0),
        height: px(120.0),
    };
    ui.layout(view, size)?;
    let paragraphs = |ui: &Ui| -> Result<_> {
        let frame = ui.record_frame(view)?;
        let [
            Draw::Text {
                paragraph: first, ..
            },
            Draw::Text {
                paragraph: second, ..
            },
        ] = frame.draws()
        else {
            panic!("expected two labels");
        };
        Ok((first.clone(), second.clone()))
    };
    let (plain, defaults) = paragraphs(&ui)?;
    let plain_line = &plain.lines[0];
    let default_line = &defaults.lines[0];
    assert_eq!(plain_line.glyphs.len(), default_line.glyphs.len() + 1);
    assert!(
        default_line
            .glyphs
            .iter()
            .any(|g| &LABEL[g.start..g.end] == "fi")
    );
    assert!(
        plain_line
            .glyphs
            .iter()
            .any(|g| &LABEL[g.start..g.end] == "f")
    );
    assert!(plain_line.width > default_line.width);

    let between = px((plain_line.width + default_line.width) * 0.5);
    ui.layout(
        view,
        Size {
            width: between,
            ..size
        },
    )?;
    let (wrapped, single) = paragraphs(&ui)?;
    assert_eq!((wrapped.lines.len(), single.lines.len()), (2, 1));
    let children: Vec<_> = ui.children(view, ui.root(view)?)?.collect();
    assert_eq!(
        ui.geometry(view, children[1]).unwrap().origin.y,
        wrapped.content.height
    );
    Ok(())
}

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
fn inherited_color_only_repaints_consumers_and_reuses_shaped_text() -> Result<()> {
    let mut ui = Ui::with_font(FONT.to_vec(), "en")?;
    let ink = ui.context().signal(INK);
    let attached = ui.context().signal(true);
    let view = ui.mount(component(|_| {
        div()
            .text_color(ink)
            .child_when(attached, || text("Output"))
            .child(text("System").text_color(INK))
    }))?;
    let size = Size {
        width: px(120.0),
        height: px(60.0),
    };
    ui.flush()?;
    ui.layout(view, size)?;
    let label = ui.children(view, ui.root(view)?)?.next().unwrap();
    let paragraph = |ui: &Ui, expected: Rgba| -> Result<_> {
        let frame = ui.record_frame(view)?;
        let [
            Draw::Text {
                color, paragraph, ..
            },
            Draw::Text { color: fixed, .. },
        ] = frame.draws()
        else {
            panic!("expected two labels")
        };
        assert_eq!((*color, *fixed), (expected, INK));
        Ok(Rc::clone(paragraph))
    };
    let original = paragraph(&ui, INK)?;
    paragraph(&ui, INK)?;
    ui.context().set(ink, Rgba::new(100, 80, 60, 170))?;
    ui.context().set(ink, ACCENT)?;
    ui.flush()?;
    let mut dirty = Vec::new();
    ui.collect_repaints(view, &mut dirty)?;
    assert_eq!(dirty, [label]);
    assert!(Rc::ptr_eq(&original, &paragraph(&ui, ACCENT)?));
    assert_eq!(ui.layout(view, size)?, 0);

    ui.context().set(attached, false)?;
    ui.flush()?;
    ui.context().set(ink, INK)?;
    assert!(!ui.has_pending_bindings());
    ui.context().set(attached, true)?;
    ui.flush()?;
    ui.layout(view, size)?;
    paragraph(&ui, INK)?;
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
            cx.fill(cx.size, cx.text_color()?);
            Ok(())
        }
    }

    let gradient = gradient_left_to_right([
        ColorStop::new(0.0, INK),
        ColorStop::new(0.4, ACCENT),
        ColorStop::new(1.0, INK),
    ])?;
    let mut ui = Ui::with_font(FONT.to_vec(), "zh-CN")?;
    let tint = ui.context().signal(ACCENT);
    let base = StyleRefinement::default()
        .size(px(40.0))
        .px_2()
        .py(px(3.0))
        .bg(gradient.clone())
        .text_color(tint);
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
    ui.context().set(tint, INK)?;
    ui.flush()?;
    let mut dirty = Vec::new();
    ui.collect_repaints(view, &mut dirty)?;
    assert_eq!(dirty.len(), 2);
    assert!(dirty.contains(&badge) && dirty.contains(&label));
    commands.clear();
    ui.paint(view, badge, &mut commands)?;
    assert!(matches!(
        commands.last(),
        Some(Draw::Fill {
            fill: Fill::Solid(INK),
            ..
        })
    ));
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

#[test]
fn content_box_percentages_resolve_before_padding_and_child_percentages_use_the_inner_box()
-> Result<()> {
    use solor::{percent, style::BoxSizing};

    let mut ui = Ui::new();
    let card_style = StyleRefinement::default()
        .box_sizing(BoxSizing::ContentBox)
        .w(percent(50.0))
        .h(percent(25.0))
        .pl(px(2.25))
        .pr(px(4.75))
        .pt(px(1.5))
        .pb(px(3.0));
    let view = ui.mount(component(|_| {
        div()
            .p(px(1.0))
            .child(
                div()
                    .with_style(&card_style)
                    .child(div().w(percent(50.0)).h(percent(100.0))),
            )
            .child(
                div()
                    .with_style(&card_style)
                    .box_sizing(BoxSizing::BorderBox),
            )
    }))?;
    let root = ui.root(view)?;
    for (width, height) in [(180.0, 100.0), (91.0, 75.0)] {
        ui.layout(
            view,
            Size {
                width: px(width),
                height: px(height),
            },
        )?;
        let mut children = ui.children(view, root)?;
        let card = children.next().unwrap();
        let next = ui.geometry(view, children.next().unwrap()).unwrap();
        let inner = ui
            .geometry(view, ui.children(view, card)?.next().unwrap())
            .unwrap();
        let card = ui.geometry(view, card).unwrap();
        let w = px((width - 2.0) * 0.5);
        let h = px((height - 2.0) * 0.25);
        assert_eq!(
            card.metrics.size,
            Size {
                width: w + px(7.0),
                height: h + px(4.5)
            }
        );
        assert_eq!(
            inner.origin,
            Origin {
                x: px(2.25),
                y: px(1.5)
            }
        );
        assert_eq!(
            inner.metrics.size,
            Size {
                width: w * 0.5,
                height: h
            }
        );
        assert_eq!(
            next.metrics.size,
            Size {
                width: w,
                height: h
            }
        );
        assert_eq!(next.origin.y, card.origin.y + card.metrics.size.height);
    }
    Ok(())
}

#[test]
fn content_box_label_wraps_and_paints_at_the_constrained_inner_width() -> Result<()> {
    use solor::{rem, style::BoxSizing};

    let mut ui = Ui::with_font(FONT.to_vec(), "zh-CN")?;
    let view = ui.mount(component(|_| {
        div()
            .child(
                text("Audio settings and output device")
                    .box_sizing(BoxSizing::ContentBox)
                    .w(rem(6.0))
                    .p(px(4.5))
                    .bg(ACCENT)
                    .line_height(px(20.0)),
            )
            .child(div().size(px(4.0)))
    }))?;
    let root = ui.root(view)?;
    let mut line_counts = Vec::new();
    for (viewport_width, outer, inner) in [
        (160.0, 105.0, 96.0),
        (51.0, 51.0, 42.0),
        (160.0, 105.0, 96.0),
    ] {
        ui.layout(
            view,
            Size {
                width: px(viewport_width),
                height: px(300.0),
            },
        )?;
        let mut children = ui.children(view, root)?;
        let label = children.next().unwrap();
        let marker = ui.geometry(view, children.next().unwrap()).unwrap();
        let geometry = ui.geometry(view, label).unwrap();
        let mut commands = Vec::new();
        ui.paint(view, label, &mut commands)?;
        let [
            Draw::Fill {
                size: background, ..
            },
            Draw::Text {
                origin,
                size,
                paragraph,
                ..
            },
        ] = &commands[..]
        else {
            panic!("expected the label background and shaped paragraph");
        };
        let lines = paragraph.lines.len();
        assert_eq!(size.width, px(inner));
        assert_eq!(size.height, px(lines as f32 * 20.0));
        assert_eq!(
            *origin,
            Origin {
                x: px(4.5),
                y: px(4.5)
            }
        );
        assert_eq!(
            *background,
            Size {
                width: px(outer),
                height: size.height + px(9.0)
            }
        );
        assert_eq!(geometry.metrics.size, *background);
        assert_eq!(marker.origin.y, background.height);
        line_counts.push(lines);
    }
    assert!(line_counts[1] > line_counts[0]);
    assert_eq!(line_counts[0], line_counts[2]);
    Ok(())
}

#[test]
fn invisible_root_keeps_label_bindings_and_skips_local_paint_subscriptions() -> Result<()> {
    use solor::RefreshMode;

    let mut ui = Ui::with_font(FONT.to_vec(), "en")?;
    let caption = ui.context().signal(SharedString::from_static("A"));
    let fill = ui.context().signal(Fill::from(ACCENT));
    let add_label = ui.context().signal(true);
    let view = ui.mount(component(|_| {
        div()
            .invisible()
            .bg(INK)
            .refresh_mode(RefreshMode::Continuous)
            .child_when(add_label, move || {
                text(caption).visible().bg(fill).line_height(px(20.0))
            })
            .child(div().size(px(5.0)))
    }))?;
    for (value, lines) in [("A", 1.0), ("A\nB\nC", 3.0)] {
        ui.context()
            .set(caption, SharedString::from_static(value))?;
        ui.flush()?;
        ui.layout(
            view,
            Size {
                width: px(100.0),
                height: px(60.0),
            },
        )?;
        let root = ui.root(view)?;
        let mut nodes = ui.children(view, root)?;
        let label = nodes.next().unwrap();
        let marker = nodes.next().unwrap();
        assert_eq!(
            ui.geometry(view, marker).unwrap().origin.y,
            px(lines * 20.0)
        );
        let mut dirty = Vec::new();
        ui.collect_repaints(view, &mut dirty)?;
        assert!(dirty.is_empty());
        let mut draws = Vec::new();
        ui.paint(view, root, &mut draws)?;
        ui.paint(view, label, &mut draws)?;
        assert!(draws.is_empty());
        let frame = ui.record_frame(view)?;
        assert!(frame.draws().is_empty());
        assert!(!frame.requests_animation_frame());
    }
    ui.context().set(fill, Fill::from(INK))?;
    assert!(!ui.has_pending_bindings());
    Ok(())
}

#[test]
fn visibility_bindings_retire_with_branches_and_refresh_cached_text_order() -> Result<()> {
    let mut ui = Ui::with_font(FONT.to_vec(), "en")?;
    let attached = ui.context().signal(true);
    let show = ui.context().signal(true);
    let view = ui.mount(component(|_| {
        div()
            .child_when(attached, move || {
                text("A").visible_when(show).text_color(ACCENT)
            })
            .child(text("B").text_color(INK))
    }))?;
    let colors = |ui: &Ui| -> Result<Vec<_>> {
        Ok(ui
            .record_frame(view)?
            .draws()
            .iter()
            .map(|draw| match draw {
                Draw::Text { color, .. } => *color,
                _ => panic!("expected text"),
            })
            .collect())
    };
    let size = Size {
        width: px(100.0),
        height: px(100.0),
    };
    ui.flush()?;
    ui.layout(view, size)?;
    for _ in 0..3 {
        assert_eq!(colors(&ui)?, [ACCENT, INK]);
    }
    ui.context().set(show, false)?;
    ui.flush()?;
    for _ in 0..3 {
        assert_eq!(colors(&ui)?, [INK]);
    }
    ui.context().set(show, true)?;
    ui.context().set(attached, false)?;
    ui.flush()?;
    ui.layout(view, size)?;
    assert_eq!(colors(&ui)?, [INK]);
    ui.context().set(show, false)?;
    assert!(!ui.has_pending_bindings());
    ui.context().set(attached, true)?;
    ui.flush()?;
    ui.layout(view, size)?;
    assert_eq!(colors(&ui)?, [INK]);
    ui.context().set(show, true)?;
    ui.flush()?;
    assert_eq!(colors(&ui)?, [ACCENT, INK]);
    ui.unmount(view)?;
    ui.context().set(show, false)?;
    assert!(!ui.has_pending_bindings());
    Ok(())
}
