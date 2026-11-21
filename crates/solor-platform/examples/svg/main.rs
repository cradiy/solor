//! Compare SVG illustrations in foreground ink and their original colors.
use solor::{
    AssetSource, Div, Render, Rgba, Signal, Styled, Svg, Ui, component, div, percent, px, rgb, svg,
    text,
};
use solor_platform::{WindowOptions, current_platform};

use std::borrow::Cow;

struct Illustrations;

impl AssetSource for Illustrations {
    fn svg(&self, name: &str) -> anyhow::Result<Option<Cow<'_, [u8]>>> {
        let bytes: &[u8] = match name {
            "sea-nettle" => include_bytes!("sea-nettle.svg"),
            "kingfisher" => include_bytes!("kingfisher.svg"),
            _ => return Ok(None),
        };
        Ok(Some(Cow::Borrowed(bytes)))
    }
}

fn specimen(label: &str, art: Svg) -> Div {
    div()
        .flex_1()
        .items_stretch()
        .gap(px(8.0))
        .child(
            div()
                .h(px(254.0))
                .items_center()
                .justify_center()
                .rounded_xl()
                .bg(rgb(0xeae8df))
                .child(art.w(px(294.0)).max_w(percent(100.0))),
        )
        .child(text(label).text_size(px(14.0)))
}

fn studies(name: &str, ink: Signal<Rgba>) -> Div {
    div()
        .flex_row()
        .gap(px(12.0))
        .child(specimen("Single color", svg(name).text_color(ink)))
        .child(specimen("Original colors", svg(name).preserve_colors()))
}

fn button(label: &'static str) -> Div {
    div()
        .px(px(14.0))
        .py(px(8.0))
        .rounded_full()
        .cursor_pointer()
        .child(label)
}

fn comparison() -> impl Render {
    component(move |cx| {
        let teal = rgb(0x246b69);
        let copper = rgb(0xa65432);
        let ink = cx.signal(teal);
        let show_nettle = cx.signal(true);
        let show_bird = cx.signal(false);

        div()
            .p(px(24.0))
            .gap(px(16.0))
            .items_stretch()
            .bg(rgb(0xf9f8f3))
            .text_color(rgb(0x253431))
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(28.0))
                            .child_when(show_nettle, || "Sea nettle")
                            .child_when(show_bird, || "Kingfisher"),
                    )
                    .child(
                        div()
                            .flex_row()
                            .gap(px(8.0))
                            .child(
                                button("Switch artwork")
                                    .bg(rgb(0xeae8df))
                                    .on_click(move |cx| {
                                        let next = !*cx.read(show_nettle)?;
                                        cx.set(show_nettle, next)?;
                                        cx.set(show_bird, !next)
                                    }),
                            )
                            .child(
                                button("Change ink")
                                    .bg(rgb(0x253431))
                                    .text_color(rgb(0xffffff))
                                    .on_click(move |cx| {
                                        cx.update(ink, |color| {
                                            *color = if *color == teal { copper } else { teal };
                                        })
                                    }),
                            ),
                    ),
            )
            .child_when(show_nettle, move || studies("sea-nettle", ink))
            .child_when(show_bird, move || studies("kingfisher", ink))
    })
}

fn main() -> anyhow::Result<()> {
    let options = WindowOptions::new("Solor · SVG")
        .app_id("org.solor.SvgExample")
        .size(px(704.0), px(400.0));
    current_platform()?
        .with_assets(Illustrations)
        .run_window(options, Ui::new(), comparison())
}
