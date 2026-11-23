use crate::{CITRON, INK, MUTED, Selection, control, details, label, page, stage};
use solor::{
    Div, Fill, FillRule, Gradient, GradientKind, PaintContext, Rgba, Signal, Styled, div,
    layout::Origin,
    px, rgb, rgba,
    style::{Distribution, FontFeature},
};

pub(super) fn typography(spacing: Selection<3>) -> Div {
    let mut specimen = div().items_stretch();
    for (flag, tracking) in spacing.flags().into_iter().zip([-1.5, 0.0, 3.0]) {
        specimen = specimen.child_when(flag, move || {
            let latin = div()
                .flex_grow(2.0)
                .flex_basis(px(0.0))
                .items_stretch()
                .justify_center()
                .px(px(16.0))
                .overflow_hidden()
                .child(label("NOTO SANS CJK SC", 11.0, 22.0).letter_spacing(px(1.4)))
                .child(label("Aa", 82.0, 100.0).letter_spacing(px(tracking)))
                .child(div().h(px(1.0)).bg(rgb(0xb6c77c)))
                .child(label("AV 0123456789", 16.0, 29.0).letter_spacing(px(tracking)));
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
    preview: Selection<3>,
    ramp: Signal<Gradient>,
    patterns: &[Fill; 2],
) -> Div {
    let mut specimen = stage(rgb(0x262336))
        .h(px(192.0))
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
                preview.buttons(["Swatch", "Even-odd", "Nonzero"]),
            )),
        specimen,
        details("Color stops", stops),
    )
}

fn paint_contours(
    cx: &mut PaintContext<'_>,
    ramp: Signal<Gradient>,
    rule: FillRule,
) -> anyhow::Result<()> {
    let center = (cx.size.width / 2., cx.size.height / 2.);
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
            x: center.0 + radius * (x + dx),
            y: center.1 + radius * (y + dy),
        })
    };
    let contours = [hexagon(-0.56, -0.15), hexagon(0.56, 0.15)].map(std::rc::Rc::<[Origin]>::from);
    cx.fill_contours(contours.iter().cloned(), cx.read(ramp)?.clone(), rule)?;
    cx.stroke_contours(contours, px(1.5), rgba(0xfff7eaad))
}

pub(super) fn layout(alignment: Selection<3>) -> Div {
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
    let weights = div().flex_row().gap(px(6.0)).children(
        [
            (1.0, "1", 0xc8dbbe),
            (2.0, "2", 0xa5cbb0),
            (1.0, "1", 0xc8dbbe),
        ]
        .map(|(weight, name, tint)| {
            div()
                .flex_grow(weight)
                .flex_basis(px(0.0))
                .h(px(50.0))
                .rounded(px(8.0))
                .items_center()
                .justify_center()
                .bg(rgb(tint))
                .child(label(name, 19.0, 26.0).text_color(INK))
        }),
    );
    page(
        "Flex layout",
        control("Justify", alignment.buttons(["Start", "Center", "Between"])),
        specimen,
        details("Flex grow", weights),
    )
}
