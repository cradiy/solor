//! Pixel coverage and retained repair of filtered backgrounds.
use anyhow::Result;
use solor::{
    Styled, Ui, component, div,
    platform::{WindowBackground, WindowEvent, WindowHandler},
    px,
    raster::{DeviceSize, RasterScale},
    rgb, rgba,
};
use solor_render_cpu::CpuWindow;
use std::time::Duration;

fn panel(changed: bool) -> Result<CpuWindow> {
    CpuWindow::mount_with_background(
        Ui::new(),
        component(move |cx| {
            let tint = cx.signal(if changed {
                rgba(0xff000080)
            } else {
                rgba(0x0000ff80)
            });
            div()
                .items_start()
                .child(
                    div()
                        .absolute()
                        .left(px(7.))
                        .top(px(2.))
                        .w(px(1.))
                        .h(px(18.))
                        .bg(tint),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(8.))
                        .top(px(2.))
                        .size(px(18.))
                        .rounded(px(4.))
                        .backdrop_blur(px(2.))
                        .child(
                            div()
                                .absolute()
                                .left(px(7.))
                                .top(px(7.))
                                .size(px(3.))
                                .bg(rgb(0x00ff00)),
                        ),
                )
                .on_frame(move |_, cx| cx.set(tint, rgba(0xff000080)))
        }),
        WindowBackground::Transparent,
    )
}

#[test]
fn background_change_outside_the_panel_reaches_its_rounded_output() -> Result<()> {
    let size = DeviceSize {
        width: 48,
        height: 28,
    }
    .at_unit_scale();
    for scale in [RasterScale::ONE, RasterScale(1.5)] {
        let mut retained = panel(false)?;
        retained.frame(size, scale)?;
        retained.frame_submitted()?;
        retained.event(WindowEvent::FrameReady(Duration::ZERO))?;
        let repaired = retained.frame(size, scale)?;
        let mut fresh = panel(true)?;
        assert_eq!(repaired.rgba, fresh.frame(size, scale)?.rgba);
        assert!(repaired.damage.bounds().unwrap().right < i64::from(repaired.size.width));
        if scale.0 == 1. {
            let pixel = |x, y| repaired.rgba.as_chunks::<4>().0[y * 48usize + x];
            assert_eq!(pixel(8, 10), [26, 0, 0, 26]);
            assert_eq!(pixel(8, 2), [0; 4]);
            assert_eq!(pixel(16, 10), [0, 255, 0, 255]);
            assert_eq!(pixel(7, 10), [128, 0, 0, 128]);
        }
    }
    Ok(())
}

#[test]
fn translucent_group_samples_across_rows_before_compositing() -> Result<()> {
    let mut window = CpuWindow::mount(
        Ui::new(),
        component(|_| {
            div().bg(rgb(0xff0000)).child(
                div()
                    .w_full()
                    .h_full()
                    .opacity(0.5)
                    .child(div().w_full().h(px(128.)).bg(rgb(0xffffff)))
                    .child(div().absolute().inset(px(0.)).backdrop_blur(px(1.))),
            )
        }),
    )?;
    let size = DeviceSize {
        width: 512,
        height: 256,
    }
    .at_unit_scale();
    let frame = window.frame(size, RasterScale::ONE)?;
    let pixel = |y| frame.rgba.as_chunks::<4>().0[y * 512usize + 256usize];
    assert_eq!(pixel(127), [255, 85, 85, 255]);
    assert_eq!(pixel(128), [255, 43, 43, 255]);
    assert_eq!(pixel(129), [255, 0, 0, 255]);
    Ok(())
}
