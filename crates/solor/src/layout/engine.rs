#[cfg(feature = "layout-audit")]
#[path = "validation.rs"]
/// Dependency capture and verification for layout audits.
pub mod validation;

use std::any::Any;
#[cfg(feature = "layout-audit")]
use std::collections::HashMap;
use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::rc::Rc;

use super::node_store::{AccessError, Context, NodeStore};
use super::tree::NodeId;

#[path = "absolute.rs"]
mod absolute;

#[path = "sizing.rs"]
pub(super) mod sizing;
use sizing::{
    AxisLimits, ComputedLength, Dimension, ItemBasis, ItemSizing, Positioning, PreferredSize,
    SizeRange,
};

use super::Align;
use super::column::Margins;
use super::{MEASUREMENT_CACHE_SLOTS, Pixels};

const _: () = assert!(MEASUREMENT_CACHE_SLOTS > 0);

/// A failed tree operation, measurement, placement or publication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutError {
    /// The node store rejected an access.
    Access(AccessError),
    /// A size, baseline or coordinate exceeded its allowed range.
    InvalidSize,
    /// A node could not produce its measurement or layout output.
    Node(&'static str),
    /// The stored layout body has a different concrete type.
    WrongNodeType,
    /// No direct child exists at the supplied index.
    MissingChild(usize),
    /// The child already has a position in this layout pass.
    DuplicatePlacement(usize),
    /// The child already has an allocation in this layout pass.
    DuplicateLayout(usize),
    /// The child needs an allocation before it can be placed.
    MissingLayout(usize),
    /// The layout body received an unexpected number of children.
    UnexpectedChildCount,
    /// A layout body returned with children still awaiting placement.
    UnplacedChildren,
    /// A previous operation on this context failed.
    FailedChildOperation,
    /// The scene no longer accepts this construction operation.
    StructureLocked,
    /// The revision counter cannot advance.
    RevisionExhausted,
    #[cfg(feature = "layout-audit")]
    /// The checkpoint belongs to another node or scene.
    InvalidCheckpointOwner,
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Access(error) => error.fmt(f),
            Self::InvalidSize => f.write_str("invalid dimensions or baseline"),
            Self::Node(message) => f.write_str(message),
            Self::WrongNodeType => f.write_str("layout node type mismatch"),
            Self::MissingChild(index) => write!(f, "missing child at index {index}"),
            Self::DuplicatePlacement(index) => write!(f, "child {index} was placed twice"),
            Self::DuplicateLayout(index) => write!(f, "child {index} was laid out twice"),
            Self::MissingLayout(index) => write!(f, "child {index} has no layout to place"),
            Self::UnexpectedChildCount => f.write_str("unexpected child count"),
            Self::UnplacedChildren => f.write_str("layout left a child unplaced"),
            Self::FailedChildOperation => f.write_str("an earlier child operation failed"),
            Self::StructureLocked => f.write_str("construction has finished"),
            Self::RevisionExhausted => f.write_str("layout revision exhausted"),
            #[cfg(feature = "layout-audit")]
            Self::InvalidCheckpointOwner => f.write_str("invalid layout checkpoint owner"),
        }
    }
}

impl std::error::Error for LayoutError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Access(error) => Some(error),
            _ => None,
        }
    }
}

impl From<AccessError> for LayoutError {
    fn from(error: AccessError) -> Self {
        Self::Access(error)
    }
}

/// Result of a fallible layout operation.
pub type Result<T> = std::result::Result<T, LayoutError>;

/// Width constraint for a preferred-size measurement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Query {
    /// Measure at the content's preferred width.
    Natural,
    /// Limit the preferred width to the supplied maximum.
    AtMost(Pixels),
    /// Measure at exactly the supplied width.
    Exact(Pixels),
}

impl Query {
    /// Applies this query to a preferred width.
    pub fn resolve(self, preferred: Pixels) -> Pixels {
        match self {
            Self::Natural => preferred,
            Self::AtMost(limit) => preferred.min(limit),
            Self::Exact(width) => width,
        }
    }

    fn validate(self) -> Result<()> {
        match self {
            Self::Natural => Ok(()),
            Self::Exact(v) | Self::AtMost(v) if v.is_extent() => Ok(()),
            _ => Err(LayoutError::InvalidSize),
        }
    }

    fn accepts(self, width: Pixels) -> bool {
        match self {
            Self::Natural => true,
            Self::AtMost(limit) => width <= limit,
            Self::Exact(exact) => width == exact,
        }
    }
}

/// Extent on both axes in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    /// Horizontal extent.
    pub width: Pixels,
    /// Vertical extent.
    pub height: Pixels,
}

/// Definite content dimensions of the containing block during measurement.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ContainingBlock {
    /// Available inline dimension, when definite.
    pub width: Option<Pixels>,
    /// Available block dimension, when definite.
    pub height: Option<Pixels>,
}

impl Size {
    pub(crate) fn validate(self) -> Result<()> {
        if self.width.is_extent() && self.height.is_extent() {
            Ok(())
        } else {
            Err(LayoutError::InvalidSize)
        }
    }
}

impl From<Size> for ContainingBlock {
    fn from(size: Size) -> Self {
        Self {
            width: Some(size.width),
            height: Some(size.height),
        }
    }
}

/// Position relative to the parent's origin in layout units.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Origin {
    /// Horizontal offset.
    pub x: Pixels,
    /// Vertical offset.
    pub y: Pixels,
}

impl Origin {
    pub(crate) fn checked_add(self, offset: Self) -> Option<Self> {
        Some(Self {
            x: self.x.checked_add(offset.x)?,
            y: self.y.checked_add(offset.y)?,
        })
    }
}

/// Measured or assigned size and its optional text baseline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    /// Box extent under the current constraint.
    pub size: Size,
    /// Baseline offset measured from the top of the box.
    pub baseline: Option<Pixels>,
}

impl Metrics {
    /// Checks a measurement before it is used to size an enclosing box.
    pub fn validate_for(self, query: Query) -> Result<Self> {
        self.size.validate()?;
        if !query.accepts(self.size.width)
            || self
                .baseline
                .is_some_and(|value| !value.is_extent() || value > self.size.height)
        {
            return Err(LayoutError::InvalidSize);
        }
        Ok(self)
    }
}

/// Published geometry of a retained layout node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    /// Assigned dimensions and baseline.
    pub metrics: Metrics,
    /// Offset from the parent box.
    pub origin: Origin,
    /// Full local content extent, including overflow clipped by this node.
    pub content: Size,
}

/// Axes on which the element clips overflow at the border box.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct OverflowClip {
    pub(crate) x: bool,
    pub(crate) y: bool,
}

impl Geometry {
    fn parent_output(self, clip: OverflowClip) -> LayoutOutput {
        let mut content = self.content;
        if clip.x {
            content.width = content.width.min(self.metrics.size.width);
        }
        if clip.y {
            content.height = content.height.min(self.metrics.size.height);
        }
        LayoutOutput {
            baseline: self.metrics.baseline,
            content,
        }
    }

    // Shift the placed border box and extend its parent's positive overflow.
    fn shift_and_extend(
        &mut self,
        offset: Origin,
        clip: OverflowClip,
        parent_content: &mut Size,
    ) -> Result<()> {
        fn shift(
            origin: &mut Pixels,
            offset: Pixels,
            extent: Pixels,
            parent: &mut Pixels,
        ) -> Result<()> {
            *origin = origin.checked_add(offset).ok_or(LayoutError::InvalidSize)?;
            let end = origin.checked_add(extent).ok_or(LayoutError::InvalidSize)?;
            *parent = (*parent).max(end);
            Ok(())
        }

        let content = self.parent_output(clip).content;
        shift(
            &mut self.origin.x,
            offset.x,
            self.metrics.size.width.max(content.width),
            &mut parent_content.width,
        )?;
        shift(
            &mut self.origin.y,
            offset.y,
            self.metrics.size.height.max(content.height),
            &mut parent_content.height,
        )
    }
}

/// Content geometry returned after arranging children.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutOutput {
    /// Optional baseline relative to the content origin.
    pub baseline: Option<Pixels>,
    /// Extent occupied by content, including overflow.
    pub content: Size,
}

impl LayoutOutput {
    /// Checks that the baseline fits within the reported content.
    pub fn validate(self) -> Result<Self> {
        self.content.validate()?;
        if self
            .baseline
            .is_some_and(|value| !value.is_extent() || value > self.content.height)
        {
            return Err(LayoutError::InvalidSize);
        }
        Ok(self)
    }
}

/// Cumulative measurement and placement counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Preferred-size queries including cache hits.
    pub queries: usize,
    /// Queries satisfied from the measurement cache.
    pub cache_hits: usize,
    /// Calls to a layout body's measurement method.
    pub measures: usize,
    /// Calls to a layout body's placement method.
    pub layouts: usize,
}

/// Measures preferred dimensions and arranges direct children within an assigned size.
/// A successful layout must lay out and place each child exactly once.
pub trait LayoutNode: Any {
    /// Resolves outer spacing against the parent content width. Intrinsic
    /// measurement supplies no width for resolving percentages.
    fn margins(&self, _inline_width: Option<Pixels>) -> Result<Margins> {
        Ok(Margins::default())
    }

    /// Measures preferred size and baseline under the given width constraint.
    fn measure(&mut self, cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics>;
    /// Assigns each direct child its size and parent-local position.
    fn layout(&mut self, cx: &mut LayoutContext<'_, '_>, assigned: Size) -> Result<LayoutOutput>;
}

// A trace is evidence for one result, not a history of all reads by a node.
#[cfg(feature = "layout-audit")]
pub(super) const MAX_RECORDED_READS: usize = 16_384;

#[cfg(feature = "layout-audit")]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Read {
    Measure {
        node: NodeId,
        query: Query,
        containing_block: ContainingBlock,
        metrics: Metrics,
        margins: Option<Margins>,
        mode: ItemQuery,
    },
    Layout {
        node: NodeId,
        assigned: Size,
        output: LayoutOutput,
    },
}

#[cfg(feature = "layout-audit")]
#[derive(Clone, Debug, PartialEq)]
pub struct ReadSet {
    pub reads: Vec<Read>,
    pub complete: bool,
}

#[cfg(feature = "layout-audit")]
impl Default for ReadSet {
    fn default() -> Self {
        Self {
            reads: Vec::new(),
            complete: true,
        }
    }
}

#[cfg(feature = "layout-audit")]
impl ReadSet {
    fn is_empty(&self) -> bool {
        self.complete && self.reads.is_empty()
    }

    fn push(&mut self, read: Read) {
        if !self.complete {
            return;
        }
        if self.reads.len() == MAX_RECORDED_READS {
            self.reads = Vec::new();
            self.complete = false;
        } else {
            self.reads.push(read);
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
struct MeasureInput {
    query: Query,
    containing_block: ContainingBlock,
    item_basis: ItemBasis,
}

impl MeasureInput {
    fn direct(query: Query) -> Self {
        Self {
            query,
            containing_block: ContainingBlock::default(),
            item_basis: ItemBasis::Preferred,
        }
    }
}

struct Measurement {
    input: MeasureInput,
    metrics: Metrics,
}

#[cfg(feature = "layout-audit")]
struct DependencyRecords {
    measurements: [ReadSet; MEASUREMENT_CACHE_SLOTS],
    layout: ReadSet,
}

#[cfg(feature = "layout-audit")]
impl Default for DependencyRecords {
    fn default() -> Self {
        Self {
            measurements: std::array::from_fn(|_| ReadSet::default()),
            layout: ReadSet::default(),
        }
    }
}

#[cfg(feature = "layout-audit")]
impl DependencyRecords {
    fn is_empty(&self) -> bool {
        self.layout.is_empty() && self.measurements.iter().all(ReadSet::is_empty)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum FlexFactor {
    Grow,
    Shrink,
}

impl FlexFactor {
    pub(crate) const ALL: [Self; 2] = [Self::Grow, Self::Shrink];

    pub(crate) fn slot(self, style: &mut FlexItemStyle) -> &mut f32 {
        match self {
            Self::Grow => &mut style.grow,
            Self::Shrink => &mut style.shrink,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct FlexItemStyle {
    pub(crate) display_none: bool,
    pub(crate) overflow_clip: OverflowClip,
    pub(crate) grow: f32,
    pub(crate) shrink: f32,
    pub(crate) align_self: Align,
    pub(crate) preferred: PreferredSize,
}

impl Default for FlexItemStyle {
    fn default() -> Self {
        Self {
            display_none: false,
            overflow_clip: OverflowClip::default(),
            grow: 0.0,
            shrink: 0.0,
            align_self: Align::Auto,
            preferred: PreferredSize::default(),
        }
    }
}

impl FlexItemStyle {
    pub(super) fn is_flexible(self) -> bool {
        self.grow > 0.0 || self.shrink > 0.0
    }

    pub(crate) fn alignment(self, parent: Align) -> Align {
        if self.align_self == Align::Auto {
            parent
        } else {
            self.align_self
        }
    }
}

pub(crate) struct MeasuredItem {
    pub(crate) metrics: Metrics,
    pub(crate) margins: Margins,
    pub(crate) style: FlexItemStyle,
    pub(super) main_limits: SizeRange,
    pub(super) shrink_inset: Pixels,
    height_limits: Option<AxisLimits>,
}

impl MeasuredItem {
    pub(crate) fn stretch_height(&self, proposed: Pixels, parent_height: Pixels) -> Result<Pixels> {
        match self.height_limits {
            Some(limits) => Ok(limits.resolve(Some(parent_height))?.clamp(proposed)),
            None => Ok(proposed),
        }
    }

    fn stack(self) -> (Metrics, Margins) {
        (self.metrics, self.margins)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ItemQuery {
    Direct,
    Stack,
    Row(Option<Pixels>),
    Column(Align, Option<Pixels>),
}

struct Entry {
    groups: Option<Box<ChildGroups>>,
    // Replaced together with the box inputs during a style update.
    item_style: FlexItemStyle,
    item_sizing: Option<Box<ItemSizing>>,
    body: Box<dyn LayoutNode>,
    // Cache hits restore metrics, not the behavior's private measurement state.
    measure_cache: [Option<Measurement>; MEASUREMENT_CACHE_SLOTS],
    #[cfg(feature = "layout-audit")]
    records: Option<Box<DependencyRecords>>,
    #[cfg(feature = "layout-audit")]
    input_revision: u64,
    replacement: usize,
    measure_dirty: bool,
    layout_dirty: bool,
    geometry: Option<Geometry>,
    counts: Counts,
}

impl Entry {
    fn new(
        body: impl LayoutNode,
        item_style: FlexItemStyle,
        item_sizing: Option<ItemSizing>,
    ) -> Self {
        Self {
            groups: None,
            item_style,
            item_sizing: item_sizing.map(Box::new),
            body: Box::new(body),
            measure_cache: std::array::from_fn(|_| None),
            #[cfg(feature = "layout-audit")]
            records: None,
            #[cfg(feature = "layout-audit")]
            input_revision: 0,
            replacement: 0,
            measure_dirty: true,
            layout_dirty: true,
            geometry: None,
            counts: Counts::default(),
        }
    }

    #[cfg(feature = "layout-audit")]
    fn discard_empty_records(&mut self) {
        if self
            .records
            .as_ref()
            .is_some_and(|records| records.is_empty())
        {
            self.records = None;
        }
    }

    #[cfg(feature = "layout-audit")]
    fn set_measurement_reads(&mut self, slot: usize, reads: ReadSet) {
        if self.records.is_some() || !reads.is_empty() {
            self.records
                .get_or_insert_with(Default::default)
                .measurements[slot] = reads;
            self.discard_empty_records();
        }
    }

    #[cfg(feature = "layout-audit")]
    fn set_layout_reads(&mut self, reads: ReadSet) {
        if self.records.is_some() || !reads.is_empty() {
            self.records.get_or_insert_with(Default::default).layout = reads;
            self.discard_empty_records();
        }
    }
}

struct ChildGroups {
    flow: Vec<NodeId>,
    absolute: Vec<NodeId>,
}

#[derive(Clone, Copy)]
struct FlowChildren<'a>(Option<&'a [NodeId]>);

impl<'a> FlowChildren<'a> {
    fn from_groups(groups: Option<&'a ChildGroups>) -> Self {
        Self(groups.map(|groups| groups.flow.as_slice()))
    }

    fn len(self, access: &Context<'_, Entry>) -> usize {
        self.0.map_or_else(|| access.child_count(), <[_]>::len)
    }

    fn child(self, access: &Context<'_, Entry>, index: usize) -> Result<NodeId> {
        match self.0 {
            Some(children) => children.get(index).copied(),
            None => access.child(index),
        }
        .ok_or(LayoutError::MissingChild(index))
    }
}

/// Access to children during a preferred-size calculation.
pub struct MeasureContext<'a, 'store> {
    pub(crate) preferred_size: PreferredSize,
    pub(crate) height_range: SizeRange,
    pub(crate) item_basis: ItemBasis,
    containing_block: ContainingBlock,
    child_basis: ContainingBlock,
    flow_children: FlowChildren<'a>,
    access: &'a mut Context<'store, Entry>,
    #[cfg(feature = "layout-audit")]
    reads: &'a mut ReadSet,
    force: bool,
    failed: bool,
}

impl MeasureContext<'_, '_> {
    /// Returns the parent content dimensions available for this measurement.
    pub fn containing_block(&self) -> ContainingBlock {
        self.containing_block
    }
    pub(crate) fn definite_content_height(&self) -> Option<Pixels> {
        self.child_basis.height
    }

    /// Supplies this node's content dimensions for its children's preferred-size queries.
    pub fn set_child_containing_block(&mut self, basis: ContainingBlock) {
        self.child_basis = basis;
    }

    /// Returns the number of children participating in layout, in sibling order.
    pub fn child_count(&self) -> usize {
        self.flow_children.len(self.access)
    }

    /// Queries one child's preferred metrics under the supplied constraint.
    pub fn measure_child(&mut self, index: usize, query: Query) -> Result<Metrics> {
        Ok(self.measure_item(index, query, ItemQuery::Direct)?.metrics)
    }

    pub(crate) fn measure_stack_item(
        &mut self,
        index: usize,
        query: Query,
    ) -> Result<(Metrics, Margins)> {
        Ok(self.measure_item(index, query, ItemQuery::Stack)?.stack())
    }

    pub(super) fn measure_item(
        &mut self,
        index: usize,
        query: Query,
        mode: ItemQuery,
    ) -> Result<MeasuredItem> {
        if self.failed {
            return Err(LayoutError::FailedChildOperation);
        }
        self.failed = true;
        let id = self.flow_children.child(self.access, index)?;
        let (result, style, height_limits, main_limits, shrink_inset) =
            self.access.execute_child(id, |entry, access| {
                let (main_limits, shrink_inset) =
                    match (entry.item_style.is_flexible(), mode, &entry.item_sizing) {
                        (true, ItemQuery::Row(_), Some(s)) => (
                            s.width.map(|w| (w, self.child_basis.width)),
                            s.shrink_inset.width,
                        ),
                        (true, ItemQuery::Column(_, _), Some(s)) => (
                            s.height.map(|h| (h, self.child_basis.height)),
                            s.shrink_inset.height,
                        ),
                        _ => (None, Pixels::default()),
                    };
                Ok((
                    measure_item(entry, access, query, self.child_basis, self.force, mode),
                    entry.item_style,
                    entry.item_sizing.as_ref().and_then(|limits| limits.height),
                    main_limits,
                    shrink_inset,
                ))
            })?;
        let (metrics, margins) = result?;
        let main_limits = match main_limits {
            Some((limits, basis)) => limits.resolve(basis)?,
            None => SizeRange::default(),
        };
        #[cfg(feature = "layout-audit")]
        self.reads.push(Read::Measure {
            node: id,
            containing_block: self.child_basis,
            query,
            metrics,
            margins: (!matches!(mode, ItemQuery::Direct)).then_some(margins),
            mode,
        });
        self.failed = false;
        Ok(MeasuredItem {
            metrics,
            margins,
            style,
            height_limits,
            main_limits,
            shrink_inset,
        })
    }
}

fn measure_item(
    entry: &mut Entry,
    access: &mut Context<'_, Entry>,
    query: Query,
    basis: ContainingBlock,
    force: bool,
    mode: ItemQuery,
) -> Result<(Metrics, Margins)> {
    let (query, margins) = if !matches!(mode, ItemQuery::Direct) {
        let margins = entry.body.margins(match query {
            Query::Exact(width) => Some(width),
            _ => None,
        })?;
        let stretch = matches!(mode, ItemQuery::Column(align, _)
            if entry.item_style.preferred.width.is_none() && entry.item_style.alignment(align) == Align::Stretch);
        let query = match query {
            Query::Exact(width) if stretch && !margins.has_auto_horizontal() => {
                Query::Exact(margins.inner_width(width)?)
            }
            Query::Exact(width) | Query::AtMost(width) => {
                Query::AtMost(margins.inner_width(width)?)
            }
            Query::Natural => Query::Natural,
        };
        (query, margins)
    } else {
        (query, Margins::default())
    };
    let item_basis = match mode {
        ItemQuery::Row(Some(width)) => ItemBasis::Width(Some(width)),
        ItemQuery::Column(_, Some(height)) => ItemBasis::Height(Some(height)),
        _ => match entry
            .item_sizing
            .as_ref()
            .and_then(|sizing| sizing.basis.as_ref())
        {
            Some(value) => value.resolve(mode, basis)?,
            None => ItemBasis::Preferred,
        },
    };
    let query = match item_basis {
        ItemBasis::Width(Some(width)) => Query::Exact(width),
        _ if mode == ItemQuery::Row(None) && entry.item_style.shrink > 0.0 => Query::Natural,
        _ => query,
    };
    let input = MeasureInput {
        query,
        containing_block: basis,
        item_basis,
    };
    if let Some(limits) = entry.item_sizing.as_ref().and_then(|v| v.width.as_ref())
        && let Some((bounded, min)) = limits.constrain_width(
            query,
            basis.width,
            entry.item_style.preferred.width,
            mode,
            item_basis,
        )?
    {
        let mut metrics = measure(
            entry,
            access,
            MeasureInput {
                query: bounded,
                ..input
            },
            force,
        )?;
        if !matches!(bounded, Query::Exact(_))
            && (metrics.size.width < min
                || FlowChildren::from_groups(entry.groups.as_deref()).len(access) > 0)
        {
            // Descendant percentages and wrapping need a definite containing width.
            let used = metrics.size.width.max(min);
            metrics = measure(
                entry,
                access,
                MeasureInput {
                    query: Query::Exact(used),
                    ..input
                },
                force,
            )?;
        }
        return Ok((metrics, margins));
    }
    Ok((measure(entry, access, input, force)?, margins))
}

fn measure(
    entry: &mut Entry,
    access: &mut Context<'_, Entry>,
    input: MeasureInput,
    force: bool,
) -> Result<Metrics> {
    let MeasureInput {
        query,
        containing_block,
        item_basis,
    } = input;
    query.validate()?;
    if containing_block
        .width
        .into_iter()
        .chain(containing_block.height)
        .any(|v| !v.is_extent())
    {
        return Err(LayoutError::InvalidSize);
    }
    entry.counts.queries += 1;
    if entry.measure_dirty {
        entry.measure_cache.iter_mut().for_each(|slot| *slot = None);
        #[cfg(feature = "layout-audit")]
        if let Some(records) = entry.records.as_mut() {
            records.measurements = std::array::from_fn(|_| ReadSet::default());
        }
        #[cfg(feature = "layout-audit")]
        entry.discard_empty_records();
        entry.replacement = 0;
    } else if !force {
        for cached in entry.measure_cache.iter().flatten() {
            if cached.input == input {
                entry.counts.cache_hits += 1;
                return Ok(cached.metrics);
            }
        }
    }
    entry.measure_dirty = true;
    entry.counts.measures += 1;
    #[cfg(feature = "layout-audit")]
    let mut reads = ReadSet::default();
    let mut cx = MeasureContext {
        preferred_size: entry.item_style.preferred,
        item_basis,
        height_range: entry
            .item_sizing
            .as_ref()
            .and_then(|v| v.height)
            .map(|limits| limits.resolve(containing_block.height))
            .transpose()?
            .unwrap_or_default(),
        containing_block,
        child_basis: ContainingBlock::default(),
        flow_children: FlowChildren::from_groups(entry.groups.as_deref()),
        access,
        #[cfg(feature = "layout-audit")]
        reads: &mut reads,
        force,
        failed: false,
    };
    let metrics = entry.body.measure(&mut cx, query)?.validate_for(query)?;
    if cx.failed {
        return Err(LayoutError::FailedChildOperation);
    }
    if let Some(slot) = entry
        .measure_cache
        .iter()
        .position(|slot| slot.as_ref().is_some_and(|cached| cached.input == input))
    {
        entry.measure_cache[slot] = Some(Measurement { input, metrics });
        #[cfg(feature = "layout-audit")]
        entry.set_measurement_reads(slot, reads);
    } else {
        entry.measure_cache[entry.replacement] = Some(Measurement { input, metrics });
        #[cfg(feature = "layout-audit")]
        entry.set_measurement_reads(entry.replacement, reads);
        entry.replacement = (entry.replacement + 1) % entry.measure_cache.len();
    }
    entry.measure_dirty = false;
    Ok(metrics)
}

struct Draft {
    node: NodeId,
    geometry: Geometry,
    #[cfg(feature = "layout-audit")]
    reads: Option<ReadSet>,
}

#[derive(Clone, Copy, Default)]
struct ChildState {
    overflow_clip: OverflowClip,
    relative_offset: Origin,
    draft: Option<usize>,
    placed: bool,
}

#[derive(Default)]
struct Scratch {
    draft: Vec<Draft>,
    children: Vec<ChildState>,
    #[cfg(feature = "layout-audit")]
    reuse: Option<StagedLayouts>,
}

#[cfg(feature = "layout-audit")]
#[derive(Default)]
struct StagedLayouts {
    active: bool,
    auditing: bool,
    candidates: HashMap<NodeId, std::ops::Range<usize>>,
    live: Vec<bool>,
    hits: usize,
}

#[cfg(feature = "layout-audit")]
impl Scratch {
    fn record_draft(&mut self, position: usize, subtree_start: Option<usize>) {
        let Some(reuse) = self.reuse.as_mut().filter(|reuse| reuse.active) else {
            return;
        };
        debug_assert_eq!(reuse.live.len(), position);
        reuse.live.push(!reuse.auditing);
        if reuse.auditing
            && let Some(start) = subtree_start
        {
            // Audit execution never reuses staged work, so each subtree is contiguous.
            reuse
                .candidates
                .insert(self.draft[position].node, start..position + 1);
        }
    }
}

// Discard unpublished data on every exit, including unwinding, while retaining capacity.
struct Attempt<'a>(&'a mut Scratch);

impl Drop for Attempt<'_> {
    fn drop(&mut self) {
        self.0.draft.clear();
        self.0.children.clear();
        #[cfg(feature = "layout-audit")]
        if let Some(reuse) = &mut self.0.reuse {
            reuse.active = false;
            reuse.auditing = false;
            reuse.candidates.clear();
            reuse.live.clear();
            reuse.hits = 0;
        }
    }
}

/// Access to measurement and staged placement for the current node's children.
pub struct LayoutContext<'a, 'store> {
    pub(crate) padding_box: (Origin, Size),
    child_basis: ContainingBlock,
    flow_children: FlowChildren<'a>,
    access: &'a mut Context<'store, Entry>,
    scratch: &'a mut Scratch,
    children_start: usize,
    #[cfg(feature = "layout-audit")]
    reads: ReadSet,
    force: bool,
    failed: bool,
}

impl Drop for LayoutContext<'_, '_> {
    fn drop(&mut self) {
        self.scratch.children.truncate(self.children_start);
    }
}

impl LayoutContext<'_, '_> {
    /// Sets the parent content dimensions for subsequent child measurements.
    pub fn set_child_containing_block(&mut self, basis: ContainingBlock) {
        self.child_basis = basis;
    }

    /// Returns the number of children participating in this layout.
    pub fn child_count(&self) -> usize {
        self.flow_children.len(self.access)
    }

    /// Queries a child's preferred metrics before assigning its box.
    pub fn measure_child(&mut self, index: usize, query: Query) -> Result<Metrics> {
        Ok(self.measure_item(index, query, ItemQuery::Direct)?.metrics)
    }

    pub(crate) fn measure_stack_item(
        &mut self,
        index: usize,
        query: Query,
    ) -> Result<(Metrics, Margins)> {
        Ok(self.measure_item(index, query, ItemQuery::Stack)?.stack())
    }

    pub(super) fn measure_item(
        &mut self,
        index: usize,
        query: Query,
        mode: ItemQuery,
    ) -> Result<MeasuredItem> {
        if self.failed {
            return Err(LayoutError::FailedChildOperation);
        }
        self.failed = true;
        let metrics = MeasureContext {
            preferred_size: PreferredSize::default(),
            height_range: SizeRange::default(),
            item_basis: ItemBasis::Preferred,
            containing_block: ContainingBlock::default(),
            child_basis: self.child_basis,
            flow_children: self.flow_children,
            access: self.access,
            #[cfg(feature = "layout-audit")]
            reads: &mut self.reads,
            force: self.force,
            failed: false,
        }
        .measure_item(index, query, mode)?;
        self.failed = false;
        Ok(metrics)
    }

    /// Arranges a child and returns the content extent exposed to its parent on each axis.
    pub fn layout_child(&mut self, index: usize, assigned: Size) -> Result<LayoutOutput> {
        if self.failed {
            return Err(LayoutError::FailedChildOperation);
        }
        self.failed = true;
        let child = self.scratch.children[self.children_start..]
            .get_mut(index)
            .ok_or(LayoutError::MissingChild(index))?;
        if child.draft.is_some() {
            return Err(LayoutError::DuplicateLayout(index));
        }
        let id = self.flow_children.child(self.access, index)?;
        let (result, relative_offset, clip) = self.access.execute_child(id, |entry, access| {
            let offset = entry
                .item_sizing
                .as_ref()
                .and_then(|s| match &s.position {
                    Positioning::Relative(offset) => Some(offset),
                    _ => None,
                })
                .map_or_else(Origin::default, |offset| offset.resolve(self.child_basis));
            Ok((
                layout(entry, access, assigned, self.scratch, self.force),
                offset,
                entry.item_style.overflow_clip,
            ))
        })?;
        let (position, output) = result?;
        // Recursive growth may relocate the vector; retain indexes, never references.
        let child = &mut self.scratch.children[self.children_start + index];
        child.draft = Some(position);
        child.relative_offset = relative_offset;
        child.overflow_clip = clip;
        #[cfg(feature = "layout-audit")]
        self.reads.push(Read::Layout {
            node: id,
            assigned,
            output,
        });
        self.failed = false;
        Ok(output)
    }

    /// Translates the direct children placed by a content layout into its enclosing box.
    pub fn offset_children(&mut self, by: Origin) -> Result<()> {
        if self.failed {
            return Err(LayoutError::FailedChildOperation);
        }
        self.failed = true;
        for child in &self.scratch.children[self.children_start..] {
            if !child.placed {
                return Err(LayoutError::UnplacedChildren);
            }
            let position = child.draft.ok_or(LayoutError::UnplacedChildren)?;
            let origin = &mut self.scratch.draft[position].geometry.origin;
            origin.x = origin.x.checked_add(by.x).ok_or(LayoutError::InvalidSize)?;
            origin.y = origin.y.checked_add(by.y).ok_or(LayoutError::InvalidSize)?;
        }
        self.failed = false;
        Ok(())
    }

    /// Sets the origin of an already arranged child.
    pub fn place_child(&mut self, index: usize, origin: Origin) -> Result<()> {
        if !origin.x.get().is_finite() || !origin.y.get().is_finite() {
            self.failed = true;
            return Err(LayoutError::InvalidSize);
        }
        if self.failed {
            return Err(LayoutError::FailedChildOperation);
        }
        self.failed = true;
        let child = self.scratch.children[self.children_start..]
            .get_mut(index)
            .ok_or(LayoutError::MissingChild(index))?;
        if child.placed {
            return Err(LayoutError::DuplicatePlacement(index));
        }
        let position = child.draft.ok_or(LayoutError::MissingLayout(index))?;
        self.scratch.draft[position].geometry.origin = origin;
        child.placed = true;
        self.failed = false;
        Ok(())
    }
}

fn layout(
    entry: &mut Entry,
    access: &mut Context<'_, Entry>,
    assigned: Size,
    scratch: &mut Scratch,
    force: bool,
) -> Result<(usize, LayoutOutput)> {
    assigned.validate()?;
    #[cfg(feature = "layout-audit")]
    if !force
        && entry.layout_dirty
        && let Some(reuse) = scratch
            .reuse
            .as_mut()
            .filter(|reuse| reuse.active && !reuse.auditing)
        && let Some(range) = reuse.candidates.get(&access.current())
        && scratch.draft[range.end - 1].geometry.metrics.size == assigned
    {
        reuse.hits += 1;
        reuse.live[range.clone()].fill(true);
        let position = range.end - 1;
        let geometry = scratch.draft[position].geometry;
        return Ok((
            position,
            geometry.parent_output(entry.item_style.overflow_clip),
        ));
    }
    if !force
        && !entry.layout_dirty
        && entry
            .geometry
            .is_some_and(|old| old.metrics.size == assigned)
    {
        let geometry = entry
            .geometry
            .expect("layout reuse requires published geometry");
        let position = scratch.draft.len();
        scratch.draft.push(Draft {
            node: access.current(),
            geometry,
            #[cfg(feature = "layout-audit")]
            reads: None,
        });
        #[cfg(feature = "layout-audit")]
        scratch.record_draft(position, None);
        return Ok((
            position,
            geometry.parent_output(entry.item_style.overflow_clip),
        ));
    }
    #[cfg(feature = "layout-audit")]
    let subtree_start = scratch.draft.len();
    entry.layout_dirty = true;
    entry.counts.layouts += 1;
    let children_start = scratch.children.len();
    let flow_children = FlowChildren::from_groups(entry.groups.as_deref());
    let count = flow_children.len(access);
    scratch
        .children
        .resize(children_start + count, ChildState::default());
    let mut cx = LayoutContext {
        padding_box: (Origin::default(), assigned),
        child_basis: ContainingBlock::default(),
        access,
        flow_children,
        scratch,
        children_start,
        #[cfg(feature = "layout-audit")]
        reads: ReadSet::default(),
        force,
        failed: false,
    };
    let mut output = entry.body.layout(&mut cx, assigned)?.validate()?;
    if cx.failed {
        return Err(LayoutError::FailedChildOperation);
    }
    for child in &cx.scratch.children[children_start..] {
        if !child.placed {
            return Err(LayoutError::UnplacedChildren);
        }
        if child.relative_offset != Origin::default() {
            let position = child.draft.ok_or(LayoutError::UnplacedChildren)?;
            let geometry = &mut cx.scratch.draft[position].geometry;
            geometry.shift_and_extend(
                child.relative_offset,
                child.overflow_clip,
                &mut output.content,
            )?;
        }
    }
    if let Some(groups) = &entry.groups {
        for &child in &groups.absolute {
            cx.access.execute_child(child, |entry, access| {
                Ok(absolute::place(
                    entry,
                    access,
                    cx.padding_box,
                    cx.scratch,
                    force,
                    &mut output.content,
                ))
            })??;
        }
        #[cfg(feature = "layout-audit")]
        if !groups.absolute.is_empty() {
            // Absolute placement also reads the child's insets and margins.
            cx.reads.complete = false;
        }
    }
    let position = cx.scratch.draft.len();
    let geometry = Geometry {
        metrics: Metrics {
            size: assigned,
            baseline: output.baseline,
        },
        origin: Origin::default(),
        content: output.content,
    };
    cx.scratch.draft.push(Draft {
        node: cx.access.current(),
        #[cfg(feature = "layout-audit")]
        reads: Some(std::mem::take(&mut cx.reads)),
        geometry,
    });
    #[cfg(feature = "layout-audit")]
    cx.scratch.record_draft(position, Some(subtree_start));
    Ok((
        position,
        geometry.parent_output(entry.item_style.overflow_clip),
    ))
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum RootRequest {
    Measure(Query, Option<Size>),
    Assigned(Size),
}

fn layout_root(
    store: &mut NodeStore<Entry>,
    scratch: &mut Scratch,
    request: RootRequest,
    force: bool,
) -> Result<usize> {
    let root = store.root();
    let size = match request {
        RootRequest::Measure(query, assigned) => {
            let metrics = store.execute(root, |entry, access| {
                Ok(measure(entry, access, MeasureInput::direct(query), force))
            })??;
            assigned.unwrap_or(metrics.size)
        }
        RootRequest::Assigned(size) => size,
    };
    let (position, _) = store.execute(root, |entry, access| {
        Ok(layout(entry, access, size, scratch, force))
    })??;
    scratch.draft[position].geometry.origin = Origin::default();
    Ok(position)
}

fn publish_layout(store: &mut NodeStore<Entry>, scratch: &mut Scratch) -> usize {
    fn publish(store: &mut NodeStore<Entry>, change: &mut Draft) -> usize {
        let entry = store
            .get_mut(change.node)
            .expect("layout cannot remove nodes");
        let changed = usize::from(entry.geometry != Some(change.geometry));
        if changed != 0 {
            entry.geometry = Some(change.geometry);
        }
        entry.layout_dirty = false;
        #[cfg(feature = "layout-audit")]
        if let Some(reads) = change.reads.take() {
            entry.set_layout_reads(reads);
        }
        changed
    }
    #[cfg(feature = "layout-audit")]
    if let Some(reuse) = scratch.reuse.as_ref().filter(|reuse| reuse.active) {
        return scratch
            .draft
            .iter_mut()
            .zip(&reuse.live)
            .filter(|(_, live)| **live)
            .map(|(draft, _)| publish(store, draft))
            .sum();
    }
    scratch
        .draft
        .iter_mut()
        .map(|draft| publish(store, draft))
        .sum()
}

/// Owns the ordered layout tree, measurement caches and last published geometry.
pub struct Scene {
    store: NodeStore<Entry>,
    scratch: Scratch,
    nodes: Vec<NodeId>,
    flow_updates: HashSet<NodeId>,
    shared_geometry: Option<Rc<[Geometry]>>,
    pending: bool,
    request: Option<RootRequest>,
    #[cfg(feature = "layout-audit")]
    changes: Option<validation::ChangeLog>,
    #[cfg(feature = "layout-audit")]
    publication: u64,
    #[cfg(feature = "layout-audit")]
    topology_revision: u64,
}

impl Scene {
    /// Create a scene with a three-query measurement cache and a maximum callback depth of 64.
    pub fn new(root: impl LayoutNode) -> Self {
        Self::with_root_style(root, FlexItemStyle::default(), None)
    }

    pub(crate) fn with_root_style(
        root: impl LayoutNode,
        style: FlexItemStyle,
        sizing: Option<ItemSizing>,
    ) -> Self {
        let store = NodeStore::new(
            Box::new(Entry::new(root, style, sizing)),
            NonZeroUsize::new(64).unwrap(),
        );
        let nodes = vec![store.root()];
        Self {
            store,
            scratch: Scratch::default(),
            nodes,
            flow_updates: HashSet::new(),
            shared_geometry: None,
            pending: true,
            request: None,
            #[cfg(feature = "layout-audit")]
            changes: None,
            #[cfg(feature = "layout-audit")]
            publication: 0,
            #[cfg(feature = "layout-audit")]
            topology_revision: 0,
        }
    }

    /// Returns the root node of this scene.
    pub fn root(&self) -> NodeId {
        self.store.root()
    }

    /// Release temporary capacity without invalidating measurements or published geometry.
    pub fn release_scratch(&mut self) {
        self.scratch = Scratch::default();
    }

    pub(crate) fn parent(&self, node: NodeId) -> Result<Option<NodeId>> {
        Ok(self.store.parent(node).map_err(AccessError::from)?)
    }

    /// Returns the live children in layout order.
    pub fn children(
        &self,
        parent: NodeId,
    ) -> Result<impl DoubleEndedIterator<Item = NodeId> + ExactSizeIterator + '_> {
        Ok(self.store.children(parent).map_err(AccessError::from)?)
    }

    pub(crate) fn sort_children_by_key(
        &mut self,
        parent: NodeId,
        key: impl Fn(NodeId) -> usize,
    ) -> Result<()> {
        #[cfg(feature = "layout-audit")]
        let revision = self
            .topology_revision
            .checked_add(1)
            .ok_or(LayoutError::RevisionExhausted)?;
        self.flow_changed(parent, false)?;
        self.invalidate(parent)?;
        self.store
            .sort_children_by_key(parent, key)
            .map_err(AccessError::from)?;
        #[cfg(feature = "layout-audit")]
        {
            self.topology_revision = revision;
        }
        Ok(())
    }

    /// Append during construction, before the first successful root layout.
    pub fn append(&mut self, parent: NodeId, body: impl LayoutNode) -> Result<NodeId> {
        if self.request.is_some() {
            return Err(LayoutError::StructureLocked);
        }
        self.insert_entry(
            parent,
            None,
            Entry::new(body, FlexItemStyle::default(), None),
        )
    }

    fn insert_entry(
        &mut self,
        parent: NodeId,
        index: Option<usize>,
        entry: Entry,
    ) -> Result<NodeId> {
        let out_of_flow =
            entry.item_style.display_none || ItemSizing::is_absolute(entry.item_sizing.as_deref());
        let entry = Box::new(entry);
        let node = match index {
            Some(index) => self.store.insert(parent, index, entry),
            None => self.store.append(parent, entry),
        }
        .map_err(AccessError::from)?;
        self.invalidate(parent)?;
        self.flow_changed(parent, out_of_flow)?;
        self.nodes.push(node);
        self.shared_geometry = None;
        Ok(node)
    }

    /// Mount a child at the end of its parent's ordered children.
    pub fn mount(&mut self, parent: NodeId, body: impl LayoutNode) -> Result<NodeId> {
        self.mount_entry(parent, None, body, FlexItemStyle::default(), None)
    }

    /// Mount at a live child index or at the end. Existing siblings retain their handles.
    pub fn mount_at(
        &mut self,
        parent: NodeId,
        index: usize,
        body: impl LayoutNode,
    ) -> Result<NodeId> {
        self.mount_entry(parent, Some(index), body, FlexItemStyle::default(), None)
    }

    pub(crate) fn mount_entry(
        &mut self,
        parent: NodeId,
        index: Option<usize>,
        body: impl LayoutNode,
        item_style: FlexItemStyle,
        item_sizing: Option<ItemSizing>,
    ) -> Result<NodeId> {
        #[cfg(feature = "layout-audit")]
        let revision = self
            .topology_revision
            .checked_add(1)
            .ok_or(LayoutError::RevisionExhausted)?;
        let node = self.insert_entry(parent, index, Entry::new(body, item_style, item_sizing))?;
        #[cfg(feature = "layout-audit")]
        {
            self.topology_revision = revision;
        }
        Ok(node)
    }

    /// Removes a childless node and invalidates its parent.
    pub fn unmount_leaf(&mut self, node: NodeId) -> Result<()> {
        self.remove_mounted(node, true)
    }

    /// Remove a mounted subtree and invalidate its former parent.
    pub fn unmount_subtree(&mut self, node: NodeId) -> Result<()> {
        self.remove_mounted(node, false)
    }

    fn remove_mounted(&mut self, node: NodeId, leaf_only: bool) -> Result<()> {
        let parent = self
            .store
            .parent(node)
            .map_err(AccessError::from)?
            .ok_or(AccessError::Tree(super::tree::TreeError::CannotRemoveRoot))?;
        let leaf = self.store.child_count(node).map_err(AccessError::from)? == 0;
        if leaf_only && !leaf {
            return Err(LayoutError::UnexpectedChildCount);
        }
        #[cfg(feature = "layout-audit")]
        let revision = self
            .topology_revision
            .checked_add(1)
            .ok_or(LayoutError::RevisionExhausted)?;
        self.flow_changed(parent, false)?;
        self.invalidate(parent)?;
        #[cfg(feature = "layout-audit")]
        {
            self.topology_revision = revision;
        }
        self.shared_geometry = None;
        if leaf {
            self.nodes.retain(|&id| id != node);
            self.store.remove_subtree(node).map_err(AccessError::from)?;
        } else {
            // A payload destructor can unwind after only part of the subtree is retired.
            struct PruneRemoved<'a> {
                nodes: &'a mut Vec<NodeId>,
                store: &'a mut NodeStore<Entry>,
            }
            impl Drop for PruneRemoved<'_> {
                fn drop(&mut self) {
                    let interrupted = std::thread::panicking();
                    self.nodes.retain(|&id| {
                        let Ok(entry) = self.store.get_mut(id) else {
                            return false;
                        };
                        if interrupted {
                            entry.measure_dirty = true;
                            entry.layout_dirty = true;
                        }
                        true
                    });
                }
            }
            let removal = PruneRemoved {
                nodes: &mut self.nodes,
                store: &mut self.store,
            };
            removal
                .store
                .remove_subtree(node)
                .map_err(AccessError::from)?;
        }
        Ok(())
    }

    fn invalidate(&mut self, node: NodeId) -> Result<()> {
        let mut current = Some(node);
        while let Some(id) = current {
            let entry = self.store.get_mut(id)?;
            entry.measure_dirty = true;
            entry.layout_dirty = true;
            current = self.store.parent(id).map_err(AccessError::from)?;
        }
        self.pending = true;
        Ok(())
    }

    /// Invalidate the node and its ancestors before mutating a layout input.
    pub fn edit_layout<N: LayoutNode>(
        &mut self,
        node: NodeId,
        change: impl FnOnce(&mut N),
    ) -> Result<()> {
        self.body::<N>(node)?;
        self.invalidate_input(node)?;
        change(self.body_mut::<N>(node)?);
        Ok(())
    }

    fn invalidate_input(&mut self, node: NodeId) -> Result<()> {
        #[cfg(feature = "layout-audit")]
        {
            let entry = self.store.get_mut(node)?;
            let revision = entry
                .input_revision
                .checked_add(1)
                .ok_or(LayoutError::RevisionExhausted)?;
            if let Some(changes) = &mut self.changes {
                changes.record(node)?;
            }
            entry.input_revision = revision;
        }
        self.invalidate(node)
    }

    pub(crate) fn dimension(
        &self,
        node: NodeId,
        dimension: Dimension,
    ) -> Result<Option<ComputedLength>> {
        let entry = self.store.get(node)?;
        Ok(dimension.value(entry.item_style.preferred, entry.item_sizing.as_deref()))
    }

    pub(crate) fn set_dimension(
        &mut self,
        node: NodeId,
        dimension: Dimension,
        value: ComputedLength,
    ) -> Result<()> {
        if self.dimension(node, dimension)? != Some(value) {
            self.invalidate_input(node)?;
            let entry = self.store.get_mut(node)?;
            let slot = dimension
                .value_mut(
                    &mut entry.item_style.preferred,
                    entry.item_sizing.as_deref_mut(),
                )
                .expect("active transition has a declared dimension");
            *slot = value;
        }
        Ok(())
    }

    pub(crate) fn flex_factor(&self, node: NodeId, factor: FlexFactor) -> Result<f32> {
        let style = &self.store.get(node)?.item_style;
        Ok(match factor {
            FlexFactor::Grow => style.grow,
            FlexFactor::Shrink => style.shrink,
        })
    }

    pub(crate) fn set_flex_factor(
        &mut self,
        node: NodeId,
        factor: FlexFactor,
        value: f32,
    ) -> Result<()> {
        if self.flex_factor(node, factor)? != value {
            self.invalidate_input(node)?;
            *factor.slot(&mut self.store.get_mut(node)?.item_style) = value;
        }
        Ok(())
    }

    pub(crate) fn set_item_insets(
        &mut self,
        node: NodeId,
        insets: Size,
        extra: Size,
    ) -> Result<()> {
        if let Some(sizing) = self.store.get_mut(node)?.item_sizing.as_deref_mut() {
            sizing.shrink_inset = insets;
            if let Some(basis) = &mut sizing.basis {
                basis.extra = extra;
            }
            if let Some(width) = &mut sizing.width {
                width.extra = extra.width;
            }
            if let Some(height) = &mut sizing.height {
                height.extra = extra.height;
            }
        }
        Ok(())
    }

    pub(crate) fn set_item_style(
        &mut self,
        node: NodeId,
        style: FlexItemStyle,
        sizing: Option<ItemSizing>,
    ) -> Result<()> {
        let entry = self.store.get_mut(node)?;
        let flow_changed = entry.item_style.display_none != style.display_none
            || ItemSizing::is_absolute(entry.item_sizing.as_deref())
                != ItemSizing::is_absolute(sizing.as_ref());
        entry.item_style = style;
        entry.item_sizing = sizing.map(Box::new);
        if flow_changed {
            if let Some(parent) = self.parent(node)? {
                self.flow_changed(parent, true)?;
            }
            #[cfg(feature = "layout-audit")]
            {
                self.topology_revision = self
                    .topology_revision
                    .checked_add(1)
                    .ok_or(LayoutError::RevisionExhausted)?;
            }
        }
        Ok(())
    }

    fn flow_changed(&mut self, parent: NodeId, membership_changed: bool) -> Result<()> {
        if membership_changed || self.store.get(parent)?.groups.is_some() {
            self.flow_updates.insert(parent);
        }
        Ok(())
    }

    // Rebuild once for a batch of mounts, reorders or display changes.
    // Parents with all children in flow continue indexing the tree directly.
    fn prepare_flow(&mut self) -> Result<Vec<NodeId>> {
        let mut hidden = Vec::new();
        for &parent in &self.flow_updates {
            if self.store.get(parent).is_err() {
                continue;
            }
            let start = hidden.len();
            let mut flow = Vec::new();
            let mut absolute = Vec::new();
            for id in self.store.children(parent).map_err(AccessError::from)? {
                let child = self.store.get(id)?;
                if child.item_style.display_none {
                    hidden.push(id);
                } else if ItemSizing::is_absolute(child.item_sizing.as_deref()) {
                    absolute.push(id);
                } else {
                    flow.push(id);
                }
            }
            self.store.get_mut(parent)?.groups = (hidden.len() != start || !absolute.is_empty())
                .then(|| Box::new(ChildGroups { flow, absolute }));
        }
        Ok(hidden)
    }

    /// Read node data for presentation. Layout-affecting changes must use `edit_layout`.
    pub fn body<N: LayoutNode>(&self, node: NodeId) -> Result<&N> {
        (self.store.get(node)?.body.as_ref() as &dyn Any)
            .downcast_ref()
            .ok_or(LayoutError::WrongNodeType)
    }

    /// Edits content without invalidating its layout. Use only for paint-only changes.
    pub(crate) fn body_mut<N: LayoutNode>(&mut self, node: NodeId) -> Result<&mut N> {
        (self.store.get_mut(node)?.body.as_mut() as &mut dyn Any)
            .downcast_mut()
            .ok_or(LayoutError::WrongNodeType)
    }

    /// Measure and arrange the root, then publish all geometry together. Returns the
    /// number of changed entries. Force bypasses cached measurements and placements.
    pub fn run(&mut self, query: Query, assigned: Option<Size>, force: bool) -> Result<usize> {
        self.run_request(RootRequest::Measure(query, assigned), force)
    }

    /// Place the root at the host allocation without requesting its preferred size.
    pub fn run_assigned(&mut self, size: Size, force: bool) -> Result<usize> {
        self.run_request(RootRequest::Assigned(size), force)
    }

    fn run_request(&mut self, request: RootRequest, force: bool) -> Result<usize> {
        if !self.pending && self.request == Some(request) && !force {
            return Ok(0);
        }
        #[cfg(feature = "layout-audit")]
        let publication = self
            .publication
            .checked_add(1)
            .ok_or(LayoutError::RevisionExhausted)?;
        self.pending = true;
        let mut hidden = self.prepare_flow()?;
        let root = self.root();
        let attempt = Attempt(&mut self.scratch);
        if self.store.get(root)?.item_style.display_none {
            match request {
                RootRequest::Measure(query, size) => {
                    query.validate()?;
                    if let Some(size) = size {
                        size.validate()?;
                    }
                }
                RootRequest::Assigned(size) => size.validate()?,
            }
            hidden.push(root);
        } else {
            layout_root(&mut self.store, attempt.0, request, force)?;
        }
        let mut published = publish_layout(&mut self.store, attempt.0);
        drop(attempt);
        while let Some(id) = hidden.pop() {
            if self.store.get_mut(id)?.geometry.take().is_some() {
                published += 1;
                hidden.extend(self.store.children(id).map_err(AccessError::from)?);
            }
        }
        self.flow_updates.clear();
        self.finish_layout(request, published);
        #[cfg(feature = "layout-audit")]
        {
            self.publication = publication;
        }
        Ok(published)
    }

    fn finish_layout(&mut self, request: RootRequest, changed: usize) {
        if changed != 0 {
            self.shared_geometry = None;
        }
        self.request = Some(request);
        self.pending = false;
    }

    /// Share the geometry of participating nodes in insertion order, in parent-local coordinates.
    /// Returns `None` before the first layout or while an update is pending.
    /// The first request copies the geometry. Further requests share that allocation
    /// until geometry or membership changes. Held snapshots survive edits and scene drop.
    pub fn shared_snapshot(&mut self) -> Option<Rc<[Geometry]>> {
        if self.pending {
            return None;
        }
        let geometry = self.shared_geometry.get_or_insert_with(|| {
            self.nodes
                .iter()
                .filter_map(|&id| self.store.get(id).expect("live layout node").geometry)
                .collect()
        });
        Some(Rc::clone(geometry))
    }

    /// Copy the last published local geometry in surviving insertion order.
    pub fn snapshot(&self) -> Vec<Option<Geometry>> {
        self.nodes
            .iter()
            .map(|&id| self.store.get(id).unwrap().geometry)
            .collect()
    }

    /// Sums the counters across live nodes.
    pub fn counts(&self) -> Counts {
        self.nodes.iter().fold(Counts::default(), |mut total, &id| {
            let counts = self.store.get(id).unwrap().counts;
            total.queries += counts.queries;
            total.cache_hits += counts.cache_hits;
            total.measures += counts.measures;
            total.layouts += counts.layouts;
            total
        })
    }

    /// Counts retained evidence independently of measurement/layout execution work.
    #[cfg(feature = "layout-audit")]
    pub fn dependency_stats(&self) -> (usize, usize, usize) {
        let mut stats = (0, 0, 0);
        for &id in &self.nodes {
            let entry = self.store.get(id).unwrap();
            let Some(records) = entry.records.as_ref() else {
                continue;
            };
            for reads in records
                .measurements
                .iter()
                .chain(std::iter::once(&records.layout))
            {
                stats.0 += reads.reads.len();
                stats.1 += reads.reads.capacity() * std::mem::size_of::<Read>();
                stats.2 += usize::from(!reads.complete);
            }
        }
        stats
    }

    /// Returns the counters for one live node.
    pub fn node_counts(&self, id: NodeId) -> Result<Counts> {
        Ok(self.store.get(id)?.counts)
    }

    /// Read local geometry, or return `None` for an unknown or unpublished node.
    pub fn geometry(&self, id: NodeId) -> Option<Geometry> {
        self.store.get(id).ok()?.geometry
    }

    /// Reports whether layout publication is pending.
    pub fn is_pending(&self) -> bool {
        self.pending
    }

    /// Query preferred metrics without publishing geometry.
    pub fn probe(&mut self, node: NodeId, query: Query) -> Result<Metrics> {
        self.prepare_flow()?;
        self.store.execute(node, |entry, access| {
            Ok(measure(entry, access, MeasureInput::direct(query), false))
        })?
    }
}
