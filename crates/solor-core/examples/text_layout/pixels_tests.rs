use crate::frame::{Frame, Mode, Presentation};
use crate::layout::Size;
use crate::pixels::{Clip, Surface};
use std::rc::Rc;

const SIZE: Size = Size {
    width: 128,
    height: 80,
};
const WHITE: &[u8] = &[255; 4];

fn render(frame: &Frame) -> Surface {
    Surface::draw(frame, SIZE, Clip::canvas(SIZE)).unwrap()
}
fn at(surface: &Surface, x: usize, y: usize) -> &[u8] {
    let i = (y * surface.size.width as usize + x) * 4;
    &surface.rgba[i..i + 4]
}

#[test]
fn same_geometry_replaces_only_target_ink_and_old_snapshot_replays() {
    let mut view = Presentation::new(2, "中文", true);
    let old = view.update(SIZE, Mode::Pruned).unwrap().frame;
    let before = render(&old);
    view.set_content(0, "文字").unwrap();
    let new = view.update(SIZE, Mode::Pruned).unwrap();
    assert!(new.skipped_parent);
    assert_eq!(old.geometry, new.frame.geometry);
    drop(view);
    let after = render(&new.frame);
    let sibling_start = old.draws[1].origin.y as usize * SIZE.width as usize * 4;
    assert_ne!(before.rgba[..sibling_start], after.rgba[..sibling_start]);
    assert_eq!(before.rgba[sibling_start..], after.rgba[sibling_start..]);
    assert!(
        before.rgba[sibling_start..]
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p != WHITE)
    );
    assert_eq!(before.rgba, render(&old).rgba);
}

#[test]
fn motion_and_scissor_match_translation_and_crop_of_complete_pixels() {
    let mut view = Presentation::new(1, "宽度 Hello", true);
    let before = render(&view.update(SIZE, Mode::Pruned).unwrap().frame);
    view.set_shift(0, 9).unwrap();
    let frame = view.update(SIZE, Mode::Pruned).unwrap().frame;
    let shifted = render(&frame);
    let clip = Clip {
        left: 15,
        top: 9,
        right: 44,
        bottom: 17,
    };
    let cropped = Surface::draw(&frame, SIZE, clip).unwrap();
    let mut kept = 0;
    let mut removed = 0;
    for y in 0..SIZE.height as usize {
        for x in 0..SIZE.width as usize {
            let original = if x >= 9 { at(&before, x - 9, y) } else { WHITE };
            assert_eq!(at(&shifted, x, y), original);
            let inside = (15..44).contains(&x) && (9..17).contains(&y);
            assert_eq!(at(&cropped, x, y), if inside { original } else { WHITE });
            if original != WHITE {
                if inside { kept += 1 } else { removed += 1 }
            }
        }
    }
    assert!(kept > 0 && removed > 0);
}

#[test]
fn baselines_place_identical_lines_and_descenders_below_capitals() {
    let mut view = Presentation::new(1, "Hg\nHg", true);
    let frame = view.update(SIZE, Mode::Normal).unwrap().frame;
    let draw = &frame.draws[0];
    let first = &draw.paragraph.lines[0];
    let baseline = draw.origin.y as usize + first.baseline.round() as usize;
    let g_start = draw.origin.x as usize + first.glyphs[1].x.round() as usize;
    let image = render(&frame);
    let mut descender = false;
    let mut cap_ink = false;
    for y in draw.origin.y as usize..draw.origin.y as usize + 24 {
        for x in 0..SIZE.width as usize {
            let pixel = at(&image, x, y);
            assert_eq!(pixel, at(&image, x, y + 24));
            if pixel != WHITE && x < g_start {
                assert!(y < baseline);
                cap_ink = true;
            }
            if pixel != WHITE && x >= g_start && y >= baseline {
                descender = true;
            }
        }
    }
    assert!(cap_ink && descender);
}

#[test]
fn coverage_and_opacity_compose_commands_in_order_with_leaf_clipping() {
    let mut view = Presentation::new(2, "中", true);
    view.set_color(0, [0, 0, 0, 255]).unwrap();
    view.set_color(1, [255, 0, 0, 128]).unwrap();
    let mut frame = view.update(SIZE, Mode::Normal).unwrap().frame;
    drop(view);
    let black = render(&frame);
    let frame = Rc::get_mut(&mut frame).unwrap();
    frame.draws[1].origin = frame.draws[0].origin;
    frame.draws[1].allocated.width = 10;
    let edge = frame.draws[1].origin.x as usize + 10;
    let overlaid = render(frame);
    let mut covered = false;
    let mut clipped = false;
    for y in 0..26 {
        for x in 0..SIZE.width as usize {
            let gray = u32::from(at(&black, x, y)[0]);
            let coverage = 255 - gray;
            let alpha = if x < edge {
                (coverage * 128 + 127) / 255
            } else {
                0
            };
            let other = (gray * (255 - alpha) + 127) / 255;
            assert_eq!(
                at(&overlaid, x, y),
                &[(alpha + other) as u8, other as u8, other as u8, 255]
            );
            covered |= x < edge && coverage > 0;
            clipped |= x >= edge && coverage > 0;
        }
    }
    assert!(covered && clipped);
}

#[test]
fn cached_masks_match_every_frame_through_style_motion_color_and_eviction() {
    use crate::glyph_masks::GlyphMasks;
    use crate::text::Style;
    let mut view = Presentation::new(2, "Hello 世界！", true);
    let old = view.update(SIZE, Mode::Normal).unwrap().frame;
    let mut cache = GlyphMasks::new(12, 2048);
    for i in 0..24 {
        view.set_color(0, [0, 140, 80, 128]).unwrap();
        view.set_shift(1, i % 16).unwrap();
        view.set_content(0, if i % 3 == 0 { "文字" } else { "中文" })
            .unwrap();
        view.set_style(
            0,
            Style {
                size: if i % 2 == 0 { 14 } else { 20 },
                line_height: 24,
            },
        )
        .unwrap();
        let frame = view.update(SIZE, Mode::Pruned).unwrap().frame;
        let clip = Clip {
            left: i64::from(i / 2),
            top: 0,
            right: 100,
            bottom: 63,
        };
        let actual = Surface::draw_cached(&frame, SIZE, clip, &mut cache).unwrap();
        assert_eq!(actual.rgba, Surface::draw(&frame, SIZE, clip).unwrap().rgba);
        let (entries, bytes) = cache.usage();
        assert!(entries <= 12 && bytes <= 2048);
    }
    drop(view);
    assert!(cache.hits > 0 && cache.misses > 12);
    assert_eq!(
        Surface::draw_cached(&old, SIZE, Clip::canvas(SIZE), &mut cache)
            .unwrap()
            .rgba,
        render(&old).rgba
    );
    cache.release();
    assert_eq!(cache.usage(), (0, 0));
}

#[test]
fn budgets_retain_small_masks_bypass_large_masks_and_rebind_fonts() {
    use crate::glyph_masks::GlyphMasks;
    use swash::scale::image::Image;
    let mut cache = GlyphMasks::new(3, 32);
    cache.bind(b"first");
    let mut scratch = Image::new();
    for glyph in 1..=3 {
        cache
            .resolve(16, glyph, &mut scratch, |m| {
                m.data = vec![glyph as u8; 16];
                Ok(())
            })
            .unwrap();
    }
    assert_eq!(cache.usage(), (2, 32));
    let large = cache
        .resolve(16, 7, &mut scratch, |m| {
            m.data = vec![42; 64];
            Ok(())
        })
        .unwrap();
    assert_eq!(large.data.len(), 64);
    assert_eq!(cache.usage(), (2, 32));
    let hit = cache
        .resolve(16, 3, &mut scratch, |_| panic!("expected hit"))
        .unwrap();
    assert_eq!(hit.data[0], 3);
    cache
        .resolve(16, 8, &mut scratch, |m| {
            m.data.resize(8, 1);
            Ok(())
        })
        .unwrap();
    assert_eq!(cache.usage(), (2, 24));
    cache
        .resolve(16, 8, &mut scratch, |_| {
            panic!("small mask should be retained")
        })
        .unwrap();
    cache.bind(b"other");
    assert_eq!(cache.usage(), (0, 0));
    assert!(
        cache
            .resolve(16, 3, &mut scratch, |_| Err(std::io::Error::other(
                "missing outline"
            )))
            .is_err()
    );
    assert_eq!(cache.usage(), (0, 0));
}

#[test]
fn retained_pixels_match_every_edit_and_replay_across_viewports() {
    use crate::{damage::RetainedPixels, glyph_masks::GlyphMasks};
    let mut view = Presentation::new(3, "文字", true);
    let first = view.update(SIZE, Mode::Normal).unwrap().frame;
    let mut masks = GlyphMasks::new(3, 1024);
    let mut retained =
        RetainedPixels::new(first.clone(), SIZE, Clip::canvas(SIZE), &mut masks).unwrap();
    for i in 0..56 {
        let size = if i % 9 == 0 {
            Size {
                width: 96,
                height: 50,
            }
        } else {
            SIZE
        };
        let mut clip = Clip::canvas(size);
        if i % 11 == 0 {
            clip.left = 12;
        }
        let row = i % 3;
        match i % 4 {
            0 => view
                .set_content(
                    row,
                    if i % 8 == 0 {
                        "中文"
                    } else {
                        "Hello 世界！文字"
                    },
                )
                .unwrap(),
            1 => view
                .set_color(row, [0, 120, 20, if i % 8 == 1 { 0 } else { 150 }])
                .unwrap(),
            2 => view.set_shift(row, (i % 16) as u32).unwrap(),
            _ => view
                .set_style(
                    row,
                    crate::text::Style {
                        size: 18,
                        line_height: 32,
                    },
                )
                .unwrap(),
        }
        let frame = view.update(size, Mode::Pruned).unwrap().frame;
        retained
            .update(frame.clone(), size, clip, &mut masks)
            .unwrap();
        assert_eq!(
            retained.surface().rgba,
            Surface::draw(&frame, size, clip).unwrap().rgba
        );
        assert_eq!(retained.update(frame, size, clip, &mut masks).unwrap(), 0);
    }
    drop(view);
    retained
        .update(first.clone(), SIZE, Clip::canvas(SIZE), &mut masks)
        .unwrap();
    assert_eq!(retained.surface().rgba, render(&first).rgba);
    // An invalid new target cannot advance the previously committed pixels.
    let invalid = Size {
        width: 0,
        height: 48,
    };
    assert!(
        retained
            .update(first.clone(), invalid, Clip::canvas(invalid), &mut masks)
            .is_err()
    );
    assert_eq!(
        retained
            .update(first, SIZE, Clip::canvas(SIZE), &mut masks)
            .unwrap(),
        0
    );
}

#[test]
fn damage_replays_unchanged_overlapping_layers_in_order() {
    use crate::{damage::RetainedPixels, glyph_masks::GlyphMasks};
    fn layered(color: [u8; 4], view: &mut Presentation) -> Rc<Frame> {
        view.set_color(0, color).unwrap();
        view.set_color(1, [0, 0, 255, 128]).unwrap();
        let frame = view.update(SIZE, Mode::Normal).unwrap().frame;
        // Assemble an overlapping fixture while sharing the original shaped paragraphs.
        let mut draws: Vec<_> = frame
            .draws
            .iter()
            .map(|d| crate::frame::TextDraw {
                node: d.node,
                origin: d.origin,
                allocated: d.allocated,
                color: d.color,
                paragraph: d.paragraph.clone(),
            })
            .collect();
        draws[1].origin = draws[0].origin;
        draws[1].origin.x += 3;
        Rc::new(Frame {
            revision: frame.revision,
            font: frame.font,
            geometry: frame.geometry.clone(),
            draws,
        })
    }
    let mut view = Presentation::new(2, "中文", true);
    let first = layered([255, 0, 0, 255], &mut view);
    let second = layered([0, 128, 0, 200], &mut view);
    let mut masks = GlyphMasks::new(16, 2048);
    let mut retained = RetainedPixels::new(first, SIZE, Clip::canvas(SIZE), &mut masks).unwrap();
    let n = retained
        .update(second.clone(), SIZE, Clip::canvas(SIZE), &mut masks)
        .unwrap();
    assert_eq!(
        n,
        (second.draws[0].allocated.width * second.draws[0].allocated.height) as usize
    );
    assert_eq!(retained.surface().rgba, render(&second).rgba);
    let mut swapped = layered([0, 128, 0, 200], &mut view);
    Rc::get_mut(&mut swapped).unwrap().draws.swap(0, 1);
    assert_ne!(render(&second).rgba, render(&swapped).rgba);
    retained
        .update(swapped.clone(), SIZE, Clip::canvas(SIZE), &mut masks)
        .unwrap();
    assert_eq!(retained.surface().rgba, render(&swapped).rgba);
}

#[test]
fn fractional_scale_matches_a_native_size_glyph_with_a_warm_one_x_cache() {
    use crate::{glyph_masks::GlyphMasks, pixels::RasterScale, text::Style};
    let physical = Size {
        width: 192,
        height: 120,
    };
    let mut view = Presentation::new(1, "界", true);
    let logical = view.update(SIZE, Mode::Normal).unwrap().frame;
    let mut masks = GlyphMasks::new(16, 16384);
    Surface::draw_cached(&logical, SIZE, Clip::canvas(SIZE), &mut masks).unwrap();
    let scaled = Surface::raster(
        &logical,
        physical,
        Clip::canvas(physical),
        (0, 0),
        RasterScale(180),
        Some(&mut masks),
    )
    .unwrap();

    // Produce the reference by shaping and rasterizing directly at 24 px.
    view.set_style(
        0,
        Style {
            size: 24,
            line_height: 36,
        },
    )
    .unwrap();
    let mut native = view.update(physical, Mode::Normal).unwrap().frame;
    drop(view);
    let draw = &mut Rc::get_mut(&mut native).unwrap().draws[0];
    draw.origin.x = 3;
    draw.origin.y = 3;
    let reference = Surface::draw(&native, physical, Clip::canvas(physical)).unwrap();
    assert!(reference.rgba.iter().any(|&channel| channel < 128));
    assert_eq!(scaled.rgba, reference.rgba);
}
