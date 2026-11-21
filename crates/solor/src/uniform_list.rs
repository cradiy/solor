//! Retained rows in a bounded vertical viewport.
mod measured;
use measured::{HeightIndex, MeasuredWindow};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    hash::Hash,
    ops::Range,
    rc::Rc,
};

use anyhow::{Context as _, ensure};

use crate::{
    Attachment, Child, Element, Instance, IntoElement, NodeId, Path, Pixels, Result, Signal,
    StyleRefinement, Styled, StyledBody, component, element,
    layout::{
        self, LayoutContext, LayoutNode, LayoutOutput, MeasureContext, Metrics, Origin, Query, Size,
    },
    pointer::ScrollDelta,
    px,
    state::{Runtime, SignalId},
};

/// The ordered, unique row keys of a uniform list. A count represents the keys
/// `0..count` without storing them. A vector signal publishes insertions, removals and reorders.
pub struct ListItems<K>(Source<K>);

enum Input<T> {
    Fixed(T),
    Bound(Signal<T>),
}

impl<T: 'static> Input<T> {
    fn read<'a>(&'a self, state: &'a Runtime<()>) -> Result<&'a T> {
        match self {
            Self::Fixed(value) => Ok(value),
            Self::Bound(signal) => Ok(state.read(*signal)?),
        }
    }

    fn signal(&self) -> Option<SignalId> {
        match self {
            Self::Bound(signal) => Some(signal.id()),
            Self::Fixed(_) => None,
        }
    }
}

enum Source<K> {
    Indices(Input<usize>, fn(usize) -> K),
    Keys(Input<Vec<K>>),
}

impl From<usize> for ListItems<usize> {
    fn from(count: usize) -> Self {
        Self(Source::Indices(Input::Fixed(count), |i| i))
    }
}

impl From<Signal<usize>> for ListItems<usize> {
    fn from(count: Signal<usize>) -> Self {
        Self(Source::Indices(Input::Bound(count), |i| i))
    }
}

impl<K> From<Vec<K>> for ListItems<K> {
    fn from(keys: Vec<K>) -> Self {
        Self(Source::Keys(Input::Fixed(keys)))
    }
}

impl<K> From<Signal<Vec<K>>> for ListItems<K> {
    fn from(keys: Signal<Vec<K>>) -> Self {
        Self(Source::Keys(Input::Bound(keys)))
    }
}

impl<K: Clone + 'static> ListItems<K> {
    fn signal(&self) -> Option<SignalId> {
        match &self.0 {
            Source::Indices(count, _) => count.signal(),
            Source::Keys(keys) => keys.signal(),
        }
    }

    fn len(&self, state: &Runtime<()>) -> Result<usize> {
        Ok(match &self.0 {
            Source::Indices(count, _) => *count.read(state)?,
            Source::Keys(keys) => keys.read(state)?.len(),
        })
    }

    fn keys(&self, state: &Runtime<()>, range: Range<usize>) -> Result<Vec<K>> {
        Ok(match &self.0 {
            Source::Indices(_, key) => range.map(key).collect(),
            Source::Keys(keys) => keys.read(state)?[range].to_vec(),
        })
    }
}

struct RowLayout {
    measured: bool,
    height: Pixels,
    first_y: Pixels,
}

impl Element for RowLayout {
    fn prepare_content(&mut self, _: &crate::Style) -> Result<()> {
        ensure!(
            self.height.is_extent() && self.height.get() > 0.0,
            "row height must be positive and finite"
        );
        Ok(())
    }
}

impl LayoutNode for RowLayout {
    fn measure(&mut self, _: &mut MeasureContext<'_, '_>, query: Query) -> layout::Result<Metrics> {
        Ok(Metrics {
            size: Size {
                width: query.resolve(px(0.0)),
                height: px(0.0),
            },
            baseline: None,
        })
    }

    fn layout(
        &mut self,
        cx: &mut LayoutContext<'_, '_>,
        size: Size,
    ) -> layout::Result<LayoutOutput> {
        let mut y = f64::from(self.first_y);
        for i in 0..cx.child_count() {
            let height = if self.measured {
                cx.measure_child(i, Query::Exact(size.width))?.size.height
            } else {
                self.height
            };
            cx.layout_child(
                i,
                Size {
                    width: size.width,
                    height,
                },
            )?;
            cx.place_child(
                i,
                Origin {
                    x: px(0.0),
                    y: px(y as f32),
                },
            )?;
            y += f64::from(height);
        }
        Ok(LayoutOutput {
            content: size,
            baseline: None,
        })
    }
}

/// Creates a vertical list with a fixed row height in logical pixels. Give the list
/// a viewport through layout, for example with `.h(px(300.0))` or `.flex_1()`.
/// The builder runs once when a key enters the visible range or its one-row buffer.
/// Keys retained across a scroll or reorder keep their nodes and local signals.
/// Keep state that must survive leaving the buffer in the parent or application.
pub fn uniform_list<K, E>(
    items: impl Into<ListItems<K>>,
    height: Pixels,
    row: impl Fn(K) -> E + 'static,
) -> impl IntoElement + Styled
where
    K: Clone + Eq + Hash + 'static,
    E: IntoElement + 'static,
{
    VirtualList {
        measured: false,
        items: items.into(),
        height,
        row,
        style: StyleRefinement::default(),
    }
    .overflow_hidden()
}

/// Creates a vertical virtual list whose rows measure their own positive heights at the
/// available width. Supply a typical height for locating rows outside the mounted window.
/// Reflow preserves the top row and its within-row distance, subject to the scroll bounds.
/// Keyed edits retain the top visible key when it is still present.
pub fn variable_list<K, E>(
    items: impl Into<ListItems<K>>,
    estimated_height: Pixels,
    row: impl Fn(K) -> E + 'static,
) -> impl IntoElement + Styled
where
    K: Clone + Eq + Hash + 'static,
    E: IntoElement + 'static,
{
    VirtualList {
        items: items.into(),
        height: estimated_height,
        row,
        measured: true,
        style: StyleRefinement::default(),
    }
    .overflow_hidden()
}

struct VirtualList<K, F> {
    measured: bool,
    items: ListItems<K>,
    height: Pixels,
    row: F,
    style: StyleRefinement,
}

impl<K, F> Styled for VirtualList<K, F> {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

type Reconcile = Box<dyn FnMut(&mut Instance, NodeId, &Path, &mut Attachment<'_>) -> Result<()>>;

pub(super) struct UpdateList {
    pub(super) dirty: Rc<Cell<bool>>,
    reconcile: Reconcile,
}

struct Viewport<K> {
    measured: Option<MeasuredWindow<K>>,
    items: Rc<ListItems<K>>,
    height: Pixels,
    offset: Signal<f64>,
    keys_changed: Rc<Cell<bool>>,
    range: Range<usize>,
    rows: MountedRows<K>,
}

impl<K, F, E> IntoElement for VirtualList<K, F>
where
    K: Clone + Eq + Hash + 'static,
    F: Fn(K) -> E + 'static,
    E: IntoElement + 'static,
{
    fn into_element(self) -> crate::AnyElement {
        component(move |cx| {
            let height = self.height;
            let offset = cx.signal(0_f64);
            let items = Rc::new(self.items);
            let scroll_items = items.clone();
            let index = self
                .measured
                .then(|| Rc::new(RefCell::new(HeightIndex::default())));
            let scroll_index = index.clone();
            let mut node = element(RowLayout {
                measured: self.measured,
                height,
                first_y: px(0.0),
            })
            .on_scroll(move |event, cx| {
                let count = scroll_items.len(cx.state)?;
                let extent = scroll_index.as_ref().map_or_else(
                    || count as f64 * f64::from(height),
                    |index| index.borrow().top(count, f64::from(height)),
                );
                let limit = (extent - f64::from(event.pointer.content_size().height)).max(0.0);
                let previous = cx.read(offset)?.min(limit);
                let delta = f64::from(event.delta.y);
                let used = delta.clamp(-previous, limit - previous);
                cx.set(offset, previous + used)?;
                cx.propagate_remaining(ScrollDelta {
                    x: event.delta.x,
                    y: px((delta - used) as f32),
                })
            });
            *node.style() = self.style;
            let dirty = Rc::new(Cell::new(true));
            let viewport_dirty = dirty.clone();
            let keys_changed = Rc::new(Cell::new(true));
            let source_changed = keys_changed.clone();
            let source = items.signal();
            node.watches.push(Box::new(move |cx, target, _, _, _| {
                let scrolled = viewport_dirty.clone();
                cx.state
                    .observe(offset.id(), target, move || scrolled.set(true))?;
                if let Some(source) = source {
                    cx.state.observe(source, target, move || {
                        source_changed.set(true);
                        viewport_dirty.set(true);
                    })?;
                }
                Ok(())
            }));
            let mut viewport = Viewport {
                measured: index.map(|index| MeasuredWindow {
                    index,
                    keys: Vec::new(),
                    count: 0,
                    height: px(0.0),
                }),
                items,
                height,
                offset,
                keys_changed,
                range: 0..0,
                rows: MountedRows::default(),
            };
            node.children.push(Child::List(UpdateList {
                dirty,
                reconcile: Box::new(move |instance, id, path, cx| {
                    viewport.update(instance, id, path, cx, &self.row)
                }),
            }));
            node
        })
        .into_element()
    }
}

struct Window {
    offset: f64,
    range: Range<usize>,
    first_y: Pixels,
}

impl Window {
    fn new(
        offset: f64,
        range: Range<usize>,
        top: impl FnOnce(usize) -> f64,
        size: Size,
        visible: bool,
    ) -> Self {
        let range = if visible && size.width.get() > 0.0 && size.height.get() > 0.0 {
            range
        } else {
            0..0
        };
        let first_y = if range.is_empty() {
            px(0.0)
        } else {
            px((top(range.start) - offset) as f32)
        };
        Self {
            offset,
            range,
            first_y,
        }
    }
}

impl<K: Clone + Eq + Hash + 'static> Viewport<K> {
    fn update<E: IntoElement>(
        &mut self,
        instance: &mut Instance,
        id: NodeId,
        path: &Path,
        cx: &mut Attachment<'_>,
        build: &impl Fn(K) -> E,
    ) -> Result<()> {
        let body = instance.scene.body::<StyledBody<RowLayout>>(id)?;
        let size = body.content_size(
            instance
                .scene
                .geometry(id)
                .context("missing list geometry")?
                .metrics
                .size,
        );
        let count = self.items.len(cx.state)?;
        let distance = *cx.state.read(self.offset)?;
        let (window, index) = if self.measured.is_some() {
            let (window, index) = self.measured_window(instance, id, size, cx, count, distance)?;
            (window, Some(index))
        } else {
            let h = f64::from(self.height);
            let offset = distance.min((count as f64 * h - f64::from(size.height)).max(0.0));
            let first = (offset / h).floor() as usize;
            let end = ((offset + f64::from(size.height)) / h).ceil() as usize;
            (
                Window::new(
                    offset,
                    first.saturating_sub(1).min(count)..end.saturating_add(1).min(count),
                    |i| i as f64 * h,
                    size,
                    instance.bodies[&id].visible,
                ),
                None,
            )
        };
        let Window {
            offset,
            range,
            first_y,
        } = window;
        let refresh_measurements = self.measured.is_some() && self.keys_changed.get();
        let moved = body.content.first_y != first_y;
        if range != self.range || self.keys_changed.get() {
            let keys = self.items.keys(cx.state, range.clone())?;
            self.rows.reconcile(instance, path, cx, &keys, build)?;
            if let Some(measured) = &mut self.measured {
                measured.keys = keys;
            }
            self.keys_changed.set(false);
            self.range = range;
        }
        if let (Some(measured), Some(index)) = (&mut self.measured, index) {
            *measured.index.borrow_mut() = index;
            measured.count = count;
            measured.height = size.height;
        }
        cx.state.set(self.offset, offset)?;
        if moved || refresh_measurements {
            instance
                .scene
                .edit_layout::<StyledBody<RowLayout>>(id, |body| body.content.first_y = first_y)?;
        }
        Ok(())
    }
}

struct MountedRows<K> {
    slots: HashMap<K, usize>,
    next: usize,
}

impl<K> Default for MountedRows<K> {
    fn default() -> Self {
        Self {
            slots: HashMap::new(),
            next: 0,
        }
    }
}

impl<K: Eq + Hash + Clone> MountedRows<K> {
    fn reconcile<E: IntoElement>(
        &mut self,
        instance: &mut Instance,
        path: &Path,
        cx: &mut Attachment<'_>,
        keys: &[K],
        build: &impl Fn(K) -> E,
    ) -> Result<()> {
        let wanted: HashSet<_> = keys.iter().collect();
        ensure!(wanted.len() == keys.len(), "duplicate visible list key");
        let parent = instance.nodes[path];
        let mut added = Vec::new();
        let mut child = path.clone();
        child.push(0);
        let depth = path.len();
        self.next
            .checked_add(keys.len())
            .context("list row identity exhausted")?;
        for key in keys {
            if !self.slots.contains_key(key) {
                child[depth] = self.next;
                self.next += 1;
                let end = instance.scene.children(parent)?.len();
                if let Err(error) =
                    instance.insert(parent, end, &child, build(key.clone()).into_element(), cx)
                {
                    instance.remove(&child, cx)?;
                    for (key, slot) in added {
                        self.slots.remove(key);
                        child[depth] = slot;
                        instance.remove(&child, cx)?;
                    }
                    return Err(error);
                }
                self.slots.insert(key.clone(), child[depth]);
                added.push((key, child[depth]));
            }
        }
        for (_, slot) in self.slots.extract_if(|key, _| !wanted.contains(key)) {
            child[depth] = slot;
            instance.remove(&child, cx)?;
        }
        let order: HashMap<_, _> = keys
            .iter()
            .enumerate()
            .map(|(i, k)| (self.slots[k], i))
            .collect();
        let rank = |id: NodeId| order[&instance.bodies[&id].offset];
        if !instance.scene.children(parent)?.is_sorted_by_key(rank) {
            instance.scene.sort_children_by_key(parent, rank)?;
        }
        Ok(())
    }
}

impl Instance {
    pub(super) fn update_lists(
        &mut self,
        cx: &mut Attachment<'_>,
        layout_changed: bool,
    ) -> Result<()> {
        // Parent updates may retire child lists; newly attached ones need their first allocation.
        let paths: Vec<_> = self
            .lists
            .iter()
            .filter(|(_, list)| layout_changed || list.dirty.get())
            .map(|(path, _)| path.clone())
            .collect();
        for path in paths {
            let Some(mut update) = self.lists.remove(&path) else {
                continue;
            };
            let node = self.nodes[&path];
            let result = if self.scene.geometry(node).is_some() {
                let result = (update.reconcile)(self, node, &path, cx);
                update.dirty.set(result.is_err());
                result
            } else {
                Ok(())
            };
            self.lists.insert(path, update);
            result?;
        }
        Ok(())
    }
}
