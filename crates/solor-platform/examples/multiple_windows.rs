//! Two independently editable windows in one application.
use solor::{
    Context, Render, Styled, Ui, component, div,
    keyboard::{Key, KeyBinding},
    px, rgb, text,
};
use solor_platform::{WindowOptions, current_platform};

struct ToggleBookmark;

fn sheet(name: &'static str) -> impl Render {
    component(move |cx| {
        let marked = cx.signal(false);
        let toggle = move |cx: &mut Context<'_>| cx.update(marked, |value| *value = !*value);
        div()
            .flex_col()
            .p(px(16.0))
            .gap(px(8.0))
            .bg(rgb(0xf8f7f2))
            .child(text(name))
            .child(
                div()
                    .p(px(8.0))
                    .bg(rgb(0xd6e2da))
                    .child(text("Toggle bookmark"))
                    .focusable()
                    .on_click(toggle)
                    .on_action(move |_: &ToggleBookmark, cx| toggle(cx)),
            )
            .child_when(marked, || text("Bookmarked"))
    })
}

fn main() -> anyhow::Result<()> {
    current_platform()?.run(|app| {
        for name in ["Field notes", "Reading notes"] {
            let options = WindowOptions::new(name)
                .app_id("solor.notes")
                .size(px(300.0), px(180.0));
            let mut ui = Ui::new();
            ui.bind_keys([KeyBinding::new(Key::Enter, ToggleBookmark)]);
            app.open_window(options, ui, sheet(name))?;
        }
        Ok(())
    })
}
