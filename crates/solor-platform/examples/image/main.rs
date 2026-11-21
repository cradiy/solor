//! Display one decoded WebP as an image and a resizable card background.
use solor::{ImageFile, Styled, Ui, component, div, image, px, rgb, style::ObjectFit, text};
use solor_platform::{WindowOptions, current_platform};
use std::path::Path;

fn main() -> anyhow::Result<()> {
    let bird = ImageFile::new(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/image/kingfisher.webp"),
    );
    let view = component(move |cx| {
        let fit = cx.signal(ObjectFit::Cover);
        div()
            .p(px(24.0))
            .gap(px(16.0))
            .items_stretch()
            .bg(rgb(0xf3f1e9))
            .text_color(rgb(0x1b3937))
            .child(
                div()
                    .flex_row()
                    .gap(px(12.0))
                    .items_center()
                    .child(image(bird.clone()).size(px(60.0)))
                    .child(text("Kingfisher").text_size(px(30.0))),
            )
            .child(
                div()
                    .flex_1()
                    .items_stretch()
                    .rounded_xl()
                    .bg(rgb(0x183735))
                    .child(div().flex_1().rounded_xl().bg(bird).object_fit(fit)),
            )
            .child({
                let mut controls = div().flex_row().gap(px(8.0));
                for (label, mode) in [("Cover", ObjectFit::Cover), ("Contain", ObjectFit::Contain)]
                {
                    controls = controls.child(
                        div()
                            .px(px(16.0))
                            .py(px(8.0))
                            .rounded_full()
                            .bg(rgb(0xdce3d7))
                            .cursor_pointer()
                            .child(label)
                            .on_click(move |cx| cx.set(fit, mode)),
                    );
                }
                controls
            })
    });
    let window = WindowOptions::new("Solor · Image")
        .app_id("org.solor.ImageExample")
        .size(px(640.0), px(440.0));
    current_platform()?.run_window(window, Ui::new(), view)
}
