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
    use solor::style::{BorderPaint, BorderStyle};
    use std::rc::Rc;

    let mut ui = Ui::new();
    let ramp = gradient_left_to_right([
        color_stop(rgba(0xff000080), 0.),
        color_stop(rgba(0x0000ff80), 1.),
    ])?;
    let mut border = BorderPaint::solid(px(4.), rgba(0xff000080));
    border.radii = px(6.).into();
    border.fills.top = Some(ramp.clone().into());
    border.fills.left = Some(rgba(0x0000ff80).into());
    border.styles.right = BorderStyle::None;
    let border = Rc::new(border);
    let painted = ui.mount(component(move |_| {
        div().paint(move |cx| cx.stroke_rect(Origin::default(), cx.size, border.clone()))
    }))?;
    let view = ui.mount(component(move |_| {
        div()
            .border(px(4.))
            .rounded(px(6.))
            .border_color(rgba(0xff000080))
            .border_right_style(BorderStyle::None)
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
    ui.layout(painted, size)?;
    let custom = Surface::draw_view(&ui.record_frame(painted)?, scale.buffer_size(size)?, scale)?;
    assert_eq!(custom.rgba, image.rgba);
    assert_eq!(pixel(&image, 58, 20), [255; 4]);
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

#[test]
fn a_signal_switches_a_single_rule_between_dashes_and_round_dots() -> Result<()> {
    use solor::style::BorderStyle;
    let mut ui = Ui::new();
    let pattern = ui.context().signal(BorderStyle::Dashed);
    let view = ui.mount(component(move |_| {
        div()
            .border_t(px(8.))
            .border_top_color(rgb(0))
            .border_top_style(pattern)
    }))?;
    let size = Size {
        width: px(80.),
        height: px(16.),
    };
    ui.layout(view, size)?;
    let root = ui.root(view)?;
    let geometry = ui.geometry(view, root);
    let render = |ui: &mut Ui| -> Result<Surface> {
        Ok(Surface::draw_view(
            &ui.record_frame(view)?,
            RasterScale::ONE.buffer_size(size)?,
            RasterScale::ONE,
        )?)
    };
    let dashed = render(&mut ui)?;
    assert_eq!(pixel(&dashed, 4, 3), [0, 0, 0, 255]);
    assert_eq!(pixel(&dashed, 18, 3), [255; 4]);
    assert_eq!(pixel(&dashed, 75, 3), [0, 0, 0, 255]);
    ui.context().set(pattern, BorderStyle::Dotted)?;
    ui.flush()?;
    assert_eq!(ui.geometry(view, root), geometry);
    let dotted = render(&mut ui)?;
    // The first dot has its center at (4,4) and radius 4.
    assert_eq!(pixel(&dotted, 0, 0), [255; 4]);
    assert_eq!(pixel(&dotted, 4, 1), [0, 0, 0, 255]);
    assert_eq!(pixel(&dotted, 1, 4), [0, 0, 0, 255]);
    assert_eq!(pixel(&dotted, 9, 4), [255; 4]);
    Ok(())
}

#[test]
fn a_partial_repair_preserves_the_pattern_phase_through_a_rounded_corner() -> Result<()> {
    use solor::{
        Render,
        platform::{WindowEvent, WindowHandler},
        style::BorderStyle,
    };
    use solor_render_cpu::CpuWindow;
    use std::time::Duration;

    fn scene(changed: bool) -> impl Render {
        component(move |cx| {
            let fill = cx.signal(if changed {
                rgb(0x203040)
            } else {
                rgb(0xffffff)
            });
            div()
                .bg(rgb(0xffffff))
                .child(
                    div()
                        .absolute()
                        .left(px(6.25))
                        .top(px(0.))
                        .w(px(19.))
                        .h(px(10.))
                        .bg(fill)
                        .on_frame(move |_, cx| cx.set(fill, rgb(0x203040))),
                )
                .child(
                    div()
                        .absolute()
                        .inset(px(0.))
                        .border(px(3.))
                        .rounded(px(12.))
                        .border_dashed()
                        .border_right_style(BorderStyle::Dotted)
                        .border_color(rgba(0x000000c0)),
                )
        })
    }
    let size = Size {
        width: px(72.),
        height: px(36.),
    };
    let scale = RasterScale(1.5);
    let mut window = CpuWindow::mount(Ui::new(), scene(false))?;
    let initial = window.frame(size, scale)?;
    let gray = |x: usize, y: usize| initial.rgba[(y * initial.size.width as usize + x) * 4];
    // Follow the top-left arc from the shared corner dash into the next gap.
    assert!(gray(6, 6) < 100);
    assert_eq!(gray(12, 3), 255);
    let first = initial.rgba.to_vec();
    window.frame_submitted()?;
    window.event(WindowEvent::FrameReady(Duration::ZERO))?;
    let repaired = window.frame(size, scale)?;
    assert!(repaired.damage.bounds().unwrap().right < i64::from(repaired.size.width));
    assert_ne!(first, repaired.rgba);
    let mut reference = CpuWindow::mount(Ui::new(), scene(true))?;
    assert_eq!(repaired.rgba, reference.frame(size, scale)?.rgba);
    Ok(())
}

#[test]
fn moving_an_outline_repaints_both_rings_inside_the_ancestor_clip() -> Result<()> {
    use solor::platform::{WindowEvent, WindowHandler};
    use solor_render_cpu::CpuWindow;
    use std::time::Duration;

    let scene = |initial: f32| {
        component(move |cx| {
            let offset = cx.signal(px(initial));
            div().p(px(8.)).child(
                div().size(px(24.)).overflow_hidden().child(
                    div()
                        .absolute()
                        .top(px(4.))
                        .left(px(1.))
                        .size(px(16.))
                        .rounded(px(2.))
                        .overflow_hidden()
                        .bg(rgb(0x447799))
                        .outline_width(px(2.))
                        .outline_offset(offset)
                        .outline_color(rgb(0))
                        .on_frame(move |_, cx| cx.set(offset, px(-3.))),
                ),
            )
        })
    };
    let size = Size {
        width: px(80.),
        height: px(48.),
    };
    let scale = RasterScale(1.5);
    let mut window = CpuWindow::mount(Ui::new(), scene(2.))?;
    let first = window.frame(size, scale)?;
    let at = |x: usize, y: usize| 4 * (y * first.size.width as usize + x);
    let ink = |x, y| first.rgba[at(x, y)];
    // Top edge at logical y=8..10, with a two-pixel gap to the card.
    assert_eq!((ink(24, 13), ink(24, 16), ink(24, 22)), (0, 255, 0x44));
    // The left edge extends beyond the ancestor at x=8.
    assert_eq!(ink(8, 27), 255);
    window.frame_submitted()?;
    window.event(WindowEvent::FrameReady(Duration::ZERO))?;
    let repaired = window.frame(size, scale)?;
    let damage = repaired.damage.bounds().unwrap();
    assert!(damage.right < i64::from(repaired.size.width));
    let mut reference = CpuWindow::mount(Ui::new(), scene(-3.))?;
    assert_eq!(repaired.rgba, reference.frame(size, scale)?.rgba);
    let stride = repaired.size.width as usize;
    let ink = |x: usize, y: usize| repaired.rgba[4 * (y * stride + x)];
    assert_eq!((ink(24, 13), ink(24, 18), ink(24, 21)), (255, 0x44, 0));
    Ok(())
}
