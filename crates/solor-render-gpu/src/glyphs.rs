use solor::raster::glyphs::{GlyphKey, GlyphMask};
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
    glyph_count: usize,
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
    pages: VecDeque<Page>,
    first_page: usize,
    bytes: u64,
    scratch: GlyphMask,
}

impl Atlas {
    pub(super) fn trim(&mut self) {
        const BYTE_BUDGET: u64 = 4 * 1024 * 1024;
        const ENTRY_BUDGET: usize = 16 * 1024;
        if self.scratch.data.capacity() as u64 > BYTE_BUDGET {
            self.scratch = GlyphMask::default();
        }
        if self.bytes <= BYTE_BUDGET && self.entries.len() <= ENTRY_BUDGET {
            return;
        }
        let mut count: usize = self.pages.iter().map(|page| page.glyph_count).sum();
        while self.bytes > BYTE_BUDGET || count > ENTRY_BUDGET {
            let page = self.pages.pop_front().unwrap();
            self.bytes -= u64::from(page.texture.width()) * u64::from(page.texture.height());
            count -= page.glyph_count;
            self.first_page += 1;
        }
        self.entries
            .retain(|_, tile| tile.is_some_and(|t| t.page >= self.first_page));
    }

    pub(super) fn group(&self, page: usize) -> &wgpu::BindGroup {
        &self.pages[page - self.first_page].group
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
        if p.width.max(p.height) > device.limits().max_texture_dimension_2d {
            return Err(Error::other("glyph exceeds GPU texture dimensions"));
        }
        let pos = self
            .pages
            .back_mut()
            .and_then(|page| page.allocate(p.width, p.height));
        let uv = match pos {
            Some(pos) => pos,
            None => {
                let width = p.width.max(512);
                let height = p.height.max(512);
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Solor glyph atlas"),
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
                    x: p.width,
                    y: 0,
                    row_height: p.height,
                    glyph_count: 0,
                });
                self.bytes += u64::from(width) * u64::from(height);
                [0, 0]
            }
        };
        let page = self.pages.back_mut().unwrap();
        page.glyph_count += 1;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                origin: wgpu::Origin3d {
                    x: uv[0],
                    y: uv[1],
                    z: 0,
                },
                ..page.texture.as_image_copy()
            },
            &self.scratch.data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(p.width),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: p.width,
                height: p.height,
                depth_or_array_layers: 1,
            },
        );
        let tile = Tile {
            page: self.first_page + self.pages.len() - 1,
            uv,
            size: [p.width, p.height],
            offset: [p.left, -p.top],
        };
        self.entries.insert(key, Some(tile));
        Ok(Some(tile))
    }
}
