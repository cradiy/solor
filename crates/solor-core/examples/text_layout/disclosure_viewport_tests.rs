use super::*;
use crate::{
    damage::RetainedPixels,
    glyph_masks::GlyphMasks,
    pixels::{Clip, RasterScale, Surface},
};

const BEFORE: &[&str] = &["Previous item"];
const DETAIL: &str = "Hello 世界！Rust GUI 123";

#[test]
fn assigned_root_only_publishes_measurement_evidence_when_queried() {
    use crate::layout::validation::CheckTarget;

    let size = Size {
        width: 132,
        height: 160,
    };
    let query = Query::Exact(size.width);
    let mut list = DisclosureLayout::new(BEFORE, &["Short"]).unwrap();
    let root = list.scene.root();
    for open in [false, true, false] {
        if list.detail.is_some() != open {
            list.set_detail(
                if open { "Hide details" } else { "Show details" },
                open.then_some(DETAIL),
            )
            .unwrap();
        }
        let previous = list.scene.node_counts(root);
        let frame = list.frame(size, false).unwrap();
        assert_eq!(list.scene.node_counts(root).queries, previous.queries);
        assert!(
            list.scene
                .capture_dependencies(root, CheckTarget::Measure(query))
                .unwrap()
                .is_none()
        );
        assert!(
            list.scene
                .capture_dependencies(root, CheckTarget::Layout(size))
                .unwrap()
                .is_some()
        );

        // The allocation is identical, but a caller now needs a preferred size.
        assert_eq!(list.scene.run(query, Some(size), false).unwrap(), 0);
        assert_eq!(list.scene.node_counts(root).measures, previous.measures + 1);
        assert!(
            list.scene
                .capture_dependencies(root, CheckTarget::Measure(query))
                .unwrap()
                .is_some()
        );
        assert!(frame.same_content(&list.collect_frame(0, size.height).unwrap()));
    }
}

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
        (13, 185, true),
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
        let mut oracle =
            DisclosureLayout::with_engine(BEFORE, &tail, Engine::new(), false).unwrap();
        if open {
            oracle.set_detail("Hide details", Some(DETAIL)).unwrap();
        }
        oracle
            .scene
            .run(Query::Exact(width), Some(size), true)
            .unwrap();
        let full = oracle.collect_frame(0, u32::MAX).unwrap();
        assert_eq!(
            frame.geometry,
            oracle
                .scene
                .snapshot()
                .into_iter()
                .collect::<Option<Vec<_>>>()
                .unwrap()
        );
        assert_eq!(
            frame.draws.len(),
            full.draws
                .iter()
                .filter(|draw| draw.origin.y < height)
                .count()
        );
        assert_eq!(
            list.published_title().is_some(),
            full.draws[BEFORE.len()].origin.y < height
        );
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
fn moving_the_tail_keeps_row_measurements_and_local_positions() {
    let mut list = DisclosureLayout::new(BEFORE, &["Short", DETAIL, "Last"]).unwrap();
    let size = Size {
        width: 132,
        height: 300,
    };
    let group = list.tail_group.unwrap();
    let closed = list.frame(size, false).unwrap();
    let group_y = list.geometry(group).unwrap().origin.y;
    let rows = list
        .after
        .iter()
        .map(|&id| (id, list.geometry(id).unwrap(), list.scene.node_counts(id)))
        .collect::<Vec<_>>();
    list.set_detail("Hide details", Some(DETAIL)).unwrap();
    let open = list.frame(size, false).unwrap();
    assert!(list.geometry(group).unwrap().origin.y > group_y);
    for (id, local, counts) in rows {
        assert_eq!(list.geometry(id).unwrap(), local);
        assert_eq!(list.scene.node_counts(id), counts);
        let before = closed.draws.iter().find(|draw| draw.node == id).unwrap();
        let after = open.draws.iter().find(|draw| draw.node == id).unwrap();
        assert_eq!(
            after.origin.y - before.origin.y,
            list.geometry(group).unwrap().origin.y - group_y
        );
        assert!(Rc::ptr_eq(&before.paragraph, &after.paragraph));
    }
}

#[test]
#[ignore = "release timing experiment"]
fn measure_snapshot_collection() {
    use std::{hint::black_box, time::Instant};

    for row_count in [3, 4, 1024] {
        let labels: Vec<_> = (3..=row_count).map(|n| format!("Item {n}")).collect();
        let tail: Vec<_> = labels.iter().map(String::as_str).collect();
        let size = Size {
            width: 240,
            height: 160,
        };
        let bottoms = [u32::MAX, size.height, size.height, size.height];
        let mut lists = [false, false, false, tail.len() > 1].map(|grouped| {
            let mut list =
                DisclosureLayout::with_engine(BEFORE, &tail, Engine::new(), grouped).unwrap();
            list.frame(size, false).unwrap();
            list
        });
        let mut samples = bottoms.map(|_| [Vec::new(), Vec::new()]);
        for batch in 0..8 {
            for offset in 0..lists.len() {
                let mode = (batch + offset) % lists.len();
                let list = &mut lists[mode];
                let bottom = bottoms[mode];
                let mut elapsed = [0.0; 2];
                for step in 0..80 {
                    let start = Instant::now();
                    let open = step % 2 == 0;
                    list.set_detail(
                        if open { "Hide details" } else { "Show details" },
                        open.then_some(DETAIL),
                    )
                    .unwrap();
                    if mode >= 2 {
                        list.scene.run_assigned(size, false).unwrap();
                    } else {
                        list.scene
                            .run(Query::Exact(size.width), Some(size), false)
                            .unwrap();
                    }
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
        assert!(
            lists[1]
                .published
                .as_ref()
                .unwrap()
                .same_content(lists[2].published.as_ref().unwrap())
        );
        assert!(
            lists[2]
                .published
                .as_ref()
                .unwrap()
                .same_content(lists[3].published.as_ref().unwrap())
        );
        for (i, stages) in samples.into_iter().enumerate() {
            let root = [
                "measured flat",
                "measured flat",
                "assigned flat",
                "assigned grouped",
            ][i];
            let frame = lists[i].published.as_ref().unwrap();
            for (stage, mut samples) in ["snapshot", "edit through snapshot"]
                .into_iter()
                .zip(stages)
            {
                samples.sort_by(f64::total_cmp);
                eprintln!(
                    "{row_count} rows, {root} root, {} draws, {} bytes, {stage}: {:.2} us",
                    frame.draws.len(),
                    frame.shallow_bytes(),
                    samples[3]
                );
            }
        }
    }
}

#[test]
#[ignore = "release timing experiment"]
fn measure_tail_resize() {
    use std::{hint::black_box, time::Instant};

    let tail = ["Short", DETAIL].repeat(511);
    let mut lists = [false, true].map(|grouped| {
        DisclosureLayout::with_engine(BEFORE, &tail, Engine::new(), grouped).unwrap()
    });
    let mut samples = [Vec::new(), Vec::new()];
    for batch in 0..8 {
        for offset in 0..2 {
            let mode = (batch + offset) % 2;
            let start = Instant::now();
            for step in 0..20 {
                let width = if step % 2 == 0 { 84 } else { 240 };
                black_box(
                    lists[mode]
                        .frame(Size { width, height: 160 }, false)
                        .unwrap(),
                );
            }
            if batch > 0 {
                samples[mode].push(start.elapsed().as_secs_f64() * 1e6 / 20.0);
            }
        }
        assert!(
            lists[0]
                .published
                .as_ref()
                .unwrap()
                .same_content(lists[1].published.as_ref().unwrap())
        );
    }
    for (name, mut times) in ["flat", "grouped"].into_iter().zip(samples) {
        times.sort_by(f64::total_cmp);
        eprintln!(
            "1024 rows, {name} resize through snapshot: {:.2} us",
            times[3]
        );
    }
}
