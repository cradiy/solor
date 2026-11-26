use crate::{pixels::Surface, rounded_clip::CornerMask};
use solor::raster::svg::SvgRaster;
use solor::{SvgAsset, raster::Clip};
use std::{collections::VecDeque, io::Result, rc::Rc};

pub(super) struct Viewport {
    pub bounds: Clip,
    pub visible: Clip,
    pub corners: Option<CornerMask>,
}

fn multiply_alpha(value: u8, alpha: u8) -> u8 {
    ((u16::from(value) * u16::from(alpha) + 127) / 255) as u8
}

struct Entry {
    region: SvgRaster,
    monochrome: bool,
    pixels: Rc<Vec<u8>>,
}

#[derive(Default)]
pub(super) struct SvgCache {
    entries: VecDeque<Entry>,
    bytes: usize,
}

impl SvgCache {
    fn resolve(&mut self, region: SvgRaster, monochrome: bool) -> Result<Rc<Vec<u8>>> {
        if let Some(entry) = self
            .entries
            .iter()
            .find(|e| e.region == region && e.monochrome == monochrome)
        {
            return Ok(Rc::clone(&entry.pixels));
        }
        let pixels = Rc::new(if monochrome {
            region.mask()?
        } else {
            region.rgba()?
        });
        const BUDGET: usize = 8 * 1024 * 1024;
        let bytes = pixels.capacity();
        if bytes <= BUDGET {
            while self.entries.len() >= 128 || self.bytes + bytes > BUDGET {
                let oldest = self.entries.pop_front().expect("nonempty SVG cache");
                self.bytes -= oldest.pixels.capacity();
            }
            self.entries.push_back(Entry {
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
        mut tint: Option<impl crate::mask::MaskPainter>,
        viewport: Viewport,
        damage: Clip,
    ) -> Result<()> {
        let Viewport {
            bounds: viewport,
            visible,
            corners,
        } = viewport;
        let region = SvgRaster::new(asset, viewport, visible)?;
        let stride = region.size().width as usize;
        let pixels = self.resolve(region, tint.is_some())?;
        let width = (damage.right - damage.left) as usize;
        let mut masked = Vec::new();
        for y in damage.top..damage.bottom {
            let dest = (y as usize * surface.size.width as usize + damage.left as usize) * 4;
            let row = surface.rgba[dest..dest + width * 4].as_chunks_mut::<4>().0;
            let start = (y - visible.top) as usize * stride + (damage.left - visible.left) as usize;
            let full = damage.left..damage.right;
            let inner = corners
                .as_ref()
                .map_or(full.clone(), |c| c.interior(y, full));
            for (span, mask) in [
                (damage.left..inner.start, corners.as_ref()),
                (inner.clone(), None),
                (inner.end..damage.right, corners.as_ref()),
            ] {
                if span.is_empty() {
                    continue;
                }
                let from = (span.start - damage.left) as usize;
                let to = (span.end - damage.left) as usize;
                let dest = &mut row[from..to];
                if let Some(paint) = &mut tint {
                    let src = &pixels[start + from..start + to];
                    let alpha = if let Some(mask) = mask {
                        masked.clear();
                        masked.extend(
                            src.iter()
                                .zip(span.clone())
                                .map(|(&a, x)| multiply_alpha(a, mask.coverage(x, y))),
                        );
                        &masked[..]
                    } else {
                        src
                    };
                    paint.paint_row(dest, alpha, span.start, y)?;
                } else {
                    let src = pixels[(start + from) * 4..(start + to) * 4]
                        .as_chunks::<4>()
                        .0;
                    for ((dst, &src), x) in dest.iter_mut().zip(src).zip(span) {
                        let src = if let Some(mask) = mask {
                            let alpha = mask.coverage(x, y);
                            src.map(|v| multiply_alpha(v, alpha))
                        } else {
                            src
                        };
                        crate::pixels::blend_premultiplied(dst, src);
                    }
                }
            }
        }
        Ok(())
    }
}
