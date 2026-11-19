//! Component composition, retained elements and explicit update boundaries.
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use crate::layout::Scene;
use crate::state::{Runtime, SignalId};
use anyhow::Result;

mod column;
mod element;
mod fill;
pub mod layout;
mod length;
pub mod node_store;
mod pixels;
pub mod platform;
pub mod pointer;
pub mod raster;
mod shared_string;
pub mod state;
pub mod style;
mod styled_body;
/// Shared font shaping and retained text measurement.
pub mod text;
pub mod tree;

pub use crate::layout::Edges;
pub use crate::{state::Signal, tree::NodeId};
pub use element::{
    AnyElement, Div, Draw, Element, IntoElement, Node, PaintContext, Render, TextContent, div,
    element, text,
};
pub use fill::{
    ColorSpace, ColorStop, Fill, Gradient, GradientError, GradientKind, Rgba, color_stop,
    gradient_bottom_to_top, gradient_left_to_right, gradient_right_to_left, gradient_top_to_bottom,
    linear_gradient, rgb, rgba,
};
use length::UnitResolver;
pub use length::{AbsoluteLength, Length, percent, rem};
pub use pixels::{Pixels, px};
pub use shared_string::SharedString;
pub use style::{Style, StyleRefinement, Styled};

use element::{Child, Factory, Installation, MountedNode, Watch};
use styled_body::StyledBody;
use text::{Engine, SharedEngine, Text};

#[cfg(test)]
#[path = "../tests/unit/mod.rs"]
mod tests;

struct Mount<'a> {
    state: &'a mut Runtime<()>,
    owned: Vec<SignalId>,
}

impl Mount<'_> {
    fn context<'a>(&'a mut self, engine: Option<&'a SharedEngine>) -> Context<'a> {
        Context {
            state: self.state,
            engine,
            owned: Some(&mut self.owned),
        }
    }
}

impl Drop for Mount<'_> {
    fn drop(&mut self) {
        for signal in self.owned.drain(..) {
            self.state
                .remove_signal_by_id(signal)
                .expect("live mount signal");
        }
    }
}

/// State access during an application operation, component mount, or event.
pub struct Context<'a> {
    engine: Option<&'a SharedEngine>,
    state: &'a mut Runtime<()>,
    // Application-owned signals live with the runtime. Component scopes retire theirs together.
    owned: Option<&'a mut Vec<SignalId>>,
}

impl Context<'_> {
    /// Creates a typed signal owned by the active mount or application.
    pub fn signal<T: 'static>(&mut self, initial: T) -> Signal<T> {
        let signal = self.state.create_signal(initial);
        if let Some(owned) = &mut self.owned {
            owned.push(signal.id());
        }
        signal
    }

    /// Borrows the signal's current value.
    pub fn read<T: 'static>(&self, signal: Signal<T>) -> Result<&T> {
        Ok(self.state.read(signal)?)
    }

    /// Writes a value and queues subscribers when it differs.
    pub fn set<T: PartialEq + 'static>(&mut self, signal: Signal<T>, value: T) -> Result<()> {
        self.state.set(signal, value)?;
        Ok(())
    }

    /// Edits a clone of the value and publishes it on change.
    pub fn update<T: Clone + PartialEq + 'static>(
        &mut self,
        signal: Signal<T>,
        change: impl FnOnce(&mut T),
    ) -> Result<()> {
        let mut value = self.read(signal)?.clone();
        change(&mut value);
        self.set(signal, value)
    }
}

/// A component whose factory runs once per attachment.
pub struct Component<F>(F);

/// Declares a component using a closure.
pub fn component<E: IntoElement, F: FnOnce(&mut Context<'_>) -> E>(factory: F) -> Component<F> {
    Component(factory)
}

impl<E: IntoElement, F: FnOnce(&mut Context<'_>) -> E> Render for Component<F> {
    fn render(self, cx: &mut Context<'_>) -> impl IntoElement {
        (self.0)(cx)
    }
}

// Indices in the declaration stay fixed while live sibling indices change.
type Path = Vec<usize>;
type Updates = BTreeMap<Path, bool>;
type Pending = Rc<RefCell<HashMap<NodeId, Updates>>>;

struct Branch {
    target: NodeId,
    make: Factory,
}

type Repaints = Rc<RefCell<HashSet<NodeId>>>;
type LabelUpdates = Rc<RefCell<HashMap<(ViewId, NodeId), Signal<SharedString>>>>;

struct Instance {
    pointer: pointer::PointerRouter,
    units: UnitResolver,
    repaints: Repaints,
    scene: Scene,
    bodies: HashMap<NodeId, MountedNode>,
    nodes: BTreeMap<Path, NodeId>,
    branches: BTreeMap<Path, Branch>,
}

struct Attachment<'a> {
    scope: NodeId,
    state: &'a mut Runtime<()>,
    pending: &'a Pending,
    labels: &'a LabelUpdates,
    engine: Option<&'a SharedEngine>,
}

impl Instance {
    fn update_label(&mut self, node: NodeId, value: &SharedString) -> Result<()> {
        if self.scene.body::<StyledBody<Text>>(node)?.content.content() != value.as_str() {
            Text::validate_content(value)?;
            self.scene.edit_layout::<StyledBody<Text>>(node, |body| {
                body.content.replace_content(value.clone())
            })?;
            self.repaints.borrow_mut().insert(node);
        }
        Ok(())
    }

    fn dispose(self, state: &mut Runtime<()>) -> Result<()> {
        for node in self.bodies.into_values() {
            for signal in node.owned {
                state.remove_signal_by_id(signal)?;
            }
        }
        Ok(())
    }

    fn finish_attachment(
        &mut self,
        parent: NodeId,
        path: &[usize],
        watches: Vec<Watch>,
        children: Vec<Child>,
        cx: &mut Attachment<'_>,
    ) -> Result<()> {
        if !watches.is_empty() {
            let target = cx.state.append(cx.scope, ())?;
            self.bodies
                .get_mut(&parent)
                .expect("mounted node")
                .binding_target = Some(target);
            for watch in watches {
                watch(cx, target, parent, &self.repaints)?;
            }
        }
        self.register_children(parent, path, children, cx)
    }

    fn register_children(
        &mut self,
        parent: NodeId,
        path: &[usize],
        children: Vec<Child>,
        cx: &mut Attachment<'_>,
    ) -> Result<()> {
        let mut index = 0;
        for (offset, child) in children.into_iter().enumerate() {
            let mut position = path.to_vec();
            position.push(offset);
            match child {
                Child::Fixed(element) => {
                    self.insert(parent, index, &position, element, cx)?;
                    index += 1;
                }
                Child::When(signal, make) => {
                    let target = cx.state.append(cx.scope, ())?;
                    let pending = cx.pending.clone();
                    let key = position.clone();
                    let scope = cx.scope;
                    if let Err(error) = cx.state.bind(signal, target, move |value, _| {
                        pending
                            .borrow_mut()
                            .entry(scope)
                            .or_default()
                            .insert(key.clone(), *value);
                    }) {
                        cx.state.remove_subtree(target)?;
                        return Err(error.into());
                    }
                    self.branches.insert(position, Branch { target, make });
                }
            }
        }
        Ok(())
    }

    fn insert(
        &mut self,
        parent: NodeId,
        index: usize,
        path: &[usize],
        element: AnyElement,
        cx: &mut Attachment<'_>,
    ) -> Result<()> {
        let mut mount = Mount {
            state: cx.state,
            owned: Vec::new(),
        };
        let Installation {
            mut node,
            children,
            watches,
        } = element.attach(
            &mut self.scene,
            parent,
            index,
            self.bodies[&parent].text_style.clone(),
            self.units,
            &mut mount.context(cx.engine),
        )?;
        node.owned = std::mem::take(&mut mount.owned);
        drop(mount);
        node.offset = *path.last().expect("child position");
        let id = node.id;
        self.bodies.insert(id, node);
        self.nodes.insert(path.to_vec(), id);
        self.finish_attachment(id, path, watches, children, cx)?;
        Ok(())
    }

    fn remove(&mut self, path: &Path, cx: &mut Attachment<'_>) -> Result<()> {
        let Some(node) = self.nodes.get(path) else {
            return Ok(());
        };
        self.scene.unmount_subtree(*node)?;
        // Keep this slot's subscription so a later opening can mount fresh content.
        for key in subtree_keys(&self.branches, path)
            .into_iter()
            .filter(|key| key != path)
        {
            let branch = self.branches.remove(&key).expect("registered branch");
            cx.state.remove_subtree(branch.target)?;
        }
        for key in subtree_keys(&self.nodes, path) {
            let node = self.nodes.remove(&key).expect("mounted node");
            let node = self.bodies.remove(&node).expect("mounted body");
            self.pointer.retire(node.id);
            if let Some(target) = node.binding_target {
                cx.state.remove_subtree(target)?;
                self.repaints.borrow_mut().remove(&node.id);
                cx.labels.borrow_mut().remove(&(ViewId(cx.scope), node.id));
            }
            for signal in node.owned {
                cx.state.remove_signal_by_id(signal)?;
            }
        }
        Ok(())
    }

    fn reconcile(&mut self, path: &Path, open: bool, cx: &mut Attachment<'_>) -> Result<()> {
        let Some(branch) = self.branches.get(path) else {
            return Ok(());
        };
        if self.nodes.contains_key(path) == open {
            return Ok(());
        }
        if !open {
            return self.remove(path, cx);
        }
        let element = (branch.make)();
        let (&offset, parent_path) = path.split_last().expect("child position");
        let parent = self.nodes[parent_path];
        // Live children stay sorted by their immutable declaration offsets.
        let mut index = 0;
        let mut end = self.scene.children(parent)?.len();
        while index < end {
            let mid = index + (end - index) / 2;
            let child = self
                .scene
                .children(parent)?
                .nth(mid)
                .expect("live child index");
            if self.bodies[&child].offset < offset {
                index = mid + 1;
            } else {
                end = mid;
            }
        }
        if let Err(error) = self.insert(parent, index, path, element, cx) {
            self.remove(path, cx)?;
            return Err(error);
        }
        Ok(())
    }
}

fn subtree_keys<V>(map: &BTreeMap<Path, V>, path: &Path) -> Vec<Path> {
    map.range(path.clone()..)
        .map(|(key, _)| key)
        .take_while(|key| key.starts_with(path))
        .cloned()
        .collect()
}

/// Identifies a mounted component tree within its UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ViewId(NodeId);

/// Owns the application signals and its mounted component trees.
pub struct Ui {
    state: Runtime<()>,
    engine: Option<SharedEngine>,
    instances: HashMap<ViewId, Instance>,
    pending: Pending,
    labels: LabelUpdates,
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            state: Runtime::new(()),
            engine: None,
            instances: HashMap::new(),
            pending: Pending::default(),
            labels: LabelUpdates::default(),
        }
    }
}

impl Ui {
    /// Creates a UI for custom primitives and containers.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enables text elements using a single caller-supplied font.
    pub fn with_font(bytes: Vec<u8>, locale: &str) -> Result<Self> {
        Ok(Self {
            engine: Some(Engine::from_font(bytes, locale)?),
            ..Self::default()
        })
    }

    /// Renders once and attaches the declaration. Flush to initialize conditional children.
    pub fn mount(&mut self, item: impl Render) -> Result<ViewId> {
        self.mount_with_rem_size(item, px(16.0))
    }

    /// Attaches a view with its own positive, finite logical-pixel basis for rem lengths.
    pub fn mount_with_rem_size(&mut self, item: impl Render, rem_size: Pixels) -> Result<ViewId> {
        let units = UnitResolver::new(rem_size)?;
        let mut mount = Mount {
            state: &mut self.state,
            owned: Vec::new(),
        };
        let mut cx = mount.context(self.engine.as_ref());
        let item = item.render(&mut cx).into_element();
        let (
            scene,
            Installation {
                node: mut root_node,
                children,
                watches,
            },
        ) = item.root(&mut cx, units)?;
        let scope = mount.state.append(mount.state.tree().root(), ())?;
        root_node.owned = std::mem::take(&mut mount.owned);
        drop(mount);
        let root = scene.root();
        let mut instance = Instance {
            pointer: Default::default(),
            units,
            repaints: Repaints::default(),
            scene,
            bodies: HashMap::from([(root, root_node)]),
            nodes: BTreeMap::from([(Vec::new(), root)]),
            branches: BTreeMap::new(),
        };
        let mut cx = Attachment {
            scope,
            state: &mut self.state,
            pending: &self.pending,
            labels: &self.labels,
            engine: self.engine.as_ref(),
        };
        if let Err(error) = instance.finish_attachment(root, &[], watches, children, &mut cx) {
            self.state.remove_subtree(scope)?;
            instance.dispose(&mut self.state)?;
            return Err(error);
        }
        let view = ViewId(scope);
        self.instances.insert(view, instance);
        Ok(view)
    }

    /// Opens an application-owned state context.
    pub fn context(&mut self) -> Context<'_> {
        Context {
            state: &mut self.state,
            engine: self.engine.as_ref(),
            owned: None,
        }
    }

    /// Dispatches to the registered handler and reports whether it ran.
    pub fn click(&mut self, scope: ViewId, target: NodeId) -> Result<bool> {
        let Some(instance) = self.instances.get_mut(&scope) else {
            return Ok(false);
        };
        let Some(node) = instance.bodies.get_mut(&target) else {
            return Ok(false);
        };
        let Some(click) = node.handlers.as_deref().and_then(|h| h.click.as_ref()) else {
            return Ok(false);
        };
        click(&mut Context {
            state: &mut self.state,
            engine: self.engine.as_ref(),
            owned: Some(&mut node.owned),
        })?;
        Ok(true)
    }

    /// Applies structural, label and paint bindings. Returns the number of branch positions visited.
    pub fn flush(&mut self) -> Result<usize> {
        let mut visited = 0;
        loop {
            self.state.flush();
            let batch = std::mem::take(&mut *self.pending.borrow_mut());
            if batch.is_empty() {
                self.flush_labels()?;
                return Ok(visited);
            }
            let mut failure = None;
            for (scope, updates) in batch {
                let instance = self
                    .instances
                    .get_mut(&ViewId(scope))
                    .expect("queued scope is live");
                let mut cx = Attachment {
                    scope,
                    state: &mut self.state,
                    pending: &self.pending,
                    labels: &self.labels,
                    engine: self.engine.as_ref(),
                };
                // Lexicographic paths apply parent removals before updates to their descendants.
                for (path, open) in updates {
                    visited += 1;
                    if let Err(error) = instance.reconcile(&path, open, &mut cx) {
                        self.pending
                            .borrow_mut()
                            .entry(scope)
                            .or_default()
                            .insert(path, open);
                        failure.get_or_insert(error);
                    }
                }
            }
            if let Some(error) = failure {
                return Err(error);
            }
            // A newly mounted subtree can register initial bindings in this same boundary.
        }
    }

    fn flush_labels(&mut self) -> Result<()> {
        let mut labels = self.labels.borrow_mut();
        if labels.is_empty() {
            return Ok(());
        }
        let mut failure = None;
        labels.retain(|&(view, id), source| {
            let Ok(value) = self.state.read(*source) else {
                return false;
            };
            let instance = self.instances.get_mut(&view).expect("live label view");
            if let Err(error) = instance.update_label(id, value) {
                failure.get_or_insert(error);
                return true;
            }
            false
        });
        failure.map_or(Ok(()), Err)
    }

    /// Appends nodes with outstanding signal-driven paint requests in unspecified order.
    /// Successful local recording acknowledges a node. The caller can reuse its output buffer.
    pub fn collect_repaints(&self, view: ViewId, nodes: &mut Vec<NodeId>) -> Result<()> {
        let ready = self.instance(view)?.repaints.borrow();
        nodes.extend(ready.iter().copied());
        Ok(())
    }

    /// Returns the retained root of a mounted view.
    pub fn root(&self, view: ViewId) -> Result<NodeId> {
        Ok(self.instance(view)?.scene.root())
    }

    /// Iterates over mounted direct children in layout order.
    pub fn children(
        &self,
        view: ViewId,
        parent: NodeId,
    ) -> Result<impl ExactSizeIterator<Item = NodeId> + '_> {
        Ok(self.instance(view)?.scene.children(parent)?)
    }

    fn instance(&self, view: ViewId) -> Result<&Instance> {
        self.instances
            .get(&view)
            .ok_or_else(|| crate::tree::TreeError::InvalidNode(view.0).into())
    }

    /// Applies the viewport allocation and returns the number of changed geometry entries.
    /// Call flush first to apply queued structural changes.
    pub fn layout(&mut self, view: ViewId, size: layout::Size) -> Result<usize> {
        let instance = self
            .instances
            .get_mut(&view)
            .ok_or(crate::tree::TreeError::InvalidNode(view.0))?;
        Ok(instance.scene.run_assigned(size, false)?)
    }

    /// Published geometry in parent-local coordinates after a completed layout.
    pub fn geometry(&self, view: ViewId, node: NodeId) -> Option<layout::Geometry> {
        let scene = &self.instances.get(&view)?.scene;
        if scene.is_pending() {
            return None;
        }
        scene.geometry(node)
    }

    /// Appends one node's local commands. The caller selects the nodes to repaint.
    pub fn paint(&self, view: ViewId, node: NodeId, commands: &mut Vec<Draw>) -> Result<()> {
        let instance = self.instance(view)?;
        let body = instance
            .bodies
            .get(&node)
            .ok_or(crate::tree::TreeError::InvalidNode(node))?;
        body.paint(&instance.scene, &self.state, commands)?;
        if body.binding_target.is_some() {
            instance.repaints.borrow_mut().remove(&node);
        }
        Ok(())
    }

    /// Retires a mounted tree, its local signals and subscriptions.
    pub fn unmount(&mut self, scope: ViewId) -> Result<()> {
        if !self.instances.contains_key(&scope) {
            return Err(crate::tree::TreeError::InvalidNode(scope.0).into());
        }
        self.state.remove_subtree(scope.0)?;
        self.pending.borrow_mut().remove(&scope.0);
        self.labels
            .borrow_mut()
            .retain(|(view, _), _| *view != scope);
        let instance = self.instances.remove(&scope).expect("live mounted scope");
        instance.dispose(&mut self.state)
    }
}
