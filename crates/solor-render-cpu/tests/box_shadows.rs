//! Pixel composition and damage of retained box shadows.
use anyhow::Result;
use solor::{
    Styled, Ui, component, div,
    layout::Size,
    platform::{WindowEvent, WindowHandler},
    px,
    raster::RasterScale,
    rgb,
    style::BoxShadow,
};
use solor_render_cpu::CpuWindow;
use std::time::Duration;

#[test]
fn shadows_respect_transparent_interiors_and_layer_order() -> Result<()> {
    let mut window = CpuWindow::mount(
        Ui::new(),
        component(|_| {
            div().p(px(10.)).child(
                div()
                    .size(px(24.))
                    .overflow_hidden()
                    .border(px(1.))
                    .border_color(rgb(0))
                    .shadow([
                        BoxShadow::new(px(0.), px(0.), rgb(0xcc0000)).spread(px(2.)),
                        BoxShadow::new(px(0.), px(0.), rgb(0x0000cc)).spread(px(4.)),
                        BoxShadow::new(px(3.), px(0.), rgb(0x00cc00)).inset(),
                    ]),
            )
        }),
    )?;
    let frame = window.frame(
        Size {
            width: px(50.),
            height: px(48.),
        },
        RasterScale::ONE,
    )?;
    let at = |x: usize, y: usize| &frame.rgba[4 * (y * 50 + x)..4 * (y * 50 + x) + 3];
    assert_eq!(at(6, 20), [0, 0, 204]);
    assert_eq!(at(8, 20), [204, 0, 0]);
    assert_eq!(at(10, 20), [0, 0, 0]);
    assert_eq!(at(11, 20), [0, 204, 0]);
    assert_eq!(at(17, 20), [255; 3]);
    Ok(())
}

#[test]
fn moving_a_soft_shadow_repairs_its_tail_through_an_ancestor_clip() -> Result<()> {
    let shadows = |x| [BoxShadow::new(px(x), px(0.), rgb(0)).blur(px(6.))];
    let scene = |initial| {
        component(move |cx| {
            let layers = cx.signal(shadows(initial));
            div().p(px(8.)).child(
                div()
                    .w(px(64.))
                    .h(px(40.))
                    .overflow_hidden()
                    .p(px(8.))
                    .child(
                        div()
                            .size(px(24.))
                            .rounded(px(6.))
                            .shadow(layers)
                            .on_frame(move |_, cx| cx.set(layers, shadows(-8.))),
                    ),
            )
        })
    };
    let size = Size {
        width: px(96.),
        height: px(60.),
    };
    let scale = RasterScale(1.5);
    let mut window = CpuWindow::mount(Ui::new(), scene(8.))?;
    let frame = window.frame(size, scale)?;
    let gray = |x: usize, y: usize| frame.rgba[4 * (y * frame.size.width as usize + x)];
    assert_eq!(gray(56, 40), 255);
    assert!(gray(66, 40) < gray(72, 40) && gray(72, 40) < gray(78, 40) && gray(78, 40) < 255);
    assert!(gray(66, 24) > gray(66, 40));
    window.frame_submitted()?;
    window.event(WindowEvent::FrameReady(Duration::ZERO))?;
    let changed = window.frame(size, scale)?;
    let damage = changed.damage.bounds().unwrap();
    assert!(damage.left >= 12 && damage.right < i64::from(changed.size.width));
    let mut reference = CpuWindow::mount(Ui::new(), scene(-8.))?;
    assert_eq!(changed.rgba, reference.frame(size, scale)?.rgba);
    let gray = |x: usize, y: usize| changed.rgba[4 * (y * changed.size.width as usize + x)];
    assert_eq!(gray(11, 40), 255);
    assert!(gray(12, 40) < 255);
    Ok(())
}
