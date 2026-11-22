use super::{ContentClip, Corners, Draw, Origin, Pixels, Size, Style, UnitResolver};
use crate::{
    layout::{LayoutError, Result},
    style::BoxShadow,
};
use std::sync::Arc;

pub(super) struct Shadows {
    layers: Vec<BoxShadow<Pixels>>,
    radii: Corners<Pixels>,
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
            let shadow = BoxShadow {
                x: units.absolute(layer.x),
                y: units.absolute(layer.y),
                spread: units.absolute(layer.spread),
                blur: units.absolute(layer.blur),
                color: layer.color,
                inset: layer.inset,
            };
            if !shadow.blur.is_extent()
                || [shadow.x, shadow.y, shadow.spread]
                    .iter()
                    .any(|v| !v.get().is_finite())
            {
                return Err(LayoutError::InvalidSize);
            }
            if shadow.color.alpha > 0 {
                resolved.push(shadow);
            }
        }
        Ok((!resolved.is_empty()).then(|| {
            Box::new(Self {
                layers: resolved,
                radii: style.corner_radii,
            })
        }))
    }

    pub(super) fn record(
        &self,
        inset: bool,
        size: Size,
        clip: &Option<Arc<ContentClip>>,
        draws: &mut Vec<Draw>,
    ) {
        let corner_radii = self.radii.fit_to(size);
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
