use anyhow::{Result, ensure};
use solor::{
    ImageAsset,
    raster::{DeviceSize, svg::SvgRaster},
};
use std::borrow::Cow;
use std::collections::{HashMap, hash_map::Entry};

pub(super) struct Images {
    bindings: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    resident: HashMap<TextureSource, Texture>,
}

struct Texture {
    pixels: wgpu::Texture,
    group: wgpu::BindGroup,
}

fn bind_texture(
    device: &wgpu::Device,
    bindings: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    pixels: &wgpu::Texture,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Solor image"),
        layout: bindings,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(
                    &pixels.create_view(&Default::default()),
                ),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

impl Texture {
    fn new(
        device: &wgpu::Device,
        bindings: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        extent: wgpu::Extent3d,
    ) -> Self {
        let pixels = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Solor image"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let group = bind_texture(device, bindings, sampler, &pixels);
        Texture { pixels, group }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) enum TextureSource {
    Bitmap(ImageAsset),
    Svg(SvgRaster),
}

impl TextureSource {
    fn dimensions(&self) -> DeviceSize {
        match self {
            Self::Bitmap(asset) => asset.dimensions(),
            Self::Svg(region) => region.size(),
        }
    }

    fn pixels(&self) -> Result<Cow<'_, [u8]>> {
        Ok(match self {
            Self::Bitmap(asset) => Cow::Borrowed(bytemuck::cast_slice(asset.pixels())),
            Self::Svg(region) => Cow::Owned(region.rgba()?),
        })
    }
}

#[derive(Default)]
pub(super) struct FrameImages {
    indices: HashMap<TextureSource, usize>,
    assets: Vec<TextureSource>,
}

impl FrameImages {
    pub(super) fn index(&mut self, asset: TextureSource) -> usize {
        *self.indices.entry(asset.clone()).or_insert_with(|| {
            let i = self.assets.len();
            self.assets.push(asset);
            i
        })
    }
}

impl Images {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty,
            count: None,
        };
        let bindings = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Solor image"),
            entries: &[
                entry(
                    0,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(
                    1,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            bindings,
            sampler,
            resident: HashMap::new(),
        }
    }

    pub(super) fn bind(&self, device: &wgpu::Device, pixels: &wgpu::Texture) -> wgpu::BindGroup {
        bind_texture(device, &self.bindings, &self.sampler, pixels)
    }

    pub(super) fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.bindings
    }

    pub(super) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: FrameImages,
    ) -> Result<Vec<wgpu::BindGroup>> {
        // Only recycle textures whose old pixels are unused by every draw in this submission.
        let mut reusable = HashMap::<(u32, u32), Vec<Texture>>::new();
        for (asset, texture) in self
            .resident
            .extract_if(|asset, _| !frame.indices.contains_key(asset))
        {
            let size = asset.dimensions();
            reusable
                .entry((size.width, size.height))
                .or_default()
                .push(texture);
        }
        let mut groups = Vec::with_capacity(frame.assets.len());
        for asset in frame.assets {
            let size = asset.dimensions();
            let entry = match self.resident.entry(asset) {
                Entry::Occupied(entry) => entry.into_mut(),
                Entry::Vacant(entry) => {
                    ensure!(
                        size.width.max(size.height) <= device.limits().max_texture_dimension_2d,
                        "image exceeds GPU texture dimensions"
                    );
                    let extent = wgpu::Extent3d {
                        width: size.width,
                        height: size.height,
                        depth_or_array_layers: 1,
                    };
                    let texture = reusable
                        .get_mut(&(size.width, size.height))
                        .and_then(Vec::pop)
                        .unwrap_or_else(|| {
                            Texture::new(device, &self.bindings, &self.sampler, extent)
                        });
                    queue.write_texture(
                        texture.pixels.as_image_copy(),
                        &entry.key().pixels()?,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(size.width * 4),
                            rows_per_image: None,
                        },
                        extent,
                    );
                    entry.insert(texture)
                }
            };
            groups.push(entry.group.clone());
        }
        Ok(groups)
    }
}
