//! Keep one settings window and close it from the main window.
use solor::{
    Context, Div, Render, Styled, Ui, component, div,
    keyboard::{Key, KeyBinding},
    px, rgb, text,
};
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

struct OpenSettings;

fn main_view(app: Application) -> impl Render {
    component(move |cx| {
        let settings_window = cx.signal(None::<WindowHandle>);
        let open = move |cx: &mut Context<'_>| {
            if let Some(window) = cx.read(settings_window)?.as_ref().filter(|w| w.is_open()) {
                window.activate();
                return Ok(());
            }
            let options = WindowOptions::new("Settings")
                .app_id("org.solor.SettingsExample")
                .size(px(280.0), px(160.0));
            let window = app.open_window(options, Ui::new(), settings())?;
            cx.set(settings_window, Some(window))
        };
        div()
            .flex_col()
            .p(px(12.0))
            .gap(px(6.0))
            .child(
                button("Open settings")
                    .focusable()
                    .on_click(open.clone())
                    .on_action(move |_: &OpenSettings, cx| open(cx)),
            )
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
        let options = WindowOptions::new("Workspace")
            .app_id("org.solor.SettingsExample")
            .size(px(280.0), px(160.0));
        let mut ui = Ui::new();
        ui.bind_keys([KeyBinding::new(Key::Enter, OpenSettings)]);
        app.open_window(options, ui, main_view(app.clone()))?;
        Ok(())
    })
}
