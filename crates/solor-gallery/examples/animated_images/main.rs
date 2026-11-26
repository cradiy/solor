//! A looping GIF and a WebP sunrise that settles on its final frame.
use anyhow::Result;
use solor::{Div, ImageSource, Styled, Ui, component, div, image, px, rgb, text};
use solor_platform::{WindowOptions, current_platform};

fn image_card(title: &'static str, caption: &'static str, source: impl Into<ImageSource>) -> Div {
    div()
        .flex_1()
        .min_w(px(0.))
        .gap(px(6.))
        .child(image(source).w_full().rounded_lg())
        .child(text(title).text_size(px(21.)))
        .child(text(caption).text_size(px(14.)).text_color(rgb(0x586768)))
}

fn main() -> Result<()> {
    let view = component(|_| {
        div()
            .justify_center()
            .items_stretch()
            .p(px(24.))
            .gap(px(16.))
            .bg(rgb(0xf4f1e8))
            .text_color(rgb(0x192f34))
            .child(text("Coastal lights").text_size(px(30.)))
            .child(
                div()
                    .flex_row()
                    .items_start()
                    .gap(px(16.))
                    .child(image_card(
                        "Night watch",
                        "GIF · continuous loop",
                        include_bytes!("beacon.gif"),
                    ))
                    .child(image_card(
                        "First light",
                        "WebP · plays once",
                        include_bytes!("dawn.webp"),
                    )),
            )
    });
    let window = WindowOptions::new("Solor · Animated images")
        .app_id("org.solor.AnimatedImages")
        .size(px(688.), px(376.));
    current_platform()?.run_window(window, Ui::new(), view)
}
