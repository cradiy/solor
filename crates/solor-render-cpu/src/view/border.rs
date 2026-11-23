//! Rasterize the border ring as disjoint spans so translucent joins composite once.
use super::{Brush, SampleGradient, brush};
use crate::pixels::{Surface, blend_premultiplied, blend_solid_span};
use solor::{
    Fill, ImageCache,
    layout::{Origin, Size},
    raster::{Clip, RasterScale},
    style::BorderPaint,
};
use std::io::{Error, Result};

mod pattern;
use pattern::Pattern;

use super::rounded_box::RoundedBox;

#[derive(Clone, Copy)]
struct Event {
    x: i64,
    side: usize,
    delta: f64,
}

fn span_events(events: &mut Vec<Event>, side: usize, from: f64, to: f64, weight: f64, clip: Clip) {
    let from = from.max(clip.left as f64);
    let to = to.min(clip.right as f64);
    if from >= to {
        return;
    }
    for (edge, change) in [(from, weight), (to, -weight)] {
        let x = edge.floor() as i64;
        let fraction = edge - x as f64;
        events.push(Event {
            x,
            side,
            delta: change * (1.0 - fraction),
        });
        if fraction > 0.0 {
            events.push(Event {
                x: x + 1,
                side,
                delta: change * fraction,
            });
        }
    }
}

struct Ring {
    outer: RoundedBox,
    inner: RoundedBox,
    widths: [f64; 4],
    patterns: [Option<Pattern>; 4],
}

impl Ring {
    fn row_events(&self, y: i64, active: [bool; 4], clip: Clip, events: &mut Vec<Event>) {
        let row = y as f64;
        let [left, top, right, bottom] = self.outer.bounds;
        let [t, r, b, l] = self.widths;
        // Split at horizontal edges as well as quarter pixels to retain thin fractional rules.
        let mut cuts = [
            row,
            row + 0.25,
            row + 0.5,
            row + 0.75,
            row + 1.0,
            top,
            bottom,
            top + t,
            bottom - b,
        ];
        cuts[5..]
            .iter_mut()
            .for_each(|v| *v = v.clamp(row, row + 1.0));
        cuts.sort_unstable_by(f64::total_cmp);
        for pair in cuts.windows(2) {
            let weight = pair[1] - pair[0];
            if weight == 0.0 {
                continue;
            }
            let sy = (pair[0] + pair[1]) * 0.5;
            let Some((start, end)) = self.outer.span(sy) else {
                continue;
            };
            let (hole_l, hole_r) = self.inner.span(sy).unwrap_or((end, end));
            let ratio = |distance: f64, width: f64| {
                if width > 0.0 {
                    distance / width
                } else {
                    f64::INFINITY
                }
            };
            let upper = ratio(sy - top, t);
            let lower = ratio(bottom - sy, b);
            let middle = if upper <= lower { 0 } else { 2 };
            let depth = upper.min(lower);
            let split = if l + r > 0.0 {
                left + (right - left) * l / (l + r)
            } else {
                left
            };
            let x1 = if l > 0.0 {
                (left + l * depth).min(split)
            } else {
                left
            };
            let x2 = if r > 0.0 {
                (right - r * depth).max(split)
            } else {
                right
            };
            for (side, a, z) in [
                (3, start, x1.min(end)),
                (middle, x1.max(start), x2.min(end)),
                (1, x2.max(start), end),
            ] {
                if active[side] {
                    for (from, to) in [(a, z.min(hole_l)), (a.max(hole_r), z)] {
                        if let Some(pattern) = &self.patterns[side] {
                            pattern.spans(
                                from.max(clip.left as f64),
                                to.min(clip.right as f64),
                                sy,
                                |start, end, coverage| {
                                    span_events(events, side, start, end, weight * coverage, clip)
                                },
                            );
                        } else {
                            span_events(events, side, from, to, weight, clip);
                        }
                    }
                }
            }
        }
    }
}

pub(super) fn edge_clips(
    origin: Origin,
    size: Size,
    paint: &BorderPaint,
    scale: RasterScale,
    clip: Clip,
) -> [Clip; 4] {
    let [x, y] = [origin.x, origin.y].map(|v| f64::from(v) * scale.0);
    let [w, h] = [size.width, size.height].map(f64::from);
    let r = paint.radii.fit_to(size);
    let b = paint.widths;
    let top = b.top.max(r.top_left).max(r.top_right);
    let right = b.right.max(r.top_right).max(r.bottom_right);
    let bottom = b.bottom.max(r.bottom_right).max(r.bottom_left);
    let left = b.left.max(r.bottom_left).max(r.top_left);
    [
        Clip {
            bottom: clip
                .bottom
                .min((y + f64::from(top) * scale.0).ceil() as i64),
            ..clip
        },
        Clip {
            left: clip
                .left
                .max((x + (w - f64::from(right)) * scale.0).floor() as i64),
            ..clip
        },
        Clip {
            top: clip
                .top
                .max((y + (h - f64::from(bottom)) * scale.0).floor() as i64),
            ..clip
        },
        Clip {
            right: clip
                .right
                .min((x + f64::from(left) * scale.0).ceil() as i64),
            ..clip
        },
    ]
}

impl Surface {
    pub(super) fn draw_border(
        &mut self,
        origin: Origin,
        size: Size,
        paint: &BorderPaint,
        scale: RasterScale,
        clip: Clip,
        images: &mut ImageCache,
    ) -> Result<()> {
        let left = f64::from(origin.x) * scale.0;
        let top = f64::from(origin.y) * scale.0;
        let right = left + f64::from(size.width) * scale.0;
        let bottom = top + f64::from(size.height) * scale.0;
        let w = &paint.widths;
        let [t, r, b, l] = [w.top, w.right, w.bottom, w.left].map(|v| f64::from(v) * scale.0);
        let radii = paint.radii.fit_to(size).map(|v| f64::from(v) * scale.0);
        let radii = [
            radii.top_left,
            radii.top_right,
            radii.bottom_right,
            radii.bottom_left,
        ];
        let outer = RoundedBox {
            bounds: [left, top, right, bottom],
            corners: radii.map(|v| [v, v]),
        };
        let inner = RoundedBox {
            bounds: [left + l, top + t, right - r, bottom - b],
            corners: std::array::from_fn(|i| {
                let (x, y) = [(l, t), (r, t), (r, b), (l, b)][i];
                [(radii[i] - x).max(0.0), (radii[i] - y).max(0.0)]
            }),
        };

        let s = paint.styles;
        let styles = [s.top, s.right, s.bottom, s.left];
        let fills = paint.visible_fills();
        let clips = edge_clips(origin, size, paint, scale, clip);
        // A straight, single-sided solid rule uses the ordinary rectangular fill rasterizer.
        if radii == [0.0; 4]
            && [t, r, b, l]
                .into_iter()
                .filter(|width| *width > 0.0)
                .count()
                == 1
            && let Some(i) = fills.iter().position(|f| f.is_some())
            && styles[i] == solor::style::BorderStyle::Solid
            && let Some(fill @ Fill::Solid(_)) = fills[i]
        {
            let mut at = origin;
            let mut extent = size;
            match i {
                0 => extent.height = w.top.min(size.height),
                1 => {
                    extent.width = w.right.min(size.width);
                    at.x += size.width - extent.width;
                }
                2 => {
                    extent.height = w.bottom.min(size.height);
                    at.y += size.height - extent.height;
                }
                _ => extent.width = w.left.min(size.width),
            }
            if !clips[i].empty() {
                self.fill_box(at, extent, fill, Default::default(), scale, clips[i])?;
            }
            return Ok(());
        }
        let mut resolved = [const { None }; 4];
        for (i, fill) in fills.into_iter().enumerate() {
            if !clips[i].empty()
                && let Some(fill) = fill
            {
                resolved[i] = Some(images.resolve(fill).map_err(Error::other)?);
            }
        }
        let mut brushes = [const { None }; 4];
        for (i, fill) in resolved.iter().enumerate() {
            if let Some(fill) = fill {
                brushes[i] = brush(fill, origin, size, scale, clips[i])?;
            }
        }
        let active = std::array::from_fn(|i| brushes[i].is_some());
        let Some(rows) = clips
            .into_iter()
            .zip(active)
            .filter_map(|(clip, enabled)| enabled.then_some(clip))
            .reduce(Clip::union)
        else {
            return Ok(());
        };
        let mut events = Vec::with_capacity(64);
        let widths = [t, r, b, l];
        let patterns =
            std::array::from_fn(|i| Pattern::new(i, styles[i], outer.bounds, widths, radii));
        let ring = Ring {
            patterns,
            outer,
            inner,
            widths,
        };
        for y in rows.top..rows.bottom {
            events.clear();
            ring.row_events(y, active, clip, &mut events);
            events.sort_unstable_by_key(|e| e.x);
            let mut weights = [0.0_f64; 4];
            let mut from = clip.left;
            for event in &events {
                if event.x > from {
                    let cover = weights.map(|v| (v.clamp(0.0, 1.0) * 65535.0).round() as u32);
                    let active = cover.iter().position(|&v| v != 0);
                    if let Some(side) = active {
                        let start = (y as usize * self.size.width as usize + from as usize) * 4;
                        let pixels = self.rgba[start..start + (event.x - from) as usize * 4]
                            .as_chunks_mut::<4>()
                            .0;
                        paint_span(
                            pixels,
                            &mut brushes,
                            cover,
                            side,
                            (from, y),
                            [left, top],
                            scale,
                        )?;
                    }
                }
                weights[event.side] += event.delta;
                from = event.x;
            }
        }
        Ok(())
    }
}

fn paint_span<F: SampleGradient>(
    pixels: &mut [[u8; 4]],
    brushes: &mut [Option<Brush<'_, F>>],
    cover: [u32; 4],
    first: usize,
    (from, y): (i64, i64),
    origin: [f64; 2],
    scale: RasterScale,
) -> Result<()> {
    if cover[first + 1..].iter().all(|v| *v == 0) {
        let brush = brushes[first].as_mut().unwrap();
        let coverage = ((cover[first] + 128) / 257) as u8;
        if let Brush::Solid(c) = brush {
            blend_solid_span(pixels, *c, coverage);
        } else {
            for (x, pixel) in (from..).zip(pixels) {
                brush.blend_at(pixel, x, y, origin, scale, coverage)?;
            }
        }
        return Ok(());
    }
    for (x, pixel) in (from..).zip(pixels) {
        let mut channels = [0u32; 4];
        for (side, &weight) in cover.iter().enumerate().filter(|(_, v)| **v > 0) {
            let mut src = [0; 4];
            brushes[side]
                .as_mut()
                .unwrap()
                .blend_at(&mut src, x, y, origin, scale, 255)?;
            for (sum, c) in channels.iter_mut().zip(src) {
                *sum += weight * u32::from(c);
            }
        }
        blend_premultiplied(
            pixel,
            channels.map(|v| ((v + 32767) / 65535).min(255) as u8),
        );
    }
    Ok(())
}
