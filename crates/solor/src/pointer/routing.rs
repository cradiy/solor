//! Synchronous direct delivery to live nodes from accepted hit geometry.
use std::collections::HashMap;

use crate::state::Runtime;
use anyhow::{Context as _, ensure};

use super::{
    Handlers, PointerButton, PointerDownContext, PointerEvent, PointerFrame, PointerId,
    PointerInput, PointerSample,
};
use crate::{
    Context, Instance, NodeId, Result, Ui, ViewId, element::MountedNode, text::SharedEngine,
};

struct Press {
    owner: NodeId,
    button: PointerButton,
    captured: bool,
    eligible: bool,
}

#[derive(Default)]
pub(crate) struct PointerRouter {
    pub(super) frame: Option<PointerFrame>,
    presses: HashMap<PointerId, Vec<Press>>,
}

impl PointerRouter {
    pub(crate) fn retire(&mut self, node: NodeId) {
        self.presses.retain(|_, presses| {
            presses.retain(|press| press.owner != node);
            !presses.is_empty()
        });
    }

    fn capture(&self, id: PointerId) -> Option<NodeId> {
        self.presses
            .get(&id)?
            .iter()
            .find(|p| p.captured)
            .map(|p| p.owner)
    }

    fn release(&mut self, id: PointerId, button: PointerButton) -> Option<Press> {
        let presses = self.presses.get_mut(&id)?;
        let index = presses.iter().position(|p| p.button == button)?;
        let press = presses.swap_remove(index);
        if presses.is_empty() {
            self.presses.remove(&id);
        }
        Some(press)
    }
}

// The callback can change signals, but structural reconciliation occurs only at flush.
struct Delivery<'a> {
    instance: &'a mut Instance,
    state: &'a mut Runtime<()>,
    engine: Option<&'a SharedEngine>,
}

fn context_and_handlers<'a>(
    node: &'a mut MountedNode,
    state: &'a mut Runtime<()>,
    engine: Option<&'a SharedEngine>,
) -> (&'a Handlers, Context<'a>) {
    (
        node.handlers.as_deref().expect("interactive recipient"),
        Context {
            state,
            engine,
            owned: Some(&mut node.owned),
        },
    )
}

impl Delivery<'_> {
    fn hit(&self, sample: PointerSample) -> Option<NodeId> {
        let (leaf, recipient) = self.instance.pointer.frame.as_ref()?.hit(sample.position)?;
        // A retired frontmost hit blocks everything underneath, even if its ancestor survives.
        (self.instance.bodies.contains_key(&leaf) && self.instance.bodies.contains_key(&recipient))
            .then_some(recipient)
    }

    fn event(
        &self,
        node: NodeId,
        sample: PointerSample,
        button: Option<PointerButton>,
    ) -> Result<PointerEvent> {
        let event = self
            .instance
            .pointer
            .frame
            .as_ref()
            .and_then(|frame| frame.event(node, sample, button))
            .context("recipient missing from accepted pointer frame")?;
        ensure!(
            event.position.x.get().is_finite() && event.position.y.get().is_finite(),
            "invalid local pointer position"
        );
        Ok(event)
    }

    fn down(&mut self, sample: PointerSample, button: PointerButton) -> Result<()> {
        let router = &self.instance.pointer;
        if router
            .presses
            .get(&sample.id)
            .is_some_and(|active| active.iter().any(|p| p.button == button))
        {
            return Ok(());
        }
        let capture = router.capture(sample.id);
        let Some(owner) = capture.or_else(|| self.hit(sample)) else {
            return Ok(());
        };
        let event = self.event(owner, sample, Some(button))?;
        let node = self
            .instance
            .bodies
            .get_mut(&owner)
            .expect("live recipient");
        let (handlers, context) = context_and_handlers(node, self.state, self.engine);
        let mut cx = PointerDownContext {
            context,
            capture: false,
        };
        if let Some(down) = &handlers.down {
            down(&event, &mut cx)?;
        }
        let captured = cx.capture && capture.is_none();
        let eligible = button == PointerButton::Primary
            && capture.is_none()
            && !captured
            && handlers.click.is_some();
        let presses = self.instance.pointer.presses.entry(sample.id).or_default();
        if captured {
            for press in presses.iter_mut() {
                press.eligible = false;
            }
        }
        presses.push(Press {
            owner,
            button,
            captured,
            eligible,
        });
        Ok(())
    }

    fn moved(&mut self, sample: PointerSample) -> Result<()> {
        let capture = self.instance.pointer.capture(sample.id);
        let target = capture.or_else(|| self.hit(sample));
        if let Some(presses) = self.instance.pointer.presses.get_mut(&sample.id) {
            for press in presses {
                press.eligible &= target == Some(press.owner);
            }
        }
        let Some(target) = target else {
            return Ok(());
        };
        let event = self.event(target, sample, None)?;
        let node = self
            .instance
            .bodies
            .get_mut(&target)
            .expect("live recipient");
        let (handlers, mut cx) = context_and_handlers(node, self.state, self.engine);
        if let Some(moved) = &handlers.moved {
            moved(&event, &mut cx)?;
        }
        Ok(())
    }

    fn up(&mut self, sample: PointerSample, button: PointerButton) -> Result<()> {
        let capture = self.instance.pointer.capture(sample.id);
        let Some(press) = self.instance.pointer.release(sample.id, button) else {
            return Ok(());
        };
        let Some(target) = capture.or_else(|| self.hit(sample)) else {
            return Ok(());
        };
        let event = self.event(target, sample, Some(button))?;
        let node = self
            .instance
            .bodies
            .get_mut(&target)
            .expect("live recipient");
        let (handlers, mut cx) = context_and_handlers(node, self.state, self.engine);
        if let Some(up) = &handlers.up {
            up(&event, &mut cx)?;
        }
        if press.eligible
            && target == press.owner
            && let Some(click) = &handlers.click
        {
            click(&mut cx)?;
        }
        Ok(())
    }

    fn cancel_ids(&mut self, ids: Vec<PointerId>) -> Result<()> {
        let mut result = Ok(());
        for id in ids {
            result = result.and(self.cancel(id));
        }
        result
    }

    fn cancel(&mut self, id: PointerId) -> Result<()> {
        let Some(presses) = self.instance.pointer.presses.remove(&id) else {
            return Ok(());
        };
        let mut result = Ok(());
        for (index, press) in presses.iter().enumerate() {
            if presses[..index]
                .iter()
                .any(|previous| previous.owner == press.owner)
            {
                continue;
            }
            let node = self
                .instance
                .bodies
                .get_mut(&press.owner)
                .expect("live press owner");
            let (handlers, mut cx) = context_and_handlers(node, self.state, self.engine);
            if let Some(cancel) = &handlers.cancel {
                result = result.and(cancel(id, &mut cx));
            }
        }
        result
    }
}

impl PointerSample {
    fn validated(self) -> Result<Self> {
        ensure!(
            self.position.x.get().is_finite() && self.position.y.get().is_finite(),
            "invalid pointer position"
        );
        Ok(self)
    }
}

impl Ui {
    fn pointer_delivery(&mut self, view: ViewId) -> Result<Delivery<'_>> {
        let instance = self
            .instances
            .get_mut(&view)
            .context("pointer view is no longer mounted")?;
        Ok(Delivery {
            instance,
            state: &mut self.state,
            engine: self.engine.as_ref(),
        })
    }

    /// Makes this submission's snapshot authoritative for input. Active sequences whose
    /// recipients disappear from the accepted frame are canceled after the switch.
    pub fn accept_pointer_frame(&mut self, frame: PointerFrame) -> Result<()> {
        let mut delivery = self.pointer_delivery(frame.view)?;
        let router = &mut delivery.instance.pointer;
        let canceled: Vec<_> = router
            .presses
            .iter()
            .filter(|(_, presses)| presses.iter().any(|p| !frame.has_recipient(p.owner)))
            .map(|(&id, _)| id)
            .collect();
        router.frame = Some(frame);
        delivery.cancel_ids(canceled)
    }

    /// Delivers host input against the view's accepted frame. Signal delivery occurs at flush.
    pub fn dispatch_pointer(&mut self, view: ViewId, input: PointerInput) -> Result<()> {
        let mut delivery = self.pointer_delivery(view)?;
        match input {
            PointerInput::Down(sample, button) => delivery.down(sample.validated()?, button),
            PointerInput::Move(sample) => delivery.moved(sample.validated()?),
            PointerInput::Up(sample, button) => delivery.up(sample.validated()?, button),
            PointerInput::Cancel(id) => delivery.cancel(id),
            PointerInput::Leave(id) if delivery.instance.pointer.capture(id).is_none() => {
                delivery.cancel(id)
            }
            PointerInput::Leave(_) => Ok(()),
        }
    }

    /// Cancels every active pointer on view deactivation, even if a callback reports an error.
    pub fn cancel_pointers(&mut self, view: ViewId) -> Result<()> {
        let mut delivery = self.pointer_delivery(view)?;
        let ids = delivery.instance.pointer.presses.keys().copied().collect();
        delivery.cancel_ids(ids)
    }
}
