use std::time::Duration;

use crate::{INK, MUTED, Selection, label};
use solor::{
    Context, Div, Length, Render, Styled, Transition,
    TransitionProperty::{
        BorderRadius, Gap, Height, MaxHeight, MaxWidth, MinHeight, MinWidth, Width,
    },
    component, div, percent, px, rgb,
};

pub(super) fn specimens() -> impl Render {
    component(|cx| {
        let timing = Transition::new(Duration::from_millis(750)).ease_out();
        div()
            .items_stretch()
            .gap(px(12.))
            .child(shape(cx, timing))
            .child(spacing(cx, timing))
            .child(limits(cx, timing))
    })
}

fn specimen(title: &'static str, controls: Div, preview: Div) -> Div {
    div()
        .flex_row()
        .items_center()
        .gap(px(16.))
        .child(
            div()
                .w(px(172.))
                .gap(px(10.))
                .child(label(title, 18., 24.).text_color(INK))
                .child(controls),
        )
        .child(
            preview
                .flex_1()
                .min_w(px(0.))
                .h(px(116.))
                .rounded(px(14.))
                .overflow_hidden(),
        )
}

fn shape(cx: &mut Context<'_>, timing: Transition) -> Div {
    let width = cx.signal(px(84.));
    let height = cx.signal(px(40.));
    let radius = cx.signal(px(6.));
    let controls =
        Selection::new(cx, 0, false).buttons_with(["Compact", "Expanded"], move |i, cx| {
            let (w, h, r) = [(84., 40., 6.), (176., 72., 36.)][i];
            cx.set(width, px(w))?;
            cx.set(height, px(h))?;
            cx.set(radius, px(r))
        });
    specimen(
        "Size & radius",
        controls,
        div()
            .items_center()
            .justify_center()
            .bg(rgb(0xe9e5f2))
            .child(
                div()
                    .w(width)
                    .h(height)
                    .rounded(radius)
                    .bg(rgb(0x66548e))
                    .outline_width(px(1.))
                    .outline_offset(px(4.))
                    .outline_color(rgb(0x9786b2))
                    .transition([Width, Height, BorderRadius], timing),
            ),
    )
}

fn spacing(cx: &mut Context<'_>, timing: Transition) -> Div {
    let gap = cx.signal(px(4.));
    let controls = Selection::new(cx, 0, false).buttons_with(["4 px", "24 px"], move |i, cx| {
        cx.set(gap, px([4., 24.][i]))
    });
    specimen(
        "Flex gap",
        controls,
        div()
            .flex_row()
            .items_center()
            .justify_center()
            .bg(rgb(0xe4ede7))
            .gap(gap)
            .transition(Gap, timing)
            .children(
                [
                    (32., 0x679c90),
                    (50., 0x448277),
                    (68., 0x285f57),
                    (50., 0x448277),
                    (32., 0x679c90),
                ]
                .map(|(height, tint)| div().w(px(18.)).h(px(height)).rounded(px(9.)).bg(rgb(tint))),
            ),
    )
}

fn limits(cx: &mut Context<'_>, timing: Transition) -> Div {
    let limit = cx.signal(Length::from(px(24.)));
    let controls = Selection::new(cx, 0, false).buttons_with(["24 px", "80%"], move |i, cx| {
        cx.set(limit, [px(24.).into(), percent(80.)][i])
    });
    let box_at_half = || div().size(percent(50.)).rounded(px(4.));
    let sample = |title, item: Div| {
        div()
            .flex_1()
            .min_w(px(0.))
            .items_stretch()
            .gap(px(6.))
            .child(label(title, 12., 18.).text_color(MUTED))
            .child(
                div()
                    .h(px(58.))
                    .bg(rgb(0xe3dac9))
                    .rounded(px(5.))
                    .items_start()
                    .justify_center()
                    .child(item),
            )
    };
    specimen(
        "Size limits",
        controls,
        div()
            .flex_row()
            .items_center()
            .px(px(14.))
            .gap(px(14.))
            .bg(rgb(0xf1ebdf))
            .child(sample(
                "Minimum (50% base)",
                box_at_half()
                    .min_w(limit)
                    .min_h(limit)
                    .bg(rgb(0x9d643e))
                    .transition([MinWidth, MinHeight], timing),
            ))
            .child(sample(
                "Maximum (50% base)",
                box_at_half()
                    .max_w(limit)
                    .max_h(limit)
                    .bg(rgb(0x67513e))
                    .transition([MaxWidth, MaxHeight], timing),
            )),
    )
}
