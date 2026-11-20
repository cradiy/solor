//! Complete mounted frames keep drawing, resource lifetime and hit geometry in sync.
use std::cell::Cell;

use anyhow::Result;
use solor::{
    Fill, Styled, Ui, color_stop, component, div, gradient_left_to_right,
    layout::Origin,
    pointer::{PointerButton, PointerId, PointerInput, PointerKind, PointerSample},
    px,
    raster::{DeviceSize, RasterScale},
    rgb, rgba, text,
};
use solor_render_cpu::pixels::Surface;

fn pixel(surface: &Surface, x: usize, y: usize) -> &[u8] {
    let i = (y * surface.size.width as usize + x) * 4;
    &surface.rgba[i..i + 4]
}

#[test]
fn nested_paint_and_input_share_the_published_origin() -> Result<()> {
    let size = DeviceSize {
        width: 100,
        height: 64,
    };
    let font = include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf");
    let mut ui = Ui::with_font(font.to_vec(), "en")?;
    let clicked = ui.context().signal(false);
    let view = ui.mount(component(|_| {
        div().bg(rgb(0)).p(px(4.5)).child(
            div().p(px(2.5)).bg(rgba(0xff000080)).child(
                div()
                    .p(px(2.0))
                    .w(px(52.0))
                    .h(px(28.0))
                    .bg(rgb(0xffffff))
                    .rounded(px(6.0))
                    .child(text("Solor").text_color(rgb(0)))
                    .on_click(move |cx| cx.set(clicked, true)),
            ),
        )
    }))?;
    ui.flush()?;
    ui.layout(view, size.at_unit_scale())?;
    let frame = ui.record_frame(view)?;
    let scale = RasterScale(2.0);
    let physical = scale.buffer_size(size.at_unit_scale())?;
    let image = Surface::draw_view(&frame, physical, scale)?;
    assert_eq!(pixel(&image, 0, 0), [0, 0, 0, 255]);
    assert_eq!(pixel(&image, 9, 9), [128, 0, 0, 255]);
    assert_eq!(pixel(&image, 14, 14), [128, 0, 0, 255]);
    assert_eq!(pixel(&image, 26, 14), [255; 4]);
    assert!((19..63).any(|y| (19..100).any(|x| pixel(&image, x, y) == [0, 0, 0, 255])));

    let sample = PointerSample {
        id: PointerId(1),
        kind: PointerKind::Mouse,
        position: Origin {
            x: px(9.0),
            y: px(12.0),
        },
        modifiers: Default::default(),
    };
    let click = |ui: &mut Ui| -> Result<()> {
        ui.dispatch_pointer(view, PointerInput::Down(sample, PointerButton::Primary))?;
        ui.dispatch_pointer(view, PointerInput::Up(sample, PointerButton::Primary))
    };
    click(&mut ui)?;
    assert!(!*ui.context().read(clicked)?);
    ui.accept_pointer_frame(frame.into_pointer_frame())?;
    click(&mut ui)?;
    assert!(*ui.context().read(clicked)?);
    let held = ui.record_frame(view)?;
    ui.unmount(view)?;
    drop(ui);
    assert_eq!(Surface::draw_view(&held, physical, scale)?.rgba, image.rgba);
    Ok(())
}

#[test]
fn clipped_gradient_samples_use_the_entire_box() -> Result<()> {
    let mut ui = Ui::new();
    let ramp = gradient_left_to_right([
        color_stop(rgb(0xff0000), 0.0),
        color_stop(rgb(0x00ff00), 0.5),
        color_stop(rgb(0x0000ff), 1.0),
    ])?;
    let view = ui.mount(component(|_| div().bg(ramp.clone())))?;
    let full = DeviceSize {
        width: 9,
        height: 3,
    };
    ui.layout(view, full.at_unit_scale())?;
    let frame = ui.record_frame(view)?;
    let crop = Surface::draw_view(
        &frame,
        DeviceSize {
            width: 5,
            height: 1,
        },
        RasterScale::ONE,
    )?;
    for x in 0..5 {
        let c = ramp.sample_at(x as f32 + 0.5, 0.5, full.at_unit_scale())?;
        assert_eq!(pixel(&crop, x, 0), [c.red, c.green, c.blue, 255]);
    }
    assert_eq!(pixel(&crop, 4, 0), [0, 255, 0, 255]);
    Ok(())
}

#[test]
fn late_recording_failure_preserves_earlier_nodes_repaint_requests() -> Result<()> {
    let mut ui = Ui::new();
    let source = Cell::new(None);
    let owner = ui.mount(component(|cx| {
        source.set(Some(cx.signal(Fill::Solid(rgb(0xffffff)))));
        div()
    }))?;
    let visible = ui.context().signal(true);
    let first = ui.context().signal(Fill::Solid(rgb(0)));
    let last = source.get().unwrap();
    let view = ui.mount(component(|_| {
        div().bg(first).child_when(visible, move || div().bg(last))
    }))?;
    let size = DeviceSize {
        width: 20,
        height: 20,
    }
    .at_unit_scale();
    ui.flush()?;
    ui.layout(view, size)?;
    ui.record_frame(view)?;
    ui.context().set(first, Fill::Solid(rgb(0xff0000)))?;
    ui.flush()?;
    ui.unmount(owner)?;
    assert!(ui.record_frame(view).is_err());
    let mut ready = Vec::new();
    ui.collect_repaints(view, &mut ready)?;
    assert_eq!(ready, [ui.root(view)?]);
    ui.context().set(visible, false)?;
    ui.flush()?;
    ui.layout(view, size)?;
    let frame = ui.record_frame(view)?;
    ready.clear();
    ui.collect_repaints(view, &mut ready)?;
    assert!(ready.is_empty());
    assert_eq!(frame.draws().len(), 1);
    Ok(())
}

#[test]
fn fixed_label_recording_matches_live_text_across_edits_and_relayout() -> Result<()> {
    use solor::{SharedString, TextContent};

    let font = include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf");
    let mut ui = Ui::with_font(font.to_vec(), "en")?;
    let fixed = ui.context().signal(SharedString::from("Output level"));
    let value = ui.context().signal(SharedString::from("A"));
    let background = ui.context().signal(Fill::Solid(rgb(0xffffff)));
    let open = ui.context().signal(false);
    let panel = |use_fixed: bool| {
        let label = move || {
            text(if use_fixed {
                TextContent::from("Output level")
            } else {
                TextContent::Bound(fixed)
            })
        };
        component(move |_| {
            div()
                .p(px(2.5))
                .child_when(open, move || label().text_color(rgb(0x903010)))
                .child(label().text_color(rgb(0x152030)))
                .child(text(value))
                .child(label().bg(background))
                .child(label().p(px(1.25)))
        })
    };
    let cached = ui.mount(panel(true))?;
    let live = ui.mount(panel(false))?;
    for (width, visible, content, color) in [
        (120, false, "A", 0xffffff),
        (120, false, "B", 0x233040),
        (120, true, "B", 0x233040),
        (46, true, "Output level", 0xffffff),
        (46, false, "A", 0x233040),
    ] {
        ui.context().set(value, SharedString::from(content))?;
        ui.context().set(open, visible)?;
        ui.context().set(background, Fill::Solid(rgb(color)))?;
        ui.flush()?;
        let size = DeviceSize { width, height: 180 }.at_unit_scale();
        ui.layout(cached, size)?;
        ui.layout(live, size)?;
        let reference = ui.record_frame(live)?;
        for _ in 0..3 {
            assert_eq!(ui.record_frame(cached)?.draws(), reference.draws());
        }
    }
    Ok(())
}

#[test]
fn overlapping_subtrees_share_paint_and_accepted_hit_order() -> Result<()> {
    let size = DeviceSize {
        width: 32,
        height: 32,
    };
    let mut ui = Ui::new();
    let open = ui.context().signal(false);
    let clicked = ui.context().signal(0u8);
    let layer = || div().size(px(16.0)).mb(px(-16.0));
    let view = ui.mount(component(|_| {
        div()
            .flex_col()
            .child_when(open, move || {
                layer()
                    .z_index(1)
                    .on_click(move |cx| cx.set(clicked, 1))
                    .child(div().size(px(16.0)).bg(rgb(0x0000ff)))
            })
            .child(
                layer()
                    .bg(rgb(0xff0000))
                    .on_click(move |cx| cx.set(clicked, 2)),
            )
            .child(
                layer()
                    .bg(rgb(0x00ff00))
                    .on_click(move |cx| cx.set(clicked, 3)),
            )
            .child(
                layer().z_index(-1).child(
                    div()
                        .size(px(16.0))
                        .z_index(i32::MAX)
                        .bg(rgb(0xffffff))
                        .on_click(move |cx| cx.set(clicked, 4)),
                ),
            )
    }))?;
    let sample = PointerSample {
        id: PointerId(1),
        kind: PointerKind::Mouse,
        position: Origin {
            x: px(5.0),
            y: px(5.0),
        },
        modifiers: Default::default(),
    };
    let click = |ui: &mut Ui| -> Result<u8> {
        ui.context().set(clicked, 0)?;
        ui.dispatch_pointer(view, PointerInput::Down(sample, PointerButton::Primary))?;
        ui.dispatch_pointer(view, PointerInput::Up(sample, PointerButton::Primary))?;
        Ok(*ui.context().read(clicked)?)
    };

    ui.flush()?;
    ui.layout(view, size.at_unit_scale())?;
    let root = ui.root(view)?;
    let initial_children: Vec<_> = ui.children(view, root)?.collect();
    let initial = ui.record_frame(view)?;
    let image = Surface::draw_view(&initial, size, RasterScale::ONE)?;
    assert_eq!(pixel(&image, 5, 5), [0, 255, 0, 255]);
    ui.accept_pointer_frame(initial.into_pointer_frame())?;
    assert_eq!(click(&mut ui)?, 3);

    ui.context().set(open, true)?;
    ui.flush()?;
    ui.layout(view, size.at_unit_scale())?;
    assert!(
        ui.children(view, root)?
            .skip(1)
            .eq(initial_children.iter().copied())
    );
    let pending = ui.record_frame(view)?;
    let image = Surface::draw_view(&pending, size, RasterScale::ONE)?;
    assert_eq!(pixel(&image, 5, 5), [0, 0, 255, 255]);
    drop(pending);
    assert_eq!(click(&mut ui)?, 3);
    let retry = ui.record_frame(view)?;
    ui.accept_pointer_frame(retry.into_pointer_frame())?;
    assert_eq!(click(&mut ui)?, 1);

    ui.context().set(open, false)?;
    ui.flush()?;
    ui.layout(view, size.at_unit_scale())?;
    let closed = ui.record_frame(view)?;
    ui.accept_pointer_frame(closed.into_pointer_frame())?;
    assert_eq!(click(&mut ui)?, 3);
    Ok(())
}

#[test]
fn asymmetric_background_radii_fit_each_allocation_and_survive_replay() -> Result<()> {
    use solor::{Draw, rem, style::Corners};

    let mut ui = Ui::new();
    let view = ui.mount(component(|_| {
        div()
            .bg(rgb(0))
            .rounded_tl(rem(1.5))
            .rounded_tr(px(8.0))
            .rounded_br(px(4.0))
    }))?;
    let radii = Corners {
        top_left: px(24.0),
        top_right: px(8.0),
        bottom_right: px(4.0),
        bottom_left: px(0.0),
    };
    let mut held = None;
    for (width, height, factor) in [(16, 24, 0.5), (32, 6, 0.25), (32, 24, 1.0)] {
        let size = DeviceSize { width, height }.at_unit_scale();
        ui.layout(view, size)?;
        let frame = ui.record_frame(view)?;
        assert!(matches!(frame.draws(), [Draw::Fill { corner_radii, .. }]
            if *corner_radii == radii.map(|r| r * factor)));
        if held.is_none() {
            held = Some(frame);
        }
    }

    let frame = held.unwrap();
    let scale = RasterScale(1.5);
    let image = Surface::draw_view(
        &frame,
        DeviceSize {
            width: 24,
            height: 36,
        },
        scale,
    )?;
    // The large top-left arc extends beyond the middle of the top edge.
    assert_eq!(pixel(&image, 8, 0), [255; 4]);
    assert_eq!(pixel(&image, 18, 0), [0, 0, 0, 255]);
    assert_eq!(pixel(&image, 23, 0), [255; 4]);
    assert_eq!(pixel(&image, 0, 35), [0, 0, 0, 255]);
    assert_eq!(pixel(&image, 23, 35), [255; 4]);
    let edge = pixel(&image, 22, 35);
    assert!(edge[0] > 0 && edge[0] < 255);
    let crop = Surface::draw_view(
        &frame,
        DeviceSize {
            width: 10,
            height: 8,
        },
        scale,
    )?;
    for y in 0..8 {
        for x in 0..10 {
            assert_eq!(pixel(&crop, x, y), pixel(&image, x, y));
        }
    }
    Ok(())
}

#[test]
fn aligned_labels_use_each_visual_lines_allocated_width() -> Result<()> {
    use solor::{Draw, SharedString, percent};

    let font = include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf");
    let mut ui = Ui::with_font(font.to_vec(), "en")?;
    let value = ui.context().signal(SharedString::from("A"));
    let view = ui.mount(component(|_| {
        div()
            .text_center()
            .child(text("A").w(percent(100.0)).px(px(3.5)))
            .child(text(value).w(percent(100.0)).px(px(3.5)).text_right())
    }))?;
    for (width, content) in [(104, "A"), (72, "Output level A")] {
        ui.context().set(value, SharedString::from(content))?;
        ui.flush()?;
        let size = DeviceSize { width, height: 120 };
        ui.layout(view, size.at_unit_scale())?;
        let frame = ui.record_frame(view)?;
        let image = Surface::draw_view(&frame, size, RasterScale::ONE)?;
        for (index, draw) in frame.draws().iter().enumerate() {
            let Draw::Text {
                origin,
                size: inner,
                paragraph,
                ..
            } = draw
            else {
                panic!("expected a label");
            };
            assert_eq!(inner.width, px(width as f32 - 7.0));
            if index == 1 && content != "A" {
                assert!(paragraph.lines.len() > 1);
            }
            for line in &paragraph.lines {
                let spare = inner.width.get() - line.width;
                let offset = if index == 0 { spare / 2.0 } else { spare };
                assert!((line.glyphs[0].x - offset).abs() < 0.01);
                let left = (origin.x.get() + offset).floor() as usize;
                let y = (origin.y.get() + line.top) as usize;
                let first_ink = (0..width as usize)
                    .find(|&x| {
                        (y..y + line.height as usize).any(|y| pixel(&image, x, y) != [255; 4])
                    })
                    .expect("visible glyphs");
                assert!(first_ink.abs_diff(left) <= 3);
            }
        }
        assert_eq!(frame.draws().len(), 2);
    }
    Ok(())
}

#[test]
fn logical_alignment_follows_paragraph_direction_and_justification_skips_last_line() -> Result<()> {
    use solor::Draw;

    let font = include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf");
    let mut ui = Ui::with_font(font.to_vec(), "en")?;
    // The leading right-to-left mark selects the base direction for this paragraph.
    let rtl = "\u{200f}A";
    let view = ui.mount(component(|_| {
        div()
            .child(text(rtl).w(px(90.0)).text_start())
            .child(text(rtl).w(px(90.0)).text_end())
            .child(text("A A A A A A A").w(px(62.0)).text_justify())
            .child(text("A A A A A A A").w(px(62.0)).text_left())
    }))?;
    ui.layout(
        view,
        DeviceSize {
            width: 100,
            height: 180,
        }
        .at_unit_scale(),
    )?;
    let frame = ui.record_frame(view)?;
    let paragraphs: Vec<_> = frame
        .draws()
        .iter()
        .map(|draw| match draw {
            Draw::Text { paragraph, .. } => paragraph,
            _ => panic!("expected a label"),
        })
        .collect();
    let letter_x = |i: usize| {
        paragraphs[i].lines[0]
            .glyphs
            .iter()
            .find(|g| g.start == '\u{200f}'.len_utf8())
            .unwrap()
            .x
    };
    assert!(letter_x(0) - letter_x(1) > 70.0);

    let justified = &paragraphs[2].lines;
    let left = &paragraphs[3].lines;
    assert_eq!(justified.len(), 2);
    assert_eq!(justified.len(), left.len());
    assert!((justified[0].width - 62.0).abs() < 0.01);
    assert!(justified[0].glyphs.last().unwrap().x > left[0].glyphs.last().unwrap().x);
    assert_eq!(justified[1].glyphs, left[1].glyphs);
    Ok(())
}

#[test]
fn inherited_line_clamp_sizes_and_paints_only_visible_lines() -> Result<()> {
    use solor::{Draw, SharedString, percent};
    use std::num::NonZeroUsize;

    let font = include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf");
    let mut ui = Ui::with_font(font.to_vec(), "en")?;
    let value = ui.context().signal(SharedString::from("A"));
    let view = ui.mount(component(|_| {
        div()
            .line_clamp(NonZeroUsize::new(2).unwrap())
            .child(text(value).w(percent(100.0)))
            .child(div().w(percent(100.0)).h(px(2.0)).bg(rgb(0xff0000)))
            .child(text(value).w(percent(100.0)).clear_line_clamp())
    }))?;
    for (width, source, clipped) in [
        (60, "Output level Output level", true),
        (240, "Output level Output level", false),
        (80, "A\n\nB", true),
        (80, "B", false),
    ] {
        ui.context().set(value, SharedString::from(source))?;
        ui.flush()?;
        let size = DeviceSize { width, height: 240 };
        ui.layout(view, size.at_unit_scale())?;
        let frame = ui.record_frame(view)?;
        let [
            Draw::Text {
                paragraph: preview, ..
            },
            Draw::Fill {
                origin: divider, ..
            },
            Draw::Text {
                origin: below,
                paragraph: full,
                ..
            },
        ] = frame.draws()
        else {
            panic!("expected two labels separated by a divider");
        };
        assert_eq!(full.lines.len() > 2, clipped);
        assert_eq!(preview.lines, full.lines[..full.lines.len().min(2)]);
        assert_eq!(preview.text.as_str(), source);
        let height = preview.lines.len() * 24;
        assert_eq!(preview.content.height, px(height as f32));
        assert_eq!(divider.y, preview.content.height);
        assert_eq!(below.y, divider.y + px(2.0));
        let image = Surface::draw_view(&frame, size, RasterScale::ONE)?;
        let stride = width as usize * 4;
        assert_eq!(pixel(&image, 0, height), [255, 0, 0, 255]);
        assert_eq!(
            image.rgba[..height * stride],
            image.rgba[(height + 2) * stride..(2 * height + 2) * stride]
        );
    }
    Ok(())
}

#[test]
fn single_line_ellipsis_keeps_source_ranges_across_resize_and_edits() -> Result<()> {
    use solor::{Draw, SharedString, percent};

    let font = include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf");
    let mut ui = Ui::with_font(font.to_vec(), "en")?;
    let source = ui.context().signal(SharedString::default());
    let view = ui.mount(component(|_| {
        div()
            .truncate()
            .child(text(source).w(percent(100.0)))
            .child(text(source).w(percent(100.0)).text_ellipsis_start())
            .child(text(source).w(percent(100.0)).text_ellipsis_middle())
    }))?;
    for (width, value, clipped) in [
        (80, "1234567890中文世界", true),
        (240, "1234567890中文世界", false),
        (80, "  A\r\n\t 中  ", false),
    ] {
        ui.context().set(source, SharedString::from(value))?;
        ui.flush()?;
        let size = DeviceSize { width, height: 72 };
        ui.layout(view, size.at_unit_scale())?;
        let frame = ui.record_frame(view)?;
        let image = Surface::draw_view(&frame, size, RasterScale::ONE)?;
        assert_eq!(frame.draws().len(), 3);
        for (index, draw) in frame.draws().iter().enumerate() {
            let Draw::Text { paragraph, .. } = draw else {
                panic!("expected label");
            };
            assert_eq!(paragraph.text.as_str(), value);
            let [line] = paragraph.lines.as_slice() else {
                panic!("expected one line");
            };
            let slices: Vec<_> = line.glyphs.iter().map(|g| &value[g.start..g.end]).collect();
            if clipped {
                let marker = slices
                    .iter()
                    .position(|s| s.is_empty())
                    .expect("ellipsis at elision boundary");
                let prefix = slices[..marker].concat();
                let suffix = slices[marker..].concat();
                assert!(value.starts_with(&prefix) && value.ends_with(&suffix));
                assert_eq!(prefix.is_empty(), index == 1);
                assert_eq!(suffix.is_empty(), index == 0);
                assert!(prefix.len() + suffix.len() < value.len());
                assert!(line.width <= width as f32);
            } else if value.starts_with(' ') {
                assert_eq!(slices, ["A", "\r\n\t ", "中"]);
            } else {
                assert_eq!(slices.concat(), value);
            }
        }
        let row = 24 * width as usize * 4;
        let images: Vec<_> = image.rgba.chunks_exact(row).collect();
        assert!(images[0].iter().any(|&v| v != 255));
        assert_eq!(images[0] != images[1], clipped);
        assert_eq!(images[0] != images[2], clipped);
    }
    Ok(())
}

#[test]
fn multiline_ellipsis_marks_only_the_last_visible_line() -> Result<()> {
    use solor::{Draw, SharedString, percent};
    use std::num::NonZeroUsize;

    let font = include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf");
    let mut ui = Ui::with_font(font.to_vec(), "en")?;
    let value = ui.context().signal(SharedString::default());
    let view = ui.mount(component(|_| {
        div()
            .child(
                text(value)
                    .w(percent(100.0))
                    .line_clamp(NonZeroUsize::new(2).unwrap())
                    .text_ellipsis(),
            )
            .child(text(value).w(percent(100.0)))
    }))?;
    for (width, source, expected) in [
        (62, "Output level Output level", true),
        (240, "Output level Output level", false),
        (48, "A\n中文世界中文世界", true),
        (48, "A\n中文\nC", true),
        (48, "A\n\nC", true),
        (48, "A\n中文", false),
        (48, "A\n", false),
        (48, "A\nB\n", true),
        (1, "word a abcdefghi\nmore", true),
        (45, "\u{200f}1234567890中文世界", true),
    ] {
        ui.context().set(value, SharedString::from(source))?;
        ui.flush()?;
        let size = DeviceSize { width, height: 192 };
        ui.layout(view, size.at_unit_scale())?;
        let frame = ui.record_frame(view)?;
        let [
            Draw::Text {
                paragraph: preview, ..
            },
            Draw::Text {
                paragraph: full,
                origin,
                ..
            },
        ] = frame.draws()
        else {
            panic!("expected two labels")
        };
        let hidden = full.lines.len() > 2;
        assert_eq!(expected, hidden);
        assert_eq!(preview.text.as_str(), source);
        assert_eq!(
            preview.lines.len(),
            full.lines.len().min(2),
            "{width} {source:?}"
        );
        let last = preview.lines.len() - 1;
        assert_eq!(
            preview.lines[..last],
            full.lines[..last],
            "{width} {source}"
        );
        let line = &preview.lines[last];
        assert_eq!(line.source_line, full.lines[last].source_line);
        let source_line = source.split('\n').nth(line.source_line).unwrap();
        assert!(
            line.glyphs
                .iter()
                .all(|g| source_line.get(g.start..g.end).is_some())
        );
        assert_eq!(
            line.glyphs.iter().any(|g| g.start == g.end),
            hidden,
            "{source:?}"
        );
        assert_eq!(origin.y, preview.content.height);
        // A sub-glyph viewport can clip the entire marker.
        if width == 1 {
            continue;
        }
        let image = Surface::draw_view(&frame, size, RasterScale::ONE)?;
        let row = width as usize * 4 * 24;
        let reference = (last + 1) * row;
        assert_eq!(
            image.rgba[..last * row],
            image.rgba[reference..reference + last * row]
        );
        assert_eq!(
            image.rgba[last * row..reference] != image.rgba[reference + last * row..2 * reference],
            hidden,
            "{source:?}"
        );
    }
    Ok(())
}
