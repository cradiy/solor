use crate::{
    Ink, Pixels,
    layout::{LayoutError, Result},
    style::{Typography, UnderlineStyle},
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
    pub background: Option<Ink>,
    pub underline: Option<(Stroke, UnderlineStyle)>,
    pub strike: Option<Stroke>,
    pub thickness: Option<Pixels>,
    pub color: Option<Ink>,
}

impl Text {
    pub(crate) fn set_decorations(&mut self, text: &Typography) -> Result<()> {
        if text.decoration_thickness.is_some_and(|v| !v.is_extent()) {
            return Err(LayoutError::InvalidSize);
        }
        self.decorations = text.attributes.as_deref().and_then(|attrs| {
            let background = attrs
                .text_background
                .as_ref()
                .filter(|ink| !ink.is_clear_solid())
                .cloned();
            let strokes_visible = !text.decoration_thickness.is_some_and(|v| v.get() == 0.0);
            let underline = attrs
                .underline
                .filter(|s| strokes_visible && *s != UnderlineStyle::None);
            let strike = strokes_visible && attrs.strikethrough == Some(true);
            if background.is_none() && underline.is_none() && !strike {
                return None;
            }
            let engine = self.engine.borrow();
            let db = engine.fonts.db();
            let family = engine.preferences.resolve(self.paragraph.family.as_ref());
            let query = cosmic_text::fontdb::Query {
                families: &[family],
                weight: self.paragraph.weight,
                style: self.paragraph.slant,
                ..Default::default()
            };
            let strokes = (underline.is_some() || strike)
                .then(|| {
                    let id = db
                        .query(&query)
                        .or_else(|| db.query(&super::sans_query()))?;
                    db.with_face_data(id, |bytes, index| {
                        ttf_parser::Face::parse(bytes, index)
                            .ok()
                            .map(|face| FontStrokes::new(&face))
                    })
                    .flatten()
                })
                .flatten();
            Some(Box::new(Decorations {
                background,
                underline: strokes
                    .as_ref()
                    .zip(underline)
                    .map(|(s, kind)| (s.underline, kind)),
                strike: strokes.as_ref().filter(|_| strike).map(|s| s.strike),
                thickness: text.decoration_thickness,
                color: attrs.decoration_color.clone(),
            }))
        });
        Ok(())
    }
}
