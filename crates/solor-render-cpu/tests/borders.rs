//! Pixel coverage, layout and invalidation for styled borders.
use anyhow::Result;
use solor::{
    Styled, Ui, color_stop, component, div, gradient_left_to_right,
    layout::{Origin, Size},
    px,
    raster::RasterScale,
    rgb, rgba,
};
use solor_render_cpu::pixels::Surface;

fn pixel(surface: &Surface, x: usize, y: usize) -> &[u8] {
    let start = (y * surface.size.width as usize + x) * 4;
    &surface.rgba[start..start + 4]
}

#[test]
fn border_width_moves_content_and_anchors_while_ink_only_repaints() -> Result<()> {
    let mut ui = Ui::new();
    let width = ui.context().signal(px(3.));
    let ink = ui.context().signal(rgb(0x226699));
    let view = ui.mount(component(move |_| {
        div()
            .p(px(2.))
            .border_l(width)
            .border_left_color(ink)
            .border_t(px(2.))
            .border_top_color(rgb(0x993322))
            .child(div().size(px(4.)).bg(rgb(0)))
            .child(
                div()
                    .absolute()
                    .left(px(0.))
                    .top(px(0.))
                    .size(px(1.))
                    .bg(rgb(0x00ff00)),
            )
    }))?;
    let size = Size {
        width: px(30.),
        height: px(20.),
    };
    let root = ui.root(view)?;
    let nodes: Vec<_> = ui.children(view, root)?.collect();
    for (w, color) in [
        (3., rgb(0x226699)),
        (5., rgb(0x226699)),
        (5., rgb(0x114466)),
    ] {
        ui.context().set(width, px(w))?;
        ui.context().set(ink, color)?;
        ui.flush()?;
        if w == 5. && color == rgb(0x114466) {
            assert!(ui.geometry(view, nodes[0]).is_some());
        }
        ui.layout(view, size)?;
        assert_eq!(
            ui.geometry(view, nodes[0]).unwrap().origin,
            Origin {
                x: px(w + 2.),
                y: px(4.)
            }
        );
        assert_eq!(
            ui.geometry(view, nodes[1]).unwrap().origin,
            Origin {
                x: px(w),
                y: px(2.)
            }
        );
        let scale = RasterScale::ONE;
        let surface = Surface::draw_view(&ui.record_frame(view)?, scale.buffer_size(size)?, scale)?;
        assert_eq!(
            pixel(&surface, 1, 9),
            [color.red, color.green, color.blue, 255]
        );
        assert_eq!(pixel(&surface, 10, 0), [0x99, 0x33, 0x22, 255]);
        assert_eq!(pixel(&surface, w as usize, 2), [0, 255, 0, 255]);
        assert_eq!(pixel(&surface, w as usize + 2, 4), [0, 0, 0, 255]);
    }
    Ok(())
}

#[test]
fn rounded_translucent_edges_meet_without_a_seam_at_fractional_scale() -> Result<()> {
    let mut ui = Ui::new();
    let ramp = gradient_left_to_right([
        color_stop(rgba(0xff000080), 0.),
        color_stop(rgba(0x0000ff80), 1.),
    ])?;
    let view = ui.mount(component(move |_| {
        div()
            .border(px(4.))
            .rounded(px(6.))
            .border_color(rgba(0xff000080))
            .border_top_fill(ramp.clone())
            .border_left_color(rgba(0x0000ff80))
    }))?;
    let size = Size {
        width: px(40.),
        height: px(30.),
    };
    ui.layout(view, size)?;
    let scale = RasterScale(1.5);
    let image = Surface::draw_view(&ui.record_frame(view)?, scale.buffer_size(size)?, scale)?;
    assert_eq!(pixel(&image, 0, 0), [255; 4]);
    assert_eq!(pixel(&image, 9, 9), [255; 4]);
    assert_eq!(pixel(&image, 3, 20), [127, 127, 255, 255]);
    assert_eq!(pixel(&image, 30, 42), [255, 127, 127, 255]);
    // Both inks have zero green and the same alpha. The diagonal join must cover exactly once.
    for y in 2..6 {
        for x in 4..10 {
            assert_eq!(pixel(&image, x, y)[1], 127, "({x},{y})");
        }
    }
    assert!(pixel(&image, 15, 2)[0] > pixel(&image, 45, 2)[0]);
    Ok(())
}

#[test]
fn moving_a_fractional_rule_repairs_the_old_and_new_coverage() -> Result<()> {
    use solor::platform::{WindowEvent, WindowHandler};
    use solor_render_cpu::CpuWindow;
    use std::time::Duration;

    let mut window = CpuWindow::mount(
        Ui::new(),
        component(|cx| {
            let width = cx.signal(px(2.));
            div()
                .bg(rgb(0xffffff))
                .border_b(width)
                .border_bottom_color(rgb(0))
                .on_frame(move |_, cx| cx.set(width, px(0.5)))
        }),
    )?;
    let size = Size {
        width: px(120.),
        height: px(40.),
    };
    let scale = RasterScale(1.5);
    window.frame(size, scale)?;
    window.frame_submitted()?;
    window.event(WindowEvent::FrameReady(Duration::ZERO))?;
    let frame = window.frame(size, scale)?;
    let damage = frame.damage.bounds().unwrap();
    assert_eq!((damage.top, damage.bottom), (57, 60));
    let width = frame.size.width as usize;
    for (y, gray) in [(57, 255), (58, 255), (59, 64)] {
        let i = 4 * (y * width + 5);
        assert_eq!(&frame.rgba[i..i + 4], &[gray, gray, gray, 255]);
    }
    Ok(())
}
