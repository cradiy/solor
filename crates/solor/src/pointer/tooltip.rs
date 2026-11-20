//! A passive text overlay driven by the accepted hover path.
use super::hover::Hover;
use super::{PointerFrame, PointerId, PointerInput, PointerKind};
use crate::{
    Draw, Instance, NodeId, Result, SharedString,
    layout::{Origin, Query, Size},
    px, rgb,
    style::Corners,
    text::{SharedEngine, Text, TextStyle},
};
use std::{
    cell::{Cell, OnceCell},
    collections::HashMap,
    time::{Duration, Instant},
};

#[derive(Default)]
pub(crate) struct Tooltips {
    mouse: Option<PointerId>,
    tip: Option<Tooltip>,
    erase: Cell<bool>,
}

struct Tooltip {
    owner: NodeId,
    deadline: Instant,
    label: OnceCell<Text>,
    painted: Cell<bool>,
}

impl Tooltips {
    pub(super) fn observe(&mut self, input: PointerInput, hover: &Hover) {
        match input {
            PointerInput::Move(p) if p.kind == PointerKind::Mouse && hover.unpressed(p.id) => {
                self.mouse = Some(p.id)
            }
            PointerInput::Cancel(id) | PointerInput::Leave(id) if self.mouse == Some(id) => {
                self.mouse = None
            }
            PointerInput::Down(..) | PointerInput::Scroll(..) => self.mouse = None,
            _ => {}
        }
    }

    pub(super) fn reconcile(
        &mut self,
        hover: &Hover,
        bodies: &HashMap<NodeId, crate::element::MountedNode>,
    ) {
        let owner = self.mouse.and_then(|id| {
            hover.path(id).iter().copied().find(|node| {
                bodies
                    .get(node)
                    .and_then(|body| body.handlers.as_deref())
                    .is_some_and(|h| h.tooltip.is_some())
            })
        });
        self.set_tip(owner);
    }

    pub(super) fn retire(&mut self, id: NodeId) {
        if self.tip.as_ref().is_some_and(|tip| tip.owner == id) {
            self.set_tip(None);
        }
    }

    fn set_tip(&mut self, owner: Option<NodeId>) {
        if self.tip.as_ref().map(|t| t.owner) == owner {
            return;
        }
        if self.tip.as_ref().is_some_and(|t| t.painted.get()) {
            self.erase.set(true);
        }
        self.tip = owner.map(|owner| Tooltip {
            owner,
            deadline: Instant::now() + Duration::from_millis(500),
            label: OnceCell::new(),
            painted: Cell::new(false),
        });
    }

    pub(super) fn deadline(&self) -> Option<Instant> {
        if self.erase.get() {
            return Some(Instant::now());
        }
        self.tip
            .as_ref()
            .filter(|t| !t.painted.get())
            .map(|t| t.deadline)
    }

    pub(crate) fn paint(
        &self,
        frame: &PointerFrame,
        instance: &Instance,
        engine: Option<&SharedEngine>,
        draws: &mut Vec<Draw>,
    ) -> Result<()> {
        self.erase.set(false);
        let Some(tip) = &self.tip else { return Ok(()) };
        if Instant::now() < tip.deadline {
            return Ok(());
        }
        let Some(mouse) = self
            .mouse
            .and_then(|id| instance.pointer.hover.position(id))
        else {
            return Ok(());
        };
        let target = frame.mouse_target(mouse, |n| instance.bodies.contains_key(&n));
        let owner = frame.hover_path(target.hover).find(|n| {
            instance.bodies[n]
                .handlers
                .as_deref()
                .is_some_and(|h| h.tooltip.is_some())
        });
        if owner != Some(tip.owner) {
            return Ok(());
        }
        let (at, size) = frame.bounds(tip.owner).expect("live tooltip hit box");
        let text = instance.bodies[&tip.owner]
            .handlers
            .as_ref()
            .and_then(|h| h.tooltip.as_ref())
            .expect("tooltip owner");
        let viewport = instance
            .scene
            .geometry(instance.scene.root())
            .expect("laid out view")
            .metrics
            .size;
        tip.draw(
            text,
            engine.expect("validated tooltip font"),
            at,
            size,
            viewport,
            draws,
        )?;
        tip.painted.set(true);
        Ok(())
    }
}

impl Tooltip {
    fn draw(
        &self,
        text: &SharedString,
        engine: &SharedEngine,
        at: Origin,
        owner: Size,
        viewport: Size,
        draws: &mut Vec<Draw>,
    ) -> Result<()> {
        let available = viewport.width.saturating_sub(px(16.0)).min(px(280.0));
        if available == px(0.0) || viewport.height <= px(10.0) {
            return Ok(());
        }
        let label = if let Some(label) = self.label.get() {
            label
        } else {
            let label = Text::new(
                engine.clone(),
                text.clone(),
                TextStyle {
                    size: px(13.0),
                    line_height: px(18.0),
                },
                true,
            )?;
            self.label.get_or_init(|| label)
        };
        let paragraph = label.resolve(Query::AtMost(available))?;
        let content = Size {
            width: paragraph.content.width.min(available),
            height: paragraph.content.height.min(viewport.height - px(10.0)),
        };
        let size = Size {
            width: content.width + px(16.0),
            height: content.height + px(10.0),
        };
        let below = at.y + owner.height + px(5.0);
        let y = if below + size.height <= viewport.height {
            below
        } else {
            at.y - px(5.0) - size.height
        };
        let origin = Origin {
            x: at
                .x
                .max(px(0.0))
                .min(viewport.width.saturating_sub(size.width)),
            y: y.max(px(0.0))
                .min(viewport.height.saturating_sub(size.height)),
        };
        draws.push(Draw::Fill {
            origin,
            size,
            corner_radii: Corners::all(px(5.0)).fit_to(size),
            clip: None,
            fill: rgb(0x303933).into(),
        });
        draws.push(Draw::Text {
            origin: Origin {
                x: origin.x + px(8.0),
                y: origin.y + px(5.0),
            },
            size: content,
            paragraph,
            clip: None,
            color: rgb(0xf8f7f2),
        });
        Ok(())
    }
}
