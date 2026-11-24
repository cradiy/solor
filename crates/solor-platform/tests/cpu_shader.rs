//! Shader fills through retained backgrounds, custom painting and CPU frame repair.
#![cfg(target_os = "linux")]

use anyhow::Result;
use solor::{EffectShader, raster::DeviceSize};
use std::{hint::black_box, time::Instant};

const SWEEP: &str = include_str!("../examples/cpu_shader/sweep.wgsl");
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
    solor_render_cpu::CpuWindow::mount(
        Ui::new(),
        component(move |cx| {
            let position = cx.signal(0.4);
            let mut positions = positions;
            surface::shader_surface(program, position, regional).on_frame(move |_, cx| {
                if let Some(next) = positions.next() {
                    cx.set(position, next)?;
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
fn moving_light_repairs_match_full_frames() -> Result<()> {
    use solor::{
        platform::{WindowEvent, WindowHandler},
        raster::RasterScale,
    };
    let positions = [0.41, 0.02, 0.98, 0.01];
    let mut partial = shader_window(SWEEP, true, positions.into_iter())?;
    let mut full = shader_window(SWEEP, false, positions.into_iter())?;
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
            assert!(dirty.right - dirty.left < i64::from(scale.buffer_size(logical)?.width) / 2);
        }
        for window in [&mut partial, &mut full] {
            window.frame_submitted()?;
            window.event(WindowEvent::FrameReady(std::time::Duration::from_millis(
                (index + 1) as u64 * 8,
            )))?;
        }
    }
    Ok(())
}

#[test]
#[ignore = "release-mode retained CPU frame timings"]
fn compare_shader_frames() -> Result<()> {
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
