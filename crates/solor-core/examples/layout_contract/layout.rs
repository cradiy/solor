use std::any::Any;
use std::num::NonZeroUsize;

use solor_core::tree::NodeId;

use super::MEASUREMENT_CACHE_SLOTS;
use super::store::{AccessError, Context, NodeStore};

const _: () = assert!(MEASUREMENT_CACHE_SLOTS > 0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutError {
    Access(AccessError),
    InvalidSize,
    WrongNodeType,
    MissingChild(usize),
    DuplicatePlacement(usize),
    DuplicateLayout(usize),
    MissingLayout(usize),
    UnexpectedChildCount,
    UnplacedChildren,
    FailedChildOperation,
    StructureLocked,
    #[cfg(test)]
    Injected,
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

pub trait LayoutNode: Any {
    fn measure(&mut self, cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics>;
    fn layout(&mut self, cx: &mut LayoutContext<'_, '_>, assigned: Size) -> Result<LayoutOutput>;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

struct Entry {
    body: Box<dyn LayoutNode>,
    // Cache hits restore metrics, not the behavior's private measurement state.
    measure_cache: [Option<(Query, Metrics)>; MEASUREMENT_CACHE_SLOTS],
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
            measure_cache: [None; MEASUREMENT_CACHE_SLOTS],
            replacement: 0,
            measure_dirty: true,
            layout_dirty: true,
            geometry: None,
            counts: Counts::default(),
        }
    }
}

pub struct MeasureContext<'a, 'store> {
    access: &'a mut Context<'store, Entry>,
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
        entry.measure_cache.fill(None);
        entry.replacement = 0;
    } else if !force {
        for &(cached_query, metrics) in entry.measure_cache.iter().flatten() {
            if cached_query == query {
                entry.counts.cache_hits += 1;
                return Ok(metrics);
            }
        }
    }
    entry.measure_dirty = true;
    entry.counts.measures += 1;
    let mut cx = MeasureContext {
        access,
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
        .iter_mut()
        .find(|slot| slot.is_some_and(|(key, _)| key == query))
    {
        *slot = Some((query, metrics));
    } else {
        entry.measure_cache[entry.replacement] = Some((query, metrics));
        entry.replacement = (entry.replacement + 1) % entry.measure_cache.len();
    }
    entry.measure_dirty = false;
    Ok(metrics)
}

struct Draft {
    node: NodeId,
    geometry: Geometry,
}

#[derive(Clone, Copy, Default)]
struct ChildState {
    draft: Option<usize>,
    placed: bool,
}

pub struct LayoutContext<'a, 'store> {
    access: &'a mut Context<'store, Entry>,
    draft: &'a mut Vec<Draft>,
    children: Vec<ChildState>,
    force: bool,
    failed: bool,
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
        let child = self
            .children
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
            Ok(layout(entry, access, assigned, self.draft, self.force))
        })??;
        child.draft = Some(position);
        self.failed = false;
        Ok(output)
    }

    pub fn place_child(&mut self, index: usize, origin: Origin) -> Result<()> {
        if self.failed {
            return Err(LayoutError::FailedChildOperation);
        }
        self.failed = true;
        let child = self
            .children
            .get_mut(index)
            .ok_or(LayoutError::MissingChild(index))?;
        if child.placed {
            return Err(LayoutError::DuplicatePlacement(index));
        }
        let position = child.draft.ok_or(LayoutError::MissingLayout(index))?;
        self.draft[position].geometry.origin = origin;
        child.placed = true;
        self.failed = false;
        Ok(())
    }
}

fn layout(
    entry: &mut Entry,
    access: &mut Context<'_, Entry>,
    assigned: Size,
    draft: &mut Vec<Draft>,
    force: bool,
) -> Result<(usize, LayoutOutput)> {
    if !force
        && !entry.layout_dirty
        && entry
            .geometry
            .is_some_and(|old| old.metrics.size == assigned)
    {
        let geometry = entry
            .geometry
            .expect("layout reuse requires published geometry");
        let position = draft.len();
        draft.push(Draft {
            node: access.current(),
            geometry,
        });
        return Ok((
            position,
            LayoutOutput {
                baseline: geometry.metrics.baseline,
                content: geometry.content,
            },
        ));
    }
    entry.layout_dirty = true;
    entry.counts.layouts += 1;
    let children = vec![ChildState::default(); access.child_count()];
    let mut cx = LayoutContext {
        access,
        draft,
        children,
        force,
        failed: false,
    };
    let output = entry.body.layout(&mut cx, assigned)?.validate()?;
    if cx.failed {
        return Err(LayoutError::FailedChildOperation);
    }
    if cx.children.iter().any(|child| !child.placed) {
        return Err(LayoutError::UnplacedChildren);
    }
    let position = cx.draft.len();
    cx.draft.push(Draft {
        node: cx.access.current(),
        geometry: Geometry {
            metrics: Metrics {
                size: assigned,
                baseline: output.baseline,
            },
            origin: Origin::default(),
            content: output.content,
        },
    });
    Ok((position, output))
}

pub struct Scene {
    store: NodeStore<Entry>,
    nodes: Vec<NodeId>,
    pending: bool,
    request: Option<(Query, Option<Size>)>,
}

impl Scene {
    pub fn new(root: impl LayoutNode) -> Self {
        let store = NodeStore::new(Box::new(Entry::new(root)), NonZeroUsize::new(64).unwrap());
        let nodes = vec![store.root()];
        Self {
            store,
            nodes,
            pending: true,
            request: None,
        }
    }

    pub fn root(&self) -> NodeId {
        self.store.root()
    }

    pub fn append(&mut self, parent: NodeId, body: impl LayoutNode) -> Result<NodeId> {
        if self.request.is_some() {
            return Err(LayoutError::StructureLocked);
        }
        self.invalidate(parent)?;
        let node = self
            .store
            .append(parent, Box::new(Entry::new(body)))
            .map_err(AccessError::from)?;
        self.nodes.push(node);
        Ok(node)
    }

    fn invalidate(&mut self, node: NodeId) -> Result<()> {
        self.pending = true;
        let mut current = Some(node);
        while let Some(id) = current {
            let entry = self.store.get_mut(id)?;
            entry.measure_dirty = true;
            entry.layout_dirty = true;
            current = self.store.parent(id).map_err(AccessError::from)?;
        }
        Ok(())
    }

    pub fn edit_layout<N: LayoutNode>(
        &mut self,
        node: NodeId,
        change: impl FnOnce(&mut N),
    ) -> Result<()> {
        self.invalidate(node)?;
        change(self.body_mut::<N>(node)?);
        Ok(())
    }

    pub(super) fn body_mut<N: LayoutNode>(&mut self, node: NodeId) -> Result<&mut N> {
        self.store
            .get_mut(node)?
            .body
            .as_any_mut()
            .downcast_mut()
            .ok_or(LayoutError::WrongNodeType)
    }

    pub fn run(&mut self, query: Query, assigned: Option<Size>, force: bool) -> Result<usize> {
        if !self.pending && self.request == Some((query, assigned)) && !force {
            return Ok(0);
        }
        self.pending = true;
        let root = self.store.root();
        let metrics = self.store.execute(root, |entry, access| {
            Ok(measure(entry, access, query, force))
        })??;
        let mut draft = Vec::new();
        let (position, _) = self.store.execute(root, |entry, access| {
            Ok(layout(
                entry,
                access,
                assigned.unwrap_or(metrics.size),
                &mut draft,
                force,
            ))
        })??;
        draft[position].geometry.origin = Origin::default();
        let mut published = 0;
        for change in draft {
            let entry = self
                .store
                .get_mut(change.node)
                .expect("layout cannot remove nodes");
            if entry.geometry != Some(change.geometry) {
                published += 1;
                entry.geometry = Some(change.geometry);
            }
            entry.layout_dirty = false;
        }
        self.request = Some((query, assigned));
        self.pending = false;
        Ok(published)
    }

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

    #[cfg(test)]
    pub fn node_counts(&self, id: NodeId) -> Counts {
        self.store.get(id).unwrap().counts
    }

    #[cfg(test)]
    pub fn geometry(&self, id: NodeId) -> Option<Geometry> {
        self.store.get(id).unwrap().geometry
    }

    #[cfg(test)]
    pub fn is_pending(&self) -> bool {
        self.pending
    }

    #[cfg(test)]
    pub fn probe(&mut self, node: NodeId, query: Query) -> Result<Metrics> {
        self.store.execute(node, |entry, access| {
            Ok(measure(entry, access, query, false))
        })?
    }

    #[cfg(test)]
    pub fn cached_queries(&self, node: NodeId) -> usize {
        let entry = self.store.get(node).unwrap();
        if entry.measure_dirty {
            0
        } else {
            entry.measure_cache.iter().flatten().count()
        }
    }
}
