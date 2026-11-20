use super::*;
use solor::{color_stop, gradient_left_to_right, raster::DeviceSize, rgb, rgba};

fn point(x: f32, y: f32) -> Origin {
    Origin { x: px(x), y: px(y) }
}

const SIZE: DeviceSize = DeviceSize {
    width: 18,
    height: 18,
};

fn render(points: &[Origin], fill: Fill, scale: RasterScale, clip: Clip) -> Result<Surface> {
    let mut image = Surface::white(SIZE, scale)?;
    image.fill_polygon(point(0.5, 1.0), points, &fill, scale, clip)?;
    Ok(image)
}

fn pixel(image: &Surface, x: usize, y: usize) -> &[u8] {
    let offset = (y * SIZE.width as usize + x) * 4;
    &image.rgba[offset..offset + 4]
}

#[test]
fn concave_contour_composites_fractional_coverage_once_and_double_winding_cancels() -> Result<()> {
    let points = [
        point(1.0, 1.0),
        point(9.0, 1.0),
        point(9.0, 5.0),
        point(4.0, 5.0),
        point(4.0, 10.0),
        point(1.0, 10.0),
    ];
    let fill: Fill = rgba(0xff000080).into();
    let clip = Clip::canvas(SIZE);
    let image = render(&points, fill.clone(), RasterScale::ONE, clip)?;
    assert_eq!(pixel(&image, 1, 3), [255, 191, 191, 255]);
    assert_eq!(pixel(&image, 3, 9), [255, 127, 127, 255]);
    assert_eq!(pixel(&image, 7, 9), [255; 4]);
    let twice = render(&points.repeat(2), fill, RasterScale::ONE, clip)?;
    assert!(twice.rgba.iter().all(|&v| v == 255));
    Ok(())
}

#[test]
fn gradient_uses_unclipped_vertex_bounds_across_a_fractional_repair() -> anyhow::Result<()> {
    let points = [
        point(-1.0, 2.0),
        point(8.0, 0.0),
        point(11.0, 8.0),
        point(3.0, 10.0),
    ];
    let gradient = gradient_left_to_right([
        color_stop(rgb(0xff0000), 0.0),
        color_stop(rgb(0x00ff00), 0.5),
        color_stop(rgb(0x0000ff), 1.0),
    ])?;
    let scale = RasterScale(1.5);
    let clip = Clip {
        left: 6,
        right: 13,
        top: 4,
        bottom: 12,
    };
    let full = render(&points, gradient.clone().into(), scale, Clip::canvas(SIZE))?;
    let part = render(&points, gradient.clone().into(), scale, clip)?;
    let mut sample = gradient.sampler(Size {
        width: px(12.0),
        height: px(10.0),
    })?;
    let c = sample(7.5 / 1.5 + 0.5, 7.5 / 1.5 - 1.0)?;
    assert_eq!(pixel(&full, 7, 7), [c.red, c.green, c.blue, 255]);
    for y in 0..SIZE.height as usize {
        for x in 0..SIZE.width as usize {
            let expected = if (6..13).contains(&x) && (4..12).contains(&y) {
                pixel(&full, x, y)
            } else {
                &[255; 4]
            };
            assert_eq!(pixel(&part, x, y), expected);
        }
    }
    Ok(())
}
