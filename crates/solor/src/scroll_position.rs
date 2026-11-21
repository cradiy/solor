//! Vertical position shared by scroll containers, virtual lists and scrollbars.
use crate::{Context, Signal, state::Runtime};
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

/// Vertical position and extent of a laid-out scrollable viewport.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ListScrollPosition {
    /// Distance from the top in logical pixels.
    pub offset_px: f64,
    /// Maximum scroll distance in logical pixels, excluding the viewport height.
    pub max_offset_px: f64,
    /// Height of the visible content area in logical pixels.
    pub viewport_height: crate::Pixels,
    /// Whether the document extent includes any unmeasured row heights.
    pub is_estimated: bool,
}

impl ListScrollPosition {
    /// Percentage of scrollable travel, from 0 to 100. Content that fits returns 0.
    pub fn percent(self) -> f64 {
        scroll_percent(self.offset_px, self.max_offset_px)
    }
}

pub(crate) fn scroll_percent(offset: f64, maximum: f64) -> f64 {
    if maximum > 0.0 {
        offset / maximum * 100.0
    } else {
        0.0
    }
}

pub(crate) type PositionSignal = Signal<Option<ListScrollPosition>>;

#[derive(Default)]
pub(crate) struct PositionObservers(RefCell<Vec<Weak<PositionSignal>>>);

impl PositionObservers {
    pub(crate) fn observe(
        &self,
        cx: &mut Context<'_>,
        position: Option<ListScrollPosition>,
    ) -> Rc<PositionSignal> {
        let signal = Rc::new(cx.signal(position));
        let mut observers = self.0.borrow_mut();
        observers.retain(|observer| observer.strong_count() > 0);
        observers.push(Rc::downgrade(&signal));
        signal
    }

    pub(crate) fn publish(&self, state: &mut Runtime<()>, position: Option<ListScrollPosition>) {
        self.0.borrow_mut().retain(|observer| {
            observer
                .upgrade()
                .is_some_and(|signal| state.publish_layout_value(*signal, position).is_ok())
        });
    }
}
