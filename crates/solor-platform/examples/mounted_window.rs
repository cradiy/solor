//! Opens a mounted settings component on the current platform.
fn main() -> anyhow::Result<()> {
    use solor::{Styled, Ui, component, div, px, rgb, text};
    use solor_platform::{WindowOptions, current_platform};

    let platform = current_platform()?;
    let window = WindowOptions::new("Audio settings")
        .app_id("solor.mounted-window")
        .size(px(260.0), px(110.0));

    let ui = Ui::new();
    let settings = component(|cx| {
        let open = cx.signal(false);
        div()
            .flex_col()
            .p(px(12.0))
            .gap(px(6.0))
            .bg(rgb(0xf5f5f5))
            .child(
                div()
                    .w(px(200.0))
                    .h(px(32.0))
                    .p(px(4.0))
                    .bg(rgb(0x315f88))
                    .text_color(rgb(0xffffff))
                    .child(text("Audio settings"))
                    .on_click(move |cx| cx.update(open, |open| *open = !*open)),
            )
            .child_when(open, move || {
                div()
                    .w(px(200.0))
                    .h(px(32.0))
                    .p(px(4.0))
                    .bg(rgb(0xd5e7f6))
                    .child(text("Close settings"))
                    .on_click(move |cx| cx.set(open, false))
            })
    });
    platform.run_window(window, ui, settings)
}
