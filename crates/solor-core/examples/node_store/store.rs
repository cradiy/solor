use std::fmt;
use std::num::NonZeroUsize;

use solor_core::tree::{NodeId, Tree, TreeError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessError {
    Tree(TreeError),
    ActiveNode(NodeId),
    NotChild { parent: NodeId, node: NodeId },
    DepthLimit { limit: usize },
}

impl fmt::Display for AccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tree(error) => error.fmt(f),
            Self::ActiveNode(node) => write!(f, "node is already executing: {node:?}"),
            Self::NotChild { parent, node } => {
                write!(f, "{node:?} is not a direct child of {parent:?}")
            }
            Self::DepthLimit { limit } => write!(f, "node execution depth exceeds {limit}"),
        }
    }
}

impl std::error::Error for AccessError {}

impl From<TreeError> for AccessError {
    fn from(error: TreeError) -> Self {
        Self::Tree(error)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ready,
    Running,
    Interrupted,
}

struct Slot<T: ?Sized> {
    body: Option<Box<T>>,
    interrupted: bool,
}

pub struct NodeStore<T: ?Sized> {
    tree: Tree<Slot<T>>,
    max_depth: NonZeroUsize,
}

impl<T: ?Sized> NodeStore<T> {
    pub fn new(root: Box<T>, max_depth: NonZeroUsize) -> Self {
        Self {
            tree: Tree::new(Slot {
                body: Some(root),
                interrupted: false,
            }),
            max_depth,
        }
    }

    pub fn root(&self) -> NodeId {
        self.tree.root()
    }

    pub fn node_count(&self) -> usize {
        self.tree.node_count()
    }

    pub fn parent(&self, node: NodeId) -> Result<Option<NodeId>, TreeError> {
        self.tree.parent(node)
    }

    #[allow(dead_code, reason = "used by the live layout experiment")]
    pub fn child_count(&self, node: NodeId) -> Result<usize, TreeError> {
        Ok(self.tree.children(node)?.len())
    }

    pub fn append(&mut self, parent: NodeId, body: Box<T>) -> Result<NodeId, TreeError> {
        self.tree.append(
            parent,
            Slot {
                body: Some(body),
                interrupted: false,
            },
        )
    }

    pub fn remove_subtree(&mut self, node: NodeId) -> Result<usize, TreeError> {
        self.tree.remove_subtree(node)
    }

    pub fn status(&self, node: NodeId) -> Result<Status, AccessError> {
        let slot = self.tree.get(node).ok_or(TreeError::InvalidNode(node))?;
        Ok(if slot.body.is_none() {
            Status::Running
        } else if slot.interrupted {
            Status::Interrupted
        } else {
            Status::Ready
        })
    }

    pub fn get(&self, node: NodeId) -> Result<&T, AccessError> {
        self.tree
            .get(node)
            .ok_or(TreeError::InvalidNode(node))?
            .body
            .as_deref()
            .ok_or(AccessError::ActiveNode(node))
    }

    pub fn get_mut(&mut self, node: NodeId) -> Result<&mut T, AccessError> {
        self.tree
            .get_mut(node)
            .ok_or(TreeError::InvalidNode(node))?
            .body
            .as_deref_mut()
            .ok_or(AccessError::ActiveNode(node))
    }

    pub fn execute<R>(
        &mut self,
        node: NodeId,
        operation: impl FnOnce(&mut T, &mut Context<'_, T>) -> Result<R, AccessError>,
    ) -> Result<R, AccessError> {
        self.execute_at(node, 1, operation)
    }

    fn execute_at<R>(
        &mut self,
        node: NodeId,
        depth: usize,
        operation: impl FnOnce(&mut T, &mut Context<'_, T>) -> Result<R, AccessError>,
    ) -> Result<R, AccessError> {
        if depth > self.max_depth.get() {
            return Err(AccessError::DepthLimit {
                limit: self.max_depth.get(),
            });
        }
        let body = self
            .tree
            .get_mut(node)
            .ok_or(TreeError::InvalidNode(node))?
            .body
            .take()
            .ok_or(AccessError::ActiveNode(node))?;
        let mut loan = Loan {
            store: self,
            node,
            body: Some(body),
            completed: false,
        };
        let result = operation(
            loan.body.as_deref_mut().expect("loan owns the payload"),
            &mut Context {
                store: &mut *loan.store,
                current: node,
                depth,
            },
        );
        loan.completed = result.is_ok();
        result
    }
}

struct Loan<'a, T: ?Sized> {
    store: &'a mut NodeStore<T>,
    node: NodeId,
    body: Option<Box<T>>,
    completed: bool,
}

impl<T: ?Sized> Drop for Loan<'_, T> {
    fn drop(&mut self) {
        let slot = self
            .store
            .tree
            .get_mut(self.node)
            .expect("active nodes cannot be removed");
        debug_assert!(slot.body.is_none());
        slot.body = self.body.take();
        slot.interrupted = !self.completed;
    }
}

/// Grants execution of direct children and read-only topology/status access.
/// No store reference, payload reference, or structural mutation API is exposed.
pub struct Context<'a, T: ?Sized> {
    store: &'a mut NodeStore<T>,
    current: NodeId,
    depth: usize,
}

impl<T: ?Sized> Context<'_, T> {
    pub fn current(&self) -> NodeId {
        self.current
    }

    pub fn parent(&self) -> Option<NodeId> {
        self.store
            .parent(self.current)
            .expect("current node is live")
    }

    pub fn status(&self, node: NodeId) -> Result<Status, AccessError> {
        self.store.status(node)
    }

    pub fn child_count(&self) -> usize {
        self.store
            .tree
            .children(self.current)
            .expect("current node is live")
            .len()
    }

    pub fn child(&self, index: usize) -> Option<NodeId> {
        self.store
            .tree
            .children(self.current)
            .expect("current node is live")
            .nth(index)
    }

    pub fn execute_child<R>(
        &mut self,
        node: NodeId,
        operation: impl FnOnce(&mut T, &mut Context<'_, T>) -> Result<R, AccessError>,
    ) -> Result<R, AccessError> {
        if self.store.status(node)? == Status::Running {
            return Err(AccessError::ActiveNode(node));
        }
        if self.store.tree.parent(node)? != Some(self.current) {
            return Err(AccessError::NotChild {
                parent: self.current,
                node,
            });
        }
        let depth = self.depth.checked_add(1).ok_or(AccessError::DepthLimit {
            limit: self.store.max_depth.get(),
        })?;
        self.store.execute_at(node, depth, operation)
    }
}
