use crate::pixels::{Surface, blend, blend_solid_span};
use resvg::tiny_skia::{Pixmap, Transform};
use solor::{Rgba, SvgAsset, raster::Clip};
use std::{
    collections::VecDeque,
    io::{Error, Result},
    rc::{Rc, Weak},
};

fn tint_row(row: &mut [[u8; 4]], mask: &[u8], ink: Rgba) {
    // Solid interiors and empty margins cover many consecutive pixels.
    let (blocks, tail) = row.as_chunks_mut::<8>();
    let (masks, tail_mask) = mask.as_chunks::<8>();
    for (pixels, coverage) in blocks.iter_mut().zip(masks) {
        match u64::from_ne_bytes(*coverage) {
            0 => {}
            u64::MAX => blend_solid_span(pixels, ink, 255),
            _ => tint_pixels(pixels, coverage, ink),
        }
    }
    tint_pixels(tail, tail_mask, ink);
}

fn tint_pixels(row: &mut [[u8; 4]], mask: &[u8], ink: Rgba) {
    for (dst, &coverage) in row.iter_mut().zip(mask) {
        match coverage {
            0 => {}
            255 if ink.alpha == 255 => *dst = [ink.red, ink.green, ink.blue, 255],
            _ => blend(dst, ink, coverage),
        }
    }
}

#[derive(PartialEq)]
struct Region {
    viewport_width: u64,
    viewport_height: u64,
    left: u64,
    top: u64,
    width: u32,
    height: u32,
}

impl Region {
    fn rasterize(&self, asset: &SvgAsset, monochrome: bool) -> Result<Vec<u8>> {
        let mut pixmap = Pixmap::new(self.width, self.height)
            .ok_or_else(|| Error::other("invalid SVG raster extent"))?;
        let source = asset.tree().size();
        let transform = Transform::from_row(
            self.viewport_width as f32 / source.width(),
            0.0,
            0.0,
            self.viewport_height as f32 / source.height(),
            -(self.left as f32),
            -(self.top as f32),
        );
        resvg::render(asset.tree(), transform, &mut pixmap.as_mut());
        Ok(if monochrome {
            pixmap
                .data()
                .as_chunks::<4>()
                .0
                .iter()
                .map(|pixel| pixel[3])
                .collect()
        } else {
            pixmap.take()
        })
    }
}

struct Entry {
    asset: Weak<resvg::usvg::Tree>,
    region: Region,
    monochrome: bool,
    pixels: Rc<Vec<u8>>,
}

#[derive(Default)]
pub(super) struct SvgCache {
    entries: VecDeque<Entry>,
    bytes: usize,
}

impl SvgCache {
    fn resolve(
        &mut self,
        asset: &SvgAsset,
        region: Region,
        monochrome: bool,
    ) -> Result<Rc<Vec<u8>>> {
        let tree = Rc::downgrade(asset.tree());
        if let Some(entry) = self
            .entries
            .iter()
            .find(|e| e.asset.ptr_eq(&tree) && e.region == region && e.monochrome == monochrome)
        {
            return Ok(Rc::clone(&entry.pixels));
        }
        let pixels = Rc::new(region.rasterize(asset, monochrome)?);
        const BUDGET: usize = 8 * 1024 * 1024;
        let bytes = pixels.capacity();
        if bytes <= BUDGET {
            while self.entries.len() >= 128 || self.bytes + bytes > BUDGET {
                let oldest = self.entries.pop_front().expect("nonempty SVG cache");
                self.bytes -= oldest.pixels.capacity();
            }
            self.entries.push_back(Entry {
                asset: tree,
                region,
                monochrome,
                pixels: Rc::clone(&pixels),
            });
            self.bytes += bytes;
        }
        Ok(pixels)
    }

    pub(super) fn draw(
        &mut self,
        surface: &mut Surface,
        asset: &SvgAsset,
        tint: Option<Rgba>,
        viewport: Clip,
        visible: Clip,
        damage: Clip,
    ) -> Result<()> {
        let region = Region {
            viewport_width: viewport.right.abs_diff(viewport.left),
            viewport_height: viewport.bottom.abs_diff(viewport.top),
            left: visible.left.abs_diff(viewport.left),
            top: visible.top.abs_diff(viewport.top),
            width: (visible.right - visible.left) as u32,
            height: (visible.bottom - visible.top) as u32,
        };
        let stride = region.width as usize;
        let pixels = self.resolve(asset, region, tint.is_some())?;
        let width = (damage.right - damage.left) as usize;
        for y in damage.top..damage.bottom {
            let dest = (y as usize * surface.size.width as usize + damage.left as usize) * 4;
            let row = surface.rgba[dest..dest + width * 4].as_chunks_mut::<4>().0;
            let start = (y - visible.top) as usize * stride + (damage.left - visible.left) as usize;
            if let Some(ink) = tint {
                tint_row(row, &pixels[start..start + width], ink);
            } else {
                let src = pixels[start * 4..(start + width) * 4].as_chunks::<4>().0;
                for (dst, src) in row.iter_mut().zip(src) {
                    crate::pixels::blend_premultiplied(dst, *src);
                }
            }
        }
        Ok(())
    }
}
