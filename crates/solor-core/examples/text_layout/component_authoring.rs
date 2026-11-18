//! Private authoring experiment for a column with one optional trailing text leaf.

use std::collections::HashMap;

use smol_str::SmolStr;
use solor_core::layout::Scene;
use solor_core::state::{Runtime, Signal, SignalId};
use solor_core::tree::NodeId;

use super::column::Column;
use super::frame::STYLE;
use super::text::{Engine, SharedEngine, Text};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Mount<'a> {
    state: &'a mut Runtime<bool>,
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

struct Event<'a>(&'a mut Runtime<bool>);

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

impl TextLeaf {
    fn mount(self, scene: &mut Scene, parent: NodeId, engine: &SharedEngine) -> Result<NodeId> {
        Ok(scene.mount(parent, Text::new(engine.clone(), self.0, STYLE, true)?)?)
    }
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
    children: Vec<Element>,
    click: Option<Click>,
}

impl Container {
    fn gap(mut self, gap: u32) -> Self {
        self.layout.gap = gap;
        self
    }

    fn child(mut self, child: impl Into<Element>) -> Self {
        self.children.push(child.into());
        self
    }

    fn on_click(mut self, click: impl Fn(&mut Event<'_>) -> Result<()> + 'static) -> Self {
        self.click = Some(Box::new(click));
        self
    }

    fn child_when(self, open: Signal<bool>, detail: impl Fn() -> TextLeaf + 'static) -> Branch {
        Branch {
            container: self,
            open,
            detail: Box::new(detail),
        }
    }
}

struct Branch {
    container: Container,
    open: Signal<bool>,
    detail: Box<dyn Fn() -> TextLeaf>,
}

fn mount_contents(
    scene: &mut Scene,
    parent: NodeId,
    children: Vec<Element>,
    click: Option<Click>,
    engine: &SharedEngine,
    handlers: &mut HashMap<NodeId, Click>,
) -> Result<()> {
    if let Some(click) = click {
        handlers.insert(parent, click);
    }
    for child in children {
        match child {
            Element::Text(leaf) => {
                leaf.mount(scene, parent, engine)?;
            }
            Element::Container(container) => {
                let id = scene.mount(parent, container.layout)?;
                mount_contents(
                    scene,
                    id,
                    container.children,
                    container.click,
                    engine,
                    handlers,
                )?;
            }
        }
    }
    Ok(())
}

struct Component<F>(F);

fn component<F: FnOnce(&mut Mount<'_>) -> Branch>(factory: F) -> Component<F> {
    Component(factory)
}

struct Instance {
    scene: Scene,
    handlers: HashMap<NodeId, Click>,
    make_detail: Box<dyn Fn() -> TextLeaf>,
    detail: Option<NodeId>,
    owned: Vec<SignalId>,
}

struct Host {
    state: Runtime<bool>,
    engine: SharedEngine,
    instances: HashMap<NodeId, Instance>,
}

impl Host {
    fn new() -> Self {
        Self {
            state: Runtime::new(false),
            engine: Engine::new(),
            instances: HashMap::new(),
        }
    }

    fn mount(
        &mut self,
        component: Component<impl FnOnce(&mut Mount<'_>) -> Branch>,
    ) -> Result<NodeId> {
        let mut mount = Mount {
            state: &mut self.state,
            owned: Vec::new(),
        };
        let Branch {
            container,
            open,
            detail,
        } = (component.0)(&mut mount);
        let mut scene = Scene::new(container.layout);
        let root = scene.root();
        let mut handlers = HashMap::new();
        mount_contents(
            &mut scene,
            root,
            container.children,
            container.click,
            &self.engine,
            &mut handlers,
        )?;
        let scope = mount.state.append(mount.state.tree().root(), false)?;
        if let Err(error) = mount
            .state
            .bind(open, scope, |value, desired| *desired = *value)
        {
            mount.state.remove_subtree(scope)?;
            return Err(error.into());
        }
        self.instances.insert(
            scope,
            Instance {
                scene,
                handlers,
                make_detail: detail,
                detail: None,
                owned: std::mem::take(&mut mount.owned),
            },
        );
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

    fn flush(&mut self) -> Result<()> {
        self.state.flush();
        for (&scope, instance) in &mut self.instances {
            let open = *self.state.tree().get(scope).expect("live mounted scope");
            match (open, instance.detail) {
                (true, None) => {
                    let root = instance.scene.root();
                    instance.detail = Some((instance.make_detail)().mount(
                        &mut instance.scene,
                        root,
                        &self.engine,
                    )?);
                }
                (false, Some(detail)) => {
                    instance.scene.unmount_leaf(detail)?;
                    instance.detail = None;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn unmount(&mut self, scope: NodeId) -> Result<()> {
        self.state.remove_subtree(scope)?;
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

    fn section(open: Signal<bool>, detail: impl Fn() -> TextLeaf + 'static) -> Branch {
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
        assert!(host.instances[&scope].detail.is_none());
        host.click(scope, header)?;
        host.flush()?;
        assert_eq!(builds.get(), 0);

        host.click(scope, header)?;
        host.flush()?;
        let first = host.instances[&scope].detail.unwrap();
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
        assert_eq!(host.instances[&scope].detail, Some(first));
        assert_eq!(builds.get(), 1);
        assert!(*host.state.read(open)?);

        host.click(scope, header)?;
        host.flush()?;
        assert!(host.instances[&scope].scene.body::<Text>(first).is_err());
        host.click(scope, header)?;
        host.flush()?;
        assert_ne!(host.instances[&scope].detail.unwrap(), first);
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
        assert!(host.instances[&scope].detail.is_some());
        assert!(host.instances[&sibling].detail.is_none());
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
    fn borrowed_state_survives_remount_and_stale_input_rolls_back_mount() -> Result<()> {
        let mut host = Host::new();
        let open = host.state.create_signal(true);
        let make = || component(|_| section(open, || text("Detail")));
        let scope = host.mount(make())?;
        host.flush()?;
        assert!(host.instances[&scope].detail.is_some());
        host.click(
            scope,
            *host.instances[&scope].handlers.keys().next().unwrap(),
        )?;
        host.unmount(scope)?;
        let scope = host.mount(make())?;
        host.flush()?;
        assert!(!*host.state.read(open)?);
        assert!(host.instances[&scope].detail.is_none());
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
