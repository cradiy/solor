//! Open additional notes from a running window.
use solor::{
    Context, Div, Render, Styled, Ui, component, div,
    keyboard::{Key, KeyBinding},
    px, rgb, text,
};
use solor_platform::{Application, WindowOptions, current_platform};

struct Activate;

fn button(
    label: &'static str,
    activate: impl Fn(&mut Context<'_>) -> anyhow::Result<()> + Clone + 'static,
) -> Div {
    div()
        .p(px(8.0))
        .bg(rgb(0xd6e2da))
        .child(text(label))
        .focusable()
        .on_click(activate.clone())
        .on_action(move |_: &Activate, cx| activate(cx))
}

fn sheet(name: String, app: Application) -> impl Render {
    component(move |cx| {
        let marked = cx.signal(false);
        let next = cx.signal(1_u64);
        let toggle = move |cx: &mut Context<'_>| cx.update(marked, |value| *value = !*value);
        div()
            .flex_col()
            .p(px(16.0))
            .gap(px(8.0))
            .bg(rgb(0xf8f7f2))
            .child(text(name))
            .child(button("Toggle bookmark", toggle))
            .child(button("Open another note", move |cx| {
                let number = *cx.read(next)?;
                open_note(&app, format!("Note {number}"))?;
                cx.set(next, number + 1)
            }))
            .child_when(marked, || text("Bookmarked"))
    })
}

fn open_note(app: &Application, name: String) -> anyhow::Result<()> {
    let options = WindowOptions::new(&name)
        .app_id("solor.notes")
        .size(px(300.0), px(220.0));
    let mut ui = Ui::new();
    ui.bind_keys([KeyBinding::new(Key::Enter, Activate)]);
    app.open_window(options, ui, sheet(name.clone(), app.clone()))
}

fn main() -> anyhow::Result<()> {
    current_platform()?.run(|app| open_note(&app, "Field notes".into()))
}
