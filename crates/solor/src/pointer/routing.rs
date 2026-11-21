//! Synchronous direct delivery to live nodes from accepted hit geometry.
use std::{cell::OnceCell, collections::HashMap, sync::Arc};

use crate::state::Runtime;
use anyhow::{Context as _, ensure};

use super::{
    Handlers, PointerButton, PointerDownContext, PointerEvent, PointerFrame, PointerId,
    PointerInput, PointerSample, ScrollContext, ScrollDelta, ScrollEvent,
    frame::{HitGeometry, Recipients},
    hover::HoverChange,
};
use crate::{
    Context, Instance, NodeId, Result, Ui, ViewId, WindowControl, element::MountedNode,
    text::TextSystem,
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
    pub(super) geometry: OnceCell<Arc<HitGeometry>>,
    presses: HashMap<PointerId, Vec<Press>>,
    pub(super) hover: super::hover::Hover,
    pub(crate) tooltips: super::tooltip::Tooltips,
}

impl PointerRouter {
    pub(crate) fn frame_listeners(&self) -> &[NodeId] {
        self.frame
            .as_ref()
            .map_or(&[], PointerFrame::frame_listeners)
    }
    pub(crate) fn invalidate_geometry(&mut self) {
        self.geometry.take();
    }

    pub(crate) fn retire(&mut self, node: NodeId) {
        self.hover.retire(node);
        self.tooltips.retire(node);
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
    tooltip_style: &'a crate::TooltipStyle,
    instance: &'a mut Instance,
    state: &'a mut Runtime<()>,
    engine: &'a TextSystem,
}

fn context_and_handlers<'a>(
    node: &'a mut MountedNode,
    state: &'a mut Runtime<()>,
    engine: &'a TextSystem,
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
    fn hover_changed(&mut self) -> Result<()> {
        let router = &mut self.instance.pointer;
        let changes = router
            .hover
            .reconcile(router.frame.as_ref(), &self.instance.bodies);
        router
            .tooltips
            .reconcile(&router.hover, self.tooltip_style, &self.instance.bodies);
        let mut result = Ok(());
        for (id, change) in changes {
            let node = self
                .instance
                .bodies
                .get_mut(&id)
                .expect("live hover listener");
            let (handlers, mut cx) = context_and_handlers(node, self.state, self.engine);
            match change {
                HoverChange::Presence(entered) => {
                    if let Some(handler) = &handlers.hover {
                        result = result.and(handler(entered, &mut cx));
                    }
                }
                HoverChange::Position(position) => {
                    if let Some(handler) = &handlers.hover_position {
                        result = result.and(handler(position, &mut cx));
                    }
                }
            }
        }
        result
    }

    fn hit(
        &self,
        sample: PointerSample,
        select: impl FnOnce(Recipients) -> Option<NodeId>,
    ) -> Option<NodeId> {
        let (leaf, recipients) = self.instance.pointer.frame.as_ref()?.hit(sample.position)?;
        let recipient = select(recipients)?;
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
        let content = event.content_position();
        ensure!(
            content.x.get().is_finite() && content.y.get().is_finite(),
            "invalid local pointer position"
        );
        Ok(event)
    }

    fn down(
        &mut self,
        sample: PointerSample,
        button: PointerButton,
    ) -> Result<Option<WindowControl>> {
        let router = &self.instance.pointer;
        if router
            .presses
            .get(&sample.id)
            .is_some_and(|active| active.iter().any(|p| p.button == button))
        {
            return Ok(None);
        }
        let capture = router.capture(sample.id);
        let Some(owner) = capture.or_else(|| self.hit(sample, |r| r.pointer)) else {
            return Ok(None);
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
        if button == PointerButton::Primary
            && capture.is_none()
            && !captured
            && let Some(control @ (WindowControl::Drag | WindowControl::Resize(_))) =
                handlers.control
        {
            return Ok(Some(control));
        }
        let eligible = button == PointerButton::Primary
            && capture.is_none()
            && !captured
            && (handlers.click.is_some() || handlers.control.is_some());
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
        Ok(None)
    }

    fn scroll(&mut self, sample: PointerSample, mut delta: ScrollDelta) -> Result<()> {
        let Some(mut owner) = self.hit(sample, |r| r.scroll) else {
            return Ok(());
        };
        let frame = self
            .instance
            .pointer
            .frame
            .as_ref()
            .expect("accepted scroll hit");
        loop {
            let pointer = self.event(owner, sample, None)?;
            let node = self
                .instance
                .bodies
                .get_mut(&owner)
                .expect("live scroll ancestor");
            let (handlers, context) = context_and_handlers(node, self.state, self.engine);
            let scroll = handlers.scroll.as_ref().expect("accepted scroll listener");
            let mut cx = ScrollContext {
                context,
                received: delta,
                remaining: ScrollDelta::default(),
            };
            scroll(&ScrollEvent { pointer, delta }, &mut cx)?;
            delta = cx.remaining;
            if delta == ScrollDelta::default() {
                break;
            }
            let Some(parent) = frame.scroll_parent(owner) else {
                break;
            };
            owner = parent;
        }
        Ok(())
    }

    fn moved(&mut self, sample: PointerSample) -> Result<()> {
        let capture = self.instance.pointer.capture(sample.id);
        let target = capture.or_else(|| self.hit(sample, |r| r.pointer));
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

    fn up(
        &mut self,
        sample: PointerSample,
        button: PointerButton,
    ) -> Result<Option<WindowControl>> {
        let capture = self.instance.pointer.capture(sample.id);
        let Some(press) = self.instance.pointer.release(sample.id, button) else {
            return Ok(None);
        };
        let Some(target) = capture.or_else(|| self.hit(sample, |r| r.pointer)) else {
            return Ok(None);
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
        if press.eligible && target == press.owner {
            if let Some(click) = &handlers.click {
                click(&mut cx)?;
            }
            return Ok(handlers.control);
        }
        Ok(None)
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

impl PointerInput {
    fn validate(self) -> Result<()> {
        let sample = match self {
            Self::Scroll(p, delta) => {
                ensure!(
                    delta.x.get().is_finite() && delta.y.get().is_finite(),
                    "invalid scroll distance"
                );
                p
            }
            Self::Move(p) | Self::Down(p, _) | Self::Up(p, _) => p,
            Self::Leave(_) | Self::Cancel(_) => return Ok(()),
        };
        ensure!(
            sample.position.x.get().is_finite() && sample.position.y.get().is_finite(),
            "invalid pointer position"
        );
        Ok(())
    }
}

impl Ui {
    fn pointer_delivery(&mut self, view: ViewId) -> Result<Delivery<'_>> {
        let instance = self
            .instances
            .get_mut(&view)
            .context("pointer view is no longer mounted")?;
        Ok(Delivery {
            tooltip_style: &self.tooltip_style,
            instance,
            state: self.state.get_mut(),
            engine: &self.engine,
        })
    }

    /// Returns the cursor for this mouse in the accepted view, or the default arrow outside it.
    pub fn pointer_cursor(&self, view: ViewId, pointer: PointerId) -> crate::style::Cursor {
        self.instances
            .get(&view)
            .map_or(crate::style::Cursor::Default, |instance| {
                instance.pointer.hover.cursor(pointer)
            })
    }

    /// Earliest time an idle host should record this view for pointer feedback.
    /// A host rechecks this after input and after accepting a frame.
    pub fn next_redraw(&self, view: ViewId) -> Option<std::time::Instant> {
        let instance = self.instances.get(&view)?;
        if self.has_pending_bindings() {
            return Some(std::time::Instant::now());
        }
        instance.pointer.tooltips.deadline()
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
        let changed = router
            .frame
            .as_ref()
            .is_none_or(|old| !old.same_geometry(&frame));
        router.frame = Some(frame);
        let result = delivery.cancel_ids(canceled);
        if changed {
            result.and(delivery.hover_changed())
        } else {
            result
        }
    }

    /// Delivers host input against the view's accepted frame. Returns the native operation requested by this event.
    pub fn dispatch_pointer(
        &mut self,
        view: ViewId,
        input: PointerInput,
    ) -> Result<Option<WindowControl>> {
        let mut delivery = self.pointer_delivery(view)?;
        input.validate()?;
        if matches!(input, PointerInput::Scroll(_, d) if d == ScrollDelta::default()) {
            return Ok(None);
        }
        let router = &mut delivery.instance.pointer;
        router.hover.observe(input);
        router.tooltips.observe(input, &router.hover);
        let hover = delivery.hover_changed();
        let routed = match input {
            PointerInput::Down(sample, button) => delivery.down(sample, button),
            PointerInput::Scroll(sample, delta) => delivery.scroll(sample, delta).map(|()| None),
            PointerInput::Move(sample) => delivery.moved(sample).map(|()| None),
            PointerInput::Up(sample, button) => delivery.up(sample, button),
            PointerInput::Cancel(id) => delivery.cancel(id).map(|()| None),
            PointerInput::Leave(id) if delivery.instance.pointer.capture(id).is_none() => {
                delivery.cancel(id).map(|()| None)
            }
            PointerInput::Leave(_) => Ok(None),
        };
        hover.and(routed)
    }

    /// Cancels every active pointer on view deactivation, even if a callback reports an error.
    pub fn cancel_pointers(&mut self, view: ViewId) -> Result<()> {
        let mut delivery = self.pointer_delivery(view)?;
        let ids = delivery.instance.pointer.presses.keys().copied().collect();
        delivery.instance.pointer.hover.clear();
        delivery.hover_changed().and(delivery.cancel_ids(ids))
    }
}
