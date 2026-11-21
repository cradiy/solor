//! Keep one settings window and close it from the main window.
use solor::{Div, Render, Styled, Ui, component, div, px, rgb, text};
use solor_platform::{Application, WindowHandle, WindowOptions, current_platform};

fn button(label: &'static str) -> Div {
    div()
        .p(px(8.0))
        .rounded_md()
        .bg(rgb(0xd6e2da))
        .child(text(label))
}

fn settings() -> impl Render {
    component(|cx| {
        let details = cx.signal(false);
        div()
            .flex_col()
            .p(px(12.0))
            .gap(px(6.0))
            .child(button("Show details").on_click(move |cx| cx.update(details, |v| *v = !*v)))
            .child_when(details, || text("Settings apply to this session."))
            .child(button("Done").window_close_button())
    })
}

fn main_view(app: Application) -> impl Render {
    component(move |cx| {
        let settings_window = cx.signal(None::<WindowHandle>);
        div()
            .flex_col()
            .p(px(12.0))
            .gap(px(6.0))
            .child(button("Open settings").on_click(move |cx| {
                if cx
                    .read(settings_window)?
                    .as_ref()
                    .is_some_and(WindowHandle::is_open)
                {
                    return Ok(());
                }
                let options = WindowOptions::new("Settings").size(px(280.0), px(160.0));
                let window = app.open_window(options, Ui::new(), settings())?;
                cx.set(settings_window, Some(window))
            }))
            .child(button("Close settings").on_click(move |cx| {
                if let Some(window) = cx.read(settings_window)? {
                    window.close();
                }
                Ok(())
            }))
    })
}

fn main() -> anyhow::Result<()> {
    current_platform()?.run(|app| {
        let options = WindowOptions::new("Workspace").size(px(280.0), px(160.0));
        app.open_window(options, Ui::new(), main_view(app.clone()))?;
        Ok(())
    })
}
