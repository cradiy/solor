use crate::{CITRON, INK, MUTED, Selection, control, details, label, page, stage};
use solor::{
    Div, Gradient, GradientKind, Rgba, Styled, div, px, rgb,
    style::{Distribution, FontFeature},
};

pub(super) fn typography(spacing: Selection<3>) -> Div {
    let mut specimen = div().items_stretch();
    for (flag, tracking) in spacing.0.into_iter().zip([-1.5, 0.0, 3.0]) {
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
            .children([("Enabled", 1), ("Disabled", 0)].map(|(name, value)| {
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
            }));
    page(
        "Typography",
        control(
            "Letter spacing",
            spacing.buttons(["Tight", "Natural", "Wide"], false),
        ),
        specimen,
        details("Standard ligatures", comparison),
    )
}

pub(super) const STOPS: [(f32, Rgba, &str); 4] = [
    (0.0, rgb(0x393664), "#393664"),
    (0.36, rgb(0xb07cb5), "#B07CB5"),
    (0.68, rgb(0xf1a891), "#F1A891"),
    (1.0, rgb(0xf4db89), "#F4DB89"),
];

pub(super) fn paint(geometry: Selection<3>, ramp: &Gradient) -> Div {
    let mut specimen = div().items_stretch();
    for (active, kind) in geometry.0.into_iter().zip([
        GradientKind::Linear,
        GradientKind::Radial,
        GradientKind::Diamond,
    ]) {
        let fill = ramp.clone().gradient_kind(kind);
        specimen = specimen.child_when(active, move || stage(fill.clone()));
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
        control(
            "Gradient",
            geometry.buttons(["Linear", "Radial", "Diamond"], false),
        ),
        specimen,
        details("Color stops", stops),
    )
}

pub(super) fn layout(alignment: Selection<3>) -> Div {
    let mut specimen = stage(rgb(0xe3e2f1)).px(px(18.0));
    for (active, distribution) in alignment.0.into_iter().zip([
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
        control(
            "Justify",
            alignment.buttons(["Start", "Center", "Between"], false),
        ),
        specimen,
        details("Flex grow", weights),
    )
}
