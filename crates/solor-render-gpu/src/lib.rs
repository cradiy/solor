//! Draws Solor frames into a GPU texture.
mod effect;
mod glyphs;
mod images;
use anyhow::{Result, bail, ensure};
use images::{FrameImages, Images};
use solor::{
    ColorSpace, Draw, EffectShader, Fill, GradientKind, Rgba, ViewFrame,
    raster::{Clip, DeviceSize, RasterScale, glyphs::GlyphRasterizer},
};
use std::ops::Range;
use wgpu::{BufferUsages, ShaderStages, TextureFormat};

/// Pipelines and upload buffers for one device and SDR render-target format.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    format: TextureFormat,
    layout: wgpu::PipelineLayout,
    bindings: wgpu::BindGroupLayout,
    brush: wgpu::RenderPipeline,
    images: Images,
    glyphs: glyphs::Atlas,
    rasterizer: GlyphRasterizer,
    glyph_pipeline: wgpu::RenderPipeline,
    files: solor::ImageCache,
    image_pipeline: wgpu::RenderPipeline,
    effects: Vec<(EffectShader, wgpu::RenderPipeline)>,
    buffers: Option<Buffers>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Material {
    Brush,
    Effect(usize),
    Image(usize),
    Glyph(usize),
}

struct Buffers {
    quads: wgpu::Buffer,
    stops: wgpu::Buffer,
    group: wgpu::BindGroup,
}

fn rgba(color: Rgba) -> [f32; 4] {
    [color.red, color.green, color.blue, color.alpha].map(|c| f32::from(c) / 255.)
}

impl Renderer {
    /// Creates a renderer for an RGBA8 or BGRA8 unorm target using encoded sRGB colors.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: TextureFormat) -> Result<Self> {
        ensure!(
            matches!(
                format,
                TextureFormat::Rgba8Unorm | TextureFormat::Bgra8Unorm
            ),
            "unsupported Solor render target {format:?}"
        );
        let entry = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let bindings = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Solor fills"),
            entries: &[
                entry(0, ShaderStages::VERTEX_FRAGMENT),
                entry(1, ShaderStages::FRAGMENT),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            bind_group_layouts: &[Some(&bindings)],
            ..Default::default()
        });
        let brush = pipeline(device, &layout, format, include_str!("brush.wgsl"));
        let images = Images::new(device);
        let image_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            bind_group_layouts: &[Some(&bindings), Some(images.layout())],
            ..Default::default()
        });
        let image_pipeline = pipeline(device, &image_layout, format, include_str!("image.wgsl"));
        let glyph_pipeline = pipeline(
            device,
            &image_layout,
            format,
            "fn paint(local: vec2f, size: vec2f, q: Quad) -> vec4f {
                let alpha = textureLoad(bitmap, vec2i(q.ramp.xy + floor(local)), 0).r;
                return vec4f(q.params.rgb, q.params.a * alpha);
            }",
        );
        Ok(Self {
            glyph_pipeline,
            glyphs: Default::default(),
            rasterizer: Default::default(),
            images,
            files: Default::default(),
            image_pipeline,
            device: device.clone(),
            queue: queue.clone(),
            format,
            bindings,
            layout,
            brush,
            effects: Vec::new(),
            buffers: None,
        })
    }

    /// Clears and draws a frame in paint order. Supports solid text and solid, gradient, image and WGSL fills.
    pub fn draw(
        &mut self,
        frame: &ViewFrame,
        target: &wgpu::TextureView,
        size: DeviceSize,
        scale: RasterScale,
        clear: wgpu::Color,
    ) -> Result<()> {
        let result = self.draw_frame(frame, target, size, scale, clear);
        self.glyphs.trim();
        result
    }

    fn draw_frame(
        &mut self,
        frame: &ViewFrame,
        target: &wgpu::TextureView,
        size: DeviceSize,
        scale: RasterScale,
        clear: wgpu::Color,
    ) -> Result<()> {
        let mut quads = Vec::<[[f32; 4]; 6]>::new();
        let mut stops = vec![[[0f32; 4]; 2]];
        let mut frame_images = FrameImages::default();
        let mut batches: Vec<(Material, Range<u32>)> = Vec::new();
        // Release compiled effects after the last view frame using them disappears.
        self.effects.retain(|(shader, _)| {
            frame
                .draws()
                .iter()
                .any(|d| matches!(d,Draw::Fill{fill:Fill::Shader(f),..} if f.shader==*shader))
        });
        for cmd in frame.draws() {
            if let Draw::Text {
                clip,
                origin,
                size: extent,
                paragraph,
                color,
            } = cmd
            {
                let mut bounds = scale.bounds(*origin, *extent).intersect(Clip::canvas(size));
                if let Some(clip) = clip {
                    if let Some(x) = &clip.x {
                        bounds.left = bounds.left.max(scale.edge(x.start.into()));
                        bounds.right = bounds.right.min(scale.edge(x.end.into()));
                    }
                    if let Some(y) = &clip.y {
                        bounds.top = bounds.top.max(scale.edge(y.start.into()));
                        bounds.bottom = bounds.bottom.min(scale.edge(y.end.into()));
                    }
                }
                if bounds.empty() {
                    continue;
                }
                let solor::Ink::Solid(color) = color else {
                    bail!("GPU text requires solid ink")
                };
                if color.alpha == 0 {
                    continue;
                }
                self.rasterizer.visit(
                    paragraph,
                    *origin,
                    scale,
                    bounds,
                    |key, [x, y], rasterize| {
                        let Some(tile) = self.glyphs.resolve(
                            &self.device,
                            &self.queue,
                            &self.images,
                            key,
                            rasterize,
                        )?
                        else {
                            return Ok(());
                        };
                        let left = x.saturating_add(i64::from(tile.offset[0]));
                        let top = y.saturating_add(i64::from(tile.offset[1]));
                        if (Clip {
                            left,
                            top,
                            right: left.saturating_add(tile.size[0].into()),
                            bottom: top.saturating_add(tile.size[1].into()),
                        })
                        .intersect(bounds)
                        .empty()
                        {
                            return Ok(());
                        }
                        let index = quads.len() as u32;
                        quads.push([
                            [
                                left as f32,
                                top as f32,
                                tile.size[0] as f32,
                                tile.size[1] as f32,
                            ],
                            [0.; 4],
                            [
                                bounds.left as f32,
                                bounds.top as f32,
                                bounds.right as f32,
                                bounds.bottom as f32,
                            ],
                            rgba(*color),
                            [size.width as f32, size.height as f32, 1., 0.],
                            [tile.uv[0] as f32, tile.uv[1] as f32, 0., 0.],
                        ]);
                        batch(&mut batches, Material::Glyph(tile.page), index);
                        Ok(())
                    },
                )?;
                continue;
            }
            let Draw::Fill {
                origin,
                size: extent,
                clip,
                fill,
                corner_radii: r,
            } = cmd
            else {
                bail!("GPU fill renderer cannot draw this command yet")
            };
            let [w, h] = [extent.width.get(), extent.height.get()];
            if w == 0. || h == 0. {
                continue;
            }
            let mut bounds = [
                0.,
                0.,
                size.width as f32 / scale.0 as f32,
                size.height as f32 / scale.0 as f32,
            ];
            if let Some(c) = clip {
                if let Some(x) = &c.x {
                    bounds[0] = bounds[0].max(x.start.get());
                    bounds[2] = bounds[2].min(x.end.get());
                }
                if let Some(y) = &c.y {
                    bounds[1] = bounds[1].max(y.start.get());
                    bounds[3] = bounds[3].min(y.end.get());
                }
            }
            if bounds[0].max(origin.x.get()) >= bounds[2].min(origin.x.get() + w)
                || bounds[1].max(origin.y.get()) >= bounds[3].min(origin.y.get() + h)
            {
                continue;
            }
            let fill = self.files.resolve(fill)?;
            let mut material_option = 0.;
            let mut ramp = [0.; 4];
            let mut material = Material::Brush;
            let params = match fill.as_ref() {
                Fill::Solid(color) => rgba(*color),
                Fill::Gradient(g) => {
                    let space = g.interpolation_space();
                    material_option = f32::from(space == ColorSpace::Oklab);
                    ramp[0] = stops.len() as f32;
                    ramp[1] = g.stops().len() as f32;
                    for (index, s) in g.stops().iter().enumerate() {
                        let color = color::AlphaColor::<color::Srgb>::new(rgba(s.color));
                        let components = match space {
                            ColorSpace::Srgb => color.premultiply().components,
                            ColorSpace::Oklab => {
                                color.convert::<color::Oklab>().premultiply().components
                            }
                        };
                        let midpoint = g.gradient_midpoint_at(index).unwrap_or(0.5);
                        stops.push([components, [s.offset, 0.5f32.ln() / midpoint.ln(), 0., 0.]]);
                    }
                    ramp[2] = g.repeat_phase().unwrap_or(-1.);
                    ramp[3] = g.seam_width();
                    let angle = (g.angle_degrees() - 90.).to_radians();
                    let (sin, cos) = angle.sin_cos();
                    let opacity = g.alpha_multiplier();
                    match g.kind() {
                        GradientKind::Linear => {
                            let (dx, dy) = (cos * w / w.max(h), sin * h / w.max(h));
                            let length = dx.hypot(dy) * if dx.abs() > dy.abs() { w } else { h };
                            [dx / length, dy / length, opacity, 0.]
                        }
                        GradientKind::Radial => [0., 0., opacity, 1.],
                        GradientKind::Angular => [angle, 0., opacity, 2.],
                        GradientKind::Diamond => [cos, sin, opacity, 3.],
                    }
                }
                Fill::Shader(f) => {
                    let i = if let Some(i) = self.effects.iter().position(|(s, _)| s == &f.shader) {
                        i
                    } else {
                        let body = effect::paint(solor::raster::shader_program(&f.shader));
                        let i = self.effects.len();
                        self.effects.push((
                            f.shader.clone(),
                            pipeline(&self.device, &self.layout, self.format, &body),
                        ));
                        i
                    };
                    material = Material::Effect(i);
                    f.params
                }
                Fill::Image(image) => {
                    let (offset, fit) = image.bounds(*extent)?;
                    if fit.width.get() == 0. || fit.height.get() == 0. {
                        continue;
                    }
                    let src = image.source_bounds();
                    ramp = [
                        src.left as f32,
                        src.top as f32,
                        src.right as f32,
                        src.bottom as f32,
                    ];
                    material_option = f32::from(image.sampling() == solor::ImageSampling::Nearest);
                    material = Material::Image(frame_images.index(image.asset()));
                    [
                        offset.x.get(),
                        offset.y.get(),
                        fit.width.get(),
                        fit.height.get(),
                    ]
                }
                _ => bail!("unsupported GPU fill"),
            };
            let index = quads.len() as u32;
            quads.push([
                [origin.x.get(), origin.y.get(), w, h],
                [
                    r.top_left.get(),
                    r.top_right.get(),
                    r.bottom_right.get(),
                    r.bottom_left.get(),
                ],
                bounds,
                params,
                [
                    size.width as f32,
                    size.height as f32,
                    scale.0 as f32,
                    material_option,
                ],
                ramp,
            ]);
            batch(&mut batches, material, index);
        }
        let image_groups = self
            .images
            .prepare(&self.device, &self.queue, frame_images)?;
        let quad_bytes = bytemuck::cast_slice(&quads);
        let stop_bytes = bytemuck::cast_slice(&stops);
        let q_len = (quad_bytes.len() as u64).max(96);
        let s_len = stop_bytes.len() as u64;
        let limit = self.device.limits().max_storage_buffer_binding_size;
        ensure!(
            q_len.max(s_len) <= limit,
            "GPU fill data exceeds storage buffer limits"
        );
        if self
            .buffers
            .as_ref()
            .is_none_or(|b| b.quads.size() < q_len || b.stops.size() < s_len)
        {
            let buffer = |size: u64| {
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Solor fill data"),
                    size: size.next_power_of_two().min(limit),
                    usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            };
            let quads = buffer(q_len);
            let stops = buffer(s_len);
            let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.bindings,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: quads.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: stops.as_entire_binding(),
                    },
                ],
            });
            self.buffers = Some(Buffers {
                quads,
                stops,
                group,
            });
        }
        let buffers = self.buffers.as_ref().unwrap();
        self.queue.write_buffer(&buffers.quads, 0, quad_bytes);
        self.queue.write_buffer(&buffers.stops, 0, stop_bytes);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_bind_group(0, &buffers.group, &[]);
            for (id, range) in batches {
                match id {
                    Material::Brush => pass.set_pipeline(&self.brush),
                    Material::Effect(i) => pass.set_pipeline(&self.effects[i].1),
                    Material::Glyph(page) => {
                        pass.set_pipeline(&self.glyph_pipeline);
                        pass.set_bind_group(1, self.glyphs.group(page), &[]);
                    }
                    Material::Image(i) => {
                        pass.set_pipeline(&self.image_pipeline);
                        pass.set_bind_group(1, &image_groups[i], &[]);
                    }
                }
                pass.draw(0..6, range);
            }
        }
        self.queue.submit([encoder.finish()]);
        Ok(())
    }
}

fn batch(batches: &mut Vec<(Material, Range<u32>)>, material: Material, index: u32) {
    if let Some((id, span)) = batches.last_mut()
        && *id == material
    {
        span.end += 1;
    } else {
        batches.push((material, index..index + 1));
    }
}

fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    format: TextureFormat,
    body: &str,
) -> wgpu::RenderPipeline {
    let source = include_str!("quad.wgsl").replace("// PAINT", body);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Solor quad"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Solor fills"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vertex"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fragment"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}
