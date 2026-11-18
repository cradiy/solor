use solor_core::px;
use std::hint::black_box;
use std::time::{Duration, Instant};

use crate::frame::{Mode, Presentation, STYLE};
use crate::layout::Size;
use solor_ui::text::TextStyle;

const SAMPLE: &str = "Solor 中文排版，文字换行与基线。Hello 世界！Rust GUI 123";

fn change(view: &mut Presentation, rows: usize, kind: usize, step: usize) -> Size {
    let index = step % rows;
    match kind {
        0 => view
            .set_color(index, [(step % 251) as u8, 40, 80, 255])
            .unwrap(),
        1 => view
            .set_content(index, &format!("{SAMPLE} {}", step % 10))
            .unwrap(),
        2 => view.set_shift(index, (step % 17) as u32).unwrap(),
        3 => view
            .set_style(
                index,
                if (step / rows).is_multiple_of(2) {
                    TextStyle {
                        size: solor_ui::px(18.0),
                        line_height: solor_ui::px(28.0),
                    }
                } else {
                    STYLE
                },
            )
            .unwrap(),
        _ => {}
    }
    Size {
        width: if kind == 4 && step.is_multiple_of(2) {
            px(160.0)
        } else {
            px(240.0)
        },
        height: px(50_000.0),
    }
}

pub fn verify() {
    verify_mode(Mode::Normal);
    #[cfg(feature = "layout-audit")]
    verify_mode(Mode::Pruned);
}

fn verify_mode(mode: Mode) {
    let mut incremental = Presentation::new(8, SAMPLE, true);
    let mut reference = Presentation::new(8, SAMPLE, false);
    for step in 0..96 {
        let kind = (step / 3) % 5;
        let size = change(&mut incremental, 8, kind, step);
        change(&mut reference, 8, kind, step);
        let actual = incremental.update(size, mode).unwrap();
        let expected = reference.update(size, Mode::Forced).unwrap();
        assert!(
            actual.frame.same_content(&expected.frame),
            "frame at step {step}"
        );
        if step % 13 == 0 {
            incremental.release_caches();
        }
    }
    println!(
        "frame oracle: 96 updates match geometry, absolute origins, glyphs, colors and font data"
    );
}

fn median(mut samples: [Duration; 7], iterations: u32) -> f64 {
    samples.sort_unstable();
    samples[3].as_secs_f64() * 1e6 / f64::from(iterations)
}

pub fn run() {
    for rows in [64, 512] {
        for (kind, name) in ["color", "content", "motion", "style", "width", "idle"]
            .iter()
            .enumerate()
        {
            let mut views = [
                Presentation::new(rows, SAMPLE, true),
                Presentation::new(rows, SAMPLE, true),
            ];
            for (mode, view) in views.iter_mut().enumerate() {
                for step in 0..20 {
                    let size = change(view, rows, kind, step);
                    if mode == 0 {
                        view.layout_only(size).unwrap();
                    } else {
                        view.update(size, Mode::Normal).unwrap();
                    }
                }
            }
            let before = views
                .each_ref()
                .map(|v| (v.layout_counts(), v.shape_count()));
            let mut timings = [[Duration::ZERO; 2]; 7];
            let mut resolved = 0;
            let mut published = 0;
            let mut skipped = 0;
            for (sample, timing) in timings.iter_mut().enumerate() {
                for offset in 0..2 {
                    let mode = (sample + offset) % 2;
                    let start = Instant::now();
                    for step in 0..31 {
                        let size = change(&mut views[mode], rows, kind, 20 + sample * 31 + step);
                        if mode == 0 {
                            views[mode].layout_only(size).unwrap();
                        } else {
                            let report = views[mode].update(size, Mode::Normal).unwrap();
                            resolved += report.resolved;
                            published += usize::from(report.published);
                            skipped += usize::from(report.skipped_parent);
                            black_box(report);
                        }
                    }
                    timing[mode] = start.elapsed();
                }
            }
            let size = change(&mut views[0], rows, kind, 237);
            change(&mut views[1], rows, kind, 237);
            let a = views[0].update(size, Mode::Normal).unwrap().frame;
            let b = views[1].update(size, Mode::Normal).unwrap().frame;
            assert!(a.same_content(&b));
            // Resolution/publication counters cover only the timed updates.
            println!(
                "frame/{rows}/{name}: layout_us={:.2} frame_us={:.2} resolved_per_update={:.2} published={published} skipped={skipped} frame_shallow_bytes={}",
                median(timings.map(|t| t[0]), 31),
                median(timings.map(|t| t[1]), 31),
                resolved as f64 / 217.0,
                b.shallow_bytes()
            );
            if kind == 0 {
                assert_eq!(views[1].layout_counts(), before[1].0);
                assert_eq!(views[1].shape_count(), before[1].1);
            }
        }
    }
    println!("begin frame-warmed");
    let mut view = Presentation::new(64, SAMPLE, true);
    for step in 0..100 {
        let size = change(&mut view, 64, step % 5, step);
        view.update(size, Mode::Normal).unwrap();
    }
    println!("end frame-warmed");
    println!("begin frame-color-update");
    view.set_color(0, [255, 0, 0, 255]).unwrap();
    view.update(
        Size {
            width: px(240.0),
            height: px(50_000.0),
        },
        Mode::Normal,
    )
    .unwrap();
    println!("end frame-color-update");
    println!("begin frame-idle-update");
    black_box(
        view.update(
            Size {
                width: px(240.0),
                height: px(50_000.0),
            },
            Mode::Normal,
        )
        .unwrap(),
    );
    println!("end frame-idle-update");
    println!("begin frame-steady");
    for step in 100..1100 {
        let size = change(&mut view, 64, step % 5, step);
        view.update(size, Mode::Normal).unwrap();
    }
    println!("end frame-steady");
    println!("begin frame-steady-again");
    for step in 1100..2100 {
        let size = change(&mut view, 64, step % 5, step);
        view.update(size, Mode::Normal).unwrap();
    }
    println!("end frame-steady-again");
    println!("begin frame-release-caches");
    view.release_caches();
    println!("end frame-release-caches");
    let retained = view.current().unwrap();
    println!("begin frame-drop-scene");
    drop(view);
    println!("end frame-drop-scene");
    black_box(&retained);
    println!("begin frame-drop-last-snapshot");
    drop(retained);
    println!("end frame-drop-last-snapshot");
}
