use super::*;
use crate::{
    damage::RetainedPixels,
    glyph_masks::GlyphMasks,
    pixels::{Clip, RasterScale, Surface},
};

const BEFORE: &[&str] = &["Previous item"];
const DETAIL: &str = "Hello 世界！Rust GUI 123";

#[test]
fn viewport_frames_match_full_draws_through_edges_resizes_and_edits() {
    let tail = ["Short", "Hello 世界！Rust GUI 123", "Other text"].repeat(20);
    let mut list = DisclosureLayout::new(BEFORE, &tail).unwrap();
    let scale = RasterScale(1.5);
    let mut retained: Option<RetainedPixels> = None;
    let mut masks = GlyphMasks::new(64, 32 * 1024);
    let mut saved = None;
    // The title starts at y=29. A row crossing the bottom still needs its command.
    for (width, height, open) in [
        (240, 29, false),
        (240, 30, false),
        (240, 40, true),
        (132, 158, true),
        (132, 158, false),
        (240, 2, false),
        (240, 160, false),
    ] {
        if list.detail.is_some() != open {
            list.set_detail(
                if open { "Hide details" } else { "Show details" },
                open.then_some(DETAIL),
            )
            .unwrap();
        }
        let size = Size { width, height };
        let frame = list.frame(size, false).unwrap();
        let mut oracle = DisclosureLayout::new(BEFORE, &tail).unwrap();
        if open {
            oracle.set_detail("Hide details", Some(DETAIL)).unwrap();
        }
        oracle
            .scene
            .run(Query::Exact(width), Some(size), true)
            .unwrap();
        let full = oracle.collect_frame(0, u32::MAX).unwrap();
        assert_eq!(frame.geometry, full.geometry);
        assert_eq!(
            frame.draws.len(),
            full.draws
                .iter()
                .filter(|draw| draw.origin.y < height)
                .count()
        );
        assert_eq!(list.published_title().is_some(), height > 29);
        let physical = Size {
            width: (width * 3).div_ceil(2),
            height: (height * 3).div_ceil(2),
        };
        let clip = Clip::canvas(physical);
        let expected = Surface::raster(&full, physical, clip, (0, 0), scale, None).unwrap();
        let pixels = match &mut retained {
            Some(pixels) => {
                pixels
                    .update(frame.clone(), physical, clip, &mut masks)
                    .unwrap();
                pixels
            }
            None => retained.insert(
                RetainedPixels::new_scaled(frame.clone(), physical, clip, scale, &mut masks)
                    .unwrap(),
            ),
        };
        assert_eq!(pixels.surface().rgba, expected.rgba);
        if height == 158 && open {
            saved = Some((frame, physical, expected.rgba));
        }
    }
    drop(list);
    let (held, physical, expected) = saved.unwrap();
    assert_eq!(
        Surface::raster(&held, physical, Clip::canvas(physical), (0, 0), scale, None)
            .unwrap()
            .rgba,
        expected
    );
}

#[test]
#[ignore = "release timing experiment"]
fn measure_snapshot_collection() {
    use std::{hint::black_box, time::Instant};

    for row_count in [3, 1024] {
        let labels: Vec<_> = (3..=row_count).map(|n| format!("Item {n}")).collect();
        let tail: Vec<_> = labels.iter().map(String::as_str).collect();
        let size = Size {
            width: 240,
            height: 160,
        };
        let mut lists = [u32::MAX, size.height].map(|_| {
            let mut list = DisclosureLayout::new(BEFORE, &tail).unwrap();
            list.frame(size, false).unwrap();
            list
        });
        let mut samples = [[Vec::new(), Vec::new()], [Vec::new(), Vec::new()]];
        for batch in 0..8 {
            for offset in 0..2 {
                let mode = (batch + offset) % 2;
                let list = &mut lists[mode];
                let bottom = [u32::MAX, size.height][mode];
                let mut elapsed = [0.0; 2];
                for step in 0..80 {
                    let start = Instant::now();
                    let open = step % 2 == 0;
                    list.set_detail(
                        if open { "Hide details" } else { "Show details" },
                        open.then_some(DETAIL),
                    )
                    .unwrap();
                    list.scene
                        .run(Query::Exact(size.width), Some(size), false)
                        .unwrap();
                    let laid_out = Instant::now();
                    let frame = Rc::new(list.collect_frame(step, bottom).unwrap());
                    list.published = Some(black_box(frame));
                    elapsed[0] += laid_out.elapsed().as_secs_f64();
                    elapsed[1] += start.elapsed().as_secs_f64();
                }
                if batch > 0 {
                    for (samples, seconds) in samples[mode].iter_mut().zip(elapsed) {
                        samples.push(seconds * 1e6 / 80.0);
                    }
                }
            }
        }
        for (i, stages) in samples.into_iter().enumerate() {
            let frame = lists[i].published.as_ref().unwrap();
            for (stage, mut samples) in ["snapshot", "edit through snapshot"]
                .into_iter()
                .zip(stages)
            {
                samples.sort_by(f64::total_cmp);
                eprintln!(
                    "{row_count} rows, {} draws, {} bytes, {stage}: {:.2} us",
                    frame.draws.len(),
                    frame.shallow_bytes(),
                    samples[3]
                );
            }
        }
    }
}
