use super::*;
use crate::{SharedString, percent, rem};
use std::num::NonZeroUsize;

// Macros expand inside Styled so all authoring primitives receive the same methods.
macro_rules! property_setters {
    ($group:ident; $($name:ident => $field:ident: $ty:ty),+ $(,)?) => {$(
        #[doc = concat!("Sets the `", stringify!($field), "` declaration.")]
        fn $name(mut self, value: $ty) -> Self {
            self.style().$group().$field = Some(value);
            self
        }
    )+};
}
macro_rules! converting_setters {
    ($group:ident; $($name:ident => $field:ident: $ty:ty),+ $(,)?) => {$(
        #[doc = concat!("Sets the element's `", stringify!($field), "` declaration.")]
        fn $name(mut self, value: impl Into<$ty>) -> Self {
            self.style().$group().$field = Some(value.into());
            self
        }
    )+};
}
macro_rules! keyword_setters {
    ($($setter:ident { $($name:ident: $value:path),+ $(,)? };)+) => {$($(
        #[doc = concat!("Sets `", stringify!($setter), "` to [`", stringify!($value), "`].")]
        fn $name(self) -> Self { self.$setter($value) }
    )+)+};
}
macro_rules! side_setters {
    ($group:ident, $field:ident: $ty:ty; $($name:ident => [$($edge:ident),+]);+ $(;)?) => {$(
        #[doc = concat!("Sets `", stringify!($field), "` on the ", stringify!($($edge),+), " edges.")]
        fn $name(mut self, value: impl Into<$ty>) -> Self {
            let value = value.into();
            let edges = &mut self.style().$group().$field;
            $(edges.$edge = Some(value.clone());)+
            self
        }
    )+};
}

macro_rules! color_setters {
    ($($name:ident => $setter:ident),+ $(,)?) => {$(
        #[doc = concat!("Sets a solid color with [`", stringify!($setter), "`](Self::", stringify!($setter), ").")]
        fn $name(self, value: impl Into<Rgba>) -> Self { self.$setter(value.into()) }
    )+};
}
macro_rules! margin_aliases {
    ($($name:ident => $setter:ident),+ $(,)?) => {$(
        #[doc = concat!("Sets the same margins as [`", stringify!($setter), "`](Self::", stringify!($setter), ").")]
        fn $name(self, value: impl Into<Length>) -> Self { self.$setter(value) }
    )+};
}

// One row describes which edges a whole-box, axis or side setter refines.
macro_rules! box_methods {
    ($field:ident; $($name:ident, $long:ident => [$($edge:ident),+];)+) => {$(
        #[doc = concat!("Sets ", stringify!($field), " for ", stringify!($($edge),+), ".")]
        fn $name(mut self, value: impl Into<AbsoluteLength>) -> Self {
            let value = value.into();
            let edges = &mut self.style().$field;
            $(edges.$edge = Some(value);)+
            self
        }
        #[doc = concat!("Sets ", stringify!($field), " for ", stringify!($($edge),+), ".")]
        fn $long(self, value: impl Into<AbsoluteLength>) -> Self { self.$name(value) }
    )+};
}

macro_rules! presets {
    ($($setter:ident { $($name:ident: $value:expr),+ };)+) => {$($(
        #[doc = concat!("Applies `", stringify!($setter), "` with a length of ", stringify!($value), "rem.")]
        fn $name(self) -> Self { self.$setter(rem($value)) }
    )+)+};
}

/// Common style declarations for containers, text and custom nodes.
///
/// Builders compose field by field. Custom elements can read the cascaded values
/// in [`Element::prepare_content`](crate::Element::prepare_content). Containers consume dimensions, padding,
/// margins, direction and main-axis spacing.
pub trait Styled: Sized {
    /// Borrows the element's local declarations.
    fn style(&mut self) -> &mut StyleRefinement;

    /// Applies a builder operation once if the condition is true during construction.
    fn when(self, condition: bool, build: impl FnOnce(Self) -> Self) -> Self {
        if condition { build(self) } else { self }
    }

    /// Passes a present value to a builder operation during construction.
    fn when_some<T>(self, value: Option<T>, build: impl FnOnce(Self, T) -> Self) -> Self {
        match value {
            Some(value) => build(self, value),
            None => self,
        }
    }

    /// Overlays a reusable set of style declarations.
    fn with_style(mut self, style: &StyleRefinement) -> Self {
        self.style().refine(style);
        self
    }

    property_setters! { layout_mut;
        display => display: Display, position => position: Position, box_sizing => box_sizing: BoxSizing,
        overflow_x => overflow_x: Overflow, overflow_y => overflow_y: Overflow,
        aspect_ratio => aspect_ratio: f32, order => order: i32,
        flex_direction => flex_direction: FlexDirection, flex_wrapping => flex_wrap: FlexWrap,
        flex_grow => flex_grow: f32, flex_shrink => flex_shrink: f32,
        align_items => align_items: Align, align_self => align_self: Align,
        align_content => align_content: Distribution, justify_content => justify_content: Distribution,
        justify_items => justify_items: Align, justify_self => justify_self: Align,
        grid_auto_flow => grid_auto_flow: GridAutoFlow,
        col_start => grid_column_start: GridLine, col_end => grid_column_end: GridLine,
        row_start => grid_row_start: GridLine, row_end => grid_row_end: GridLine
    }
    converting_setters! { layout_mut;
        min_w => min_width: Length, max_w => max_width: Length,
        min_h => min_height: Length, max_h => max_height: Length,
        flex_basis => flex_basis: Length, scrollbar_width => scrollbar_width: AbsoluteLength,
        grid_cols => grid_columns: Rc<[GridTrack]>, grid_rows => grid_rows: Rc<[GridTrack]>,
        grid_auto_cols => grid_auto_columns: Rc<[GridTrack]>, grid_auto_rows => grid_auto_rows: Rc<[GridTrack]>
    }
    side_setters! { layout_mut, margin: Length;
        margin => [top,right,bottom,left]; mx => [left,right]; my => [top,bottom];
        mt => [top]; mr => [right]; mb => [bottom]; ml => [left]
    }
    side_setters! { layout_mut, inset: Length;
        inset => [top,right,bottom,left]; inset_x => [left,right]; inset_y => [top,bottom];
        top => [top]; right => [right]; bottom => [bottom]; left => [left]
    }
    property_setters! { appearance_mut;
        object_fit => object_fit: ObjectFit, outline_style => outline_style: BorderStyle,
        visibility => visibility: Visibility, z_index => z_index: i32,
        opacity => opacity: f32, cursor => cursor: Cursor, pointer_events => pointer_events: PointerEvents
    }
    converting_setters! { appearance_mut;
        outline_width => outline_width: AbsoluteLength, outline_offset => outline_offset: AbsoluteLength,
        outline_color => outline_color: Rgba, object_position_x => object_position_x: Length,
        object_position_y => object_position_y: Length, shadow => shadows: Rc<[BoxShadow]>, backdrop_blur => backdrop_blur: AbsoluteLength,
        transform => transform: Rc<[Transform]>, transform_origin_x => transform_origin_x: Length,
        transform_origin_y => transform_origin_y: Length
    }
    side_setters! { appearance_mut, border_width: AbsoluteLength;
        border => [top,right,bottom,left]; border_x => [left,right]; border_y => [top,bottom];
        border_t => [top]; border_r => [right]; border_b => [bottom]; border_l => [left]
    }
    side_setters! { appearance_mut, border_fill: Fill;
        border_fill => [top,right,bottom,left]; border_top_fill => [top]; border_right_fill => [right];
        border_bottom_fill => [bottom]; border_left_fill => [left]
    }
    side_setters! { appearance_mut, border_style: BorderStyle;
        border_style => [top,right,bottom,left]; border_top_style => [top]; border_right_style => [right];
        border_bottom_style => [bottom]; border_left_style => [left]
    }
    property_setters! { attributes_mut;
        font_weight => font_weight: u16, font_style => font_style: FontStyle, text_align => text_align: TextAlign,
        white_space => white_space: WhiteSpace, text_overflow => text_overflow: TextOverflow,
         text_underline => underline: bool,
        text_strikethrough => strikethrough: bool
    }
    converting_setters! { attributes_mut;
        font_features => font_features: Rc<[FontFeature]>, font_family => font_family: SharedString, font_fallbacks => font_fallbacks: Rc<[SharedString]>,
        letter_spacing => letter_spacing: AbsoluteLength, text_bg => text_background: Rgba,
        decoration_color => decoration_color: Rgba, decoration_thickness => decoration_thickness: AbsoluteLength
    }
    keyword_setters! {
        display { block: Display::Block, flex: Display::Flex, grid: Display::Grid, hidden: Display::None };
        position { relative: Position::Relative, absolute: Position::Absolute, fixed: Position::Fixed, sticky: Position::Sticky };
        visibility { visible: Visibility::Visible, invisible: Visibility::Hidden };
        flex_direction { flex_row: FlexDirection::Row, flex_col: FlexDirection::Column, flex_row_reverse: FlexDirection::RowReverse, flex_col_reverse: FlexDirection::ColumnReverse };
        flex_wrapping { flex_wrap: FlexWrap::Wrap, flex_nowrap: FlexWrap::NoWrap, flex_wrap_reverse: FlexWrap::WrapReverse };
        align_items { items_center: Align::Center, items_start: Align::Start, items_end: Align::End, items_stretch: Align::Stretch, items_baseline: Align::Baseline };
        align_self { self_center: Align::Center, self_start: Align::Start, self_end: Align::End, self_stretch: Align::Stretch, self_baseline: Align::Baseline };
        justify_content { justify_center: Distribution::Center, justify_between: Distribution::SpaceBetween, justify_around: Distribution::SpaceAround, justify_evenly: Distribution::SpaceEvenly, justify_start: Distribution::Start, justify_end: Distribution::End };
        overflow { overflow_hidden: Overflow::Hidden, overflow_scroll: Overflow::Scroll, overflow_auto: Overflow::Auto, overflow_visible: Overflow::Visible };
        border_style { border_solid: BorderStyle::Solid, border_dashed: BorderStyle::Dashed, border_dotted: BorderStyle::Dotted };
        mx { mx_auto: Length::Auto };
        margin { m_auto: Length::Auto };
        font_style { italic: FontStyle::Italic };
        white_space { whitespace_nowrap: WhiteSpace::NoWrap, whitespace_normal: WhiteSpace::Normal };
        text_align { text_start: TextAlign::Start, text_end: TextAlign::End, text_left: TextAlign::Left, text_right: TextAlign::Right, text_center: TextAlign::Center, text_justify: TextAlign::Justify };
        text_overflow { text_ellipsis: TextOverflow::Ellipsis, text_ellipsis_start: TextOverflow::EllipsisStart, text_ellipsis_middle: TextOverflow::EllipsisMiddle };
        cursor { cursor_pointer: Cursor::Pointer, cursor_text: Cursor::Text };
    }
    color_setters! {
        border_color => border_fill, border_top_color => border_top_fill, border_right_color => border_right_fill,
        border_bottom_color => border_bottom_fill, border_left_color => border_left_fill
    }
    margin_aliases! {
        margin_x => mx, margin_y => my, margin_top => mt, margin_right => mr,
        margin_bottom => mb, margin_left => ml
    }
    /// Limits the number of visible lines.
    fn line_clamp(mut self, lines: NonZeroUsize) -> Self {
        self.style().attributes_mut().line_clamp = Some(Some(lines));
        self
    }
    /// Removes a line limit from a reusable or inherited style.
    fn clear_line_clamp(mut self) -> Self {
        self.style().attributes_mut().line_clamp = Some(None);
        self
    }
    /// Sets overflow on both axes.
    fn overflow(self, value: Overflow) -> Self {
        self.overflow_x(value).overflow_y(value)
    }
    /// Sets all four margins.
    fn m(self, value: impl Into<Length>) -> Self {
        self.margin(value)
    }
    /// Requests the full containing width.
    fn w_full(self) -> Self {
        self.w(percent(100.0))
    }
    /// Requests the full containing height.
    fn h_full(self) -> Self {
        self.h(percent(100.0))
    }
    /// Declares a zero-percent basis and grow and shrink factors of one.
    fn flex_1(self) -> Self {
        self.flex_grow(1.0)
            .flex_shrink(1.0)
            .flex_basis(percent(0.0))
    }
    /// Keeps the item's automatic basis without growing or shrinking.
    fn flex_none(self) -> Self {
        self.flex_grow(0.0)
            .flex_shrink(0.0)
            .flex_basis(Length::Auto)
    }
    /// Requests a single clipped line with a trailing ellipsis.
    fn truncate(self) -> Self {
        self.whitespace_nowrap().overflow_hidden().text_ellipsis()
    }
    /// Enables underlining.
    fn underline(self) -> Self {
        self.text_underline(true)
    }
    /// Enables strikethrough.
    fn line_through(self) -> Self {
        self.text_strikethrough(true)
    }

    box_methods! { padding;
        p, padding => [top, right, bottom, left];
        px, padding_x => [left, right];
        py, padding_y => [top, bottom];
        pt, padding_top => [top];
        pr, padding_right => [right];
        pb, padding_bottom => [bottom];
        pl, padding_left => [left];
    }

    box_methods! { corner_radii;
        rounded_tl, rounded_top_left => [top_left];
        rounded_tr, rounded_top_right => [top_right];
        rounded_br, rounded_bottom_right => [bottom_right];
        rounded_bl, rounded_bottom_left => [bottom_left];
    }

    // One spacing step is a quarter rem.
    presets! {
        m { m_0: 0.0, m_1: 0.25, m_2: 0.5, m_3: 0.75, m_4: 1.0 };
        mx { mx_0: 0.0, mx_1: 0.25, mx_2: 0.5, mx_3: 0.75, mx_4: 1.0 };
        my { my_0: 0.0, my_1: 0.25, my_2: 0.5, my_3: 0.75, my_4: 1.0 };
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

    /// Sets the preferred width of the box selected by `box_sizing`.
    fn w(mut self, width: impl Into<Length>) -> Self {
        self.style().width = Some(width.into());
        self
    }
    /// Sets the preferred height of the box selected by `box_sizing`.
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
        let gap = gap.into();
        self.style().row_gap = Some(gap);
        self.style().column_gap = Some(gap);
        self
    }
    /// Sets the spacing between columns.
    fn gap_x(mut self, gap: impl Into<AbsoluteLength>) -> Self {
        self.style().column_gap = Some(gap.into());
        self
    }
    /// Sets the spacing between rows.
    fn gap_y(mut self, gap: impl Into<AbsoluteLength>) -> Self {
        self.style().row_gap = Some(gap.into());
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
        self.style().corner_radii = Corners::all(Some(radius.into()));
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
