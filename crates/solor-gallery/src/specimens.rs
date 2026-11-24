use crate::{CITRON, INK, MUTED, Selection, control, details, label, page, stage};
use solor::{
    Div, Fill, FillRule, Gradient, GradientKind, GradientMap, PaintContext, Rgba, Signal, Styled,
    Transition, TransitionProperty, component, div,
    keyboard::Key,
    layout::Origin,
    px, rgb, rgba,
    style::{Distribution, FontFeature},
    text,
    text::{CaretMove, TextInputState},
};

pub(super) fn typography(spacing: Selection<3>, input: Signal<TextInputState>) -> Div {
    let mut specimen = div().items_stretch();
    for (flag, tracking) in spacing.flags().into_iter().zip([-1.5, 0.0, 3.0]) {
        specimen = specimen.child_when(flag, move || {
            let field = div()
                .items_stretch()
                .px(px(12.))
                .py(px(4.))
                .border(px(1.))
                .border_color(rgb(0x879762))
                .rounded(px(8.))
                .bg(rgb(0xf9fbea))
                .overflow_hidden()
                .text_input(input)
                .selection_color(rgb(0xc5dd99))
                .cursor_text()
                .focus_indicator(crate::viewport_focus_indicator)
                .on_key_down(move |key, cx| {
                    let mods = key.modifiers;
                    if mods.alt
                        || mods.meta
                        || cx.read(input)?.preedit().is_some()
                        || (mods.control && !matches!(key.key, Key::ArrowLeft | Key::ArrowRight))
                    {
                        return Ok(());
                    }
                    match &key.key {
                        Key::Character(ch) if !ch.is_control() => {
                            cx.update_in_place(input, |state| {
                                state.commit(ch.encode_utf8(&mut [0; 4]))
                            })?;
                        }
                        Key::Text(value) if !value.chars().any(char::is_control) => {
                            cx.update_in_place(input, |state| state.commit(value))?;
                        }
                        &Key::ArrowLeft | &Key::ArrowRight | &Key::Home | &Key::End => {
                            let movement = match key.key {
                                Key::ArrowLeft if mods.control => CaretMove::PreviousWord,
                                Key::ArrowRight if mods.control => CaretMove::NextWord,
                                Key::ArrowLeft => CaretMove::Previous,
                                Key::ArrowRight => CaretMove::Next,
                                Key::Home => CaretMove::Start,
                                _ => CaretMove::End,
                            };
                            cx.update_in_place(input, |state| {
                                if mods.shift {
                                    state.extend_selection(movement);
                                } else {
                                    state.move_caret(movement);
                                }
                            })?;
                        }
                        &Key::Delete => {
                            cx.update_in_place(input, TextInputState::delete_forward)?
                        }
                        &Key::Backspace => cx.update_in_place(input, TextInputState::backspace)?,
                        _ => return Ok(()),
                    }
                    cx.prevent_default();
                    Ok(())
                })
                .child(
                    text(input)
                        .text_size(px(48.))
                        .white_space(solor::style::WhiteSpace::Pre)
                        .letter_spacing(px(tracking)),
                );
            let latin = div()
                .flex_grow(2.0)
                .flex_basis(px(0.))
                .min_w(px(0.))
                .items_stretch()
                .justify_center()
                .px(px(12.))
                .gap(px(6.))
                .child(label("Type sample", 13., 20.).text_color(MUTED))
                .child(field);
            let cjk = div()
                .flex_1()
                .items_center()
                .justify_center()
                .rounded(px(10.0))
                .bg(rgb(0x354936))
                .text_color(CITRON)
                .overflow_hidden()
                .child(label("中文", 40.0, 60.0).letter_spacing(px(tracking)))
                .child(label("CJK  /  40 px", 11.0, 20.0));
            stage(CITRON)
                .flex_row()
                .gap(px(6.0))
                .p(px(10.0))
                .child(latin)
                .child(cjk)
        });
    }
    let comparison =
        div()
            .flex_row()
            .gap(px(12.0))
            .children(
                [("Ligatures on", 1), ("Ligatures off", 0)].map(|(name, value)| {
                    div()
                        .flex_1()
                        .rounded(px(10.0))
                        .bg(rgb(0xeeeee6))
                        .px(px(14.0))
                        .py(px(8.0))
                        .child(label(name, 12.0, 20.0).text_color(MUTED))
                        .child(
                            label("office  fi  fl", 25.0, 36.0).font_features([FontFeature {
                                tag: *b"liga",
                                value,
                            }]),
                        )
                }),
            );
    let revision = div()
        .flex_1()
        .rounded(px(10.0))
        .bg(rgb(0xe6ede4))
        .px(px(14.0))
        .py(px(8.0))
        .child(label("Revision", 12.0, 20.0).text_color(MUTED))
        .child(
            div().flex_row().gap(px(9.0)).children([
                label("Draft", 17.0, 32.0).line_through().text_color(MUTED),
                label("Final", 17.0, 32.0)
                    .text_bg(CITRON)
                    .underline()
                    .decoration_color(rgb(0x507448))
                    .decoration_thickness(px(1.5)),
            ]),
        );
    page(
        "Typography",
        control(
            "Letter spacing",
            spacing.buttons(["Tight", "Natural", "Wide"]),
        ),
        specimen,
        details("Text features", comparison.child(revision)),
    )
}

pub(super) const STOPS: [(f32, Rgba, &str); 4] = [
    (0.0, rgb(0x393664), "#393664"),
    (0.36, rgb(0xb07cb5), "#B07CB5"),
    (0.68, rgb(0xf1a891), "#F1A891"),
    (1.0, rgb(0xf4db89), "#F4DB89"),
];

pub(super) fn paint(
    geometry: Selection<3>,
    overlay: Selection<3>,
    preview: Selection<7>,
    ramp: Signal<Gradient>,
    patterns: &[Fill; 2],
    map: GradientMap,
) -> Div {
    let palette = map.palette().clone();
    let mut specimen = stage(rgb(0x262336))
        .h_full()
        .child_when(preview.flags()[0], move || {
            div().absolute().inset(px(0.)).rounded(px(16.)).bg(ramp)
        });
    for (active, rule) in preview
        .flags()
        .into_iter()
        .skip(1)
        .zip([FillRule::EvenOdd, FillRule::NonZero])
    {
        specimen = specimen.child_when(active, move || {
            div()
                .absolute()
                .inset(px(0.))
                .paint(move |cx| paint_contours(cx, ramp, rule))
        });
    }
    specimen = specimen.child_when(preview.flags()[3], move || {
        div()
            .absolute()
            .inset(px(0.))
            .flex_row()
            .items_stretch()
            .p(px(12.))
            .gap(px(12.))
            .child(landscape_preview("Original", ramp, None))
            .child(landscape_preview("Mapped", ramp, Some(map.clone())))
    });
    specimen = specimen
        .child_when(preview.flags()[4], color_change)
        .child_when(preview.flags()[5], move || color_path(palette.clone()))
        .child_when(preview.flags()[6], move || frosted_panel(ramp));
    for (active, fill) in overlay
        .flags()
        .into_iter()
        .skip(1)
        .zip(patterns.iter().cloned())
    {
        specimen = specimen.child_when(active, move || {
            stage(fill.clone()).absolute().inset(px(0.0))
        });
    }
    let stops = div()
        .flex_row()
        .gap(px(8.0))
        .children(STOPS.map(|(_, color, hex)| {
            div()
                .flex_1()
                .items_stretch()
                .gap(px(5.0))
                .child(div().h(px(21.0)).rounded(px(6.0)).bg(color))
                .child(label(hex, 11.0, 18.0).text_color(MUTED))
        }));
    page(
        "Paint",
        div()
            .gap(px(6.0))
            .child(control(
                "Gradient",
                geometry.buttons_with(["Linear", "Radial", "Diamond"], move |i, cx| {
                    let kind = [
                        GradientKind::Linear,
                        GradientKind::Radial,
                        GradientKind::Diamond,
                    ][i];
                    let next = cx.read(ramp)?.clone().gradient_kind(kind);
                    cx.set(ramp, next)
                }),
            ))
            .child(control(
                "Overlay",
                overlay.buttons(["None", "Checkerboard", "Stripes"]),
            ))
            .child(control(
                "Preview",
                preview.buttons([
                    "Swatch",
                    "Even-odd",
                    "Nonzero",
                    "Gradient map",
                    "Color change",
                    "Palette",
                    "Blur",
                ]),
            )),
        opacity_preview(specimen),
        details("Color stops", stops),
    )
}

fn frosted_panel(ramp: Signal<Gradient>) -> impl solor::Render {
    component(move |cx| {
        let radius = cx.signal(px(6.));
        let levels = Selection::new(cx, 1, true)
            .buttons_with(["Clear", "6 px", "16 px"], move |i, cx| {
                cx.set(radius, px([0., 6., 16.][i]))
            });
        div()
            .absolute()
            .inset(px(0.))
            .items_center()
            .justify_center()
            .paint(move |cx| {
                let sky = cx.read(ramp)?.clone();
                landscape(cx, &sky)
            })
            .child(
                div()
                    .rounded(px(16.))
                    .p(px(14.))
                    .gap(px(8.))
                    .bg(rgba(0xffffff35))
                    .border(px(1.))
                    .border_color(rgba(0xffffff70))
                    .backdrop_blur(radius)
                    .transition(
                        TransitionProperty::BackdropBlur,
                        Transition::new(std::time::Duration::from_millis(400)),
                    )
                    .child(label("Backdrop blur", 18., 24.).text_color(crate::PAPER))
                    .child(levels),
            )
    })
}

fn color_change() -> impl solor::Render {
    component(|cx| {
        let indigo = rgb(0x393664);
        let accent = cx.signal(indigo);
        let lettering = cx.signal(indigo);
        let choices =
            Selection::new(cx, 0, true).buttons_with(["Indigo", "Peach", "Teal"], move |i, cx| {
                cx.set(accent, [indigo, rgb(0xf1a891), rgb(0x458e88)][i])?;
                cx.set(lettering, [indigo, rgb(0x934931), rgb(0x18665d)][i])
            });
        let timing = Transition::new(std::time::Duration::from_millis(650));
        div()
            .absolute()
            .inset(px(0.))
            .p(px(12.))
            .justify_end()
            .bg(accent)
            .transition(TransitionProperty::BackgroundColor, timing)
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .gap(px(12.))
                    .p(px(8.))
                    .rounded(px(8.))
                    .bg(crate::PAPER)
                    .text_color(lettering)
                    .transition(TransitionProperty::TextColor, timing)
                    .child(label("Aa", 27., 34.))
                    .child(choices)
                    .child(
                        div()
                            .size(px(16.))
                            .rounded_full()
                            .outline_width(px(3.))
                            .outline_color(accent)
                            .transition(TransitionProperty::OutlineColor, timing),
                    ),
            )
    })
}

fn color_path(palette: Gradient) -> impl solor::Render {
    component(move |cx| {
        let position = cx.signal(0.0_f32);
        let stops = Selection::new(cx, 0, true)
            .buttons_with(["Indigo", "Mauve", "Peach", "Sand"], move |i, cx| {
                cx.set(position, STOPS[i].0)
            });
        div()
            .absolute()
            .inset(px(0.))
            .p(px(12.))
            .justify_end()
            .bg(palette.color_at(position))
            .transition(
                TransitionProperty::BackgroundColor,
                Transition::new(std::time::Duration::from_millis(1000)),
            )
            .child(stops)
    })
}

fn opacity_preview(specimen: Div) -> impl solor::Render {
    component(move |cx| {
        let alpha = cx.signal(1.0_f32);
        let choices = Selection::new(cx, 1, false);
        let buttons = choices.buttons_with(["Fade out", "Fade in"], move |index, cx| {
            cx.set(alpha, if index == 0 { 0.0 } else { 1.0 })
        });
        div()
            .items_stretch()
            .gap(px(8.))
            .child(
                stage(rgb(0xe5e3df))
                    .h(px(192.))
                    .child(specimen.opacity(alpha).transition(
                        TransitionProperty::Opacity,
                        Transition::new(std::time::Duration::from_millis(900)),
                    )),
            )
            .child(control("Opacity", buttons))
    })
}

fn landscape_preview(title: &'static str, ramp: Signal<Gradient>, map: Option<GradientMap>) -> Div {
    div()
        .flex_1()
        .items_stretch()
        .gap(px(5.))
        .child(label(title, 12., 18.).text_color(crate::PAPER))
        .child(div().flex_1().paint(move |cx| {
            let sky = cx.read(ramp)?;
            match &map {
                Some(map) => cx.with_gradient_map(map, |cx| landscape(cx, sky)),
                None => landscape(cx, sky),
            }
        }))
}

fn landscape(cx: &mut PaintContext<'_>, sky: &Gradient) -> anyhow::Result<()> {
    let size = cx.size;
    let point = |x, y| Origin {
        x: size.width * x,
        y: size.height * y,
    };
    cx.fill(size, sky.clone());
    cx.fill_circle(
        point(0.68, 0.32),
        size.height.min(size.width) * 0.19,
        rgb(0xffefbe),
    )?;
    let ridge = |peaks: &[(f32, f32)]| {
        peaks
            .iter()
            .copied()
            .chain([(1., 1.), (0., 1.)])
            .map(|(x, y)| point(x, y))
            .collect::<Vec<_>>()
    };
    cx.fill_polygon(
        ridge(&[
            (0., 0.62),
            (0.22, 0.31),
            (0.40, 0.54),
            (0.54, 0.38),
            (0.78, 0.68),
            (1., 0.43),
        ]),
        rgb(0x719289),
    )?;
    cx.fill_polygon(
        ridge(&[
            (0., 0.74),
            (0.14, 0.62),
            (0.30, 0.71),
            (0.57, 0.52),
            (0.78, 0.66),
            (0.91, 0.57),
            (1., 0.62),
        ]),
        rgb(0x385f60),
    )?;
    cx.fill_polygon(
        ridge(&[
            (0., 0.72),
            (0.24, 0.85),
            (0.45, 0.89),
            (0.72, 0.78),
            (1., 0.68),
        ]),
        rgb(0x203b4b),
    )
}

fn paint_contours(
    cx: &mut PaintContext<'_>,
    ramp: Signal<Gradient>,
    rule: FillRule,
) -> anyhow::Result<()> {
    let center = Origin {
        x: cx.size.width / 2.,
        y: cx.size.height / 2.,
    };
    let radius = (cx.size.width / 3.6).min(cx.size.height / 2.5);
    let hexagon = |dx: f32, dy: f32| {
        [
            (-1., 0.),
            (-0.5, -0.866),
            (0.5, -0.866),
            (1., 0.),
            (0.5, 0.866),
            (-0.5, 0.866),
        ]
        .map(|(x, y)| Origin {
            x: radius * (x + dx),
            y: radius * (y + dy),
        })
    };
    let contours = [hexagon(-0.56, -0.15), hexagon(0.56, 0.15)].map(std::rc::Rc::<[Origin]>::from);
    cx.with_offset(center, |cx| {
        cx.fill_contours(contours.iter().cloned(), cx.read(ramp)?.clone(), rule)?;
        cx.stroke_contours(contours, px(1.5), rgba(0xfff7eaad))
    })
}

pub(super) fn layout(alignment: Selection<3>) -> impl solor::Render {
    component(move |cx| {
        let sections = Selection::new(cx, 0, false);
        let [flex, transitions] = sections.flags();
        div()
            .items_stretch()
            .p(px(22.))
            .gap(px(14.))
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(label("Layout", 33., 44.).letter_spacing(px(-1.3)))
                    .child(sections.buttons(["Flex", "Transitions"])),
            )
            .child_when(flex, move || flex_layout(alignment))
            .child_when(transitions, crate::transitions::specimens)
    })
}

fn flex_layout(alignment: Selection<3>) -> Div {
    let mut specimen = stage(rgb(0xe3e2f1)).px(px(18.0));
    for (active, distribution) in alignment.flags().into_iter().zip([
        Distribution::Start,
        Distribution::Center,
        Distribution::SpaceBetween,
    ]) {
        specimen = specimen.child_when(active, move || {
            div()
                .flex_row()
                .items_end()
                .justify_content(distribution)
                .gap(px(5.0))
                .children(
                    [
                        (48.0, 64.0, 0xc6b9ea, INK, "48"),
                        (64.0, 98.0, 0xb5a5d8, INK, "64"),
                        (80.0, 132.0, 0x655791, crate::PAPER, "80"),
                    ]
                    .map(|(width, height, tint, ink, width_label)| {
                        div()
                            .w(px(width))
                            .h(px(height))
                            .items_center()
                            .justify_end()
                            .pb(px(6.0))
                            .rounded(px(8.0))
                            .bg(rgb(tint))
                            .child(label(width_label, 14.0, 22.0).text_color(ink))
                    }),
                )
        });
    }
    div()
        .items_stretch()
        .gap(px(14.))
        .child(control(
            "Justify",
            alignment.buttons(["Start", "Center", "Between"]),
        ))
        .child(specimen)
        .child(component(flex_allocation))
}

fn flex_allocation(cx: &mut solor::Context<'_>) -> Div {
    let zero = solor::Length::from(px(0.));
    let half = solor::percent(50.);
    let middle_basis = cx.signal(zero);
    let side_basis = cx.signal(zero);
    let controls = Selection::new(cx, 0, false).buttons_with(
        ["Zero", "Middle 50%", "All 50%"],
        move |index, cx| {
            let (middle, side) = [(zero, zero), (half, zero), (half, half)][index];
            cx.set(middle_basis, middle)?;
            cx.set(side_basis, side)
        },
    );
    let weight = cx.signal(2.0_f32);
    let weights = Selection::new(cx, 1, false)
        .buttons_with(["1:1:1", "1:2:1", "1:4:1"], move |index, cx| {
            cx.set(weight, [1.0_f32, 2., 4.][index])
        });
    let timing = Transition::new(std::time::Duration::from_millis(750)).ease_out();
    let tile = |name, tint| {
        div()
            .h(px(50.))
            .rounded(px(8.))
            .items_center()
            .justify_center()
            .bg(rgb(tint))
            .child(label(name, 19., 26.).text_color(INK))
    };
    let side = |name| {
        tile(name, 0xc8dbbe)
            .flex_grow(1.)
            .flex_shrink(1.)
            .flex_basis(side_basis)
            .transition(TransitionProperty::FlexBasis, timing)
    };
    let middle = tile("Middle", 0xa5cbb0)
        .flex_grow(weight)
        .flex_shrink(weight)
        .flex_basis(middle_basis)
        .transition(
            [
                TransitionProperty::FlexBasis,
                TransitionProperty::FlexGrow,
                TransitionProperty::FlexShrink,
            ],
            timing,
        );
    details(
        "Flex allocation",
        div()
            .items_stretch()
            .gap(px(6.))
            .child(
                div()
                    .flex_row()
                    .gap(px(6.))
                    .children([side("Left"), middle, side("Right")]),
            )
            .child(
                div()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(18.))
                    .child(control("Basis", controls))
                    .child(control("Weights", weights)),
            ),
    )
}
