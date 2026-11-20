//! A deterministic quote feed driven by the window's frame opportunities.
mod chart;
use chart::Chart;

use std::{cell::RefCell, rc::Rc, time::Duration};

use anyhow::Result;
use solor::{
    Context, Div, Fill, IntoElement, RefreshMode, SharedString, Signal, Styled, component, div,
    element, px, rgb, text,
};

use crate::{Selection, label};

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
const BASE: solor::Rgba = rgb(0x151b20);
const PANEL: solor::Rgba = rgb(0x20282e);
const ACTIVE: solor::Rgba = rgb(0x2b393b);
const INK: solor::Rgba = rgb(0xe9efe9);
const DIM: solor::Rgba = rgb(0xa4b2b6);
const UP: solor::Rgba = rgb(0x57d4a6);
const DOWN: solor::Rgba = rgb(0xec8590);

#[derive(Clone, Copy, Default)]
struct Candle {
    open: i32,
    high: i32,
    low: i32,
    close: i32,
    volume: u32,
}

struct Tape {
    candles: [Candle; PERIODS],
    next: usize,
    price: i32,
    anchor: i32,
    seed: u32,
}

impl Tape {
    fn new(anchor: i32, seed: u32) -> Self {
        let mut tape = Self {
            candles: [Candle::default(); PERIODS],
            next: 0,
            price: anchor,
            anchor,
            seed,
        };
        for _ in 0..PERIODS {
            tape.advance();
        }
        tape
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
        let mut c = Candle {
            open: self.price,
            high: self.price,
            low: self.price,
            ..Default::default()
        };
        for _ in 0..TICKS {
            self.seed ^= self.seed << 13;
            self.seed ^= self.seed >> 17;
            self.seed ^= self.seed << 5;
            self.price += (self.seed % 9) as i32 - 4 + (self.anchor - self.price) / 160;
            c.high = c.high.max(self.price);
            c.low = c.low.min(self.price);
            c.volume += 1 + (self.seed >> 8) % 100;
        }
        c.close = self.price;
        self.candles[self.next] = c;
        self.next = (self.next + 1) % PERIODS;
    }
}

type Feed = Rc<RefCell<[Tape; SYMBOLS.len()]>>;

#[derive(Clone, Copy)]
struct Quotes {
    rows: [Signal<SharedString>; SYMBOLS.len()],
    highlight: [Signal<Fill>; SYMBOLS.len()],
    selected: Signal<usize>,
    symbol: Signal<SharedString>,
    name: Signal<SharedString>,
    last: Signal<SharedString>,
    change: Signal<SharedString>,
    ink: Signal<solor::Rgba>,
}

fn price(cents: i32) -> SharedString {
    format!("{:.2}", f64::from(cents) / 100.0).into()
}

impl Quotes {
    fn detail(&self, cx: &mut Context<'_>, tape: &Tape) -> Result<()> {
        cx.set(self.last, price(tape.price))?;
        cx.set(self.change, tape.change())?;
        cx.set(self.ink, tape.ink())
    }
}

fn watchlist(quotes: Quotes, feed: Feed) -> Div {
    let mut list = div().w(px(196.0)).items_stretch().bg(PANEL).child(
        div()
            .px(px(14.0))
            .h(px(42.0))
            .justify_center()
            .child(label("Watchlist", 15.0, 22.0)),
    );
    for (i, &(symbol, name, _)) in SYMBOLS.iter().enumerate() {
        let feed = feed.clone();
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
                .child(label(name, 11.0, 17.0).text_color(DIM))
                .on_click(move |cx| {
                    let previous = *cx.read(quotes.selected)?;
                    cx.set(quotes.highlight[previous], PANEL.into())?;
                    cx.set(quotes.highlight[i], ACTIVE.into())?;
                    cx.set(quotes.selected, i)?;
                    cx.set(quotes.symbol, symbol.into())?;
                    cx.set(quotes.name, name.into())?;
                    quotes.detail(cx, &feed.borrow()[i])
                }),
        );
    }
    list
}

fn live_updates(
    feed: Feed,
    quotes: Quotes,
    fps: Signal<SharedString>,
) -> impl FnMut(Duration, &mut Context<'_>) -> Result<()> {
    let mut start: Option<Duration> = None;
    let mut intervals = 0u32;
    move |now, cx| {
        let mut data = feed.borrow_mut();
        for (i, tape) in data.iter_mut().enumerate() {
            tape.advance();
            cx.set(quotes.rows[i], price(tape.price))?;
        }
        let i = *cx.read(quotes.selected)?;
        quotes.detail(cx, &data[i])?;
        if let Some(first) = start {
            intervals += 1;
            let elapsed = now.saturating_sub(first);
            if elapsed >= Duration::from_secs(1) {
                cx.set(
                    fps,
                    format!("{:.1}", f64::from(intervals) / elapsed.as_secs_f64()).into(),
                )?;
                start = Some(now);
                intervals = 0;
            }
        } else {
            start = Some(now);
        }
        Ok(())
    }
}

pub(super) fn market() -> impl IntoElement {
    component(|cx| {
        let feed: Feed = Rc::new(RefCell::new(std::array::from_fn(|i| {
            Tape::new(SYMBOLS[i].2, 9127 + i as u32 * 31)
        })));
        let quotes = Quotes {
            selected: cx.signal(0),
            rows: std::array::from_fn(|i| cx.signal(price(feed.borrow()[i].price))),
            highlight: std::array::from_fn(|i| {
                cx.signal(if i == 0 { ACTIVE.into() } else { PANEL.into() })
            }),
            symbol: cx.signal(SYMBOLS[0].0.into()),
            name: cx.signal(SYMBOLS[0].1.into()),
            last: cx.signal(price(feed.borrow()[0].price)),
            change: cx.signal(feed.borrow()[0].change()),
            ink: cx.signal(feed.borrow()[0].ink()),
        };
        let playback = Selection::new(cx, 0);
        let [running, paused] = playback.0;
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
                    .child(label("Simulated feed", 12.0, 20.0).text_color(DIM)),
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
                            .on_click(move |cx| {
                                let run = !*cx.read(running)?;
                                playback.select(usize::from(!run), cx)?;
                                cx.set(fps, "--".into())?;
                                cx.set(action, if run { "Pause" } else { "Resume" }.into())
                            }),
                    ),
            );
        let detail = div()
            .flex_1()
            .items_stretch()
            .p(px(18.0))
            .gap(px(7.0))
            .overflow_hidden()
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        text(quotes.symbol)
                            .text_size(px(22.0))
                            .line_height(px(32.0)),
                    )
                    .child(text(quotes.name).text_color(DIM)),
            )
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .gap(px(16.0))
                    .child(text(quotes.last).text_size(px(38.0)).line_height(px(52.0)))
                    .child(text(quotes.change).text_color(quotes.ink)),
            )
            .child(label("Price and volume", 12.0, 20.0).text_color(DIM))
            .child_when(running, {
                let feed = feed.clone();
                move || {
                    element(Chart {
                        feed: feed.clone(),
                        selected: quotes.selected,
                    })
                    .flex_1()
                    .refresh_mode(RefreshMode::Continuous)
                    .on_frame(live_updates(feed.clone(), quotes, fps))
                }
            })
            .child_when(paused, {
                let feed = feed.clone();
                move || {
                    element(Chart {
                        feed: feed.clone(),
                        selected: quotes.selected,
                    })
                    .flex_1()
                }
            })
            .child(
                div()
                    .flex_row()
                    .justify_between()
                    .text_color(DIM)
                    .child(label("96 periods", 11.0, 18.0))
                    .child(label("Latest", 11.0, 18.0)),
            );
        let rows = watchlist(quotes, feed.clone());
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
