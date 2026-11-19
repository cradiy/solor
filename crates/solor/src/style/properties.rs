//! Independently composable groups of style declarations.
use super::values::*;
use crate::{AbsoluteLength, Edges, Fill, Length, Rgba, SharedString};
use std::num::NonZeroUsize;
use std::rc::Rc;

pub(super) trait Overlay {
    fn overlay(&mut self, later: &Self);
}

pub(super) fn overlay_group<T: Overlay + Clone>(base: &mut Option<Rc<T>>, later: &Option<Rc<T>>) {
    if let Some(later) = later {
        if let Some(base) = base {
            if !Rc::ptr_eq(base, later) {
                Rc::make_mut(base).overlay(later);
            }
        } else {
            *base = Some(later.clone());
        }
    }
}

// Each field participates in the cascade at the same granularity as its setter.
macro_rules! properties {
    ($(#[$meta:meta])* $name:ident {
        $($(#[$field_doc:meta])+ $field:ident: $ty:ty),* $(,)?
    } edges { $($(#[$edge_doc:meta])+ $edge:ident: $edge_ty:ty),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Default, PartialEq)]
        pub struct $name {
            $($(#[$field_doc])+ pub $field: Option<$ty>,)*
            $($(#[$edge_doc])+ pub $edge: Edges<Option<$edge_ty>>,)*
        }
        impl Overlay for $name {
            fn overlay(&mut self, later: &Self) {
                $(if later.$field.is_some() { self.$field.clone_from(&later.$field); })*
                $(overlay_edges(&mut self.$edge, &later.$edge);)*
            }
        }
    };
}

fn overlay_edges<T: Clone>(base: &mut Edges<Option<T>>, later: &Edges<Option<T>>) {
    for (base, later) in [
        (&mut base.top, &later.top),
        (&mut base.right, &later.right),
        (&mut base.bottom, &later.bottom),
        (&mut base.left, &later.left),
    ] {
        if later.is_some() {
            base.clone_from(later);
        }
    }
}

properties! {
    /// Placement, sizing and container layout declarations.
    LayoutProperties {
        /// Formatting model for this box.
        display: Display,
        /// Positioning scheme for this box.
        position: Position,
        /// Box measured by the size constraints.
        box_sizing: BoxSizing,
        /// Minimum horizontal size.
        min_width: Length,
        /// Minimum vertical size.
        min_height: Length,
        /// Maximum horizontal size.
        max_width: Length,
        /// Maximum vertical size.
        max_height: Length,
        /// Preferred width divided by height.
        aspect_ratio: f32,
        /// Horizontal overflow policy.
        overflow_x: Overflow,
        /// Vertical overflow policy.
        overflow_y: Overflow,
        /// Width reserved for a scrollbar.
        scrollbar_width: AbsoluteLength,
        /// Main axis of a flex container.
        flex_direction: FlexDirection,
        /// Line wrapping in a flex container.
        flex_wrap: FlexWrap,
        /// Initial main-axis size of a flex item.
        flex_basis: Length,
        /// Share of surplus space.
        flex_grow: f32,
        /// Factor for distributing a space deficit.
        flex_shrink: f32,
        /// Order within the container.
        order: i32,
        /// Cross-axis alignment of children.
        align_items: Align,
        /// Cross-axis alignment of this item.
        align_self: Align,
        /// Distribution of tracks or flex lines on the cross axis.
        align_content: Distribution,
        /// Distribution along the main axis.
        justify_content: Distribution,
        /// Inline-axis alignment of grid children.
        justify_items: Align,
        /// Inline-axis alignment of this grid item.
        justify_self: Align,
        /// Explicit column template, in order.
        grid_columns: Rc<[GridTrack]>,
        /// Explicit row template, in order.
        grid_rows: Rc<[GridTrack]>,
        /// Placement order for automatic grid items.
        grid_auto_flow: GridAutoFlow,
        /// Sizing pattern for implicit columns.
        grid_auto_columns: Rc<[GridTrack]>,
        /// Sizing pattern for implicit rows.
        grid_auto_rows: Rc<[GridTrack]>,
        /// Starting column edge.
        grid_column_start: GridLine,
        /// Ending column edge.
        grid_column_end: GridLine,
        /// Starting row edge.
        grid_row_start: GridLine,
        /// Ending row edge.
        grid_row_end: GridLine,
    } edges {
        /// Outer spacing on each side, including automatic and signed lengths.
        margin: Length,
        /// Offsets from the positioning reference on each side.
        inset: Length,
    }
}

properties! {
    /// Visual and pointer declarations local to a box.
    Appearance {
        /// Whether the box is visible.
        visibility: Visibility,
        /// Sibling paint order. Higher values draw in front, and ties follow declaration order.
        z_index: i32,
        /// Opacity of the composited subtree, from zero to one.
        opacity: f32,
        /// Ordered shadow layers. An empty list clears all layers.
        shadows: Rc<[BoxShadow]>,
        /// Blur radius applied to the backdrop.
        backdrop_blur: AbsoluteLength,
        /// Ordered transform functions. An empty list is the identity.
        transform: Rc<[Transform]>,
        /// Horizontal transform origin in the border box.
        transform_origin_x: Length,
        /// Vertical transform origin in the border box.
        transform_origin_y: Length,
        /// Sizing of replaced content within the content box.
        object_fit: ObjectFit,
        /// Horizontal position of replaced content.
        object_position_x: Length,
        /// Vertical position of replaced content.
        object_position_y: Length,
        /// Width of the outline outside the border.
        outline_width: AbsoluteLength,
        /// Spacing from the border to the outline.
        outline_offset: AbsoluteLength,
        /// Outline paint.
        outline_color: Rgba,
        /// Outline stroke pattern.
        outline_style: BorderStyle,
        /// Requested pointer cursor.
        cursor: Cursor,
        /// Eligibility for pointer targeting.
        pointer_events: PointerEvents,
    } edges {
        /// Border thickness on each side.
        border_width: AbsoluteLength,
        /// Border paint on each side.
        border_fill: Fill,
        /// Border stroke pattern on each side.
        border_style: BorderStyle,
    }
}

properties! {
    /// Inheritable font, paragraph and text decoration declarations.
    TextAttributes {
        /// Preferred font family name.
        font_family: SharedString,
        /// Ordered fallback family names.
        font_fallbacks: Rc<[SharedString]>,
        /// Ordered OpenType shaping feature settings.
        font_features: Rc<[FontFeature]>,
        /// Numeric font weight on the OpenType scale from 1 to 1000.
        font_weight: u16,
        /// Font slant.
        font_style: FontStyle,
        /// Additional spacing between characters.
        letter_spacing: AbsoluteLength,
        /// Alignment of text within a line.
        text_align: TextAlign,
        /// Whitespace and line wrapping policy.
        white_space: WhiteSpace,
        /// Truncation at an overflowing line.
        text_overflow: TextOverflow,
        /// Maximum visible lines. An explicit inner None removes the limit.
        line_clamp: Option<NonZeroUsize>,
        /// Background color behind glyph runs.
        text_background: Rgba,
        /// Draw an underline.
        underline: bool,
        /// Draw a line through the text.
        strikethrough: bool,
        /// Color for text decorations.
        decoration_color: Rgba,
        /// Stroke thickness for text decorations.
        decoration_thickness: AbsoluteLength,
    } edges {}
}
