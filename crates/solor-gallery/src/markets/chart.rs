use solor::{
    Context, Div, SharedString, Signal, div,
    layout::{Origin, Size},
    pointer::PointerKind,
    px, rgb, rgba,
};

use super::{AVERAGE_INK, Feed, PERIODS, Tape, UP};

const INSPECT_HINT: &str = "Hover to inspect";

#[derive(Clone, Copy)]
pub(super) struct Inspection {
    fraction: Signal<Option<f32>>,
    age: Signal<Option<usize>>,
    pub(super) readout: Signal<SharedString>,
}

impl Inspection {
    pub(super) fn new(cx: &mut Context<'_>) -> Self {
        Self {
            fraction: cx.signal(None),
            age: cx.signal(None),
            readout: cx.signal(INSPECT_HINT.into()),
        }
    }

    pub(super) fn locate(
        self,
        cx: &mut Context<'_>,
        count: usize,
        tape: Signal<Tape>,
    ) -> anyhow::Result<()> {
        let age = cx
            .read(self.fraction)?
            .map(|fraction| count - 1 - (fraction * (count - 1) as f32).round() as usize);
        if age != *cx.read(self.age)? {
            cx.set(self.age, age)?;
            match age {
                Some(_) => self.refresh(cx, tape)?,
                None => cx.set(self.readout, INSPECT_HINT.into())?,
            }
        }
        Ok(())
    }

    pub(super) fn refresh(self, cx: &mut Context<'_>, tape: Signal<Tape>) -> anyhow::Result<()> {
        if let Some(age) = *cx.read(self.age)? {
            let tape = cx.read(tape)?;
            let sample = tape.samples[(tape.next + PERIODS - 1 - age) % PERIODS];
            let price = f64::from(sample.price) / 100.0;
            let average = f64::from(sample.average) / 100.0;
            cx.set(
                self.readout,
                format!("Price {price:.2}   MA {average:.2}").into(),
            )?;
        }
        Ok(())
    }
}

fn plot_inset(size: Size) -> f32 {
    1.0_f32
        .min(size.width.get() * 0.5)
        .min(size.height.get() * 0.5)
}

pub(super) fn chart(
    feed: Feed,
    selected: Signal<usize>,
    visible: Signal<usize>,
    inspection: Inspection,
) -> Div {
    let plot = div().paint(move |cx| {
        let (w, h) = (cx.size.width.get(), cx.size.height.get());
        if w < 1.0 || h < 1.0 {
            return Ok(());
        }
        let count = *cx.read(visible)?;
        let tape = cx.read(feed[*cx.read(selected)?])?;
        let (lo, hi) = tape.range();
        let inset = plot_inset(cx.size);
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
        let marker = cx.read(inspection.age)?.map(|age| prices[count - 1 - age]);
        if let Some(point) = marker {
            cx.fill_rect(
                Origin {
                    x: point.x.min(px(w - 1.0)),
                    y: px(0.0),
                },
                Size {
                    width: px(1.0),
                    height: px(h),
                },
                rgba(0x60ddba70),
            )?;
        }
        cx.stroke_polyline(prices, px(2.0), UP)?;
        cx.stroke_polyline(averages, px(1.5), AVERAGE_INK)?;
        if let Some(point) = marker {
            cx.with_clip(Origin::default(), cx.size, |cx| {
                cx.fill_circle(point, px(4.0), super::BASE)?;
                cx.fill_circle(point, px(2.5), UP)
            })?;
        }
        Ok(())
    });
    plot.on_pointer_move(move |event, cx| {
        if event.sample.kind == PointerKind::Mouse {
            let size = event.content_size();
            let inset = plot_inset(size);
            let span = size.width.get() - 2.0 * inset;
            let fraction = (span > 0.0)
                .then(|| ((event.content_position().x.get() - inset) / span).clamp(0.0, 1.0));
            cx.set(inspection.fraction, fraction)?;
            inspection.locate(cx, *cx.read(visible)?, feed[*cx.read(selected)?])?;
        }
        Ok(())
    })
    .on_hover(move |inside, cx| {
        if !inside {
            cx.set(inspection.fraction, None)?;
            inspection.locate(cx, *cx.read(visible)?, feed[*cx.read(selected)?])?;
        }
        Ok(())
    })
}
