//! Records a styled outline in the border box's local coordinates.
use super::{ContentClip, Draw, Origin, Size, Style, UnitResolver};
use crate::layout::{LayoutError, Result};
use crate::style::{BorderPaint, BorderStyle, Corners};
use crate::{Edges, Pixels, TransitionProperty, px};
use std::{rc::Rc, sync::Arc};

pub(super) struct MountedOutline {
    offset: Pixels,
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
        let Some(color) = a.outline_color else {
            return Ok(None);
        };
        let transitions = |property| {
            style
                .transitions
                .as_ref()
                .is_some_and(|rules| rules.contains_key(&property))
        };
        if pattern == BorderStyle::None
            || (width == px(0.) && !transitions(TransitionProperty::OutlineWidth))
            || (color.alpha == 0 && !transitions(TransitionProperty::OutlineColor))
        {
            return Ok(None);
        }
        Ok(Some(Box::new(Self {
            offset,
            base_radii: style.corner_radii,
            paint: Rc::new(BorderPaint {
                styles: Edges::all(pattern),
                radii: outer_radii(style.corner_radii, outset),
                ..BorderPaint::solid(width, color)
            }),
        })))
    }

    pub(super) fn length(&self, property: TransitionProperty) -> Option<Pixels> {
        match property {
            TransitionProperty::OutlineWidth => Some(self.paint.widths.top),
            TransitionProperty::OutlineOffset => Some(self.offset),
            _ => None,
        }
    }

    pub(super) fn set_length(&mut self, property: TransitionProperty, value: Pixels) -> bool {
        match property {
            TransitionProperty::OutlineWidth if self.paint.widths.top != value => {
                Rc::make_mut(&mut self.paint).widths = Edges::all(value);
            }
            TransitionProperty::OutlineOffset if self.offset != value => self.offset = value,
            _ => return false,
        }
        self.set_radii(self.base_radii);
        true
    }

    pub(super) fn set_radii(&mut self, base: Corners<Pixels>) {
        self.base_radii = base;
        let radii = outer_radii(base, self.paint.widths.top + self.offset);
        if self.paint.radii != radii {
            Rc::make_mut(&mut self.paint).radii = radii;
        }
    }

    pub(super) fn color(&self) -> Option<crate::Rgba> {
        match self.paint.fills.top.as_ref()? {
            crate::Fill::Solid(color) => Some(*color),
            _ => None,
        }
    }

    pub(super) fn set_color(&mut self, color: crate::Rgba) {
        let fills = &mut Rc::make_mut(&mut self.paint).fills;
        for fill in [
            &mut fills.top,
            &mut fills.right,
            &mut fills.bottom,
            &mut fills.left,
        ] {
            *fill = Some(color.into());
        }
    }

    pub(super) fn draw(&self, size: Size, clip: &Option<Arc<ContentClip>>) -> Result<Option<Draw>> {
        if self.paint.widths.top == px(0.) || self.color().is_some_and(|color| color.alpha == 0) {
            return Ok(None);
        }
        let outset = self.paint.widths.top + self.offset;
        let expand = |axis: Pixels| {
            axis.checked_add(outset * 2.)
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
            Rc::make_mut(&mut paint).radii = outer_radii(fitted, outset);
        }
        let start = px(0.) - outset;
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
