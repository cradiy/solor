//! Resolved box properties and field-wise declaration refinements.
mod properties;
mod styled;
mod values;

use crate::length::UnitResolver;
use crate::{AbsoluteLength, Edges, Fill, Length, Pixels, Rgba, Signal, px};
use properties::overlay_group;
pub use properties::{Appearance, LayoutProperties, TextAttributes};
use std::rc::Rc;
pub use styled::Styled;
pub use values::*;

/// Paint source used when recording a background.
#[derive(Clone, Debug)]
pub enum Background {
    /// Paint stored directly on the declaration.
    Fixed(Fill),
    /// Paint read from the signal at recording time.
    Bound(Signal<Fill>),
}
impl<T: Into<Fill>> From<T> for Background {
    fn from(fill: T) -> Self {
        Self::Fixed(fill.into())
    }
}
impl From<Signal<Fill>> for Background {
    fn from(signal: Signal<Fill>) -> Self {
        Self::Bound(signal)
    }
}

/// Font metrics and foreground color for content.
#[derive(Clone, Debug, PartialEq)]
pub struct Typography {
    /// Cascaded font, paragraph and decoration declarations.
    pub attributes: Option<Rc<TextAttributes>>,
    /// Em size in logical pixels.
    pub size: Pixels,
    /// Vertical distance between successive lines.
    pub line_height: Pixels,
    /// Foreground used to record glyphs.
    pub color: Rgba,
}
impl Default for Typography {
    fn default() -> Self {
        Self {
            attributes: None,
            size: px(16.0),
            line_height: px(24.0),
            color: Rgba::new(0, 0, 0, 255),
        }
    }
}

/// Inheritable typography declarations applied per property.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextStyleRefinement {
    /// Font, paragraph and decoration overrides.
    pub attributes: Option<Rc<TextAttributes>>,
    /// Local font-size override.
    pub size: Option<AbsoluteLength>,
    /// Local line-height override.
    pub line_height: Option<AbsoluteLength>,
    /// Local foreground-color override.
    pub color: Option<Rgba>,
}
impl TextStyleRefinement {
    fn overlay(&self, higher: &Self) -> Self {
        let mut attributes = self.attributes.clone();
        overlay_group(&mut attributes, &higher.attributes);
        Self {
            attributes,
            size: higher.size.or(self.size),
            line_height: higher.line_height.or(self.line_height),
            color: higher.color.or(self.color),
        }
    }
    fn apply_metrics(&self, text: &mut Typography, units: UnitResolver) {
        text.size = self.size.map_or(text.size, |size| units.absolute(size));
        text.line_height = self.line_height.map_or_else(
            || px(text.line_height.get().max(text.size.get())),
            |height| units.absolute(height),
        );
        text.color = self.color.unwrap_or(text.color);
    }
}

/// Local box properties after cascading. An automatic dimension follows the content.
#[derive(Clone, Debug, Default)]
pub struct Style {
    /// Local placement and container layout declarations.
    pub layout: Option<Rc<LayoutProperties>>,
    /// Local paint and interaction declarations.
    pub appearance: Option<Rc<Appearance>>,
    /// Space between the border box and content on each side.
    pub padding: Edges<Pixels>,
    /// Preferred horizontal size before the parent's constraint.
    pub width: Length,
    /// Preferred vertical size.
    pub height: Length,
    /// Space between adjacent items in column layout.
    pub row_gap: Pixels,
    /// Space between adjacent columns.
    pub column_gap: Pixels,
    /// Optional paint covering the assigned box.
    pub background: Option<Background>,
    /// Radius applied to background corners.
    pub corner_radii: Corners<Pixels>,
    /// Resolved typography for this body.
    pub text: Typography,
}

/// Each outer option denotes a declaration. For nullable properties, Some(None) clears it.
#[derive(Clone, Debug, Default)]
pub struct StyleRefinement {
    /// Local placement and container layout declarations.
    pub layout: Option<Rc<LayoutProperties>>,
    /// Local paint and interaction declarations.
    pub appearance: Option<Rc<Appearance>>,
    /// Independently specified padding edges.
    pub padding: Edges<Option<AbsoluteLength>>,
    /// Horizontal size declaration.
    pub width: Option<Length>,
    /// Vertical size declaration.
    pub height: Option<Length>,
    /// Inter-item spacing declaration.
    pub row_gap: Option<AbsoluteLength>,
    /// Inter-column spacing declaration.
    pub column_gap: Option<AbsoluteLength>,
    /// Sets or clears the background when specified.
    pub background: Option<Option<Background>>,
    /// Background corner-radius declaration.
    pub corner_radii: Corners<Option<AbsoluteLength>>,
    /// Inheritable typography overrides.
    pub text: TextStyleRefinement,
}

impl Style {
    pub(crate) fn refine(
        &mut self,
        later: &StyleRefinement,
        inherited: &TextStyleRefinement,
        units: UnitResolver,
    ) -> TextStyleRefinement {
        macro_rules! resolve {
            ($($($field:ident).+),+) => {$(if let Some(value) = later.$($field).+ {
                self.$($field).+ = units.absolute(value);
            })+};
        }
        resolve!(
            padding.top,
            padding.right,
            padding.bottom,
            padding.left,
            row_gap,
            column_gap,
            corner_radii.top_left,
            corner_radii.top_right,
            corner_radii.bottom_right,
            corner_radii.bottom_left
        );
        overlay_group(&mut self.layout, &later.layout);
        overlay_group(&mut self.appearance, &later.appearance);
        if let Some(background) = &later.background {
            self.background.clone_from(background);
        }
        self.width = later.width.unwrap_or(self.width);
        self.height = later.height.unwrap_or(self.height);
        inherited.apply_metrics(&mut self.text, units);
        later.text.apply_metrics(&mut self.text, units);
        let text = inherited.overlay(&later.text);
        overlay_group(&mut self.text.attributes, &text.attributes);
        text
    }
}

impl StyleRefinement {
    fn layout_mut(&mut self) -> &mut LayoutProperties {
        Rc::make_mut(self.layout.get_or_insert_with(Rc::default))
    }

    fn appearance_mut(&mut self) -> &mut Appearance {
        Rc::make_mut(self.appearance.get_or_insert_with(Rc::default))
    }

    fn attributes_mut(&mut self) -> &mut TextAttributes {
        Rc::make_mut(self.text.attributes.get_or_insert_with(Rc::default))
    }

    /// Overlays specified fields while retaining other declarations.
    pub fn refine(&mut self, later: &Self) {
        macro_rules! overlay {
            ($($($field:ident).+),+) => {$(if later.$($field).+.is_some() {
                self.$($field).+ = later.$($field).+.clone();
            })+};
        }
        overlay!(
            padding.top,
            padding.right,
            padding.bottom,
            padding.left,
            width,
            height,
            row_gap,
            column_gap,
            background,
            corner_radii.top_left,
            corner_radii.top_right,
            corner_radii.bottom_right,
            corner_radii.bottom_left
        );
        overlay_group(&mut self.layout, &later.layout);
        overlay_group(&mut self.appearance, &later.appearance);
        self.text = self.text.overlay(&later.text);
    }
}
