use super::{ContentClip, Corners, Draw, Origin, PaintContext, Pixels, Size, Style, UnitResolver};
use crate::{
    layout::{LayoutError, Result},
    style::BoxShadow,
};
use std::sync::Arc;

impl PaintContext<'_> {
    /// Paints one box shadow around a content-local rectangle. Draw outer shadows
    /// before the background and inset shadows after it.
    pub fn paint_shadow(
        &mut self,
        offset: Origin,
        size: Size,
        radii: impl Into<Corners<Pixels>>,
        shadow: BoxShadow,
    ) -> crate::Result<()> {
        size.validate()?;
        let origin = self
            .origin
            .checked_add(offset)
            .ok_or(LayoutError::InvalidSize)?;
        let radii = radii.into();
        anyhow::ensure!(radii.valid_radii(), "invalid corner radius");
        let shadow = shadow.resolve(self.units)?;
        if shadow.color.alpha > 0 && size.width.get() > 0. && size.height.get() > 0. {
            self.commands.push(Draw::Shadow {
                clip: self.clip.clone(),
                origin,
                size,
                corner_radii: radii.fit_to(size),
                shadow,
            });
        }
        Ok(())
    }
}

impl BoxShadow {
    fn resolve(self, units: UnitResolver) -> Result<BoxShadow<Pixels>> {
        let shadow = BoxShadow {
            x: units.absolute(self.x),
            y: units.absolute(self.y),
            spread: units.absolute(self.spread),
            blur: units.absolute(self.blur),
            color: self.color,
            inset: self.inset,
        };
        if !shadow.blur.is_extent()
            || [shadow.x, shadow.y, shadow.spread]
                .iter()
                .any(|v| !v.get().is_finite())
        {
            return Err(LayoutError::InvalidSize);
        }
        Ok(shadow)
    }
}

pub(super) struct Shadows {
    layers: Vec<BoxShadow<Pixels>>,
}

impl Shadows {
    pub(super) fn resolve(style: &Style, units: UnitResolver) -> Result<Option<Box<Self>>> {
        let Some(layers) = style
            .appearance
            .as_deref()
            .and_then(|a| a.shadows.as_deref())
        else {
            return Ok(None);
        };
        let mut resolved = Vec::new();
        for layer in layers {
            let shadow = layer.resolve(units)?;
            if shadow.color.alpha > 0 {
                resolved.push(shadow);
            }
        }
        Ok((!resolved.is_empty()).then(|| Box::new(Self { layers: resolved })))
    }

    pub(super) fn record(
        &self,
        inset: bool,
        radii: Corners<Pixels>,
        size: Size,
        clip: &Option<Arc<ContentClip>>,
        draws: &mut Vec<Draw>,
    ) {
        let corner_radii = radii.fit_to(size);
        for &shadow in self.layers.iter().rev().filter(|s| s.inset == inset) {
            draws.push(Draw::Shadow {
                clip: clip.clone(),
                origin: Origin::default(),
                size,
                corner_radii,
                shadow,
            });
        }
    }
}
