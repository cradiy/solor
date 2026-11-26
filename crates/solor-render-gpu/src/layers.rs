use super::{Batch, GradientBrush, Images};
use solor::{ColorSpace, GradientKind, GradientMap, raster::DeviceSize};

pub(super) struct Pass {
    pub depth: usize,
    pub load: wgpu::LoadOp<wgpu::Color>,
    pub blur: Option<super::backdrop::Blur>,
    pub batches: Vec<Batch>,
}

impl Pass {
    pub fn new(depth: usize, load: wgpu::LoadOp<wgpu::Color>) -> Self {
        Self {
            depth,
            load,
            blur: None,
            batches: Vec::new(),
        }
    }
}

pub(super) struct Layer {
    pub size: DeviceSize,
    pub view: wgpu::TextureView,
    pub binding: wgpu::BindGroup,
}

impl Layer {
    pub fn new(
        device: &wgpu::Device,
        images: &Images,
        size: DeviceSize,
        format: wgpu::TextureFormat,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Solor subtree"),
            size: wgpu::Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        Self {
            size,
            view: texture.create_view(&Default::default()),
            binding: images.bind(device, &texture),
        }
    }
}

pub(super) fn composite_quad(
    size: DeviceSize,
    alpha: u8,
    map: Option<&GradientMap>,
    stops: &mut Vec<[[f32; 4]; 2]>,
) -> [[f32; 4]; 6] {
    let (w, h) = (size.width as f32, size.height as f32);
    let mut quad = [[0.; 4]; 6];
    quad[0] = [0., 0., w, h];
    quad[4] = [w, h, 1., f32::from(alpha) / 255.];
    if let Some(map) = map {
        let palette = map.palette();
        quad[3] = [
            map.strength(),
            f32::from(palette.interpolation_space() == ColorSpace::Oklab),
            palette.alpha_multiplier(),
            f32::from(palette.kind() == GradientKind::Angular),
        ];
        quad[5] = GradientBrush::prepare_ramp(palette, stops);
    }
    quad
}
