//! Group opacity through retained window submission and CPU composition.
use std::time::Duration;

use anyhow::Result;
use solor::{
    ImageAsset, Styled, Ui, component, div, image,
    platform::{WindowBackground, WindowEvent, WindowHandler},
    px,
    raster::{Clip, DeviceSize, RasterScale},
    rgb,
};
use solor_render_cpu::CpuWindow;

#[test]
fn nested_image_and_background_fade_as_one_group() -> Result<()> {
    let bitmap = ImageAsset::from_rgba8(1, 1, vec![0, 0, 255, 128])?;
    let mut window = CpuWindow::mount_with_background(
        Ui::new(),
        component(move |cx| {
            let opacity = cx.signal(1.0_f32);
            let mut changes = [0.5, 0.0, 1.0].into_iter();
            div()
                .flex_row()
                .on_frame(move |_, cx| {
                    if let Some(value) = changes.next() {
                        cx.set(opacity, value)?;
                    }
                    Ok(())
                })
                .child(
                    div()
                        .w(px(8.))
                        .h_full()
                        .opacity(opacity)
                        .bg(rgb(0xff0000))
                        .child(image(bitmap).size(px(4.)).opacity(0.5)),
                )
                .child(div().w(px(4.)).h_full().bg(rgb(0x00ff00)))
        }),
        WindowBackground::Transparent,
    )?;
    let size = DeviceSize {
        width: 12,
        height: 4,
    }
    .at_unit_scale();
    for (index, (overlap, background)) in [
        ([191, 0, 64, 255], [255, 0, 0, 255]),
        ([96, 0, 32, 128], [128, 0, 0, 128]),
        ([0; 4], [0; 4]),
        ([191, 0, 64, 255], [255, 0, 0, 255]),
    ]
    .into_iter()
    .enumerate()
    {
        if index > 0 {
            window.event(WindowEvent::FrameReady(Duration::ZERO))?;
        }
        let frame = window.frame(size, RasterScale::ONE)?;
        assert_eq!(&frame.rgba[0..4], &overlap);
        assert_eq!(&frame.rgba[24..28], &background);
        assert_eq!(&frame.rgba[40..44], &[0, 255, 0, 255]);
        if index > 0 {
            assert_eq!(
                frame.damage.bounds(),
                Some(Clip {
                    left: 0,
                    top: 0,
                    right: 8,
                    bottom: 4
                })
            );
        }
        window.frame_submitted()?;
    }
    Ok(())
}

#[test]
fn partial_repair_inside_a_translucent_container_preserves_the_backdrop() -> Result<()> {
    let mut window = CpuWindow::mount(
        Ui::new(),
        component(|cx| {
            let tint = cx.signal(rgb(0));
            div().bg(rgb(0x204060)).child(
                div().w_full().h_full().opacity(0.5).child(
                    div()
                        .size(px(2.))
                        .bg(tint)
                        .on_frame(move |_, cx| cx.set(tint, rgb(0xffffff))),
                ),
            )
        }),
    )?;
    let size = DeviceSize {
        width: 20,
        height: 10,
    }
    .at_unit_scale();
    let scale = RasterScale(1.5);
    let before = window.frame(size, scale)?.rgba.to_vec();
    window.frame_submitted()?;
    window.event(WindowEvent::FrameReady(Duration::ZERO))?;
    let frame = window.frame(size, scale)?;
    assert_eq!(
        frame.damage.bounds(),
        Some(Clip {
            left: 0,
            top: 0,
            right: 3,
            bottom: 3
        })
    );
    for (i, pixel) in frame.rgba.as_chunks::<4>().0.iter().enumerate() {
        if i % 30 < 3 && i / 30 < 3 {
            assert_eq!(*pixel, [144, 160, 176, 255]);
        } else {
            assert_eq!(*pixel, before[4 * i..4 * i + 4]);
        }
    }
    Ok(())
}
