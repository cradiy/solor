use anyhow::Result;
use solor::{
    Element, PaintContext, Signal,
    layout::{
        self, LayoutContext, LayoutNode, LayoutOutput, MeasureContext, Metrics, Origin, Query, Size,
    },
    px, rgb,
};

use super::{AVERAGE_INK, Feed, PERIODS, UP};

pub(super) struct Chart {
    pub(super) feed: Feed,
    pub(super) selected: Signal<usize>,
    pub(super) visible: Signal<usize>,
}

impl LayoutNode for Chart {
    fn measure(&mut self, _: &mut MeasureContext<'_, '_>, query: Query) -> layout::Result<Metrics> {
        Ok(Metrics {
            size: Size {
                width: query.resolve(px(360.0)),
                height: px(220.0),
            },
            baseline: None,
        })
    }
    fn layout(
        &mut self,
        _: &mut LayoutContext<'_, '_>,
        size: Size,
    ) -> layout::Result<LayoutOutput> {
        Ok(LayoutOutput {
            content: size,
            baseline: None,
        })
    }
}

impl Element for Chart {
    fn paint(&self, cx: &mut PaintContext) -> Result<()> {
        let feed = self.feed.borrow();
        let tape = &feed[*cx.read(self.selected)?];
        let (w, h) = (cx.size.width.get(), cx.size.height.get());
        if w < 1.0 || h < 1.0 {
            return Ok(());
        }
        let count = *cx.read(self.visible)?;
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
        cx.stroke_polyline(prices, px(2.0), UP)?;
        cx.stroke_polyline(averages, px(1.5), AVERAGE_INK)
    }
}
