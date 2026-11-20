//! A deterministic quote feed driven by the window's frame opportunities.
mod chart;
use chart::chart;

use std::{cell::Cell, time::Duration};

use anyhow::Result;
use solor::{
    Context, Div, Fill, IntoElement, SharedString, Signal, Styled, component, div, px, rgb, text,
};

use crate::label;

const SYMBOLS: [(&str, &str, i32); 6] = [
    ("ORBX", "Orbex Freight", 4895),
    ("KLPW", "Kelpwise Foods", 2138),
    ("VLTG", "Voltgrid Energy", 11307),
    ("LUMO", "Lumora Optics", 23483),
    ("BRXH", "Brixhill Homes", 6901),
    ("QBFG", "Qubit Forge", 1798),
];
const PERIODS: usize = 96;
const TICKS: usize = 32;
const AVERAGE_PERIODS: usize = 16;
const AVERAGE_INK: solor::Rgba = rgb(0xf7b64d);
const BASE: solor::Rgba = rgb(0x12161e);
const PANEL: solor::Rgba = rgb(0x191f29);
const ACTIVE: solor::Rgba = rgb(0x293a42);
const INK: solor::Rgba = rgb(0xf1f4f6);
const DIM: solor::Rgba = rgb(0xa1adb9);
const UP: solor::Rgba = rgb(0x60ddba);
const DOWN: solor::Rgba = rgb(0xff8492);

#[derive(Clone, Copy)]
struct QuoteSample {
    price: i32,
    average: f32,
}

struct Tape {
    samples: [QuoteSample; PERIODS],
    next: usize,
    price: i32,
    anchor: i32,
    seed: u32,
    rolling_sum: i32,
}

impl Tape {
    fn new(anchor: i32, seed: u32) -> Self {
        let mut tape = Self {
            samples: [QuoteSample {
                price: anchor,
                average: anchor as f32,
            }; PERIODS],
            next: 0,
            price: anchor,
            anchor,
            seed,
            rolling_sum: anchor * AVERAGE_PERIODS as i32,
        };
        // Warm the moving average before any of its samples become visible.
        for _ in 0..(PERIODS + AVERAGE_PERIODS - 1) {
            tape.advance();
        }
        tape
    }

    fn range(&self) -> (i32, i32) {
        self.samples
            .iter()
            .fold((self.price, self.price), |(lo, hi), sample| {
                (
                    lo.min(sample.price).min(sample.average.floor() as i32),
                    hi.max(sample.price).max(sample.average.ceil() as i32),
                )
            })
    }

    fn change(&self) -> SharedString {
        format!(
            "{:+.2}%",
            100.0 * f64::from(self.price - self.anchor) / f64::from(self.anchor)
        )
        .into()
    }

    fn ink(&self) -> solor::Rgba {
        if self.price >= self.anchor { UP } else { DOWN }
    }

    fn advance(&mut self) {
        for _ in 0..TICKS {
            self.seed ^= self.seed << 13;
            self.seed ^= self.seed >> 17;
            self.seed ^= self.seed << 5;
            self.price += (self.seed % 9) as i32 - 4 + (self.anchor - self.price) / 160;
        }
        let leaving = (self.next + PERIODS - AVERAGE_PERIODS) % PERIODS;
        self.rolling_sum += self.price - self.samples[leaving].price;
        self.samples[self.next] = QuoteSample {
            price: self.price,
            average: self.rolling_sum as f32 / AVERAGE_PERIODS as f32,
        };
        self.next = (self.next + 1) % PERIODS;
    }
}

type Feed = [Signal<Tape>; SYMBOLS.len()];

#[derive(Clone, Copy)]
struct Quotes {
    rows: [Signal<SharedString>; SYMBOLS.len()],
    highlight: [Signal<Fill>; SYMBOLS.len()],
    selected: Signal<usize>,
    symbol: Signal<SharedString>,
    name: Signal<SharedString>,
    last: Signal<SharedString>,
    reference: Signal<SharedString>,
    high: Signal<SharedString>,
    low: Signal<SharedString>,
    row_change: [Signal<SharedString>; SYMBOLS.len()],
    row_ink: [Signal<solor::Rgba>; SYMBOLS.len()],
    change: Signal<SharedString>,
    ink: Signal<solor::Rgba>,
}

fn price(cents: i32) -> SharedString {
    format!("{:.2}", f64::from(cents) / 100.0).into()
}

impl Quotes {
    fn detail(&self, cx: &mut Context<'_>, i: usize, tape: Signal<Tape>) -> Result<()> {
        let (lo, hi) = cx.read(tape)?.range();
        cx.set(self.high, price(hi))?;
        cx.set(self.low, price(lo))?;
        cx.set(self.last, cx.read(self.rows[i])?.clone())?;
        cx.set(self.change, cx.read(self.row_change[i])?.clone())?;
        cx.set(self.ink, *cx.read(self.row_ink[i])?)
    }
}

fn watchlist(quotes: Quotes, feed: Feed) -> Div {
    let mut list = div().w(px(210.0)).items_stretch().bg(PANEL).child(
        div()
            .px(px(14.0))
            .h(px(42.0))
            .flex_row()
            .items_center()
            .justify_between()
            .child(label("Watchlist", 15.0, 22.0))
            .child(label("6 symbols", 11.0, 18.0).text_color(DIM)),
    );
    for (i, &(symbol, name, _)) in SYMBOLS.iter().enumerate() {
        let tape = feed[i];
        list = list.child(
            div()
                .px(px(14.0))
                .h(px(51.0))
                .justify_center()
                .items_stretch()
                .bg(quotes.highlight[i])
                .child(
                    div()
                        .flex_row()
                        .justify_between()
                        .child(label(symbol, 14.0, 20.0))
                        .child(
                            text(quotes.rows[i])
                                .text_size(px(14.0))
                                .line_height(px(20.0)),
                        ),
                )
                .child(
                    div()
                        .flex_row()
                        .justify_between()
                        .child(label(name, 11.0, 17.0).text_color(DIM))
                        .child(
                            text(quotes.row_change[i])
                                .text_size(px(11.0))
                                .line_height(px(17.0))
                                .text_color(quotes.row_ink[i]),
                        ),
                )
                .cursor_pointer()
                .on_click(move |cx| {
                    let previous = *cx.read(quotes.selected)?;
                    cx.set(quotes.highlight[previous], PANEL.into())?;
                    cx.set(quotes.highlight[i], ACTIVE.into())?;
                    cx.set(quotes.selected, i)?;
                    cx.set(quotes.symbol, symbol.into())?;
                    cx.set(quotes.name, name.into())?;
                    cx.set(quotes.reference, price(SYMBOLS[i].2))?;
                    quotes.detail(cx, i, tape)
                }),
        );
    }
    list
}

fn live_updates(
    feed: Feed,
    quotes: Quotes,
    fps: Signal<SharedString>,
    timing: Signal<Option<(Duration, u32)>>,
) -> impl FnMut(Duration, &mut Context<'_>) -> Result<()> {
    move |now, cx| {
        for (i, tape) in feed.iter().copied().enumerate() {
            let (last, change, ink) = cx.update_in_place(tape, |tape| {
                tape.advance();
                (price(tape.price), tape.change(), tape.ink())
            })?;
            cx.set(quotes.rows[i], last)?;
            cx.set(quotes.row_change[i], change)?;
            cx.set(quotes.row_ink[i], ink)?;
        }
        let i = *cx.read(quotes.selected)?;
        quotes.detail(cx, i, feed[i])?;
        let rate = cx.update_in_place(timing, |sample| {
            let Some((start, intervals)) = sample else {
                *sample = Some((now, 0));
                return None;
            };
            *intervals += 1;
            let elapsed = now.saturating_sub(*start);
            if elapsed < Duration::from_secs(1) {
                return None;
            }
            let rate = f64::from(*intervals) / elapsed.as_secs_f64();
            *sample = Some((now, 0));
            Some(rate)
        })?;
        if let Some(rate) = rate {
            cx.set(fps, format!("{rate:.1}").into())?;
        }
        Ok(())
    }
}

pub(super) fn market() -> impl IntoElement {
    component(|cx| {
        let tapes: [Tape; SYMBOLS.len()] =
            std::array::from_fn(|i| Tape::new(SYMBOLS[i].2, 9127 + i as u32 * 31));
        let (lo, hi) = tapes[0].range();
        let quotes = Quotes {
            reference: cx.signal(price(SYMBOLS[0].2)),
            low: cx.signal(price(lo)),
            high: cx.signal(price(hi)),
            row_change: std::array::from_fn(|i| cx.signal(tapes[i].change())),
            row_ink: std::array::from_fn(|i| cx.signal(tapes[i].ink())),
            selected: cx.signal(0),
            rows: std::array::from_fn(|i| cx.signal(price(tapes[i].price))),
            highlight: std::array::from_fn(|i| {
                cx.signal(if i == 0 { ACTIVE.into() } else { PANEL.into() })
            }),
            symbol: cx.signal(SYMBOLS[0].0.into()),
            name: cx.signal(SYMBOLS[0].1.into()),
            last: cx.signal(price(tapes[0].price)),
            change: cx.signal(tapes[0].change()),
            ink: cx.signal(tapes[0].ink()),
        };
        let feed = tapes.map(|tape| cx.signal(tape));
        let running = cx.signal(true);
        let timing = cx.signal(None);
        let fps = cx.signal(SharedString::from("--"));
        let action = cx.signal(SharedString::from("Pause"));
        let toolbar = div()
            .flex_row()
            .items_center()
            .justify_between()
            .h(px(55.0))
            .px(px(18.0))
            .bg(PANEL)
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .gap(px(12.0))
                    .child(label("Markets", 22.0, 32.0))
                    .child(
                        div()
                            .flex_row()
                            .items_center()
                            .gap(px(6.0))
                            .child(div().size(px(6.0)).rounded(px(3.0)).bg(DIM))
                            .child(label("Simulated feed", 12.0, 20.0).text_color(DIM)),
                    ),
            )
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(label("Callback FPS", 12.0, 20.0).text_color(DIM))
                    .child(text(fps).w(px(49.0)).text_color(UP))
                    .child(
                        div()
                            .rounded(px(6.0))
                            .bg(ACTIVE)
                            .w(px(64.0))
                            .h(px(28.0))
                            .items_center()
                            .justify_center()
                            .child(text(action).text_size(px(13.0)))
                            .cursor_pointer()
                            .on_click(move |cx| {
                                let run = !*cx.read(running)?;
                                cx.set(running, run)?;
                                cx.set(timing, None)?;
                                cx.set(fps, "--".into())?;
                                cx.set(action, if run { "Pause" } else { "Resume" }.into())
                            }),
                    ),
            );
        let legend = |color, name| {
            div()
                .flex_row()
                .items_center()
                .gap(px(5.0))
                .child(div().size(px(6.0)).rounded(px(2.0)).bg(color))
                .child(label(name, 11.0, 18.0).text_color(DIM))
        };
        let visible = cx.signal(PERIODS);
        let show_latest = cx.signal(false);
        let sample_count = cx.signal(SharedString::from(format!("{PERIODS} samples")));
        let zoom = Cell::new(PERIODS as f32);
        let detail = div()
            .flex_1()
            .items_stretch()
            .p(px(18.0))
            .gap(px(8.0))
            .overflow_hidden()
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex_row()
                            .items_baseline()
                            .gap(px(10.0))
                            .child(
                                text(quotes.symbol)
                                    .text_size(px(23.0))
                                    .line_height(px(32.0)),
                            )
                            .child(text(quotes.name).text_size(px(13.0)).text_color(DIM)),
                    )
                    .child(label("USD", 11.0, 18.0).text_color(DIM)),
            )
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex_row()
                            .items_baseline()
                            .gap(px(10.0))
                            .child(
                                text(quotes.last)
                                    .text_size(px(40.0))
                                    .letter_spacing(px(-1.0))
                                    .line_height(px(50.0)),
                            )
                            .child(text(quotes.change).text_color(quotes.ink)),
                    )
                    .child(
                        div()
                            .items_end()
                            .child(label("Reference price", 11.0, 17.0).text_color(DIM))
                            .child(text(quotes.reference).text_size(px(15.0))),
                    ),
            )
            .child(div().h(px(1.0)).bg(rgb(0x303843)))
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(label("Price history", 14.0, 22.0))
                    .child(
                        div()
                            .flex_row()
                            .gap(px(10.0))
                            .child(legend(UP, "Price"))
                            .child(legend(AVERAGE_INK, "MA 16")),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .flex_row()
                    .items_stretch()
                    .gap(px(6.0))
                    .on_hover(move |inside, cx| cx.set(show_latest, inside))
                    .on_scroll(move |event, cx| {
                        let next =
                            (zoom.get() + event.delta.y.get() * 0.5).clamp(12.0, PERIODS as f32);
                        zoom.set(next);
                        let count = next.round() as usize;
                        if count != *cx.read(visible)? {
                            cx.set(visible, count)?;
                            cx.set(sample_count, format!("{count} samples").into())?;
                        }
                        Ok(())
                    })
                    .child(
                        chart(feed, quotes.selected, visible, show_latest)
                            .flex_1()
                            .on_frame_when(running, live_updates(feed, quotes, fps, timing)),
                    )
                    .child(
                        div()
                            .w(px(43.0))
                            .justify_between()
                            .items_end()
                            .child(text(quotes.high).text_size(px(10.0)).text_color(DIM))
                            .child(text(quotes.low).text_size(px(10.0)).text_color(DIM)),
                    ),
            )
            .child(
                div()
                    .flex_row()
                    .justify_between()
                    .text_color(DIM)
                    .child(
                        div()
                            .flex_row()
                            .gap(px(12.0))
                            .child(text(sample_count).text_size(px(11.0)).line_height(px(18.0)))
                            .child(label("Scroll to zoom", 11.0, 18.0)),
                    )
                    .child(label("Latest price", 11.0, 18.0)),
            );
        let rows = watchlist(quotes, feed);
        div()
            .flex_1()
            .items_stretch()
            .bg(BASE)
            .text_color(INK)
            .child(toolbar)
            .child(
                div()
                    .flex_1()
                    .flex_row()
                    .items_stretch()
                    .child(rows)
                    .child(detail),
            )
    })
}
