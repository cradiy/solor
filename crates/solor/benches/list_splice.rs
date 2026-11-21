//! Offscreen variable-list frame recording after a splice above the reading row.
use anyhow::Result;
use solor::{
    ListScrollHandle, Pixels, ScrollAlignment, Styled, Ui, component, div, layout::Size, px,
    variable_list,
};
use std::{
    ops::Range,
    time::{Duration, Instant},
};

struct Sample {
    edit: Duration,
    frame: Duration,
}

fn report(count: usize, label: &str, samples: &[Sample]) {
    let sorted = |select: fn(&Sample) -> Duration| {
        let mut times: Vec<_> = samples.iter().map(select).collect();
        times.sort_unstable();
        times
    };
    let edit = sorted(|s| s.edit);
    let frame = sorted(|s| s.frame);
    let total = sorted(|s| s.edit + s.frame);
    let mid = (samples.len() - 1) / 2;
    let p95 = (samples.len() - 1) * 95 / 100;
    println!(
        "{count:>7} initial rows  {label:20}  edit {:?}  frame {:?}  total {:?}  p95 {:?}",
        edit[mid], frame[mid], total[mid], total[p95]
    );
}

enum Workload {
    History { reserve_pages: bool },
    InsertRemove,
}

fn run(count: usize, workload: Workload) -> Result<()> {
    const PAGE_SIZE: usize = 128;
    const FOLLOWING_PAGES: usize = 100;
    let mut ui = Ui::new();
    let heights = (0..count).map(|i| px(if i % 2 == 0 { 18.0 } else { 22.0 }));
    let start = Instant::now();
    let rows = match workload {
        Workload::History {
            reserve_pages: true,
        } => ui
            .context()
            .signal_vec_with_capacity(heights, count + (FOLLOWING_PAGES + 1) * PAGE_SIZE)?,
        _ => ui.context().signal_vec(heights),
    };
    let init = start.elapsed();
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
    let reading = match workload {
        Workload::History { .. } => 12,
        Workload::InsertRemove => count - 64,
    };
    scroll.scroll_to(reading, ScrollAlignment::Top);
    frame(&mut ui)?;
    let root = ui.root(view)?;
    let children: Vec<_> = ui.children(view, root)?.collect();
    let anchor = children
        .iter()
        .copied()
        .find(|&id| ui.geometry(view, id).unwrap().origin.y == px(0.0))
        .unwrap();
    let mut sample = |range: Range<usize>, values: &[Pixels]| -> Result<Sample> {
        let start = Instant::now();
        rows.splice(range, values.iter().copied(), &mut ui.context())?;
        let edited = Instant::now();
        frame(&mut ui)?;
        let end = Instant::now();
        assert_eq!(ui.geometry(view, anchor).unwrap().origin.y, px(0.0));
        assert!(ui.children(view, root)?.eq(children.iter().copied()));
        Ok(Sample {
            edit: edited - start,
            frame: end - edited,
        })
    };
    if let Workload::History { reserve_pages } = workload {
        let storage = if reserve_pages { "reserved" } else { "default" };
        let page = [px(20.0); PAGE_SIZE];
        let first = sample(0..0, &page)?;
        println!(
            "{count:>7} initial rows  {storage} init {init:?}  first page ({PAGE_SIZE})  edit {:?}  frame {:?}  total {:?}",
            first.edit,
            first.frame,
            first.edit + first.frame
        );
        let mut pages = Vec::with_capacity(FOLLOWING_PAGES);
        for _ in 0..FOLLOWING_PAGES {
            pages.push(sample(0..0, &page)?);
        }
        report(
            count,
            &format!("{storage} next {FOLLOWING_PAGES} pages ({PAGE_SIZE})"),
            &pages,
        );
        return Ok(());
    }
    for (location, at) in [("near end", reading - 16), ("front", 0)] {
        let mut samples = [Vec::new(), Vec::new()];
        for cycle in 0..120 {
            for (step, times) in samples.iter_mut().enumerate() {
                let elapsed = if step == 0 {
                    sample(at..at, &[px(20.0); 8])?
                } else {
                    sample(at..at + 8, &[])?
                };
                if cycle >= 20 {
                    times.push(elapsed);
                }
            }
        }
        for (label, times) in ["insert", "remove"].into_iter().zip(samples) {
            report(count, &format!("{location} {label} 8"), &times);
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    for count in [10_000, 100_000, 500_000] {
        run(count, Workload::InsertRemove)?;
        for reserve_pages in [false, true, true, false] {
            run(count, Workload::History { reserve_pages })?;
        }
    }
    Ok(())
}
