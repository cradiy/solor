//! Retained rows in a bounded vertical viewport.
mod measured;
mod scroll;
use crate::ListScrollPosition;
use measured::{HeightIndex, MeasuredWindow};
pub use scroll::{ListScrollHandle, ScrollAlignment};
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

impl<T: 'static> From<crate::SignalVec<T>> for ListItems<Signal<T>> {
    fn from(rows: crate::SignalVec<T>) -> Self {
        rows.order.into()
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
) -> VirtualList<K, impl Fn(K) -> E>
where
    K: Clone + Eq + Hash + 'static,
    E: IntoElement + 'static,
{
    VirtualList {
        scroll: None,
        follow_tail: false,
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
) -> VirtualList<K, impl Fn(K) -> E>
where
    K: Clone + Eq + Hash + 'static,
    E: IntoElement + 'static,
{
    VirtualList {
        scroll: None,
        follow_tail: false,
        items: items.into(),
        height: estimated_height,
        row,
        measured: true,
        style: StyleRefinement::default(),
    }
    .overflow_hidden()
}

/// A stylable virtual list declaration with an optional scroll handle.
pub struct VirtualList<K, F> {
    scroll: Option<ListScrollHandle>,
    follow_tail: bool,
    measured: bool,
    items: ListItems<K>,
    height: Pixels,
    row: F,
    style: StyleRefinement,
}

impl<K, F> VirtualList<K, F> {
    /// Opens the list at its end and keeps the last row in view as content grows.
    /// Scrolling away pauses following. Returning to the bottom resumes it.
    pub fn follow_tail(mut self) -> Self {
        self.follow_tail = true;
        self
    }

    /// Attaches a handle for programmatic row positioning.
    pub fn scroll_handle(mut self, handle: &ListScrollHandle) -> Self {
        self.scroll = Some(handle.clone());
        self
    }
}

impl<K, F> Styled for VirtualList<K, F> {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

type Reconcile = Box<dyn FnMut(&mut Instance, NodeId, &Path, &mut Attachment<'_>) -> Result<()>>;

pub(super) struct UpdateList {
    pub(super) scroll: Option<ListScrollHandle>,
    pub(super) dirty: Rc<Cell<bool>>,
    reconcile: Reconcile,
}

struct Viewport<K> {
    scroll: Option<ListScrollHandle>,
    // When tail following is enabled, compare user input with the previous scroll limit.
    previous_limit: Option<f64>,
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
            let wheel_handle = self.scroll.clone();
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
                let delta = f64::from(event.delta.y);
                let used = scroll::apply_delta(cx, offset, limit, delta, wheel_handle.as_ref())?;
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
            let handle = self.scroll.clone();
            node.watches.push(Box::new(move |cx, target, _, _, _| {
                if let Some(handle) = handle {
                    handle.attach(&viewport_dirty, offset, cx.state)?;
                }
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
                scroll: self.scroll.clone(),
                previous_limit: self.follow_tail.then_some(0.0),
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
                scroll: self.scroll,
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
    limit: f64,
    range: Range<usize>,
    first_y: Pixels,
}

impl Window {
    fn resolve(
        count: usize,
        size: Size,
        visible: bool,
        distance_from_top: Option<f64>,
        estimate: f64,
        measured: Option<&HeightIndex>,
        handle: Option<&ListScrollHandle>,
    ) -> Self {
        let top = |i| measured.map_or_else(|| i as f64 * estimate, |index| index.top(i, estimate));
        let viewport = f64::from(size.height);
        let limit = (top(count) - viewport).max(0.0);
        let mut offset = distance_from_top.unwrap_or(limit).min(limit);
        let active = visible && size.width.get() > 0.0 && viewport > 0.0;
        let is_estimated = measured.is_some_and(|index| !index.covers(count));

        if count == 0 || !active {
            if let Some(handle) = handle {
                handle.stage(active.then_some(ListScrollPosition {
                    offset_px: offset,
                    max_offset_px: limit,
                    viewport_height: size.height,
                    is_estimated,
                }));
            }
            return Self {
                offset,
                limit,
                range: 0..0,
                first_y: px(0.0),
            };
        }
        if let Some(handle) = handle {
            offset = handle
                .resolve(offset, viewport, count, limit, top)
                .clamp(0.0, limit);
        }
        let row_at = |y: f64| {
            measured.map_or_else(
                || ((y / estimate).floor() as usize).min(count),
                |index| index.row_at(y, count, estimate),
            )
        };
        let first = row_at(offset);
        let bottom = offset + viewport;
        let last = row_at(bottom);
        let end = last.saturating_add(usize::from(top(last) < bottom));
        let range = first.saturating_sub(1)..end.saturating_add(1).min(count);
        if let Some(handle) = handle {
            handle.stage(Some(ListScrollPosition {
                offset_px: offset,
                max_offset_px: limit,
                viewport_height: size.height,
                is_estimated,
            }));
            if measured.is_some() && range != (0..count) && offset > 0.0 && offset < limit {
                handle.anchor_estimated_target(first, offset - top(first));
            }
        }
        let first_y = px((top(range.start) - offset) as f32);
        Self {
            offset,
            limit,
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
        let at_tail = self.previous_limit.is_some_and(|limit| distance >= limit);
        let (distance, index) = if self.measured.is_some() {
            let (anchored, index) =
                self.measured_offset(instance, id, size, cx, count, distance)?;
            (anchored, Some(index))
        } else {
            (distance, None)
        };
        let Window {
            offset,
            limit,
            range,
            first_y,
        } = Window::resolve(
            count,
            size,
            instance.bodies[&id].visible,
            (!at_tail).then_some(distance),
            f64::from(self.height),
            index.as_ref(),
            self.scroll.as_ref(),
        );
        let request = self.scroll.as_ref().and_then(ListScrollHandle::pending);
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
        if let Some(previous) = &mut self.previous_limit {
            *previous = limit;
        }
        if moved || refresh_measurements {
            instance
                .scene
                .edit_layout::<StyledBody<RowLayout>>(id, |body| body.content.first_y = first_y)?;
        }
        // Retain the target through all passes that mount or remeasure destination rows.
        if let Some(handle) = &self.scroll
            && request.is_some()
            && request == handle.pending()
            && !self.range.is_empty()
            && !instance.scene.is_pending()
        {
            handle.clear();
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
                update.dirty.set(false);
                let result = (update.reconcile)(self, node, &path, cx);
                if result.is_err() {
                    update.dirty.set(true);
                }
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
