//! Explicit signal-to-node bindings with batched, deduplicated updates.

use std::any::Any;
use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;

use slotmap::{SlotMap, new_key_type};

use crate::tree::{NodeId, Tree, TreeError};

new_key_type! {
    struct SignalKey;
    struct BindingKey;
}

/// Runtime-local identity for tracking and disposing signals of different types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SignalId {
    owner: NodeId,
    key: SignalKey,
}

/// A typed, non-owning signal handle. Dropping it does not remove the signal.
pub struct Signal<S> {
    id: SignalId,
    marker: PhantomData<fn(S) -> S>,
}

impl<S> Signal<S> {
    /// Returns the identity without granting untyped access to the stored value.
    pub fn id(self) -> SignalId {
        self.id
    }
}

impl<S> PartialEq for Signal<S> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<S> Eq for Signal<S> {}

impl<S> Copy for Signal<S> {}

impl<S> Clone for Signal<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S> fmt::Debug for Signal<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Signal")
            .field("owner", &self.id.owner)
            .field("key", &self.id.key)
            .finish()
    }
}

/// A non-owning handle used to explicitly cancel a binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BindingId {
    owner: NodeId,
    key: BindingKey,
}

/// A rejected operation. Invalid handles do not change runtime state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateError {
    /// The signal handle is stale or belongs to another runtime.
    InvalidSignal,
    /// The binding is no longer live in this runtime.
    InvalidBinding,
    /// The underlying tree rejected the node operation.
    Tree(TreeError),
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSignal => f.write_str("invalid signal handle"),
            Self::InvalidBinding => f.write_str("invalid binding handle"),
            Self::Tree(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for StateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Tree(error) => Some(error),
            _ => None,
        }
    }
}

impl From<TreeError> for StateError {
    fn from(error: TreeError) -> Self {
        Self::Tree(error)
    }
}

struct SignalEntry {
    // True only while every live subscriber is already pending.
    all_queued: bool,
    value: Box<dyn Any>,
    bindings: Vec<BindingKey>,
}

type Apply<N> = Box<dyn FnMut(&dyn Any, &mut N)>;

struct Binding<N> {
    node: NodeId,
    signal: SignalKey,
    signal_index: usize,
    node_index: usize,
    pending_index: Option<usize>,
    apply: Apply<N>,
}

struct Bindings<N> {
    entries: SlotMap<BindingKey, Binding<N>>,
    by_node: HashMap<NodeId, Vec<BindingKey>>,
    pending: Vec<BindingKey>,
}

impl<N> Bindings<N> {
    fn new() -> Self {
        Self {
            entries: SlotMap::with_key(),
            by_node: HashMap::new(),
            pending: Vec::new(),
        }
    }

    fn queue(&mut self, key: BindingKey) {
        let binding = &mut self.entries[key];
        if binding.pending_index.is_none() {
            binding.pending_index = Some(self.pending.len());
            self.pending.push(key);
        }
    }

    fn remove(&mut self, key: BindingKey, signals: &mut SlotMap<SignalKey, SignalEntry>) {
        let binding = self.entries.remove(key).expect("binding is live");

        let signal_bindings = &mut signals[binding.signal].bindings;
        signal_bindings.swap_remove(binding.signal_index);
        if let Some(&moved) = signal_bindings.get(binding.signal_index) {
            self.entries[moved].signal_index = binding.signal_index;
        }

        let node_bindings = self
            .by_node
            .get_mut(&binding.node)
            .expect("node is indexed");
        node_bindings.swap_remove(binding.node_index);
        if let Some(&moved) = node_bindings.get(binding.node_index) {
            self.entries[moved].node_index = binding.node_index;
        }
        if node_bindings.is_empty() {
            self.by_node.remove(&binding.node);
        }

        if let Some(index) = binding.pending_index {
            self.pending.swap_remove(index);
            if let Some(&moved) = self.pending.get(index) {
                self.entries[moved].pending_index = Some(index);
            }
        }
        // Release user captures only after every index is consistent.
        drop(binding);
    }
}

/// Owns a tree, signals, and explicit one-source bindings.
///
/// Callbacks can edit their target payload but cannot access the runtime through
/// their arguments. Execution order is unspecified. Signals need explicit removal;
/// node removal automatically cancels its bindings. No thread synchronization or
/// automatic dependency tracking is provided.
pub struct Runtime<N> {
    tree: Tree<N>,
    signals: SlotMap<SignalKey, SignalEntry>,
    bindings: Bindings<N>,
}

impl<N> Runtime<N> {
    /// Creates a runtime with one root node.
    pub fn new(root: N) -> Self {
        Self {
            tree: Tree::new(root),
            signals: SlotMap::with_key(),
            bindings: Bindings::new(),
        }
    }

    /// An immutable view; structural changes must go through the runtime.
    pub fn tree(&self) -> &Tree<N> {
        &self.tree
    }

    /// Appends a state node beneath a live parent.
    pub fn append(&mut self, parent: NodeId, value: N) -> Result<NodeId, TreeError> {
        self.tree.append(parent, value)
    }

    /// Cancels affected bindings before their node payloads are dropped.
    pub fn remove_subtree(&mut self, node: NodeId) -> Result<usize, TreeError> {
        let Self {
            tree,
            signals,
            bindings,
        } = self;
        tree.remove_subtree_with(node, |node| {
            while let Some(&key) = bindings.by_node.get(&node).and_then(|keys| keys.last()) {
                bindings.remove(key, signals);
            }
        })
    }

    /// Stores an initial value and returns its typed handle.
    pub fn create_signal<S: 'static>(&mut self, value: S) -> Signal<S> {
        let key = self.signals.insert(SignalEntry {
            all_queued: true,
            value: Box::new(value),
            bindings: Vec::new(),
        });
        Signal {
            id: SignalId {
                owner: self.tree.root(),
                key,
            },
            marker: PhantomData,
        }
    }

    /// Borrows the current value of a live signal.
    pub fn read<S: 'static>(&self, signal: Signal<S>) -> Result<&S, StateError> {
        self.signal(signal.id)?
            .value
            .downcast_ref()
            .ok_or(StateError::InvalidSignal)
    }

    /// Replaces a changed value and schedules only its subscribers.
    ///
    /// Returns `false` when `PartialEq` considers the value unchanged. Queue
    /// updates finish before the old value is dropped, including if it panics.
    pub fn set<S: PartialEq + 'static>(
        &mut self,
        signal: Signal<S>,
        value: S,
    ) -> Result<bool, StateError> {
        self.signal(signal.id)?;
        let entry = &mut self.signals[signal.id.key];
        let stored = entry
            .value
            .downcast_mut::<S>()
            .expect("signal type matches its handle");
        if *stored == value {
            return Ok(false);
        }
        let old = std::mem::replace(stored, value);
        if !entry.all_queued {
            for &key in &entry.bindings {
                self.bindings.queue(key);
            }
            entry.all_queued = true;
        }
        drop(old);
        Ok(true)
    }

    /// Registers a callback and queues its initial application for the next flush.
    pub fn bind<S: 'static>(
        &mut self,
        signal: Signal<S>,
        node: NodeId,
        mut apply: impl FnMut(&S, &mut N) + 'static,
    ) -> Result<BindingId, StateError> {
        self.register_binding(
            signal.id,
            node,
            true,
            Box::new(move |value, node| {
                apply(
                    value
                        .downcast_ref::<S>()
                        .expect("binding source type matches"),
                    node,
                );
            }),
        )
    }

    pub(crate) fn observe(
        &mut self,
        signal: SignalId,
        target: NodeId,
        invalidate: impl Fn() + 'static,
    ) -> Result<BindingId, StateError> {
        self.register_binding(signal, target, false, Box::new(move |_, _| invalidate()))
    }

    fn register_binding(
        &mut self,
        signal: SignalId,
        node: NodeId,
        initial: bool,
        apply: Apply<N>,
    ) -> Result<BindingId, StateError> {
        self.signal(signal)?;
        if self.tree.get(node).is_none() {
            return Err(TreeError::InvalidNode(node).into());
        }

        let signal_bindings = &mut self.signals[signal.key].bindings;
        let node_bindings = self.bindings.by_node.entry(node).or_default();
        let key = self.bindings.entries.insert(Binding {
            node,
            signal: signal.key,
            signal_index: signal_bindings.len(),
            node_index: node_bindings.len(),
            pending_index: None,
            apply,
        });
        signal_bindings.push(key);
        node_bindings.push(key);
        if self.bindings.pending.capacity() < self.bindings.entries.len() {
            self.bindings
                .pending
                .reserve(self.bindings.entries.len() - self.bindings.pending.len());
        }
        if initial {
            self.bindings.queue(key);
        } else {
            self.signals[signal.key].all_queued = false;
        }
        Ok(BindingId {
            owner: self.tree.root(),
            key,
        })
    }

    /// Removes a binding and cancels its pending delivery.
    pub fn unbind(&mut self, binding: BindingId) -> Result<(), StateError> {
        if binding.owner != self.tree.root() || !self.bindings.entries.contains_key(binding.key) {
            return Err(StateError::InvalidBinding);
        }
        self.bindings.remove(binding.key, &mut self.signals);
        Ok(())
    }

    /// Removes a signal and all its bindings, including queued applications.
    pub fn remove_signal<S: 'static>(&mut self, signal: Signal<S>) -> Result<(), StateError> {
        self.remove_signal_by_id(signal.id)
    }

    /// Removes a signal by identity, including its bindings and pending work.
    pub fn remove_signal_by_id(&mut self, id: SignalId) -> Result<(), StateError> {
        self.signal(id)?;
        while let Some(&key) = self.signals[id.key].bindings.last() {
            self.bindings.remove(key, &mut self.signals);
        }
        self.signals.remove(id.key);
        Ok(())
    }

    /// Applies pending bindings with the latest values, without scanning the tree.
    ///
    /// A panicking callback is consumed; remaining tasks stay pending. Changes to
    /// its target are not rolled back and the panic propagates to the caller.
    pub fn flush(&mut self) -> usize {
        let mut applied = 0;
        while let Some(key) = self.bindings.pending.pop() {
            let binding = &mut self.bindings.entries[key];
            binding.pending_index = None;
            let signal = &mut self.signals[binding.signal];
            signal.all_queued = false;
            let value = &*signal.value;
            let node = self
                .tree
                .get_mut(binding.node)
                .expect("binding target is live");
            (binding.apply)(value, node);
            applied += 1;
        }
        applied
    }

    /// Returns the number of pending updates tracked by the runtime.
    pub fn pending_count(&self) -> usize {
        self.bindings.pending.len()
    }

    /// Returns the number of live signal bindings.
    pub fn binding_count(&self) -> usize {
        self.bindings.entries.len()
    }

    /// Returns the number of live signals across all value types.
    pub fn signal_count(&self) -> usize {
        self.signals.len()
    }

    fn signal(&self, id: SignalId) -> Result<&SignalEntry, StateError> {
        if id.owner != self.tree.root() {
            return Err(StateError::InvalidSignal);
        }
        self.signals.get(id.key).ok_or(StateError::InvalidSignal)
    }
}
