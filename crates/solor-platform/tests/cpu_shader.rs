//! Shader fills through retained backgrounds, custom painting and CPU frame repair.
#![cfg(any(target_os = "linux", target_os = "macos"))]

use anyhow::Result;
use solor::{EffectShader, raster::DeviceSize};
use std::{hint::black_box, time::Instant};

use surface::{Footprint, ROUNDED_RECT, SPOTLIGHT, SWEEP};
const RIBBON: &str = include_str!("../examples/cpu_shader/ribbon.wgsl");

#[path = "../examples/cpu_shader/surface.rs"]
mod surface;

fn shader_window(
    source: &str,
    regional: bool,
    positions: impl Iterator<Item = f32> + 'static,
) -> Result<solor_render_cpu::CpuWindow> {
    use solor::{Ui, component};
    let program = EffectShader::wgsl(source)?;
    let samples_motion = source == ROUNDED_RECT;
    let footprint = if regional {
        Footprint::for_source(source)
    } else {
        None
    };
    solor_render_cpu::CpuWindow::mount(
        Ui::new(),
        component(move |cx| {
            let params = cx.signal([0.4, 0., 0., 0.]);
            let mut positions = positions;
            surface::shader_surface(program, params, footprint).on_frame(move |_, cx| {
                if let Some(next) = positions.next() {
                    let travel = if samples_motion {
                        next - cx.read(params)?[0]
                    } else {
                        0.
                    };
                    cx.set(params, [next, travel, 0., 0.])?;
                }
                Ok(())
            })
        }),
    )
}

#[test]
fn shared_shader_samples_independent_uniforms() -> Result<()> {
    use solor::{
        Styled, Ui, component, div,
        platform::{WindowEvent, WindowHandler},
        px,
        raster::RasterScale,
    };
    use solor_render_cpu::CpuWindow;
    let shader =
        EffectShader::wgsl("fn effect(uv: vec2<f32>, p: vec4<f32>) -> vec4<f32> { return p; }")?;
    let mut window = CpuWindow::mount(
        Ui::new(),
        component(move |cx| {
            let color = cx.signal([1.0_f32, 0.0, 0.0, 1.0]);
            div()
                .flex_row()
                .child(div().size(px(2.)).bg(shader.with_params(color)))
                .child(div().size(px(2.)).bg(shader.with_params([0., 0., 1., 1.])))
                .on_frame(move |_, cx| cx.set(color, [0., 1., 0., 1.]))
        }),
    )?;
    let size = DeviceSize {
        width: 4,
        height: 2,
    }
    .at_unit_scale();
    let frame = window.frame(size, RasterScale::ONE)?;
    assert_eq!(&frame.rgba[..4], &[255, 0, 0, 255]);
    assert_eq!(&frame.rgba[12..16], &[0, 0, 255, 255]);
    window.frame_submitted()?;
    window.event(WindowEvent::FrameReady(std::time::Duration::ZERO))?;
    let frame = window.frame(size, RasterScale::ONE)?;
    assert_eq!(&frame.rgba[..4], &[0, 255, 0, 255]);
    assert_eq!(&frame.rgba[12..16], &[0, 0, 255, 255]);
    Ok(())
}

#[test]
fn spotlight_keeps_its_logical_radius_across_damage_and_resize() -> Result<()> {
    use solor::{
        Styled, Ui, component, div,
        platform::{WindowEvent, WindowHandler},
        px,
        raster::RasterScale,
        rgb,
    };
    use solor_render_cpu::CpuWindow;
    let shader = EffectShader::wgsl(include_str!("../examples/cpu_shader/spotlight.wgsl"))?;
    let mut window = CpuWindow::mount(
        Ui::new(),
        component(move |cx| {
            let overlay = cx.signal(true);
            div()
                .items_center()
                .justify_center()
                .bg(shader.with_params(0.5))
                .on_frame_when(overlay, move |_, cx| cx.set(overlay, false))
                .child_when(overlay, || div().size(px(32.)).bg(rgb(0xffffff)))
        }),
    )?;
    let scale = RasterScale(1.25);
    let logical = DeviceSize {
        width: 320,
        height: 224,
    }
    .at_unit_scale();
    let wide = DeviceSize {
        width: 480,
        height: 224,
    }
    .at_unit_scale();
    let sample = |frame: &solor::raster::RasterFrame, dx: usize, dy: usize| -> [u8; 4] {
        let width = frame.size.width as usize;
        let y = frame.size.height as usize / 2 + dy;
        let offset = (y * width + width / 2 + dx) * 4;
        frame.rgba[offset..offset + 4].try_into().unwrap()
    };
    let first = window.frame(logical, scale)?;
    assert_eq!(sample(&first, 12, 0), [255; 4]);
    let ring = sample(&first, 60, 0);
    assert_eq!(ring, sample(&first, 0, 60));
    assert_ne!(ring, sample(&first, 125, 0));

    window.frame_submitted()?;
    window.event(WindowEvent::FrameReady(std::time::Duration::ZERO))?;
    let repaired = window.frame(logical, scale)?;
    let dirty = repaired.damage.bounds().unwrap();
    assert!(dirty.right - dirty.left < i64::from(repaired.size.width));
    let core = sample(&repaired, 12, 0);
    assert_ne!(core, [255; 4]);
    assert_ne!(core, ring);

    window.frame_submitted()?;
    let resized = window.frame(wide, scale)?;
    assert_eq!(core, sample(&resized, 12, 0));
    assert_eq!(ring, sample(&resized, 60, 0));
    assert_eq!(ring, sample(&resized, 0, 60));
    // At 1.25x, the 96 logical pixel halo still covers a 100 device pixel offset.
    assert_ne!(sample(&resized, 100, 0), sample(&resized, 125, 0));
    Ok(())
}

#[test]
fn rounded_contour_has_a_straight_rim_and_clear_outer_corners() -> Result<()> {
    use solor::{Styled, Ui, component, div, platform::WindowHandler, raster::RasterScale};
    let shader = EffectShader::wgsl(ROUNDED_RECT)?;
    let mut window = solor_render_cpu::CpuWindow::mount(
        Ui::new(),
        component(move |_| div().bg(shader.with_params(0.5))),
    )?;
    let size = DeviceSize {
        width: 240,
        height: 160,
    };
    let frame = window.frame(size.at_unit_scale(), RasterScale::ONE)?;
    let pixel = |x, y| {
        let offset: usize = (y * size.width as usize + x) * 4;
        &frame.rgba[offset..offset + 4]
    };
    // Pixel centers sit half a pixel inside the straight edges.
    assert_eq!(pixel(191, 79), &[168, 245, 196, 255]);
    assert_eq!(pixel(119, 123), pixel(191, 79));
    assert_eq!(pixel(191, 123), &[11, 21, 26, 255]);
    assert_eq!(pixel(119, 79), &[13, 62, 54, 255]);
    assert!(pixel(184, 79)[1] > pixel(119, 79)[1]);
    Ok(())
}

#[test]
fn moving_light_repairs_match_full_frames() -> Result<()> {
    use solor::{
        platform::{WindowEvent, WindowHandler},
        raster::RasterScale,
    };
    for source in [SWEEP, SPOTLIGHT, ROUNDED_RECT] {
        let positions = [0.41, 0.41, 0.02, 0.98, 0.01];
        let mut partial = shader_window(source, true, positions.into_iter())?;
        let mut full = shader_window(source, false, positions.into_iter())?;
        let logical = DeviceSize {
            width: 481,
            height: 223,
        }
        .at_unit_scale();
        let scale = RasterScale(1.25);
        for index in 0..=positions.len() {
            let a = partial.frame(logical, scale)?;
            let b = full.frame(logical, scale)?;
            assert_eq!(a.rgba, b.rgba);
            if index == 1 {
                let dirty = a.damage.bounds().unwrap();
                assert!(
                    dirty.right - dirty.left < i64::from(scale.buffer_size(logical)?.width) / 2
                );
            }
            for window in [&mut partial, &mut full] {
                window.frame_submitted()?;
                window.event(WindowEvent::FrameReady(std::time::Duration::from_millis(
                    (index + 1) as u64 * 8,
                )))?;
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "release-mode retained CPU frame timings"]
fn compare_shader_frames() -> Result<()> {
    measure_frames([
        ("spotlight full", SPOTLIGHT, false),
        ("spotlight regional", SPOTLIGHT, true),
    ])?;
    let vertical = RIBBON.replace("uv.x", "uv.y");
    for pair in [
        [("full", SWEEP, false), ("regional", SWEEP, true)],
        [
            ("horizontal", RIBBON, false),
            ("vertical", vertical.as_str(), false),
        ],
    ] {
        measure_frames(pair)?;
    }
    Ok(())
}

fn measure_frames(pair: [(&str, &str, bool); 2]) -> Result<()> {
    use solor::{
        platform::{WindowEvent, WindowHandler},
        raster::RasterScale,
    };
    let logical = DeviceSize {
        width: 1920,
        height: 1080,
    }
    .at_unit_scale();
    let positions = (0..).map(|i| 0.4 + i as f32 * 0.003);
    let mut first = shader_window(pair[0].1, pair[0].2, positions.clone())?;
    let mut second = shader_window(pair[1].1, pair[1].2, positions)?;
    let mut times = [Vec::new(), Vec::new()];
    for i in 0..24 {
        let mut frame = |window: &mut solor_render_cpu::CpuWindow, mode: usize| -> Result<()> {
            let start = Instant::now();
            window.event(WindowEvent::FrameReady(std::time::Duration::from_millis(
                i * 8,
            )))?;
            black_box(window.frame(logical, RasterScale(1.))?.rgba);
            window.frame_submitted()?;
            if i >= 3 {
                times[mode].push(start.elapsed().as_secs_f64() * 1000.);
            }
            Ok(())
        };
        if i % 2 == 0 {
            frame(&mut first, 0)?;
            frame(&mut second, 1)?;
        } else {
            frame(&mut second, 1)?;
            frame(&mut first, 0)?;
        }
    }
    for (mode, mut times) in pair.map(|(name, _, _)| name).into_iter().zip(times) {
        times.sort_by(f64::total_cmp);
        eprintln!(
            "1080p CPU frame {mode}: median {:.3} ms, p95 {:.3} ms",
            times[10], times[19]
        );
    }
    Ok(())
}

#[test]
fn path_ink_repairs_the_band_and_tracks_contour_changes() -> Result<()> {
    use solor::{
        Ui, component, div,
        layout::Origin,
        platform::{WindowEvent, WindowHandler},
        px,
        raster::RasterScale,
    };
    use solor_render_cpu::CpuWindow;
    use std::time::Duration;

    let shader = EffectShader::wgsl(
        "fn effect(uv: vec2<f32>, p: f32) -> vec4<f32> {
            return vec4<f32>(0.0, 1.0 - smoothstep(0.0, 0.02, abs(uv.x - p)), 0.4, 1.0);
        }",
    )?;
    let window = |initial: usize| {
        let shader = shader.clone();
        CpuWindow::mount(
            Ui::new(),
            component(move |cx| {
                let step = cx.signal(initial);
                div()
                    .paint(move |cx| {
                        let step = *cx.read(step)?;
                        let p = [0.03, 0.12, 0.06][step];
                        let fill = shader
                            .with_params(p)
                            .changing_region([p - 0.02, 0.]..[p + 0.02, 1.]);
                        let point = |x, y| Origin { x: px(x), y: px(y) };
                        cx.fill_polygon(
                            [
                                point(15.5, 2.5),
                                point(112.25, 4.),
                                point(112.25, if step == 2 { 8. } else { 21. }),
                                point(15.5, 21.),
                            ],
                            fill.clone(),
                        )?;
                        cx.stroke_polyline(
                            [point(17.5, 35.5), point(105., 35.5)],
                            px(if step == 2 { 5. } else { 16. }),
                            fill,
                        )
                    })
                    .on_frame(move |_, cx| cx.update(step, |s| *s = (*s + 1).min(2)))
            }),
        )
    };
    let size = DeviceSize {
        width: 120,
        height: 50,
    }
    .at_unit_scale();
    let scale = RasterScale(1.25);
    let mut retained = window(0)?;
    let mut previous = retained.frame(size, scale)?.rgba.to_vec();
    for step in [1, 2] {
        retained.frame_submitted()?;
        retained.event(WindowEvent::FrameReady(Duration::from_millis(
            step as u64 * 8,
        )))?;
        let frame = retained.frame(size, scale)?;
        let bounds = frame.damage.bounds().unwrap();
        if step == 1 {
            assert!(bounds.right - bounds.left < 32, "{bounds:?}");
        }
        assert_ne!(previous, frame.rgba);
        assert!(frame.rgba == window(step)?.frame(size, scale)?.rgba);
        previous.copy_from_slice(frame.rgba);
    }
    Ok(())
}
