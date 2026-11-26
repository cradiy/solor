use crate::pixels::{Clip, RasterScale, Surface};
use solor::{RoundedClip, RoundedClips, style::Corners};
use std::ops::Range;

pub(super) struct CornerMask {
    left: f64,
    top: f64,
    width: f64,
    height: f64,
    radii: Corners<f64>,
}

impl CornerMask {
    pub(super) fn new(clip: &RoundedClip, scale: RasterScale) -> Self {
        let device = |v| f64::from(v) * scale.0;
        Self {
            left: device(clip.origin.x),
            top: device(clip.origin.y),
            width: device(clip.size.width),
            height: device(clip.size.height),
            radii: clip.radii.map(device),
        }
    }

    pub(super) fn interior(&self, y: i64, span: Range<i64>) -> Range<i64> {
        let top = y as f64 - self.top;
        let bottom = self.height - top - 1.;
        if top < 0. || bottom < 0. {
            return span.start..span.start;
        }
        let inset = |a: f64, b: f64| {
            let upper = if top < a { a } else { 0. };
            upper.max(if bottom < b { b } else { 0. })
        };
        let r = self.radii;
        let left = (self.left + inset(r.top_left, r.bottom_left)).ceil() as i64;
        let right = (self.left + self.width - inset(r.top_right, r.bottom_right)).floor() as i64;
        let start = left.clamp(span.start, span.end);
        start..right.clamp(start, span.end)
    }

    pub(super) fn covers(&self, bounds: Clip) -> bool {
        let span = bounds.left..bounds.right;
        self.interior(bounds.top, span.clone()) == span
            && self.interior(bounds.bottom - 1, span.clone()) == span
    }

    pub(super) fn coverage(&self, x: i64, y: i64) -> u8 {
        let dy = y as f64 - self.top;
        let row_height = ((dy + 1.).min(self.height) - dy.max(0.)).clamp(0., 1.);
        crate::view::coverage(
            x as f64 - self.left,
            dy,
            self.width,
            self.height,
            row_height,
            self.radii,
        )
    }
}

// Preserve destination pixels touched by a partially covered draw, including backdrop filters.
#[derive(Default)]
pub(super) struct SavedEdges {
    spans: Vec<Range<usize>>,
    pixels: Vec<([u8; 4], u8)>,
}

impl SavedEdges {
    pub fn capture(
        surface: &Surface,
        bounds: Clip,
        scale: RasterScale,
        clips: &RoundedClips,
    ) -> Self {
        let span = bounds.left..bounds.right;
        let masks: Vec<_> = clips
            .iter()
            .map(|clip| CornerMask::new(clip, scale))
            .filter(|mask| !mask.covers(bounds))
            .collect();
        let mut saved = Self::default();
        if !masks.is_empty() {
            let pixels = surface.rgba.as_chunks::<4>().0;
            for y in bounds.top..bounds.bottom {
                let mut inside = span.clone();
                for mask in &masks {
                    inside = mask.interior(y, inside);
                }
                let row = y as usize * surface.size.width as usize;
                for x in (span.start..inside.start).chain(inside.end..span.end) {
                    let coverage = masks.iter().map(|m| m.coverage(x, y)).min().unwrap_or(255);
                    if coverage < 255 {
                        let index = row + x as usize;
                        if let Some(span) = saved.spans.last_mut()
                            && span.end == index
                        {
                            span.end += 1;
                        } else {
                            saved.spans.push(index..index + 1);
                        }
                        saved.pixels.push((pixels[index], coverage));
                    }
                }
            }
        }
        saved
    }

    pub fn apply(self, surface: &mut Surface) {
        let pixels = surface.rgba.as_chunks_mut::<4>().0;
        let mut saved = self.pixels.as_slice();
        for span in self.spans {
            let (edge, rest) = saved.split_at(span.len());
            saved = rest;
            for (pixel, &(before, coverage)) in pixels[span].iter_mut().zip(edge) {
                if coverage == 0 {
                    *pixel = before;
                    continue;
                }
                let t = u16::from(coverage);
                for (new, old) in pixel.iter_mut().zip(before) {
                    *new = ((u16::from(*new) * t + u16::from(old) * (255 - t) + 127) / 255) as u8;
                }
            }
        }
    }
}
