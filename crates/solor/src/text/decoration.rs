use crate::{
    Ink, Pixels,
    layout::{LayoutError, Result},
    style::{Typography, UnderlineStyle},
};

use super::Text;

pub(crate) struct Stroke {
    pub position: Pixels,
    pub thickness: Pixels,
    pub color: Option<Ink>,
}

impl Stroke {
    fn from_metrics(
        metrics: Option<ttf_parser::LineMetrics>,
        units_per_em: f32,
        font_size: Pixels,
        fallback_position: f32,
        thickness: Option<Pixels>,
        color: Option<Ink>,
    ) -> Self {
        let position = metrics.map_or(fallback_position, |m| f32::from(m.position) / units_per_em);
        let thickness = thickness.unwrap_or_else(|| {
            font_size
                * metrics
                    .filter(|m| m.thickness > 0)
                    .map_or(1.0 / 16.0, |m| f32::from(m.thickness) / units_per_em)
        });
        Self {
            position: font_size * position,
            thickness,
            color,
        }
    }
}

pub(crate) struct Decorations {
    pub background: Option<Ink>,
    pub underline: Option<(Stroke, UnderlineStyle)>,
    pub strike: Option<Stroke>,
}

impl Text {
    pub(crate) fn set_decorations(&mut self, text: &Typography) -> Result<()> {
        if [text.underline_thickness, text.strikethrough_thickness]
            .into_iter()
            .flatten()
            .any(|v| !v.is_extent())
        {
            return Err(LayoutError::InvalidSize);
        }
        self.decorations = text.attributes.as_deref().and_then(|attrs| {
            let background = attrs
                .text_background
                .as_ref()
                .filter(|ink| !ink.is_clear_solid())
                .cloned();
            let visible = |width: Option<Pixels>| width.is_none_or(|v| v.get() > 0.0);
            let underline = attrs
                .underline
                .filter(|s| visible(text.underline_thickness) && *s != UnderlineStyle::None);
            let strike = visible(text.strikethrough_thickness) && attrs.strikethrough == Some(true);
            if underline.is_none() && !strike {
                return background.map(|ink| {
                    Box::new(Decorations {
                        background: Some(ink),
                        underline: None,
                        strike: None,
                    })
                });
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
            let metrics = db
                .query(&query)
                .or_else(|| db.query(&super::sans_query()))
                .and_then(|id| {
                    db.with_face_data(id, |bytes, index| {
                        let face = ttf_parser::Face::parse(bytes, index).ok()?;
                        Some((
                            f32::from(face.units_per_em()),
                            face.underline_metrics(),
                            face.strikeout_metrics(),
                        ))
                    })?
                });
            Some(Box::new(Decorations {
                background,
                underline: underline.zip(metrics).map(|(kind, (em, line, _))| {
                    (
                        Stroke::from_metrics(
                            line,
                            em,
                            text.size,
                            -0.1,
                            text.underline_thickness,
                            attrs.underline_color.clone(),
                        ),
                        kind,
                    )
                }),
                strike: metrics.filter(|_| strike).map(|(em, _, line)| {
                    Stroke::from_metrics(
                        line,
                        em,
                        text.size,
                        0.3,
                        text.strikethrough_thickness,
                        attrs.strikethrough_color.clone(),
                    )
                }),
            }))
        });
        Ok(())
    }
}
