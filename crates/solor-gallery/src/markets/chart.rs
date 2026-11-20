use solor::{
    Div, Signal, div,
    layout::{Origin, Size},
    px, rgb, rgba,
};

use super::{AVERAGE_INK, Feed, PERIODS, UP};

pub(super) fn chart(
    feed: Feed,
    selected: Signal<usize>,
    visible: Signal<usize>,
    show_latest: Signal<bool>,
) -> Div {
    div().paint(move |cx| {
        let (w, h) = (cx.size.width.get(), cx.size.height.get());
        if w < 1.0 || h < 1.0 {
            return Ok(());
        }
        let count = *cx.read(visible)?;
        let tape = cx.read(feed[*cx.read(selected)?])?;
        let (lo, hi) = tape.range();
        let inset = 1.0_f32.min(w * 0.5).min(h * 0.5);
        let plot_height = h - 2.0 * inset;
        for line in 0..5 {
            cx.fill_rect(
                Origin {
                    x: px(0.0),
                    y: px(inset + (plot_height - 1.0).max(0.0) * line as f32 / 4.0),
                },
                Size {
                    width: px(w),
                    height: px(1.0),
                },
                rgb(0x2b333e),
            )?;
        }
        let y = |value| {
            if hi == lo {
                px(h * 0.5)
            } else {
                px(inset + (hi as f32 - value) / (hi - lo) as f32 * plot_height)
            }
        };
        let mut prices = Vec::with_capacity(count);
        let mut averages = Vec::with_capacity(count);
        for (i, sample) in tape.samples[tape.next..]
            .iter()
            .chain(&tape.samples[..tape.next])
            .skip(PERIODS - count)
            .enumerate()
        {
            let x = px(inset + i as f32 / (count - 1) as f32 * (w - 2.0 * inset));
            prices.push(Origin {
                x,
                y: y(sample.price as f32),
            });
            averages.push(Origin {
                x,
                y: y(sample.average),
            });
        }
        let mut area = Vec::with_capacity(count + 2);
        area.extend_from_slice(&prices);
        let baseline = px(h - inset);
        area.push(Origin {
            x: px(w - inset),
            y: baseline,
        });
        area.push(Origin {
            x: px(inset),
            y: baseline,
        });
        cx.fill_polygon(area, rgba(0x40ce9420))?;
        if *cx.read(show_latest)? {
            cx.fill_rect(
                Origin {
                    x: px(0.0),
                    y: y(tape.price as f32).min(px(h - 1.0)),
                },
                Size {
                    width: px(w),
                    height: px(1.0),
                },
                rgba(0x60ddba70),
            )?;
        }
        cx.stroke_polyline(prices, px(2.0), UP)?;
        cx.stroke_polyline(averages, px(1.5), AVERAGE_INK)
    })
}
