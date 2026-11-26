use super::{FrameData, Material, layers::Pass, module_pipeline};
use solor::{
    Draw,
    raster::{Clip, DeviceSize, RasterScale},
};

pub(super) struct Blur {
    size: DeviceSize,
    quad: u32,
}

impl Blur {
    pub fn record(
        draw: &Draw,
        canvas: DeviceSize,
        scale: RasterScale,
        depth: usize,
        clip_id: u32,
        frame: &mut FrameData,
    ) {
        let Draw::BackdropBlur {
            origin,
            size,
            clip,
            radius,
            corner_radii,
        } = draw
        else {
            unreachable!()
        };
        let physical = |v: solor::Pixels| f64::from(v) * scale.0;
        let (x, y) = (physical(origin.x), physical(origin.y));
        let bounds = Clip {
            left: x.floor() as i64,
            top: y.floor() as i64,
            right: (x + physical(size.width)).ceil() as i64,
            bottom: (y + physical(size.height)).ceil() as i64,
        };
        let output = super::clipped_pixels(bounds, clip.as_deref(), canvas, scale);
        if output.empty() {
            return;
        }
        let radius = physical(*radius).min(f64::from(canvas.width.max(canvas.height)));
        let halo = radius.ceil() as i64;
        let top = (output.top - halo).max(0);
        let bottom = (output.bottom + halo).min(i64::from(canvas.height));
        let work = DeviceSize {
            width: (output.right - output.left) as u32,
            height: (bottom - top) as u32,
        };
        let quad = frame.quads.len() as u32;
        let params = [
            output.left as f32,
            top as f32,
            radius as f32,
            work.height as f32,
        ];
        frame.quads.push([
            [0., 0., work.width as f32, work.height as f32],
            [0.; 4],
            [0.; 4],
            params,
            [work.width as f32, work.height as f32, 1., 0.],
            [0.; 4],
        ]);
        frame.quads.push([
            [
                origin.x.get(),
                origin.y.get(),
                size.width.get(),
                size.height.get(),
            ],
            [
                corner_radii.top_left,
                corner_radii.top_right,
                corner_radii.bottom_right,
                corner_radii.bottom_left,
            ]
            .map(solor::Pixels::get),
            [output.left, output.top, output.right, output.bottom].map(|e| e as f32),
            params,
            [
                canvas.width as f32,
                canvas.height as f32,
                scale.0 as f32,
                0.,
            ],
            [0.; 4],
        ]);
        let mut resume = Pass::new(depth, wgpu::LoadOp::Load);
        resume.blur = Some(Self { size: work, quad });
        super::batch(
            &mut resume.batches,
            Material::Blur,
            clip_id,
            quad + 1..quad + 2,
        );
        frame.passes.push(resume);
    }
}

struct Rows {
    view: wgpu::TextureView,
    group: wgpu::BindGroup,
    size: DeviceSize,
}

pub(super) struct Filter {
    horizontal: wgpu::RenderPipeline,
    pub vertical: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    rows: Option<Rows>,
}

impl Filter {
    pub fn new(
        device: &wgpu::Device,
        bindings: &wgpu::BindGroupLayout,
        texture_layout: &wgpu::PipelineLayout,
        format: wgpu::TextureFormat,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Solor filtered rows"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Uint,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let vertical_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            bind_group_layouts: &[Some(bindings), Some(&layout)],
            ..Default::default()
        });
        let horizontal = module_pipeline(
            device,
            texture_layout,
            wgpu::TextureFormat::Rg32Uint,
            concat!(
                include_str!("vertex.wgsl"),
                include_str!("blur_kernel.wgsl"),
                include_str!("blur_horizontal.wgsl")
            )
            .to_owned(),
            "solor_",
            None,
        );
        let vertical = module_pipeline(
            device,
            &vertical_layout,
            format,
            concat!(
                include_str!("vertex.wgsl"),
                include_str!("coverage.wgsl"),
                include_str!("blur_kernel.wgsl"),
                include_str!("blur_vertical.wgsl")
            )
            .to_owned(),
            "solor_",
            None,
        );
        Self {
            horizontal,
            vertical,
            layout,
            rows: None,
        }
    }

    pub fn prepare(&mut self, device: &wgpu::Device, passes: &[Pass], canvas: DeviceSize) {
        // Bucket by tile size so animating a fractional radius can reuse the row texture.
        let mut size = DeviceSize {
            width: 0,
            height: 0,
        };
        for blur in passes.iter().filter_map(|p| p.blur.as_ref()) {
            size.width = size.width.max(blur.size.width);
            size.height = size.height.max(blur.size.height);
        }
        if size.width == 0 {
            self.rows = None;
            return;
        }
        size.width = size.width.next_multiple_of(32).min(canvas.width);
        size.height = size.height.next_multiple_of(32).min(canvas.height);
        if self.rows.as_ref().is_some_and(|r| r.size == size) {
            return;
        }
        let view = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Solor backdrop rows"),
                size: wgpu::Extent3d {
                    width: size.width,
                    height: size.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rg32Uint,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Solor backdrop rows"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        self.rows = Some(Rows { view, group, size });
    }

    pub fn rows(&self) -> &wgpu::BindGroup {
        &self.rows.as_ref().expect("prepared blur rows").group
    }

    pub fn filter_rows(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        data: &wgpu::BindGroup,
        source: &wgpu::BindGroup,
        blur: &Blur,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.rows.as_ref().expect("prepared blur rows").view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_viewport(
            0.,
            0.,
            blur.size.width as f32,
            blur.size.height as f32,
            0.,
            1.,
        );
        pass.set_scissor_rect(0, 0, blur.size.width, blur.size.height);
        pass.set_bind_group(0, data, &[]);
        pass.set_bind_group(1, source, &[]);
        pass.set_pipeline(&self.horizontal);
        pass.draw(0..6, blur.quad..blur.quad + 1);
    }
}
