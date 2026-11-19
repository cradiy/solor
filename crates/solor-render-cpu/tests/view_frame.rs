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
