//! Offscreen variable-list frame recording after a splice above the reading row.
use anyhow::Result;
use solor::{
    ListScrollHandle, ScrollAlignment, Styled, Ui, component, div, layout::Size, px, variable_list,
};
use std::time::Instant;

fn run(count: usize) -> Result<()> {
    let mut ui = Ui::new();
    let rows = ui
        .context()
        .signal_vec((0..count).map(|i| px(if i % 2 == 0 { 18.0 } else { 22.0 })));
    let scroll = ListScrollHandle::new();
    let view = ui.mount(component(|_| {
        variable_list(rows, px(20.0), |height| div().h(height)).scroll_handle(&scroll)
    }))?;
    let size = Size {
        width: px(320.0),
        height: px(240.0),
    };
    let frame = |ui: &mut Ui| -> Result<()> {
        ui.flush()?;
        ui.layout(view, size)?;
        let frame = ui.record_frame(view)?;
        ui.accept_pointer_frame(frame.into_pointer_frame())
    };
    let reading = count - 64;
    scroll.scroll_to(reading, ScrollAlignment::Top);
    frame(&mut ui)?;
    let root = ui.root(view)?;
    let children: Vec<_> = ui.children(view, root)?.collect();
    let anchor = children
        .iter()
        .copied()
        .find(|&id| ui.geometry(view, id).unwrap().origin.y == px(0.0))
        .unwrap();
    let at = reading - 16;
    let mut samples = [Vec::new(), Vec::new()];
    for cycle in 0..120 {
        for (step, times) in samples.iter_mut().enumerate() {
            let start = Instant::now();
            if step == 0 {
                rows.splice(at..at, [px(20.0); 8], &mut ui.context())?;
            } else {
                rows.splice(at..at + 8, [], &mut ui.context())?;
            }
            frame(&mut ui)?;
            let elapsed = start.elapsed();
            assert_eq!(ui.geometry(view, anchor).unwrap().origin.y, px(0.0));
            assert!(ui.children(view, root)?.eq(children.iter().copied()));
            if cycle >= 20 {
                times.push(elapsed);
            }
        }
    }
    for (label, mut times) in ["insert", "remove"].into_iter().zip(samples) {
        times.sort_unstable();
        println!(
            "{count:>7} rows  {label} 8  median {:?}  p95 {:?}",
            times[49], times[94]
        );
    }
    Ok(())
}

fn main() -> Result<()> {
    for count in [10_000, 100_000, 500_000] {
        run(count)?;
    }
    Ok(())
}
