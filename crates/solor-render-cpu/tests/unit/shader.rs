use super::{rasterize, render, unorm};
use anyhow::Result;
use solor::EffectShader;
use solor::{ImageAsset, ImageRegion, raster::DeviceSize};
use std::{hint::black_box, time::Instant};

const SWEEP: &str = include_str!("../../../solor-platform/examples/cpu_shader/sweep.wgsl");

fn normalized(size: DeviceSize, mut region: ImageRegion) -> ImageRegion {
    for (i, length) in [size.width, size.height].into_iter().enumerate() {
        region.source_origin[i] /= f64::from(length);
        region.source_step[i] /= f64::from(length);
    }
    region
}

fn reference(size: DeviceSize, region: ImageRegion, position: f32) -> Result<ImageAsset> {
    rasterize(
        normalized(size, region),
        region.output,
        || (),
        |_, u, v, pixels| {
            for (pixel, x) in pixels.iter_mut().zip(u) {
                let t = ((x + (v - 0.5) * 0.12 - position).abs() / 0.16).clamp(0., 1.);
                let light = 1. - t * t * (3. - 2. * t);
                let mix = |a, b| unorm(a * (1. - light) + b * light);
                *pixel = [mix(0.075, 0.58), mix(0.12, 0.90), mix(0.22, 0.84), 255];
            }
        },
    )
}

fn full(size: DeviceSize) -> ImageRegion {
    ImageRegion {
        output: size,
        source_origin: [0.; 2],
        source_step: [1.; 2],
    }
}

#[test]
fn wgsl_matches_native_for_full_and_fractional_damage() -> Result<()> {
    let program = EffectShader::wgsl(SWEEP)?;
    let size = DeviceSize {
        width: 213,
        height: 97,
    };
    let clipped = ImageRegion {
        output: DeviceSize {
            width: 41,
            height: 13,
        },
        source_origin: [74.25, 16.5],
        source_step: [0.8, 0.8],
    };
    for position in [0., 0.42, 1.] {
        for region in [full(size), clipped] {
            let expected = reference(size, region, position)?;
            let actual = render(
                &program.with_params([position, 0., 0., 0.]),
                size.at_unit_scale(),
                normalized(size, region),
            )?;
            assert_eq!(expected.pixels(), actual.pixels());
        }
    }
    Ok(())
}

#[test]
fn single_axis_alpha_uses_the_requested_pixel_centers() -> Result<()> {
    let size = DeviceSize {
        width: 151,
        height: 73,
    };
    let region = ImageRegion {
        output: DeviceSize {
            width: 67,
            height: 19,
        },
        source_origin: [13.25, 9.75],
        source_step: [0.8, 0.8],
    };
    for (axis, expression) in ["uv.x", "uv.y", "0.25"].into_iter().enumerate() {
        let source = format!(
            "fn effect(uv: vec2<f32>, p: f32) -> vec4<f32> {{
            return vec4<f32>(1.0, 1.0, 1.0, {expression} * 0.5 + p);
        }}"
        );
        let program = EffectShader::wgsl(&source)?;
        for position in [0.1, 0.3] {
            let image = render(
                &program.with_params([position, 0., 0., 0.]),
                size.at_unit_scale(),
                normalized(size, region),
            )?;
            for (y, row) in image
                .pixels()
                .chunks_exact(region.output.width as usize)
                .enumerate()
            {
                for (x, pixel) in row.iter().enumerate() {
                    let coordinate = match axis {
                        0 => (13.25 + (x as f64 + 0.5) * 0.8) / 151.0,
                        1 => (9.75 + (y as f64 + 0.5) * 0.8) / 73.0,
                        _ => 0.25,
                    };
                    let alpha = ((coordinate as f32 * 0.5 + position) * 255.).round() as u8;
                    assert_eq!(*pixel, [alpha; 4]);
                }
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "release-mode paired raster timings"]
fn compare_cpu_shader() -> Result<()> {
    let start = Instant::now();
    let program = EffectShader::wgsl(SWEEP)?;
    eprintln!("compile {:.3} ms", start.elapsed().as_secs_f64() * 1000.);
    for (width, height) in [(320, 96), (1920, 1080)] {
        let size = DeviceSize { width, height };
        let region = full(size);
        let measure = |native, position| -> Result<f64> {
            let start = Instant::now();
            let image = if native {
                reference(size, region, position)?
            } else {
                render(
                    &program.with_params([position, 0., 0., 0.]),
                    size.at_unit_scale(),
                    normalized(size, region),
                )?
            };
            black_box(image.pixels());
            Ok(start.elapsed().as_secs_f64() * 1000.)
        };
        measure(true, 0.5)?;
        measure(false, 0.5)?;
        let mut vm = Vec::new();
        let mut native = Vec::new();
        for i in 0..21 {
            let position = (i as f32 + 0.5) / 21.;
            if i % 2 == 0 {
                vm.push(measure(false, position)?);
                native.push(measure(true, position)?);
            } else {
                native.push(measure(true, position)?);
                vm.push(measure(false, position)?);
            }
        }
        for (name, mut times) in [("WGSL", vm), ("Rust", native)] {
            times.sort_by(f64::total_cmp);
            eprintln!(
                "{width}x{height} {name} median {:.3} ms, p95 {:.3} ms",
                times[10], times[19]
            );
        }
    }
    Ok(())
}
