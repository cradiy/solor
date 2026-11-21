//! Pixel and layout contracts for retained SVG artwork.
use anyhow::Result;
use solor::{
    Styled, SvgAsset, Ui, component, div, layout::Size, percent, px, raster::RasterScale, rem, rgb,
    rgba, style::ObjectFit, svg,
};
use solor_render_cpu::pixels::Surface;

const ART: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="4 2 16 8" width="16" height="8">
  <defs><linearGradient id="g"><stop stop-color="red"/><stop offset="1" stop-color="blue"/></linearGradient></defs>
  <path fill="url(#g)" d="M4 2h8v8H4z"/>
  <path fill="none" stroke="lime" stroke-opacity=".5" stroke-width="4" d="M12 6h8"/>
</svg>"##;

fn pixel(surface: &Surface, x: usize, y: usize) -> &[u8] {
    let index = (y * surface.size.width as usize + x) * 4;
    &surface.rgba[index..index + 4]
}

#[test]
fn inherited_mask_and_source_colors_survive_repaint_and_unmount() -> Result<()> {
    assert!(SvgAsset::from_bytes(b"<svg").is_err());
    let asset = SvgAsset::from_bytes(ART)?;
    let mut ui = Ui::new();
    let ink = ui.context().signal(rgba(0x0000ff80));
    let view = ui.mount(component(move |_| {
        div()
            .flex_row()
            .items_start()
            .text_color(ink)
            .child(svg(asset.clone()))
            .child(svg(asset).preserve_colors())
    }))?;
    let size = Size {
        width: px(32.0),
        height: px(12.0),
    };
    ui.layout(view, size)?;
    let before = ui.record_frame(view)?;
    ui.context().set(ink, rgb(0xff0000))?;
    ui.flush()?;
    let after = ui.record_frame(view)?;
    ui.unmount(view)?;
    for scale in [RasterScale::ONE, RasterScale(1.5), RasterScale(2.0)] {
        let physical = scale.buffer_size(size)?;
        let original = Surface::draw_view(&before, physical, scale)?;
        let changed = Surface::draw_view(&after, physical, scale)?;
        let at = |v: usize| (v as f64 * scale.0) as usize;
        assert_eq!(pixel(&original, at(2), at(4)), [127, 127, 255, 255]);
        assert_eq!(pixel(&changed, at(2), at(4)), [255, 0, 0, 255]);
        assert_eq!(pixel(&changed, at(12), at(4)), [255, 127, 127, 255]);
        assert_eq!(pixel(&changed, at(12), at(1)), [255; 4]);
        let red = pixel(&changed, at(17), at(4));
        let blue = pixel(&changed, at(23), at(4));
        assert!(red[0] > red[2] && blue[2] > blue[0]);
        assert_eq!(pixel(&changed, at(29), at(4)), [127, 255, 127, 255]);
        assert_eq!(
            pixel(&original, at(29), at(4)),
            pixel(&changed, at(29), at(4))
        );
    }
    Ok(())
}

#[test]
fn fit_and_position_use_the_padded_content_box_and_view_rem() -> Result<()> {
    let asset = SvgAsset::from_bytes(ART)?;
    let mut ui = Ui::new();
    let fit = ui.context().signal(ObjectFit::Contain);
    let view = ui.mount_with_rem_size(
        component(move |_| {
            div().items_start().child(
                svg(asset)
                    .size(px(12.0))
                    .p(px(2.0))
                    .object_fit(fit)
                    .object_position_x(percent(100.0))
                    .object_position_y(rem(0.5)),
            )
        }),
        px(4.0),
    )?;
    let size = Size {
        width: px(20.0),
        height: px(20.0),
    };
    let scale = RasterScale(1.5);
    ui.layout(view, size)?;
    let draw = |ui: &Ui| -> Result<_> {
        Ok(Surface::draw_view(
            &ui.record_frame(view)?,
            scale.buffer_size(size)?,
            scale,
        )?)
    };
    let contained = draw(&ui)?;
    assert_eq!(pixel(&contained, 4, 6), [0, 0, 0, 255]);
    assert_eq!(pixel(&contained, 4, 5), [255; 4]);
    ui.context().set(fit, ObjectFit::Cover)?;
    ui.flush()?;
    ui.layout(view, size)?;
    let covered = draw(&ui)?;
    assert_eq!(pixel(&covered, 4, 6), [255; 4]);
    assert_eq!(pixel(&covered, 4, 12), [127, 127, 127, 255]);
    assert_eq!(pixel(&covered, 2, 12), [255; 4]);
    assert_eq!(pixel(&covered, 15, 12), [255; 4]);
    Ok(())
}

#[test]
fn a_single_explicit_dimension_sizes_the_padded_svg_by_its_content_ratio() -> Result<()> {
    let asset = SvgAsset::from_bytes(ART)?;
    let mut ui = Ui::new();
    let view = ui.mount(component(move |_| {
        div()
            .items_start()
            .child(svg(asset.clone()).p(px(2.0)).h(px(12.0)))
            .child(svg(asset).p(px(2.0)).w(px(28.0)))
    }))?;
    ui.layout(
        view,
        Size {
            width: px(80.0),
            height: px(80.0),
        },
    )?;
    let sizes: Vec<_> = ui
        .children(view, ui.root(view)?)?
        .map(|id| ui.geometry(view, id).unwrap().metrics.size)
        .collect();
    assert_eq!(
        sizes,
        [
            Size {
                width: px(20.0),
                height: px(12.0)
            },
            Size {
                width: px(28.0),
                height: px(16.0)
            },
        ]
    );
    Ok(())
}

#[test]
fn a_window_repairs_only_the_tinted_icon_on_a_transparent_background() -> Result<()> {
    use solor::platform::{WindowBackground, WindowEvent, WindowHandler};
    use solor::raster::Clip;
    use solor_render_cpu::CpuWindow;

    let asset = SvgAsset::from_bytes(ART)?;
    let mut window = CpuWindow::mount_with_background(
        Ui::new(),
        component(move |cx| {
            let ink = cx.signal(rgb(0xff0000));
            div()
                .flex_row()
                .items_start()
                .on_frame(move |_, cx| cx.set(ink, rgba(0x0000ff80)))
                .text_color(ink)
                .child(svg(asset.clone()))
                .child(svg(asset).preserve_colors())
        }),
        WindowBackground::Transparent,
    )?;
    let size = Size {
        width: px(32.0),
        height: px(12.0),
    };
    let scale = RasterScale(1.5);
    let before = window.frame(size, scale)?.rgba.to_vec();
    window.frame_submitted()?;
    window.event(WindowEvent::FrameReady(std::time::Duration::ZERO))?;
    let updated = window.frame(size, scale)?;
    assert_eq!(
        updated.damage.bounds(),
        Some(Clip {
            left: 0,
            top: 0,
            right: 24,
            bottom: 12
        })
    );
    let row = 6 * 48 * 4;
    assert_eq!(&updated.rgba[row + 18 * 4..row + 19 * 4], [0, 0, 64, 64]);
    assert_eq!(&updated.rgba[row + 42 * 4..row + 43 * 4], [0, 128, 0, 128]);
    assert_eq!(
        &updated.rgba[row + 24 * 4..row + 48 * 4],
        &before[row + 24 * 4..row + 48 * 4]
    );
    Ok(())
}
