//! Decorations follow visible shaped runs, inherited paint and content bounds.
use anyhow::Result;
use solor::{
    Draw, Fill, Rgba, Styled, Ui, component, div, px,
    raster::{DeviceSize, RasterScale},
    rem, rgb, rgba, text,
};
use solor_render_cpu::pixels::Surface;

const FONT: &[u8] = include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf");

#[test]
fn inherited_rules_follow_aligned_lines_and_foreground_repaints() -> Result<()> {
    let mut ui = Ui::with_font(FONT.to_vec())?;
    let foreground = ui.context().signal(rgb(0x146480));
    let view = ui.mount(component(|_| {
        div()
            .underline()
            .line_through()
            .decoration_thickness(rem(0.125))
            .text_color(foreground)
            .child(text("Audio\n\nA").w(px(100.0)).text_right())
            .child(
                text("Muted")
                    .text_underline(false)
                    .decoration_color(rgb(0x993311)),
            )
    }))?;
    let size = DeviceSize {
        width: 120,
        height: 120,
    };
    ui.layout(view, size.at_unit_scale())?;
    for ink in [rgb(0x146480), rgb(0x333399)] {
        ui.context().set(foreground, ink)?;
        ui.flush()?;
        let frame = ui.record_frame(view)?;
        let [
            first,
            last,
            Draw::Text {
                paragraph, origin, ..
            },
            strike1,
            strike2,
            Draw::Text { .. },
            muted,
        ] = frame.draws()
        else {
            panic!("expected one pair of strokes per nonempty line and a single strike on Muted");
        };
        for ((under, strike), line) in [(first, strike1), (last, strike2)]
            .into_iter()
            .zip([&paragraph.lines[0], &paragraph.lines[2]])
        {
            let Draw::Fill {
                origin: u,
                size,
                fill,
                ..
            } = under
            else {
                panic!("missing underline")
            };
            let Draw::Fill {
                origin: s,
                size: s_size,
                fill: s_fill,
                ..
            } = strike
            else {
                panic!("missing strike")
            };
            assert_eq!(fill, &Fill::Solid(ink));
            assert_eq!(s_fill, fill);
            assert_eq!(size, s_size);
            assert_eq!(size.height, px(2.0));
            assert!((size.width.get() - line.width).abs() < 0.01);
            assert!((u.x.get() + size.width.get() - origin.x.get() - 100.0).abs() < 0.01);
            assert_eq!(u.x, s.x);
            assert!(s.y.get() < origin.y.get() + line.baseline);
            assert!(u.y.get() + size.height.get() > origin.y.get() + line.baseline);
        }
        assert!(matches!(muted, Draw::Fill { fill: Fill::Solid(c), .. } if *c == rgb(0x993311)));
    }
    Ok(())
}

#[test]
fn wrapped_ellipsized_strokes_clip_to_content_and_replay_at_fractional_scale() -> Result<()> {
    let mut ui = Ui::with_font(FONT.to_vec())?;
    let ink = rgb(0x228850);
    let view = ui.mount(component(|_| {
        div().p(px(3.5)).child(
            text("One two three four five six seven eight nine ten")
                .w(px(52.0))
                .h(px(40.0))
                .text_size(px(14.0))
                .line_height(px(20.0))
                .line_clamp(std::num::NonZeroUsize::new(2).unwrap())
                .text_ellipsis()
                .text_color(rgba(0))
                .decoration_color(ink)
                .text_bg(rgb(0xf2dbad))
                .decoration_thickness(px(80.0))
                .underline(),
        )
    }))?;
    let size = DeviceSize {
        width: 90,
        height: 80,
    }
    .at_unit_scale();
    ui.layout(view, size)?;
    let frame = ui.record_frame(view)?;
    let [first_bg, second_bg, one, two, Draw::Text { paragraph, .. }] = frame.draws() else {
        panic!("expected two decorated lines")
    };
    assert_eq!(paragraph.lines.len(), 2);
    assert!(paragraph.lines[1].glyphs.iter().any(|g| g.start == g.end));
    for (bg, stroke) in [(first_bg, one), (second_bg, two)] {
        let (
            Draw::Fill {
                origin, size, fill, ..
            },
            Draw::Fill {
                origin: under,
                size: extent,
                ..
            },
        ) = (bg, stroke)
        else {
            panic!("expected highlight and underline");
        };
        assert_eq!(fill, &Fill::Solid(rgb(0xf2dbad)));
        assert_eq!((origin.x, size.width), (under.x, extent.width));
    }
    for draw in [one, two] {
        let Draw::Fill { origin, size, .. } = draw else {
            panic!("missing decoration")
        };
        assert!(origin.x >= px(3.5) && origin.y >= px(3.5));
        assert!(origin.x + size.width <= px(55.5) && origin.y + size.height <= px(43.5));
    }
    let scale = RasterScale(1.5);
    let image = Surface::draw_view(&frame, scale.buffer_size(size)?, scale)?;
    let at = |x, y| {
        let i = 4 * (y * image.size.width as usize + x);
        Rgba::from(<[u8; 4]>::try_from(&image.rgba[i..i + 4]).unwrap())
    };
    assert_eq!(at(8usize, 12usize), ink);
    assert_eq!(at(4, 12), rgb(0xffffff));
    assert_eq!(at(8, 66), rgb(0xffffff));
    Ok(())
}

#[test]
fn inherited_highlight_follows_aligned_ink_and_clips_the_last_line() -> Result<()> {
    let mut ui = Ui::with_font(FONT.to_vec())?;
    let highlight = rgba(0xff000080);
    let view = ui.mount(component(|_| {
        div()
            .text_bg(highlight)
            .text_color(rgba(0))
            .decoration_thickness(px(0.0))
            .p(px(4.0))
            .child(
                text("ABC\n\nA")
                    .text_right()
                    .text_size(px(14.0))
                    .line_height(px(20.0))
                    .w(px(80.0))
                    .h(px(45.0)),
            )
            .child(text("Clear").text_bg(rgba(0)))
    }))?;
    let size = DeviceSize {
        width: 100,
        height: 80,
    }
    .at_unit_scale();
    ui.layout(view, size)?;
    for _ in 0..3 {
        let frame = ui.record_frame(view)?;
        let [
            Draw::Fill {
                origin: a,
                size: first,
                ..
            },
            Draw::Fill {
                origin: b,
                size: last,
                ..
            },
            Draw::Text { .. },
            Draw::Text { .. },
        ] = frame.draws()
        else {
            panic!("expected a highlight for each nonempty line of the first label");
        };
        assert_eq!((a.y, first.height), (px(4.0), px(20.0)));
        assert_eq!((b.y, last.height), (px(44.0), px(5.0)));
        assert!(b.x > a.x && a.x > px(4.0));
        for (x, width) in [(a.x, first.width), (b.x, last.width)] {
            assert!((x.get() + width.get() - 84.0).abs() < 0.01);
        }
        let scale = RasterScale(1.5);
        let image = Surface::draw_view(&frame, scale.buffer_size(size)?, scale)?;
        let pixel = |x: usize, y: usize| {
            let offset = 4 * (y * image.size.width as usize + x);
            &image.rgba[offset..offset + 4]
        };
        assert_eq!(pixel(124, 12), [255, 127, 127, 255]);
        assert_eq!(pixel(15, 12), [255; 4]);
        assert_eq!(pixel(124, 42), [255; 4]);
        assert_eq!(pixel(124, 72), [255, 127, 127, 255]);
        assert_eq!(pixel(124, 74), [255; 4]);
    }
    Ok(())
}

#[test]
fn switching_an_inherited_underline_preserves_shaping_and_draws_a_wave() -> Result<()> {
    use solor::style::UnderlineStyle;
    use std::rc::Rc;
    let mut ui = Ui::with_font(FONT.to_vec())?;
    let style = ui.context().signal(UnderlineStyle::Solid);
    let view = ui.mount(component(|_| {
        div().p(px(3.5)).text_underline(style).child(
            text("Audio settings")
                .text_size(px(20.))
                .line_height(px(36.))
                .text_color(rgba(0))
                .decoration_color(rgb(0))
                .decoration_thickness(px(2.)),
        )
    }))?;
    let size = DeviceSize {
        width: 170,
        height: 45,
    }
    .at_unit_scale();
    ui.layout(view, size)?;
    let solid = ui.record_frame(view)?;
    let [
        Draw::Fill { .. },
        Draw::Text {
            paragraph: before, ..
        },
    ] = solid.draws()
    else {
        panic!("expected a solid underline and text");
    };
    ui.context().set(style, UnderlineStyle::Wavy)?;
    ui.flush()?;
    let wave = ui.record_frame(view)?;
    let [
        Draw::WavyUnderline { origin, .. },
        Draw::Text {
            paragraph: after, ..
        },
    ] = wave.draws()
    else {
        panic!("expected a wavy underline and text");
    };
    assert!(Rc::ptr_eq(before, after));
    let scale = RasterScale(1.5);
    let surface = Surface::draw_view(&wave, scale.buffer_size(size)?, scale)?;
    let x = (f64::from(origin.x) * scale.0).floor() as usize;
    let y = (f64::from(origin.y) * scale.0 + 6.9).floor() as usize;
    let gray = |x: usize| surface.rgba[4 * (y * surface.size.width as usize + x)];
    // Sample the crest and trough at the same height within one period.
    assert!(gray(x + 7) < 64);
    assert_eq!(gray(x + 20), 255);
    Ok(())
}

#[test]
fn a_narrow_repair_under_a_wave_keeps_its_phase() -> Result<()> {
    use solor::platform::{WindowEvent, WindowHandler};
    use solor_render_cpu::CpuWindow;
    use std::time::Duration;

    let scene = |updated: bool| {
        component(move |cx| {
            let bg = cx.signal(if updated {
                rgb(0xffeeaa)
            } else {
                rgb(0xffffff)
            });
            div()
                .child(
                    div()
                        .absolute()
                        .left(px(23.5))
                        .top(px(0.))
                        .w(px(9.5))
                        .h(px(40.))
                        .bg(bg)
                        .on_frame(move |_, cx| cx.set(bg, rgb(0xffeeaa))),
                )
                .child(
                    text("Audio settings")
                        .underline_wavy()
                        .text_size(px(20.))
                        .line_height(px(36.))
                        .decoration_thickness(px(2.))
                        .decoration_color(rgb(0))
                        .text_color(rgba(0)),
                )
        })
    };
    let size = DeviceSize {
        width: 150,
        height: 45,
    }
    .at_unit_scale();
    let scale = RasterScale(1.5);
    let mut window = CpuWindow::mount(Ui::with_font(FONT.to_vec())?, scene(false))?;
    let first = window.frame(size, scale)?.rgba.to_vec();
    window.frame_submitted()?;
    window.event(WindowEvent::FrameReady(Duration::ZERO))?;
    let repaired = window.frame(size, scale)?;
    let damage = repaired.damage.bounds().unwrap();
    assert!(damage.left > 0 && damage.right - damage.left < 20);
    assert_ne!(first, repaired.rgba);
    let mut reference = CpuWindow::mount(Ui::with_font(FONT.to_vec())?, scene(true))?;
    assert_eq!(repaired.rgba, reference.frame(size, scale)?.rgba);
    Ok(())
}
