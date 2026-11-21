//! Compare a luminance palette across bitmap, vector and text content.
use solor::{
    Div, GradientMap, ImageFile, IntoElement, Rgba, Signal, Styled, SvgAsset, Ui, color_stop,
    component, div, gradient_left_to_right, image, px, rgb, style::ObjectFit, svg, text,
};
use solor_platform::{WindowOptions, current_platform};
use std::path::Path;

fn palette(colors: [Rgba; 4]) -> anyhow::Result<solor::Gradient> {
    Ok(gradient_left_to_right(
        colors
            .into_iter()
            .zip([0.0, 0.32, 0.72, 1.0])
            .map(|(color, at)| color_stop(color, at)),
    )?)
}

fn card(
    map: Signal<GradientMap>,
    bird: ImageFile,
    mark: SvgAsset,
    caption: impl IntoElement,
) -> Div {
    div()
        .flex_1()
        .items_stretch()
        .rounded_xl()
        .bg(rgb(0x202728))
        .p(px(16.0))
        .gradient_map(map)
        .child(
            div()
                .flex_1()
                .items_stretch()
                .rounded_lg()
                .bg(rgb(0xb5c7bf))
                .child(image(bird).flex_1().w_full().object_fit(ObjectFit::Contain)),
        )
        .child(
            div()
                .flex_row()
                .justify_between()
                .items_center()
                .pt(px(12.0))
                .child(caption)
                .child(
                    svg(mark)
                        .size(px(44.0))
                        .text_color(rgb(0xffffff))
                        .opacity(0.6),
                ),
        )
}

fn main() -> anyhow::Result<()> {
    let copper = palette([0x1c192a, 0x793d49, 0xe7925c, 0xffead0].map(rgb))?;
    let cyan = palette([0x102439, 0x15546e, 0x68c4c0, 0xe4f6cb].map(rgb))?;
    let modes = [
        ("Original", GradientMap::new(copper.clone(), 0.0)?),
        ("Copper", copper.into()),
        ("Cyanotype", cyan.into()),
    ];
    let bird = ImageFile::new(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/image/kingfisher.webp"),
    );
    let mark = SvgAsset::from_bytes(include_bytes!("svg/kingfisher.svg"))?;
    let view = component(move |cx| {
        let map = cx.signal(modes[1].1.clone());
        div()
            .p(px(20.0))
            .gap(px(14.0))
            .items_stretch()
            .bg(rgb(0xf1efe6))
            .child(card(
                map,
                bird,
                mark,
                text("Common kingfisher")
                    .text_color(rgb(0xfff6e2))
                    .text_size(px(26.0)),
            ))
            .child(
                div()
                    .flex_row()
                    .gap(px(8.0))
                    .children(modes.map(|(name, effect)| {
                        div()
                            .rounded_full()
                            .bg(rgb(0xdce2d9))
                            .px(px(14.0))
                            .py(px(7.0))
                            .cursor_pointer()
                            .child(text(name).text_color(rgb(0x1c3937)))
                            .on_click(move |cx| cx.set(map, effect.clone()))
                    })),
            )
    });
    current_platform()?.run_window(
        WindowOptions::new("Kingfisher · Gradient map").size(px(560.0), px(410.0)),
        Ui::new(),
        view,
    )
}
