use crate::{
    mask::{MaskPainter, painter},
    pixels::{Clip, RasterScale, Surface},
};
use solor::{
    Ink,
    layout::{Origin, Size},
};
use std::{f64::consts::TAU, io::Result};

pub(super) fn draw(
    surface: &mut Surface,
    at: Origin,
    size: Size,
    color: &Ink,
    scale: RasterScale,
    clip: Clip,
) -> Result<()> {
    let mut brush = painter(color, at, size, scale, (0, 0))?;
    let left = f64::from(at.x) * scale.0;
    let right = left + f64::from(size.width) * scale.0;
    let height = f64::from(size.height) * scale.0;
    let center = f64::from(at.y) * scale.0 + height * 0.5;
    let thickness = height / 3.;
    let amplitude = 0.8 * thickness;
    let frequency = TAU / (9. * thickness);
    // Reuse the wave's cross sections for each row in a tile.
    const TILE: usize = 64;
    for start in (clip.left..clip.right).step_by(TILE) {
        let len = (clip.right - start).min(TILE as i64) as usize;
        let mut sections = [[(0., 0., 0.); 4]; TILE];
        for (i, column) in sections[..len].iter_mut().enumerate() {
            let x = (start + i as i64) as f64;
            let from = x.max(left);
            let step = ((x + 1.).min(right) - from) / 4.;
            for (sample, section) in column.iter_mut().enumerate() {
                let dx = (from - left) + (sample as f64 + 0.5) * step;
                let (sin, cos) = (dx * frequency).sin_cos();
                let y = center + amplitude * sin;
                let half = thickness * 0.5 * (1. + (amplitude * frequency * cos).powi(2)).sqrt();
                *section = (y - half, y + half, step);
            }
        }
        for y in clip.top..clip.bottom {
            let mut mask = [0u8; TILE];
            for (alpha, column) in mask.iter_mut().zip(&sections[..len]) {
                let coverage: f64 = column
                    .iter()
                    .map(|&(top, bottom, weight)| {
                        ((y as f64 + 1.).min(bottom) - (y as f64).max(top)).max(0.) * weight
                    })
                    .sum();
                *alpha = (coverage * 255.).round() as u8;
            }
            let start_byte = 4 * (y as usize * surface.size.width as usize + start as usize);
            let pixels = surface.rgba[start_byte..start_byte + len * 4]
                .as_chunks_mut::<4>()
                .0;
            brush.paint_row(pixels, &mask[..len], start, y)?;
        }
    }
    Ok(())
}
