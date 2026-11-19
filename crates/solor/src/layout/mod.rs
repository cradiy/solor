//! Platform-independent measurement, child placement and atomic geometry publication.
use crate::column;
pub use crate::column::{Column, Edges, Margins};
pub use crate::{Pixels, px};
use crate::{node_store, tree};
const MEASUREMENT_CACHE_SLOTS: usize = 3;

mod engine;
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
