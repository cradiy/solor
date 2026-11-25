use crate::style::BorderStyle;
use std::f64::consts::{FRAC_PI_2, FRAC_PI_4};

/// Positions of equally spaced dash or dot centers along one clockwise border edge.
pub struct BorderMarks {
    /// Distance from the side origin to the first mark center.
    pub first: f64,
    /// Distance between consecutive centers.
    pub period: f64,
    /// Number of intervals between the first and last centers.
    pub intervals: f64,
}

impl BorderMarks {
    /// Fits marks to an edge in device pixels. Radii and neighboring widths are
    /// ordered from the start to the end of the clockwise edge.
    pub fn fit(
        style: BorderStyle,
        length: f64,
        width: f64,
        radii: [f64; 2],
        neighbors: [f64; 2],
    ) -> Option<Self> {
        if width <= 0. {
            return None;
        }
        let dotted = match style {
            BorderStyle::Dashed => false,
            BorderStyle::Dotted => true,
            _ => return None,
        };
        let half = width * 0.5;
        // Put a mark at each join. On an open square end, inset it by half its length.
        let end_distance = |radius: f64, neighbor: f64| {
            if radius > half {
                let angle = if neighbor == 0.0 {
                    FRAC_PI_2
                } else {
                    FRAC_PI_4
                };
                radius - (radius - half) * angle
            } else if neighbor > 0.0 {
                neighbor * 0.5
            } else if dotted {
                half
            } else {
                width
            }
        };
        let start = end_distance(radii[0], neighbors[0]);
        let end = length - end_distance(radii[1], neighbors[1]);
        let span = (end - start).max(0.0);
        let target = width * if dotted { 2.0 } else { 3.0 };
        let intervals = (span / target).round();
        let (first, period) = if intervals >= 1.0 {
            (start, span / intervals)
        } else {
            ((start + end) * 0.5, target)
        };
        Some(Self {
            first,
            period,
            intervals,
        })
    }
}
