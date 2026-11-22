//! Values used in portable style declarations.
pub use crate::layout::FlexDirection;
use crate::{AbsoluteLength, Length, Rgba, Signal};

macro_rules! keywords {
    ($(#[doc = $doc:literal] $name:ident { $($variant:ident $(($value:ty))?: $meaning:literal),+ $(,)? })+) => {$(
        #[doc = $doc]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name {
            $(#[doc = $meaning] $variant $(($value))?),+
        }
    )+};
}

keywords! {
    /// Formatting model used for an element and its children.
    Display { Block: "Block layout.", Flex: "Flexible box layout.", Grid: "Grid layout.", None: "Omit the box from layout." }
    /// How the element participates in flow and interprets offsets.
    Position {
        Static: "Normal flow without offsets.", Relative: "Shift from the normal flow position.",
        Absolute: "Place outside flow against the containing block.", Fixed: "Place outside flow relative to the viewport.",
        Sticky: "Constrain the normal flow position against a scrollport."
    }
    /// Behavior of content extending beyond one box axis.
    Overflow {
        Visible: "Allow content beyond the box on this axis.",
        Hidden: "Clip content and descendant hits to the box on this axis.",
        Scroll: "Clip this axis and scroll overflowing children with wheel input.",
        Auto: "Clip this axis and consume wheel input while child content can scroll."
    }
    /// Reference box used by width and height constraints.
    BoxSizing { BorderBox: "Include border and padding.", ContentBox: "Size the content area." }
    /// How replaced content such as an image fits its content box.
    ObjectFit { Fill: "Stretch to fill.", Contain: "Preserve proportions and fit inside.", Cover: "Preserve proportions and cover.", None: "Use intrinsic dimensions.", ScaleDown: "Use the smaller of intrinsic and contained dimensions." }
    /// How flex items break across lines.
    FlexWrap { NoWrap: "One line.", Wrap: "Multiple lines.", WrapReverse: "Multiple lines in reverse order." }
    /// Alignment of an item within the available space.
    Align { Auto: "Use the container's alignment.", Start: "Start edge.", End: "End edge.", Center: "Center.", Baseline: "Align baselines.", Stretch: "Fill the available space." }
    /// Distribution of free space between items or tracks.
    Distribution { Start: "Pack at the start.", End: "Pack at the end.", Center: "Center the group.", Stretch: "Stretch to fill.", SpaceBetween: "Equal interior gaps.", SpaceAround: "Equal space around each item.", SpaceEvenly: "Equal space at every gap and edge." }
    /// Automatic placement order of grid items.
    GridAutoFlow { Row: "Fill rows.", Column: "Fill columns.", RowDense: "Backfill holes by row.", ColumnDense: "Backfill holes by column." }
    /// Stroke pattern of an element's border.
    BorderStyle { None: "No stroke.", Solid: "Continuous stroke.", Dashed: "Dashed stroke.", Dotted: "Dotted stroke." }
    /// Requested font slant.
    FontStyle { Normal: "Upright glyphs.", Italic: "Italic glyphs.", Oblique: "Slanted glyphs." }
    /// Horizontal alignment within a text line.
    TextAlign { Start: "Writing-direction start.", End: "Writing-direction end.", Left: "Left edge.", Right: "Right edge.", Center: "Centered.", Justify: "Justified." }
    /// Treatment of whitespace and wrapping.
    WhiteSpace { Normal: "Collapse spaces and wrap.", NoWrap: "Collapse spaces without wrapping.", Pre: "Preserve whitespace without wrapping.", PreWrap: "Preserve whitespace and wrap." }
    /// Presentation of a line that does not fit.
    TextOverflow { Clip: "Clip the line.", Ellipsis: "Ellipsis at the end.", EllipsisStart: "Ellipsis at the start.", EllipsisMiddle: "Ellipsis in the middle." }
    /// Visual indication for pointer interaction.
    Cursor { Auto: "Platform default.", Default: "Standard arrow.", Pointer: "Clickable target.", Text: "Text selection.", Crosshair: "Crosshair.", Move: "Move target.", Grab: "Grabbable.", Grabbing: "Dragging.", NotAllowed: "Unavailable action.", Wait: "Busy.", Progress: "Working in the background.", ColResize: "Resize columns.", RowResize: "Resize rows.", Resize(crate::ResizeEdge): "Resize a specific edge or corner.", None: "Hide the cursor." }
    /// Participation in pointer hit testing.
    PointerEvents { Auto: "Participate in hit testing.", None: "Ignore the box for pointer targeting." }
}

/// Visibility of a mounted subtree that retains its layout space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    /// Show the subtree when its ancestors are visible.
    Visible,
    /// Hide the subtree.
    Hidden,
    /// Show the subtree while the signal is true and its ancestors are visible.
    When(Signal<bool>),
}

impl From<bool> for Visibility {
    fn from(visible: bool) -> Self {
        if visible { Self::Visible } else { Self::Hidden }
    }
}

/// Minimum breadth of a grid track.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrackMin {
    /// Content-dependent automatic minimum.
    Auto,
    /// Smallest intrinsic content breadth.
    MinContent,
    /// Preferred intrinsic content breadth.
    MaxContent,
    /// Explicit breadth in box length units.
    Length(Length),
}

/// Breadth of a grid track.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrackSize {
    /// Content-dependent automatic breadth.
    Auto,
    /// Smallest intrinsic content breadth.
    MinContent,
    /// Preferred intrinsic content breadth.
    MaxContent,
    /// Explicit breadth in box length units.
    Length(Length),
    /// Share of the remaining space.
    Fraction(f32),
    /// Intrinsic breadth capped by the specified limit.
    FitContent(Length),
}

/// Size constraints of one grid track.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GridTrack {
    /// A single track sizing function.
    Size(TrackSize),
    /// Independent lower and upper sizing functions.
    MinMax(TrackMin, TrackSize),
}

/// Placement of one edge of a grid item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridLine {
    /// Place this edge automatically.
    Auto,
    /// One-based line number, counted from the end for negative values.
    Line(std::num::NonZeroI16),
    /// Number of tracks crossed.
    Span(std::num::NonZeroU16),
}

/// An OpenType layout feature tag and its selector (zero disables the feature).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontFeature {
    /// Four-byte OpenType tag, for example `*b"liga"`.
    pub tag: [u8; 4],
    /// Feature selector.
    pub value: u32,
}

/// Parameters of one box shadow. Layers are ordered front to back.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxShadow<L = AbsoluteLength> {
    /// Horizontal displacement.
    pub x: L,
    /// Vertical displacement.
    pub y: L,
    /// Blur radius.
    pub blur: L,
    /// Expansion before blurring.
    pub spread: L,
    /// Shadow color.
    pub color: Rgba,
    /// Draw inside the border box.
    pub inset: bool,
}

impl BoxShadow {
    /// Creates an outer shadow with the given offset and color.
    pub fn new(x: impl Into<AbsoluteLength>, y: impl Into<AbsoluteLength>, color: Rgba) -> Self {
        Self {
            x: x.into(),
            y: y.into(),
            color,
            spread: crate::px(0.).into(),
            blur: crate::px(0.).into(),
            inset: false,
        }
    }

    /// Softens the shadow edge by the given blur radius.
    pub fn blur(mut self, radius: impl Into<AbsoluteLength>) -> Self {
        self.blur = radius.into();
        self
    }

    /// Expands the shadow before blurring. Negative lengths contract it.
    pub fn spread(mut self, length: impl Into<AbsoluteLength>) -> Self {
        self.spread = length.into();
        self
    }

    /// Casts the shadow into the box.
    pub fn inset(mut self) -> Self {
        self.inset = true;
        self
    }
}

/// A two-dimensional operation in a transform list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Transform {
    /// Translation along both axes; percentages refer to the element's own size.
    Translate(Length, Length),
    /// Independent horizontal and vertical scale factors.
    Scale(f32, f32),
    /// Clockwise rotation in degrees.
    RotateDegrees(f32),
}

/// Values at each corner of a box.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Corners<T> {
    /// Upper left corner.
    pub top_left: T,
    /// Upper right corner.
    pub top_right: T,
    /// Lower right corner.
    pub bottom_right: T,
    /// Lower left corner.
    pub bottom_left: T,
}

impl<T: Copy> Corners<T> {
    /// Assigns the same value to every corner.
    pub const fn all(value: T) -> Self {
        Self {
            top_left: value,
            top_right: value,
            bottom_right: value,
            bottom_left: value,
        }
    }
}

impl<T> Corners<T> {
    /// Transforms each value while preserving its corner.
    pub fn map<U>(self, mut f: impl FnMut(T) -> U) -> Corners<U> {
        Corners {
            top_left: f(self.top_left),
            top_right: f(self.top_right),
            bottom_right: f(self.bottom_right),
            bottom_left: f(self.bottom_left),
        }
    }
}

impl Corners<crate::Pixels> {
    /// Scales all radii proportionally until adjacent corners fit each edge.
    pub fn fit_to(self, size: crate::layout::Size) -> Self {
        let r = self.map(f64::from);
        let horizontal = (r.top_left + r.top_right).max(r.bottom_left + r.bottom_right);
        let vertical = (r.top_left + r.bottom_left).max(r.top_right + r.bottom_right);
        let mut factor = 1.0_f64;
        if horizontal > f64::from(size.width) {
            factor = f64::from(size.width) / horizontal;
        }
        if vertical > f64::from(size.height) {
            factor = factor.min(f64::from(size.height) / vertical);
        }
        if factor == 1.0 {
            self
        } else {
            r.map(|radius| crate::px((radius * factor) as f32))
        }
    }
}
