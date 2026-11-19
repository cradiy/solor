//! A rooted, mutable tree with stable node handles and ordered children.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use slotmap::{SlotMap, new_key_type};

new_key_type! {
    struct NodeKey;
}

static NEXT_TREE_ID: AtomicU64 = AtomicU64::new(1);

/// A process-local node handle belonging to exactly one tree.
///
/// Live handles survive storage growth. Removed handles are rejected subject to
/// SlotMap's finite generation counter; they must not be used as durable IDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId {
    tree: u64,
    key: NodeKey,
}

/// A rejected structural operation. The tree is unchanged on these errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeError {
    /// The handle is stale or belongs to another tree.
    InvalidNode(NodeId),
    /// The root must live as long as the tree.
    CannotRemoveRoot,
    /// The insertion position exceeds the current number of children.
    InvalidChildIndex(usize),
}

impl fmt::Display for TreeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidNode(id) => write!(formatter, "invalid node handle: {id:?}"),
            Self::InvalidChildIndex(index) => {
                write!(formatter, "invalid child insertion index {index}")
            }
            Self::CannotRemoveRoot => formatter.write_str("cannot remove the tree root"),
        }
    }
}

impl std::error::Error for TreeError {}

struct Node<T> {
    value: T,
    parent: Option<NodeKey>,
    children: Vec<NodeKey>,
}

/// Owns one root and its descendants. Topology is independent of payloads.
///
/// Children retain insertion order. Payload edits do not schedule UI updates.
/// Storage retains capacity after removal. Dropping the tree does not recurse
/// through its topology.
pub struct Tree<T> {
    id: u64,
    root: NodeKey,
    nodes: SlotMap<NodeKey, Node<T>>,
}

impl<T> Tree<T> {
    /// Creates a tree with a permanent root.
    ///
    /// # Panics
    ///
    /// Panics if the process exhausts its 64-bit tree identifier space.
    pub fn new(value: T) -> Self {
        let id = NEXT_TREE_ID
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .expect("tree identifier space exhausted");
        let mut nodes = SlotMap::with_key();
        let root = nodes.insert(Node {
            value,
            parent: None,
            children: Vec::new(),
        });
        Self { id, root, nodes }
    }

    /// Returns the permanent root handle.
    pub fn root(&self) -> NodeId {
        self.handle(self.root)
    }

    /// Returns the number of live nodes, including the root.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Returns a payload, or `None` for a stale or foreign handle.
    pub fn get(&self, id: NodeId) -> Option<&T> {
        self.node(id).ok().map(|node| &node.value)
    }

    /// Returns a mutable payload without exposing structural links.
    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut T> {
        if id.tree != self.id {
            return None;
        }
        self.nodes.get_mut(id.key).map(|node| &mut node.value)
    }

    /// Returns the parent handle, or `None` for the root.
    pub fn parent(&self, id: NodeId) -> Result<Option<NodeId>, TreeError> {
        Ok(self.node(id)?.parent.map(|key| self.handle(key)))
    }

    /// Iterates immediate children in sibling order without allocating.
    pub fn children(
        &self,
        id: NodeId,
    ) -> Result<impl DoubleEndedIterator<Item = NodeId> + ExactSizeIterator + '_, TreeError> {
        Ok(self.node(id)?.children.iter().map(|&key| self.handle(key)))
    }

    /// Appends a child. An invalid parent is rejected before allocating a node.
    pub fn append(&mut self, parent: NodeId, value: T) -> Result<NodeId, TreeError> {
        self.insert_child(parent, None, value)
    }

    /// Inserts a new child at an ordered position, preserving sibling handles.
    pub fn insert(&mut self, parent: NodeId, index: usize, value: T) -> Result<NodeId, TreeError> {
        self.insert_child(parent, Some(index), value)
    }

    fn insert_child(
        &mut self,
        parent: NodeId,
        index: Option<usize>,
        value: T,
    ) -> Result<NodeId, TreeError> {
        let count = self.node(parent)?.children.len();
        let index = index.unwrap_or(count);
        if index > count {
            return Err(TreeError::InvalidChildIndex(index));
        }
        let key = self.nodes.insert(Node {
            value,
            parent: Some(parent.key),
            children: Vec::new(),
        });
        self.nodes[parent.key].children.insert(index, key);
        Ok(self.handle(key))
    }

    /// Removes a non-root subtree and returns the number of removed nodes.
    ///
    /// Takes O(removed nodes + siblings of the subtree root) time and O(1)
    /// auxiliary storage. Payloads are dropped leaf-first. If a payload destructor
    /// panics, removal can be partial, but the remaining topology is valid.
    pub fn remove_subtree(&mut self, id: NodeId) -> Result<usize, TreeError> {
        self.remove_subtree_with(id, |_| {})
    }

    pub(crate) fn remove_subtree_with(
        &mut self,
        id: NodeId,
        mut before_remove: impl FnMut(NodeId),
    ) -> Result<usize, TreeError> {
        if self.node(id)?.parent.is_none() {
            return Err(TreeError::CannotRemoveRoot);
        }

        let mut current = id.key;
        let mut removed = 0;
        loop {
            if let Some(&child) = self.nodes[current].children.last() {
                current = child;
                continue;
            }

            let parent = self.nodes[current]
                .parent
                .expect("a removable node has a parent");
            before_remove(self.handle(current));
            if current == id.key {
                self.nodes[parent].children.retain(|&key| key != current);
            } else {
                let last = self.nodes[parent].children.pop();
                debug_assert_eq!(last, Some(current));
            }

            let node = self.nodes.remove(current).expect("a linked node is live");
            removed += 1;
            drop(node);
            if current == id.key {
                return Ok(removed);
            }
            current = parent;
        }
    }

    fn handle(&self, key: NodeKey) -> NodeId {
        NodeId { tree: self.id, key }
    }

    fn node(&self, id: NodeId) -> Result<&Node<T>, TreeError> {
        if id.tree != self.id {
            return Err(TreeError::InvalidNode(id));
        }
        self.nodes.get(id.key).ok_or(TreeError::InvalidNode(id))
    }
}
