use anyhow::{Result, ensure};
use solor::{
    ContentClip, Draw, Fill, FillRule, Ink, Rgba,
    layout::Origin,
    raster::{Clip, DeviceSize, RasterScale},
};
use std::rc::Rc;

enum Kind {
    Fill(FillRule),
    Stroke { width: f32, closed: bool },
}

pub(super) struct Path<'a> {
    origin: Origin,
    contours: &'a [Rc<[Origin]>],
    clip: Option<&'a ContentClip>,
    color: Rgba,
    kind: Kind,
}

impl<'a> Path<'a> {
    pub(super) fn from_draw(draw: &'a Draw) -> Result<Option<Self>, &'static str> {
        let (origin, contours, clip, color, kind) = match draw {
            Draw::Polygon {
                origin,
                contours,
                clip,
                fill,
                rule,
            } => {
                let Fill::Solid(color) = fill else {
                    return Err("polygon fill requires a solid color");
                };
                (
                    *origin,
                    contours.as_slice(),
                    clip.as_deref(),
                    *color,
                    Kind::Fill(*rule),
                )
            }
            Draw::Polyline {
                origin,
                contours,
                clip,
                ink,
                width,
                closed,
            } => {
                let Ink::Solid(color) = ink else {
                    return Err("polyline ink requires a solid color");
                };
                (
                    *origin,
                    contours.as_slice(),
                    clip.as_deref(),
                    *color,
                    Kind::Stroke {
                        width: width.get(),
                        closed: *closed,
                    },
                )
            }
            _ => return Ok(None),
        };
        Ok(Some(Self {
            origin,
            contours,
            clip,
            color,
            kind,
        }))
    }

    pub(super) fn record(
        &self,
        size: DeviceSize,
        scale: RasterScale,
        quads: &mut Vec<[[f32; 4]; 6]>,
        edges: &mut Vec<[[f32; 4]; 2]>,
    ) -> Result<()> {
        let mut clip = Clip::canvas(size);
        if let Some(c) = self.clip {
            if let Some(x) = &c.x {
                clip.left = clip.left.max(scale.edge(x.start.into()));
                clip.right = clip.right.min(scale.edge(x.end.into()));
            }
            if let Some(y) = &c.y {
                clip.top = clip.top.max(scale.edge(y.start.into()));
                clip.bottom = clip.bottom.min(scale.edge(y.end.into()));
            }
        }
        if clip.empty() || self.color.alpha == 0 {
            return Ok(());
        }
        let (mode, radius, closed) = match self.kind {
            Kind::Fill(FillRule::EvenOdd) => (0., 0., true),
            Kind::Fill(FillRule::NonZero) => (1., 0., true),
            Kind::Stroke { width, closed } => (2., f64::from(width) * scale.0 * 0.5, closed),
        };
        let reach = if mode == 2. { radius + 0.5 } else { 0. };
        // Bin by device x so graph pixels scan only nearby edges. Limit duplication for long edges.
        let vertices: usize = self.contours.iter().map(|c| c.len()).sum();
        let width = (clip.right - clip.left) as usize;
        let strip_count = width.div_ceil(16).min((65536 / vertices.max(1)).max(1));
        let strip_width = width.div_ceil(strip_count) as i64;
        let mut strips: Vec<Strip> = (0..strip_count).map(|_| Strip::default()).collect();
        let device = |p: Origin| {
            [
                (f64::from(self.origin.x) + f64::from(p.x)) * scale.0,
                (f64::from(self.origin.y) + f64::from(p.y)) * scale.0,
            ]
        };
        for contour in self.contours {
            let Some(first) = contour.first() else {
                continue;
            };
            let mut a = device(*first);
            for &point in contour.iter().skip(1).chain(closed.then_some(first)) {
                let b = device(point);
                if mode == 2. || a[0] != b[0] {
                    let left = ((a[0].min(b[0]) - reach).floor() as i64).max(clip.left);
                    let right = ((a[0].max(b[0]) + reach).ceil() as i64).min(clip.right);
                    if left < right {
                        let start = ((left - clip.left) / strip_width) as usize;
                        let end = ((right - 1 - clip.left) / strip_width) as usize;
                        let edge = [a[0] as f32, a[1] as f32, b[0] as f32, b[1] as f32];
                        for strip in &mut strips[start..=end] {
                            strip.edges.push(edge);
                            strip.top = strip.top.min((a[1].min(b[1]) - reach).floor() as i64);
                            strip.bottom = strip.bottom.max((a[1].max(b[1]) + reach).ceil() as i64);
                        }
                    }
                }
                a = b;
            }
        }
        for (i, strip) in strips.into_iter().enumerate() {
            let top = strip.top.max(clip.top);
            let bottom = strip.bottom.min(clip.bottom);
            if top >= bottom {
                continue;
            }
            let left = clip.left + i as i64 * strip_width;
            let right = (left + strip_width).min(clip.right);
            let base = edges.len();
            // The quad stores exact integer indices as f32.
            ensure!(
                base + strip.edges.len().div_ceil(2) < 1 << 24,
                "path edges exceed GPU address range"
            );
            for pair in strip.edges.chunks(2) {
                edges.push([pair[0], pair.get(1).copied().unwrap_or_default()]);
            }
            quads.push([
                [
                    left as f32,
                    top as f32,
                    (right - left) as f32,
                    (bottom - top) as f32,
                ],
                [0.; 4],
                [left as f32, top as f32, right as f32, bottom as f32],
                super::rgba(self.color),
                [size.width as f32, size.height as f32, 1., mode],
                [base as f32, strip.edges.len() as f32, radius as f32, 0.],
            ]);
        }
        Ok(())
    }
}

struct Strip {
    edges: Vec<[f32; 4]>,
    top: i64,
    bottom: i64,
}

impl Default for Strip {
    fn default() -> Self {
        Self {
            edges: Vec::new(),
            top: i64::MAX,
            bottom: i64::MIN,
        }
    }
}
