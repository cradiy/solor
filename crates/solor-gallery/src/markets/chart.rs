use anyhow::Result;
use solor::{
    Element, PaintContext, Signal,
    layout::{
        self, LayoutContext, LayoutNode, LayoutOutput, MeasureContext, Metrics, Origin, Query, Size,
    },
    px, rgb,
};

use super::{DOWN, Feed, PERIODS, TICKS, UP};

pub(super) struct Chart {
    pub(super) feed: Feed,
    pub(super) selected: Signal<usize>,
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
        let mut rect = |x, y, width, height, color| {
            cx.fill_rect(
                Origin { x: px(x), y: px(y) },
                Size {
                    width: px(width),
                    height: px(height),
                },
                color,
            )
        };
        let lo = tape
            .candles
            .iter()
            .map(|c| c.low)
            .min()
            .unwrap_or(tape.price)
            - 4;
        let hi = tape
            .candles
            .iter()
            .map(|c| c.high)
            .max()
            .unwrap_or(tape.price)
            + 4;
        let y = |price| (hi - price) as f32 / (hi - lo) as f32 * h * 0.77;
        for line in 0..5 {
            rect(0.0, line as f32 * h * 0.2, w, 1.0, rgb(0x303a40))?;
        }
        let step = w / PERIODS as f32;
        let body = step * 0.70;
        for (i, c) in tape.candles[tape.next..]
            .iter()
            .chain(&tape.candles[..tape.next])
            .enumerate()
        {
            let center = (i as f32 + 0.5) * step;
            let color = if c.close >= c.open { UP } else { DOWN };
            let wick = step.min(1.0);
            rect(
                center - wick * 0.5,
                y(c.high),
                wick,
                (y(c.low) - y(c.high)).max(0.8),
                color,
            )?;
            rect(
                center - body * 0.5,
                y(c.open.max(c.close)),
                body,
                (y(c.open) - y(c.close)).abs().max(1.0),
                color,
            )?;
            let volume = h * 0.18 * c.volume as f32 / (TICKS * 100) as f32;
            let volume_ink = if c.close >= c.open {
                rgb(0x326353)
            } else {
                rgb(0x704954)
            };
            rect(center - body * 0.5, h - volume, body, volume, volume_ink)?;
        }
        Ok(())
    }
}
