//! Headless component scopes whose text leaves share one visual parent and hit region.
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

mod disposal_cost;
mod frame_commit;

use solor_core::{
    layout::{
        LayoutContext, LayoutError, LayoutNode, LayoutOutput, MeasureContext, Metrics, Origin,
        Query, Result, Scene, Size,
    },
    state::{BindingId, Runtime, Signal},
    tree::NodeId,
};

use crate::{
    font_fixture::{FONT, fixture_engine},
    frame::{Frame, STYLE, TextDraw},
    input::Point,
    pixels::{Clip, Surface},
};
use solor_ui::text::{SharedEngine, Text};

const SIZE: Size = Size {
    width: solor_core::px(128.0),
    height: solor_core::px(32.0),
};
const INSIDE: Point = Point { x: 5.0, y: 10.0 };

struct Overlay;

impl LayoutNode for Overlay {
    fn measure(&mut self, _: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics> {
        Ok(Metrics {
            size: Size {
                width: query.resolve(SIZE.width),
                ..SIZE
            },
            baseline: None,
        })
    }

    fn layout(&mut self, cx: &mut LayoutContext<'_, '_>, assigned: Size) -> Result<LayoutOutput> {
        for index in 0..cx.child_count() {
            cx.layout_child(index, assigned)?;
            cx.place_child(index, Origin::default())?;
        }
        Ok(LayoutOutput {
            content: assigned,
            baseline: None,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Delivery {
    Binding(NodeId),
    Action(NodeId),
    Drop(NodeId),
}

struct Label {
    visual: NodeId,
    ink: u8,
    retiring: Cell<bool>,
    log: Rc<RefCell<Vec<Delivery>>>,
}

impl Drop for Label {
    fn drop(&mut self) {
        self.log.borrow_mut().push(Delivery::Drop(self.visual));
    }
}

struct Mount {
    visual: NodeId,
    state: NodeId,
    binding: BindingId,
}

impl Mount {
    fn label<'a>(&self, state: &'a Runtime<Option<Label>>) -> &'a Label {
        state.tree().get(self.state).unwrap().as_ref().unwrap()
    }
}

#[derive(Clone, Copy)]
struct Scope(NodeId);

enum Removal {
    None,
    Leaf(NodeId),
    Scope(Scope),
}

struct Fixture {
    scene: Scene,
    engine: SharedEngine,
    state: Runtime<Option<Label>>,
    ink: Signal<u8>,
    members: Vec<Mount>,
    pressed: Option<(u64, NodeId)>,
    accepted: Option<Rc<Frame>>,
    fail_draw: Option<NodeId>,
    log: Rc<RefCell<Vec<Delivery>>>,
}

impl Fixture {
    fn new() -> Self {
        let mut state = Runtime::new(None);
        let ink = state.create_signal(20);
        Self {
            scene: Scene::new(Overlay),
            engine: fixture_engine(),
            state,
            ink,
            members: Vec::new(),
            pressed: None,
            accepted: None,
            fail_draw: None,
            log: Rc::default(),
        }
    }

    fn scope(&mut self, parent: Option<Scope>) -> Scope {
        let owner = parent.map_or(self.state.tree().root(), |scope| scope.0);
        Scope(self.state.append(owner, None).unwrap())
    }

    fn mount(&mut self, scope: Scope, text: &str) -> NodeId {
        let text = Text::new(self.engine.clone(), text, STYLE, true).unwrap();
        let visual = self.scene.mount(self.scene.root(), text).unwrap();
        let label = Label {
            visual,
            ink: 0,
            retiring: Cell::new(false),
            log: self.log.clone(),
        };
        let state = self.state.append(scope.0, Some(label)).unwrap();
        let binding = self
            .state
            .bind(self.ink, state, |ink, label| {
                let label = label.as_mut().unwrap();
                label.ink = *ink;
                label.log.borrow_mut().push(Delivery::Binding(label.visual));
            })
            .unwrap();
        self.members.push(Mount {
            visual,
            state,
            binding,
        });
        visual
    }

    fn retire_subtree(&mut self, root: NodeId) {
        if self.state.tree().get(root).is_none() {
            return;
        }
        self.visit_owned(root, |label| label.retiring.set(true));
        self.retire_matching(root, |state, mount| mount.label(state).retiring.get());
    }

    fn visit_owned(&self, root: NodeId, mut visit: impl FnMut(&Label)) {
        let tree = self.state.tree();
        let mut pending = vec![root];
        while let Some(node) = pending.pop() {
            if let Some(label) = tree.get(node).expect("live subtree node") {
                visit(label);
            } else {
                pending.extend(tree.children(node).unwrap());
            }
        }
    }

    fn retire_matching(
        &mut self,
        root: NodeId,
        mut owns: impl FnMut(&Runtime<Option<Label>>, &Mount) -> bool,
    ) {
        self.members.retain(|mount| {
            if !owns(&self.state, mount) {
                return true;
            }
            if self.pressed.is_some_and(|(_, id)| id == mount.visual) {
                self.pressed = None;
            }
            self.state.unbind(mount.binding).unwrap();
            self.scene.unmount_leaf(mount.visual).unwrap();
            false
        });
        self.state.remove_subtree(root).unwrap();
    }

    // Structural requests enter at the boundary before bindings and frame preparation.
    fn prepare(&mut self, removal: Removal) -> Result<Rc<Frame>> {
        match removal {
            Removal::None => {}
            Removal::Leaf(id) => {
                if let Some(mount) = self.members.iter().find(|m| m.visual == id) {
                    self.retire_subtree(mount.state);
                }
            }
            Removal::Scope(scope) => self.retire_subtree(scope.0),
        }
        self.state.flush();
        self.scene.run_assigned(SIZE, false)?;
        let geometry = self.scene.shared_snapshot().unwrap();
        let mut draws = Vec::new();
        for member in &self.members {
            let label = member.label(&self.state);
            let row = self.scene.geometry(member.visual).unwrap();
            let paragraph = self
                .scene
                .body::<Text>(member.visual)?
                .resolve(Query::Exact(row.metrics.size.width))?;
            if self.fail_draw == Some(member.visual) {
                self.fail_draw = None;
                return Err(LayoutError::Node("injected draw failure"));
            }
            draws.push(TextDraw {
                node: member.visual,
                origin: row.origin,
                allocated: row.metrics.size,
                paragraph,
                color: [label.ink, 30, 40, 255],
            });
        }
        Ok(Rc::new(Frame {
            geometry,
            draws,
            revision: self.accepted.as_ref().map_or(1, |f| f.revision + 1),
            scroll_y: solor_core::px(0.0),
            font: FONT,
        }))
    }

    fn hit(&self, point: Point) -> Option<&Label> {
        // Resolve the frontmost hit in the accepted frame before consulting live membership.
        let target = hit_node(self.accepted.as_ref()?, SIZE, point)?;
        let member = self.members.iter().find(|m| m.visual == target)?;
        Some(member.label(&self.state))
    }

    fn down(&mut self, pointer: u64, point: Point) {
        if self.pressed.is_none() {
            self.pressed = self.hit(point).map(|label| (pointer, label.visual));
        }
    }

    fn up(&mut self, pointer: u64, point: Point) {
        if let Some((owner, target)) = self.pressed
            && owner == pointer
        {
            self.pressed = None;
            if let Some(label) = self.hit(point)
                && label.visual == target
            {
                label.log.borrow_mut().push(Delivery::Action(label.visual));
            }
        }
    }
}

fn hit_node(frame: &Frame, viewport: Size, point: Point) -> Option<NodeId> {
    if point.x < 0.0
        || point.x >= f64::from(viewport.width)
        || point.y < 0.0
        || point.y >= f64::from(viewport.height)
    {
        return None;
    }
    let draw = frame.draws.iter().rev().find(|draw| {
        let x = point.x - f64::from(draw.origin.x);
        let y = point.y + f64::from(frame.scroll_y) - f64::from(draw.origin.y);
        x >= 0.0
            && y >= 0.0
            && x < f64::from(draw.allocated.width)
            && y < f64::from(draw.allocated.height)
    })?;
    Some(draw.node)
}

fn pixels(frame: &Frame) -> Vec<u8> {
    let target = crate::pixels::RasterScale::ONE.buffer_size(SIZE).unwrap();
    Surface::draw(frame, target, Clip::canvas(target))
        .unwrap()
        .rgba
}

#[test]
fn pressed_leaf_is_retired_even_when_frame_preparation_fails() {
    for release_after_accept in [false, true] {
        let mut window = Fixture::new();
        let scope = window.scope(None);
        let label = window.mount(scope, "Label");
        let old = window.mount(scope, "Detail");
        let held = window.prepare(Removal::None).unwrap();
        window.accepted = Some(held.clone());
        let original = pixels(&held);
        window.down(1, INSIDE);
        assert_eq!(window.pressed, Some((1, old)));

        window.log.borrow_mut().clear();
        window.state.set(window.ink, 180).unwrap();
        assert_eq!(window.state.pending_count(), 2);
        window.fail_draw = Some(label);
        assert!(matches!(
            window.prepare(Removal::Leaf(old)),
            Err(LayoutError::Node("injected draw failure"))
        ));
        assert_eq!(
            *window.log.borrow(),
            [Delivery::Drop(old), Delivery::Binding(label)]
        );
        assert!(window.scene.geometry(old).is_none());
        assert_eq!(window.state.pending_count(), 0);
        assert_eq!(window.state.binding_count(), 1);
        assert!(window.pressed.is_none());
        assert!(Rc::ptr_eq(window.accepted.as_ref().unwrap(), &held));
        assert_eq!(pixels(&held), original);
        // The surviving label physically covers this point, behind the retired target.
        window.down(2, INSIDE);
        assert!(window.pressed.is_none());
        window.up(2, INSIDE);

        let retry = window.prepare(Removal::None).unwrap();
        assert_eq!(retry.draws.len(), 1);
        assert_eq!(retry.draws[0].node, label);
        assert_eq!(retry.draws[0].color[0], 180);
        assert_ne!(pixels(&retry), original);
        let replacement = window.mount(scope, "New");
        assert_ne!(old, replacement);
        let next = window.prepare(Removal::None).unwrap();
        assert_eq!(next.draws[1].origin, held.draws[1].origin);
        assert!(window.hit(INSIDE).is_none());
        window.accepted = Some(retry);
        assert_eq!(window.hit(INSIDE).unwrap().visual, label);
        if !release_after_accept {
            window.up(1, INSIDE);
        }
        window.accepted = Some(next);
        if release_after_accept {
            window.up(1, INSIDE);
        }
        window.down(1, INSIDE);
        window.up(1, INSIDE);
        assert_eq!(
            *window.log.borrow(),
            [
                Delivery::Drop(old),
                Delivery::Binding(label),
                Delivery::Binding(replacement),
                Delivery::Action(replacement),
            ]
        );
        drop(window);
        assert_eq!(pixels(&held), original);
    }
}

#[test]
fn disposing_sibling_mounts_clears_work_and_keeps_the_parent_signal() {
    let mut window = Fixture::new();
    for ink in 1..=32 {
        let scope = window.scope(None);
        let first = window.mount(scope, "One");
        let second = window.mount(scope, "Two");
        window.accepted = Some(window.prepare(Removal::None).unwrap());
        window.down(1, INSIDE);
        assert_eq!(window.pressed, Some((1, second)));
        window.state.set(window.ink, ink).unwrap();
        assert_eq!(window.state.pending_count(), 2);
        window.log.borrow_mut().clear();
        let empty = window.prepare(Removal::Scope(scope)).unwrap();
        assert!(window.pressed.is_none());
        window.up(1, INSIDE);
        assert_eq!(empty.geometry.len(), 1);
        assert!(empty.draws.is_empty());
        assert!(window.members.is_empty());
        assert_eq!(window.state.pending_count(), 0);
        assert_eq!(window.state.binding_count(), 0);
        assert_eq!(window.state.tree().node_count(), 1);
        assert_eq!(*window.state.read(window.ink).unwrap(), ink);
        let log = window.log.borrow();
        assert_eq!(log.len(), 2);
        assert!(log.contains(&Delivery::Drop(first)) && log.contains(&Delivery::Drop(second)));
    }
}

#[test]
fn child_disposal_preserves_parent_and_parent_later_disposes_the_remount() {
    let mut window = Fixture::new();
    let parent = window.scope(None);
    let child = window.scope(Some(parent));
    let title = window.mount(parent, "Title");
    let detail = window.mount(child, "Detail");
    let held = window.prepare(Removal::None).unwrap();
    window.accepted = Some(held.clone());
    let original = pixels(&held);
    window.down(1, INSIDE);
    assert_eq!(window.pressed, Some((1, detail)));

    window.log.borrow_mut().clear();
    window.state.set(window.ink, 80).unwrap();
    let surviving = window.prepare(Removal::Scope(child)).unwrap();
    assert_eq!(
        *window.log.borrow(),
        [Delivery::Drop(detail), Delivery::Binding(title)]
    );
    assert!(window.state.tree().get(child.0).is_none());
    assert!(window.state.tree().get(parent.0).is_some());
    assert!(window.pressed.is_none());
    assert!(window.hit(INSIDE).is_none());
    window.accepted = Some(surviving);
    window.down(2, INSIDE);
    assert_eq!(window.pressed, Some((2, title)));

    let new_child = window.scope(Some(parent));
    let new_detail = window.mount(new_child, "New detail");
    let remounted = window.prepare(Removal::Scope(child)).unwrap();
    assert_ne!(child.0, new_child.0);
    assert_ne!(detail, new_detail);
    assert_eq!(remounted.draws.len(), 2);
    assert_eq!(remounted.draws[1].node, new_detail);
    window.up(2, INSIDE);
    assert_eq!(window.log.borrow().last(), Some(&Delivery::Action(title)));
    window.accepted = Some(remounted);
    window.down(3, INSIDE);
    assert_eq!(window.pressed, Some((3, new_detail)));

    window.log.borrow_mut().clear();
    window.state.set(window.ink, 90).unwrap();
    let empty = window.prepare(Removal::Scope(parent)).unwrap();
    assert!(window.pressed.is_none());
    window.up(3, INSIDE);
    let log = window.log.borrow();
    assert_eq!(log.len(), 2);
    assert!(log.contains(&Delivery::Drop(title)) && log.contains(&Delivery::Drop(new_detail)));
    assert!(empty.draws.is_empty());
    assert_eq!(empty.geometry.len(), 1);
    assert!(window.members.is_empty());
    assert_eq!(window.state.tree().node_count(), 1);
    assert_eq!(window.state.binding_count(), 0);
    assert_eq!(window.state.pending_count(), 0);
    assert_eq!(pixels(&held), original);
}

#[test]
fn parent_disposal_cancels_descendant_work_without_consuming_an_unrelated_press() {
    let mut window = Fixture::new();
    let parent = window.scope(None);
    let sibling = window.scope(None);
    let child = window.scope(Some(parent));
    // The unrelated sibling holds a press before later mounts cover it.
    let first = window.mount(child, "First");
    let outside = window.mount(sibling, "Outside");
    window.accepted = Some(window.prepare(Removal::None).unwrap());
    window.down(1, INSIDE);
    assert_eq!(window.pressed, Some((1, outside)));

    let middle = window.mount(parent, "Middle");
    let last = window.mount(child, "Last");
    window.accepted = Some(window.prepare(Removal::None).unwrap());
    assert_eq!(window.hit(INSIDE).unwrap().visual, last);

    window.log.borrow_mut().clear();
    window.state.set(window.ink, 100).unwrap();
    let remaining = window.prepare(Removal::Scope(parent)).unwrap();
    assert_eq!(remaining.draws.len(), 1);
    assert_eq!(remaining.draws[0].node, outside);
    assert_eq!(remaining.geometry.len(), 2);
    assert_eq!(window.state.binding_count(), 1);
    assert!(window.state.tree().get(parent.0).is_none());
    assert!(window.state.tree().get(child.0).is_none());
    assert_eq!(window.pressed, Some((1, outside)));
    {
        let log = window.log.borrow();
        assert_eq!(log.len(), 4);
        for id in [first, middle, last] {
            assert!(log.contains(&Delivery::Drop(id)));
        }
        assert_eq!(log.last(), Some(&Delivery::Binding(outside)));
    }

    window.log.borrow_mut().clear();
    let fresh = window.scope(Some(sibling));
    let replacement = window.mount(fresh, "Replacement");
    let next = window.prepare(Removal::Scope(child)).unwrap();
    assert_eq!(next.draws.len(), 2);
    assert_eq!(next.draws[1].node, replacement);
    window.accepted = Some(remaining);
    window.up(1, INSIDE);
    assert_eq!(
        *window.log.borrow(),
        [Delivery::Binding(replacement), Delivery::Action(outside)]
    );
}
