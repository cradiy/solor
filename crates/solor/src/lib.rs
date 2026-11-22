//! Component composition, retained elements and explicit update boundaries.
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use crate::layout::Scene;
use crate::state::{BindingId, Runtime, SignalId};
use anyhow::Result;

macro_rules! impl_event_context {
    ($name:ident) => {
        impl<'a> std::ops::Deref for $name<'a> {
            type Target = crate::Context<'a>;

            fn deref(&self) -> &Self::Target {
                &self.context
            }
        }

        impl std::ops::DerefMut for $name<'_> {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.context
            }
        }
    };
}

mod tasks;
pub use tasks::AsyncContext;
use tasks::{TaskScope, Tasks};
#[cfg(feature = "url")]
pub use url::Url;
mod assets;
pub use assets::{AssetSource, Assets, ImageResource};
mod clip;
pub use clip::ContentClip;
mod column;
mod element;
mod fill;
mod frame;
pub use frame::ViewFrame;
pub mod keyboard;
pub mod layout;
mod length;
pub mod node_store;
mod pixels;
pub mod platform;
pub mod pointer;
pub mod raster;
mod scroll_position;
mod scrollbar;
pub use scrollbar::{
    Scrollbar, ScrollbarPaintState, ScrollbarThumbStyle, horizontal_scrollbar, scrollbar,
};
mod shared_string;
pub mod state;
pub mod style;
mod styled_body;
/// Shared font shaping and retained text measurement.
pub mod text;
pub mod tree;
mod uniform_list;
pub use uniform_list::{
    ListItems, ListScrollHandle, ScrollAlignment, VirtualList, uniform_list, variable_list,
};

pub use crate::layout::Edges;
pub use crate::{
    state::{Signal, SignalVec},
    tree::NodeId,
};
mod bitmap;
#[cfg(feature = "image")]
pub use bitmap::ImageFile;
pub use bitmap::{CustomImage, ImageAsset, ImageCache, ImageFill, ImageRegion, ImageRenderer};
pub use element::image::{ImageSource, image};
pub use element::svg::{Svg, SvgAsset, SvgSource, svg};
pub use element::{
    AnyElement, Children, Div, Draw, Element, IntoElement, Node, PaintContext, ParentElement,
    RefreshMode, RefreshPolicy, Render, TextContent, div, element, text,
};
pub use fill::{
    Checkerboard, ColorSpace, ColorStop, Fill, Gradient, GradientError, GradientKind, GradientMap,
    Ink, Rgba, checkerboard, color_stop, gradient_bottom_to_top, gradient_left_to_right,
    gradient_right_to_left, gradient_top_to_bottom, linear_gradient, rgb, rgba,
};
use length::UnitResolver;
pub use length::{AbsoluteLength, Length, percent, rem};
pub use pixels::{Pixels, px};
pub use platform::{ResizeEdge, WindowControl};
pub use pointer::{
    ScrollHandle, ScrollMetrics, Tooltip, TooltipAlignment, TooltipPlacement, TooltipStyle,
};
pub use scroll_position::{ListScrollPosition, ScrollAxis, ScrollPosition};
pub use shared_string::SharedString;
pub use style::{Style, StyleRefinement, Styled};

use element::{Child, Factory, Installation, MountedNode, Watch};
use styled_body::StyledBody;
pub use text::{FontFamily, Fonts, GenericFontFamily};
use text::{Text, TextSystem};

#[cfg(test)]
#[path = "../tests/unit/mod.rs"]
mod tests;

struct Mount<'a> {
    state: &'a mut Runtime<()>,
    owned: Owned,
}

#[derive(Default)]
struct Owned {
    signals: Vec<SignalId>,
    tasks: TaskScope,
}

impl Mount<'_> {
    fn context<'a>(&'a mut self, engine: &'a TextSystem, tasks: &'a Tasks) -> Context<'a> {
        Context {
            state: self.state,
            tasks,
            engine,
            owned: Some(&mut self.owned),
        }
    }
}

impl Drop for Mount<'_> {
    fn drop(&mut self) {
        for signal in self.owned.signals.drain(..) {
            self.state
                .remove_signal_by_id(signal)
                .expect("live mount signal");
        }
    }
}

/// State access during an application operation, component mount, or event.
pub struct Context<'a> {
    tasks: &'a Tasks,
    engine: &'a TextSystem,
    state: &'a mut Runtime<()>,
    // Application-owned signals live with the runtime. Component scopes retire theirs together.
    owned: Option<&'a mut Owned>,
}

impl Context<'_> {
    /// Creates a typed signal owned by the active mount or application.
    pub fn signal<T: 'static>(&mut self, initial: T) -> Signal<T> {
        let signal = self.state.create_signal(initial);
        if let Some(owned) = &mut self.owned {
            owned.signals.push(signal.id());
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

    /// Edits the stored value directly and publishes it unconditionally at the next flush.
    pub fn update_in_place<T: 'static, R>(
        &mut self,
        signal: Signal<T>,
        edit: impl FnOnce(&mut T) -> R,
    ) -> Result<R> {
        Ok(self.state.update_in_place(signal, edit)?)
    }

    /// Schedules subscribers after the application mutates shared signal data separately.
    pub fn notify<T: 'static>(&mut self, signal: Signal<T>) -> Result<()> {
        Ok(self.state.notify(signal)?)
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

#[derive(Default)]
struct PaintState {
    dirty: HashSet<NodeId>,
    commands: HashMap<NodeId, Vec<Draw>>,
}

impl PaintState {
    fn invalidate(&mut self, node: NodeId) {
        self.commands.remove(&node);
        self.dirty.insert(node);
    }
}

type Repaints = Rc<RefCell<PaintState>>;
type VisibilityBatch = BTreeMap<Rc<[usize]>, (NodeId, bool)>;
type VisibilityUpdates = Rc<RefCell<HashMap<ViewId, VisibilityBatch>>>;
type Restyle = Rc<dyn Fn(&mut Instance, NodeId, &mut Runtime<()>, bool) -> Result<()>>;
type StyleUpdates = Rc<RefCell<HashMap<(ViewId, NodeId), (Restyle, bool)>>>;
#[derive(Clone, Copy)]
enum ContentUpdate {
    Text(Signal<SharedString>),
    Image(Signal<ImageAsset>),
    #[cfg(feature = "network")]
    RemoteImage(Signal<Option<ImageAsset>>),
    Svg(Signal<SvgAsset>, bool),
    CustomImage(Signal<CustomImage>),
}
type ContentUpdates = Rc<RefCell<HashMap<(ViewId, NodeId), ContentUpdate>>>;

struct Instance {
    focused: Option<NodeId>,
    tab_order: Option<keyboard::TabOrder>,
    frame_handlers: HashMap<NodeId, element::FrameHandler>,
    scope: NodeId,
    paint_reads: RefCell<Vec<SignalId>>,
    observers: RefCell<HashMap<NodeId, Vec<(SignalId, BindingId)>>>,
    pointer: pointer::PointerRouter,
    fixed_draws: frame::FixedDraws,
    units: UnitResolver,
    repaints: Repaints,
    scene: Scene,
    bodies: HashMap<NodeId, MountedNode>,
    visible_nodes: usize,
    nodes: BTreeMap<Path, NodeId>,
    branches: BTreeMap<Path, Branch>,
    lists: BTreeMap<Path, uniform_list::UpdateList>,
    tracked_scroll: HashMap<NodeId, pointer::TrackedScroll>,
}

struct Attachment<'a> {
    tasks: &'a Tasks,
    assets: &'a assets::Resources,
    scope: NodeId,
    state: &'a mut Runtime<()>,
    pending: &'a Pending,
    styles: &'a StyleUpdates,
    content_updates: &'a ContentUpdates,
    visibility: &'a VisibilityUpdates,
    engine: &'a TextSystem,
}

impl Instance {
    fn layout_pass(&mut self, size: layout::Size) -> Result<(usize, bool)> {
        let layout_pending = self.scene.is_pending();
        let changed = self.scene.run_assigned(size, false)?;
        if changed != 0 {
            self.pointer.scroll_offsets.retain(|id, offset| {
                self.scene.geometry(*id).is_none_or(|geometry| {
                    self.bodies[id].scroll_axes.clamp_offset(offset, geometry)
                })
            });
            self.invalidate_geometry();
        } else if layout_pending {
            // Custom layout can change internal paint data at the same published size.
            self.repaints.borrow_mut().commands.clear();
        }
        Ok((changed, layout_pending || changed != 0))
    }

    fn observe_paint(&self, node: NodeId, runtime: &mut Runtime<()>) -> Result<()> {
        let reads = self.paint_reads.borrow();
        let mut observers = self.observers.borrow_mut();
        if reads.is_empty() && !observers.contains_key(&node) {
            return Ok(());
        }
        let bindings = observers.entry(node).or_default();
        bindings.retain(|(source, binding)| {
            if reads.contains(source) {
                true
            } else {
                // Source disposal may already have removed this binding.
                let _ = runtime.unbind(*binding);
                false
            }
        });
        for &source in reads.iter() {
            if !bindings.iter().any(|(id, _)| *id == source) {
                let binding = element::PaintSubscription::observe(
                    runtime,
                    source,
                    self.scope,
                    node,
                    &self.repaints,
                )?;
                bindings.push((source, binding));
            }
        }
        if bindings.is_empty() {
            observers.remove(&node);
        }
        Ok(())
    }

    fn unobserve_paint(&mut self, node: NodeId, state: &mut Runtime<()>) {
        if let Some(bindings) = self.observers.get_mut().remove(&node) {
            for (_, binding) in bindings {
                let _ = state.unbind(binding);
            }
        }
    }

    fn invalidate_geometry(&mut self) {
        self.pointer.invalidate_geometry();
        self.fixed_draws = Default::default();
        self.repaints.borrow_mut().commands.clear();
    }

    fn apply_visibility(
        &mut self,
        updates: VisibilityBatch,
        state: &mut Runtime<()>,
    ) -> Result<()> {
        // Resolve all local declarations before propagating from parents to children.
        // A branch may have retired or replaced the mount that queued an update.
        for &(id, visible) in updates.values() {
            if let Some(body) = self.bodies.get_mut(&id) {
                body.local_visible = visible;
            }
        }
        let mut pending = Vec::new();
        let mut changed = false;
        for (path, (id, _)) in updates {
            if !self.bodies.contains_key(&id) {
                continue;
            }
            let parent_visible = path
                .split_last()
                .is_none_or(|(_, path)| self.bodies[&self.nodes[path]].visible);
            pending.push((id, parent_visible));
            while let Some((id, parent_visible)) = pending.pop() {
                let body = self.bodies.get_mut(&id).expect("mounted visibility target");
                let visible = body.local_visible && parent_visible;
                if body.visible == visible {
                    continue;
                }
                body.visible = visible;
                if visible {
                    self.visible_nodes += 1;
                } else {
                    self.visible_nodes -= 1;
                    if self.focused == Some(id) {
                        self.set_focus(None, false);
                    }
                    self.unobserve_paint(id, state);
                }
                changed = true;
                self.repaints.borrow_mut().invalidate(id);
                pending.extend(self.scene.children(id)?.map(|child| (child, visible)));
            }
        }
        if changed {
            self.tab_order = None;
            for list in self.lists.values() {
                list.dirty.set(true);
            }
            self.pointer.invalidate_geometry();
            self.fixed_draws = Default::default();
        }
        Ok(())
    }

    fn update_label(&mut self, node: NodeId, value: &SharedString) -> Result<()> {
        if self.scene.body::<StyledBody<Text>>(node)?.content.content() != value.as_str() {
            Text::validate_content(value)?;
            self.scene.edit_layout::<StyledBody<Text>>(node, |body| {
                body.content.replace_content(value.clone())
            })?;
            if self.bodies[&node].visible {
                self.repaints.borrow_mut().invalidate(node);
            }
        }
        Ok(())
    }

    fn dispose(self, state: &mut Runtime<()>) -> Result<()> {
        for tracked in self.tracked_scroll.values() {
            tracked.detach(state);
        }
        for handle in self.lists.values().filter_map(|list| list.scroll.as_ref()) {
            handle.detach(state);
        }
        for node in self.bodies.into_values() {
            for signal in node.owned.signals {
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
        scroll_handle: Option<ScrollHandle>,
        cx: &mut Attachment<'_>,
    ) -> Result<()> {
        if let Some(handle) = scroll_handle {
            self.tracked_scroll
                .insert(parent, pointer::TrackedScroll::attach(handle)?);
        }
        if !watches.is_empty() {
            let target = cx.state.append(cx.scope, ())?;
            self.bodies
                .get_mut(&parent)
                .expect("mounted node")
                .binding_target = Some(target);
            let repaints = self.bodies[&parent].visible.then_some(&self.repaints);
            for watch in watches {
                watch(cx, target, parent, path, repaints)?;
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
                Child::List(update) => {
                    self.lists.insert(path.to_vec(), update);
                }
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
            owned: Owned::default(),
        };
        let Installation {
            mut node,
            scroll_handle,
            on_frame,
            children,
            watches,
        } = element.attach(
            &mut self.scene,
            parent,
            index,
            self.bodies[&parent].text_style.clone(),
            (self.units, cx.assets),
            &mut mount.context(cx.engine, cx.tasks),
        )?;
        node.owned = std::mem::take(&mut mount.owned);
        drop(mount);
        node.offset = *path.last().expect("child position");
        let id = node.id;
        node.visible &= self.bodies[&parent].visible;
        self.visible_nodes += usize::from(node.visible);
        self.bodies.insert(id, node);
        if let Some(handler) = on_frame {
            self.frame_handlers.insert(id, handler);
        }
        self.nodes.insert(path.to_vec(), id);
        self.tab_order = None;
        self.invalidate_geometry();
        self.finish_attachment(id, path, watches, children, scroll_handle, cx)?;
        Ok(())
    }

    fn remove(&mut self, path: &Path, cx: &mut Attachment<'_>) -> Result<()> {
        let Some(node) = self.nodes.get(path) else {
            return Ok(());
        };
        self.scene.unmount_subtree(*node)?;
        self.tab_order = None;
        self.invalidate_geometry();
        for key in subtree_keys(&self.lists, path) {
            if let Some(handle) = self.lists.remove(&key).and_then(|list| list.scroll) {
                handle.detach(cx.state);
            }
        }
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
            self.visible_nodes -= usize::from(node.visible);
            if let Some(tracked) = self.tracked_scroll.remove(&node.id) {
                tracked.detach(cx.state);
            }
            self.pointer.retire(node.id);
            if self.focused == Some(node.id) {
                self.focused = None;
            }
            self.frame_handlers.remove(&node.id);
            self.unobserve_paint(node.id, cx.state);
            self.repaints.borrow_mut().dirty.remove(&node.id);
            if let Some(target) = node.binding_target {
                cx.state.remove_subtree(target)?;
                cx.styles.borrow_mut().remove(&(ViewId(cx.scope), node.id));
                cx.content_updates
                    .borrow_mut()
                    .remove(&(ViewId(cx.scope), node.id));
            }
            for signal in node.owned.signals {
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
#[derive(Default)]
pub struct Ui {
    tasks: Tasks,
    assets: assets::Resources,
    keymap: HashMap<keyboard::Shortcut, Vec<keyboard::BoundAction>>,
    tooltip_style: TooltipStyle,
    state: tasks::UiState,
    engine: TextSystem,
    instances: HashMap<ViewId, Instance>,
    pending: Pending,
    styles: StyleUpdates,
    content_updates: ContentUpdates,
    visibility: VisibilityUpdates,
}

impl Ui {
    /// Creates a UI that discovers system fonts on first use of text.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a UI using an application's registered fonts and language preferences.
    pub fn with_fonts(fonts: Fonts) -> Self {
        Self {
            engine: TextSystem::with_fonts(fonts),
            ..Self::default()
        }
    }

    /// Creates a UI using this font as its primary family in an isolated font collection.
    pub fn with_font(bytes: Vec<u8>) -> Result<Self> {
        let mut fonts = Fonts::empty();
        let name = fonts.add_font(bytes)?;
        fonts.set_default_family(&name)?;
        Ok(Self::with_fonts(fonts))
    }

    /// Supplies the native host's interface font for first use of system text.
    /// Already shaped views retain their font settings.
    pub fn set_system_font_provider(&mut self, provider: fn() -> Option<String>) {
        self.engine.platform_family = Some(provider);
    }

    /// Renders once and attaches the declaration. Flush to initialize conditional children.
    pub fn mount(&mut self, item: impl Render) -> Result<ViewId> {
        self.mount_with_rem_size(item, px(16.0))
    }

    /// Attaches a view with its own positive, finite logical-pixel basis for rem lengths.
    pub fn mount_with_rem_size(&mut self, item: impl Render, rem_size: Pixels) -> Result<ViewId> {
        let units = UnitResolver::new(rem_size)?;
        let mut mount = Mount {
            state: self.state.get_mut(),
            owned: Owned::default(),
        };
        let mut cx = mount.context(&self.engine, &self.tasks);
        let item = item.render(&mut cx).into_element();
        let (
            scene,
            Installation {
                node: mut root_node,
                scroll_handle,
                on_frame,
                children,
                watches,
            },
        ) = item.root(&mut cx, (units, &self.assets))?;
        let scope = mount.state.append(mount.state.tree().root(), ())?;
        root_node.owned = std::mem::take(&mut mount.owned);
        drop(mount);
        let root = scene.root();
        let mut instance = Instance {
            focused: None,
            tab_order: None,
            frame_handlers: on_frame
                .into_iter()
                .map(|handler| (root, handler))
                .collect(),
            scope,
            paint_reads: Default::default(),
            observers: Default::default(),
            pointer: Default::default(),
            fixed_draws: Default::default(),
            units,
            repaints: Repaints::default(),
            scene,
            visible_nodes: usize::from(root_node.visible),
            bodies: HashMap::from([(root, root_node)]),
            nodes: BTreeMap::from([(Vec::new(), root)]),
            branches: BTreeMap::new(),
            lists: BTreeMap::new(),
            tracked_scroll: HashMap::new(),
        };
        let mut cx = Attachment {
            tasks: &self.tasks,
            assets: &self.assets,
            scope,
            state: self.state.get_mut(),
            pending: &self.pending,
            styles: &self.styles,
            content_updates: &self.content_updates,
            visibility: &self.visibility,
            engine: &self.engine,
        };
        if let Err(error) =
            instance.finish_attachment(root, &[], watches, children, scroll_handle, &mut cx)
        {
            cx.state.remove_subtree(scope)?;
            instance.dispose(cx.state)?;
            return Err(error);
        }
        let view = ViewId(scope);
        self.instances.insert(view, instance);
        Ok(view)
    }

    /// Opens an application-owned state context.
    pub fn context(&mut self) -> Context<'_> {
        Context {
            tasks: &self.tasks,
            state: self.state.get_mut(),
            engine: &self.engine,
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
            tasks: &self.tasks,
            state: self.state.get_mut(),
            engine: &self.engine,
            owned: Some(&mut node.owned),
        })?;
        Ok(true)
    }

    /// Whether bindings are queued for the next flush, including work retained for retry.
    pub fn has_pending_bindings(&self) -> bool {
        self.state.borrow().pending_count() != 0
            || !self.pending.borrow().is_empty()
            || !self.styles.borrow().is_empty()
            || !self.content_updates.borrow().is_empty()
            || !self.visibility.borrow().is_empty()
    }

    /// Applies structural, visibility, label and paint bindings. Returns the number of branch positions visited.
    pub fn flush(&mut self) -> Result<usize> {
        let mut visited = 0;
        loop {
            self.state.get_mut().flush();
            let batch = std::mem::take(&mut *self.pending.borrow_mut());
            if batch.is_empty() {
                let mut failure = None;
                self.styles
                    .borrow_mut()
                    .retain(|&(view, id), (apply, layout)| {
                        if let Some(instance) = self.instances.get_mut(&view)
                            && let Err(error) = apply(instance, id, self.state.get_mut(), *layout)
                        {
                            failure.get_or_insert(error);
                            return true;
                        }
                        false
                    });
                for (view, updates) in self.visibility.borrow_mut().drain() {
                    if let Some(instance) = self.instances.get_mut(&view) {
                        instance.apply_visibility(updates, self.state.get_mut())?;
                    }
                }
                if let Some(error) = failure {
                    return Err(error);
                }
                self.flush_content_updates()?;
                if self.state.get_mut().pending_count() == 0 {
                    return Ok(visited);
                }
                continue;
            }
            let mut failure = None;
            for (scope, updates) in batch {
                let instance = self
                    .instances
                    .get_mut(&ViewId(scope))
                    .expect("queued scope is live");
                let mut cx = Attachment {
                    tasks: &self.tasks,
                    assets: &self.assets,
                    scope,
                    state: self.state.get_mut(),
                    pending: &self.pending,
                    styles: &self.styles,
                    content_updates: &self.content_updates,
                    visibility: &self.visibility,
                    engine: &self.engine,
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

    fn flush_content_updates(&mut self) -> Result<()> {
        let mut content_updates = self.content_updates.borrow_mut();
        if content_updates.is_empty() {
            return Ok(());
        }
        let mut failure = None;
        content_updates.retain(|&(view, id), source| {
            let instance = self.instances.get_mut(&view).expect("live content view");
            let state = self.state.get_mut();
            let result = match *source {
                ContentUpdate::Text(source) => state
                    .read(source)
                    .ok()
                    .map(|value| instance.update_label(id, value)),
                ContentUpdate::CustomImage(source) => state
                    .read(source)
                    .ok()
                    .map(|value| instance.update_custom_image(id, value)),
                ContentUpdate::Svg(source, preserve_colors) => state
                    .read(source)
                    .ok()
                    .map(|value| instance.update_svg(id, value, preserve_colors)),
                #[cfg(feature = "network")]
                ContentUpdate::RemoteImage(source) => state
                    .read(source)
                    .ok()
                    .and_then(Option::as_ref)
                    .map(|value| instance.update_image(id, value)),
                ContentUpdate::Image(source) => state
                    .read(source)
                    .ok()
                    .map(|value| instance.update_image(id, value)),
            };
            if let Some(Err(error)) = result {
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
        nodes.extend(ready.dirty.iter().copied());
        Ok(())
    }

    /// Returns the retained root of a mounted view.
    pub fn root(&self, view: ViewId) -> Result<NodeId> {
        Ok(self.instance(view)?.scene.root())
    }

    /// Returns the element holding logical focus in this view.
    pub fn focused(&self, view: ViewId) -> Result<Option<NodeId>> {
        Ok(self.instance(view)?.focused)
    }

    /// Gives focus to a visible, focusable element. An invalid request preserves existing focus.
    pub fn focus(&mut self, view: ViewId, target: NodeId) -> Result<()> {
        let instance = self.instance_mut(view)?;
        let body = instance
            .bodies
            .get(&target)
            .ok_or(tree::TreeError::InvalidNode(target))?;
        anyhow::ensure!(
            body.visible && body.focusable,
            "focus requires a visible, focusable element"
        );
        instance.set_focus(Some(target), true);
        Ok(())
    }

    /// Clears logical focus in this view.
    pub fn blur(&mut self, view: ViewId) -> Result<()> {
        self.instance_mut(view)?.set_focus(None, false);
        Ok(())
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

    fn instance_mut(&mut self, view: ViewId) -> Result<&mut Instance> {
        self.instances
            .get_mut(&view)
            .ok_or_else(|| tree::TreeError::InvalidNode(view.0).into())
    }

    /// Applies the viewport allocation, materializes visible list rows and returns the total
    /// number of geometry changes across the required layout passes.
    /// Call flush first to apply queued structural changes.
    pub fn layout(&mut self, view: ViewId, size: layout::Size) -> Result<usize> {
        let mut total = 0;
        let instance = loop {
            let instance = self
                .instances
                .get_mut(&view)
                .ok_or(crate::tree::TreeError::InvalidNode(view.0))?;
            let (changed, layout_changed) = instance.layout_pass(size)?;
            total += changed;
            if instance.lists.is_empty() {
                break instance;
            }
            let mut cx = Attachment {
                tasks: &self.tasks,
                assets: &self.assets,
                scope: view.0,
                state: self.state.get_mut(),
                pending: &self.pending,
                styles: &self.styles,
                content_updates: &self.content_updates,
                visibility: &self.visibility,
                engine: &self.engine,
            };
            instance.update_lists(&mut cx, layout_changed)?;
            if !instance.scene.is_pending() {
                for list in instance.lists.values() {
                    if let Some(handle) = &list.scroll {
                        handle.publish(cx.state);
                    }
                }
                break instance;
            }
            // Newly mounted rows may declare bindings or contain another list.
            self.flush()?;
        };
        if !instance.tracked_scroll.is_empty() {
            instance.update_tracked_scroll(self.state.get_mut())?;
        }
        instance.notify_list_ranges(self.state.get_mut(), &self.engine, &self.tasks)?;
        Ok(total)
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
        let mut state = self.state.try_borrow_mut()?;
        let start = commands.len();
        body.paint(
            &instance.scene,
            &state,
            &instance.paint_reads,
            None,
            commands,
        )?;
        frame::wrap_group(commands, start, body);
        instance.observe_paint(node, &mut state)?;
        let mut repaints = instance.repaints.borrow_mut();
        repaints.commands.remove(&node);
        repaints.dirty.remove(&node);
        Ok(())
    }

    /// Retires a mounted tree, its local signals and subscriptions.
    pub fn unmount(&mut self, scope: ViewId) -> Result<()> {
        if !self.instances.contains_key(&scope) {
            return Err(crate::tree::TreeError::InvalidNode(scope.0).into());
        }
        self.state.get_mut().remove_subtree(scope.0)?;
        self.pending.borrow_mut().remove(&scope.0);
        self.visibility.borrow_mut().remove(&scope);
        self.styles
            .borrow_mut()
            .retain(|(view, _), _| *view != scope);
        self.content_updates
            .borrow_mut()
            .retain(|(view, _), _| *view != scope);
        let instance = self.instances.remove(&scope).expect("live mounted scope");
        instance.dispose(self.state.get_mut())
    }
}
