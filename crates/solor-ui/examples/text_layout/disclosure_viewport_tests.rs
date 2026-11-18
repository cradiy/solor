use super::*;
use crate::{
    damage::RetainedPixels,
    glyph_masks::GlyphMasks,
    pixels::{Clip, RasterScale, Surface},
};
use solor_core::px;

const BEFORE: &[&str] = &["Previous item"];
const DETAIL: &str = "Hello 世界！Rust GUI 123";

// The independent oracle has a flat tail. Compare every logical row, including offscreen rows.
fn flat_geometry(list: &DisclosureLayout, frame: &Frame) -> Vec<Geometry> {
    let mut rows = frame.geometry.to_vec();
    if list.tail_group.is_some() {
        let index = 3 + list.before.len();
        let group = rows.remove(index);
        for row in &mut rows[index..index + list.after.len()] {
            row.origin = translated(row.origin, group.origin).unwrap();
        }
    }
    rows
}

fn assert_matches_flat(grouped: &DisclosureLayout, flat: &DisclosureLayout) {
    let frame = grouped.published.as_ref().unwrap();
    let reference = flat.published.as_ref().unwrap();
    assert_eq!(flat_geometry(grouped, frame), reference.geometry.as_ref());
    assert!(frame.same_drawing(reference));
}

#[cfg(feature = "layout-audit")]
#[test]
fn assigned_root_only_publishes_measurement_evidence_when_queried() {
    use crate::layout::validation::CheckTarget;

    let size = Size {
        width: px(132.0),
        height: px(160.0),
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
        let previous = list.scene.node_counts(root).unwrap();
        let frame = list.frame(size, px(0.0), false).unwrap();
        assert_eq!(
            list.scene.node_counts(root).unwrap().queries,
            previous.queries
        );
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
        assert_eq!(
            list.scene.node_counts(root).unwrap().measures,
            previous.measures + 1
        );
        assert!(
            list.scene
                .capture_dependencies(root, CheckTarget::Measure(query))
                .unwrap()
                .is_some()
        );
        assert!(frame.same_content(&list.collect_frame(0, px(0.0), size.height).unwrap()));
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
        let size = Size {
            width: px(width as f32),
            height: px(height as f32),
        };
        let frame = list.frame(size, px(0.0), false).unwrap();
        let mut oracle =
            DisclosureLayout::with_engine(BEFORE, &tail, fixture_engine(), false).unwrap();
        if open {
            oracle.set_detail("Hide details", Some(DETAIL)).unwrap();
        }
        oracle
            .scene
            .run(Query::Exact(size.width), Some(size), true)
            .unwrap();
        let full = oracle.collect_frame(0, px(0.0), px(f32::MAX)).unwrap();
        assert_eq!(
            flat_geometry(&list, &frame),
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
                .filter(|draw| draw.origin.y < size.height)
                .count()
        );
        assert_eq!(
            list.published_title().is_some(),
            full.draws[BEFORE.len()].origin.y < size.height
        );
        let physical = scale.buffer_size(size).unwrap();
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
        width: px(132.0),
        height: px(300.0),
    };
    let group = list.tail_group.unwrap();
    let closed = list.frame(size, px(0.0), false).unwrap();
    let group_y = list.geometry(group).unwrap().origin.y;
    let rows = list
        .after
        .iter()
        .map(|&id| {
            (
                id,
                list.geometry(id).unwrap(),
                list.scene.node_counts(id).unwrap(),
            )
        })
        .collect::<Vec<_>>();
    list.set_detail("Hide details", Some(DETAIL)).unwrap();
    let open = list.frame(size, px(0.0), false).unwrap();
    assert!(list.geometry(group).unwrap().origin.y > group_y);
    for (id, local, counts) in rows {
        assert_eq!(list.geometry(id).unwrap(), local);
        assert_eq!(list.scene.node_counts(id).unwrap(), counts);
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
            width: px(240.0),
            height: px(160.0),
        };
        let bottoms = [px(f32::MAX), size.height, size.height, size.height];
        let mut lists = [false, false, false, tail.len() > 1].map(|grouped| {
            let mut list =
                DisclosureLayout::with_engine(BEFORE, &tail, fixture_engine(), grouped).unwrap();
            list.frame(size, px(0.0), false).unwrap();
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
                    let frame = Rc::new(list.collect_frame(step, px(0.0), bottom).unwrap());
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
        assert_matches_flat(&lists[3], &lists[2]);
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
        DisclosureLayout::with_engine(BEFORE, &tail, fixture_engine(), grouped).unwrap()
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
                        .frame(
                            Size {
                                width: px(width as f32),
                                height: px(160.0),
                            },
                            px(0.0),
                            false,
                        )
                        .unwrap(),
                );
            }
            if batch > 0 {
                samples[mode].push(start.elapsed().as_secs_f64() * 1e6 / 20.0);
            }
        }
        assert_matches_flat(&lists[1], &lists[0]);
    }
    for (name, mut times) in ["flat", "grouped"].into_iter().zip(samples) {
        times.sort_by(f64::total_cmp);
        eprintln!(
            "1024 rows, {name} resize through snapshot: {:.2} us",
            times[3]
        );
    }
}

#[test]
fn scroll_snapshots_share_geometry_and_find_both_edges_of_each_row() {
    let size = Size {
        width: px(88.0),
        height: px(1.0),
    };
    let mut list =
        DisclosureLayout::new(&["First", DETAIL, "Third"], &["Short", DETAIL, "Last"]).unwrap();
    list.set_detail("Hide details", Some(DETAIL)).unwrap();
    let first = list.frame(size, px(0.0), false).unwrap();
    let item_y = list.geometry(list.item).unwrap().origin.y;
    let tail_y = list.geometry(list.tail_group.unwrap()).unwrap().origin.y;
    let rows: Vec<_> = list
        .before
        .iter()
        .map(|&id| (id, px(0.0)))
        .chain([list.title, list.detail.unwrap()].map(|id| (id, item_y)))
        .chain(list.after.iter().map(|&id| (id, tail_y)))
        .collect();
    for (id, parent_y) in rows {
        let row = list.geometry(id).unwrap();
        let start = parent_y + row.origin.y;
        let end = start + row.metrics.size.height;
        for y in [start, end - px(1.0)] {
            let frame = list.frame(size, y, false).unwrap();
            assert!(Rc::ptr_eq(&frame.geometry, &first.geometry));
            assert_eq!(frame.draws.len(), 1);
            assert_eq!(frame.draws[0].node, id);
        }
        assert!(list.frame(size, end, false).unwrap().draws.is_empty());
    }

    list.set_detail("Show details", None).unwrap();
    let collapsed = list.frame(size, px(0.0), false).unwrap();
    assert!(!Rc::ptr_eq(&first.geometry, &collapsed.geometry));
    assert_eq!(first.geometry.len(), collapsed.geometry.len() + 1);
    assert!(first.geometry[0].content.height > collapsed.geometry[0].content.height);
}

#[test]
fn scrolled_frames_match_crops_of_the_full_document() {
    let tail = ["Hello", DETAIL, "Short"].repeat(6);
    let mut list = DisclosureLayout::new(BEFORE, &tail).unwrap();
    let mut masks = GlyphMasks::new(64, 32 * 1024);
    let mut pixels: Option<RetainedPixels> = None;
    let scale = RasterScale(1.5);
    let mut held = None;

    for (width, height, open) in [
        (134, 62, false),
        (100, 78, true),
        (100, 78, false),
        (240, 800, false),
    ] {
        if open != list.detail.is_some() {
            list.set_detail(
                if open { "Hide details" } else { "Show details" },
                open.then_some(DETAIL),
            )
            .unwrap();
        }
        let size = Size {
            width: px(width as f32),
            height: px(height as f32),
        };
        let mut flat =
            DisclosureLayout::with_engine(BEFORE, &tail, fixture_engine(), false).unwrap();
        if open {
            flat.set_detail("Hide details", Some(DETAIL)).unwrap();
        }
        flat.scene
            .run(Query::Exact(size.width), Some(size), true)
            .unwrap();
        let full = flat.collect_frame(0, px(0.0), px(f32::MAX)).unwrap();
        let content_height = full.geometry[0].content.height;
        let max_scroll = content_height.saturating_sub(size.height);
        let physical = crate::pixels::DeviceSize {
            width: width * 3 / 2,
            height: height * 3 / 2,
        };
        let clip = Clip::canvas(physical);
        let canvas = crate::pixels::DeviceSize {
            width: physical.width,
            height: (content_height.max(size.height).get() * 1.5).ceil() as u32,
        };
        let document =
            Surface::raster(&full, canvas, Clip::canvas(canvas), (0, 0), scale, None).unwrap();
        let mut counts = None;
        for request in [
            f32::MAX,
            0.0,
            17.0,
            26.0,
            29.0,
            content_height.get() / 2.0,
            f32::MAX,
        ]
        .map(solor_core::px)
        {
            let frame = list.frame(size, request, false).unwrap();
            let top = request.min(max_scroll);
            assert_eq!(frame.scroll_y, top);
            assert_eq!(flat_geometry(&list, &frame), full.geometry.as_ref());
            assert_eq!(
                frame.draws.iter().map(|d| d.origin).collect::<Vec<_>>(),
                full.draws
                    .iter()
                    .filter(
                        |d| d.origin.y + d.allocated.height > top && d.origin.y < top + size.height
                    )
                    .map(|d| d.origin)
                    .collect::<Vec<_>>()
            );
            let current = list.scene.counts();
            assert_eq!(current, *counts.get_or_insert(current));
            let retained = match &mut pixels {
                Some(p) => {
                    p.update(frame.clone(), physical, clip, &mut masks).unwrap();
                    p
                }
                None => pixels.insert(
                    RetainedPixels::new_scaled(frame.clone(), physical, clip, scale, &mut masks)
                        .unwrap(),
                ),
            };
            let stride = physical.width as usize * 4;
            let start = scale.edge(f64::from(top)) as usize * stride;
            let expected = &document.rgba[start..start + physical.height as usize * stride];
            assert!(
                retained.surface().rgba == expected,
                "width {width}, top {top:?}, open {open}"
            );
            assert!(Rc::ptr_eq(&frame, &list.frame(size, top, false).unwrap()));
            if request == px(17.0) && open {
                held = Some((frame, physical, expected.to_vec()));
            }
        }
    }
    drop(list);
    let (frame, size, expected) = held.unwrap();
    assert!(
        Surface::raster(&frame, size, Clip::canvas(size), (0, 0), scale, None)
            .unwrap()
            .rgba
            == expected
    );
}

#[test]
#[ignore = "release timing experiment"]
fn measure_scroll_positions() {
    use std::{hint::black_box, time::Instant};
    let labels: Vec<_> = (3..=1024).map(|n| format!("Item {n}")).collect();
    let tail: Vec<_> = labels.iter().map(String::as_str).collect();
    let size = Size {
        width: px(240.0),
        height: px(160.0),
    };
    let mut list = DisclosureLayout::new(BEFORE, &tail).unwrap();
    let first = list.frame(size, px(0.0), false).unwrap();
    let end = first.geometry[0].content.height - size.height;
    let counts = list.scene.counts();
    let positions = [px(0.0), end / 2.0, end - px(1.0)];
    let mut samples = positions.map(|_| Vec::new());
    for batch in 0..8 {
        for offset in 0..3 {
            let mode = (batch + offset) % 3;
            let start = Instant::now();
            for step in 0..200 {
                let top = positions[mode] + px((step % 2) as f32);
                black_box(list.frame(size, top, false).unwrap());
            }
            if batch > 0 {
                samples[mode].push(start.elapsed().as_secs_f64() * 1e6 / 200.0);
            }
        }
    }
    assert_eq!(list.scene.counts(), counts);
    for (position, mut times) in ["top", "middle", "end"].into_iter().zip(samples) {
        times.sort_by(f64::total_cmp);
        eprintln!("scroll {position}: {:.2} us/frame", times[3]);
    }
}
