//! Physical mouse presence on the frontmost branch of an accepted view.
use super::{
    HoverPosition, PointerButton, PointerFrame, PointerId, PointerInput, PointerKind, PointerSample,
};
use crate::{NodeId, WindowControl, layout::Origin, style::Cursor};
use smallvec::SmallVec;
use std::collections::HashMap;

type Owners = SmallVec<[NodeId; 4]>;

struct Mouse {
    sample: PointerSample,
    path: Owners,
    cursor: Option<Cursor>,
    buttons: SmallVec<[PointerButton; 2]>,
}

pub(super) enum HoverChange {
    Presence(bool),
    Position(Option<HoverPosition>),
}

type Listeners = SmallVec<[(NodeId, Option<HoverPosition>); 2]>;

#[derive(Default)]
pub(super) struct Hover {
    // Oldest to newest so a departing mouse hands feedback back to the previous one.
    mice: Vec<Mouse>,
    inside: Listeners,
}

impl Hover {
    pub(super) fn observe(&mut self, input: PointerInput) {
        let sample = match input {
            PointerInput::Move(p)
            | PointerInput::Down(p, _)
            | PointerInput::Up(p, _)
            | PointerInput::Scroll(p, _) => p,
            PointerInput::Leave(id) | PointerInput::Cancel(id) => {
                self.mice.retain(|m| m.sample.id != id);
                return;
            }
        };
        if sample.kind != PointerKind::Mouse {
            return;
        }
        let mouse = if let Some(i) = self.mice.iter().position(|m| m.sample.id == sample.id) {
            self.mice[i..].rotate_left(1);
            let mouse = self.mice.last_mut().expect("observed mouse");
            mouse.sample = sample;
            mouse
        } else {
            self.mice.push_mut(Mouse {
                sample,
                path: Owners::new(),
                cursor: None,
                buttons: SmallVec::new(),
            })
        };
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
        self.inside.retain(|(node, _)| *node != id);
    }
    fn mouse(&self, id: PointerId) -> Option<&Mouse> {
        self.mice.iter().find(|m| m.sample.id == id)
    }
    pub(super) fn position(&self, id: PointerId) -> Option<Origin> {
        Some(self.mouse(id)?.sample.position)
    }
    pub(super) fn cursor(&self, id: PointerId) -> Cursor {
        self.mouse(id)
            .and_then(|m| m.cursor)
            .unwrap_or(Cursor::Default)
    }

    pub(super) fn path(&self, id: PointerId) -> &[NodeId] {
        self.mouse(id).map_or(&[], |m| &m.path)
    }
    pub(super) fn unpressed(&self, id: PointerId) -> bool {
        self.mouse(id).is_some_and(|m| m.buttons.is_empty())
    }

    pub(super) fn reconcile(
        &mut self,
        frame: Option<&PointerFrame>,
        bodies: &HashMap<NodeId, crate::element::MountedNode>,
    ) -> SmallVec<[(NodeId, HoverChange); 4]> {
        let mut next = Listeners::new();
        for mouse in self.mice.iter_mut().rev() {
            mouse.path.clear();
            mouse.cursor = None;
            if let Some(frame) = frame {
                let target = frame.mouse_target(mouse.sample.position, |n| bodies.contains_key(&n));
                mouse.cursor = target.cursor.or_else(|| {
                    let handlers = bodies.get(&target.pointer?)?.handlers.as_deref()?;
                    match handlers.control {
                        Some(WindowControl::Resize(edge)) => Some(Cursor::Resize(edge)),
                        _ => None,
                    }
                });
                mouse.path.extend(frame.hover_path(target.hover));
                for &node in &mouse.path {
                    let Some(handlers) = bodies.get(&node).and_then(|b| b.handlers.as_deref())
                    else {
                        continue;
                    };
                    if (handlers.hover.is_some() || handlers.hover_position.is_some())
                        && !next.iter().any(|(id, _)| *id == node)
                    {
                        let position = handlers.hover_position.as_ref().map(|_| {
                            let event = frame
                                .event(node, mouse.sample, None)
                                .expect("accepted hover recipient");
                            HoverPosition {
                                position: event.content_position(),
                                size: event.content_size(),
                            }
                        });
                        next.push((node, position));
                    }
                }
            }
        }
        let mut changes = SmallVec::new();
        for &(node, previous) in &self.inside {
            if !next.iter().any(|(id, _)| *id == node) {
                changes.push((node, HoverChange::Presence(false)));
                if previous.is_some() {
                    changes.push((node, HoverChange::Position(None)));
                }
            }
        }
        for &(node, current) in next.iter().rev() {
            let previous = self.inside.iter().find(|(id, _)| *id == node);
            if previous.is_none() {
                changes.push((node, HoverChange::Presence(true)));
            }
            if current.is_some() && current != previous.and_then(|(_, pos)| *pos) {
                changes.push((node, HoverChange::Position(current)));
            }
        }
        self.inside = next;
        changes
    }
}
