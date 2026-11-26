//! Open modal settings and close-confirmation dialogs from a workspace.
use solor::{
    Context, Div, IntoElement, Render, Styled, Ui, component, div,
    keyboard::{Key, KeyBinding},
    px, rgb,
};
use solor_platform::{Application, WindowHandle, WindowOptions, current_platform};

fn button(label: impl IntoElement) -> Div {
    div().p(px(8.0)).rounded_md().bg(rgb(0xd6e2da)).child(label)
}

fn settings() -> impl Render {
    component(|cx| {
        let details = cx.signal(false);
        div()
            .flex_col()
            .p(px(12.0))
            .gap(px(6.0))
            .child(button("Show details").on_click(move |cx| cx.update(details, |v| *v = !*v)))
            .child_when(details, || "Settings apply to this session.")
            .child(button("Done").window_close_button())
    })
}

struct OpenSettings;

fn confirm_close(app: &Application, parent: &WindowHandle) -> anyhow::Result<()> {
    let owner = parent.clone();
    app.open_window(
        WindowOptions::new("Close workspace?")
            .modal_for(parent)
            .size(px(280.0), px(160.0)),
        Ui::new(),
        component(move |_| {
            div()
                .flex_col()
                .p(px(12.0))
                .gap(px(6.0))
                .child("Close this workspace?")
                .child(button("Cancel").window_close_button())
                .child(button("Close workspace").on_click(move |_| {
                    owner.close();
                    Ok(())
                }))
        }),
    )?;
    Ok(())
}

fn main_view(app: Application, parent: WindowHandle) -> impl Render {
    parent.on_should_close({
        let app = app.clone();
        let parent = parent.clone();
        move || {
            if let Err(error) = confirm_close(&app, &parent) {
                eprintln!("Could not open close confirmation: {error:#}");
            }
            false
        }
    });
    component(move |_| {
        let open = move |_: &mut Context<'_>| {
            let options = WindowOptions::new("Settings")
                .modal_for(&parent)
                .app_id("org.solor.SettingsExample")
                .size(px(280.0), px(160.0));
            app.open_window(options, Ui::new(), settings())?;
            Ok(())
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
            .child(button("Close workspace…").window_close_button())
    })
}

fn main() -> anyhow::Result<()> {
    current_platform()?.run(|app| {
        let options = WindowOptions::new("Workspace")
            .app_id("org.solor.SettingsExample")
            .size(px(280.0), px(160.0));
        let mut ui = Ui::new();
        ui.bind_keys([KeyBinding::new(Key::Enter, OpenSettings)]);
        app.open_window_with(options, ui, |parent| main_view(app.clone(), parent))?;
        Ok(())
    })
}
