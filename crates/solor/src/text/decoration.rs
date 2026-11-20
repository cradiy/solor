use crate::{
    Pixels, Rgba,
    layout::{LayoutError, Result},
    style::Typography,
};

use super::Text;

/// Position above the baseline and natural thickness, in em units.
#[derive(Clone, Copy)]
pub(crate) struct Stroke {
    pub position: f32,
    pub thickness: f32,
}

impl Stroke {
    fn from_font(
        face: &ttf_parser::Face<'_>,
        metrics: Option<ttf_parser::LineMetrics>,
        fallback: f32,
    ) -> Self {
        let em = f32::from(face.units_per_em());
        Self {
            position: metrics.map_or(fallback, |m| f32::from(m.position) / em),
            thickness: metrics
                .filter(|m| m.thickness > 0)
                .map_or(1.0 / 16.0, |m| f32::from(m.thickness) / em),
        }
    }
}

pub(super) struct FontStrokes {
    underline: Stroke,
    strike: Stroke,
}

impl FontStrokes {
    pub(super) fn new(face: &ttf_parser::Face<'_>) -> Self {
        Self {
            underline: Stroke::from_font(face, face.underline_metrics(), -0.1),
            strike: Stroke::from_font(face, face.strikeout_metrics(), 0.3),
        }
    }
}

pub(crate) struct Decorations {
    pub underline: Option<Stroke>,
    pub strike: Option<Stroke>,
    pub thickness: Option<Pixels>,
    pub color: Option<Rgba>,
}

impl Text {
    pub(crate) fn set_decorations(&mut self, text: &Typography) -> Result<()> {
        if text.decoration_thickness.is_some_and(|v| !v.is_extent()) {
            return Err(LayoutError::InvalidSize);
        }
        self.decorations = text.attributes.as_deref().and_then(|attrs| {
            let underline = attrs.underline == Some(true);
            let strike = attrs.strikethrough == Some(true);
            if (!underline && !strike) || text.decoration_thickness.is_some_and(|v| v.get() == 0.0)
            {
                return None;
            }
            let engine = self.engine.borrow();
            Some(Box::new(Decorations {
                underline: underline.then_some(engine.strokes.underline),
                strike: strike.then_some(engine.strokes.strike),
                thickness: text.decoration_thickness,
                color: attrs.decoration_color,
            }))
        });
        Ok(())
    }
}
