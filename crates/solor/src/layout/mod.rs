//! Platform-independent measurement, child placement and atomic geometry publication.
use crate::column;
pub use crate::column::{Column, Edges, Margins};
use crate::style::Align;
pub use crate::{Pixels, px};
use crate::{node_store, tree};
const MEASUREMENT_CACHE_SLOTS: usize = 3;

mod engine;
pub(crate) use engine::FlexItemStyle;
pub(crate) use engine::sizing::{
    AxisLimits, ComputedLength, FlexBasis, ItemBasis, ItemSizing, RelativeOffset, WidthLimits,
    border_extent,
};
mod flex;
pub use flex::{FlexDirection, FlexLayout};
mod margin;
#[cfg(feature = "layout-audit")]
pub use engine::validation;
pub use engine::{
    ContainingBlock, Counts, Geometry, LayoutContext, LayoutError, LayoutNode, LayoutOutput,
    MeasureContext, Metrics, Origin, Query, Result, Scene, Size,
};
pub use margin::Margin;
