//! Separable box filter on premultiplied pixels with fractional edge weights.
use super::{Clip, Corners, Draw, Origin, Pixels, RasterScale, Size, Surface, coverage};
use std::ops::Range;

pub(super) fn reads_backdrop(draw: &Draw) -> bool {
    match draw {
        Draw::BackdropBlur { .. } => true,
        Draw::Group { draws, .. } => draws.iter().any(reads_backdrop),
        _ => false,
    }
}

pub(super) fn footprint(
    draw: &Draw,
    scale: RasterScale,
    canvas: Clip,
) -> std::io::Result<Option<Clip>> {
    if !reads_backdrop(draw) {
        return Ok(None);
    }
    let output = super::device_bounds(draw, scale)?.intersect(canvas);
    if output.empty() {
        return Ok(None);
    }
    // A group's children sample only the isolated layer.
    let Draw::BackdropBlur { radius, .. } = draw else {
        return Ok(Some(output));
    };
    let halo = (f64::from(*radius) * scale.0).ceil() as i64;
    Ok(Some(
        Clip {
            left: output.left.saturating_sub(halo),
            right: output.right.saturating_add(halo),
            top: output.top.saturating_sub(halo),
            bottom: output.bottom.saturating_add(halo),
        }
        .intersect(canvas),
    ))
}

struct Kernel {
    whole: i64,
    fraction: f64,
}

impl Kernel {
    fn line(
        &self,
        output: Range<i64>,
        extent: i64,
        mut read: impl FnMut(i64) -> [u8; 4],
        mut write: impl FnMut(i64, [u8; 4]),
    ) {
        let mut sum = [0i64; 4];
        let mut left = output.start - self.whole;
        let mut right = output.start + self.whole;
        for i in left.max(0)..=right.min(extent - 1) {
            let pixel = read(i);
            for c in 0..4 {
                sum[c] += i64::from(pixel[c]);
            }
        }
        let end = output.end;
        for i in output {
            let mut weight = (right.min(extent - 1) - left.max(0) + 1) as f64;
            let mut ends = [0u16; 4];
            if self.fraction > 0. {
                for edge in [left - 1, right + 1] {
                    if (0..extent).contains(&edge) {
                        let p = read(edge);
                        for c in 0..4 {
                            ends[c] += u16::from(p[c]);
                        }
                        weight += self.fraction;
                    }
                }
            }
            write(
                i,
                std::array::from_fn(|c| {
                    ((sum[c] as f64 + f64::from(ends[c]) * self.fraction) / weight).round() as u8
                }),
            );
            if i + 1 == end {
                break;
            }
            let removed = if left >= 0 { read(left) } else { [0; 4] };
            left += 1;
            right += 1;
            let added = if right < extent { read(right) } else { [0; 4] };
            for c in 0..4 {
                sum[c] += i64::from(added[c]) - i64::from(removed[c]);
            }
        }
    }
}

pub(super) fn draw(
    surface: &mut Surface,
    origin: Origin,
    size: Size,
    radii: Corners<Pixels>,
    radius: f64,
    scale: RasterScale,
    (layer, output): (Clip, Clip),
) {
    let canvas = Clip::canvas(surface.size);
    let radius = radius.min(canvas.bottom.max(canvas.right) as f64);
    let kernel = Kernel {
        whole: radius.floor() as i64,
        fraction: radius.fract(),
    };
    let halo = radius.ceil() as i64;
    let rows = (output.top - halo).max(0)..(output.bottom + halo).min(canvas.bottom);
    let width = (output.right - output.left) as usize;
    let stride = surface.size.width as usize;
    let pixels = surface.rgba.as_chunks_mut::<4>().0;
    let mut horizontal = vec![[0u8; 4]; width * (rows.end - rows.start) as usize];
    for (y, row) in rows.clone().zip(horizontal.chunks_exact_mut(width)) {
        if !(layer.top..layer.bottom).contains(&y) {
            continue;
        }
        kernel.line(
            output.left..output.right,
            canvas.right,
            |x| {
                if (layer.left..layer.right).contains(&x) {
                    pixels[y as usize * stride + x as usize]
                } else {
                    [0; 4]
                }
            },
            |x, pixel| row[(x - output.left) as usize] = pixel,
        );
    }
    let left = f64::from(origin.x) * scale.0;
    let top = f64::from(origin.y) * scale.0;
    let w = f64::from(size.width) * scale.0;
    let h = f64::from(size.height) * scale.0;
    let radii = radii.map(|r| f64::from(r) * scale.0);
    for x in output.left..output.right {
        let column = (x - output.left) as usize;
        kernel.line(
            output.top..output.bottom,
            canvas.bottom,
            |y| horizontal[(y - rows.start) as usize * width + column],
            |y, blurred| {
                let dy = y as f64 - top;
                let row_height = ((dy + 1.).min(h) - dy.max(0.)).clamp(0., 1.);
                let mask = coverage(x as f64 - left, dy, w, h, row_height, radii);
                let dest = &mut pixels[y as usize * stride + x as usize];
                if mask == 255 {
                    *dest = blurred;
                } else if mask != 0 {
                    for (old, new) in dest.iter_mut().zip(blurred) {
                        *old = ((u16::from(*old) * u16::from(255 - mask)
                            + u16::from(new) * u16::from(mask)
                            + 127)
                            / 255) as u8;
                    }
                }
            },
        );
    }
}
