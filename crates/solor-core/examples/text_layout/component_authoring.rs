//! Private authoring experiment for ordered conditional subtrees.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use smol_str::SmolStr;
use solor_core::layout::Scene;
use solor_core::state::{Runtime, Signal, SignalId};
use solor_core::tree::NodeId;

use super::column::Column;
use super::frame::STYLE;
use super::text::{Engine, SharedEngine, Text};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Mount<'a> {
    state: &'a mut Runtime<()>,
    owned: Vec<SignalId>,
}

impl Mount<'_> {
    fn signal<T: 'static>(&mut self, initial: T) -> Signal<T> {
        let signal = self.state.create_signal(initial);
        self.owned.push(signal.id());
        signal
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

struct Event<'a>(&'a mut Runtime<()>);

impl Event<'_> {
    fn update<T: Clone + PartialEq + 'static>(
        &mut self,
        signal: Signal<T>,
        change: impl FnOnce(&mut T),
    ) -> Result<()> {
        let mut value = self.0.read(signal)?.clone();
        change(&mut value);
        self.0.set(signal, value)?;
        Ok(())
    }
}

struct TextLeaf(SmolStr);

fn text(content: impl Into<SmolStr>) -> TextLeaf {
    TextLeaf(content.into())
}

struct Div {
    padding: u32,
}

fn div() -> Div {
    Div { padding: 0 }
}

impl Div {
    fn padding(self, padding: u32) -> Self {
        Self { padding }
    }

    fn column(self) -> Container {
        Container {
            layout: Column {
                padding: self.padding,
                gap: 0,
            },
            children: Vec::new(),
            click: None,
        }
    }

    fn child(self, child: impl Into<Element>) -> Container {
        self.column().child(child)
    }
}

type Click = Box<dyn Fn(&mut Event<'_>) -> Result<()>>;

enum Element {
    Text(TextLeaf),
    Container(Container),
}

impl From<TextLeaf> for Element {
    fn from(leaf: TextLeaf) -> Self {
        Self::Text(leaf)
    }
}

impl From<Container> for Element {
    fn from(container: Container) -> Self {
        Self::Container(container)
    }
}

struct Container {
    layout: Column,
    children: Vec<Child>,
    click: Option<Click>,
}

impl Container {
    fn gap(mut self, gap: u32) -> Self {
        self.layout.gap = gap;
        self
    }

    fn child(mut self, child: impl Into<Element>) -> Self {
        self.children.push(Child::Fixed(child.into()));
        self
    }

    fn on_click(mut self, click: impl Fn(&mut Event<'_>) -> Result<()> + 'static) -> Self {
        self.click = Some(Box::new(click));
        self
    }

    fn child_when<E: Into<Element>>(
        mut self,
        open: Signal<bool>,
        make: impl Fn() -> E + 'static,
    ) -> Self {
        self.children
            .push(Child::When(open, Box::new(move || make().into())));
        self
    }
}

type Factory = Box<dyn Fn() -> Element>;

enum Child {
    Fixed(Element),
    When(Signal<bool>, Factory),
}

struct Component<F>(F);

fn component<E: Into<Element>, F: FnOnce(&mut Mount<'_>) -> E>(factory: F) -> Component<F> {
    Component(factory)
}

// Indices in the declaration stay fixed while live sibling indices change.
type Path = Vec<usize>;
type Updates = BTreeMap<Path, bool>;
type Pending = Rc<RefCell<HashMap<NodeId, Updates>>>;

struct Branch {
    target: NodeId,
    make: Factory,
}

struct Instance {
    scene: Scene,
    handlers: HashMap<NodeId, Click>,
    nodes: BTreeMap<Path, NodeId>,
    branches: BTreeMap<Path, Branch>,
    owned: Vec<SignalId>,
}

struct Attachment<'a> {
    scope: NodeId,
    state: &'a mut Runtime<()>,
    pending: &'a Pending,
    engine: &'a SharedEngine,
}

impl Instance {
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
        element: Element,
        cx: &mut Attachment<'_>,
    ) -> Result<()> {
        match element {
            Element::Text(leaf) => {
                let text = Text::new(cx.engine.clone(), leaf.0, STYLE, true)?;
                let id = self.scene.mount_at(parent, index, text)?;
                self.nodes.insert(path.to_vec(), id);
            }
            Element::Container(container) => {
                let id = self.scene.mount_at(parent, index, container.layout)?;
                self.nodes.insert(path.to_vec(), id);
                if let Some(click) = container.click {
                    self.handlers.insert(id, click);
                }
                self.register_children(id, path, container.children, cx)?;
            }
        }
        Ok(())
    }

    fn remove(&mut self, path: &Path, cx: &mut Attachment<'_>) -> Result<()> {
        let Some(&node) = self.nodes.get(path) else {
            return Ok(());
        };
        self.scene.unmount_subtree(node)?;
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
            self.handlers.remove(&node);
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
        let mut sibling = path.clone();
        let mut index = 0;
        for previous in 0..offset {
            sibling[parent_path.len()] = previous;
            if self.nodes.contains_key(&sibling) {
                index += 1;
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

struct Host {
    state: Runtime<()>,
    engine: SharedEngine,
    instances: HashMap<NodeId, Instance>,
    pending: Pending,
}

impl Host {
    fn new() -> Self {
        Self {
            state: Runtime::new(()),
            engine: Engine::new(),
            instances: HashMap::new(),
            pending: Pending::default(),
        }
    }

    fn mount<E: Into<Element>>(
        &mut self,
        component: Component<impl FnOnce(&mut Mount<'_>) -> E>,
    ) -> Result<NodeId> {
        let mut mount = Mount {
            state: &mut self.state,
            owned: Vec::new(),
        };
        let (scene, click, children) = match (component.0)(&mut mount).into() {
            Element::Container(container) => (
                Scene::new(container.layout),
                container.click,
                container.children,
            ),
            Element::Text(leaf) => (
                Scene::new(Text::new(self.engine.clone(), leaf.0, STYLE, true)?),
                None,
                Vec::new(),
            ),
        };
        let root = scene.root();
        let mut instance = Instance {
            scene,
            handlers: HashMap::new(),
            nodes: BTreeMap::from([(Vec::new(), root)]),
            branches: BTreeMap::new(),
            owned: Vec::new(),
        };
        if let Some(click) = click {
            instance.handlers.insert(root, click);
        }
        let scope = mount.state.append(mount.state.tree().root(), ())?;
        let mut cx = Attachment {
            scope,
            state: mount.state,
            pending: &self.pending,
            engine: &self.engine,
        };
        if let Err(error) = instance.register_children(root, &[], children, &mut cx) {
            mount.state.remove_subtree(scope)?;
            return Err(error);
        }
        instance.owned = std::mem::take(&mut mount.owned);
        self.instances.insert(scope, instance);
        Ok(scope)
    }

    fn click(&mut self, scope: NodeId, target: NodeId) -> Result<bool> {
        let Some(click) = self
            .instances
            .get(&scope)
            .and_then(|i| i.handlers.get(&target))
        else {
            return Ok(false);
        };
        click(&mut Event(&mut self.state))?;
        Ok(true)
    }

    fn flush(&mut self) -> Result<usize> {
        let mut visited = 0;
        loop {
            self.state.flush();
            let batch = std::mem::take(&mut *self.pending.borrow_mut());
            if batch.is_empty() {
                return Ok(visited);
            }
            let mut failure = None;
            for (scope, updates) in batch {
                let instance = self
                    .instances
                    .get_mut(&scope)
                    .expect("queued scope is live");
                let mut cx = Attachment {
                    scope,
                    state: &mut self.state,
                    pending: &self.pending,
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

    fn unmount(&mut self, scope: NodeId) -> Result<()> {
        if !self.instances.contains_key(&scope) {
            return Err(solor_core::tree::TreeError::InvalidNode(scope).into());
        }
        self.state.remove_subtree(scope)?;
        self.pending.borrow_mut().remove(&scope);
        let instance = self.instances.remove(&scope).expect("live mounted scope");
        for signal in instance.owned {
            self.state.remove_signal_by_id(signal)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solor_core::layout::Size;
    use std::{cell::Cell, rc::Rc};

    impl Instance {
        fn node_at(&self, path: &[usize]) -> Option<NodeId> {
            self.nodes.get(path).copied()
        }
    }

    fn section(open: Signal<bool>, detail: impl Fn() -> TextLeaf + 'static) -> Container {
        div()
            .column()
            .gap(8)
            .child(
                div()
                    .padding(6)
                    .child(text("Audio settings"))
                    .on_click(move |event| event.update(open, |v| *v = !*v)),
            )
            .child_when(open, detail)
    }

    #[test]
    fn clicks_write_immediately_and_only_final_visibility_builds_a_leaf() -> Result<()> {
        let mut host = Host::new();
        let open_handle = Cell::new(None);
        let mounts = Cell::new(0);
        let builds = Rc::new(Cell::new(0));
        let scope = host.mount(component(|mount| {
            mounts.set(mounts.get() + 1);
            let open = mount.signal(false);
            open_handle.set(Some(open));
            let builds = builds.clone();
            section(open, move || {
                builds.set(builds.get() + 1);
                text("Output follows the system device.")
            })
        }))?;
        let header = *host.instances[&scope].handlers.keys().next().unwrap();
        let open = open_handle.get().unwrap();
        host.flush()?;
        assert!(!*host.state.read(open)?);
        assert!(host.click(scope, header)?);
        assert!(*host.state.read(open)?);
        assert!(host.instances[&scope].node_at(&[1]).is_none());
        host.click(scope, header)?;
        host.flush()?;
        assert_eq!(builds.get(), 0);

        host.click(scope, header)?;
        host.flush()?;
        let first = host.instances[&scope].node_at(&[1]).unwrap();
        let scene = &mut host.instances.get_mut(&scope).unwrap().scene;
        for width in [320, 160, 320] {
            scene.run_assigned(Size { width, height: 120 }, false)?;
            let header_box = scene.geometry(header).unwrap();
            assert_eq!(
                scene.geometry(first).unwrap().origin.y,
                header_box.metrics.size.height + 8
            );
            assert_eq!(scene.body::<Column>(header)?.padding, 6);
        }
        host.flush()?;
        assert_eq!(host.instances[&scope].node_at(&[1]), Some(first));
        assert_eq!(builds.get(), 1);
        assert!(*host.state.read(open)?);

        host.click(scope, header)?;
        host.flush()?;
        assert!(host.instances[&scope].scene.body::<Text>(first).is_err());
        host.click(scope, header)?;
        host.flush()?;
        assert_ne!(host.instances[&scope].node_at(&[1]).unwrap(), first);
        assert_eq!((mounts.get(), builds.get()), (1, 2));
        Ok(())
    }

    #[test]
    fn instance_owns_initial_value_and_unmount_cancels_pending_work() -> Result<()> {
        let open_handle = Cell::new(None);
        let make = || {
            component(|mount| {
                let open = mount.signal(false);
                open_handle.set(Some(open));
                section(open, || text("Detail"))
            })
        };
        let mut host = Host::new();
        let scope = host.mount(make())?;
        let open = open_handle.get().unwrap();
        let sibling = host.mount(make())?;
        let header = *host.instances[&scope].handlers.keys().next().unwrap();
        host.click(scope, header)?;
        host.flush()?;
        assert!(host.instances[&scope].node_at(&[1]).is_some());
        assert!(host.instances[&sibling].node_at(&[1]).is_none());
        host.click(scope, header)?;
        host.unmount(scope)?;
        assert!(host.state.read(open).is_err());
        assert_eq!(host.state.pending_count(), 0);
        assert_eq!(host.state.binding_count(), 1);
        let new_scope = host.mount(make())?;
        assert!(!host.click(new_scope, header)?);
        assert!(!*host.state.read(open_handle.get().unwrap())?);
        host.unmount(new_scope)?;
        host.unmount(sibling)?;
        assert_eq!(
            (host.state.signal_count(), host.state.binding_count()),
            (0, 0)
        );
        Ok(())
    }

    #[test]
    fn middle_subtree_keeps_volume_identity_and_dispatches_only_the_changed_section() -> Result<()>
    {
        let mut host = Host::new();
        let volume_handle = Cell::new(None);
        let make = || {
            component(|mount| {
                let open = mount.signal(false);
                let volume = mount.signal(40_u8);
                volume_handle.set(Some(volume));
                div()
                    .column()
                    .gap(8)
                    .child(
                        div()
                            .child(text("Audio settings"))
                            .on_click(move |event| event.update(open, |v| *v = !*v)),
                    )
                    .child_when(open, move || {
                        div()
                            .column()
                            .child(text("Output device"))
                            .child(text("System default"))
                            .on_click(move |event| event.update(open, |v| *v = false))
                    })
                    .child(
                        div()
                            .child(text("Volume"))
                            .on_click(move |event| event.update(volume, |v| *v += 1)),
                    )
            })
        };
        let scope = host.mount(make())?;
        let volume = volume_handle.get().unwrap();
        let other = host.mount(make())?;
        assert_eq!(host.flush()?, 2);
        let header = host.instances[&scope].nodes[&[0][..]];
        let row = host.instances[&scope].nodes[&[2][..]];
        let layout = |host: &mut Host| -> Result<_> {
            let i = host.instances.get_mut(&scope).unwrap();
            i.scene.run_assigned(
                Size {
                    width: 320,
                    height: 200,
                },
                false,
            )?;
            assert_eq!(i.nodes[&[2][..]], row);
            Ok((
                i.scene.geometry(header).unwrap(),
                i.scene.geometry(row).unwrap(),
            ))
        };
        let (top, closed) = layout(&mut host)?;
        assert_eq!(closed.origin.y, top.metrics.size.height + 8);
        host.click(scope, row)?;
        assert_eq!(host.flush()?, 0);
        host.click(scope, header)?;
        assert_eq!(host.flush()?, 1);
        let (_, opened) = layout(&mut host)?;
        let i = &host.instances[&scope];
        let detail = i.nodes[&[1][..]];
        let bounds = i.scene.geometry(detail).unwrap();
        assert_eq!(bounds.origin.y, closed.origin.y);
        assert_eq!(
            opened.origin.y,
            bounds.origin.y + bounds.metrics.size.height + 8
        );
        assert!(i.scene.geometry(i.nodes[&[1, 1][..]]).unwrap().origin.y > 0);
        assert!(!host.instances[&other].nodes.contains_key(&[1][..]));

        assert!(host.click(scope, row)?);
        assert!(host.click(scope, detail)?);
        assert_eq!(host.flush()?, 1);
        assert_eq!(layout(&mut host)?.1, closed);
        assert!(!host.click(scope, detail)?);
        assert!(host.click(scope, row)?);
        assert_eq!(*host.state.read(volume)?, 43);
        assert_eq!(host.flush()?, 0);
        Ok(())
    }

    #[test]
    fn nested_initial_bindings_and_simultaneous_parent_removal() -> Result<()> {
        let mut host = Host::new();
        let outer = host.state.create_signal(true);
        let inner = host.state.create_signal(true);
        let builds = Rc::new(Cell::new(0));
        let scope = host.mount(component(|_| {
            div()
                .column()
                .child_when(outer, {
                    let builds = builds.clone();
                    move || {
                        let builds = builds.clone();
                        div().column().child_when(inner, move || {
                            builds.set(builds.get() + 1);
                            text("Nested")
                        })
                    }
                })
                .child_when(inner, || text("Sibling"))
        }))?;
        assert_eq!(host.flush()?, 3);
        let sibling = host.instances[&scope].nodes[&[1][..]];
        assert_eq!(builds.get(), 1);
        host.state.set(inner, false)?;
        host.flush()?;
        host.state.set(inner, true)?;
        host.state.set(outer, false)?;
        host.flush()?;
        assert_eq!(builds.get(), 1);
        assert_eq!(host.state.binding_count(), 2);
        assert_ne!(host.instances[&scope].nodes[&[1][..]], sibling);
        host.state.set(outer, true)?;
        assert_eq!(host.flush()?, 2);
        assert_eq!(builds.get(), 2);
        host.unmount(scope)?;
        assert_eq!(host.state.binding_count(), 0);
        let label = host.mount(component(|_| text("Standalone")))?;
        assert_eq!(host.flush()?, 0);
        let i = &host.instances[&label];
        assert_eq!(
            i.scene.body::<Text>(i.scene.root())?.content(),
            "Standalone"
        );
        Ok(())
    }

    #[test]
    fn failed_subtree_attachment_releases_partial_nodes_handlers_and_bindings_then_retries()
    -> Result<()> {
        let mut host = Host::new();
        let open = host.state.create_signal(true);
        let invalid = Rc::new(Cell::new(true));
        let scope = host.mount(component(|_| {
            div()
                .column()
                .child_when(open, {
                    let invalid = invalid.clone();
                    move || {
                        div()
                            .column()
                            .on_click(|_| Ok(()))
                            .child_when(open, || text("Nested"))
                            .child(text(if invalid.get() {
                                "X".repeat(super::super::text::MAX_TEXT_BYTES + 1)
                            } else {
                                "Valid".into()
                            }))
                    }
                })
                .child(text("Tail"))
        }))?;
        let tail = host.instances[&scope].nodes[&[1][..]];
        assert!(host.flush().is_err());
        let i = &host.instances[&scope];
        assert_eq!(i.nodes.len(), 2);
        assert!(i.handlers.is_empty());
        assert_eq!(host.state.binding_count(), 1);
        assert_eq!(host.state.pending_count(), 0);
        invalid.set(false);
        assert_eq!(host.flush()?, 2);
        assert_eq!(host.instances[&scope].nodes[&[1][..]], tail);
        assert!(host.instances[&scope].nodes.contains_key(&[0, 0][..]));
        Ok(())
    }

    #[test]
    fn mixed_state_uses_independent_typed_handles_and_one_scope_cleanup() -> Result<()> {
        let mut host = Host::new();
        let handles = Cell::new(None);
        let scope = host.mount(component(|mount| {
            let open = mount.signal(false);
            let muted = mount.signal(false);
            let volume = mount.signal(40_u8);
            let device = mount.signal(String::from("System"));
            handles.set(Some((open, muted, volume, device)));
            div()
                .child(text("Select headphones"))
                .on_click(move |event| {
                    event.update(muted, |v| *v = true)?;
                    event.update(volume, |v| *v += 5)?;
                    event.update(device, |v| v.push_str(" headphones"))
                })
                .child_when(open, || text("Audio settings"))
        }))?;
        let (open, muted, volume, device) = handles.get().unwrap();
        host.flush()?;
        assert!(host.click(scope, host.instances[&scope].scene.root())?);
        assert_eq!(host.state.pending_count(), 0);
        assert!(!*host.state.read(open)?);
        assert!(*host.state.read(muted)?);
        assert_eq!(*host.state.read(volume)?, 45);
        assert_eq!(host.state.read(device)?, "System headphones");

        host.unmount(scope)?;
        assert!(host.state.read(open).is_err());
        assert!(host.state.read(muted).is_err());
        assert!(host.state.read(volume).is_err());
        assert!(host.state.read(device).is_err());
        Ok(())
    }

    #[test]
    fn external_playback_logic_updates_both_views_and_outlives_them() -> Result<()> {
        struct Playback {
            playing: Signal<bool>,
        }
        impl Playback {
            fn toggle<N>(&self, state: &mut Runtime<N>) -> Result<()> {
                let next = !*state.read(self.playing)?;
                state.set(self.playing, next)?;
                Ok(())
            }
        }

        let mut host = Host::new();
        let playback = Playback {
            playing: host.state.create_signal(false),
        };
        let view = || {
            component(|_| {
                div()
                    .column()
                    .child_when(playback.playing, || text("Playing"))
            })
        };
        let player = host.mount(view())?;
        let mini_player = host.mount(view())?;
        host.flush()?;

        playback.toggle(&mut host.state)?;
        assert!(*host.state.read(playback.playing)?);
        assert!(host.instances[&player].node_at(&[0]).is_none());
        assert_eq!(host.flush()?, 2);
        assert!(host.instances[&player].node_at(&[0]).is_some());
        assert!(host.instances[&mini_player].node_at(&[0]).is_some());

        host.unmount(player)?;
        playback.toggle(&mut host.state)?;
        assert_eq!(host.flush()?, 1);
        assert!(host.instances[&mini_player].node_at(&[0]).is_none());
        host.unmount(mini_player)?;
        playback.toggle(&mut host.state)?;
        assert!(*host.state.read(playback.playing)?);
        assert_eq!(host.flush()?, 0);
        host.state.remove_signal(playback.playing)?;
        Ok(())
    }

    #[test]
    fn borrowed_state_survives_remount_and_stale_input_rolls_back_mount() -> Result<()> {
        let mut host = Host::new();
        let open = host.state.create_signal(true);
        let make = || component(|_| section(open, || text("Detail")));
        let scope = host.mount(make())?;
        host.flush()?;
        assert!(host.instances[&scope].node_at(&[1]).is_some());
        host.click(
            scope,
            *host.instances[&scope].handlers.keys().next().unwrap(),
        )?;
        host.unmount(scope)?;
        let scope = host.mount(make())?;
        host.flush()?;
        assert!(!*host.state.read(open)?);
        assert!(host.instances[&scope].node_at(&[1]).is_none());
        host.unmount(scope)?;
        host.state.remove_signal(open)?;
        assert!(
            host.mount(component(|mount| {
                mount.signal(false);
                mount.signal(String::from("System"));
                section(open, || text("Detail"))
            }))
            .is_err()
        );
        assert_eq!(
            (host.state.signal_count(), host.state.binding_count()),
            (0, 0)
        );
        Ok(())
    }
}
