//! Vertical position shared by scroll containers, virtual lists and scrollbars.
use crate::{
    Context, Pixels, Signal,
    layout::{Origin, Size},
    state::Runtime,
};
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

/// Direction of travel along a scrollbar track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollAxis {
    /// Left to right.
    Horizontal,
    /// Top to bottom.
    Vertical,
}

impl ScrollAxis {
    pub(crate) fn select<T>(self, horizontal: T, vertical: T) -> T {
        match self {
            Self::Horizontal => horizontal,
            Self::Vertical => vertical,
        }
    }

    pub(crate) fn extent(self, size: Size) -> Pixels {
        self.select(size.width, size.height)
    }

    pub(crate) fn coordinate(self, point: Origin) -> Pixels {
        self.select(point.x, point.y)
    }
}

/// Scroll position and viewport extent along a scrollbar's axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollPosition {
    /// Distance from the start in logical pixels.
    pub offset_px: f64,
    /// Maximum distance from the start in logical pixels.
    pub max_offset_px: f64,
    /// Length of the visible content along this axis.
    pub viewport_extent: Pixels,
    /// Whether any part of the document extent is estimated.
    pub is_estimated: bool,
}

impl ScrollPosition {
    /// Percentage of the available travel. Content that fits returns zero.
    pub fn percent(self) -> f64 {
        scroll_percent(self.offset_px, self.max_offset_px)
    }
}

impl From<ListScrollPosition> for ScrollPosition {
    fn from(list: ListScrollPosition) -> Self {
        Self {
            offset_px: list.offset_px,
            max_offset_px: list.max_offset_px,
            viewport_extent: list.viewport_height,
            is_estimated: list.is_estimated,
        }
    }
}

/// Vertical position and extent of a laid-out scrollable viewport.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ListScrollPosition {
    /// Index of the first row intersecting the list viewport, or zero for an empty list.
    pub first_visible_index: usize,
    /// Logical pixels scrolled into the first visible row, or zero for an empty list.
    pub offset_in_row_px: f64,
    /// Number of rows intersecting the list viewport, including partially visible rows.
    pub visible_count: usize,
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
    /// Half-open range of visible row indices. Excludes rows mounted only for overscan.
    pub fn visible_range(self) -> std::ops::Range<usize> {
        self.first_visible_index..self.first_visible_index.saturating_add(self.visible_count)
    }

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

pub(crate) type PositionSignal = Signal<Option<ScrollPosition>>;

#[derive(Default)]
pub(crate) struct PositionObservers(RefCell<Vec<Weak<PositionSignal>>>);

impl PositionObservers {
    pub(crate) fn observe(
        &self,
        cx: &mut Context<'_>,
        position: Option<ScrollPosition>,
    ) -> Rc<PositionSignal> {
        let signal = Rc::new(cx.signal(position));
        let mut observers = self.0.borrow_mut();
        observers.retain(|observer| observer.strong_count() > 0);
        observers.push(Rc::downgrade(&signal));
        signal
    }

    pub(crate) fn publish(&self, state: &mut Runtime<()>, position: Option<ScrollPosition>) {
        self.0.borrow_mut().retain(|observer| {
            observer
                .upgrade()
                .is_some_and(|signal| state.publish_for_frame(*signal, position).is_ok())
        });
    }
}
