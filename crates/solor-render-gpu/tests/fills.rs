//! Compare both renderers on a clipped, translucent paint stack.
use anyhow::Result;
use solor::{
    EffectShader, Styled, Ui, ViewFrame, color_stop, component, div,
    layout::{Origin, Size},
    linear_gradient, px,
    raster::{DeviceSize, RasterScale},
    rgb, rgba,
};
use solor_render_cpu::pixels::Surface;
use solor_render_gpu::Renderer;

fn scene(phase: f32) -> Result<(Size, ViewFrame)> {
    let gradient = linear_gradient(
        114.,
        [
            color_stop(rgb(0x18364e), 0.),
            color_stop(rgba(0xe696a0bb), 0.38),
            color_stop(rgb(0xf8eeb5), 1.),
        ],
    )?;
    let shader = EffectShader::wgsl(
        "fn effect(uv: vec2f, t: f32, size: vec2f) -> vec4f {
            let axes = select(uv, 1.0 - uv, uv >= vec2f(t));
            return select(vec4f(axes, t, 0.67), vec4f(t, axes.yx, 0.48), uv.x < t);
        }",
    )?;
    let size = |w, h| Size {
        width: px(w),
        height: px(h),
    };
    let at = |x, y| Origin { x: px(x), y: px(y) };
    let mut ui = Ui::new();
    let view = ui.mount(component(move |_| {
        div().w_full().h_full().paint(move |cx| {
            cx.fill(cx.size, gradient.clone());
            cx.fill_rounded_rect(at(4., 5.), size(35., 20.), px(6.), rgba(0x2b6fb583))?;
            cx.with_clip(at(24.36, 12.75), size(34., 30.), |cx| {
                cx.fill_rounded_rect(
                    at(17., 8.),
                    size(34., 22.),
                    px(3.),
                    shader.with_params(phase),
                )
            })
        })
    }))?;
    let logical = size(64., 48.);
    ui.layout(view, logical)?;
    Ok((logical, ui.record_frame(view)?))
}

fn pixels(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut Renderer,
    frame: &ViewFrame,
    size: DeviceSize,
    scale: RasterScale,
) -> Result<Vec<u8>> {
    let extent = wgpu::Extent3d {
        width: size.width,
        height: size.height,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    renderer.draw(
        frame,
        &texture.create_view(&Default::default()),
        size,
        scale,
        wgpu::Color::WHITE,
    )?;
    let stride = (size.width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(stride) * u64::from(size.height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: None,
            },
        },
        extent,
    );
    queue.submit([encoder.finish()]);
    let (send, recv) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
    device.poll(wgpu::PollType::wait_indefinitely())?;
    recv.recv()??;
    let mapped = buffer.slice(..).get_mapped_range()?;
    Ok(mapped
        .chunks_exact(stride as usize)
        .flat_map(|row| row[..size.width as usize * 4].iter().copied())
        .collect())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn ordered_fills_match_cpu_across_scales() -> Result<()> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))?;
    let mut renderer = Renderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm)?;
    for phase in [0.15, 0.85] {
        let (size, frame) = scene(phase)?;
        for scale in [1., 1.25, 2.].map(RasterScale) {
            let physical = scale.buffer_size(size)?;
            let cpu = Surface::draw_view(&frame, physical, scale)?;
            let gpu = pixels(&device, &queue, &mut renderer, &frame, physical, scale)?;
            let errors: Vec<_> = cpu
                .rgba
                .iter()
                .zip(&gpu)
                .map(|(c, g)| c.abs_diff(*g))
                .collect();
            let max = errors.iter().max().copied().unwrap();
            let above_two = errors.iter().filter(|e| **e > 2).count();
            eprintln!(
                "scale={} max={max} above_two={above_two}/{}",
                scale.0,
                errors.len()
            );
            assert!(max <= 16 && above_two * 500 < errors.len());
        }
    }
    compare_patterns(&device, &queue, &mut renderer)?;
    compare_gradients(&device, &queue, &mut renderer)?;
    compare_replaced_images(&device, &queue, &mut renderer)?;
    compare_text(&device, &queue, &mut renderer)?;
    compare_svg(&device, &queue, &mut renderer)
}

fn compare_patterns(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut Renderer,
) -> Result<()> {
    use solor::{Fill, checkerboard, diagonal_stripes};
    let ink = rgba(0x2a449bd3);
    let fills: [Fill; 5] = [
        checkerboard(ink, px(3.25))?.into(),
        checkerboard(ink, px(0.34))?.into(),
        diagonal_stripes(ink, px(1.8), px(3.1))?.into(),
        diagonal_stripes(ink, px(0.09), px(0.24))?.into(),
        diagonal_stripes(ink, px(1.15), px(0.))?.into(),
    ];
    let size = Size {
        width: px(40.),
        height: px(12.),
    };
    let viewport = Size {
        width: size.width,
        height: size.height * fills.len() as f32,
    };
    let mut ui = Ui::new();
    let view = ui.mount(component(move |_| {
        div().w_full().h_full().bg(rgb(0xd4c1ae)).paint(move |cx| {
            cx.with_clip(
                Origin {
                    x: px(2.4),
                    y: px(0.),
                },
                viewport,
                |cx| {
                    for (i, fill) in fills.iter().enumerate() {
                        cx.fill_rounded_rect(
                            Origin {
                                x: px(-0.35),
                                y: px(0.25) + size.height * i as f32,
                            },
                            size,
                            px(3.),
                            fill.clone(),
                        )?;
                    }
                    Ok(())
                },
            )
        })
    }))?;
    ui.layout(view, viewport)?;
    let frame = ui.record_frame(view)?;
    for scale in [1., 1.25, 2.].map(RasterScale) {
        let physical = scale.buffer_size(viewport)?;
        let cpu = Surface::draw_view(&frame, physical, scale)?;
        let gpu = pixels(device, queue, renderer, &frame, physical, scale)?;
        for (i, (a, b)) in gpu.iter().zip(&cpu.rgba).enumerate() {
            let x = i / 4 % physical.width as usize;
            let y = i / 4 / physical.width as usize;
            assert!(
                a.abs_diff(*b) <= 2,
                "pattern at ({x},{y}) scale={} GPU={a} CPU={b}",
                scale.0
            );
        }
    }
    Ok(())
}

fn compare_gradients(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut Renderer,
) -> Result<()> {
    use solor::{
        ColorSpace, Gradient,
        GradientKind::{Angular, Diamond, Linear, Radial},
    };

    // Odd, rectangular bounds put a device sample exactly at the angular center at 1x.
    let tile = Size {
        width: px(37.),
        height: px(21.),
    };
    let viewport = Size {
        width: tile.width * 8.,
        height: tile.height * 4.,
    };
    let mut tiles = Vec::new();
    let rows = [
        (0., None, 0.),
        (90., None, 0.18),
        (37., Some(0.), 0.),
        (37., Some(0.73), 0.),
    ];
    for (row, (angle, phase, seam)) in rows.into_iter().enumerate() {
        for (col, kind) in [Linear, Radial, Angular, Diamond].into_iter().enumerate() {
            let mut gradient = Gradient::new(
                kind,
                angle,
                [
                    color_stop(rgb(0x040119), 0.08),
                    color_stop(rgba(0xfea61691), 0.46),
                    color_stop(rgba(0x12856237), 0.46),
                    color_stop(rgba(0x00ff1100), 0.68),
                    color_stop(rgb(0x65aabc), 0.87),
                ],
            )?
            .gradient_midpoint(2, 0.27)?
            .opacity(0.82)?;
            if let Some(phase) = phase {
                gradient = gradient.phase(phase)?;
            }
            if kind == Angular {
                gradient = gradient.angular_seam_width(seam)?;
            }
            for (index, space) in [ColorSpace::Srgb, ColorSpace::Oklab]
                .into_iter()
                .enumerate()
            {
                let origin = Origin {
                    x: tile.width * (2 * col + index) as f32,
                    y: tile.height * row as f32,
                };
                tiles.push((origin, gradient.clone().color_space(space)));
            }
        }
    }
    let mut ui = Ui::new();
    let view = ui.mount(component(move |_| {
        div().w_full().h_full().paint(move |cx| {
            for (origin, gradient) in &tiles {
                cx.fill_rect(*origin, tile, gradient.clone())?;
            }
            Ok(())
        })
    }))?;
    ui.layout(view, viewport)?;
    let frame = ui.record_frame(view)?;
    for scale in [1., 1.25, 2.].map(RasterScale) {
        let size = scale.buffer_size(viewport)?;
        let cpu = Surface::draw_view(&frame, size, scale)?;
        let gpu = pixels(device, queue, renderer, &frame, size, scale)?;
        for (i, (actual, expected)) in gpu.iter().zip(&cpu.rgba).enumerate() {
            assert!(
                actual.abs_diff(*expected) <= 2,
                "gradient at ({}, {}), scale={}, GPU={actual}, CPU={expected}",
                i / 4 % size.width as usize,
                i / 4 / size.width as usize,
                scale.0
            );
        }
    }
    Ok(())
}

fn compare_replaced_images(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut Renderer,
) -> Result<()> {
    use solor::{
        ImageAsset, ImageFill, ImageSampling, image, percent, raster::Clip, style::ObjectFit,
    };

    let asset = |t: u8| {
        let row = [
            [255, 0, 255, 255],
            [t, 44, 99, 255],
            [0, 255, 183, 96],
            [200, 0, 31, 0],
            [255, 0, 255, 255],
        ];
        ImageAsset::from_rgba8(5, 2, row.repeat(2).into_flattened())
    };
    let first = asset(200)?;
    let second = asset(40)?;
    let tile = first.region(Clip {
        left: 1,
        top: 0,
        right: 4,
        bottom: 2,
    })?;
    let mut ui = Ui::new();
    let current = ui.context().signal(first.clone());
    let fit = ui.context().signal(ObjectFit::Contain);
    let viewport = Size {
        width: px(48.),
        height: px(32.),
    };
    let view = ui.mount(component(move |_| {
        div()
            .w_full()
            .h_full()
            .bg(rgb(0x192b31))
            .child(
                image(current)
                    .absolute()
                    .left(px(1.))
                    .top(px(2.))
                    .w(px(19.))
                    .h(px(17.))
                    .object_fit(fit),
            )
            .child(
                div()
                    .absolute()
                    .left(px(14.))
                    .top(px(12.))
                    .w(px(21.))
                    .h(px(15.))
                    .bg(current)
                    .object_position_x(percent(100.))
                    .object_fit(ObjectFit::Cover),
            )
            .child(div().w_full().h_full().paint(move |cx| {
                for (y, sampling) in [(3., ImageSampling::Linear), (17., ImageSampling::Nearest)] {
                    let region: ImageFill = tile.clone().with_sampling(sampling);
                    cx.paint_image(
                        Origin {
                            x: px(32.),
                            y: px(y),
                        },
                        Size {
                            width: px(14.),
                            height: px(12.),
                        },
                        region,
                        ObjectFit::Fill,
                    )?;
                }
                Ok(())
            }))
    }))?;
    for (next, mode) in [
        (first.clone(), ObjectFit::Contain),
        (second, ObjectFit::Cover),
        (asset(130)?, ObjectFit::ScaleDown),
        (first, ObjectFit::None),
    ] {
        ui.context().set(current, next)?;
        ui.context().set(fit, mode)?;
        ui.flush()?;
        ui.layout(view, viewport)?;
        let frame = ui.record_frame(view)?;
        for scale in [1., 1.25, 2.].map(RasterScale) {
            let size = scale.buffer_size(viewport)?;
            let expected = Surface::draw_view(&frame, size, scale)?;
            let actual = pixels(device, queue, renderer, &frame, size, scale)?;
            for (i, (a, b)) in actual
                .as_chunks::<4>()
                .0
                .iter()
                .zip(expected.rgba.as_chunks::<4>().0)
                .enumerate()
            {
                assert!(
                    a.iter().zip(b).all(|(a, b)| a.abs_diff(*b) <= 3),
                    "fit={mode:?}, scale={}, pixel=({},{}), GPU={a:?}, CPU={b:?}",
                    scale.0,
                    i % size.width as usize,
                    i / size.width as usize
                );
            }
        }
    }
    Ok(())
}

fn compare_text(device: &wgpu::Device, queue: &wgpu::Queue, renderer: &mut Renderer) -> Result<()> {
    let mut ui = Ui::with_font(
        include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf").to_vec(),
    )?;
    use solor::{ColorSpace, GradientKind, Ink};
    let blue = Ink::from(rgb(0x143a91));
    let tint = ui.context().signal(blue.clone());
    let size = Size {
        width: px(72.),
        height: px(58.),
    };
    let effect = EffectShader::wgsl(
        "fn effect(uv: vec2f, p: vec4f, size: vec2f) -> vec4f {
            let front = smoothstep(p.x - 5., p.x + 5., uv.x * size.x);
            return vec4f(p.y, uv.y, uv.x, (1. - front) * p.z + p.w);
        }",
    )?;
    let background = effect.with_params([24., 0.8, 0.08, 0.08]);
    let view = ui.mount(component(move |_| {
        div().w_full().h_full().bg(background).child(
            div()
                .absolute()
                .left(px(5.25))
                .top(px(4.5))
                .w(px(54.))
                .h(px(47.))
                .overflow_hidden()
                .child(
                    solor::text("Ag 中 Ag Ag 中 Ag Ag")
                        .relative()
                        .left(px(-6.25))
                        .w(px(67.))
                        .text_size(px(18.))
                        .underline_wavy()
                        .underline_thickness(px(1.35))
                        .text_color(tint),
                ),
        )
    }))?;
    let ramp = linear_gradient(
        124.,
        [
            color_stop(rgb(0x032b56), 0.),
            color_stop(rgba(0xe3323e90), 0.52),
            color_stop(rgb(0x0b8247), 1.),
        ],
    )?;
    let angular = ramp
        .clone()
        .gradient_kind(GradientKind::Angular)
        .color_space(ColorSpace::Oklab)
        .phase(0.24)?;
    for color in [
        blue,
        ramp.into(),
        effect.with_params([19., 0.12, 1.4, 0.04]).into(),
        effect.with_params([43., 0.55, 0.75, 0.1]).into(),
        angular.into(),
        rgba(0xad153fa0).into(),
    ] {
        ui.context().set(tint, color)?;
        ui.flush()?;
        ui.layout(view, size)?;
        let frame = ui.record_frame(view)?;
        assert!(frame.draws().iter().any(
            |d| matches!(d, solor::Draw::Text { paragraph, .. } if paragraph.lines.len() > 1)
        ));
        assert!(
            frame
                .draws()
                .iter()
                .any(|d| matches!(d, solor::Draw::WavyUnderline { .. }))
        );
        for scale in [1., 1.25, 2.].map(RasterScale) {
            let physical = scale.buffer_size(size)?;
            let cpu = Surface::draw_view(&frame, physical, scale)?;
            let gpu = pixels(device, queue, renderer, &frame, physical, scale)?;
            assert!(cpu.rgba.iter().any(|c| *c < 180));
            for (i, (a, b)) in gpu.iter().zip(&cpu.rgba).enumerate() {
                assert!(
                    a.abs_diff(*b) <= 2,
                    "scale={}, byte={i}, GPU={a}, CPU={b}",
                    scale.0
                );
            }
        }
    }
    Ok(())
}

fn compare_svg(device: &wgpu::Device, queue: &wgpu::Queue, renderer: &mut Renderer) -> Result<()> {
    use solor::{Ink, SvgAsset, style::ObjectFit, svg};
    let art = SvgAsset::from_bytes(
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20">
        <defs><linearGradient id="ink"><stop stop-color="gold"/><stop offset="1" stop-color="red"/></linearGradient></defs>
        <path fill="url(#ink)" d="M0 0h17l10 20H0z"/><circle cx="23" cy="8" r="8" fill="blue" opacity=".6"/>
    </svg>"#,
    )?;
    let mut ui = Ui::new();
    let solid = Ink::from(rgba(0x2745adcc));
    let tint = ui.context().signal(solid.clone());
    let fit = ui.context().signal(ObjectFit::Cover);
    let view = ui.mount(component(move |_| {
        div()
            .w_full()
            .h_full()
            .bg(rgb(0x93b3ae))
            .text_color(tint)
            .child(
                svg(art.clone())
                    .absolute()
                    .top(px(1.5))
                    .left(px(-3.25))
                    .w(px(29.))
                    .h(px(23.))
                    .object_fit(fit),
            )
            .child(
                svg(art.clone())
                    .absolute()
                    .top(px(12.25))
                    .left(px(26.5))
                    .w(px(22.))
                    .h(px(18.))
                    .object_fit(fit)
                    .rounded(px(6.)),
            )
            .child(
                svg(art)
                    .preserve_colors()
                    .absolute()
                    .left(px(43.25))
                    .top(px(-1.5))
                    .w(px(27.))
                    .h(px(19.))
                    .object_fit(fit)
                    .rounded(px(5.5)),
            )
    }))?;
    let viewport = Size {
        width: px(67.),
        height: px(32.),
    };
    let gradient = linear_gradient(
        127.,
        [
            color_stop(rgba(0x2730fe5a), 0.),
            color_stop(rgb(0xef5321), 1.),
        ],
    )?;
    let shader = EffectShader::wgsl(
        "fn effect(uv:vec2f, p:f32)->vec4f {return vec4f(uv.x, p, uv.y, 0.8);}",
    )?;
    for (ink, mode) in [
        (solid, ObjectFit::Cover),
        (gradient.into(), ObjectFit::Cover),
        (shader.with_params(0.28).into(), ObjectFit::Contain),
    ] {
        ui.context().set(tint, ink)?;
        ui.context().set(fit, mode)?;
        ui.flush()?;
        ui.layout(view, viewport)?;
        let frame = ui.record_frame(view)?;
        for scale in [1., 1.25, 2.].map(RasterScale) {
            let size = scale.buffer_size(viewport)?;
            let cpu = Surface::draw_view(&frame, size, scale)?;
            let gpu = pixels(device, queue, renderer, &frame, size, scale)?;
            assert!(cpu.rgba.iter().any(|c| *c < 180));
            for (i, (a, b)) in gpu.iter().zip(&cpu.rgba).enumerate() {
                let x = i / 4 % size.width as usize;
                let y = i / 4 / size.width as usize;
                assert!(
                    a.abs_diff(*b) <= 2,
                    "SVG fit={mode:?} scale={} at ({x},{y}) GPU={a}, CPU={b}",
                    scale.0
                );
            }
        }
    }
    Ok(())
}
