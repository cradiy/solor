use crate::{CITRON, INK, LINE, MUTED, label, rule};
use solor::{
    Div, IntoElement, ListScrollHandle, ScrollAlignment, SharedString, Styled, component, div,
    layout::Origin, px, rgb, scrollbar, text, uniform_list,
};
use std::{path::Path, rc::Rc};

pub(super) fn file_drop() -> impl IntoElement {
    component(|cx| {
        let paths = cx.signal(Vec::<(usize, Rc<Path>)>::new());
        let count = cx.signal(SharedString::from("0 files"));
        let prompt = cx.signal(SharedString::from("Drop files here"));
        let surface = cx.signal(rgb(0xe9ede3));
        let list = ListScrollHandle::new();
        let reset = list.clone();

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
                cx.set(
                    paths,
                    files
                        .iter()
                        .enumerate()
                        .map(|(i, path)| (i, Rc::from(path.as_path())))
                        .collect(),
                )?;
                let n = files.len();
                cx.set(
                    count,
                    format!("{n} {}", if n == 1 { "file" } else { "files" }).into(),
                )?;
                reset.scroll_to(0, ScrollAlignment::Top);
                Ok(())
            });

        div()
            .flex_1()
            .items_stretch()
            .p(px(22.))
            .gap(px(12.))
            .child(label("File drop", 33., 44.).letter_spacing(px(-1.3)))
            .child(target)
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(label("Last drop", 17., 26.))
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
