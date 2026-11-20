//! Defaults and local overrides for text tooltips.
use std::time::Duration;

use crate::{
    Edges, Fill, Pixels, Result, Rgba, SharedString, px, rgb, style::Corners, text::TextStyle,
};

/// Preferred side of the owner. A tooltip can flip to stay inside its viewport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TooltipPlacement {
    /// Above the owner.
    Above,
    /// Below the owner.
    Below,
    /// To the left of the owner.
    Left,
    /// To the right of the owner.
    Right,
}

/// Alignment along the edge shared by the owner and tooltip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TooltipAlignment {
    /// Align the left or top edges.
    Start,
    /// Align the centers.
    Center,
    /// Align the right or bottom edges.
    End,
}

/// A text tooltip with optional overrides of the UI's tooltip style.
#[derive(Clone, Debug)]
#[must_use]
pub struct Tooltip {
    pub(crate) text: SharedString,
    overrides: Option<Box<Overrides>>,
}

impl Tooltip {
    /// Creates a hint that inherits the UI's tooltip style.
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            overrides: None,
        }
    }
}

impl<T: Into<SharedString>> From<T> for Tooltip {
    fn from(text: T) -> Self {
        Self::new(text)
    }
}

macro_rules! tooltip_properties {
    ($($(#[$doc:meta])* $field:ident : $ty:ty = $default:expr => $method:ident),+ $(,)?) => {
        /// Defaults for tooltips in a UI. All dimensions are in logical pixels.
        #[derive(Clone, Debug, PartialEq)]
        pub struct TooltipStyle { $($(#[$doc])* pub $field: $ty),+ }

        impl Default for TooltipStyle {
            fn default() -> Self { Self { $($field: $default),+ } }
        }

        #[derive(Clone, Debug, Default)]
        struct Overrides { $($field: Option<$ty>),+ }

        impl Tooltip {
            $(
                $(#[$doc])*
                pub fn $method(mut self, value: impl Into<$ty>) -> Self {
                    self.overrides.get_or_insert_with(Default::default).$field = Some(value.into());
                    self
                }
            )+

            pub(crate) fn resolve(&self, defaults: &TooltipStyle) -> TooltipStyle {
                let mut style = defaults.clone();
                if let Some(local) = self.overrides.as_deref() {
                    $(if let Some(value) = &local.$field { style.$field.clone_from(value); })+
                }
                style
            }
        }
    }
}

tooltip_properties! {
    /// Fill covering the tooltip box.
    background: Fill = rgb(0x262626).into() => bg,
    /// Foreground of the tooltip text.
    text_color: Rgba = rgb(0xffffff) => text_color,
    /// Font size and line height of the label.
    typography: TextStyle = TextStyle { size: px(14.0), line_height: px(20.0) } => typography,
    /// Insets between the box and its text.
    padding: Edges = Edges { top: px(4.0), right: px(8.0), bottom: px(4.0), left: px(8.0) } => padding,
    /// Radius of each background corner.
    corner_radii: Corners<Pixels> = Corners::all(px(4.0)) => corner_radii,
    /// Maximum width of the box, including padding.
    max_width: Pixels = px(280.0) => max_width,
    /// Distance from the owner's edge.
    gap: Pixels = px(4.0) => gap,
    /// Preferred side of the owner.
    placement: TooltipPlacement = TooltipPlacement::Below => placement,
    /// Alignment along the owner's edge.
    alignment: TooltipAlignment = TooltipAlignment::Start => alignment,
    /// Time the mouse must dwell before the tooltip appears.
    delay: Duration = Duration::from_millis(500) => delay,
}

impl TooltipStyle {
    pub(crate) fn validate(&self) -> Result<()> {
        self.typography.validate()?;
        let p = self.padding;
        let r = self.corner_radii;
        anyhow::ensure!(
            [
                p.top,
                p.right,
                p.bottom,
                p.left,
                p.top + p.bottom,
                p.left + p.right,
                r.top_left,
                r.top_right,
                r.bottom_right,
                r.bottom_left,
                self.gap,
                self.max_width
            ]
            .into_iter()
            .all(Pixels::is_extent),
            "invalid tooltip dimensions"
        );
        Ok(())
    }
}
