#[cfg(feature = "layout-audit")]
#[path = "validation.rs"]
pub mod validation;

use std::any::Any;
#[cfg(feature = "layout-audit")]
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::rc::Rc;

use super::node_store::{AccessError, Context, NodeStore};
use super::tree::NodeId;

use super::MEASUREMENT_CACHE_SLOTS;

const _: () = assert!(MEASUREMENT_CACHE_SLOTS > 0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutError {
    Access(AccessError),
    InvalidSize,
    /// A node could not produce its measurement or layout output.
    Node(&'static str),
    WrongNodeType,
    MissingChild(usize),
    DuplicatePlacement(usize),
    DuplicateLayout(usize),
    MissingLayout(usize),
    UnexpectedChildCount,
    UnplacedChildren,
    FailedChildOperation,
    StructureLocked,
    RevisionExhausted,
    #[cfg(feature = "layout-audit")]
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

pub type Result<T> = std::result::Result<T, LayoutError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Query {
    Natural,
    AtMost(u32),
    Exact(u32),
}

impl Query {
    pub fn resolve(self, preferred: u32) -> u32 {
        match self {
            Self::Natural => preferred,
            Self::AtMost(limit) => preferred.min(limit),
            Self::Exact(width) => width,
        }
    }

    fn accepts(self, width: u32) -> bool {
        match self {
            Self::Natural => true,
            Self::AtMost(limit) => width <= limit,
            Self::Exact(exact) => width == exact,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Origin {
    pub x: u32,
    pub y: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Metrics {
    pub size: Size,
    pub baseline: Option<u32>,
}

impl Metrics {
    fn validate(self) -> Result<Self> {
        if self.baseline.is_some_and(|value| value > self.size.height) {
            return Err(LayoutError::InvalidSize);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub metrics: Metrics,
    pub origin: Origin,
    pub content: Size,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayoutOutput {
    pub baseline: Option<u32>,
    pub content: Size,
}

impl LayoutOutput {
    fn validate(self) -> Result<Self> {
        if self
            .baseline
            .is_some_and(|value| value > self.content.height)
        {
            return Err(LayoutError::InvalidSize);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub queries: usize,
    pub cache_hits: usize,
    pub measures: usize,
    pub layouts: usize,
}

/// Measures preferred dimensions and arranges direct children within an assigned size.
/// A successful layout must lay out and place each child exactly once.
pub trait LayoutNode: Any {
    fn measure(&mut self, cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics>;
    fn layout(&mut self, cx: &mut LayoutContext<'_, '_>, assigned: Size) -> Result<LayoutOutput>;
}

// A trace is evidence for one result, not a history of all reads by a node.
#[cfg(feature = "layout-audit")]
pub(super) const MAX_RECORDED_READS: usize = 16_384;

#[cfg(feature = "layout-audit")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Read {
    Measure {
        node: NodeId,
        query: Query,
        metrics: Metrics,
    },
    Layout {
        node: NodeId,
        assigned: Size,
        output: LayoutOutput,
    },
}

#[cfg(feature = "layout-audit")]
#[derive(Clone, Debug, PartialEq, Eq)]
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

struct Measurement {
    query: Query,
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

struct Entry {
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
    fn new(body: impl LayoutNode) -> Self {
        Self {
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

pub struct MeasureContext<'a, 'store> {
    access: &'a mut Context<'store, Entry>,
    #[cfg(feature = "layout-audit")]
    reads: &'a mut ReadSet,
    force: bool,
    failed: bool,
}

impl MeasureContext<'_, '_> {
    pub fn child_count(&self) -> usize {
        self.access.child_count()
    }

    pub fn measure_child(&mut self, index: usize, query: Query) -> Result<Metrics> {
        if self.failed {
            return Err(LayoutError::FailedChildOperation);
        }
        self.failed = true;
        let id = self
            .access
            .child(index)
            .ok_or(LayoutError::MissingChild(index))?;
        let metrics = self.access.execute_child(id, |entry, access| {
            Ok(measure(entry, access, query, self.force))
        })??;
        #[cfg(feature = "layout-audit")]
        self.reads.push(Read::Measure {
            node: id,
            query,
            metrics,
        });
        self.failed = false;
        Ok(metrics)
    }
}

fn measure(
    entry: &mut Entry,
    access: &mut Context<'_, Entry>,
    query: Query,
    force: bool,
) -> Result<Metrics> {
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
            if cached.query == query {
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
        access,
        #[cfg(feature = "layout-audit")]
        reads: &mut reads,
        force,
        failed: false,
    };
    let metrics = entry.body.measure(&mut cx, query)?.validate()?;
    if cx.failed {
        return Err(LayoutError::FailedChildOperation);
    }
    if !query.accepts(metrics.size.width) {
        return Err(LayoutError::InvalidSize);
    }
    if let Some(slot) = entry
        .measure_cache
        .iter()
        .position(|slot| slot.as_ref().is_some_and(|cached| cached.query == query))
    {
        entry.measure_cache[slot] = Some(Measurement { query, metrics });
        #[cfg(feature = "layout-audit")]
        entry.set_measurement_reads(slot, reads);
    } else {
        entry.measure_cache[entry.replacement] = Some(Measurement { query, metrics });
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

pub struct LayoutContext<'a, 'store> {
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
    pub fn child_count(&self) -> usize {
        self.access.child_count()
    }

    pub fn measure_child(&mut self, index: usize, query: Query) -> Result<Metrics> {
        if self.failed {
            return Err(LayoutError::FailedChildOperation);
        }
        self.failed = true;
        let metrics = MeasureContext {
            access: self.access,
            #[cfg(feature = "layout-audit")]
            reads: &mut self.reads,
            force: self.force,
            failed: false,
        }
        .measure_child(index, query)?;
        self.failed = false;
        Ok(metrics)
    }

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
        let id = self
            .access
            .child(index)
            .ok_or(LayoutError::MissingChild(index))?;
        let (position, output) = self.access.execute_child(id, |entry, access| {
            Ok(layout(entry, access, assigned, self.scratch, self.force))
        })??;
        // Recursive growth may relocate the vector; retain indexes, never references.
        self.scratch.children[self.children_start + index].draft = Some(position);
        #[cfg(feature = "layout-audit")]
        self.reads.push(Read::Layout {
            node: id,
            assigned,
            output,
        });
        self.failed = false;
        Ok(output)
    }

    pub fn place_child(&mut self, index: usize, origin: Origin) -> Result<()> {
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
            LayoutOutput {
                baseline: geometry.metrics.baseline,
                content: geometry.content,
            },
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
            LayoutOutput {
                baseline: geometry.metrics.baseline,
                content: geometry.content,
            },
        ));
    }
    #[cfg(feature = "layout-audit")]
    let subtree_start = scratch.draft.len();
    entry.layout_dirty = true;
    entry.counts.layouts += 1;
    let children_start = scratch.children.len();
    scratch
        .children
        .resize(children_start + access.child_count(), ChildState::default());
    let mut cx = LayoutContext {
        access,
        scratch,
        children_start,
        #[cfg(feature = "layout-audit")]
        reads: ReadSet::default(),
        force,
        failed: false,
    };
    let output = entry.body.layout(&mut cx, assigned)?.validate()?;
    if cx.failed {
        return Err(LayoutError::FailedChildOperation);
    }
    if cx.scratch.children[children_start..]
        .iter()
        .any(|child| !child.placed)
    {
        return Err(LayoutError::UnplacedChildren);
    }
    let position = cx.scratch.draft.len();
    cx.scratch.draft.push(Draft {
        node: cx.access.current(),
        #[cfg(feature = "layout-audit")]
        reads: Some(std::mem::take(&mut cx.reads)),
        geometry: Geometry {
            metrics: Metrics {
                size: assigned,
                baseline: output.baseline,
            },
            origin: Origin::default(),
            content: output.content,
        },
    });
    #[cfg(feature = "layout-audit")]
    cx.scratch.record_draft(position, Some(subtree_start));
    Ok((position, output))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
                Ok(measure(entry, access, query, force))
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
        let store = NodeStore::new(Box::new(Entry::new(root)), NonZeroUsize::new(64).unwrap());
        let nodes = vec![store.root()];
        Self {
            store,
            scratch: Scratch::default(),
            nodes,
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

    pub fn root(&self) -> NodeId {
        self.store.root()
    }

    /// Release temporary capacity without invalidating measurements or published geometry.
    pub fn release_scratch(&mut self) {
        self.scratch = Scratch::default();
    }

    /// Append during construction, before the first successful root layout.
    pub fn append(&mut self, parent: NodeId, body: impl LayoutNode) -> Result<NodeId> {
        if self.request.is_some() {
            return Err(LayoutError::StructureLocked);
        }
        self.append_entry(parent, body)
    }

    fn append_entry(&mut self, parent: NodeId, body: impl LayoutNode) -> Result<NodeId> {
        self.invalidate(parent)?;
        let node = self
            .store
            .append(parent, Box::new(Entry::new(body)))
            .map_err(AccessError::from)?;
        self.nodes.push(node);
        self.shared_geometry = None;
        Ok(node)
    }

    /// Mount a child at the end of its parent's ordered children.
    pub fn mount(&mut self, parent: NodeId, body: impl LayoutNode) -> Result<NodeId> {
        #[cfg(feature = "layout-audit")]
        let revision = self
            .topology_revision
            .checked_add(1)
            .ok_or(LayoutError::RevisionExhausted)?;
        let node = self.append_entry(parent, body)?;
        #[cfg(feature = "layout-audit")]
        {
            self.topology_revision = revision;
        }
        Ok(node)
    }

    pub fn unmount_leaf(&mut self, node: NodeId) -> Result<()> {
        let parent = self
            .store
            .parent(node)
            .map_err(AccessError::from)?
            .ok_or(AccessError::Tree(super::tree::TreeError::CannotRemoveRoot))?;
        if self.store.child_count(node).map_err(AccessError::from)? != 0 {
            return Err(LayoutError::UnexpectedChildCount);
        }
        #[cfg(feature = "layout-audit")]
        let revision = self
            .topology_revision
            .checked_add(1)
            .ok_or(LayoutError::RevisionExhausted)?;
        self.invalidate(parent)?;
        #[cfg(feature = "layout-audit")]
        {
            self.topology_revision = revision;
        }
        self.shared_geometry = None;
        self.nodes.retain(|&id| id != node);
        self.store.remove_subtree(node).map_err(AccessError::from)?;
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
        self.invalidate(node)?;
        change(self.body_mut::<N>(node)?);
        Ok(())
    }

    /// Read node data for presentation. Layout-affecting changes must use `edit_layout`.
    pub fn body<N: LayoutNode>(&self, node: NodeId) -> Result<&N> {
        (self.store.get(node)?.body.as_ref() as &dyn Any)
            .downcast_ref()
            .ok_or(LayoutError::WrongNodeType)
    }

    pub(super) fn body_mut<N: LayoutNode>(&mut self, node: NodeId) -> Result<&mut N> {
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
        let attempt = Attempt(&mut self.scratch);
        layout_root(&mut self.store, attempt.0, request, force)?;
        let published = publish_layout(&mut self.store, attempt.0);
        drop(attempt);
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

    /// Share the completed layout in surviving insertion order, in parent-local coordinates.
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
                .map(|&id| {
                    self.store
                        .get(id)
                        .expect("live layout node")
                        .geometry
                        .expect("completed layout")
                })
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

    pub fn node_counts(&self, id: NodeId) -> Result<Counts> {
        Ok(self.store.get(id)?.counts)
    }

    /// Read local geometry, or return `None` for an unknown or unpublished node.
    pub fn geometry(&self, id: NodeId) -> Option<Geometry> {
        self.store.get(id).ok()?.geometry
    }

    pub fn is_pending(&self) -> bool {
        self.pending
    }

    /// Query preferred metrics without publishing geometry.
    pub fn probe(&mut self, node: NodeId, query: Query) -> Result<Metrics> {
        self.store.execute(node, |entry, access| {
            Ok(measure(entry, access, query, false))
        })?
    }
}
