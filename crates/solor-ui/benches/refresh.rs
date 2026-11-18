//! Release workload for a retained meter and a conditional row in a flat column.
use anyhow::Result;
use solor_ui::px;
use std::hint::black_box;
use std::time::{Duration, Instant};

use solor_ui::layout::{
    self, LayoutContext, LayoutNode, LayoutOutput, MeasureContext, Metrics, Query, Size,
};
use solor_ui::{Draw, Element, PaintContext, Signal, Ui, component, div, element};

const SIZE: Size = Size {
    width: px(100.0),
    height: px(800.0),
};
const ROW: Size = Size {
    width: px(100.0),
    height: px(1.0),
};

struct Meter(Signal<u32>);

impl LayoutNode for Meter {
    fn measure(&mut self, _: &mut MeasureContext<'_, '_>, query: Query) -> layout::Result<Metrics> {
        Ok(Metrics {
            size: Size {
                width: query.resolve(ROW.width),
                ..ROW
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

impl Element for Meter {
    fn paint(&self, cx: &mut PaintContext) -> Result<()> {
        let width = cx.size.width * *cx.read(self.0)? as f32 / 100.0;
        cx.fill(Size { width, ..ROW }, [0, 255, 0, 255]);
        Ok(())
    }
}

fn report(case: &str, count: usize, mut samples: Vec<Duration>) {
    samples.sort_unstable();
    let ns = |percent: usize| samples[(samples.len() - 1) * percent / 100].as_nanos();
    println!(
        "{case:18} {count:5} items   median {:9} ns   p95 {:9} ns",
        ns(50),
        ns(95)
    );
}

fn run(siblings: usize) -> Result<()> {
    let mut ui = Ui::new();
    let level = ui.context().signal(0);
    let visible = ui.context().signal(false);
    let view = ui.mount(component(|_| {
        let mut column = div().column().child(element(Meter(level)));
        for _ in 0..siblings {
            column = column.child(div().padding(px(1.0)));
        }
        column.child_when(visible, || div().padding(px(1.0)))
    }))?;
    assert_eq!(ui.flush()?, 1);
    ui.layout(view, SIZE)?;
    let root = ui.root(view)?;
    let meter = ui.children(view, root)?.next().unwrap();
    let mut commands = Vec::with_capacity(1);
    for writes in [1, 1_000] {
        let mut samples = Vec::new();
        for frame in 0..220u32 {
            commands.clear();
            let start = Instant::now();
            for n in 0..writes {
                ui.context().set(level, black_box((frame + n) % 101))?;
            }
            let visits = ui.flush()?;
            let changed = ui.layout(view, SIZE)?;
            ui.paint(view, meter, &mut commands)?;
            let elapsed = start.elapsed();
            assert_eq!((visits, changed), (0, 0));
            assert!(
                matches!(commands[..], [Draw::Fill {size, ..}] if size.width == px(((frame+writes-1)%101) as f32))
            );
            if frame >= 20 {
                samples.push(elapsed);
            }
        }
        report(&format!("meter x{writes}"), siblings, samples);
    }
    let mut dispatch = Vec::new();
    let mut layout_times = Vec::new();
    let mut combined = Vec::new();
    for frame in 0..110 {
        let start = Instant::now();
        ui.context().set(visible, frame % 2 == 0)?;
        let visits = ui.flush()?;
        let after_flush = Instant::now();
        black_box(ui.layout(view, SIZE)?);
        let end = Instant::now();
        assert_eq!(visits, 1);
        assert_eq!(
            ui.children(view, root)?.len(),
            1 + siblings + usize::from(frame % 2 == 0)
        );
        if frame >= 10 {
            dispatch.push(after_flush - start);
            layout_times.push(end - after_flush);
            combined.push(end - start);
        }
    }
    report("toggle dispatch", siblings, dispatch);
    report("toggle layout", siblings, layout_times);
    report("toggle total", siblings, combined);
    Ok(())
}

// Every write changes the value, but the final closed state preserves geometry.
fn shared_burst(subscribers: usize) -> Result<()> {
    let mut ui = Ui::new();
    let visible = ui.context().signal(false);
    let view = ui.mount(component(|_| {
        let mut column = div().column();
        for _ in 0..subscribers {
            column = column.child_when(visible, div);
        }
        column
    }))?;
    assert_eq!(ui.flush()?, subscribers);
    ui.layout(view, SIZE)?;
    let mut samples = Vec::new();
    for frame in 0..110 {
        let start = Instant::now();
        for n in 0..1_000 {
            ui.context().set(visible, black_box(n % 2 == 0))?;
        }
        let visits = ui.flush()?;
        let changed = ui.layout(view, SIZE)?;
        let elapsed = start.elapsed();
        assert_eq!((visits, changed), (subscribers, 0));
        if frame >= 10 {
            samples.push(elapsed);
        }
    }
    report("shared x1000", subscribers, samples);
    Ok(())
}

fn paint_all(count: usize) -> Result<()> {
    let mut ui = Ui::new();
    let level = ui.context().signal(0);
    let view = ui.mount(component(|_| {
        let mut column = div().column();
        for _ in 0..count {
            column = column.child(element(Meter(level)));
        }
        column
    }))?;
    ui.flush()?;
    ui.layout(view, SIZE)?;
    let meters: Vec<_> = ui.children(view, ui.root(view)?)?.collect();
    let mut commands = Vec::with_capacity(count);
    let mut samples = Vec::new();
    for frame in 0..110 {
        commands.clear();
        let start = Instant::now();
        ui.context().set(level, (frame + 1) % 101)?;
        let visits = ui.flush()?;
        let changed = ui.layout(view, SIZE)?;
        for &meter in &meters {
            ui.paint(view, meter, &mut commands)?;
        }
        let elapsed = start.elapsed();
        assert_eq!((visits, changed, commands.len()), (0, 0, count));
        assert!(commands.iter().all(
            |draw| matches!(draw, Draw::Fill {size, ..} if size.width == px(((frame+1)%101) as f32))
        ));
        if frame >= 10 {
            samples.push(elapsed);
        }
    }
    report("paint all", count, samples);
    Ok(())
}

fn scheduled_meter(siblings: usize) -> Result<()> {
    let mut ui = Ui::new();
    let level = ui.context().signal(0);
    let view = ui.mount(component(|_| {
        let mut column = div()
            .column()
            .child(element(Meter(level)).repaint_on(level));
        for _ in 0..siblings {
            column = column.child(div().padding(px(1.0)));
        }
        column
    }))?;
    ui.flush()?;
    ui.layout(view, SIZE)?;
    let root = ui.root(view)?;
    let meter = ui.children(view, root)?.next().unwrap();
    let mut ready = Vec::with_capacity(1);
    let mut commands = Vec::with_capacity(1);
    let mut samples = Vec::new();
    for frame in 0..220 {
        ready.clear();
        commands.clear();
        let start = Instant::now();
        for n in 0..1_000 {
            ui.context().set(level, black_box((frame + n) % 101))?;
        }
        let visits = ui.flush()?;
        let changed = ui.layout(view, SIZE)?;
        ui.collect_repaints(view, &mut ready)?;
        for &node in &ready {
            ui.paint(view, node, &mut commands)?;
        }
        let elapsed = start.elapsed();
        assert_eq!((visits, changed), (0, 0));
        assert_eq!(ready, [meter]);
        assert!(
            matches!(commands[..], [Draw::Fill{size, ..}] if size.width == px(((frame+999)%101) as f32))
        );
        if frame >= 20 {
            samples.push(elapsed);
        }
    }
    report("scheduled x1000", siblings, samples);
    Ok(())
}

fn main() -> Result<()> {
    for siblings in [100, 1_000, 10_000] {
        run(siblings)?;
        paint_all(siblings)?;
        scheduled_meter(siblings)?;
    }
    for subscribers in [1, 1_000] {
        shared_burst(subscribers)?;
    }
    Ok(())
}
