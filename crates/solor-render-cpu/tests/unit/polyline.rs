use super::*;
use solor::{px, raster::DeviceSize, rgba};

fn point(x: f32, y: f32) -> Origin {
    Origin { x: px(x), y: px(y) }
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
            let mut image = Surface::white(size, scale)?;
            image.stroke_polyline(origin, &points, px(2.5), rgba(0xff000080), scale, clip);
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
