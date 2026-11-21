//! Estimated document coordinates around a contiguous window of measured rows.
use super::Viewport;
use crate::{Instance, NodeId, Pixels, Result, layout::Size};
use anyhow::{Context as _, ensure};
use std::{cell::RefCell, collections::HashMap, hash::Hash, rc::Rc};

/// Only mounted rows contribute exact heights. Prefixes are relative to the first row,
/// so distant scrolling does not lose fractional precision in local geometry.
#[derive(Default)]
pub(super) struct HeightIndex {
    first: usize,
    edges: Vec<f64>,
}

impl HeightIndex {
    pub(super) fn covers(&self, count: usize) -> bool {
        count == 0 || (self.first == 0 && self.edges.len().saturating_sub(1) >= count)
    }

    pub(super) fn top(&self, index: usize, estimate: f64) -> f64 {
        let measured = index
            .saturating_sub(self.first)
            .min(self.edges.len().saturating_sub(1));
        (index - measured) as f64 * estimate + self.edges.get(measured).copied().unwrap_or_default()
    }

    pub(super) fn row_at(&self, y: f64, count: usize, estimate: f64) -> usize {
        let mut lo = 0;
        let mut hi = count;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.top(mid + 1, estimate) <= y {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }

    fn read(
        instance: &Instance,
        id: NodeId,
        first: usize,
        model_changed: bool,
        estimate: f64,
    ) -> Result<Self> {
        let children = instance.scene.children(id)?;
        let mut edges = Vec::with_capacity(children.len() + 1);
        edges.push(0.0);
        let mut y = 0.0;
        for child in children {
            let height = f64::from(
                instance
                    .scene
                    .geometry(child)
                    .context("missing row geometry")?
                    .metrics
                    .size
                    .height,
            );
            // An edited model may be about to retire an invalid row. Validate the
            // surviving rows once their new order has been laid out.
            ensure!(
                height > 0.0 || model_changed,
                "measured list rows need a positive height"
            );
            y += if height > 0.0 { height } else { estimate };
            edges.push(y);
        }
        Ok(Self { first, edges })
    }

    fn remap<K: Eq + Hash>(
        &self,
        old_keys: &[K],
        keys: &[K],
        anchor: usize,
        estimate: f64,
    ) -> Self {
        let heights: HashMap<_, _> = old_keys
            .iter()
            .zip(self.edges.windows(2).map(|pair| pair[1] - pair[0]))
            .collect();
        let reach = old_keys.len();
        let first = anchor.saturating_sub(reach);
        let end = anchor
            .saturating_add(reach)
            .saturating_add(1)
            .min(keys.len());
        let mut edges = Vec::with_capacity(end - first + 1);
        let mut sum = 0.0;
        edges.push(sum);
        for key in &keys[first..end] {
            sum += heights.get(key).copied().unwrap_or(estimate);
            edges.push(sum);
        }
        Self { first, edges }
    }
}

pub(super) struct MeasuredWindow {
    pub(super) index: Rc<RefCell<HeightIndex>>,
    pub(super) height: Pixels,
}

impl<K: Eq + Hash + Clone + 'static> Viewport<K> {
    pub(super) fn measured_offset(
        &self,
        instance: &Instance,
        id: NodeId,
        size: Size,
        edit: Option<(&[K], &[K])>,
        count: usize,
        distance: f64,
    ) -> Result<(f64, HeightIndex)> {
        let previous = self.measured.as_ref().expect("measured list");
        let old = previous.index.borrow();
        let estimate = f64::from(self.height);
        let old_limit =
            (old.top(self.previous_count, estimate) - f64::from(previous.height)).max(0.0);
        let old_row = old.row_at(distance, self.previous_count, estimate);
        let mut within = distance - old.top(old_row, estimate);
        let changed = self.keys_changed.get();
        let mut row = old_row.min(count.saturating_sub(1));
        let mut next = HeightIndex::read(instance, id, self.range.start, changed, estimate)?;
        if let Some((previous_keys, keys)) = edit {
            (row, within) = self.relocated_row(old_row, within, previous_keys, keys);
            // Retain measured heights by key around the anchor, including a tall
            // anchor whose within-row distance exceeds the estimate.
            next = next.remap(previous_keys, keys, row, estimate);
        }
        let limit = (next.top(count, estimate) - f64::from(size.height)).max(0.0);
        let offset = if !changed && old_limit > 0.0 && distance >= old_limit {
            limit
        } else {
            let top = next.top(row, estimate);
            (top + within.clamp(0.0, next.top(row.saturating_add(1), estimate) - top)).min(limit)
        };
        Ok((offset, next))
    }
}
