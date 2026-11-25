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
        || [0.; super::LANES],
        |column, u| *column = u,
        |column, v, pixels| {
            for (pixel, &x) in pixels.iter_mut().zip(column.iter()) {
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
fn select_uses_the_condition_axes_and_vector_lanes() -> Result<()> {
    let shader = EffectShader::wgsl(
        "fn orient(color: vec2f, swap: bool) -> vec2f {
            return select(color, color.yx, swap);
        }
        fn effect(uv: vec2f, edge: f32) -> vec4f {
            let color = select(vec2f(0.), vec2f(1.), uv <= vec2f(edge, 0.25));
            return vec4f(orient(color, uv.y > 0.5), select(0., 1., uv.x != edge), 1.);
        }",
    )?;
    let size = DeviceSize {
        width: 4,
        height: 2,
    };
    for edge in [0.375, 0.625] {
        let image = render(
            &shader.with_params([edge, 0., 0., 0.]),
            size.at_unit_scale(),
            normalized(size, full(size)),
        )?;
        assert_eq!(image.dimensions(), size);
        for (i, pixel) in image.pixels().iter().enumerate() {
            let u = ((i % 4) as f32 + 0.5) / 4.;
            let left = if u <= edge { 255 } else { 0 };
            let blue = if u != edge { 255 } else { 0 };
            let expected = if i < 4 {
                [left, 255, blue, 255]
            } else {
                [0, left, blue, 255]
            };
            assert_eq!(*pixel, expected);
        }
    }
    Ok(())
}

#[test]
fn bounds_masks_match_pixel_centers_at_inclusive_and_exclusive_edges() -> Result<()> {
    let shader = EffectShader::wgsl(
        "fn effect(uv: vec2f, bounds: vec4f) -> vec4f {
            let lower = uv >= bounds.xy;
            let upper = uv < bounds.zw;
            let inside = all(lower & upper);
            let outside = any(!lower | !upper);
            return vec4f(select(0., 1., inside), select(0., 1., outside), select(0., 1., !outside), 1.);
        }",
    )?;
    let size = DeviceSize {
        width: 4,
        height: 4,
    };
    let image = render(
        &shader.with_params([0.375, 0.125, 0.875, 0.625]),
        size.at_unit_scale(),
        normalized(size, full(size)),
    )?;
    assert_eq!(image.dimensions(), size);
    for (i, pixel) in image.pixels().iter().enumerate() {
        let inside = (1..3).contains(&(i % 4)) && i / 4 < 2;
        assert_eq!(
            *pixel,
            if inside {
                [255, 0, 255, 255]
            } else {
                [0, 255, 0, 255]
            }
        );
    }
    Ok(())
}

#[test]
fn branch_assignments_and_early_returns_keep_each_pixels_local_values() -> Result<()> {
    let shader = EffectShader::wgsl(
        "fn shade(uv: vec2f, edge: f32) -> vec2f {
            var color = vec2f(0.25, 0.5);
            let original = color;
            if uv.x < edge {
                color.x += uv.y;
                if uv.y >= 0.5 { return color.yx; }
            } else if uv.y < 0.5 {
                color = uv;
            }
            color.y = original.x;
            return color;
        }
        fn effect(uv: vec2f, edge: f32) -> vec4f {
            var blue: f32;
            if (uv.x >= edge && uv.y < edge) || uv.x == uv.y { blue = 1.; }
            let pair = shade(uv, edge);
            return vec4f(pair, blue, 1.);
        }",
    )?;
    let size = DeviceSize {
        width: 4,
        height: 4,
    };
    for edge in [0.375, 0.625] {
        let image = render(
            &shader.with_params([edge, 0., 0., 0.]),
            size.at_unit_scale(),
            normalized(size, full(size)),
        )?;
        assert_eq!(image.dimensions(), size);
        for (i, pixel) in image.pixels().iter().enumerate() {
            let u = ((i % 4) as f32 + 0.5) / 4.;
            let v = ((i / 4) as f32 + 0.5) / 4.;
            let (r, g) = match (u < edge, v < 0.5) {
                (true, false) => (0.5, 0.25 + v),
                (true, true) => (0.25 + v, 0.25),
                (false, true) => (u, 0.25),
                (false, false) => (0.25, 0.25),
            };
            let blue = if (u >= edge && v < edge) || u == v {
                255
            } else {
                0
            };
            assert_eq!(
                *pixel,
                [unorm(r), unorm(g), blue, 255],
                "UV ({u}, {v}), edge {edge}"
            );
        }
    }
    Ok(())
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
fn interference_matches_native_at_fractional_sample_positions() -> Result<()> {
    let source = include_str!("../../../solor-gallery/examples/shaders/interference.wgsl");
    let shader = EffectShader::wgsl(source)?;
    let size = DeviceSize {
        width: 160,
        height: 80,
    };
    let region = normalized(
        size,
        ImageRegion {
            output: DeviceSize {
                width: 67,
                height: 19,
            },
            source_origin: [13.25, 9.75],
            source_step: [0.8, 0.8],
        },
    );
    for phase in [0., 0.35, 0.92] {
        let image = render(
            &shader.with_params([phase, 0., 0., 0.]),
            size.at_unit_scale(),
            region,
        )?;
        let angle = phase * std::f32::consts::TAU;
        for (y, row) in image
            .pixels()
            .chunks_exact(region.output.width as usize)
            .enumerate()
        {
            let coord = |axis: usize, i: usize| {
                (region.source_origin[axis] + (i as f64 + 0.5) * region.source_step[axis]) as f32
            };
            let py = (coord(1, y) - 0.5) * 6.;
            for (x, pixel) in row.iter().enumerate() {
                let px = (coord(0, x) - 0.5) * 10.;
                let a = (px + (py - angle).cos() + angle).sin();
                let b = (py + (px - angle).cos() + angle).sin();
                let light = 0.5 + 0.5 * (a + b + angle).cos();
                let mix = |dark, bright| unorm(dark * (1. - light) + bright * light);
                let expected = [mix(0.08, 0.33), mix(0.13, 0.88), mix(0.30, 0.78), 255];
                for (&actual, expected) in pixel.iter().zip(expected) {
                    assert!(
                        actual.abs_diff(expected) <= 1,
                        "phase {phase}, pixel ({x}, {y})"
                    );
                }
            }
        }
    }
    Ok(())
}

#[test]
fn translated_grid_wraps_at_negative_and_positive_cell_boundaries() -> Result<()> {
    let shader = EffectShader::wgsl(
        "fn effect(uv: vec2<f32>, offset: vec4<f32>, size: vec2<f32>) -> vec4<f32> {
            let p = uv * size * 0.5 - offset.xy;
            let cell = floor(p);
            return vec4<f32>(fract(p), 0.5 + 0.25 * (cell.x + cell.y), 1.0);
        }",
    )?;
    let size = DeviceSize {
        width: 6,
        height: 2,
    };
    let image = render(
        &shader.with_params([1.25, 0.75, 0., 0.]),
        size.at_unit_scale(),
        normalized(size, full(size)),
    )?;
    assert_eq!(image.dimensions(), size);
    // Columns sample -1, -0.5, 0, 0.5, 1, 1.5. Rows sample -0.5 and 0.
    for (row, (green, blue)) in image.pixels().as_chunks::<6>().0.iter().zip([
        (128, [0, 0, 64, 64, 128, 128]),
        (0, [64, 64, 128, 128, 191, 191]),
    ]) {
        let red = [0, 128, 0, 128, 0, 128];
        for (x, pixel) in row.iter().enumerate() {
            assert_eq!(*pixel, [red[x], green, blue[x], 255]);
        }
    }
    Ok(())
}

#[test]
fn power_curves_preserve_each_channels_coordinate_dependencies() -> Result<()> {
    let shader = EffectShader::wgsl(
        "fn effect(uv: vec2<f32>, p: f32) -> vec4<f32> {
            let curves = pow(vec2<f32>(p, uv.x), vec2<f32>(uv.x, 2.0));
            return vec4<f32>(curves, pow(uv.y, p), 1.0);
        }",
    )?;
    let size = DeviceSize {
        width: 37,
        height: 5,
    };
    let image = render(
        &shader.with_params([0.5, 0., 0., 0.]),
        size.at_unit_scale(),
        normalized(size, full(size)),
    )?;
    assert_eq!(image.dimensions(), size);
    let width = size.width as usize;
    for (i, pixel) in image.pixels().iter().enumerate() {
        let x = ((i % width) as f32 + 0.5) / size.width as f32;
        let y = ((i / width) as f32 + 0.5) / size.height as f32;
        let expected = [(-x).exp2(), x * x, y.sqrt(), 1.0].map(unorm);
        for (&actual, expected) in pixel.iter().zip(expected) {
            assert!(actual.abs_diff(expected) <= 1, "pixel {i}: {pixel:?}");
        }
    }
    Ok(())
}

#[test]
fn normalized_vectors_and_dot_products_use_every_component() -> Result<()> {
    let shader = EffectShader::wgsl(
        "fn effect(uv: vec2<f32>, p: vec4<f32>) -> vec4<f32> {
            let v = normalize(vec4<f32>(uv - vec2<f32>(0.5), p.zw));
            let light = 0.5 + 0.5 * dot(v, p);
            return vec4<f32>(sqrt(uv), light, 1.0);
        }",
    )?;
    let size = DeviceSize {
        width: 31,
        height: 17,
    };
    let region = normalized(size, full(size));
    let params = [0.25, -0.5, 0.375, -0.25];
    let image = render(&shader.with_params(params), size.at_unit_scale(), region)?;
    assert_eq!(image.dimensions(), size);
    for (i, pixel) in image.pixels().iter().enumerate() {
        let u = ((i % size.width as usize) as f32 + 0.5) / size.width as f32;
        let v = ((i / size.width as usize) as f32 + 0.5) / size.height as f32;
        let vector = [u - 0.5, v - 0.5, params[2], params[3]];
        let length = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
        let projection = vector
            .into_iter()
            .zip(params)
            .map(|(x, y)| (x / length) * y)
            .sum::<f32>();
        let expected = [u.sqrt(), v.sqrt(), 0.5 + 0.5 * projection, 1.0].map(unorm);
        for (&actual, expected) in pixel.iter().zip(expected) {
            assert!(actual.abs_diff(expected) <= 1, "pixel {i}: {pixel:?}");
        }
    }
    Ok(())
}

#[test]
fn reflection_flips_the_normal_component_and_preserves_the_tangent() -> Result<()> {
    let shader = EffectShader::wgsl(
        "fn effect(uv: vec2<f32>, incident: vec4<f32>) -> vec4<f32> {
            let r = reflect(incident, vec4<f32>(0.5));
            return vec4<f32>(0.5 * r.xyz + vec3<f32>(0.5), 1.0);
        }",
    )?;
    let size = DeviceSize {
        width: 1,
        height: 1,
    };
    for (incident, expected) in [
        ([0.5; 4], [64, 64, 64, 255]),
        ([0.0, 0.0, 1.0, -1.0], [128, 128, 255, 255]),
        ([0.0, 0.0, 0.0, -1.0], [191, 191, 191, 255]),
    ] {
        let image = render(
            &shader.with_params(incident),
            size.at_unit_scale(),
            full(size),
        )?;
        assert_eq!(image.pixels(), &[expected]);
    }
    Ok(())
}

#[test]
fn refraction_spans_transmission_grazing_and_total_internal_reflection() -> Result<()> {
    let shader = EffectShader::wgsl(
        "fn effect(uv: vec2<f32>, incident: vec4<f32>) -> vec4<f32> {
            let ray = refract(incident, vec4<f32>(0.0, 0.0, 0.0, 1.0), uv.x * 2.0);
            return vec4<f32>(ray.xy, -ray.w, 1.0);
        }",
    )?;
    let size = DeviceSize {
        width: 3,
        height: 1,
    };
    // Across the row the index ratio is 1/3, 1, then 5/3.
    for (incident, expected) in [
        (
            [1.0, 0.0, 0.0, 0.0],
            [[85, 0, 240, 255], [255, 0, 0, 255], [0, 0, 0, 255]],
        ),
        (
            [0.8, 0.0, 0.0, -0.6],
            [[68, 0, 246, 255], [204, 0, 153, 255], [0, 0, 0, 255]],
        ),
    ] {
        let image = render(
            &shader.with_params(incident),
            size.at_unit_scale(),
            normalized(size, full(size)),
        )?;
        assert_eq!(image.pixels(), &expected);
    }
    Ok(())
}

#[test]
fn nested_helpers_bind_each_calls_own_arguments() -> Result<()> {
    let body = "fn effect(uv: vec2<f32>, p: vec4<f32>) -> vec4<f32>";
    let helper = EffectShader::wgsl(&format!(
        "fn add_pair(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {{ return a + b; }}
         fn tint(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {{ return add_pair(a.yx, b); }}
         {body} {{ return vec4<f32>(tint(uv, p.xy), tint(p.zw, uv)); }}"
    ))?;
    let flat = EffectShader::wgsl(&format!(
        "{body} {{ return vec4<f32>(uv.yx + p.xy, p.wz + uv); }}"
    ))?;
    let size = DeviceSize {
        width: 47,
        height: 29,
    };
    let region = normalized(size, full(size));
    let parameters = [0.125, 0.05, 0.17, 0.25];
    let draw = |shader: &EffectShader| {
        render(
            &shader.with_params(parameters),
            size.at_unit_scale(),
            region,
        )
    };
    assert_eq!(draw(&helper)?.pixels(), draw(&flat)?.pixels());
    assert_eq!(
        solor::raster::shader_program(&helper)
            .cpu()?
            .instructions()
            .len(),
        solor::raster::shader_program(&flat)
            .cpu()?
            .instructions()
            .len()
    );
    Ok(())
}

#[test]
fn inherited_shader_tracks_only_elements_that_paint_with_its_parameters() -> Result<()> {
    use crate::pixels::Surface;
    use solor::{Draw, Styled, SvgAsset, Ui, component, div, px, raster::RasterScale, svg, text};
    use std::rc::Rc;

    let shader = EffectShader::wgsl(
        "fn effect(uv: vec2<f32>, p: vec4<f32>) -> vec4<f32> {
        return vec4<f32>(p.rgb, 1.0);
    }",
    )?;
    let icon = SvgAsset::from_bytes(br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8"/></svg>"#)?;
    let font = include_bytes!("../../../../fixtures/fonts/NotoSansCJKsc-fixture.otf");
    let mut ui = Ui::with_font(font.to_vec())?;
    let phase = ui.context().signal(0.0_f32);
    let tint = ui.context().signal([0., 0., 1., 1.]);
    let view = ui.mount(component(|_| {
        div()
            .text_color(shader.with_params(phase))
            .child(text("A"))
            .child(svg(icon).size(px(8.)).text_color(shader.with_params(tint)))
    }))?;
    let size = DeviceSize {
        width: 64,
        height: 48,
    };
    ui.layout(view, size.at_unit_scale())?;
    let (label, icon) = {
        let mut nodes = ui.children(view, ui.root(view)?)?;
        (nodes.next().unwrap(), nodes.next().unwrap())
    };
    let first = ui.record_frame(view)?;
    let Draw::Text { paragraph, .. } = &first.draws()[0] else {
        panic!("expected a label")
    };

    for (expected_node, icon_color) in [(label, [0, 0, 255, 255]), (icon, [0, 255, 0, 255])] {
        if expected_node == label {
            ui.context().set(phase, 1.)?;
        } else {
            ui.context().set(tint, [0., 1., 0., 1.])?;
        }
        ui.flush()?;
        let mut dirty = Vec::new();
        ui.collect_repaints(view, &mut dirty)?;
        assert_eq!(dirty, [expected_node]);
        assert_eq!(ui.layout(view, size.at_unit_scale())?, 0);
        let frame = ui.record_frame(view)?;
        let Draw::Text {
            paragraph: updated, ..
        } = &frame.draws()[0]
        else {
            panic!("expected a label")
        };
        assert!(Rc::ptr_eq(paragraph, updated));
        let surface = Surface::draw_view(&frame, size, RasterScale::ONE)?;
        let pixels = surface.rgba.as_chunks::<4>().0;
        assert!(pixels.contains(&icon_color));
        assert!(
            pixels
                .iter()
                .any(|&[r, g, b, _]| r == 255 && g == b && g < 200)
        );
    }
    Ok(())
}

#[test]
fn single_axis_alpha_uses_the_requested_pixel_centers() -> Result<()> {
    let size = DeviceSize {
        width: 151,
        height: 73,
    };
    let scale = solor::raster::RasterScale(1.25);
    let clip = solor::raster::Clip {
        left: 17,
        top: 13,
        right: 84,
        bottom: 32,
    };
    let origin = [
        clip.left as f64 - 13.25 * scale.0,
        clip.top as f64 - 9.75 * scale.0,
    ];
    for (axis, expression) in ["uv.x", "uv.y", "0.25"].into_iter().enumerate() {
        let source = format!(
            "fn effect(uv: vec2<f32>, p: f32) -> vec4<f32> {{
            return vec4<f32>(1.0, 1.0, 1.0, {expression} * 0.5 + p);
        }}"
        );
        let program = EffectShader::wgsl(&source)?;
        for position in [0.1, 0.3] {
            let sampler = crate::bitmap::RegionSampler::shader(
                &program.with_params([position, 0., 0., 0.]),
                origin,
                size.at_unit_scale(),
                scale,
                clip,
            )?
            .unwrap();
            let mut row = vec![[0; 4]; (clip.right - clip.left) as usize];
            let mut masked = vec![[0; 4]; row.len() + 2];
            let coverage: Vec<_> = [128, 0, 255]
                .into_iter()
                .cycle()
                .take(masked.len())
                .collect();
            for (y, device_y) in (clip.top..clip.bottom).enumerate() {
                row.fill([0; 4]);
                sampler.blend_span(&mut row, clip.left, device_y, scale);
                masked.fill([0; 4]);
                sampler.blend_shader_mask(&mut masked, &coverage, clip.left - 1, device_y);
                assert_eq!(masked[0], [0; 4]);
                assert_eq!(masked[row.len() + 1], [0; 4]);
                for (x, pixel) in row.iter().enumerate() {
                    let coordinate = match axis {
                        0 => (13.25 + (x as f64 + 0.5) * 0.8) / 151.0,
                        1 => (9.75 + (y as f64 + 0.5) * 0.8) / 73.0,
                        _ => 0.25,
                    };
                    let alpha = ((coordinate as f32 * 0.5 + position) * 255.).round() as u8;
                    assert_eq!(*pixel, [alpha; 4]);
                    let expected = (u16::from(alpha) * u16::from(coverage[x + 1]) + 127) / 255;
                    assert_eq!(masked[x + 1], [expected as u8; 4]);
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
