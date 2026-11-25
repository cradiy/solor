//! Distance along a side runs clockwise, including its portions of the corner arcs.
use solor::{raster::BorderMarks, style::BorderStyle};
use std::f64::consts::FRAC_PI_4;

pub(super) struct Pattern {
    side: usize,
    bounds: [f64; 4],
    width: f64,
    length: f64,
    start_radius: f64,
    end_radius: f64,
    marks: BorderMarks,
    dotted: bool,
}

impl Pattern {
    pub(super) fn new(
        side: usize,
        style: BorderStyle,
        bounds: [f64; 4],
        widths: [f64; 4],
        radii: [f64; 4],
    ) -> Option<Self> {
        let width = widths[side];
        let [left, top, right, bottom] = bounds;
        let length = if side.is_multiple_of(2) {
            right - left
        } else {
            bottom - top
        };
        let start_radius = radii[side];
        let end_radius = radii[(side + 1) % 4];
        let marks = BorderMarks::fit(
            style,
            length,
            width,
            [start_radius, end_radius],
            [widths[(side + 3) % 4], widths[(side + 1) % 4]],
        )?;
        Some(Self {
            side,
            bounds,
            width,
            length,
            start_radius,
            end_radius,
            marks,
            dotted: style == BorderStyle::Dotted,
        })
    }

    fn local(&self, x: f64, y: f64) -> (f64, f64) {
        let [l, t, r, b] = self.bounds;
        match self.side {
            0 => (x - l, y - t),
            1 => (y - t, r - x),
            2 => (r - x, b - y),
            _ => (b - y, x - l),
        }
    }

    fn along(&self, u: f64, v: f64) -> f64 {
        let half = self.width * 0.5;
        let r = self.start_radius;
        if r > half && u < r && v < r {
            return r - (r - half) * (r - u).atan2(r - v);
        }
        let r = self.end_radius;
        let tangent = self.length - r;
        if r > half && u > tangent && v < r {
            return tangent + (r - half) * (u - tangent).atan2(r - v);
        }
        u
    }

    fn mark_center(&self, s: f64) -> (f64, f64) {
        let half = self.width * 0.5;
        let r = self.start_radius;
        if r > half && s < r {
            let radius = r - half;
            let (sin, cos) = ((r - s) / radius).sin_cos();
            return (r - radius * sin, r - radius * cos);
        }
        let r = self.end_radius;
        let tangent = self.length - r;
        if r > half && s > tangent {
            let radius = r - half;
            let (sin, cos) = ((s - tangent) / radius).sin_cos();
            return (tangent + radius * sin, r - radius * cos);
        }
        (s, half)
    }

    fn nearest_mark(&self, along: f64) -> f64 {
        let index = ((along - self.marks.first) / self.marks.period)
            .round()
            .clamp(0.0, self.marks.intervals);
        self.marks.first + index * self.marks.period
    }

    fn straight_marks(&self, low: f64, high: f64, chord: f64, mut emit: impl FnMut(f64, f64)) {
        if low >= high || chord <= 0.0 {
            return;
        }
        if 2.0 * chord >= self.marks.period {
            let start = low.max(self.marks.first - chord);
            let end = high.min(self.marks.first + self.marks.period * self.marks.intervals + chord);
            if start < end {
                emit(start, end);
            }
            return;
        }
        let first = ((low - self.marks.first - chord) / self.marks.period)
            .ceil()
            .max(0.0);
        let last = ((high - self.marks.first + chord) / self.marks.period)
            .floor()
            .min(self.marks.intervals);
        if first <= last {
            for offset in 0..=(last - first) as u64 {
                let center = self.marks.first + (first + offset as f64) * self.marks.period;
                emit(low.max(center - chord), high.min(center + chord));
            }
        }
    }

    fn covered(&self, x: f64, y: f64) -> bool {
        let (u, v) = self.local(x, y);
        let along = self.along(u, v);
        let mark = self.nearest_mark(along);
        if self.dotted {
            let (cx, cy) = self.mark_center(mark);
            (u - cx).powi(2) + (v - cy).powi(2) <= (self.width * 0.5).powi(2)
        } else {
            (along - mark).abs() <= self.width
        }
    }

    pub(super) fn spans(&self, from: f64, to: f64, y: f64, mut emit: impl FnMut(f64, f64, f64)) {
        if from >= to {
            return;
        }
        // Unresolvable periods use the mean ink coverage, independent of sample phase.
        if self.marks.period < 0.5 && self.marks.intervals >= 1.0 {
            let fraction =
                self.width / self.marks.period * if self.dotted { FRAC_PI_4 } else { 2.0 };
            emit(from, to, fraction.min(1.0));
            return;
        }
        let (u0, v0) = self.local(from, y);
        let (u1, _) = self.local(to, y);
        let half = self.width * 0.5;
        // Leave space for dots whose centers are on the adjacent arc.
        let straight_start = if self.start_radius > half {
            self.start_radius + half
        } else {
            0.0
        };
        let straight_end = self.length
            - if self.end_radius > half {
                self.end_radius + half
            } else {
                0.0
            };
        if self.side.is_multiple_of(2) && straight_start < straight_end {
            let forward = self.side == 0;
            let project = |u: f64| {
                if forward {
                    self.bounds[0] + u
                } else {
                    self.bounds[2] - u
                }
            };
            let start = project(straight_start).clamp(from, to);
            let end = project(straight_end).clamp(from, to);
            let (a, b) = (start.min(end), start.max(end));
            self.sample_curve(from, a, y, &mut emit);
            let chord = if self.dotted {
                (half * half - (v0 - half).powi(2)).max(0.0).sqrt()
            } else {
                self.width
            };
            let low = u0.min(u1).max(straight_start);
            let high = u0.max(u1).min(straight_end);
            self.straight_marks(low, high, chord, |l, r| {
                let (start, end) = (project(l), project(r));
                emit(start.min(end), start.max(end), 1.0);
            });
            self.sample_curve(b, to, y, &mut emit);
        } else if !self.side.is_multiple_of(2) && (straight_start..=straight_end).contains(&u0) {
            let distance = (u0 - self.nearest_mark(u0)).abs();
            if self.dotted && distance < half {
                let chord = (half * half - distance * distance).sqrt();
                let center = if self.side == 1 {
                    self.bounds[2] - half
                } else {
                    self.bounds[0] + half
                };
                emit(from.max(center - chord), to.min(center + chord), 1.0);
            } else if !self.dotted && distance <= self.width {
                emit(from, to, 1.0);
            }
        } else {
            self.sample_curve(from, to, y, &mut emit);
        }
    }

    fn sample_curve(&self, from: f64, to: f64, y: f64, emit: &mut impl FnMut(f64, f64, f64)) {
        let mut x = from;
        let mut run = None;
        while x < to {
            let end = (x.floor() + 1.0).min(to);
            let width = end - x;
            let count = [0.125, 0.375, 0.625, 0.875]
                .into_iter()
                .filter(|f| self.covered(x + width * f, y))
                .count();
            if count == 4 {
                run.get_or_insert(x);
            } else {
                if let Some(start) = run.take() {
                    emit(start, x, 1.0)
                }
                if count > 0 {
                    emit(x, end, count as f64 * 0.25)
                }
            }
            x = end;
        }
        if let Some(start) = run {
            emit(start, to, 1.0)
        }
    }
}
