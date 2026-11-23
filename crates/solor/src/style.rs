//! Resolved box properties and field-wise declaration refinements.
mod binding;
pub use binding::Bindings;
pub use binding::Property;
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

/// Resolved per-edge paint for a border drawn inside an element's bounds.
#[derive(Clone, Debug, PartialEq)]
pub struct BorderPaint {
    /// Border widths in logical pixels.
    pub widths: Edges<Pixels>,
    /// Paint mapped across the border box on each visible side.
    pub fills: Edges<Option<Fill>>,
    /// Stroke pattern on each side of the border box.
    pub styles: Edges<BorderStyle>,
    /// Outer corner radii in logical pixels.
    pub radii: Corners<Pixels>,
}

impl BorderPaint {
    /// Creates a square-cornered border with one width and fill on all four sides.
    pub fn solid(width: Pixels, fill: impl Into<Fill>) -> Self {
        let fill = Some(fill.into());
        Self {
            widths: Edges::all(width),
            styles: Edges::all(BorderStyle::Solid),
            fills: Edges {
                top: fill.clone(),
                right: fill.clone(),
                bottom: fill.clone(),
                left: fill,
            },
            radii: Corners::default(),
        }
    }

    /// Returns the painted sides in top, right, bottom, left order.
    pub fn visible_fills(&self) -> [Option<&Fill>; 4] {
        let widths = self.widths;
        let styles = self.styles;
        let fills = &self.fills;
        [
            visible_fill(widths.top, styles.top, &fills.top),
            visible_fill(widths.right, styles.right, &fills.right),
            visible_fill(widths.bottom, styles.bottom, &fills.bottom),
            visible_fill(widths.left, styles.left, &fills.left),
        ]
    }
}

fn visible_fill(width: Pixels, style: BorderStyle, fill: &Option<Fill>) -> Option<&Fill> {
    fill.as_ref()
        .filter(|_| width > px(0.) && style != BorderStyle::None)
}

macro_rules! background_sources {
    ($($(#[$doc:meta])* $variant:ident($ty:ty)),+ $(,)?) => {
        /// Paint source used when recording a background.
        #[derive(Clone, Debug)]
        pub enum Background {
            /// Paint stored directly on the declaration.
            Fixed(Fill),
            $($(#[$doc])* $variant(Signal<$ty>)),+
        }

        $(impl From<Signal<$ty>> for Background {
            fn from(signal: Signal<$ty>) -> Self {
                Self::$variant(signal)
            }
        })+

        impl Background {
            pub(crate) fn sample(&self, cx: &crate::PaintContext<'_>) -> crate::Result<Fill> {
                fn read<T: Clone + Into<Fill> + 'static>(
                    cx: &crate::PaintContext<'_>, signal: Signal<T>,
                ) -> crate::Result<Fill> {
                    Ok(cx.read(signal)?.clone().into())
                }
                match self {
                    Self::Fixed(fill) => Ok(fill.clone()),
                    $(Self::$variant(signal) => read(cx, *signal)),+
                }
            }
        }
    };
}

background_sources! {
    /// Fill sampled during paint.
    Bound(Fill),
    /// Solid color sampled during paint.
    Color(Rgba),
    /// Gradient sampled during paint.
    Gradient(crate::Gradient),
    /// Checkerboard sampled during paint.
    Checkerboard(crate::Checkerboard),
    /// Diagonal stripes sampled during paint.
    DiagonalStripes(crate::DiagonalStripes),
    /// Shared bitmap sampled during paint.
    Image(crate::ImageAsset),
    /// Application-rendered image sampled during paint.
    CustomImage(crate::CustomImage),
}

impl<T: Into<Fill>> From<T> for Background {
    fn from(fill: T) -> Self {
        Self::Fixed(fill.into())
    }
}

/// Inherited foreground sampled by text and custom content during paint.
#[derive(Clone, Debug, PartialEq)]
pub enum Foreground {
    /// Use this color for the lifetime of the declaration.
    Fixed(crate::Ink),
    /// Read the current color when an element paints with the foreground.
    Bound(Signal<Rgba>),
    /// Read the current gradient when painting.
    Gradient(Signal<crate::Gradient>),
    /// Read a foreground that may switch between solid and gradient paint.
    Ink(Signal<crate::Ink>),
}

impl<T: Into<crate::Ink>> From<T> for Foreground {
    fn from(color: T) -> Self {
        Self::Fixed(color.into())
    }
}

impl From<Signal<Rgba>> for Foreground {
    fn from(signal: Signal<Rgba>) -> Self {
        Self::Bound(signal)
    }
}

impl From<Signal<crate::Gradient>> for Foreground {
    fn from(signal: Signal<crate::Gradient>) -> Self {
        Self::Gradient(signal)
    }
}
impl From<Signal<crate::Ink>> for Foreground {
    fn from(signal: Signal<crate::Ink>) -> Self {
        Self::Ink(signal)
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
    /// Additional horizontal advance per shaped glyph, in logical pixels.
    pub letter_spacing: Pixels,
    /// Underline thickness in logical pixels, defaulting to the font metric.
    pub underline_thickness: Option<Pixels>,
    /// Strikethrough thickness in logical pixels, defaulting to the font metric.
    pub strikethrough_thickness: Option<Pixels>,
    /// Foreground used to record glyphs.
    pub color: Foreground,
}
impl Default for Typography {
    fn default() -> Self {
        let metrics = crate::text::TextStyle::default();
        Self {
            attributes: None,
            size: metrics.size,
            line_height: metrics.line_height,
            letter_spacing: px(0.0),
            underline_thickness: None,
            strikethrough_thickness: None,
            color: Rgba::new(0, 0, 0, 255).into(),
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
    /// Line height, with percentages relative to this element's font size.
    pub line_height: Option<Length>,
    /// Local foreground-color override.
    pub color: Option<Foreground>,
}
impl TextStyleRefinement {
    pub(crate) fn overlay(&self, higher: &Self) -> Self {
        let mut attributes = self.attributes.clone();
        overlay_group(&mut attributes, &higher.attributes);
        Self {
            attributes,
            size: higher.size.or(self.size),
            line_height: higher.line_height.or(self.line_height),
            color: higher.color.clone().or_else(|| self.color.clone()),
        }
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
    /// Signal sources installed by the property builders.
    pub bindings: Bindings,
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
    pub(crate) fn border_widths(
        &self,
        units: UnitResolver,
    ) -> crate::layout::Result<Edges<Pixels>> {
        let Some(a) = self.appearance.as_deref() else {
            return Ok(Edges::default());
        };
        let width = |v: Option<AbsoluteLength>| {
            let value = v.map_or(Pixels::default(), |v| units.absolute(v));
            if value.is_extent() {
                Ok(value)
            } else {
                Err(crate::layout::LayoutError::InvalidSize)
            }
        };
        Ok(Edges {
            top: width(a.border_width.top)?,
            right: width(a.border_width.right)?,
            bottom: width(a.border_width.bottom)?,
            left: width(a.border_width.left)?,
        })
    }

    pub(crate) fn border_paint(
        &self,
        units: UnitResolver,
    ) -> crate::layout::Result<Option<Rc<BorderPaint>>> {
        let Some(a) = self.appearance.as_deref() else {
            return Ok(None);
        };
        let widths = self.border_widths(units)?;
        if widths == Edges::default() {
            return Ok(None);
        }
        let styles = Edges {
            top: a.border_style.top.unwrap_or(BorderStyle::Solid),
            right: a.border_style.right.unwrap_or(BorderStyle::Solid),
            bottom: a.border_style.bottom.unwrap_or(BorderStyle::Solid),
            left: a.border_style.left.unwrap_or(BorderStyle::Solid),
        };
        let fills = Edges {
            top: visible_fill(widths.top, styles.top, &a.border_fill.top).cloned(),
            right: visible_fill(widths.right, styles.right, &a.border_fill.right).cloned(),
            bottom: visible_fill(widths.bottom, styles.bottom, &a.border_fill.bottom).cloned(),
            left: visible_fill(widths.left, styles.left, &a.border_fill.left).cloned(),
        };
        Ok((fills != Edges::default()).then(|| {
            Rc::new(BorderPaint {
                widths,
                fills,
                styles,
                radii: self.corner_radii,
            })
        }))
    }

    pub(crate) fn display_none(&self) -> bool {
        self.layout.as_deref().and_then(|layout| layout.display) == Some(Display::None)
    }

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
        let text = inherited.overlay(&later.text);
        if let Some(size) = text.size {
            self.text.size = units.absolute(size);
        }
        self.text.line_height = match text.line_height {
            Some(Length::Percent(value)) => self.text.size * (value / 100.0),
            Some(Length::Absolute(height)) if later.text.line_height.is_none() => {
                units.absolute(height).max(self.text.size)
            }
            Some(Length::Absolute(height)) => units.absolute(height),
            None if text.size.is_none() => self.text.line_height,
            None | Some(Length::Auto) => {
                crate::text::TextStyle::default_line_height(self.text.size)
            }
        };
        if let Some(color) = &text.color {
            self.text.color = color.clone();
        }
        overlay_group(&mut self.text.attributes, &text.attributes);
        if let Some(attrs) = self.text.attributes.as_deref() {
            if let Some(spacing) = attrs.letter_spacing {
                self.text.letter_spacing = units.absolute(spacing);
            }
            if let Some(width) = attrs.underline_thickness {
                self.text.underline_thickness = Some(units.absolute(width));
            }
            if let Some(width) = attrs.strikethrough_thickness {
                self.text.strikethrough_thickness = Some(units.absolute(width));
            }
        }
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
        let higher = later.bindings.without_fixed_overrides(later);
        let mut bindings = self
            .bindings
            .without_fixed_overrides(self)
            .unshadowed(later);
        bindings.extend(&higher);
        self.bindings = bindings;
        higher.clear_fixed(self);
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
