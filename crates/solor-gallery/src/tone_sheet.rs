use std::rc::Rc;

use crate::{INK, LINE, PAPER, activation_button, label};
use solor::{
    Context, Div, Fill, IntoElement, Pixels, Rgba, ScrollHandle, SharedString, Signal, Styled,
    component, div, horizontal_scrollbar,
    keyboard::{Key, KeyBinding, Modifier},
    layout::{Origin, Size},
    pointer::ScrollDelta,
    px, rgb, scrollbar, text,
};

const TILE_SIZE: Size = Size {
    width: px(124.0),
    height: px(64.0),
};
const TILE_GAP: Pixels = px(4.0);
const LABEL_HEIGHT: Pixels = px(19.0);

const TONE_SHEET: &str = "ToneSheet";

const TONES: usize = 9;

struct Pan(ScrollDelta);
struct MoveSelection {
    columns: isize,
    rows: isize,
}

pub(super) fn bind_keys(ui: &mut solor::Ui) {
    let column = TILE_SIZE.width + TILE_GAP;
    let row = TILE_SIZE.height + TILE_GAP;
    for (key, columns, rows) in [
        (Key::ArrowLeft, -1, 0),
        (Key::ArrowRight, 1, 0),
        (Key::ArrowUp, 0, -1),
        (Key::ArrowDown, 0, 1),
    ] {
        let delta = ScrollDelta {
            x: column * columns as f32,
            y: row * rows as f32,
        };
        ui.bind_keys([
            KeyBinding::new(key.clone(), Pan(delta)).in_context(TONE_SHEET),
            KeyBinding::new(key.with(Modifier::Shift), MoveSelection { columns, rows })
                .in_context(TONE_SHEET),
        ]);
    }
}

const FAMILIES: [(&str, Rgba); 9] = [
    ("Clay", rgb(0xb86145)),
    ("Amber", rgb(0xc68d35)),
    ("Olive", rgb(0x939445)),
    ("Sage", rgb(0x66886c)),
    ("Lagoon", rgb(0x388a91)),
    ("Cobalt", rgb(0x456bab)),
    ("Iris", rgb(0x7663ac)),
    ("Plum", rgb(0x965886)),
    ("Rose", rgb(0xbd6579)),
];

fn swatch(base: Rgba, step: u8) -> Rgba {
    // Step five is the family color. Each adjacent step mixes in 18% more paper or ink.
    let end = if step < 5 { PAPER } else { INK };
    let weight = u16::from(step.abs_diff(5)) * 18;
    let mix = |a, b| ((u16::from(a) * (100 - weight) + u16::from(b) * weight + 50) / 100) as u8;
    Rgba::new(
        mix(base.red, end.red),
        mix(base.green, end.green),
        mix(base.blue, end.blue),
        255,
    )
}

fn hex(color: Rgba) -> String {
    format!("#{:02X}{:02X}{:02X}", color.red, color.green, color.blue)
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Tone {
    row: usize,
    column: usize,
}

impl Tone {
    fn origin(self) -> Origin {
        Origin {
            x: (TILE_SIZE.width + TILE_GAP) * self.column as f32,
            y: (TILE_SIZE.height + TILE_GAP) * self.row as f32,
        }
    }

    fn color(self) -> Rgba {
        swatch(FAMILIES[self.row].1, self.column as u8 + 1)
    }

    fn name(self) -> String {
        format!("{} {}00", FAMILIES[self.row].0, self.column + 1)
    }

    fn caption(self) -> SharedString {
        format!("{}  /  {}", self.name(), hex(self.color())).into()
    }

    fn moved(self, by: &MoveSelection) -> Self {
        Self {
            row: self
                .row
                .saturating_add_signed(by.rows)
                .min(FAMILIES.len() - 1),
            column: self.column.saturating_add_signed(by.columns).min(TONES - 1),
        }
    }
}

struct ToneSelection {
    current: Signal<Tone>,
    chip: Signal<Fill>,
    readout: Signal<SharedString>,
    marked_column: [Signal<Option<usize>>; FAMILIES.len()],
}

impl ToneSelection {
    fn new(cx: &mut Context<'_>) -> Self {
        let first = Tone::default();
        Self {
            current: cx.signal(first),
            chip: cx.signal(first.color().into()),
            readout: cx.signal(first.caption()),
            marked_column: std::array::from_fn(|row| {
                cx.signal((row == first.row).then_some(first.column))
            }),
        }
    }

    fn select(&self, next: Tone, cx: &mut Context<'_>) -> anyhow::Result<()> {
        let previous = *cx.read(self.current)?;
        if next != previous {
            if previous.row != next.row {
                cx.set(self.marked_column[previous.row], None)?;
            }
            cx.set(self.marked_column[next.row], Some(next.column))?;
            cx.set(self.chip, next.color().into())?;
            cx.set(self.readout, next.caption())?;
            cx.set(self.current, next)?;
        }
        Ok(())
    }
}

fn tile(tone: Tone, selection: Rc<ToneSelection>) -> Div {
    div()
        .w(TILE_SIZE.width)
        .h(TILE_SIZE.height)
        .flex_shrink(0.0)
        .items_stretch()
        .child(div().flex_1().rounded(px(6.0)).bg(tone.color()))
        .child(
            div().px(px(6.0)).child(
                text(tone.name())
                    .text_size(px(12.0))
                    .line_height(LABEL_HEIGHT),
            ),
        )
        .cursor_pointer()
        .on_click(move |cx| selection.select(tone, cx))
}

pub(super) fn sheet() -> impl IntoElement {
    component(|cx| {
        let scroll = ScrollHandle::new();
        let selection = Rc::new(ToneSelection::new(cx));
        let selected = selection.current;
        let reveal = scroll.clone();
        let pan = scroll.clone();
        let scroll_for_selection = scroll.clone();
        let keyboard_selection = selection.clone();
        let rows = (0..FAMILIES.len()).map(|row| {
            let marked = selection.marked_column[row];
            div()
                .flex_row()
                .gap(TILE_GAP)
                .flex_shrink(0.0)
                .paint(move |paint| {
                    if let Some(column) = *paint.read(marked)? {
                        paint.fill_rect(
                            Origin {
                                x: (TILE_SIZE.width + TILE_GAP) * column as f32,
                                y: TILE_SIZE.height - LABEL_HEIGHT,
                            },
                            Size {
                                width: TILE_SIZE.width,
                                height: LABEL_HEIGHT,
                            },
                            LINE,
                        )?;
                    }
                    Ok(())
                })
                .children((0..TONES).map(|column| tile(Tone { row, column }, selection.clone())))
        });
        div()
            .flex_1()
            .items_stretch()
            .gap(px(4.0))
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .h(px(32.0))
                    .child(div().size(px(20.0)).rounded(px(5.0)).bg(selection.chip))
                    .child(
                        text(selection.readout)
                            .text_size(px(14.0))
                            .whitespace_nowrap(),
                    )
                    .child(div().flex_1())
                    .child(label("9 families  /  81 swatches", 12.0, 18.0).text_color(crate::MUTED))
                    .child(activation_button(
                        div()
                            .px(px(10.0))
                            .h(px(28.0))
                            .justify_center()
                            .rounded(px(6.0))
                            .bg(LINE)
                            .child(label("Show selection", 12.0, 18.0)),
                        Some(INK),
                        move |cx| reveal.scroll_to_reveal(cx.read(selected)?.origin(), TILE_SIZE),
                    )),
            )
            .child(
                div()
                    .flex_row()
                    .items_stretch()
                    .flex_1()
                    .pt(px(2.0))
                    .key_context(TONE_SHEET)
                    .focus_indicator(crate::viewport_focus_indicator)
                    .on_action(move |by: &MoveSelection, cx| {
                        let next = cx.read(selected)?.moved(by);
                        keyboard_selection.select(next, cx)?;
                        scroll_for_selection.scroll_to_reveal(next.origin(), TILE_SIZE)
                    })
                    .on_action(move |pan_by: &Pan, _| {
                        pan.scroll_by(pan_by.0)?;
                        Ok(())
                    })
                    .child(
                        div()
                            .flex_1()
                            .gap(TILE_GAP)
                            .overflow_auto()
                            .track_scroll(&scroll)
                            .children(rows),
                    )
                    .child(scrollbar(&scroll)),
            )
            .child(horizontal_scrollbar(&scroll))
    })
}
