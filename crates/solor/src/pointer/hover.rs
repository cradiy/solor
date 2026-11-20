//! Physical mouse presence on the frontmost branch of an accepted view.
use super::{PointerButton, PointerFrame, PointerId, PointerInput, PointerKind};
use crate::{NodeId, WindowControl, layout::Origin, style::Cursor};
use smallvec::SmallVec;
use std::collections::HashMap;

type Owners = SmallVec<[NodeId; 4]>;

#[derive(Default)]
struct Mouse {
    at: Origin,
    path: Owners,
    cursor: Option<Cursor>,
    buttons: SmallVec<[PointerButton; 2]>,
}

#[derive(Default)]
pub(super) struct Hover {
    mice: HashMap<PointerId, Mouse>,
    inside: Owners,
}

impl Hover {
    pub(super) fn observe(&mut self, input: PointerInput) {
        let sample = match input {
            PointerInput::Move(p)
            | PointerInput::Down(p, _)
            | PointerInput::Up(p, _)
            | PointerInput::Scroll(p, _) => p,
            PointerInput::Leave(id) | PointerInput::Cancel(id) => {
                self.mice.remove(&id);
                return;
            }
        };
        if sample.kind != PointerKind::Mouse {
            return;
        }
        let mouse = self.mice.entry(sample.id).or_default();
        mouse.at = sample.position;
        match input {
            PointerInput::Down(_, b) if !mouse.buttons.contains(&b) => mouse.buttons.push(b),
            PointerInput::Up(_, b) => mouse.buttons.retain(|v| *v != b),
            _ => {}
        }
    }

    pub(super) fn clear(&mut self) {
        self.mice.clear();
    }
    pub(super) fn retire(&mut self, id: NodeId) {
        self.inside.retain(|n| *n != id);
    }
    pub(super) fn position(&self, id: PointerId) -> Option<Origin> {
        Some(self.mice.get(&id)?.at)
    }
    pub(super) fn cursor(&self, id: PointerId) -> Cursor {
        self.mice
            .get(&id)
            .and_then(|m| m.cursor)
            .unwrap_or(Cursor::Default)
    }

    pub(super) fn path(&self, id: PointerId) -> &[NodeId] {
        self.mice.get(&id).map_or(&[], |m| &m.path)
    }
    pub(super) fn unpressed(&self, id: PointerId) -> bool {
        self.mice.get(&id).is_some_and(|m| m.buttons.is_empty())
    }

    pub(super) fn reconcile(
        &mut self,
        frame: Option<&PointerFrame>,
        bodies: &HashMap<NodeId, crate::element::MountedNode>,
    ) -> SmallVec<[(NodeId, bool); 4]> {
        let mut next = Owners::new();
        for mouse in self.mice.values_mut() {
            mouse.path.clear();
            mouse.cursor = None;
            if let Some(frame) = frame {
                let target = frame.mouse_target(mouse.at, |n| bodies.contains_key(&n));
                mouse.cursor = target.cursor.or_else(|| {
                    let handlers = bodies.get(&target.pointer?)?.handlers.as_deref()?;
                    match handlers.control {
                        Some(WindowControl::Resize(edge)) => Some(Cursor::Resize(edge)),
                        _ => None,
                    }
                });
                mouse.path.extend(frame.hover_path(target.hover));
                for &node in &mouse.path {
                    if !next.contains(&node)
                        && bodies
                            .get(&node)
                            .and_then(|b| b.handlers.as_deref())
                            .is_some_and(|h| h.hover.is_some())
                    {
                        next.push(node);
                    }
                }
            }
        }
        let mut changes = SmallVec::new();
        changes.extend(
            self.inside
                .iter()
                .filter(|n| !next.contains(n))
                .map(|&n| (n, false)),
        );
        changes.extend(
            next.iter()
                .rev()
                .filter(|n| !self.inside.contains(n))
                .map(|&n| (n, true)),
        );
        self.inside = next;
        changes
    }
}
