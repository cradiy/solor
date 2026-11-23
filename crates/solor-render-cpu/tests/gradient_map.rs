//! Checks subtree color mapping through retained CPU window submission.
use anyhow::Result;
use solor::{
    GradientMap, ImageAsset, SharedString, Styled, SvgAsset, Ui, color_stop, component, div,
    gradient_left_to_right, image,
    platform::{WindowBackground, WindowEvent, WindowHandler},
    px,
    raster::{DeviceSize, RasterScale},
    rgb, rgba, svg, text,
};
use solor_render_cpu::CpuWindow;
use std::time::Duration;

#[test]
fn paint_scope_samples_the_composited_overlap_and_leaves_other_draws_in_color() -> Result<()> {
    use solor::layout::{Origin, Size};
    use solor_render_cpu::pixels::Surface;

    let ramp = gradient_left_to_right([
        color_stop(rgb(0x0000ff), 0.0),
        color_stop(rgb(0x0000ff), 0.6),
        color_stop(rgb(0xff0000), 0.6),
        color_stop(rgb(0xff0000), 1.0),
    ])?;
    let mut ui = Ui::new();
    let map = ui.context().signal(GradientMap::from(ramp.clone()));
    let block = Size {
        width: px(2.),
        height: px(2.),
    };
    let at = |x| Origin {
        x: px(x),
        y: px(0.),
    };
    let view = ui.mount(component(move |_| {
        div().paint(move |cx| {
            cx.with_gradient_map(cx.read(map)?, |cx| {
                cx.fill(block, rgb(0xffffff));
                cx.fill_rect(at(1.), block, rgba(0x00000080))
            })?;
            cx.fill_rect(at(3.), block, rgb(0x00ff00))
        })
    }))?;
    let size = DeviceSize {
        width: 5,
        height: 2,
    };
    ui.layout(view, size.at_unit_scale())?;
    let render = |ui: &Ui| -> Result<Surface> {
        Ok(Surface::draw_view(
            &ui.record_frame(view)?,
            size,
            RasterScale::ONE,
        )?)
    };
    let mapped = render(&ui)?;
    let row = mapped.rgba.as_chunks::<4>().0;
    assert_eq!(row[0], [255, 0, 0, 255]);
    assert_eq!(row[1], [0, 0, 255, 255]);
    assert_eq!(row[2], [127, 127, 255, 255]);
    assert_eq!(row[3], [0, 255, 0, 255]);
    ui.context().set(map, GradientMap::new(ramp, 0.)?)?;
    ui.flush()?;
    let original = render(&ui)?;
    assert_eq!(&original.rgba[4..8], &[127, 127, 127, 255]);
    Ok(())
}

#[test]
fn close_luminances_cross_a_hard_stop_and_translucent_colors_mix_in_premultiplied_space()
-> Result<()> {
    let ramp = gradient_left_to_right([
        color_stop(rgb(0x0000ff), 0.0),
        color_stop(rgb(0x0000ff), 0.21215),
        color_stop(rgb(0xff0000), 0.21215),
        color_stop(rgba(0xff000080), 1.0),
    ])?;
    let half = GradientMap::new(ramp.clone(), 0.5)?;
    let mut window = CpuWindow::mount_with_background(
        Ui::new(),
        component(move |_| {
            div()
                .flex_row()
                .child(
                    div()
                        .flex_row()
                        .gradient_map(ramp)
                        .child(div().size(px(2.)).bg(rgb(0xfe0000)))
                        .child(div().size(px(2.)).bg(rgb(0xff0000))),
                )
                .child(
                    div()
                        .size(px(2.))
                        .bg(rgba(0xffffff80))
                        .gradient_map(half)
                        .opacity(0.5),
                )
        }),
        WindowBackground::Transparent,
    )?;
    let size = DeviceSize {
        width: 8,
        height: 2,
    }
    .at_unit_scale();
    let frame = window.frame(size, RasterScale::ONE)?;
    let row = frame.rgba.as_chunks::<4>().0;
    assert_eq!(row[0], [0, 0, 255, 255]);
    assert_eq!(row[2], [255, 0, 0, 255]);
    assert_eq!(row[4], [48, 32, 32, 48]);
    assert_eq!(row[6], [0; 4]);
    Ok(())
}

fn swatch(initial: usize) -> Result<CpuWindow> {
    let dark = rgb(0x180e35);
    let ramp = gradient_left_to_right([color_stop(dark, 0.0), color_stop(rgb(0xfa9934), 1.0)])?;
    let maps = [
        GradientMap::from(ramp.clone()),
        GradientMap::new(ramp, 0.0)?,
    ];
    let photo = ImageAsset::from_rgba8(1, 1, vec![34, 128, 87, 220])?;
    let icon=SvgAsset::from_bytes(br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><circle cx="4" cy="4" r="4" opacity=".6"/></svg>"#)?;
    let ui = Ui::with_font(
        include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf").to_vec(),
    )?;
    CpuWindow::mount(
        ui,
        component(move |cx| {
            let label = cx.signal(SharedString::from(if initial == 0 { "A" } else { "B" }));
            let map = cx.signal(maps[usize::from(initial == 2)].clone());
            let mut step = initial;
            div().bg(rgb(0x376482)).items_start().child(
                div()
                    .w(px(80.))
                    .h(px(50.))
                    .items_start()
                    .gradient_map(map)
                    .child(image(photo).size(px(12.)))
                    .child(svg(icon).size(px(8.)).text_color(rgb(0xffffff)))
                    .child(text(label).text_color(rgb(0x132343)))
                    .on_frame(move |_, cx| {
                        step += 1;
                        if step == 1 {
                            cx.set(label, SharedString::from("B"))
                        } else {
                            cx.set(map, maps[1].clone())
                        }
                    }),
            )
        }),
    )
}

#[test]
fn line_repair_and_switching_off_match_fresh_frames_at_fractional_scale() -> Result<()> {
    let size = DeviceSize {
        width: 96,
        height: 64,
    }
    .at_unit_scale();
    let scale = RasterScale(1.5);
    let mut retained = swatch(0)?;
    retained.frame(size, scale)?;
    retained.frame_submitted()?;
    for step in [1, 2] {
        retained.event(WindowEvent::FrameReady(Duration::ZERO))?;
        let result = retained.frame(size, scale)?;
        let changed = result.damage.bounds().expect("a visible change");
        if step == 1 {
            assert!(changed.top >= 30 && changed.right < 120, "{changed:?}");
        }
        let mut fresh = swatch(step)?;
        assert_eq!(result.rgba, fresh.frame(size, scale)?.rgba);
        retained.frame_submitted()?;
    }
    Ok(())
}
