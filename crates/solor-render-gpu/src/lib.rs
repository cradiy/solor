//! Draws Solor frames into a GPU texture.
mod effect;
mod images;
mod masks;
mod path;
use anyhow::{Result, ensure};
use images::{FrameImages, Images, TextureSource};
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
    texture_layout: wgpu::PipelineLayout,
    brush: wgpu::RenderPipeline,
    border: Option<wgpu::RenderPipeline>,
    path: Option<wgpu::RenderPipeline>,
    underline: Option<wgpu::RenderPipeline>,
    images: Images,
    masks: masks::Atlas,
    rasterizer: GlyphRasterizer,
    mask_pipeline: wgpu::RenderPipeline,
    files: solor::ImageCache,
    image_pipeline: wgpu::RenderPipeline,
    effects: Vec<effect::Pipeline>,
    buffers: Option<Buffers>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Material {
    Brush,
    Border,
    Path,
    Underline,
    Effect(usize),
    Image(usize),
    Mask { page: usize, effect: Option<usize> },
}

struct Buffers {
    quads: wgpu::Buffer,
    stops: wgpu::Buffer,
    group: wgpu::BindGroup,
}

// Leave room for the full pattern period and its neighboring band in shader arithmetic.
fn pattern_span(device_pixels: f64) -> f32 {
    device_pixels.min(f64::from(f32::MAX) / 4.) as f32
}

fn rgba(color: Rgba) -> [f32; 4] {
    [color.red, color.green, color.blue, color.alpha].map(|c| f32::from(c) / 255.)
}

struct GradientBrush {
    params: [f32; 4],
    ramp: [f32; 4],
    oklab: f32,
}

impl GradientBrush {
    fn prepare(
        g: &solor::Gradient,
        size: solor::layout::Size,
        stops: &mut Vec<[[f32; 4]; 2]>,
    ) -> Self {
        let (w, h) = (size.width.get(), size.height.get());
        let space = g.interpolation_space();
        let oklab = f32::from(space == ColorSpace::Oklab);
        let mut ramp = [0.; 4];
        ramp[0] = stops.len() as f32;
        ramp[1] = g.stops().len() as f32;
        for (index, s) in g.stops().iter().enumerate() {
            let color = color::AlphaColor::<color::Srgb>::new(rgba(s.color));
            let components = match space {
                ColorSpace::Srgb => color.premultiply().components,
                ColorSpace::Oklab => color.convert::<color::Oklab>().premultiply().components,
            };
            let midpoint = g.gradient_midpoint_at(index).unwrap_or(0.5);
            stops.push([components, [s.offset, 0.5f32.ln() / midpoint.ln(), 0., 0.]]);
        }
        ramp[2] = g.repeat_phase().unwrap_or(-1.);
        ramp[3] = g.seam_width();
        let angle = (g.angle_degrees() - 90.).to_radians();
        let (sin, cos) = angle.sin_cos();
        let opacity = g.alpha_multiplier();
        let params = match g.kind() {
            GradientKind::Linear => {
                let (dx, dy) = (cos * w / w.max(h), sin * h / w.max(h));
                let length = dx.hypot(dy) * if dx.abs() > dy.abs() { w } else { h };
                [dx / length, dy / length, opacity, 0.]
            }
            GradientKind::Radial => [0., 0., opacity, 1.],
            GradientKind::Angular => [angle, 0., opacity, 2.],
            GradientKind::Diamond => [cos, sin, opacity, 3.],
        };
        Self {
            params,
            ramp,
            oklab,
        }
    }
}

const BRUSH: &str = concat!(include_str!("gradient.wgsl"), include_str!("brush.wgsl"));

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
        let brush = pipeline(device, &layout, format, BRUSH, effect::Target::Fill);
        let images = Images::new(device);
        let texture_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            bind_group_layouts: &[Some(&bindings), Some(images.layout())],
            ..Default::default()
        });
        let image_pipeline = pipeline(
            device,
            &texture_layout,
            format,
            include_str!("image.wgsl"),
            effect::Target::Fill,
        );
        let mask_pipeline = pipeline(
            device,
            &texture_layout,
            format,
            concat!(
                include_str!("gradient.wgsl"),
                "fn solor_foreground(local: vec2f, size: vec2f, q: solor_Quad, ramp: vec4f) -> vec4f {
                    return gradient_color(local, size, q.params, ramp, q.viewport.w != 0.);
                }",
                include_str!("mask.wgsl")
            ),
            effect::Target::Mask,
        );
        Ok(Self {
            mask_pipeline,
            underline: None,
            border: None,
            path: None,
            masks: Default::default(),
            rasterizer: Default::default(),
            images,
            files: Default::default(),
            image_pipeline,
            device: device.clone(),
            queue: queue.clone(),
            format,
            bindings,
            layout,
            texture_layout,
            brush,
            effects: Vec::new(),
            buffers: None,
        })
    }

    /// Clears the target and draws fills, solid borders, paths, text, wavy underlines and SVGs in paint order.
    pub fn draw(
        &mut self,
        frame: &ViewFrame,
        target: &wgpu::TextureView,
        size: DeviceSize,
        scale: RasterScale,
        clear: wgpu::Color,
    ) -> Result<()> {
        let result = self.draw_frame(frame, target, size, scale, clear);
        self.masks.trim();
        self.effects.retain_mut(|p| std::mem::take(&mut p.used));
        result
    }

    fn effect_pipeline(&mut self, shader: &EffectShader, target: effect::Target) -> Result<usize> {
        if let Some(index) = self
            .effects
            .iter()
            .position(|p| p.shader == *shader && p.target == target)
        {
            self.effects[index].used = true;
            return Ok(index);
        }
        let layout = match target {
            effect::Target::Fill | effect::Target::Underline => &self.layout,
            effect::Target::Mask => &self.texture_layout,
        };
        let pipeline = effect::pipeline(
            &self.device,
            layout,
            self.format,
            solor::raster::shader_program(shader),
            target,
        )?;
        let index = self.effects.len();
        self.effects.push(effect::Pipeline {
            shader: shader.clone(),
            target,
            pipeline,
            used: true,
        });
        Ok(index)
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
        for (index, cmd) in frame.draws().iter().enumerate() {
            let unsupported = |reason: &str| {
                anyhow::anyhow!(
                    "GPU draw #{} (Draw::{}): {reason}",
                    index + 1,
                    draw_name(cmd)
                )
            };
            if let Some(path) = path::Path::from_draw(cmd).map_err(unsupported)? {
                let first = quads.len();
                path.record(size, scale, &mut quads, &mut stops)?;
                if quads.len() > first {
                    batches.push((Material::Path, first as u32..quads.len() as u32));
                }
                continue;
            }
            let raster = match cmd {
                Draw::Text {
                    clip,
                    origin,
                    size,
                    color,
                    ..
                } => Some((clip, origin, size, Some(color))),
                Draw::Svg {
                    clip,
                    origin,
                    size,
                    tint,
                    ..
                } => Some((clip, origin, size, tint.as_ref())),
                _ => None,
            };
            if let Some((clip, origin, extent, ink)) = raster {
                let viewport = scale.bounds(*origin, *extent);
                let mut bounds = viewport.intersect(Clip::canvas(size));
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
                let clip_edges = [
                    bounds.left as f32,
                    bounds.top as f32,
                    bounds.right as f32,
                    bounds.bottom as f32,
                ];
                let svg = if let Draw::Svg {
                    asset,
                    rounded_clip,
                    ..
                } = cmd
                {
                    let region = solor::raster::svg::SvgRaster::new(asset, viewport, bounds)?;
                    let region_size = region.size();
                    let width = region_size.width as f32;
                    let height = region_size.height as f32;
                    let rect = [clip_edges[0], clip_edges[1], width, height];
                    let (rect, radii) = rounded_clip.as_ref().map_or((rect, [0.; 4]), |clip| {
                        let device = |v: solor::Pixels| (f64::from(v) * scale.0) as f32;
                        let r = clip.radii;
                        (
                            [
                                device(clip.origin.x),
                                device(clip.origin.y),
                                device(clip.size.width),
                                device(clip.size.height),
                            ],
                            [r.top_left, r.top_right, r.bottom_right, r.bottom_left].map(device),
                        )
                    });
                    if ink.is_none() {
                        let image = frame_images.index(TextureSource::Svg(region));
                        let index = quads.len() as u32;
                        quads.push([
                            rect,
                            radii,
                            clip_edges,
                            [
                                clip_edges[0] - rect[0],
                                clip_edges[1] - rect[1],
                                width,
                                height,
                            ],
                            [size.width as f32, size.height as f32, 1., 1.],
                            [0., 0., width, height],
                        ]);
                        batch(&mut batches, Material::Image(image), index);
                        continue;
                    }
                    Some((region, rect, radii))
                } else {
                    None
                };
                let mut effect = None;
                let (params, ramp, oklab) = match ink {
                    Some(solor::Ink::Solid(color)) if color.alpha == 0 => continue,
                    Some(solor::Ink::Solid(color)) => (rgba(*color), None, 0.),
                    Some(solor::Ink::Gradient(g)) => {
                        let brush = GradientBrush::prepare(g, *extent, &mut stops);
                        (brush.params, Some(brush.ramp), brush.oklab)
                    }
                    Some(solor::Ink::Shader(fill)) => {
                        effect = Some(self.effect_pipeline(&fill.shader, effect::Target::Mask)?);
                        (fill.params, Some([0.; 4]), 0.)
                    }
                    _ => return Err(unsupported("unsupported mask ink")),
                };
                let paint_box = ramp.map_or(0., |ramp| {
                    let index = stops.len();
                    stops.push([
                        [
                            origin.x.get(),
                            origin.y.get(),
                            extent.width.get(),
                            extent.height.get(),
                        ],
                        ramp,
                    ]);
                    index as f32
                });
                let mut emit = |page, rect: [f32; 4], radii, uv: [f32; 2]| {
                    let index = quads.len() as u32;
                    quads.push([
                        rect,
                        radii,
                        clip_edges,
                        params,
                        [size.width as f32, size.height as f32, 1., oklab],
                        [uv[0], uv[1], paint_box, scale.0 as f32],
                    ]);
                    batch(&mut batches, Material::Mask { page, effect }, index);
                };
                if let Some((region, rect, radii)) = svg {
                    let tile = self
                        .masks
                        .svg(&self.device, &self.queue, &self.images, region)?;
                    emit(
                        tile.page,
                        rect,
                        radii,
                        [
                            tile.uv[0] as f32 + (rect[0] - bounds.left as f32),
                            tile.uv[1] as f32 + (rect[1] - bounds.top as f32),
                        ],
                    );
                } else if let Draw::Text { paragraph, .. } = cmd {
                    self.rasterizer.visit(
                        paragraph,
                        *origin,
                        scale,
                        bounds,
                        |key, [x, y], rasterize| {
                            let Some(tile) = self.masks.resolve(
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
                            emit(
                                tile.page,
                                [
                                    left as f32,
                                    top as f32,
                                    tile.size[0] as f32,
                                    tile.size[1] as f32,
                                ],
                                [0.; 4],
                                tile.uv.map(|v| v as f32),
                            );
                            Ok(())
                        },
                    )?;
                }
                continue;
            }
            if let Draw::Border {
                origin,
                size: extent,
                paint,
                clip,
            } = cmd
            {
                let Some(bounds) = visible_bounds(*origin, *extent, clip.as_deref(), size, scale)
                else {
                    continue;
                };
                let styles = paint.styles;
                let mut colors = [[0.; 4]; 4];
                for ((out, fill), style) in colors.iter_mut().zip(paint.visible_fills()).zip([
                    styles.top,
                    styles.right,
                    styles.bottom,
                    styles.left,
                ]) {
                    let Some(fill) = fill else { continue };
                    let Fill::Solid(color) = fill else {
                        return Err(unsupported("borders require solid colors"));
                    };
                    if style != solor::style::BorderStyle::Solid {
                        return Err(unsupported(&format!("unsupported border style {style:?}")));
                    }
                    let [r, g, b, a] = rgba(*color);
                    *out = [r * a, g * a, b * a, a];
                }
                let index = quads.len() as u32;
                let colors_start = stops.len() as f32;
                let [t, r, b, l] = colors;
                stops.extend([[t, r], [b, l]]);
                let r = paint.radii.fit_to(*extent);
                let w = paint.widths;
                quads.push([
                    [
                        origin.x.get(),
                        origin.y.get(),
                        extent.width.get(),
                        extent.height.get(),
                    ],
                    [r.top_left, r.top_right, r.bottom_right, r.bottom_left]
                        .map(solor::Pixels::get),
                    bounds,
                    [w.top, w.right, w.bottom, w.left].map(solor::Pixels::get),
                    [size.width as f32, size.height as f32, scale.0 as f32, 0.],
                    [colors_start, 0., 0., 0.],
                ]);
                batch(&mut batches, Material::Border, index);
                continue;
            }
            let underline_fill;
            let (origin, extent, clip, radii, fill, mut material) = match cmd {
                Draw::Fill {
                    origin,
                    size,
                    clip,
                    corner_radii: r,
                    fill,
                } => (
                    origin,
                    size,
                    clip,
                    [
                        r.top_left.get(),
                        r.top_right.get(),
                        r.bottom_right.get(),
                        r.bottom_left.get(),
                    ],
                    fill,
                    Material::Brush,
                ),
                Draw::WavyUnderline {
                    origin,
                    size,
                    clip,
                    color,
                } => {
                    underline_fill = color.clone().into();
                    (
                        origin,
                        size,
                        clip,
                        [0.; 4],
                        &underline_fill,
                        Material::Underline,
                    )
                }
                _ => return Err(unsupported("command not implemented")),
            };
            let [w, h] = [extent.width.get(), extent.height.get()];
            if w == 0. || h == 0. {
                continue;
            }
            let Some(bounds) = visible_bounds(*origin, *extent, clip.as_deref(), size, scale)
            else {
                continue;
            };
            let mut material_option = 0.;
            let mut ramp = [0.; 4];
            let fill = self.files.resolve(fill)?;
            let params = match fill.as_ref() {
                Fill::Solid(color) => rgba(*color),
                Fill::Checkerboard(pattern) => {
                    material_option = 2.;
                    ramp[0] = pattern_span(f64::from(pattern.cell_size()) * scale.0);
                    rgba(pattern.color)
                }
                Fill::DiagonalStripes(pattern) => {
                    material_option = 3.;
                    let axis = scale.0 * std::f64::consts::SQRT_2;
                    let width = f64::from(pattern.width()) * axis;
                    let gap = f64::from(pattern.gap()) * axis;
                    let period = width + gap;
                    let side = 1. % period;
                    // Full periods in a device pixel contribute their mean density.
                    let whole = (1. - side * side) * (width / period);
                    ramp = [
                        pattern_span(width),
                        pattern_span(gap),
                        side as f32,
                        whole as f32,
                    ];
                    rgba(pattern.color)
                }
                Fill::Gradient(g) => {
                    let brush = GradientBrush::prepare(g, *extent, &mut stops);
                    material_option = brush.oklab;
                    ramp = brush.ramp;
                    brush.params
                }
                Fill::Shader(f) => {
                    let target = if material == Material::Underline {
                        effect::Target::Underline
                    } else {
                        effect::Target::Fill
                    };
                    material = Material::Effect(self.effect_pipeline(&f.shader, target)?);
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
                    material = Material::Image(
                        frame_images.index(TextureSource::Bitmap(image.asset().clone())),
                    );
                    [
                        offset.x.get(),
                        offset.y.get(),
                        fit.width.get(),
                        fit.height.get(),
                    ]
                }
                _ => return Err(unsupported("unsupported fill")),
            };
            let index = quads.len() as u32;
            quads.push([
                [origin.x.get(), origin.y.get(), w, h],
                radii,
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
                    Material::Path => {
                        pass.set_pipeline(self.path.get_or_insert_with(|| {
                            module_pipeline(
                                &self.device,
                                &self.layout,
                                self.format,
                                concat!(
                                    include_str!("vertex.wgsl"),
                                    "\n",
                                    include_str!("gradient.wgsl"),
                                    "\n",
                                    include_str!("path.wgsl")
                                )
                                .to_owned(),
                                "solor_",
                            )
                        }));
                    }
                    Material::Border => {
                        pass.set_pipeline(self.border.get_or_insert_with(|| {
                            module_pipeline(
                                &self.device,
                                &self.layout,
                                self.format,
                                concat!(
                                    include_str!("vertex.wgsl"),
                                    "\n",
                                    include_str!("border.wgsl")
                                )
                                .to_owned(),
                                "solor_",
                            )
                        }));
                    }
                    Material::Underline => {
                        pass.set_pipeline(self.underline.get_or_insert_with(|| {
                            pipeline(
                                &self.device,
                                &self.layout,
                                self.format,
                                BRUSH,
                                effect::Target::Underline,
                            )
                        }))
                    }
                    Material::Effect(i) => pass.set_pipeline(&self.effects[i].pipeline),
                    Material::Mask { page, effect } => {
                        pass.set_pipeline(
                            effect.map_or(&self.mask_pipeline, |i| &self.effects[i].pipeline),
                        );
                        pass.set_bind_group(1, self.masks.group(page), &[]);
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

fn draw_name(draw: &Draw) -> &'static str {
    match draw {
        Draw::Fill { .. } => "Fill",
        Draw::WavyUnderline { .. } => "WavyUnderline",
        Draw::Text { .. } => "Text",
        Draw::Svg { .. } => "Svg",
        Draw::Shadow { .. } => "Shadow",
        Draw::Border { .. } => "Border",
        Draw::Group { .. } => "Group",
        Draw::BackdropBlur { .. } => "BackdropBlur",
        Draw::Polygon { .. } => "Polygon",
        Draw::Polyline { .. } => "Polyline",
    }
}

fn visible_bounds(
    origin: solor::layout::Origin,
    extent: solor::layout::Size,
    clip: Option<&solor::ContentClip>,
    size: DeviceSize,
    scale: RasterScale,
) -> Option<[f32; 4]> {
    let [w, h] = [extent.width.get(), extent.height.get()];
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
        return None;
    }
    Some(bounds)
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
    target: effect::Target,
) -> wgpu::RenderPipeline {
    module_pipeline(device, layout, format, target.module(body), "solor_")
}

fn module_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    format: TextureFormat,
    source: String,
    prefix: &str,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Solor quad"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Solor fills"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some(&format!("{prefix}vertex")),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some(&format!("{prefix}fragment")),
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
