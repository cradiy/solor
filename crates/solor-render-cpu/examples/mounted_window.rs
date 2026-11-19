//! Opens a mounted settings component on a Wayland window.
#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    use solor::{
        Styled, Ui, component, div,
        layout::Size,
        platform::{Platform, WindowOptions},
        px, rgb, text,
    };
    use solor_linux::Wayland;
    use solor_render_cpu::CpuWindow;

    let font = include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf");
    let ui = Ui::with_font(font.to_vec(), "en")?;
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
    Wayland {
        verify_shm: std::env::var_os("SOLOR_VERIFY_SHM").is_some(),
        trace_submissions: std::env::var_os("SOLOR_TRACE_SUBMISSIONS").is_some(),
    }
    .run_window(
        WindowOptions {
            title: "Audio settings",
            app_id: "solor.mounted-window",
            size: Size {
                width: px(260.0),
                height: px(110.0),
            },
        },
        Box::new(CpuWindow::mount(ui, settings)?),
    )
}

#[cfg(not(target_os = "linux"))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("mounted_window requires Linux with a Wayland compositor")
}
