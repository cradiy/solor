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
fn paint_scope_composites_overlaps_once_and_tracks_opacity_changes() -> Result<()> {
    use solor::layout::{Origin, Size};

    let mut window = CpuWindow::mount_with_background(
        Ui::new(),
        component(|cx| {
            let opacity = cx.signal(0.5);
            let mut changes = [0.0, 1.0].into_iter();
            div()
                .on_frame(move |_, cx| {
                    if let Some(value) = changes.next() {
                        cx.set(opacity, value)?;
                    }
                    Ok(())
                })
                .child(
                    div()
                        .absolute()
                        .left(px(3.))
                        .w(px(10.))
                        .h(px(2.))
                        .paint(move |cx| {
                            let at = |x| Origin {
                                x: px(x),
                                y: px(0.),
                            };
                            let size = |w| Size {
                                width: px(w),
                                height: px(2.),
                            };
                            cx.with_opacity(*cx.read(opacity)?, |cx| {
                                cx.fill(size(6.), rgb(0xff0000));
                                cx.fill_rect(at(2.), size(4.), rgb(0x0000ff))?;
                                cx.with_opacity(0.5, |cx| {
                                    cx.fill_rect(at(6.), size(2.), rgb(0x00ff00))
                                })
                            })?;
                            cx.fill_rect(at(8.), size(2.), rgb(0xffffff))
                        }),
                )
        }),
        WindowBackground::Transparent,
    )?;
    let size = DeviceSize {
        width: 14,
        height: 2,
    }
    .at_unit_scale();
    for (i, (alpha, nested)) in [(128, 64), (0, 0), (255, 128)].into_iter().enumerate() {
        if i > 0 {
            window.event(WindowEvent::FrameReady(Duration::ZERO))?;
        }
        let frame = window.frame(size, RasterScale::ONE)?;
        let row = frame.rgba.as_chunks::<4>().0;
        assert_eq!(row[2], [0; 4]);
        assert_eq!(row[3], [alpha, 0, 0, alpha]);
        assert_eq!(row[5], [0, 0, alpha, alpha]);
        assert_eq!(row[9], [0, nested, 0, nested]);
        assert_eq!(row[11], [255; 4]);
        window.frame_submitted()?;
    }
    Ok(())
}

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

#[test]
fn moving_a_translucent_group_in_a_long_display_list_matches_full_repaint() -> Result<()> {
    fn scene(phase: usize) -> impl solor::Render {
        component(move |cx| {
            let positions = [2., 2., 20., 2.];
            let left = cx.signal(px(positions[phase]));
            let tint = cx.signal(if phase == 0 { rgb(0) } else { rgb(0xff8800) });
            let fillers = || (0..100).map(|_| div().absolute().size(px(1.)).bg(rgb(0x334455)));
            div()
                .bg(rgb(0x102030))
                .on_frame(move |time, cx| {
                    cx.set(tint, rgb(0xff8800))?;
                    cx.set(left, px(positions[time.as_secs() as usize]))
                })
                .children(fillers())
                .child(
                    div()
                        .absolute()
                        .left(left)
                        .size(px(4.))
                        .opacity(0.5)
                        .bg(tint)
                        .child(div().size(px(2.)).bg(rgb(0x0088ff))),
                )
                .children(fillers())
                .child(
                    div()
                        .absolute()
                        .left(px(21.))
                        .size(px(2.))
                        .bg(solor::rgba(0xff000080)),
                )
        })
    }

    let size = DeviceSize {
        width: 30,
        height: 6,
    }
    .at_unit_scale();
    let scale = RasterScale(1.5);
    let mut window = CpuWindow::mount(Ui::new(), scene(0))?;
    window.frame(size, scale)?;
    window.frame_submitted()?;
    for phase in 1..=3 {
        window.event(WindowEvent::FrameReady(Duration::from_secs(phase)))?;
        let frame = window.frame(size, scale)?;
        let mut reference = CpuWindow::mount(Ui::new(), scene(phase as usize))?;
        let full = reference.frame(size, scale)?;
        assert!(!frame.damage.is_empty());
        assert_ne!(frame.damage, full.damage);
        assert_eq!(frame.rgba, full.rgba, "phase {phase}");
        window.frame_submitted()?;
    }
    Ok(())
}
