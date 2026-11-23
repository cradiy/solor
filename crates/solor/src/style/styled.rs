use super::*;
use crate::{FontFamily, percent, rem};
use std::num::NonZeroUsize;

// Macros expand inside Styled so all authoring primitives receive the same methods.
// A field's key and presence check also govern cascade and shorthand precedence.
macro_rules! declare {
    ($style:expr, $value:expr, $group:ident, $($field:ident).+) => {{
        let field = binding::Field {
            key: stringify!($group.$($field).+),
            inherited: inheritable!($group),
            layout: affects_layout!($group.$($field).+),
            specified: |s| group_ref!(s, $group).is_some_and(|g| g.$($field).+.is_some()),
        };
        $value.declare($style, field, |s, v| s.$group().$($field).+ = Some(v),
            |s| s.$group().$($field).+ = None);
    }};
    ($style:expr, $value:expr, $($field:ident).+) => {{
        let field = binding::Field {
            key: stringify!($($field).+),
            inherited: inheritable!($($field).+),
            layout: affects_layout!($($field).+),
            specified: |s| s.$($field).+.is_some(),
        };
        $value.declare($style, field, |s, v| s.$($field).+ = Some(v), |s| s.$($field).+ = None);
    }};
}
macro_rules! affects_layout {
    (appearance_mut.border_width.$side:ident) => {
        true
    };
    (appearance_mut.$($field:ident).+) => {
        false
    };
    (corner_radii.$side:ident) => {
        false
    };
    (layout_mut.overflow_x) => {
        false
    };
    (layout_mut.overflow_y) => {
        false
    };
    (layout_mut.vertical_wheel_as_horizontal) => {
        false
    };
    (attributes_mut.text_background) => {
        false
    };
    (attributes_mut.underline) => {
        false
    };
    (attributes_mut.strikethrough) => {
        false
    };
    (attributes_mut.underline_color) => {
        false
    };
    (attributes_mut.strikethrough_color) => {
        false
    };
    (attributes_mut.underline_thickness) => {
        false
    };
    (attributes_mut.strikethrough_thickness) => {
        false
    };
    ($($field:ident).+) => {
        true
    };
}
macro_rules! inheritable {
    (attributes_mut) => {
        true
    };
    (text.$field:ident) => {
        true
    };
    ($($field:ident).+) => {
        false
    };
}
macro_rules! group_ref {
    ($s:expr, layout_mut) => {
        $s.layout.as_deref()
    };
    ($s:expr, appearance_mut) => {
        $s.appearance.as_deref()
    };
    ($s:expr, attributes_mut) => {
        $s.text.attributes.as_deref()
    };
}
macro_rules! property_setters {
    ($group:ident; $($name:ident => $field:ident: $ty:ty),+ $(,)?) => {$(
        #[doc = concat!("Sets the `", stringify!($field), "` declaration from a value or signal.")]
        fn $name(mut self, value: impl Into<Property<$ty>>) -> Self {
            declare!(self.style(), value.into(), $group, $field);
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
        fn $name(mut self, value: impl Into<Property<$ty>>) -> Self {
            let value = value.into();
            $(declare!(self.style(), value.clone(), $group, $field.$edge);)+
            self
        }
    )+};
}

macro_rules! color_setters {
    ($($name:ident => $setter:ident),+ $(,)?) => {$(
        #[doc = concat!("Sets a solid color with [`", stringify!($setter), "`](Self::", stringify!($setter), ").")]
        fn $name(self, value: impl Into<Property<Rgba>>) -> Self { self.$setter(value.into().map(Fill::from)) }
    )+};
}
macro_rules! margin_aliases {
    ($($name:ident => $setter:ident),+ $(,)?) => {$(
        #[doc = concat!("Sets the same margins as [`", stringify!($setter), "`](Self::", stringify!($setter), ").")]
        fn $name(self, value: impl Into<Property<Length>>) -> Self { self.$setter(value) }
    )+};
}

// One row describes which edges a whole-box, axis or side setter refines.
macro_rules! box_methods {
    ($field:ident; $($name:ident, $long:ident => [$($edge:ident),+];)+) => {$(
        #[doc = concat!("Sets ", stringify!($field), " for ", stringify!($($edge),+), ".")]
        fn $name(mut self, value: impl Into<Property<AbsoluteLength>>) -> Self {
            let value = value.into();
            $(declare!(self.style(), value.clone(), $field.$edge);)+
            self
        }
        #[doc = concat!("Sets ", stringify!($field), " for ", stringify!($($edge),+), ".")]
        fn $long(self, value: impl Into<Property<AbsoluteLength>>) -> Self { self.$name(value) }
    )+};
}

macro_rules! presets {
    ($($setter:ident { $($name:ident: $value:expr),+ };)+) => {$($(
        #[doc = concat!("Applies `", stringify!($setter), "` with a length of ", stringify!($value), "rem.")]
        fn $name(self) -> Self { self.$setter(rem($value)) }
    )+)+};
}

macro_rules! flex_presets {
    ($(#[doc = $doc:literal] $name:ident($grow:literal, $shrink:literal, $basis:expr);)+) => {$(
        #[doc = $doc]
        fn $name(self) -> Self {
            self.flex_grow($grow).flex_shrink($shrink).flex_basis($basis)
        }
    )+};
}

macro_rules! corner_presets {
    ($($setter:ident => $corners:literal),+ $(,)?) => { $(
        corner_presets!(@scale $setter, $corners;
            _none: px(0.0), _xs: rem(0.125), _sm: rem(0.25), _md: rem(0.375),
            _lg: rem(0.5), _xl: rem(0.75), _2xl: rem(1.0), _3xl: rem(1.5),
            _full: px(9999.0),
        );
    )+ };
    (@scale $setter:ident, $corners:literal; $($suffix:ident: $unit:ident($radius:literal)),+ $(,)?) => {
        pastey::paste! { $(
            #[doc = concat!(stringify!($radius), stringify!($unit), " on ", $corners, ".")]
            fn [<$setter $suffix>](self) -> Self { self.$setter($unit($radius)) }
        )+ }
    };
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
        vertical_wheel_as_horizontal => vertical_wheel_as_horizontal: bool,
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
    property_setters! { layout_mut;
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
        gradient_map => gradient_map: crate::GradientMap, opacity => opacity: f32, cursor => cursor: Cursor, pointer_events => pointer_events: PointerEvents
    }
    property_setters! { appearance_mut;
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
        text_underline => underline: UnderlineStyle,
        text_strikethrough => strikethrough: bool
    }
    property_setters! { attributes_mut;
        font_features => font_features: Rc<[FontFeature]>, font_family => font_family: FontFamily, font_fallbacks => font_fallbacks: Rc<[FontFamily]>,
        letter_spacing => letter_spacing: AbsoluteLength, text_bg => text_background: crate::Ink,
        underline_color => underline_color: crate::Ink, strikethrough_color => strikethrough_color: crate::Ink,
        underline_thickness => underline_thickness: AbsoluteLength, strikethrough_thickness => strikethrough_thickness: AbsoluteLength
    }
    /// Sets the paint for both underline and strikethrough.
    fn decoration_color(self, color: impl Into<Property<crate::Ink>>) -> Self {
        let color = color.into();
        self.underline_color(color.clone())
            .strikethrough_color(color)
    }
    /// Sets the thickness of both underline and strikethrough.
    fn decoration_thickness(self, thickness: impl Into<Property<AbsoluteLength>>) -> Self {
        let thickness = thickness.into();
        self.underline_thickness(thickness.clone())
            .strikethrough_thickness(thickness)
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
        overflow_x { overflow_x_scroll: Overflow::Scroll };
        overflow_y { overflow_y_scroll: Overflow::Scroll };
        text_underline { underline: UnderlineStyle::Solid, underline_wavy: UnderlineStyle::Wavy };
        border_style { border_solid: BorderStyle::Solid, border_dashed: BorderStyle::Dashed, border_dotted: BorderStyle::Dotted };
        mx { mx_auto: Length::Auto };
        margin { m_auto: Length::Auto };
        font_style { italic: FontStyle::Italic };
        white_space { whitespace_nowrap: WhiteSpace::NoWrap, whitespace_normal: WhiteSpace::Normal };
        text_align { text_start: TextAlign::Start, text_end: TextAlign::End, text_left: TextAlign::Left, text_right: TextAlign::Right, text_center: TextAlign::Center, text_justify: TextAlign::Justify };
        text_overflow { text_ellipsis: TextOverflow::Ellipsis, text_ellipsis_start: TextOverflow::EllipsisStart, text_ellipsis_middle: TextOverflow::EllipsisMiddle };
        cursor {
            cursor_pointer: Cursor::Pointer, cursor_text: Cursor::Text, cursor_grab: Cursor::Grab,
            cursor_grabbing: Cursor::Grabbing, cursor_zoom_in: Cursor::ZoomIn, cursor_zoom_out: Cursor::ZoomOut
        };
    }
    color_setters! {
        border_color => border_fill, border_top_color => border_top_fill, border_right_color => border_right_fill,
        border_bottom_color => border_bottom_fill, border_left_color => border_left_fill
    }
    margin_aliases! {
        margin_x => mx, margin_y => my, margin_top => mt, margin_right => mr,
        margin_bottom => mb, margin_left => ml
    }
    /// Shows the mounted subtree while the signal is true, preserving its layout space.
    fn visible_when(self, source: Signal<bool>) -> Self {
        self.visibility(Visibility::When(source))
    }
    /// Limits the number of visible lines.
    fn line_clamp(mut self, lines: impl Into<Property<Option<NonZeroUsize>>>) -> Self {
        declare!(self.style(), lines.into(), attributes_mut, line_clamp);
        self
    }
    /// Removes a line limit from a reusable or inherited style.
    fn clear_line_clamp(self) -> Self {
        self.line_clamp(None)
    }
    /// Sets overflow on both axes.
    fn overflow(self, value: impl Into<Property<Overflow>>) -> Self {
        let value = value.into();
        self.overflow_x(value.clone()).overflow_y(value)
    }
    /// Sets all four margins.
    fn m(self, value: impl Into<Property<Length>>) -> Self {
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
    /// Requests equal width and height when one dimension is automatic.
    fn aspect_square(self) -> Self {
        self.aspect_ratio(1.0)
    }
    flex_presets! {
        /// Grows and shrinks from a zero-percent basis.
        flex_1(1.0, 1.0, percent(0.0));
        /// Grows and shrinks from the item's preferred main-axis size.
        flex_auto(1.0, 1.0, Length::Auto);
        /// Shrinks from the item's preferred main-axis size when space is limited.
        flex_initial(0.0, 1.0, Length::Auto);
        /// Keeps the item's automatic basis without growing or shrinking.
        flex_none(0.0, 0.0, Length::Auto);
    }
    /// Requests a single clipped line with a trailing ellipsis.
    fn truncate(self) -> Self {
        self.whitespace_nowrap().overflow_hidden().text_ellipsis()
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
        rounded_t, rounded_top => [top_left, top_right];
        rounded_r, rounded_right => [top_right, bottom_right];
        rounded_b, rounded_bottom => [bottom_left, bottom_right];
        rounded_l, rounded_left => [top_left, bottom_left];
        rounded_tl, rounded_top_left => [top_left];
        rounded_tr, rounded_top_right => [top_right];
        rounded_br, rounded_bottom_right => [bottom_right];
        rounded_bl, rounded_bottom_left => [bottom_left];
    }

    corner_presets!(
        rounded => "all corners",
        rounded_t => "top corners",
        rounded_r => "right corners",
        rounded_b => "bottom corners",
        rounded_l => "left corners",
        rounded_tl => "the top-left corner",
        rounded_tr => "the top-right corner",
        rounded_br => "the bottom-right corner",
        rounded_bl => "the bottom-left corner",
    );

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
    }

    /// Sets the preferred width of the box selected by `box_sizing`.
    fn w(mut self, width: impl Into<Property<Length>>) -> Self {
        declare!(self.style(), width.into(), width);
        self
    }
    /// Sets the preferred height of the box selected by `box_sizing`.
    fn h(mut self, height: impl Into<Property<Length>>) -> Self {
        declare!(self.style(), height.into(), height);
        self
    }
    /// Sets both dimensions to the same length.
    fn size(self, side: impl Into<Property<Length>>) -> Self {
        let side = side.into();
        self.w(side.clone()).h(side)
    }
    /// Derives preferred width from content.
    fn w_auto(self) -> Self {
        self.w(Length::Auto)
    }
    /// Derives preferred height from content.
    fn h_auto(self) -> Self {
        self.h(Length::Auto)
    }
    /// Sets the spacing between adjacent items.
    fn gap(mut self, gap: impl Into<Property<AbsoluteLength>>) -> Self {
        let gap = gap.into();
        declare!(self.style(), gap.clone(), row_gap);
        declare!(self.style(), gap, column_gap);
        self
    }
    /// Sets the spacing between columns.
    fn gap_x(mut self, gap: impl Into<Property<AbsoluteLength>>) -> Self {
        declare!(self.style(), gap.into(), column_gap);
        self
    }
    /// Sets the spacing between rows.
    fn gap_y(mut self, gap: impl Into<Property<AbsoluteLength>>) -> Self {
        declare!(self.style(), gap.into(), row_gap);
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
    fn rounded(self, radius: impl Into<Property<AbsoluteLength>>) -> Self {
        let r = radius.into();
        self.rounded_tl(r.clone())
            .rounded_tr(r.clone())
            .rounded_bl(r.clone())
            .rounded_br(r)
    }
    /// Sets the inherited solid or gradient foreground for text and monochrome SVGs.
    fn text_color(mut self, color: impl Into<Foreground>) -> Self {
        self.style().text.color = Some(color.into());
        self
    }
    /// Sets the inherited font size.
    /// The default line height is the resolved font size times the golden ratio.
    fn text_size(mut self, size: impl Into<Property<AbsoluteLength>>) -> Self {
        declare!(self.style(), size.into(), text.size);
        self
    }
    /// Sets the inherited line height. Percentages use each label's resolved
    /// font size, and automatic height uses its default line spacing.
    /// Text accepts resolved heights from its font size through 256 logical pixels.
    fn line_height(mut self, height: impl Into<Property<Length>>) -> Self {
        declare!(self.style(), height.into(), text.line_height);
        self
    }
}
impl Styled for StyleRefinement {
    fn style(&mut self) -> &mut Self {
        self
    }
}
