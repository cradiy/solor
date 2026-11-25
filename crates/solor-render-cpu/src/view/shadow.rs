//! Scanline convolution of rounded boxes with a compact three-box blur kernel.
use crate::pixels::{Clip, Surface, blend};
use solor::raster::{BoxShadowGeometry, RoundedBox};

type Span = (f64, f64);
const EMPTY: Span = (0., 0.);

fn overlap((a, b): Span, (c, d): Span) -> f64 {
    (b.min(d) - a.max(c)).max(0.)
}

pub(super) fn draw(geometry: &BoxShadowGeometry, surface: &mut Surface, clip: Clip) {
    for y in clip.top..clip.bottom {
        let row = y as f64;
        let mut cuts = [
            row,
            row + 0.25,
            row + 0.5,
            row + 0.75,
            row + 1.,
            geometry.base.bounds[1],
            geometry.base.bounds[3],
            geometry.caster.bounds[1],
            geometry.caster.bounds[3],
        ];
        for sy in &mut cuts[5..] {
            *sy = sy.clamp(row, row + 1.);
        }
        cuts.sort_unstable_by(f64::total_cmp);
        let mut bands = [Band::default(); 8];
        let mut count = 0;
        let mut solid_left = f64::NEG_INFINITY;
        let mut solid_right = f64::INFINITY;
        for pair in cuts.windows(2).filter(|p| p[0] < p[1]) {
            let sy = (pair[0] + pair[1]) * 0.5;
            let base = geometry.base.span(sy).unwrap_or(EMPTY);
            let caster = geometry.caster.span(sy).unwrap_or(EMPTY);
            if count > 0 && bands[count - 1].base == base && bands[count - 1].caster == caster {
                bands[count - 1].bottom = pair[1];
            } else {
                bands[count] = Band {
                    top: pair[0],
                    bottom: pair[1],
                    base,
                    caster,
                    soft: None,
                };
                count += 1;
            }
            solid_left = solid_left.max(base.0);
            solid_right = solid_right.min(base.1);
        }
        let bands = &mut bands[..count];
        if geometry.blur > 0. {
            for band in &mut *bands {
                band.soft = Some(BlurRow::new(
                    &geometry.caster,
                    band.top,
                    band.bottom,
                    geometry.blur,
                ));
            }
        }
        let stride = surface.size.width as usize;
        let row_start = 4 * (y as usize * stride + clip.left as usize);
        let row_end = row_start + 4 * (clip.right - clip.left) as usize;
        let pixels = surface.rgba[row_start..row_end].as_chunks_mut::<4>().0;
        let mut run = |start: i64, end: i64| {
            for x in start..end {
                let pixel = (x as f64, x as f64 + 1.);
                let coverage: f64 = bands
                    .iter()
                    .map(|b| b.coverage(pixel, geometry.blur, geometry.inset))
                    .sum();
                let alpha = (coverage * 255.).round() as u8;
                if alpha > 0 {
                    blend(&mut pixels[(x - clip.left) as usize], geometry.color, alpha);
                }
            }
        };
        // A rounded box is convex. Inside the common chord, even the full
        // blur footprint is inside the inset shadow's transparent hole.
        if geometry.inset {
            let margin = 1.5 * geometry.blur;
            let upper = geometry.caster.span(row - margin).unwrap_or(EMPTY);
            let lower = geometry.caster.span(row + 1. + margin).unwrap_or(EMPTY);
            solid_left = upper.0.max(lower.0) + margin;
            solid_right = upper.1.min(lower.1) - margin;
        }
        let left = solid_left.ceil() as i64;
        let right = solid_right.floor() as i64;
        if left < right {
            run(clip.left, clip.right.min(left));
            run(clip.left.max(right), clip.right);
        } else {
            run(clip.left, clip.right);
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Band {
    top: f64,
    bottom: f64,
    base: Span,
    caster: Span,
    soft: Option<BlurRow>,
}

impl Band {
    fn coverage(&self, pixel: Span, blur: f64, inset: bool) -> f64 {
        let ink = |left: f64, right: f64| {
            if left >= right {
                0.
            } else if let Some(row) = &self.soft {
                row.coverage((left, right), blur)
            } else {
                overlap((left, right), self.caster)
            }
        };
        let start = pixel.0.max(self.base.0);
        let end = pixel.1.min(self.base.1);
        let area = if inset {
            (end - start).max(0.) - ink(start, end)
        } else if start >= end {
            ink(pixel.0, pixel.1)
        } else {
            ink(pixel.0, start) + ink(end, pixel.1)
        };
        (self.bottom - self.top) * area
    }
}

// The sum of three unit-width box filters has standard deviation 0.5 and support [-1.5,1.5].
fn cdf(x: f64) -> f64 {
    let t = x.abs();
    let tail = if t >= 1.5 {
        0.
    } else if t >= 0.5 {
        (1.5 - t).powi(3) / 6.
    } else {
        0.5 - 0.75 * t + t.powi(3) / 3.
    };
    if x < 0. { tail } else { 1. - tail }
}

// Integral of the kernel CDF with respect to physical distance, for fractional pixel coverage.
fn integrated(x: f64, blur: f64) -> f64 {
    let t = x.abs() / blur;
    if t >= 1.5 {
        return x.max(0.);
    }
    let tail = ((1.5 - t).powi(4) - 3. * (0.5 - t).max(0.).powi(4)) / 24.;
    x.max(0.) + blur * tail
}

// Integrate an edge over the visible part of a pixel. For broad kernels
// the midpoint also avoids subtracting nearly equal antiderivatives.
fn edge_integral(d: f64, length: f64, blur: f64) -> f64 {
    if d >= 1.5 * blur {
        length
    } else if d + length <= -1.5 * blur {
        0.
    } else if blur >= 4. * length {
        length * cdf((d + 0.5 * length) / blur)
    } else {
        integrated(d + length, blur) - integrated(d, blur)
    }
}

#[derive(Clone, Copy)]
struct BlurRow {
    spans: [(Span, f64); 12],
    count: usize,
}

impl BlurRow {
    fn new(shape: &RoundedBox, y0: f64, y1: f64, blur: f64) -> Self {
        let mut row = Self {
            spans: [(EMPTY, 0.); 12],
            count: 0,
        };
        let top = shape.bounds[1].max(y0 - 1.5 * blur);
        let bottom = shape.bounds[3].min(y1 + 1.5 * blur);
        if top >= bottom {
            return row;
        }
        let steps = if shape.corners == [[0.; 2]; 4] {
            1
        } else {
            row.spans.len()
        };
        let step = (bottom - top) / steps as f64;
        let cumulative = |at: f64| edge_integral(at - y1, y1 - y0, blur) / (y1 - y0);
        let mut previous = cumulative(top);
        for index in 0..steps {
            let sy = top + (index as f64 + 0.5) * step;
            let next = cumulative(top + (index + 1) as f64 * step);
            if let Some(span) = shape.span(sy) {
                if row.count > 0 && row.spans[row.count - 1].0 == span {
                    row.spans[row.count - 1].1 += next - previous;
                } else {
                    row.spans[row.count] = (span, next - previous);
                    row.count += 1;
                }
            }
            previous = next;
        }
        row
    }

    fn coverage(&self, (x, end): Span, blur: f64) -> f64 {
        self.spans[..self.count]
            .iter()
            .map(|&((l, r), weight)| {
                if end <= l - 1.5 * blur || x >= r + 1.5 * blur {
                    0.
                } else {
                    weight
                        * (edge_integral(x - l, end - x, blur)
                            - edge_integral(x - r, end - x, blur))
                }
            })
            .sum()
    }
}
