//! A passive text overlay driven by the accepted hover path.
mod style;
use super::hover::Hover;
use super::{PointerFrame, PointerId, PointerInput, PointerKind};
use crate::{
    Draw, Instance, NodeId, Pixels, Result, Ui,
    layout::{Origin, Query, Size},
    px,
    text::{SharedEngine, Text, TextSystem},
};
use std::{
    cell::{Cell, OnceCell},
    collections::HashMap,
};
pub use style::{Tooltip, TooltipAlignment, TooltipPlacement, TooltipStyle};
use web_time::Instant;

impl Ui {
    /// Sets the default tooltip style for existing and future views.
    /// Visible hints update on the next redraw. Local overrides take precedence.
    pub fn set_tooltip_style(&mut self, style: TooltipStyle) -> Result<()> {
        style.validate()?;
        for instance in self.instances.values_mut() {
            let tooltips = &mut instance.pointer.tooltips;
            if let Some(tip) = tooltips.tip.as_mut() {
                let declaration = instance.bodies[&tip.owner]
                    .handlers
                    .as_ref()
                    .and_then(|h| h.tooltip.as_ref())
                    .expect("live tooltip owner");
                let resolved = declaration.resolve(&style);
                if resolved != tip.style {
                    if resolved.typography != tip.style.typography {
                        tip.label.take();
                    }
                    tooltips
                        .erase
                        .set(tooltips.erase.get() || tip.painted.get());
                    tip.style = resolved;
                }
            }
        }
        self.tooltip_style = style;
        Ok(())
    }

    /// The UI's current defaults for text tooltips.
    pub fn tooltip_style(&self) -> &TooltipStyle {
        &self.tooltip_style
    }
}

#[derive(Default)]
pub(crate) struct Tooltips {
    mouse: Option<PointerId>,
    tip: Option<Box<ActiveTooltip>>,
    erase: Cell<bool>,
}

struct ActiveTooltip {
    owner: NodeId,
    entered: Instant,
    style: TooltipStyle,
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
        defaults: &TooltipStyle,
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
        if self.tip.as_ref().map(|tip| tip.owner) == owner {
            return;
        }
        self.clear();
        if let Some(owner) = owner {
            let declaration = bodies[&owner]
                .handlers
                .as_ref()
                .and_then(|h| h.tooltip.as_ref())
                .expect("live tooltip owner");
            self.tip = Some(Box::new(ActiveTooltip {
                owner,
                entered: Instant::now(),
                style: declaration.resolve(defaults),
                label: OnceCell::new(),
                painted: Cell::new(false),
            }));
        }
    }

    pub(super) fn retire(&mut self, id: NodeId) {
        if self.tip.as_ref().is_some_and(|tip| tip.owner == id) {
            self.clear();
        }
    }

    fn clear(&mut self) {
        if self.tip.take().is_some_and(|tip| tip.painted.get()) {
            self.erase.set(true);
        }
    }

    pub(super) fn deadline(&self) -> Option<Instant> {
        if self.erase.get() {
            return Some(Instant::now());
        }
        self.tip
            .as_ref()
            .filter(|t| !t.painted.get())
            .and_then(|t| t.entered.checked_add(t.style.delay))
    }

    pub(crate) fn paint(
        &self,
        frame: &PointerFrame,
        instance: &Instance,
        engine: &TextSystem,
        draws: &mut Vec<Draw>,
    ) -> Result<()> {
        self.erase.set(false);
        let Some(tip) = &self.tip else { return Ok(()) };
        if !tip.painted.get() && tip.entered.elapsed() < tip.style.delay {
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
        let bounds = frame.bounds(tip.owner).expect("live tooltip hit box");
        let tooltip = instance.bodies[&tip.owner]
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
            tooltip,
            engine.engine(),
            bounds.origin,
            bounds.size,
            viewport,
            draws,
        )?;
        tip.painted.set(true);
        Ok(())
    }
}

impl ActiveTooltip {
    fn draw(
        &self,
        tooltip: &Tooltip,
        engine: &SharedEngine,
        at: Origin,
        owner: Size,
        viewport: Size,
        draws: &mut Vec<Draw>,
    ) -> Result<()> {
        let style = &self.style;
        let padding = style.padding;
        let horizontal = padding.left + padding.right;
        let vertical = padding.top + padding.bottom;
        let available = viewport
            .width
            .min(style.max_width)
            .saturating_sub(horizontal);
        if available == px(0.0) || viewport.height <= vertical {
            return Ok(());
        }
        let label = if let Some(label) = self.label.get() {
            label
        } else {
            let label = Text::new(engine.clone(), tooltip.text.clone(), style.typography, true)?;
            self.label.get_or_init(|| label)
        };
        let paragraph = label.resolve(Query::AtMost(available))?;
        let content = Size {
            width: paragraph.content.width.min(available),
            height: paragraph.content.height.min(viewport.height - vertical),
        };
        let size = Size {
            width: content.width + horizontal,
            height: content.height + vertical,
        };
        let origin = style.origin(at, owner, size, viewport);
        draws.push(Draw::Fill {
            origin,
            size,
            corner_radii: style.corner_radii.fit_to(size),
            clip: None,
            fill: style.background.clone(),
        });
        draws.push(Draw::Text {
            origin: Origin {
                x: origin.x + padding.left,
                y: origin.y + padding.top,
            },
            size: content,
            paragraph,
            clip: None,
            color: style.text_color.into(),
        });
        Ok(())
    }
}

impl TooltipStyle {
    fn origin(&self, at: Origin, owner: Size, tip: Size, viewport: Size) -> Origin {
        let aligned = |start: Pixels, span: Pixels, extent: Pixels| match self.alignment {
            TooltipAlignment::Start => start,
            TooltipAlignment::Center => start + (span - extent) / 2.0,
            TooltipAlignment::End => start + (span - extent),
        };
        let side = |start: Pixels, span: Pixels, extent: Pixels, limit: Pixels, after: bool| {
            let before = start - self.gap - extent;
            let beyond = start + span + self.gap;
            if (after && beyond + extent <= limit) || (!after && before < px(0.0)) {
                beyond
            } else {
                before
            }
        };
        let (x, y) = match self.placement {
            TooltipPlacement::Above | TooltipPlacement::Below => (
                aligned(at.x, owner.width, tip.width),
                side(
                    at.y,
                    owner.height,
                    tip.height,
                    viewport.height,
                    self.placement == TooltipPlacement::Below,
                ),
            ),
            TooltipPlacement::Left | TooltipPlacement::Right => (
                side(
                    at.x,
                    owner.width,
                    tip.width,
                    viewport.width,
                    self.placement == TooltipPlacement::Right,
                ),
                aligned(at.y, owner.height, tip.height),
            ),
        };
        Origin {
            x: x.max(px(0.0)).min(viewport.width.saturating_sub(tip.width)),
            y: y.max(px(0.0))
                .min(viewport.height.saturating_sub(tip.height)),
        }
    }
}
