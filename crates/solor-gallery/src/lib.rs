//! Interactive specimens of Solor typography, fills and layout.
mod markets;
mod specimens;

use anyhow::Result;
use solor::{
    Context, Div, Fill, IntoElement, Render, ResizeEdge, Rgba, Signal, Styled, Ui, color_stop,
    component, div, linear_gradient, px, rgb, text,
};

const PAPER: Rgba = rgb(0xf8f7f2);
const INK: Rgba = rgb(0x252e29);
const MUTED: Rgba = rgb(0x526058);
const LINE: Rgba = rgb(0xd7dbd2);
const CITRON: Rgba = rgb(0xe0efa3);
const PAGES: [&str; 4] = ["Markets", "Typography", "Paint", "Layout"];

fn label(value: &'static str, size: f32, leading: f32) -> impl IntoElement + Styled {
    text(value)
        .text_size(px(size))
        .line_height(px(leading))
        .whitespace_nowrap()
}

fn rule() -> Div {
    div().h(px(1.0)).bg(LINE)
}

#[derive(Clone, Copy)]
struct Selection<const N: usize>([Signal<bool>; N]);

impl<const N: usize> Selection<N> {
    fn new(cx: &mut Context<'_>, initial: usize) -> Self {
        Self(std::array::from_fn(|i| cx.signal(i == initial)))
    }

    fn select(self, index: usize, cx: &mut Context<'_>) -> Result<()> {
        for (i, flag) in self.0.into_iter().enumerate() {
            cx.set(flag, i == index)?;
        }
        Ok(())
    }

    fn buttons(self, names: [&'static str; N], dark: bool) -> Div {
        let (active, active_ink, idle, ink) = if dark {
            (CITRON, INK, INK, PAPER)
        } else {
            (INK, PAPER, rgb(0xedeee7), INK)
        };
        let mut bar = div();
        for (current, flag) in self.0.into_iter().enumerate() {
            bar = bar.child_when(flag, move || {
                div()
                    .flex_row()
                    .gap(px(3.0))
                    .children(names.into_iter().enumerate().map(|(i, name)| {
                        let selected = i == current;
                        div()
                            .px(px(10.0))
                            .h(px(30.0))
                            .justify_center()
                            .rounded(px(8.0))
                            .bg(if selected { active } else { idle })
                            .text_color(if selected { active_ink } else { ink })
                            .child(label(name, 13.0, 18.0))
                            .cursor_pointer()
                            .on_click(move |cx| self.select(i, cx))
                    }))
            });
        }
        bar
    }
}

fn brand() -> Div {
    let mark = div()
        .size(px(26.0))
        .rounded(px(8.0))
        .bg(CITRON)
        .items_center()
        .justify_center()
        .gap(px(2.0))
        .children(
            [8.0, 14.0, 8.0].map(|width| div().w(px(width)).h(px(2.5)).rounded(px(2.0)).bg(INK)),
        );
    div()
        .flex_row()
        .gap(px(9.0))
        .items_center()
        .child(mark)
        .child(label("Solor", 19.0, 24.0).text_color(PAPER))
}

fn header_button(title: &'static str, hint: &'static str, cx: &mut Context<'_>) -> Div {
    let background = cx.signal(Fill::from(MUTED));
    div()
        .h(px(30.0))
        .px(px(10.0))
        .justify_center()
        .rounded(px(8.0))
        .bg(background)
        .on_hover(move |inside, cx| {
            cx.set(
                background,
                Fill::from(if inside { rgb(0x687970) } else { MUTED }),
            )
        })
        .cursor_pointer()
        .tooltip(hint)
        .text_color(PAPER)
        .child(label(title, 13.0, 18.0))
}

/// Creates the gallery UI with its specimen font and its unmounted root component.
pub fn gallery() -> Result<(Ui, impl Render)> {
    let font = include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf");
    let ui = Ui::with_font(font.to_vec(), "en")?;
    let ramp = linear_gradient(
        110.0,
        specimens::STOPS.map(|(at, color, _)| color_stop(color, at)),
    )?;
    Ok((
        ui,
        component(move |cx| {
            let pages = Selection::new(cx, 1);
            let spacing = Selection::new(cx, 1);
            let geometry = Selection::new(cx, 0);
            let alignment = Selection::new(cx, 1);
            let header = div()
                .window_drag_region()
                .flex_row()
                .items_center()
                .justify_between()
                .bg(INK)
                .px(px(18.0))
                .h(px(53.0))
                .child(
                    div()
                        .flex_row()
                        .items_center()
                        .gap(px(16.0))
                        .child(brand())
                        .child(
                            header_button(
                                "Toggle frame",
                                "Show or hide the native window frame",
                                cx,
                            )
                            .window_decorations_toggle(),
                        ),
                )
                .child(
                    div()
                        .flex_row()
                        .items_center()
                        .gap(px(14.0))
                        .child(pages.buttons(PAGES, true))
                        .child(
                            header_button("Close", "Close this window", cx).window_close_button(),
                        ),
                );
            div()
                .items_stretch()
                .bg(PAPER)
                .text_color(INK)
                .child(
                    div()
                        .flex_row()
                        .items_stretch()
                        .bg(INK)
                        .h(px(5.0))
                        .child(div().w(px(14.0)).window_resize_region(ResizeEdge::TopLeft))
                        .child(div().flex_1().window_resize_region(ResizeEdge::Top))
                        .child(div().w(px(14.0)).window_resize_region(ResizeEdge::TopRight)),
                )
                .child(header)
                .child_when(pages.0[0], markets::market)
                .child_when(pages.0[1], move || specimens::typography(spacing))
                .child_when(pages.0[2], move || specimens::paint(geometry, &ramp))
                .child_when(pages.0[3], move || specimens::layout(alignment))
        }),
    ))
}

fn page(title: &'static str, control: Div, sample: Div, details: Div) -> Div {
    div()
        .items_stretch()
        .p(px(22.0))
        .gap(px(14.0))
        .child(label(title, 33.0, 44.0).letter_spacing(px(-1.3)))
        .child(control)
        .child(sample)
        .child(details)
}

fn control(caption: &'static str, buttons: Div) -> Div {
    div()
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .child(label(caption, 13.0, 20.0).text_color(MUTED))
        .child(buttons)
}

fn stage(fill: impl Into<Fill>) -> Div {
    div()
        .items_stretch()
        .justify_center()
        .h(px(208.0))
        .rounded(px(16.0))
        .bg(fill.into())
        .overflow_hidden()
}

fn details(title: &'static str, body: Div) -> Div {
    div()
        .items_stretch()
        .gap(px(8.0))
        .child(label(title, 17.0, 26.0))
        .child(rule())
        .child(body)
}
