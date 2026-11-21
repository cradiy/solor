use crate::{INK, PAPER, label};
use solor::{
    Div, Fill, IntoElement, Rgba, ScrollHandle, SharedString, Signal, Styled, component, div,
    horizontal_scrollbar, px, rgb, scrollbar, text,
};

const FAMILIES: [(&str, Rgba); 9] = [
    ("Clay", rgb(0xb86145)),
    ("Amber", rgb(0xc68d35)),
    ("Olive", rgb(0x939445)),
    ("Sage", rgb(0x66886c)),
    ("Lagoon", rgb(0x388a91)),
    ("Cobalt", rgb(0x456bab)),
    ("Iris", rgb(0x7663ac)),
    ("Plum", rgb(0x965886)),
    ("Rose", rgb(0xbd6579)),
];

fn swatch(base: Rgba, step: u8) -> Rgba {
    // Step five is the family color. Each adjacent step mixes in 18% more paper or ink.
    let end = if step < 5 { PAPER } else { INK };
    let weight = u16::from(step.abs_diff(5)) * 18;
    let mix = |a, b| ((u16::from(a) * (100 - weight) + u16::from(b) * weight + 50) / 100) as u8;
    Rgba::new(
        mix(base.red, end.red),
        mix(base.green, end.green),
        mix(base.blue, end.blue),
        255,
    )
}

fn hex(color: Rgba) -> String {
    format!("#{:02X}{:02X}{:02X}", color.red, color.green, color.blue)
}

fn tile(
    family: &str,
    base: Rgba,
    step: u8,
    chip: Signal<Fill>,
    readout: Signal<SharedString>,
) -> Div {
    let color = swatch(base, step);
    let name = format!("{family} {step}00");
    let caption = SharedString::from(format!("{name}  /  {}", hex(color)));
    div()
        .w(px(124.0))
        .h(px(64.0))
        .flex_shrink(0.0)
        .items_stretch()
        .child(div().flex_1().rounded(px(6.0)).bg(color))
        .child(
            div()
                .px(px(6.0))
                .child(text(name).text_size(px(12.0)).line_height(px(19.0))),
        )
        .cursor_pointer()
        .on_click(move |cx| {
            cx.set(chip, color.into())?;
            cx.set(readout, caption.clone())
        })
}

pub(super) fn sheet() -> impl IntoElement {
    component(|cx| {
        let scroll = ScrollHandle::new();
        let (family, base) = FAMILIES[0];
        let first = swatch(base, 1);
        let chip = cx.signal(Fill::from(first));
        let readout = cx.signal(SharedString::from(format!(
            "{family} 100  /  {}",
            hex(first)
        )));
        let rows = FAMILIES.into_iter().map(|(family, base)| {
            div()
                .flex_row()
                .gap(px(4.0))
                .flex_shrink(0.0)
                .children((1..=9).map(|step| tile(family, base, step, chip, readout)))
        });
        div()
            .flex_1()
            .items_stretch()
            .gap(px(4.0))
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .h(px(32.0))
                    .child(div().size(px(20.0)).rounded(px(5.0)).bg(chip))
                    .child(text(readout).text_size(px(14.0)).whitespace_nowrap())
                    .child(div().flex_1())
                    .child(
                        label("9 families  /  81 swatches", 12.0, 18.0).text_color(crate::MUTED),
                    ),
            )
            .child(
                div()
                    .flex_row()
                    .items_stretch()
                    .flex_1()
                    .child(
                        div()
                            .flex_1()
                            .gap(px(4.0))
                            .overflow_auto()
                            .track_scroll(&scroll)
                            .children(rows),
                    )
                    .child(scrollbar(&scroll)),
            )
            .child(horizontal_scrollbar(&scroll))
    })
}
