//! Interactive specimens of Solor typography, fills and layout.
mod lists;
mod markets;
mod specimens;
mod tone_sheet;
mod transitions;

use anyhow::Result;
use solor::{
    Context, Div, Fill, IntoElement, Render, ResizeEdge, Rgba, Signal, Styled, TooltipStyle, Ui,
    checkerboard, color_stop, component, diagonal_stripes, div,
    keyboard::{Key, KeyBinding},
    layout::{Origin, Size},
    linear_gradient, px, rgb, rgba, text,
};

const PAPER: Rgba = rgb(0xf8f7f2);
const INK: Rgba = rgb(0x252e29);
const MUTED: Rgba = rgb(0x526058);
const LINE: Rgba = rgb(0xd7dbd2);
const CITRON: Rgba = rgb(0xe0efa3);
const PAGES: [&str; 5] = ["Lists", "Markets", "Typography", "Paint", "Layout"];

fn label(value: &'static str, size: f32, leading: f32) -> impl IntoElement + Styled {
    text(value)
        .text_size(px(size))
        .line_height(px(leading))
        .whitespace_nowrap()
}

fn viewport_focus_indicator(paint: &mut solor::PaintContext<'_>) -> anyhow::Result<()> {
    paint.fill_rect(
        Origin::default(),
        Size {
            width: paint.size.width,
            height: px(2.0),
        },
        MUTED,
    )
}

fn rule() -> Div {
    div().h(px(1.0)).bg(LINE)
}

#[derive(Clone, Copy)]
struct Choice {
    selected: Signal<bool>,
    background: Signal<Rgba>,
    ink: Signal<Rgba>,
}

#[derive(Clone, Copy)]
struct Selection<const N: usize> {
    choices: [Choice; N],
    colors: [(Rgba, Rgba); 2],
}

impl<const N: usize> Selection<N> {
    fn new(cx: &mut Context<'_>, initial: usize, dark: bool) -> Self {
        let colors = if dark {
            [(INK, PAPER), (CITRON, INK)]
        } else {
            [(rgb(0xedeee7), INK), (INK, PAPER)]
        };
        let choices = std::array::from_fn(|i| {
            let selected = i == initial;
            let (background, ink) = colors[usize::from(selected)];
            Choice {
                selected: cx.signal(selected),
                background: cx.signal(background),
                ink: cx.signal(ink),
            }
        });
        Self { choices, colors }
    }

    fn flags(self) -> [Signal<bool>; N] {
        self.choices.map(|choice| choice.selected)
    }

    fn select(self, index: usize, cx: &mut Context<'_>) -> Result<()> {
        for (i, choice) in self.choices.into_iter().enumerate() {
            let selected = i == index;
            if *cx.read(choice.selected)? != selected {
                let (bg, ink) = self.colors[usize::from(selected)];
                cx.set(choice.selected, selected)?;
                cx.set(choice.background, bg)?;
                cx.set(choice.ink, ink)?;
            }
        }
        Ok(())
    }

    fn buttons(self, names: [&'static str; N]) -> Div {
        self.buttons_with(names, |_, _| Ok(()))
    }

    fn buttons_with(
        self,
        names: [&'static str; N],
        changed: impl Fn(usize, &mut Context<'_>) -> Result<()> + Clone + 'static,
    ) -> Div {
        div()
            .flex_row()
            .gap(px(3.0))
            .children(names.into_iter().enumerate().map(|(i, name)| {
                let choice = self.choices[i];
                let button = div()
                    .px(px(10.0))
                    .h(px(30.0))
                    .justify_center()
                    .rounded(px(8.0))
                    .bg(choice.background)
                    .text_color(choice.ink)
                    .child(label(name, 13.0, 18.0));
                let changed = changed.clone();
                activation_button(button, None, move |cx| {
                    if !*cx.read(choice.selected)? {
                        self.select(i, cx)?;
                        changed(i, cx)?;
                    }
                    Ok(())
                })
            }))
    }
}

struct ActivateChoice;

fn activation_button(
    button: Div,
    accent: Option<Rgba>,
    activate: impl Fn(&mut Context<'_>) -> Result<()> + Clone + 'static,
) -> Div {
    button
        .cursor_pointer()
        .focus_indicator(move |paint| {
            let inset = px(10.).min(paint.size.width / 2.);
            paint.fill_rounded_rect(
                Origin {
                    x: inset,
                    y: (paint.size.height - px(4.)).max(px(0.)),
                },
                Size {
                    width: paint.size.width - inset * 2.,
                    height: paint.size.height.min(px(2.)),
                },
                px(1.),
                match accent {
                    Some(color) => color.into(),
                    None => paint.text_color()?,
                },
            )
        })
        .on_click(activate.clone())
        .on_action(move |_: &ActivateChoice, cx| activate(cx))
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

/// Configures the supplied UI and creates the gallery's unmounted root component.
pub fn gallery(mut ui: Ui) -> Result<(Ui, impl Render)> {
    let clipboard = ui.clipboard();
    ui.bind_keys([
        KeyBinding::new(Key::Enter, ActivateChoice),
        KeyBinding::new(Key::Space, ActivateChoice),
    ]);
    lists::bind_keys(&mut ui);
    markets::bind_keys(&mut ui);
    tone_sheet::bind_keys(&mut ui);
    ui.set_tooltip_style(TooltipStyle {
        background: INK.into(),
        text_color: PAPER,
        ..TooltipStyle::default()
    })?;
    let ramp = linear_gradient(
        110.0,
        specimens::STOPS.map(|(at, color, _)| color_stop(color, at)),
    )?;
    let overlay_ink = rgba(0x201e394d);
    let patterns = [
        checkerboard(overlay_ink, px(10.0))?.into(),
        diagonal_stripes(overlay_ink, px(2.5), px(5.0))?.into(),
    ];
    Ok((
        ui,
        component(move |cx| {
            let pages = Selection::new(cx, 2, true);
            let spacing = Selection::new(cx, 1, false);
            let sample_text = cx.signal(solor::text::TextInputState::new("Aa"));
            let geometry = Selection::new(cx, 0, false);
            let map = solor::GradientMap::from(ramp.clone());
            let ramp = cx.signal(ramp);
            let overlay = Selection::new(cx, 0, false);
            let preview = Selection::new(cx, 0, false);
            let alignment = Selection::new(cx, 1, false);
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
                        .child(pages.buttons(PAGES))
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
                .child_when(pages.choices[0].selected, lists::palette)
                .child_when(pages.choices[1].selected, markets::market)
                .child_when(pages.choices[2].selected, move || {
                    specimens::typography(spacing, sample_text, &clipboard)
                })
                .child_when(pages.choices[3].selected, move || {
                    specimens::paint(geometry, overlay, preview, ramp, &patterns, map.clone())
                })
                .child_when(pages.choices[4].selected, move || {
                    specimens::layout(alignment)
                })
        }),
    ))
}

fn page(title: &'static str, control: Div, sample: impl IntoElement, details: Div) -> Div {
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

fn stage(fill: impl Into<solor::style::Background>) -> Div {
    div()
        .items_stretch()
        .justify_center()
        .h(px(208.0))
        .rounded(px(16.0))
        .bg(fill)
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
