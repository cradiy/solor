//! Foreground colors and mask coverage at fractional device scale.
use anyhow::Result;
use solor::{
    Draw, Ink, Styled, SvgAsset, Ui, color_stop, component, div, gradient_left_to_right,
    layout::Size, px, raster::RasterScale, rgb, rgba, svg, text,
};
use solor_render_cpu::pixels::Surface;

#[test]
fn clipped_glyphs_and_svg_sample_the_full_box_through_the_original_mask() -> Result<()> {
    let mut ui = Ui::with_font(
        include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf").to_vec(),
    )?;
    let ink = ui.context().signal(Ink::from(rgb(0)));
    let icon = SvgAsset::from_bytes(br#"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="20"><ellipse cx="40" cy="10" rx="39" ry="9" opacity=".7"/></svg>"#)?;
    let view = ui.mount(component(move |_| {
        div().items_start().p(px(3.25)).text_color(ink).child(
            div()
                .w(px(51.))
                .items_start()
                .overflow_hidden()
                .child(text("AB CD\nAB CD").w(px(80.)).text_size(px(18.)))
                .child(svg(icon).w(px(80.)).h(px(20.))),
        )
    }))?;
    let size = Size {
        width: px(100.),
        height: px(100.),
    };
    let scale = RasterScale(1.5);
    ui.layout(view, size)?;
    let black = ui.record_frame(view)?;
    let ramp = gradient_left_to_right([
        color_stop(rgba(0xb13112cc), 0.0),
        color_stop(rgb(0x246195), 0.45),
        color_stop(rgba(0xe4af2855), 1.0),
    ])?;
    ui.context().set(ink, ramp.clone().into())?;
    ui.flush()?;
    let colored = ui.record_frame(view)?;
    let physical = scale.buffer_size(size)?;
    let reference = Surface::draw_view(&black, physical, scale)?;
    let result = Surface::draw_view(&colored, physical, scale)?;
    assert!(matches!(
        colored.draws(),
        [Draw::Text { .. }, Draw::Svg { .. }]
    ));
    for draw in colored.draws() {
        let (at, size) = match draw {
            Draw::Text { origin, size, .. } | Draw::Svg { origin, size, .. } => (*origin, *size),
            _ => continue,
        };
        let bounds = scale.bounds(at, size);
        let mut edges = 0;
        for y in bounds.top..bounds.bottom {
            for x in bounds.left..bounds.right {
                let offset = (y as usize * physical.width as usize + x as usize) * 4;
                let coverage = 255 - reference.rgba[offset];
                let c = ramp.sample_at(
                    ((x as f64 + 0.5) / scale.0 - f64::from(at.x)) as f32,
                    ((y as f64 + 0.5) / scale.0 - f64::from(at.y)) as f32,
                    size,
                )?;
                let alpha = (u32::from(coverage) * u32::from(c.alpha) + 127) / 255;
                let expected = [c.red, c.green, c.blue, 255]
                    .map(|v| ((u32::from(v) * alpha + 255 * (255 - alpha) + 127) / 255) as u8);
                assert_eq!(&result.rgba[offset..offset + 4], &expected, "({x}, {y})");
                edges += usize::from(coverage > 0 && coverage < 255);
            }
        }
        assert!(edges > 0);
    }
    Ok(())
}
