//! One-shot positioning for retained list viewports.
use crate::{
    Context, ListScrollPosition, Signal,
    scroll_position::{PositionObservers, PositionSignal},
    state::Runtime,
};
use anyhow::{Result, ensure};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

/// Placement of a requested row within the list's content viewport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScrollAlignment {
    /// Align the row's top edge with the viewport's top edge.
    Top,
    /// Center the row vertically, subject to the document bounds.
    Center,
    /// Align the row's bottom edge with the viewport's bottom edge.
    Bottom,
    /// Move the minimum distance to show the row, or fill the viewport with an oversized row.
    Nearest,
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Request {
    Row {
        index: usize,
        alignment: ScrollAlignment,
        initial_relative_offset: Option<f64>,
    },
    Coordinate {
        target: Coordinate,
        anchor: Option<(usize, f64)>,
    },
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Coordinate {
    Percent(f64),
    Offset(f64),
}

impl Request {
    fn offset(
        &mut self,
        current: f64,
        viewport: f64,
        count: usize,
        limit: f64,
        top: impl Fn(usize) -> f64,
    ) -> f64 {
        match self {
            Self::Coordinate { target, anchor } => {
                // Keep the selected reading position while distant estimates are replaced.
                if let Some((index, within)) = *anchor {
                    let index = index.min(count - 1);
                    let start = top(index);
                    return start + within.min(top(index + 1) - start);
                }
                match target {
                    Coordinate::Percent(percent) => (*percent / 100.0) * limit,
                    Coordinate::Offset(pixels) => *pixels,
                }
            }
            Self::Row {
                index,
                alignment,
                initial_relative_offset,
            } => {
                let index = (*index).min(count - 1);
                let start = top(index);
                let end = top(index + 1) - viewport;
                match alignment {
                    ScrollAlignment::Top => start,
                    ScrollAlignment::Center => start + (end - start) / 2.0,
                    ScrollAlignment::Bottom => end,
                    ScrollAlignment::Nearest => {
                        let relative = *initial_relative_offset.get_or_insert(current - start);
                        start + relative.clamp(0.0_f64.min(end - start), 0.0_f64.max(end - start))
                    }
                }
            }
        }
    }
}

#[derive(Default)]
struct State {
    pending: Cell<Option<Request>>,
    staged: Cell<Option<ListScrollPosition>>,
    published: Cell<Option<ListScrollPosition>>,
    position_signal: Option<Signal<Option<ListScrollPosition>>>,
    observers: PositionObservers,
    offset: Cell<Option<Signal<f64>>>,
    viewport: RefCell<Weak<Cell<bool>>>,
}

/// Positions one mounted virtual list. Clones address the same list.
/// A handle can be reused after its list unmounts.
#[derive(Clone, Default)]
pub struct ListScrollHandle(Rc<State>);

impl ListScrollHandle {
    /// Creates a handle that can receive a request before its list is mounted.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a handle and a signal for observing its last completed layout.
    /// The signal belongs to the creating context. It contains `None` before layout,
    /// when the viewport is hidden or zero-sized, and after the list unmounts.
    pub fn with_position_signal(
        cx: &mut Context<'_>,
    ) -> (Self, Signal<Option<ListScrollPosition>>) {
        let position = cx.signal(None);
        let handle = Self(Rc::new(State {
            position_signal: Some(position),
            ..State::default()
        }));
        (handle, position)
    }

    /// Requests alignment of a zero-based row in the next layout. The latest request wins.
    /// An empty or hidden list waits for a visible, nonempty viewport. An index beyond
    /// the current data targets the final row.
    pub fn scroll_to(&self, index: usize, alignment: ScrollAlignment) {
        self.enqueue(Request::Row {
            index,
            alignment,
            initial_relative_offset: None,
        });
    }

    /// Scrolls to a percentage of the available travel, with 100 at the bottom.
    /// Finite inputs are clamped to 0..=100.
    pub fn scroll_to_percent(&self, percent: f64) -> Result<()> {
        ensure!(percent.is_finite(), "scroll percentage must be finite");
        self.enqueue(Request::Coordinate {
            target: Coordinate::Percent(percent.clamp(0.0, 100.0)),
            anchor: None,
        });
        Ok(())
    }

    /// Scrolls to a logical-pixel distance from the top, clamped to the document bounds.
    /// Accepts a finite double-precision distance or a framework pixel length.
    pub fn scroll_to_offset_px(&self, pixels: impl Into<f64>) -> Result<()> {
        let pixels = pixels.into();
        ensure!(pixels.is_finite(), "scroll offset must be finite");
        self.enqueue(Request::Coordinate {
            target: Coordinate::Offset(pixels.max(0.0)),
            anchor: None,
        });
        Ok(())
    }

    /// Reads the last completed layout. Returns none before layout, for a hidden or
    /// zero-sized viewport, or after its list unmounts.
    pub fn position(&self) -> Option<ListScrollPosition> {
        self.0.viewport.borrow().upgrade()?;
        self.0.published.get()
    }

    pub(crate) fn observe(&self, cx: &mut Context<'_>) -> Rc<PositionSignal> {
        self.0
            .observers
            .observe(cx, self.position().map(Into::into))
    }

    pub(crate) fn scroll_by(&self, delta: f64, cx: &mut Context<'_>) -> Result<f64> {
        let (Some(position), Some(offset)) = (self.position(), self.0.offset.get()) else {
            return Ok(0.0);
        };
        apply_delta(cx, offset, position.max_offset_px, delta, Some(self))
    }

    fn enqueue(&self, request: Request) {
        self.0.pending.set(Some(request));
        if let Some(dirty) = self.0.viewport.borrow().upgrade() {
            dirty.set(true);
        }
    }

    pub(super) fn attach(
        &self,
        dirty: &Rc<Cell<bool>>,
        offset: Signal<f64>,
        state: &Runtime<()>,
    ) -> Result<()> {
        let mut viewport = self.0.viewport.borrow_mut();
        ensure!(
            viewport.upgrade().is_none(),
            "list scroll handle already attached"
        );
        if let Some(signal) = self.0.position_signal {
            state.read(signal)?;
        }
        // A deferred command may outlive a previous mount. Resolve it against this document.
        if let Some(mut request) = self.pending() {
            match &mut request {
                Request::Coordinate { anchor, .. } => *anchor = None,
                Request::Row {
                    initial_relative_offset,
                    ..
                } => *initial_relative_offset = None,
            }
            self.0.pending.set(Some(request));
        }
        *viewport = Rc::downgrade(dirty);
        self.0.offset.set(Some(offset));
        self.0.staged.set(None);
        self.0.published.set(None);
        Ok(())
    }

    pub(super) fn resolve(
        &self,
        current: f64,
        viewport: f64,
        count: usize,
        limit: f64,
        top: impl Fn(usize) -> f64,
    ) -> f64 {
        let Some(mut request) = self.pending() else {
            return current;
        };
        let offset = request.offset(current, viewport, count, limit, top);
        self.0.pending.set(Some(request));
        offset
    }

    pub(super) fn anchor_estimated_target(&self, index: usize, within: f64) {
        if let Some(Request::Coordinate {
            target,
            anchor: None,
        }) = self.pending()
        {
            self.0.pending.set(Some(Request::Coordinate {
                target,
                anchor: Some((index, within)),
            }));
        }
    }

    pub(super) fn stage(&self, position: Option<ListScrollPosition>) {
        self.0.staged.set(position);
    }

    pub(crate) fn publish(&self, state: &mut Runtime<()>) {
        self.publish_position(self.0.staged.get(), state);
    }

    pub(crate) fn detach(&self, state: &mut Runtime<()>) {
        self.0.offset.set(None);
        self.publish_position(None, state);
    }

    fn publish_position(&self, position: Option<ListScrollPosition>, state: &mut Runtime<()>) {
        if self.0.published.replace(position) == position {
            return;
        }
        if let Some(signal) = self.0.position_signal {
            // The scope owning the signal may have unmounted while the list was still live.
            let _ = state.publish_layout_value(signal, position);
        }
        self.0.observers.publish(state, position.map(Into::into));
    }

    pub(super) fn pending(&self) -> Option<Request> {
        self.0.pending.get()
    }

    pub(super) fn clear(&self) {
        self.0.pending.set(None);
    }
}

pub(super) fn apply_delta(
    cx: &mut Context<'_>,
    offset: Signal<f64>,
    limit: f64,
    delta: f64,
    handle: Option<&ListScrollHandle>,
) -> Result<f64> {
    let previous = cx.read(offset)?.min(limit);
    let used = delta.clamp(-previous, limit - previous);
    cx.set(offset, previous + used)?;
    if used != 0.0
        && let Some(handle) = handle
    {
        handle.clear();
    }
    Ok(used)
}
