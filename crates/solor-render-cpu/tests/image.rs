//! Bitmap sizing, sampling and retained backgrounds on an offscreen CPU surface.
use anyhow::Result;
use solor::{
    ImageAsset, Styled, Ui, component, div, image,
    layout::Size,
    percent, px,
    raster::{Clip, RasterScale},
    rgb,
    style::ObjectFit,
};
use solor_render_cpu::pixels::Surface;

fn size(w: f32, h: f32) -> Size {
    Size {
        width: px(w),
        height: px(h),
    }
}
fn pixel(surface: &Surface, x: usize, y: usize) -> [u8; 4] {
    surface.rgba.as_chunks::<4>().0[y * surface.size.width as usize + x]
}

#[test]
fn shared_image_remeasures_the_leaf_and_repaints_the_rounded_background() -> Result<()> {
    let wide = ImageAsset::from_rgba8(
        4,
        2,
        [
            [255, 0, 0, 255],
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 255, 0, 255],
        ]
        .repeat(2)
        .into_flattened(),
    )?;
    let mut ui = Ui::new();
    let photo = ui.context().signal(wide);
    let fit = ui.context().signal(ObjectFit::Cover);
    let view = ui.mount(component(move |_| {
        div()
            .items_start()
            .child(
                div()
                    .size(px(12.))
                    .p(px(2.))
                    .rounded(px(6.))
                    .bg(photo)
                    .object_fit(fit)
                    .object_position_x(percent(100.))
                    .child(div().size(px(2.)).bg(rgb(0x111111))),
            )
            .child(image(photo).h(px(4.)))
    }))?;
    let viewport = size(20., 20.);
    ui.layout(view, viewport)?;
    let children: Vec<_> = ui.children(view, ui.root(view)?)?.collect();
    let bounds = |ui: &Ui, id| ui.geometry(view, id).unwrap().metrics.size;
    assert_eq!(bounds(&ui, children[1]), size(8., 4.));
    let scale = RasterScale(1.5);
    let physical = scale.buffer_size(viewport)?;
    let first = ui.record_frame(view)?;
    let draw = |ui: &Ui| -> Result<_> {
        Ok(Surface::draw_view(
            &ui.record_frame(view)?,
            physical,
            scale,
        )?)
    };
    let covered = draw(&ui)?;
    assert_eq!(pixel(&covered, 0, 0), [255; 4]);
    assert_eq!(pixel(&covered, 12, 2), [0, 255, 0, 255]);
    assert_eq!(pixel(&covered, 4, 4), [17, 17, 17, 255]);
    ui.context().set(fit, ObjectFit::Contain)?;
    ui.flush()?;
    ui.layout(view, viewport)?;
    let contained = draw(&ui)?;
    assert_eq!(pixel(&contained, 12, 2), [255; 4]);
    assert_eq!(pixel(&contained, 14, 9), [0, 255, 0, 255]);
    ui.context().set(
        photo,
        ImageAsset::from_rgba8(1, 2, [0, 0, 255, 255].repeat(2))?,
    )?;
    ui.flush()?;
    ui.layout(view, viewport)?;
    assert_eq!(
        ui.children(view, ui.root(view)?)?.collect::<Vec<_>>(),
        children
    );
    assert_eq!(bounds(&ui, children[0]), size(12., 12.));
    assert_eq!(bounds(&ui, children[1]), size(2., 4.));
    let changed = draw(&ui)?;
    assert_eq!(pixel(&changed, 14, 9), [0, 0, 255, 255]);
    assert_eq!(pixel(&changed, 6, 20), [255; 4]);
    ui.unmount(view)?;
    assert_eq!(
        Surface::draw_view(&first, physical, scale)?.rgba,
        covered.rgba
    );
    Ok(())
}

#[test]
fn switching_atlas_regions_only_remeasures_when_the_region_size_changes() -> Result<()> {
    let red = [255, 0, 0, 255];
    let green = [0, 255, 0, 255];
    let atlas = ImageAsset::from_rgba8(4, 1, [red, red, green, green].as_flattened().to_vec())?;
    let region = |left, right| {
        atlas.region(Clip {
            left,
            right,
            top: 0,
            bottom: 1,
        })
    };
    let fixed = region(0, 2)?;
    let mut ui = Ui::new();
    let tile = ui.context().signal(fixed.clone());
    let view = ui.mount(component(move |_| {
        div()
            .items_start()
            .child(image(fixed))
            .child(image(tile))
            .child(
                div()
                    .w(px(6.))
                    .h(px(2.))
                    .bg(tile)
                    .object_fit(ObjectFit::None)
                    .object_position_x(percent(100.))
                    .object_position_y(percent(0.)),
            )
    }))?;
    let viewport = size(6., 4.);
    ui.layout(view, viewport)?;
    let root = ui.root(view)?;
    let children: Vec<_> = ui.children(view, root)?.collect();
    assert_eq!(
        ui.geometry(view, children[0]).unwrap().metrics.size,
        size(2., 1.)
    );
    let scale = RasterScale::ONE;
    let physical = scale.buffer_size(viewport)?;
    let draw = |ui: &Ui| -> Result<_> {
        Ok(Surface::draw_view(
            &ui.record_frame(view)?,
            physical,
            scale,
        )?)
    };
    assert_eq!(pixel(&draw(&ui)?, 5, 2), red);
    for (left, right, relayout) in [(2, 4, false), (3, 4, true)] {
        ui.context().set(tile, region(left, right)?)?;
        ui.flush()?;
        assert_eq!(ui.layout(view, viewport)? > 0, relayout);
        let width = (right - left) as usize;
        assert_eq!(
            ui.geometry(view, children[1]).unwrap().metrics.size,
            size(width as f32, 1.)
        );
        let surface = draw(&ui)?;
        assert_eq!(pixel(&surface, 0, 0), red);
        assert_eq!(pixel(&surface, width - 1, 1), green);
        assert_eq!(pixel(&surface, width, 1), [255; 4]);
        assert_eq!(pixel(&surface, 5, 2), green);
        assert_eq!(pixel(&surface, 6 - width, 2), green);
        assert_eq!(pixel(&surface, 5 - width, 2), [255; 4]);
        assert_eq!(pixel(&surface, 5, 3), [255; 4]);
    }
    Ok(())
}

#[test]
fn bilinear_sampling_keeps_transparent_rgb_out_of_blended_edges() -> Result<()> {
    let asset = ImageAsset::from_rgba8(2, 1, vec![255, 0, 0, 0, 0, 0, 255, 255])?;
    let mut ui = Ui::new();
    let view = ui.mount(component(move |_| {
        div()
            .items_start()
            .child(
                image(asset.clone())
                    .w(px(4.))
                    .h(px(1.))
                    .object_fit(ObjectFit::Fill),
            )
            .child(
                div()
                    .w(px(4.))
                    .h(px(1.))
                    .bg(asset)
                    .object_fit(ObjectFit::Fill),
            )
    }))?;
    let extent = size(4., 2.);
    ui.layout(view, extent)?;
    let scale = RasterScale::ONE;
    let surface = Surface::draw_view(&ui.record_frame(view)?, scale.buffer_size(extent)?, scale)?;
    let expected = [
        [255; 4],
        [191, 191, 255, 255],
        [64, 64, 255, 255],
        [0, 0, 255, 255],
    ];
    assert_eq!(surface.rgba.as_chunks::<4>().0, expected.repeat(2));
    Ok(())
}

#[test]
fn native_bitmap_rows_respect_crop_letterboxing_and_alpha() -> Result<()> {
    let bytes: Vec<_> = (0..24u8)
        .flat_map(|i| [i * 9, i * 3, 0, [0, 128, 255][usize::from(i % 3)]])
        .collect();
    let mut atlas = [255, 0, 255, 255].repeat(8 * 5);
    for (y, row) in bytes.as_chunks::<24>().0.iter().enumerate() {
        let start = ((y + 1) * 8 + 2) * 4;
        atlas[start..start + row.len()].copy_from_slice(row);
    }
    let atlas = ImageAsset::from_rgba8(8, 5, atlas)?;
    let source = atlas.region(Clip {
        left: 2,
        top: 1,
        right: 8,
        bottom: 5,
    })?;
    let asset = ImageAsset::from_rgba8(6, 4, bytes)?;
    let mut ui = Ui::new();
    let view = ui.mount(component(move |_| {
        div().paint(move |cx| cx.paint_image(Default::default(), cx.size, &source, ObjectFit::None))
    }))?;
    for (width, height, left, top) in [(10, 2, 2, -1), (4, 6, -1, 1)] {
        let extent = size(width as f32, height as f32);
        ui.layout(view, extent)?;
        let surface = Surface::draw_view(
            &ui.record_frame(view)?,
            RasterScale::ONE.buffer_size(extent)?,
            RasterScale::ONE,
        )?;
        for y in 0..height {
            for x in 0..width {
                let (sx, sy) = (x - left, y - top);
                let expected = if (0..6).contains(&sx) && (0..4).contains(&sy) {
                    let [r, g, b, a] = asset.pixels()[(sy * 6 + sx) as usize];
                    [r + (255 - a), g + (255 - a), b + (255 - a), 255]
                } else {
                    [255; 4]
                };
                assert_eq!(pixel(&surface, x as usize, y as usize), expected);
            }
        }
    }
    Ok(())
}

#[test]
fn canvas_bitmap_fit_matches_the_image_element_at_a_fractional_offset() -> Result<()> {
    use solor::layout::Origin;
    let mut ui = Ui::new();
    let fit = ui.context().signal(ObjectFit::Contain);
    let row = [[255, 0, 0, 255], [0, 255, 0, 128], [0, 0, 255, 255]];
    let asset = ImageAsset::from_rgba8(3, 1, row.as_flattened().to_vec())?;
    let position = Origin {
        x: px(2.25),
        y: px(1.5),
    };
    let tile = size(4., 6.);
    let background = rgb(0x273851);
    let mut pixels = [[255, 0, 255, 255]; 15];
    pixels[6..9].copy_from_slice(&row);
    let atlas = ImageAsset::from_rgba8(5, 3, pixels.as_flattened().to_vec())?;
    let source = atlas.region(Clip {
        left: 1,
        top: 1,
        right: 4,
        bottom: 2,
    })?;
    let canvas = ui.mount(component(move |_| {
        div()
            .bg(background)
            .paint(move |cx| cx.paint_image(position, tile, &source, *cx.read(fit)?))
    }))?;
    let element = ui.mount(component(move |_| {
        div().bg(background).child(
            image(asset)
                .absolute()
                .left(position.x)
                .top(position.y)
                .w(tile.width)
                .h(tile.height)
                .object_fit(fit),
        )
    }))?;
    let viewport = size(10., 9.);
    let scale = RasterScale(1.5);
    let physical = scale.buffer_size(viewport)?;
    for mode in [
        ObjectFit::Contain,
        ObjectFit::Cover,
        ObjectFit::Fill,
        ObjectFit::None,
        ObjectFit::ScaleDown,
    ] {
        ui.context().set(fit, mode)?;
        ui.flush()?;
        ui.layout(canvas, viewport)?;
        ui.layout(element, viewport)?;
        let painted = Surface::draw_view(&ui.record_frame(canvas)?, physical, scale)?;
        let styled = Surface::draw_view(&ui.record_frame(element)?, physical, scale)?;
        assert_eq!(painted.rgba, styled.rgba, "{mode:?}");
        assert_ne!(pixel(&painted, 6, 6), [0x27, 0x38, 0x51, 255]);
    }
    Ok(())
}

#[test]
fn clipped_scaled_bitmap_matches_individual_scanlines() -> Result<()> {
    use solor::layout::Origin;
    let scale = RasterScale(1.5);
    let viewport = size(24., 16.);
    let pixels = (0..35u8)
        .flat_map(|i| [i * 7, 255 - i, i * 3, [0, 128, 255][usize::from(i % 3)]])
        .collect();
    let asset = ImageAsset::from_rgba8(7, 5, pixels)?;
    let draw = |rows: usize| -> Result<Surface> {
        let mut ui = Ui::new();
        let source = asset.clone();
        let view = ui.mount(component(move |_| {
            div().bg(rgb(0x273851)).paint(move |cx| {
                // The same crop starts inside the fitted image at a fractional origin.
                for y in (2..22).step_by(rows) {
                    let at = Origin {
                        x: px(2.),
                        y: px(y as f32 / 1.5),
                    };
                    cx.with_clip(at, size(18., rows.min(22 - y) as f32 / 1.5), |cx| {
                        cx.fill_rounded_rect(
                            Origin {
                                x: px(1.25),
                                y: px(0.5),
                            },
                            size(20.5, 15.25),
                            px(2.75),
                            source.clone(),
                        )
                    })?;
                }
                Ok(())
            })
        }))?;
        ui.layout(view, viewport)?;
        Ok(Surface::draw_view(
            &ui.record_frame(view)?,
            scale.buffer_size(viewport)?,
            scale,
        )?)
    };
    let whole = draw(20)?;
    assert_ne!(pixel(&whole, 10, 10), [0x27, 0x38, 0x51, 255]);
    assert_eq!(whole.rgba, draw(1)?.rgba);
    assert_eq!(whole.rgba, draw(4)?.rgba);
    Ok(())
}
