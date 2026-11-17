use std::any::Any;
use std::num::NonZeroUsize;

use solor_core::tree::NodeId;

use super::store::{AccessError, Context, NodeStore};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutError {
    Access(AccessError),
    InvalidSize,
    WrongNodeType,
    MissingChild(usize),
    DuplicatePlacement(usize),
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
pub struct Query {
    width: u32,
}

impl Query {
    pub fn new(width: u32) -> Result<Self> {
        if width == 0 {
            return Err(LayoutError::InvalidSize);
        }
        Ok(Self { width })
    }

    pub fn width(self) -> u32 {
        self.width
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
        if self.size.width == 0 || self.baseline.is_some_and(|value| value > self.size.height) {
            return Err(LayoutError::InvalidSize);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub metrics: Metrics,
    pub origin: Origin,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub measures: usize,
    pub layouts: usize,
}

pub trait LayoutNode: Any {
    fn measure(&mut self, cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics>;
    fn layout(&mut self, cx: &mut LayoutContext<'_, '_>, assigned: Size) -> Result<()>;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

struct Entry {
    body: Box<dyn LayoutNode>,
    measure_cache: Option<(Query, Metrics)>,
    measure_dirty: bool,
    layout_dirty: bool,
    geometry: Option<Geometry>,
    counts: Counts,
}

impl Entry {
    fn new(body: impl LayoutNode) -> Self {
        Self {
            body: Box::new(body),
            measure_cache: None,
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
    if !force
        && !entry.measure_dirty
        && let Some((cached_query, metrics)) = entry.measure_cache
        && cached_query == query
    {
        return Ok(metrics);
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
    if metrics.size.width > query.width {
        return Err(LayoutError::InvalidSize);
    }
    entry.measure_cache = Some((query, metrics));
    entry.measure_dirty = false;
    Ok(metrics)
}

struct Draft {
    node: NodeId,
    geometry: Geometry,
}

pub struct LayoutContext<'a, 'store> {
    access: &'a mut Context<'store, Entry>,
    draft: &'a mut Vec<Draft>,
    placed: Vec<bool>,
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

    pub fn place_child(&mut self, index: usize, metrics: Metrics, origin: Origin) -> Result<()> {
        if self.failed {
            return Err(LayoutError::FailedChildOperation);
        }
        self.failed = true;
        let placed = self
            .placed
            .get_mut(index)
            .ok_or(LayoutError::MissingChild(index))?;
        if *placed {
            return Err(LayoutError::DuplicatePlacement(index));
        }
        let id = self
            .access
            .child(index)
            .ok_or(LayoutError::MissingChild(index))?;
        self.access.execute_child(id, |entry, access| {
            Ok(layout(
                entry,
                access,
                Geometry { metrics, origin },
                self.draft,
                self.force,
            ))
        })??;
        *placed = true;
        self.failed = false;
        Ok(())
    }
}

fn layout(
    entry: &mut Entry,
    access: &mut Context<'_, Entry>,
    geometry: Geometry,
    draft: &mut Vec<Draft>,
    force: bool,
) -> Result<()> {
    geometry.metrics.validate()?;
    if !force
        && !entry.layout_dirty
        && entry
            .geometry
            .is_some_and(|old| old.metrics == geometry.metrics)
    {
        if entry.geometry != Some(geometry) {
            draft.push(Draft {
                node: access.current(),
                geometry,
            });
        }
        return Ok(());
    }
    entry.layout_dirty = true;
    entry.counts.layouts += 1;
    let placed = vec![false; access.child_count()];
    let mut cx = LayoutContext {
        access,
        draft,
        placed,
        force,
        failed: false,
    };
    entry.body.layout(&mut cx, geometry.metrics.size)?;
    if cx.failed {
        return Err(LayoutError::FailedChildOperation);
    }
    if cx.placed.iter().any(|&placed| !placed) {
        return Err(LayoutError::UnplacedChildren);
    }
    cx.draft.push(Draft {
        node: cx.access.current(),
        geometry,
    });
    Ok(())
}

pub struct Scene {
    store: NodeStore<Entry>,
    nodes: Vec<NodeId>,
    pending: bool,
    width: Option<u32>,
}

impl Scene {
    pub fn new(root: impl LayoutNode) -> Self {
        let store = NodeStore::new(Box::new(Entry::new(root)), NonZeroUsize::new(64).unwrap());
        let nodes = vec![store.root()];
        Self {
            store,
            nodes,
            pending: true,
            width: None,
        }
    }

    pub fn root(&self) -> NodeId {
        self.store.root()
    }

    pub fn append(&mut self, parent: NodeId, body: impl LayoutNode) -> Result<NodeId> {
        if self.width.is_some() {
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

    pub fn run(&mut self, width: u32, force: bool) -> Result<usize> {
        let query = Query::new(width)?;
        if !self.pending && self.width == Some(width) && !force {
            return Ok(0);
        }
        self.pending = true;
        let root = self.store.root();
        let metrics = self.store.execute(root, |entry, access| {
            Ok(measure(entry, access, query, force))
        })??;
        let mut draft = Vec::new();
        self.store.execute(root, |entry, access| {
            Ok(layout(
                entry,
                access,
                Geometry {
                    metrics,
                    origin: Origin::default(),
                },
                &mut draft,
                force,
            ))
        })??;
        let published = draft.len();
        for change in draft {
            let entry = self
                .store
                .get_mut(change.node)
                .expect("layout cannot remove nodes");
            entry.geometry = Some(change.geometry);
            entry.layout_dirty = false;
        }
        self.width = Some(width);
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
}
