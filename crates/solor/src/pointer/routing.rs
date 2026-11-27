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
use crate::{Context, Instance, NodeId, Result, Ui, ViewId, WindowControl, text::TextSystem};

struct Press {
    owner: NodeId,
    button: PointerButton,
    captured: bool,
    eligible: bool,
}

#[derive(Default)]
pub(crate) struct PointerRouter {
    fixed_size: bool,
    pub(super) frame: Option<PointerFrame>,
    pub(crate) scroll_offsets: HashMap<NodeId, crate::layout::Origin>,
    pub(super) scroll_pending: bool,
    pub(super) geometry: OnceCell<Arc<HitGeometry>>,
    presses: HashMap<PointerId, Vec<Press>>,
    pub(super) hover: super::hover::Hover,
    pub(crate) tooltips: super::tooltip::Tooltips,
}

impl PointerRouter {
    pub(crate) fn callback_focus<'a>(
        &'a self,
        focused: Option<NodeId>,
        request: &'a mut Option<crate::FocusHandle>,
    ) -> crate::keyboard::CallbackFocus<'a> {
        crate::keyboard::CallbackFocus {
            request,
            target: focused.zip(self.frame.as_ref()),
        }
    }

    pub(crate) fn frame_listeners(&self) -> &[NodeId] {
        self.frame
            .as_ref()
            .map_or(&[], PointerFrame::frame_listeners)
    }
    pub(crate) fn invalidate_geometry(&mut self) {
        self.geometry.take();
    }

    pub(crate) fn retire(&mut self, node: NodeId) {
        self.scroll_offsets.remove(&node);
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
    focus_request: Option<crate::FocusHandle>,
    tasks: &'a crate::Tasks,
    tooltip_style: &'a crate::TooltipStyle,
    instance: &'a mut Instance,
    state: &'a mut Runtime<()>,
    engine: &'a TextSystem,
}

impl Instance {
    fn hit_recipient(
        &self,
        position: crate::layout::Origin,
        select: impl FnOnce(Recipients) -> Option<NodeId>,
    ) -> Option<NodeId> {
        let (leaf, recipients) = self.pointer.frame.as_ref()?.hit(position)?;
        let recipient = select(recipients)?;
        // A retired frontmost hit blocks everything underneath, even if its ancestor survives.
        (self.bodies.contains_key(&leaf) && self.bodies.contains_key(&recipient))
            .then_some(recipient)
    }
}

impl Delivery<'_> {
    fn handlers(&mut self, target: NodeId) -> Option<(&Handlers, Context<'_>)> {
        let node = self.instance.bodies.get_mut(&target)?;
        Some((
            node.handlers.as_deref()?,
            Context {
                focus: Some(
                    self.instance
                        .pointer
                        .callback_focus(self.instance.focused, &mut self.focus_request),
                ),
                tasks: self.tasks,
                state: self.state,
                engine: self.engine,
                owned: Some(&mut node.owned),
            },
        ))
    }

    fn hover_changed(&mut self) -> Result<()> {
        let router = &mut self.instance.pointer;
        let changes = router.hover.reconcile(
            router.frame.as_ref(),
            &self.instance.bodies,
            router.fixed_size,
        );
        router
            .tooltips
            .reconcile(&router.hover, self.tooltip_style, &self.instance.bodies);
        let mut result = Ok(());
        for (id, change) in changes {
            let (handlers, mut cx) = self.handlers(id).expect("live hover listener");
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

    fn press_target(
        &mut self,
        sample: PointerSample,
        button: PointerButton,
    ) -> Result<Option<NodeId>> {
        let Some(frame) = self.instance.pointer.frame.as_ref() else {
            return Ok(None);
        };
        let Some((leaf, recipients)) = frame.hit(sample.position) else {
            if button == PointerButton::Primary {
                self.instance.set_focus(None, false, self.state);
            }
            return Ok(None);
        };
        let Some(body) = self.instance.bodies.get(&leaf) else {
            return Ok(None);
        };
        if button == PointerButton::Primary && body.visible {
            // A live visible leaf retains its mounted ancestors, all of which are visible.
            if let Some(owner) = recipients.focus {
                frame.place_caret(owner, sample.position, self.state)?;
            }
            self.instance.set_focus(recipients.focus, false, self.state);
        }
        Ok(recipients.pointer)
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
        let target = match capture {
            Some(owner) => Some(owner),
            None => self.press_target(sample, button)?,
        };
        let Some(owner) = target else {
            return Ok(None);
        };
        let event = self.event(owner, sample, Some(button))?;
        let fixed_size = self.instance.pointer.fixed_size;
        let (handlers, context) = self.handlers(owner).expect("live recipient");
        let control = handlers.window_control(fixed_size);
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
            && let Some(control @ (WindowControl::Drag | WindowControl::Resize(_))) = control
        {
            return Ok(Some(control));
        }
        let eligible = button == PointerButton::Primary
            && capture.is_none()
            && !captured
            && (handlers.click.is_some() || control.is_some());
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
        let Some(mut owner) = self.instance.hit_recipient(sample.position, |r| r.scroll) else {
            return Ok(());
        };
        let vertical_input = delta.x == crate::px(0.0);
        loop {
            let frame = self
                .instance
                .pointer
                .frame
                .as_ref()
                .expect("accepted scroll hit");
            let parent = frame.scroll_parent(owner);
            if let Some(viewport) = frame.scroll_viewport(owner) {
                delta = self.instance.scroll_children(
                    owner,
                    viewport,
                    delta,
                    vertical_input,
                    self.state,
                )?;
            } else {
                let pointer = self.event(owner, sample, None)?;
                let (handlers, context) = self.handlers(owner).expect("live scroll ancestor");
                let scroll = handlers.scroll.as_ref().expect("accepted scroll listener");
                let mut cx = ScrollContext {
                    context,
                    received: delta,
                    remaining: ScrollDelta::default(),
                };
                scroll(&ScrollEvent { pointer, delta }, &mut cx)?;
                delta = cx.remaining;
            }
            if delta == ScrollDelta::default() {
                break;
            }
            let Some(parent) = parent else {
                break;
            };
            owner = parent;
        }
        Ok(())
    }

    fn moved(&mut self, sample: PointerSample) -> Result<()> {
        let capture = self.instance.pointer.capture(sample.id);
        let target =
            capture.or_else(|| self.instance.hit_recipient(sample.position, |r| r.pointer));
        if let Some(presses) = self.instance.pointer.presses.get_mut(&sample.id) {
            for press in presses {
                press.eligible &= target == Some(press.owner);
            }
        }
        let Some(target) = target else {
            return Ok(());
        };
        let event = self.event(target, sample, None)?;
        let (handlers, mut cx) = self.handlers(target).expect("live recipient");
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
        let Some(target) =
            capture.or_else(|| self.instance.hit_recipient(sample.position, |r| r.pointer))
        else {
            return Ok(None);
        };
        let event = self.event(target, sample, Some(button))?;
        let fixed_size = self.instance.pointer.fixed_size;
        let (handlers, mut cx) = self.handlers(target).expect("live recipient");
        if let Some(up) = &handlers.up {
            up(&event, &mut cx)?;
        }
        if press.eligible && target == press.owner {
            if let Some(click) = &handlers.click {
                click(&mut cx)?;
            }
            return Ok(handlers.window_control(fixed_size));
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
            let (handlers, mut cx) = self.handlers(press.owner).expect("live press owner");
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
    fn with_pointer_delivery<T>(
        &mut self,
        view: ViewId,
        deliver: impl FnOnce(&mut Delivery<'_>) -> Result<T>,
    ) -> Result<T> {
        let instance = self
            .instances
            .get_mut(&view)
            .context("pointer view is no longer mounted")?;
        let mut delivery = Delivery {
            focus_request: None,
            tooltip_style: &self.tooltip_style,
            instance,
            tasks: &self.tasks,
            state: self.state.get_mut(),
            engine: &self.engine,
        };
        let result = deliver(&mut delivery)?;
        delivery
            .instance
            .apply_focus_request(delivery.focus_request, delivery.state);
        Ok(result)
    }

    pub(crate) fn file_drop_target(
        &self,
        view: ViewId,
        position: crate::layout::Origin,
    ) -> Option<NodeId> {
        self.instances
            .get(&view)?
            .hit_recipient(position, |r| r.file_drop)
    }

    pub(crate) fn file_drop_hover(
        &mut self,
        view: ViewId,
        target: NodeId,
        hovered: bool,
    ) -> Result<()> {
        self.with_pointer_delivery(view, |delivery| {
            if let Some((handlers, mut cx)) = delivery.handlers(target)
                && let Some(handler) = &handlers.file_drop_hover
            {
                handler(hovered, &mut cx)?;
            }
            Ok(())
        })
    }

    pub(crate) fn drop_files(
        &mut self,
        view: ViewId,
        target: NodeId,
        paths: &[std::path::PathBuf],
    ) -> Result<bool> {
        self.with_pointer_delivery(view, |delivery| {
            if let Some((handlers, mut cx)) = delivery.handlers(target)
                && let Some(handler) = &handlers.file_drop
            {
                handler(paths, &mut cx)?;
                return Ok(true);
            }
            Ok(false)
        })
    }

    /// Enables or disables native resize and maximize actions for a mounted window.
    /// An explicit cursor style always applies, including on a fixed-size window.
    pub fn set_window_resizable(&mut self, view: ViewId, resizable: bool) -> Result<()> {
        self.with_pointer_delivery(view, |delivery| {
            let fixed = !resizable;
            if delivery.instance.pointer.fixed_size != fixed {
                delivery.instance.pointer.fixed_size = fixed;
                delivery.hover_changed()?;
            }
            Ok(())
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

    /// Earliest time an idle host should record this view for pending updates or pointer feedback.
    /// A host rechecks this after input and after accepting a frame.
    pub fn next_redraw(&self, view: ViewId) -> Option<web_time::Instant> {
        let instance = self.instances.get(&view)?;
        if self.has_pending_bindings()
            || !instance.repaints.borrow().dirty.is_empty()
            || instance
                .tracked_scroll
                .values()
                .any(super::TrackedScroll::has_request)
            || instance.pointer.scroll_pending
            || instance.lists.values().any(|list| list.dirty.get())
        {
            return Some(web_time::Instant::now());
        }
        let deadline = instance.pointer.tooltips.deadline();
        #[cfg(any(feature = "gif", feature = "webp"))]
        let deadline = instance
            .repaints
            .borrow()
            .image_deadlines
            .values()
            .copied()
            .chain(deadline)
            .min();
        deadline
    }

    /// Makes this submission's snapshot authoritative for input. Active sequences whose
    /// recipients disappear from the accepted frame are canceled after the switch.
    pub fn accept_pointer_frame(&mut self, frame: PointerFrame) -> Result<()> {
        self.with_pointer_delivery(frame.view, |delivery| {
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
            if frame.is_current(router) {
                router.scroll_pending = false;
            }
            router.frame = Some(frame);
            let result = delivery.cancel_ids(canceled);
            if changed {
                result.and(delivery.hover_changed())
            } else {
                result
            }
        })
    }

    /// Delivers host input against the view's accepted frame. Returns the native operation requested by this event.
    pub fn dispatch_pointer(
        &mut self,
        view: ViewId,
        input: PointerInput,
    ) -> Result<Option<WindowControl>> {
        self.with_pointer_delivery(view, |delivery| {
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
                PointerInput::Scroll(sample, delta) => {
                    delivery.scroll(sample, delta).map(|()| None)
                }
                PointerInput::Move(sample) => delivery.moved(sample).map(|()| None),
                PointerInput::Up(sample, button) => delivery.up(sample, button),
                PointerInput::Cancel(id) => delivery.cancel(id).map(|()| None),
                PointerInput::Leave(id) if delivery.instance.pointer.capture(id).is_none() => {
                    delivery.cancel(id).map(|()| None)
                }
                PointerInput::Leave(_) => Ok(None),
            };
            hover.and(routed)
        })
    }

    /// Cancels every active pointer on view deactivation, even if a callback reports an error.
    pub fn cancel_pointers(&mut self, view: ViewId) -> Result<()> {
        self.with_pointer_delivery(view, |delivery| {
            let ids = delivery.instance.pointer.presses.keys().copied().collect();
            delivery.instance.pointer.hover.clear();
            delivery.hover_changed().and(delivery.cancel_ids(ids))
        })
    }
}
