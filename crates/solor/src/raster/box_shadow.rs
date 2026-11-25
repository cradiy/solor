use super::{Clip, RasterScale};
use crate::{
    Pixels, Rgba,
    layout::{Origin, Size},
    style::{BoxShadow, Corners},
};

/// Axis-aligned device-space box with elliptical corners.
pub struct RoundedBox {
    /// Left, top, right, and bottom edges.
    pub bounds: [f64; 4],
    /// Horizontal and vertical radii, clockwise from the top left.
    pub corners: [[f64; 2]; 4],
}

impl RoundedBox {
    /// Horizontal interval inside the box at a given device-space y coordinate.
    pub fn span(&self, y: f64) -> Option<(f64, f64)> {
        let [l, t, r, b] = self.bounds;
        if y < t || y >= b || l >= r {
            return None;
        }
        let inset = |[rx, ry]: [f64; 2], dy: f64| {
            if dy >= ry {
                0.0
            } else {
                let k = 1.0 - dy / ry;
                rx * (1.0 - (1.0 - k * k).sqrt())
            }
        };
        let [tl, tr, br, bl] = self.corners;
        let start = l + inset(tl, y - t).max(inset(bl, b - y));
        let end = r - inset(tr, y - t).max(inset(br, b - y));
        (start < end).then_some((start, end))
    }
}

/// Device-space silhouette and caster used to rasterize one box shadow.
pub struct BoxShadowGeometry {
    /// Element silhouette that separates inner and outer shadow coverage.
    pub base: RoundedBox,
    /// Offset, spread-adjusted shape. For an inset shadow it describes the hole.
    pub caster: RoundedBox,
    /// Width of each of the three box filters, in device pixels.
    pub blur: f64,
    /// Whether the shadow is clipped to the inside of the silhouette.
    pub inset: bool,
    /// Shadow color before coverage is applied.
    pub color: Rgba,
}

impl BoxShadowGeometry {
    /// Resolves the spread, offset and fitted corner radii at the target scale.
    pub fn new(
        origin: Origin,
        size: Size,
        corners: Corners<Pixels>,
        shadow: BoxShadow<Pixels>,
        scale: RasterScale,
    ) -> Self {
        let dev = |v: Pixels| f64::from(v) * scale.0;
        let (l, t, w, h) = (
            dev(origin.x),
            dev(origin.y),
            dev(size.width),
            dev(size.height),
        );
        let radii = [
            corners.top_left,
            corners.top_right,
            corners.bottom_right,
            corners.bottom_left,
        ]
        .map(dev);
        let spread = dev(shadow.spread) * if shadow.inset { -1. } else { 1. };
        let dx = dev(shadow.x);
        let dy = dev(shadow.y);
        let mut expanded = radii.map(|r| if r == 0. { 0. } else { (r + spread).max(0.) });
        let [tl, tr, br, bl] = expanded;
        let mut fit = 1.0_f64;
        for (length, pair) in [
            (w + 2. * spread, (tl + tr).max(bl + br)),
            (h + 2. * spread, (tl + bl).max(tr + br)),
        ] {
            if pair > 0. {
                fit = fit.min(length.max(0.) / pair);
            }
        }
        expanded.iter_mut().for_each(|r| *r *= fit);
        Self {
            base: RoundedBox {
                bounds: [l, t, l + w, t + h],
                corners: radii.map(|v| [v, v]),
            },
            caster: RoundedBox {
                bounds: [
                    l + dx - spread,
                    t + dy - spread,
                    l + dx + w + spread,
                    t + dy + h + spread,
                ],
                corners: expanded.map(|v| [v, v]),
            },
            blur: dev(shadow.blur),
            color: shadow.color,
            inset: shadow.inset,
        }
    }

    /// Device-pixel bounds outside which the shadow cannot paint.
    pub fn bounds(&self) -> Clip {
        let [l, t, r, b] = self.base.bounds;
        let (bounds, margin) = if self.inset {
            (self.base.bounds, 0.)
        } else {
            (self.caster.bounds, self.blur * 1.5)
        };
        let [x, y, right, bottom] = bounds;
        if l >= r || t >= b || x >= right || y >= bottom {
            return Clip {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
        }
        Clip {
            left: (x - margin).floor() as i64,
            top: (y - margin).floor() as i64,
            right: (right + margin).ceil() as i64,
            bottom: (bottom + margin).ceil() as i64,
        }
    }
}
