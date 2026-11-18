//! Platform-independent measurement, child placement and atomic geometry publication.
pub use crate::column::{Column, Edges};
pub use crate::{Pixels, px};
use crate::{node_store, tree};
const MEASUREMENT_CACHE_SLOTS: usize = 3;

mod engine;
#[cfg(feature = "layout-audit")]
pub use engine::validation;
pub use engine::{
    ContainingBlock, Counts, Geometry, LayoutContext, LayoutError, LayoutNode, LayoutOutput,
    MeasureContext, Metrics, Origin, Query, Result, Scene, Size,
};
