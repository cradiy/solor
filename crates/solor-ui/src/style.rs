//! Resolved box properties and field-wise declaration refinements.
use crate::length::UnitResolver;
use crate::{AbsoluteLength, Edges, Fill, Length, Pixels, Rgba, Signal, px, rem};

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
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Typography {
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
            size: px(16.0),
            line_height: px(24.0),
            color: Rgba::new(0, 0, 0, 255),
        }
    }
}

/// Inheritable typography declarations applied per property.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextStyleRefinement {
    /// Local font-size override.
    pub size: Option<AbsoluteLength>,
    /// Local line-height override.
    pub line_height: Option<AbsoluteLength>,
    /// Local foreground-color override.
    pub color: Option<Rgba>,
}
impl TextStyleRefinement {
    pub(crate) fn overlay(self, higher: Self) -> Self {
        Self {
            size: higher.size.or(self.size),
            line_height: higher.line_height.or(self.line_height),
            color: higher.color.or(self.color),
        }
    }
    pub(crate) fn apply(self, text: &mut Typography, units: UnitResolver) {
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
    /// Space between the border box and content on each side.
    pub padding: Edges<Pixels>,
    /// Preferred horizontal size before the parent's constraint.
    pub width: Length,
    /// Preferred vertical size.
    pub height: Length,
    /// Space between adjacent items in column layout.
    pub gap: Pixels,
    /// Optional paint covering the assigned box.
    pub background: Option<Background>,
    /// Radius applied to background corners.
    pub corner_radius: Pixels,
    /// Resolved typography for this body.
    pub text: Typography,
}

/// Each outer option denotes a declaration. For nullable properties, Some(None) clears it.
#[derive(Clone, Debug, Default)]
pub struct StyleRefinement {
    /// Independently specified padding edges.
    pub padding: Edges<Option<AbsoluteLength>>,
    /// Horizontal size declaration.
    pub width: Option<Length>,
    /// Vertical size declaration.
    pub height: Option<Length>,
    /// Inter-item spacing declaration.
    pub gap: Option<AbsoluteLength>,
    /// Sets or clears the background when specified.
    pub background: Option<Option<Background>>,
    /// Background corner-radius declaration.
    pub corner_radius: Option<AbsoluteLength>,
    /// Inheritable typography overrides.
    pub text: TextStyleRefinement,
}

impl Style {
    pub(crate) fn refine(&mut self, later: &StyleRefinement, units: UnitResolver) {
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
            gap,
            corner_radius
        );
        if let Some(background) = &later.background {
            self.background.clone_from(background);
        }
        self.width = later.width.unwrap_or(self.width);
        self.height = later.height.unwrap_or(self.height);
        later.text.apply(&mut self.text, units);
    }
}

impl StyleRefinement {
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
            gap,
            background,
            corner_radius
        );
        self.text = self.text.overlay(later.text);
    }
}

// One row describes which edges a whole-box, axis or side setter refines.
macro_rules! padding_methods {
    ($($name:ident, $long:ident => [$($edge:ident),+];)+) => {$(
        #[doc = concat!("Sets padding on these edges: ", stringify!($($edge),+), ".")]
        fn $name(mut self, value: impl Into<AbsoluteLength>) -> Self {
            let value = value.into();
            let edges = &mut self.style().padding;
            $(edges.$edge = Some(value);)+
            self
        }
        #[doc = concat!("Sets padding on these edges: ", stringify!($($edge),+), ".")]
        fn $long(self, value: impl Into<AbsoluteLength>) -> Self { self.$name(value) }
    )+};
}

macro_rules! presets {
    ($($setter:ident { $($name:ident: $value:expr),+ };)+) => {$($(
        #[doc = concat!("Applies `", stringify!($setter), "` with a length of ", stringify!($value), "rem.")]
        fn $name(self) -> Self { self.$setter(rem($value)) }
    )+)+};
}

/// Common style builders for containers, text and custom nodes.
pub trait Styled: Sized {
    /// Borrows the element's local declarations.
    fn style(&mut self) -> &mut StyleRefinement;

    /// Overlays a reusable set of style declarations.
    fn with_style(mut self, style: &StyleRefinement) -> Self {
        self.style().refine(style);
        self
    }

    padding_methods! {
        p, padding => [top, right, bottom, left];
        px, padding_x => [left, right];
        py, padding_y => [top, bottom];
        pt, padding_top => [top];
        pr, padding_right => [right];
        pb, padding_bottom => [bottom];
        pl, padding_left => [left];
    }

    // One spacing step is a quarter rem.
    presets! {
        p { p_0: 0.0, p_1: 0.25, p_2: 0.5, p_3: 0.75, p_4: 1.0 };
        px { px_0: 0.0, px_1: 0.25, px_2: 0.5, px_3: 0.75, px_4: 1.0 };
        py { py_0: 0.0, py_1: 0.25, py_2: 0.5, py_3: 0.75, py_4: 1.0 };
        pt { pt_0: 0.0, pt_1: 0.25, pt_2: 0.5 };
        pr { pr_0: 0.0, pr_1: 0.25, pr_2: 0.5 };
        pb { pb_0: 0.0, pb_1: 0.25, pb_2: 0.5 };
        pl { pl_0: 0.0, pl_1: 0.25, pl_2: 0.5 };
        gap { gap_0: 0.0, gap_1: 0.25, gap_2: 0.5, gap_3: 0.75, gap_4: 1.0 };
        rounded { rounded_none: 0.0, rounded_sm: 0.125, rounded_md: 0.375, rounded_lg: 0.5 };
    }

    /// Sets the preferred border-box width.
    fn w(mut self, width: impl Into<Length>) -> Self {
        self.style().width = Some(width.into());
        self
    }
    /// Sets the preferred border-box height.
    fn h(mut self, height: impl Into<Length>) -> Self {
        self.style().height = Some(height.into());
        self
    }
    /// Sets both dimensions to the same length.
    fn size(self, side: impl Into<Length>) -> Self {
        let side = side.into();
        self.w(side).h(side)
    }
    /// Derives preferred width from content.
    fn w_auto(mut self) -> Self {
        self.style().width = Some(Length::Auto);
        self
    }
    /// Derives preferred height from content.
    fn h_auto(mut self) -> Self {
        self.style().height = Some(Length::Auto);
        self
    }
    /// Sets the spacing between adjacent items.
    fn gap(mut self, gap: impl Into<AbsoluteLength>) -> Self {
        self.style().gap = Some(gap.into());
        self
    }
    /// Paints the assigned box with a fixed or signal-bound fill.
    fn bg(mut self, fill: impl Into<Background>) -> Self {
        self.style().background = Some(Some(fill.into()));
        self
    }
    /// Clears the local background declaration's paint.
    fn clear_bg(mut self) -> Self {
        self.style().background = Some(None);
        self
    }
    /// Sets the radius of the background corners.
    fn rounded(mut self, radius: impl Into<AbsoluteLength>) -> Self {
        self.style().corner_radius = Some(radius.into());
        self
    }
    /// Sets the foreground inherited by descendant text.
    fn text_color(mut self, color: impl Into<Rgba>) -> Self {
        self.style().text.color = Some(color.into());
        self
    }
    /// Sets the inherited font size.
    fn text_size(mut self, size: impl Into<AbsoluteLength>) -> Self {
        self.style().text.size = Some(size.into());
        self
    }
    /// Sets the inherited line height.
    fn line_height(mut self, height: impl Into<AbsoluteLength>) -> Self {
        self.style().text.line_height = Some(height.into());
        self
    }
}
impl Styled for StyleRefinement {
    fn style(&mut self) -> &mut Self {
        self
    }
}
