use super::*;
use solor::{color_stop, gradient_left_to_right, layout::Size, px, raster::DeviceSize, rgb, rgba};

fn point(x: f32, y: f32) -> Origin {
    Origin { x: px(x), y: px(y) }
}

const SIZE: DeviceSize = DeviceSize {
    width: 18,
    height: 18,
};

fn render(points: &[Origin], fill: Fill, scale: RasterScale, clip: Clip) -> Result<Surface> {
    let mut image = Surface::cleared(SIZE, scale, 255)?;
    image.fill_polygon(
        point(0.5, 1.0),
        &[Rc::from(points)],
        &fill,
        FillRule::EvenOdd,
        scale,
        clip,
    )?;
    Ok(image)
}

fn pixel(image: &Surface, x: usize, y: usize) -> &[u8] {
    let offset = (y * SIZE.width as usize + x) * 4;
    &image.rgba[offset..offset + 4]
}

#[test]
fn compound_fill_respects_holes_and_shares_its_gradient_across_islands() -> anyhow::Result<()> {
    use solor::{Ui, component, div};

    let rect = |left, top, right, bottom| {
        [
            point(left, top),
            point(right, top),
            point(right, bottom),
            point(left, bottom),
        ]
    };
    let size = Size {
        width: px(12.),
        height: px(12.),
    };
    let scale = RasterScale(1.5);
    let gradient =
        gradient_left_to_right([color_stop(rgb(0xff0000), 0.), color_stop(rgb(0x0000ff), 1.)])?;
    for (rule, reverse_inner, hole) in [
        (FillRule::EvenOdd, false, true),
        (FillRule::NonZero, false, false),
        (FillRule::NonZero, true, true),
    ] {
        let mut inner = rect(2., 2., 6., 8.);
        if reverse_inner {
            inner.reverse();
        }
        let rings = [rect(1., 1., 7., 9.), inner, rect(9., 3., 11., 7.)];
        let fill = gradient.clone();
        let mut ui = Ui::new();
        let view = ui.mount(component(move |_| {
            div().paint(move |cx| cx.fill_contours(rings, fill.clone(), rule))
        }))?;
        ui.layout(view, size)?;
        let image = Surface::draw_view(&ui.record_frame(view)?, SIZE, scale)?;
        for (x, filled) in [(2, true), (5, !hole), (12, false), (15, true)] {
            let expected = if filled {
                let position = ((x as f32 + 0.5) / 1.5 - 1.) / 10.;
                let color = gradient.sample(position)?;
                [color.red, color.green, color.blue, 255]
            } else {
                [255; 4]
            };
            assert_eq!(pixel(&image, x, 7), expected);
        }
    }
    Ok(())
}

#[test]
fn winding_rule_selects_the_star_center_during_full_and_clipped_replay() -> anyhow::Result<()> {
    use solor::{Ui, component, div};

    let scale = RasterScale(1.5);
    let size = Size {
        width: px(12.),
        height: px(12.),
    };
    let crop = Clip {
        left: 8,
        top: 4,
        right: 13,
        bottom: 14,
    };
    for rule in [FillRule::EvenOdd, FillRule::NonZero] {
        let mut ui = Ui::new();
        let view = ui.mount(component(move |_| {
            div().paint(move |cx| {
                cx.fill_polygon_with_rule(
                    [
                        point(6., 0.),
                        point(10., 11.),
                        point(0.5, 4.),
                        point(11.5, 4.),
                        point(2., 11.),
                    ],
                    rgba(0xff000080),
                    rule,
                )
            })
        }))?;
        ui.layout(view, size)?;
        let frame = ui.record_frame(view)?;
        let full = Surface::draw_view(&frame, SIZE, scale)?;
        let red = [255, 127, 127, 255];
        assert_eq!(pixel(&full, 9, 4), red);
        assert_eq!(
            pixel(&full, 9, 8),
            if rule == FillRule::NonZero {
                red
            } else {
                [255; 4]
            }
        );
        let mut part = Surface::cleared(SIZE, scale, 255)?;
        part.replay(frame.draws(), scale, crop, &mut Default::default())?;
        for y in crop.top..crop.bottom {
            for x in crop.left..crop.right {
                assert_eq!(
                    pixel(&part, x as usize, y as usize),
                    pixel(&full, x as usize, y as usize)
                );
            }
        }
    }
    Ok(())
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

#[test]
fn diagonal_stripes_match_box_paint_across_a_clipped_polygon() -> anyhow::Result<()> {
    use solor::{Styled, Ui, component, diagonal_stripes, div};

    let scale = RasterScale(1.5);
    let points = [
        point(0.5, 0.),
        point(10.5, 0.),
        point(10.5, 10.),
        point(0.5, 10.),
    ];
    let clip = Clip {
        left: 4,
        top: 4,
        right: 14,
        bottom: 12,
    };
    let logical = Size {
        width: px(12.),
        height: px(12.),
    };
    // Convert distances along a device-pixel row to perpendicular logical widths.
    let width = |n| px(n / (1.5 * std::f32::consts::SQRT_2));
    for (ink, gap, expected) in [(2., 2., [128u8, 255, 0]), (0.0625, 0.1875, [191; 3])] {
        let stripes = diagonal_stripes(rgb(0), width(ink), width(gap))?;
        let mut ui = Ui::new();
        let view = ui.mount(component(move |_| {
            div().child(
                div()
                    .absolute()
                    .left(px(1.))
                    .top(px(1.))
                    .size(px(10.))
                    .bg(stripes),
            )
        }))?;
        ui.layout(view, logical)?;
        let background = Surface::draw_view(&ui.record_frame(view)?, SIZE, scale)?;
        let part = render(&points, stripes.into(), scale, clip)?;
        for (x, gray) in [4, 5, 7].into_iter().zip(expected) {
            assert!(pixel(&background, x, 4)[0].abs_diff(gray) <= 1);
        }
        for y in 4..12 {
            for x in 4..14 {
                assert_eq!(pixel(&background, x, y), pixel(&part, x, y));
            }
        }
    }
    Ok(())
}
