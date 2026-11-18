//! Headless ownership fixture with two sibling text leaves covering the same hit region.
use std::{cell::RefCell, rc::Rc};

use solor_core::{
    layout::{
        LayoutContext, LayoutError, LayoutNode, LayoutOutput, MeasureContext, Metrics, Origin,
        Query, Result, Scene, Size,
    },
    state::{BindingId, Runtime, Signal},
    tree::NodeId,
};

use crate::{
    frame::{Frame, STYLE, TextDraw},
    input::Point,
    pixels::{Clip, Surface},
    text::{Engine, FONT, SharedEngine, Text},
};

const SIZE: Size = Size {
    width: 128,
    height: 32,
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

enum Removal {
    None,
    Leaf(NodeId),
    Scope,
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
            engine: Engine::new(),
            state,
            ink,
            members: Vec::new(),
            pressed: None,
            accepted: None,
            fail_draw: None,
            log: Rc::default(),
        }
    }

    fn mount(&mut self, text: &str) -> NodeId {
        let text = Text::new(self.engine.clone(), text, STYLE, true).unwrap();
        let visual = self.scene.mount(self.scene.root(), text).unwrap();
        let label = Label {
            visual,
            ink: 0,
            log: self.log.clone(),
        };
        let state = self
            .state
            .append(self.state.tree().root(), Some(label))
            .unwrap();
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

    fn remove(&mut self, index: usize) {
        let mount = self.members.remove(index);
        if self
            .pressed
            .is_some_and(|(_, target)| target == mount.visual)
        {
            self.pressed = None;
        }
        self.state.unbind(mount.binding).unwrap();
        self.scene.unmount_leaf(mount.visual).unwrap();
        self.state.remove_subtree(mount.state).unwrap();
    }

    // Structural requests enter at the boundary before bindings and frame preparation.
    fn prepare(&mut self, removal: Removal) -> Result<Rc<Frame>> {
        match removal {
            Removal::None => {}
            Removal::Leaf(id) => {
                let index = self.members.iter().position(|m| m.visual == id).unwrap();
                self.remove(index);
            }
            Removal::Scope => {
                while !self.members.is_empty() {
                    self.remove(self.members.len() - 1);
                }
            }
        }
        self.state.flush();
        self.scene.run_assigned(SIZE, false)?;
        let geometry = self.scene.shared_snapshot().unwrap();
        let mut draws = Vec::new();
        for member in &self.members {
            let label = self
                .state
                .tree()
                .get(member.state)
                .unwrap()
                .as_ref()
                .unwrap();
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
            scroll_y: 0,
            font: FONT,
        }))
    }

    fn hit(&self, point: Point) -> Option<&Label> {
        // Resolve the frontmost hit in the accepted frame before consulting live membership.
        let draw = self.accepted.as_ref()?.draws.iter().rev().find(|draw| {
            let x = point.x - f64::from(draw.origin.x);
            let y = point.y - f64::from(draw.origin.y);
            x >= 0.0
                && y >= 0.0
                && x < f64::from(draw.allocated.width)
                && y < f64::from(draw.allocated.height)
        })?;
        let member = self.members.iter().find(|m| m.visual == draw.node)?;
        self.state.tree().get(member.state)?.as_ref()
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

fn pixels(frame: &Frame) -> Vec<u8> {
    Surface::draw(frame, SIZE, Clip::canvas(SIZE)).unwrap().rgba
}

#[test]
fn pressed_leaf_is_retired_even_when_frame_preparation_fails() {
    for release_after_accept in [false, true] {
        let mut window = Fixture::new();
        let label = window.mount("Label");
        let old = window.mount("Detail");
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
        let replacement = window.mount("New");
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
        let first = window.mount("One");
        let second = window.mount("Two");
        window.accepted = Some(window.prepare(Removal::None).unwrap());
        window.down(1, INSIDE);
        assert_eq!(window.pressed, Some((1, second)));
        window.state.set(window.ink, ink).unwrap();
        assert_eq!(window.state.pending_count(), 2);
        window.log.borrow_mut().clear();
        let empty = window.prepare(Removal::Scope).unwrap();
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
