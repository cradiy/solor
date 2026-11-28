use crate::{CITRON, INK, LINE, MUTED, activation_button, label, rule};
use solor::{
    Div, IntoElement, ListScrollHandle, ScrollAlignment, SharedString, Styled, component, div,
    layout::Origin, px, rgb, scrollbar, text, uniform_list,
};
use solor_platform::{FilePickerOptions, pick_paths};
use std::{
    path::{Path, PathBuf},
    rc::Rc,
};

pub(super) fn file_drop() -> impl IntoElement {
    component(|cx| {
        let paths = cx.signal(Vec::<(usize, Rc<Path>)>::new());
        let count = cx.signal(SharedString::from("0 files"));
        let prompt = cx.signal(SharedString::from("Drop files here"));
        let surface = cx.signal(rgb(0xe9ede3));
        let list = ListScrollHandle::new();
        let reset = list.clone();
        let picker_list = list.clone();
        let choosing = cx.signal(false);
        let choose = activation_button(
            div()
                .px(px(12.))
                .py(px(6.))
                .rounded_md()
                .bg(CITRON)
                .child(label("Choose images", 14., 20.)),
            None,
            move |cx| {
                if *cx.read(choosing)? {
                    return Ok(());
                }
                cx.set(choosing, true)?;
                let list = picker_list.clone();
                cx.spawn(move |cx| async move {
                    let options = FilePickerOptions::files()
                        .title("Select images")
                        .accept_label("Select")
                        .filter("Images", ["png", "jpg", "jpeg", "webp"]);
                    let result = pick_paths(options).await;
                    cx.set(choosing, false)?;
                    match result {
                        Ok(files) if !files.is_empty() => {
                            cx.set(paths, file_rows(&files))?;
                            cx.set(count, file_count(files.len()))?;
                            list.scroll_to(0, ScrollAlignment::Top);
                        }
                        Err(error) => {
                            log::warn!("Could not open file picker: {error:#}");
                            cx.set(count, "File picker unavailable".into())?;
                        }
                        _ => {}
                    }
                    Ok(())
                });
                Ok(())
            },
        );

        let target = div()
            .h(px(154.))
            .items_center()
            .justify_center()
            .gap(px(10.))
            .rounded(px(16.))
            .bg(surface)
            .child(tray_icon())
            .child(text(prompt).text_size(px(20.)).line_height(px(28.)))
            .on_file_drop_hover(move |over, cx| {
                cx.set(surface, if over { CITRON } else { rgb(0xe9ede3) })?;
                cx.set(
                    prompt,
                    if over {
                        "Release to drop"
                    } else {
                        "Drop files here"
                    }
                    .into(),
                )
            })
            .on_file_drop(move |files, cx| {
                cx.set(paths, file_rows(files))?;
                cx.set(count, file_count(files.len()))?;
                reset.scroll_to(0, ScrollAlignment::Top);
                Ok(())
            });

        div()
            .flex_1()
            .items_stretch()
            .p(px(22.))
            .gap(px(12.))
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(label("Files", 33., 44.).letter_spacing(px(-1.3)))
                    .child(choose),
            )
            .child(target)
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(label("Selected files", 17., 26.))
                    .child(text(count).text_size(px(13.)).text_color(MUTED)),
            )
            .child(rule())
            .child(
                div()
                    .flex_1()
                    .flex_row()
                    .items_stretch()
                    .child(
                        uniform_list(paths, px(52.), |(_, path)| file_row(&path))
                            .scroll_handle(&list)
                            .flex_1(),
                    )
                    .child(scrollbar(&list).ml(px(-12.))),
            )
    })
}

fn file_rows(files: &[PathBuf]) -> Vec<(usize, Rc<Path>)> {
    files
        .iter()
        .enumerate()
        .map(|(i, path)| (i, Rc::from(path.as_path())))
        .collect()
}

fn file_count(n: usize) -> SharedString {
    format!("{n} {}", if n == 1 { "file" } else { "files" }).into()
}

fn file_row(path: &Path) -> Div {
    let name = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy();
    let directory = path.parent().unwrap_or(Path::new("")).to_string_lossy();
    div()
        .items_stretch()
        .justify_center()
        .pr(px(14.))
        .border_b(px(1.))
        .border_color(LINE)
        .child(
            text(name.into_owned())
                .text_size(px(15.))
                .line_height(px(21.))
                .truncate(),
        )
        .child(
            text(directory.into_owned())
                .text_size(px(12.))
                .line_height(px(17.))
                .text_color(MUTED)
                .truncate()
                .text_ellipsis_middle(),
        )
}

fn tray_icon() -> Div {
    div().size(px(30.)).paint(|cx| {
        let point = |x, y| Origin { x: px(x), y: px(y) };
        cx.stroke_polyline([point(15., 3.), point(15., 17.)], px(2.), INK)?;
        cx.stroke_polyline(
            [point(9., 12.), point(15., 18.), point(21., 12.)],
            px(2.),
            INK,
        )?;
        cx.stroke_polyline(
            [
                point(4., 19.),
                point(4., 25.),
                point(26., 25.),
                point(26., 19.),
            ],
            px(2.),
            INK,
        )
    })
}
