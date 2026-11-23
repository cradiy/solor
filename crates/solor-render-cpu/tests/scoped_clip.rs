//! Content-local paint clips in local recordings and submitted view coordinates.
use anyhow::Result;
use solor::{
    Draw, Styled, Ui, component, div,
    layout::{Origin, Size},
    px,
    raster::RasterScale,
    rgb,
};
use solor_render_cpu::pixels::Surface;

fn at(x: f32, y: f32) -> Origin {
    Origin { x: px(x), y: px(y) }
}

fn size(w: f32, h: f32) -> Size {
    Size {
        width: px(w),
        height: px(h),
    }
}

#[test]
fn nested_clips_intersect_the_padded_box_and_restore_for_following_draws() -> Result<()> {
    let mut ui = Ui::new();
    let view = ui.mount(component(|_| {
        div().p(px(2.0)).child(
            div()
                .w(px(12.0))
                .h(px(10.0))
                .p(px(2.0))
                .overflow_hidden()
                .paint(|cx| {
                    cx.with_clip(at(2.0, 0.0), size(12.0, 8.0), |cx| {
                        cx.fill_rect(at(-4.0, -4.0), size(24.0, 24.0), rgb(0xff0000))?;
                        cx.with_clip(at(-2.0, 2.0), size(6.0, 4.0), |cx| {
                            cx.fill_rect(at(-4.0, -4.0), size(24.0, 24.0), rgb(0x0000ff))
                        })
                    })?;
                    cx.fill(size(2.0, 2.0), rgb(0x00ff00));
                    Ok(())
                }),
        )
    }))?;
    let viewport = size(20.0, 16.0);
    ui.layout(view, viewport)?;
    let node = ui.children(view, ui.root(view)?)?.next().unwrap();
    let mut local = Vec::new();
    ui.paint(view, node, &mut local)?;
    let Draw::Fill {
        clip: Some(clip), ..
    } = &local[1]
    else {
        panic!("expected a clipped fill");
    };
    assert_eq!(clip.x, Some(px(4.0)..px(6.0)));
    assert_eq!(clip.y, Some(px(4.0)..px(8.0)));

    let scale = RasterScale(1.5);
    let physical = scale.buffer_size(viewport)?;
    for _ in 0..2 {
        let frame = ui.record_frame(view)?;
        let image = Surface::draw_view(&frame, physical, scale)?;
        for (i, pixel) in image.rgba.as_chunks::<4>().0.iter().enumerate() {
            let x = i % physical.width as usize;
            let y = i / physical.width as usize;
            let color = match (x, y) {
                (6..9, 6..9) => [0, 255, 0, 255],
                (9..12, 9..15) => [0, 0, 255, 255],
                (9..21, 6..18) => [255, 0, 0, 255],
                _ => [255; 4],
            };
            assert_eq!(*pixel, color, "pixel ({x}, {y})");
        }
    }
    Ok(())
}

#[test]
fn nested_offsets_place_new_clips_relative_to_the_drawing_origin() -> Result<()> {
    let mut ui = Ui::new();
    let view = ui.mount(component(|_| {
        div().p(px(2.)).paint(|cx| {
            cx.with_clip(at(1., 2.), size(4., 3.), |cx| {
                cx.with_offset(at(-0.5, 1.5), |cx| {
                    cx.with_clip(at(0.5, 1.), size(3.5, 3.), |cx| {
                        cx.with_offset(at(1., 1.5), |cx| {
                            cx.fill(size(8., 8.), rgb(0xff0000));
                            Ok(())
                        })
                    })
                })
            })?;
            cx.fill(size(1., 1.), rgb(0x0000ff));
            Ok(())
        })
    }))?;
    let viewport = size(12., 12.);
    ui.layout(view, viewport)?;
    let scale = RasterScale(2.);
    let physical = scale.buffer_size(viewport)?;
    let image = Surface::draw_view(&ui.record_frame(view)?, physical, scale)?;
    for (i, pixel) in image.rgba.as_chunks::<4>().0.iter().enumerate() {
        let x = i % physical.width as usize;
        let y = i / physical.width as usize;
        let expected = match (x, y) {
            (6..11, 10..14) => [255, 0, 0, 255],
            (4..6, 4..6) => [0, 0, 255, 255],
            _ => [255; 4],
        };
        assert_eq!(*pixel, expected, "pixel ({x}, {y})");
    }
    Ok(())
}

#[test]
fn a_caught_clip_error_allows_the_element_to_paint_a_fallback() -> Result<()> {
    let mut ui = Ui::new();
    let ready = ui.context().signal(false);
    let view = ui.mount(component(|_| {
        div().paint(move |cx| {
            let attempt = cx.with_clip(at(1.0, 1.0), size(2.0, 2.0), |cx| {
                cx.fill(cx.size, rgb(0xff0000));
                cx.request_animation_frame();
                anyhow::ensure!(*cx.read(ready)?, "missing plot data");
                Ok(())
            });
            if attempt.is_err() {
                cx.fill(cx.size, rgb(0x00ff00));
            }
            Ok(())
        })
    }))?;
    ui.layout(view, size(4.0, 4.0))?;
    let frame = ui.record_frame(view)?;
    assert!(
        matches!(frame.draws(), [Draw::Fill { clip: None, fill, .. }] if *fill == rgb(0x00ff00).into())
    );
    assert!(!frame.requests_animation_frame());
    ui.context().set(ready, true)?;
    ui.flush()?;
    let frame = ui.record_frame(view)?;
    assert!(
        matches!(frame.draws(), [Draw::Fill { clip: Some(_), fill, .. }] if *fill == rgb(0xff0000).into())
    );
    assert!(frame.requests_animation_frame());
    Ok(())
}
