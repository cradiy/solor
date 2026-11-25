use super::{GradientBrush, rgba};
use solor::{Checkerboard, DiagonalStripes, Fill, Gradient, layout::Size, raster::RasterScale};

#[derive(Clone, Copy)]
pub(super) enum Brush<'a> {
    Gradient(&'a Gradient),
    Checkerboard(Checkerboard),
    DiagonalStripes(DiagonalStripes),
}

impl<'a> Brush<'a> {
    pub(super) fn from_fill(fill: &'a Fill) -> Option<Self> {
        match fill {
            Fill::Gradient(g) => Some(Self::Gradient(g)),
            Fill::Checkerboard(p) => Some(Self::Checkerboard(*p)),
            Fill::DiagonalStripes(p) => Some(Self::DiagonalStripes(*p)),
            _ => None,
        }
    }

    pub(super) fn parameters(
        self,
        size: Size,
        scale: RasterScale,
        stops: &mut Vec<[[f32; 4]; 2]>,
    ) -> ([f32; 4], [f32; 4], f32) {
        match self {
            Self::Gradient(g) => {
                let brush = GradientBrush::prepare(g, size, stops);
                (brush.params, brush.ramp, brush.oklab)
            }
            Self::Checkerboard(pattern) => {
                let cell = pattern_span(f64::from(pattern.cell_size()) * scale.0);
                (rgba(pattern.color), [cell, 0., 0., 0.], 2.)
            }
            Self::DiagonalStripes(pattern) => {
                let axis = scale.0 * std::f64::consts::SQRT_2;
                let width = f64::from(pattern.width()) * axis;
                let gap = f64::from(pattern.gap()) * axis;
                let period = width + gap;
                let side = 1. % period;
                // Full periods in a device pixel contribute their mean density.
                let whole = (1. - side * side) * (width / period);
                (
                    rgba(pattern.color),
                    [
                        pattern_span(width),
                        pattern_span(gap),
                        side as f32,
                        whole as f32,
                    ],
                    3.,
                )
            }
        }
    }
}

// Leave room for the full pattern period and its neighboring band in shader arithmetic.
fn pattern_span(device_pixels: f64) -> f32 {
    device_pixels.min(f64::from(f32::MAX) / 4.) as f32
}
