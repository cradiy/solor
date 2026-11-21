//! An ordered collection with independent item and membership notifications.
use super::{Runtime, Signal};
use crate::Context;
use anyhow::{Context as _, Result, ensure};
use std::{any::Any, ops::Range};

/// A context-owned sequence of independently observable items.
///
/// Both virtual list builders accept this handle and pass a stable `Signal<T>` to
/// the row builder. Moving an item preserves that signal. Removing it retires
/// its storage once its last reactive binding detaches.
pub struct SignalVec<T> {
    pub(crate) order: Signal<Vec<Signal<T>>>,
}

impl<T> Copy for SignalVec<T> {}
impl<T> Clone for SignalVec<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl Context<'_> {
    /// Creates a sequence in the current scope, with one signal per value.
    pub fn signal_vec<T: 'static>(&mut self, values: impl IntoIterator<Item = T>) -> SignalVec<T> {
        let values = values.into_iter();
        let order = Vec::with_capacity(values.size_hint().0);
        self.build_signal_vec(values, order)
    }

    /// Creates a sequence with room for at least `capacity` total item handles.
    ///
    /// Use an expected total when loading pages into a large collection. Only
    /// initial values get item signals. The sequence grows beyond the estimate
    /// as needed. Returns an error if the initial allocation cannot be made.
    pub fn signal_vec_with_capacity<T: 'static>(
        &mut self,
        values: impl IntoIterator<Item = T>,
        capacity: usize,
    ) -> Result<SignalVec<T>> {
        let values = values.into_iter();
        let mut order = Vec::new();
        order.try_reserve_exact(capacity.max(values.size_hint().0))?;
        Ok(self.build_signal_vec(values, order))
    }

    fn build_signal_vec<T: 'static>(
        &mut self,
        values: impl Iterator<Item = T>,
        order: Vec<Signal<T>>,
    ) -> SignalVec<T> {
        let order = self.signal(order);
        self.state
            .vector_disposers
            .insert(order.id.key, |value, state| {
                let order = value
                    .downcast::<Vec<Signal<T>>>()
                    .expect("typed signal vector");
                state.retire_items(*order);
            });
        let rows = SignalVec { order };
        for value in values {
            rows.push(value, self).expect("new signal vector");
        }
        rows
    }
}

impl<N> Runtime<N> {
    fn retire_item<T>(&mut self, item: Signal<T>) -> Option<Box<dyn Any>> {
        let entry = &mut self.signals[item.id.key];
        if entry.bindings.is_empty() {
            self.signals.remove(item.id.key).map(|entry| entry.value)
        } else {
            // A mounted row can still read the value before list reconciliation.
            entry.retire_after_unbind = true;
            None
        }
    }

    fn retire_items<T>(&mut self, items: Vec<Signal<T>>) {
        // Retire every handle before a user-defined destructor can unwind.
        let values: Vec<_> = items
            .into_iter()
            .filter_map(|item| self.retire_item(item))
            .collect();
        drop(values);
    }
}

impl<T: 'static> SignalVec<T> {
    /// Borrows the ordered item handles without cloning their values.
    pub fn items<'a>(self, cx: &'a Context<'_>) -> Result<&'a [Signal<T>]> {
        Ok(cx.read(self.order)?.as_slice())
    }

    /// Returns the item signal at the index, or an error if out of bounds.
    pub fn item(self, index: usize, cx: &Context<'_>) -> Result<Signal<T>> {
        self.items(cx)?
            .get(index)
            .copied()
            .context("item index out of bounds")
    }

    /// Finds the current index of an item by its signal identity in a linear scan.
    /// Returns `None` if the item is absent, even if another binding still holds it.
    pub fn index_of(self, item: Signal<T>, cx: &Context<'_>) -> Result<Option<usize>> {
        Ok(self
            .items(cx)?
            .iter()
            .position(|&candidate| candidate == item))
    }

    /// Replaces a single value, notifying its subscribers only if it changes.
    pub fn set(self, index: usize, value: T, cx: &mut Context<'_>) -> Result<()>
    where
        T: PartialEq,
    {
        cx.set(self.item(index, cx)?, value)
    }

    /// Edits one item without cloning it and notifies its subscribers unconditionally.
    pub fn update_in_place<R>(
        self,
        index: usize,
        cx: &mut Context<'_>,
        edit: impl FnOnce(&mut T) -> R,
    ) -> Result<R> {
        cx.update_in_place(self.item(index, cx)?, edit)
    }

    /// Appends a new value and returns its stable item signal.
    pub fn push(self, value: T, cx: &mut Context<'_>) -> Result<Signal<T>> {
        cx.read(self.order)?;
        let item = cx.state.create_signal(value);
        cx.update_in_place(self.order, |items| items.push(item))?;
        Ok(item)
    }

    /// Inserts a new item before the index. An index equal to the length appends it.
    pub fn insert(self, index: usize, value: T, cx: &mut Context<'_>) -> Result<Signal<T>> {
        ensure!(index <= self.items(cx)?.len(), "item index out of bounds");
        let item = cx.state.create_signal(value);
        cx.update_in_place(self.order, |items| items.insert(index, item))?;
        Ok(item)
    }

    /// Removes the item at the index and publishes the new order.
    pub fn remove(self, index: usize, cx: &mut Context<'_>) -> Result<()> {
        self.item(index, cx)?;
        let removed = cx.update_in_place(self.order, |items| items.remove(index))?;
        drop(cx.state.retire_item(removed));
        Ok(())
    }

    /// Replaces a half-open range with new items and publishes the order once.
    ///
    /// An empty range inserts at its start. An empty iterator removes the range.
    /// Items outside the range keep their signals. An invalid range returns an
    /// error before consuming the iterator.
    pub fn splice(
        self,
        range: Range<usize>,
        values: impl IntoIterator<Item = T>,
        cx: &mut Context<'_>,
    ) -> Result<()> {
        self.splice_in(range, values, cx.state)
    }

    pub(crate) fn splice_in(
        self,
        range: Range<usize>,
        values: impl IntoIterator<Item = T>,
        state: &mut Runtime<()>,
    ) -> Result<()> {
        let len = state.read(self.order)?.len();
        ensure!(
            range.start <= range.end && range.end <= len,
            "invalid item range"
        );
        // Finish running caller code before registering any new item signals.
        let values: Vec<_> = values.into_iter().collect();
        if range.is_empty() && values.is_empty() {
            return Ok(());
        }
        let inserted: Vec<_> = values
            .into_iter()
            .map(|value| state.create_signal(value))
            .collect();
        let removed =
            state.update_in_place(self.order, |items| items.splice(range, inserted).collect())?;
        state.retire_items(removed);
        Ok(())
    }

    /// Moves an item to its final zero-based index. Both indices must exist.
    pub fn move_item(self, from: usize, to: usize, cx: &mut Context<'_>) -> Result<()> {
        let len = self.items(cx)?.len();
        ensure!(from < len && to < len, "item index out of bounds");
        if from < to {
            cx.update_in_place(self.order, |items| items[from..=to].rotate_left(1))?;
        } else if from > to {
            cx.update_in_place(self.order, |items| items[to..=from].rotate_right(1))?;
        }
        Ok(())
    }
}
