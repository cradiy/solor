//! Device-pixel outline masks shared by text renderers.
use super::{Clip, RasterScale};
use crate::{layout::Origin, text::Paragraph};
use std::io::{Error, Result};
/// Grayscale outline coverage and its offset from the glyph baseline.
pub use swash::scale::image::Image as GlyphMask;
use swash::{
    FontRef,
    scale::{Render, ScaleContext, Source},
};

/// Identity of a rasterized outline at one device size and baseline phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    /// Face identity, including its variation settings.
    pub face: u64,
    /// IEEE 754 bits of the device em size.
    pub size_bits: u32,
    /// Glyph index in the face.
    pub glyph: u16,
    /// Fraction of a device pixel below the integral baseline in thirty-seconds.
    pub y_phase: u8,
}

/// Reuses outline scaling state across shaped glyph runs.
#[derive(Default)]
pub struct GlyphRasterizer {
    context: ScaleContext,
}
impl GlyphRasterizer {
    /// Visits glyphs in paint order with their device-space baseline and a mask producer
    /// that the renderer calls on a cache miss. The horizontal clip includes glyph overhang.
    pub fn visit(
        &mut self,
        paragraph: &Paragraph,
        at: Origin,
        scale: RasterScale,
        bounds: Clip,
        mut visit: impl FnMut(
            GlyphKey,
            [i64; 2],
            &mut dyn FnMut(&mut GlyphMask) -> Result<()>,
        ) -> Result<()>,
    ) -> Result<()> {
        const Y_PHASES: i64 = 32;
        let mut renderer = Render::new(&[Source::Outline]);
        let em = paragraph.style.size.get();
        let factor = scale.0 as f32;
        let device_em = em * factor;
        for line in &paragraph.lines {
            for run in line.glyphs.chunk_by(|a, b| a.face == b.face) {
                let face = paragraph
                    .faces
                    .get(run[0].face)
                    .ok_or_else(|| Error::other("invalid glyph face index"))?;
                let font = FontRef::from_index(face.data(), face.index() as usize)
                    .ok_or_else(|| Error::other("invalid paragraph font"))?;
                let mut scaler = self
                    .context
                    .builder_with_id(font, [face.cache_key(), 0])
                    .size(device_em)
                    .variations([("wght", f32::from(face.weight()))])
                    .hint(false)
                    .build();
                // Invert the clip using the full outline box, including overhang and
                // a device pixel of raster rounding on either edge.
                let visible_origins = face.horizontal_outline_bounds().map(|(left, right)| {
                    let min = bounds
                        .left
                        .saturating_sub((right * device_em).ceil() as i64)
                        .saturating_sub(1);
                    let max = bounds
                        .right
                        .saturating_sub((left * device_em).floor() as i64)
                        .saturating_add(1);
                    min..max
                });
                for glyph in run {
                    let glyph_x =
                        scale.edge(f64::from(at.x) + f64::from(glyph.x + em * glyph.x_offset));
                    if visible_origins
                        .as_ref()
                        .is_some_and(|range| !range.contains(&glyph_x))
                    {
                        continue;
                    }

                    let baseline =
                        f64::from(at.y) + f64::from(line.baseline + glyph.y - em * glyph.y_offset);
                    let fixed_y = (baseline * scale.0 * Y_PHASES as f64).round() as i64;
                    let baseline_y = fixed_y.div_euclid(Y_PHASES);
                    let y_phase = fixed_y.rem_euclid(Y_PHASES) as u8;

                    let key = GlyphKey {
                        face: face.cache_key(),
                        size_bits: device_em.to_bits(),
                        glyph: glyph.id,
                        y_phase,
                    };
                    let mut rasterize = |mask: &mut GlyphMask| {
                        mask.clear();
                        // Outline coordinates grow upward from the baseline.
                        renderer.offset(swash::zeno::Vector::new(
                            0.,
                            -f32::from(y_phase) / Y_PHASES as f32,
                        ));
                        if renderer.render_into(&mut scaler, glyph.id, mask) {
                            Ok(())
                        } else {
                            Err(Error::other("glyph outline unavailable"))
                        }
                    };
                    visit(key, [glyph_x, baseline_y], &mut rasterize)?;
                }
            }
        }
        Ok(())
    }
}
