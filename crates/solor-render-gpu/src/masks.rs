use solor::raster::{
    DeviceSize,
    glyphs::{GlyphKey, GlyphMask},
    svg::SvgRaster,
};
use std::{
    collections::{HashMap, VecDeque},
    io::{Error, Result},
};

#[derive(Clone, Copy)]
pub(super) struct Tile {
    pub page: usize,
    pub uv: [u32; 2],
    pub size: [u32; 2],
    pub offset: [i32; 2],
}

struct Page {
    texture: wgpu::Texture,
    group: wgpu::BindGroup,
    x: u32,
    y: u32,
    row_height: u32,
    tile_count: usize,
}

impl Page {
    fn allocate(&mut self, width: u32, height: u32) -> Option<[u32; 2]> {
        if width > self.texture.width() {
            return None;
        }
        let (x, y) = if self.x + width > self.texture.width() {
            (0, self.y + self.row_height)
        } else {
            (self.x, self.y)
        };
        if y + height > self.texture.height() {
            return None;
        }
        self.row_height = if y == self.y {
            self.row_height.max(height)
        } else {
            height
        };
        self.x = x + width;
        self.y = y;
        Some([x, y])
    }
}

#[derive(Default)]
pub(super) struct Atlas {
    entries: HashMap<GlyphKey, Option<Tile>>,
    svgs: HashMap<SvgRaster, Tile>,
    storage: Pages,
    scratch: GlyphMask,
}

#[derive(Default)]
struct Pages {
    pages: VecDeque<Page>,
    first_page: usize,
    bytes: u64,
}

impl Atlas {
    pub(super) fn trim(&mut self) {
        const BYTE_BUDGET: u64 = 4 * 1024 * 1024;
        const ENTRY_BUDGET: usize = 16 * 1024;
        if self.scratch.data.capacity() as u64 > BYTE_BUDGET {
            self.scratch = GlyphMask::default();
        }
        if self.storage.bytes <= BYTE_BUDGET && self.entries.len() + self.svgs.len() <= ENTRY_BUDGET
        {
            return;
        }
        let mut count: usize = self.storage.pages.iter().map(|page| page.tile_count).sum();
        while self.storage.bytes > BYTE_BUDGET || count > ENTRY_BUDGET {
            let page = self.storage.pages.pop_front().unwrap();
            self.storage.bytes -=
                u64::from(page.texture.width()) * u64::from(page.texture.height());
            count -= page.tile_count;
            self.storage.first_page += 1;
        }
        self.svgs.retain(|_, t| t.page >= self.storage.first_page);
        self.entries
            .retain(|_, tile| tile.is_some_and(|t| t.page >= self.storage.first_page));
    }

    pub(super) fn group(&self, page: usize) -> &wgpu::BindGroup {
        &self.storage.pages[page - self.storage.first_page].group
    }

    pub(super) fn resolve(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        images: &super::images::Images,
        key: GlyphKey,
        rasterize: &mut dyn FnMut(&mut GlyphMask) -> Result<()>,
    ) -> Result<Option<Tile>> {
        if let Some(tile) = self.entries.get(&key) {
            return Ok(*tile);
        }
        rasterize(&mut self.scratch)?;
        let p = self.scratch.placement;
        if p.width == 0 || p.height == 0 {
            self.entries.insert(key, None);
            return Ok(None);
        }
        let mut tile = self.storage.upload(
            device,
            queue,
            images,
            DeviceSize {
                width: p.width,
                height: p.height,
            },
            &self.scratch.data,
        )?;
        tile.offset = [p.left, -p.top];
        self.entries.insert(key, Some(tile));
        Ok(Some(tile))
    }

    pub(super) fn svg(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        images: &super::images::Images,
        region: SvgRaster,
    ) -> Result<Tile> {
        if let Some(tile) = self.svgs.get(&region) {
            return Ok(*tile);
        }
        let tile = self
            .storage
            .upload(device, queue, images, region.size(), &region.mask()?)?;
        self.svgs.insert(region, tile);
        Ok(tile)
    }
}

impl Pages {
    fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        images: &super::images::Images,
        size: DeviceSize,
        pixels: &[u8],
    ) -> Result<Tile> {
        if size.width.max(size.height) > device.limits().max_texture_dimension_2d {
            return Err(Error::other("mask exceeds GPU texture dimensions"));
        }
        let pos = self
            .pages
            .back_mut()
            .and_then(|page| page.allocate(size.width, size.height));
        let uv = match pos {
            Some(pos) => pos,
            None => {
                let width = size.width.max(512);
                let height = size.height.max(512);
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Solor mask atlas"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::R8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                let group = images.bind(device, &texture);
                self.pages.push_back(Page {
                    texture,
                    group,
                    x: size.width,
                    y: 0,
                    row_height: size.height,
                    tile_count: 0,
                });
                self.bytes += u64::from(width) * u64::from(height);
                [0, 0]
            }
        };
        let page = self.pages.back_mut().unwrap();
        page.tile_count += 1;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                origin: wgpu::Origin3d {
                    x: uv[0],
                    y: uv[1],
                    z: 0,
                },
                ..page.texture.as_image_copy()
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size.width),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
        );
        Ok(Tile {
            page: self.first_page + self.pages.len() - 1,
            uv,
            size: [size.width, size.height],
            offset: [0, 0],
        })
    }
}
