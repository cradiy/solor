use super::*;
use solor::{px, raster::DeviceSize, rgba};

fn point(x: f32, y: f32) -> Origin {
    Origin { x: px(x), y: px(y) }
}

#[test]
fn polygon_outline_closes_and_blends_the_seam_once() -> anyhow::Result<()> {
    use solor::{Ui, component, div};

    let mut ui = Ui::new();
    let view = ui.mount(component(|_| {
        div().paint(|cx| {
            cx.stroke_polygon(
                [point(4., 4.), point(16., 4.), point(4., 16.)],
                px(3.),
                rgba(0xff000080),
            )
        })
    }))?;
    let size = Size {
        width: px(20.),
        height: px(20.),
    };
    ui.layout(view, size)?;
    let frame = ui.record_frame(view)?;
    let scale = RasterScale(1.5);
    let physical = scale.buffer_size(size)?;
    let image = Surface::draw_view(&frame, physical, scale)?;
    let pixel = |x: usize, y: usize| {
        let i = (y * physical.width as usize + x) * 4;
        &image.rgba[i..i + 4]
    };
    assert_eq!(pixel(6, 15), [255, 127, 127, 255]); // Closing edge.
    assert_eq!(pixel(6, 6), [255, 127, 127, 255]); // First vertex.
    assert_eq!(pixel(12, 12), [255; 4]); // Interior.
    Ok(())
}

#[test]
fn clipped_gradient_uses_the_full_retraced_stroke_box() -> anyhow::Result<()> {
    use solor::{Ui, color_stop, component, div, gradient_left_to_right};

    let ramp = gradient_left_to_right([
        color_stop(rgba(0xff000080), 0.0),
        color_stop(rgba(0x00ff0080), 0.5),
        color_stop(rgba(0x0000ff80), 1.0),
    ])?;
    let ink = Ink::from(ramp.clone());
    let mut ui = Ui::new();
    let view = ui.mount(component(move |_| {
        div().paint(move |cx| {
            cx.stroke_polyline(
                [point(4., 8.), point(20., 8.), point(4., 8.)],
                px(4.),
                ink.clone(),
            )
        })
    }))?;
    let size = Size {
        width: px(24.),
        height: px(16.),
    };
    ui.layout(view, size)?;
    let frame = ui.record_frame(view)?;
    let scale = RasterScale(1.5);
    let physical = scale.buffer_size(size)?;
    let full = Surface::draw_view(&frame, physical, scale)?;
    let mut partial = Surface::cleared(physical, scale, 255)?;
    let crop = Clip {
        left: 6,
        right: 29,
        top: 8,
        bottom: 16,
    };
    partial.replay(frame.draws(), scale, crop, &mut Default::default())?;
    let stride = physical.width as usize * 4;
    for y in crop.top..crop.bottom {
        let row = y as usize * stride;
        let span = row + crop.left as usize * 4..row + crop.right as usize * 4;
        assert_eq!(partial.rgba[span.clone()], full.rgba[span]);
    }
    let stroke_box = Size {
        width: px(20.),
        height: px(4.),
    };
    for x in [6, 17, 28] {
        let color = ramp.sample_at((x as f32 + 0.5) / 1.5 - 2., 2., stroke_box)?;
        let expected = [color.red, color.green, color.blue, 255]
            .map(|c| ((u32::from(c) * 128 + 255 * 127 + 127) / 255) as u8);
        let i = 12 * stride + x * 4;
        assert_eq!(partial.rgba[i..i + 4], expected);
    }
    Ok(())
}

#[test]
fn retraced_translucent_stroke_blends_once_and_clipped_replay_preserves_coverage()
-> std::io::Result<()> {
    let size = DeviceSize {
        width: 21,
        height: 19,
    };
    let points = [
        point(0.5, 4.5),
        point(8.5, 4.5),
        point(8.5, 4.5),
        point(0.5, 4.5),
        point(4.5, 0.5),
        point(4.5, 8.5),
    ];
    let crop = Clip {
        left: 6,
        right: 13,
        top: 3,
        bottom: 9,
    };
    for scale in [RasterScale::ONE, RasterScale(1.5)] {
        let origin = point(3.0, 2.0);
        let render = |clip| -> std::io::Result<Surface> {
            let mut image = Surface::cleared(size, scale, 255)?;
            image.stroke_polyline(
                origin,
                Contour {
                    points: &points,
                    closed: false,
                },
                px(2.5),
                &rgba(0xff000080).into(),
                scale,
                clip,
            )?;
            Ok(image)
        };
        let full = render(Clip::canvas(size))?;
        let partial = render(crop)?;
        if scale == RasterScale::ONE {
            let pixel = |x, y| &full.rgba[(21 * y + x) * 4..(21 * y + x + 1) * 4];
            assert_eq!(pixel(7usize, 6usize), [255, 127, 127, 255]);
            assert_eq!(pixel(10, 7), [255, 159, 159, 255]);
            assert_eq!(pixel(13, 6), [255; 4]);
        }
        let bounds = bounds(origin, &points, px(2.5), scale)?;
        for (i, (a, b)) in full
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .zip(partial.rgba.as_chunks::<4>().0)
            .enumerate()
        {
            let (x, y) = ((i % 21) as i64, (i / 21) as i64);
            let inside = |r: Clip| (r.left..r.right).contains(&x) && (r.top..r.bottom).contains(&y);
            assert!(a == &[255; 4] || inside(bounds));
            assert_eq!(b, if inside(crop) { a } else { &[255; 4] });
        }
    }
    Ok(())
}
