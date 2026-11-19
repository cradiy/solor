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
