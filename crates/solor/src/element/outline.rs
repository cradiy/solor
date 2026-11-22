//! Records a styled outline in the border box's local coordinates.
use super::{ContentClip, Draw, Origin, Size, Style, UnitResolver};
use crate::layout::{LayoutError, Result};
use crate::style::{BorderPaint, BorderStyle, Corners};
use crate::{Edges, Fill, Pixels, px};
use std::{rc::Rc, sync::Arc};

pub(super) struct MountedOutline {
    outset: Pixels,
    base_radii: Corners<Pixels>,
    paint: Rc<BorderPaint>,
}

impl MountedOutline {
    pub(super) fn resolve(style: &Style, units: UnitResolver) -> Result<Option<Box<Self>>> {
        let Some(a) = style.appearance.as_deref() else {
            return Ok(None);
        };
        let Some(width) = a.outline_width else {
            return Ok(None);
        };
        let width = units.absolute(width);
        let offset = a.outline_offset.map_or(px(0.), |v| units.absolute(v));
        let outset = width.checked_add(offset).ok_or(LayoutError::InvalidSize)?;
        if !width.is_extent() {
            return Err(LayoutError::InvalidSize);
        }
        let pattern = a.outline_style.unwrap_or(BorderStyle::Solid);
        let Some(color) = a.outline_color.filter(|color| color.alpha != 0) else {
            return Ok(None);
        };
        if width == px(0.) || pattern == BorderStyle::None {
            return Ok(None);
        }
        let fill = || Some(Fill::Solid(color));
        Ok(Some(Box::new(Self {
            outset,
            base_radii: style.corner_radii,
            paint: Rc::new(BorderPaint {
                widths: Edges::all(width),
                styles: Edges::all(pattern),
                fills: Edges {
                    top: fill(),
                    right: fill(),
                    bottom: fill(),
                    left: fill(),
                },
                radii: outer_radii(style.corner_radii, outset),
            }),
        })))
    }

    pub(super) fn draw(&self, size: Size, clip: &Option<Arc<ContentClip>>) -> Result<Option<Draw>> {
        let expand = |axis: Pixels| {
            axis.checked_add(self.outset * 2.)
                .ok_or(LayoutError::InvalidSize)
        };
        let outer = Size {
            width: expand(size.width)?,
            height: expand(size.height)?,
        };
        if outer.width <= px(0.) || outer.height <= px(0.) {
            return Ok(None);
        }
        let mut paint = self.paint.clone();
        let fitted = self.base_radii.fit_to(size);
        if fitted != self.base_radii {
            Rc::make_mut(&mut paint).radii = outer_radii(fitted, self.outset);
        }
        let start = px(0.) - self.outset;
        Ok(Some(Draw::Border {
            clip: clip.clone(),
            origin: Origin { x: start, y: start },
            size: outer,
            paint,
        }))
    }
}

fn outer_radii(base: Corners<Pixels>, outset: Pixels) -> Corners<Pixels> {
    base.map(|r| {
        if r == px(0.) {
            r
        } else {
            (r + outset).max(px(0.))
        }
    })
}
