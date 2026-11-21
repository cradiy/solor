//! Bitmap sizing, sampling and retained backgrounds on an offscreen CPU surface.
use anyhow::Result;
use solor::{
    ImageAsset, Styled, Ui, component, div, image, layout::Size, percent, px, raster::RasterScale,
    rgb, style::ObjectFit,
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
