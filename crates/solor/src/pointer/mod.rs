//! View-local pointer input, capture and accepted-frame hit testing.
mod frame;
mod routing;

use std::ops::{Deref, DerefMut};

use crate::{
    Context, Result,
    layout::{Origin, Size},
};
pub use frame::PointerFrame;
pub(crate) use routing::PointerRouter;

/// Identity supplied by the host, unique among simultaneous pointers in a view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PointerId(pub u64);

/// Input device that produced a positioned sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerKind {
    /// Mouse or trackpad cursor.
    Mouse,
    /// Individual touch contact.
    Touch,
    /// Stylus.
    Pen,
}

/// Button that changed in a down or up event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerButton {
    /// Primary mouse action, touch contact or pen tip.
    Primary,
    /// Secondary mouse or pen action.
    Secondary,
    /// Middle mouse button.
    Middle,
    /// Additional button identified by the host.
    Other(u16),
}

/// Keyboard modifiers held when the sample was produced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    /// Shift is held.
    pub shift: bool,
    /// Control is held.
    pub control: bool,
    /// Alt or Option is held.
    pub alt: bool,
    /// Super or Command is held.
    pub meta: bool,
}

/// A pointer sample in view-local logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerSample {
    /// Pointer within the receiving view.
    pub id: PointerId,
    /// Source device type.
    pub kind: PointerKind,
    /// Position relative to the view origin.
    pub position: Origin,
    /// Modifiers at sample time.
    pub modifiers: Modifiers,
}

/// Scroll distance in logical pixels. Positive values move right and down.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollDelta {
    /// Horizontal distance.
    pub x: crate::Pixels,
    /// Vertical distance.
    pub y: crate::Pixels,
}

/// Scroll delivered to the nearest listening element under the pointer.
#[derive(Clone, Copy, Debug)]
pub struct ScrollEvent {
    /// Location and bounds in the listening element's accepted frame.
    pub pointer: PointerEvent,
    /// Logical distance requested by the input device.
    pub delta: ScrollDelta,
}

/// Normalized input sent by a platform adapter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PointerInput {
    /// A button was pressed.
    Down(PointerSample, PointerButton),
    /// A wheel or trackpad scrolled at this position.
    Scroll(PointerSample, ScrollDelta),
    /// The pointer moved.
    Move(PointerSample),
    /// A button was released.
    Up(PointerSample, PointerButton),
    /// The pointer left the view; an existing capture can continue.
    Leave(PointerId),
    /// The host lost the sequence, including any capture.
    Cancel(PointerId),
}

/// Positioned input delivered in the recipient's accepted border box.
#[derive(Clone, Copy, Debug)]
pub struct PointerEvent {
    /// Original view-local sample.
    pub sample: PointerSample,
    /// Changed button on press or release.
    pub button: Option<PointerButton>,
    position: Origin,
    size: Size,
}

impl PointerEvent {
    /// Identifies the pointer producing this event.
    pub fn pointer_id(&self) -> PointerId {
        self.sample.id
    }
    /// Returns the position relative to the recipient's border-box origin.
    pub fn local_position(&self) -> Origin {
        self.position
    }
    /// Returns the recipient's accepted border-box dimensions.
    pub fn bounds(&self) -> Size {
        self.size
    }
    /// Whether the changed button represents the primary action.
    pub fn is_primary_action(&self) -> bool {
        self.button == Some(PointerButton::Primary)
    }
}

/// State access and capture requests during a pointer-down callback.
pub struct PointerDownContext<'a> {
    context: Context<'a>,
    capture: bool,
}

impl PointerDownContext<'_> {
    /// Captures this pointer for the recipient if the callback completes successfully.
    pub fn capture_pointer(&mut self) {
        self.capture = true;
    }
}
impl<'a> Deref for PointerDownContext<'a> {
    type Target = Context<'a>;
    fn deref(&self) -> &Self::Target {
        &self.context
    }
}
impl DerefMut for PointerDownContext<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.context
    }
}

type DownHandler = Box<dyn Fn(&PointerEvent, &mut PointerDownContext<'_>) -> Result<()>>;
type PositionHandler = Box<dyn Fn(&PointerEvent, &mut Context<'_>) -> Result<()>>;
type ScrollHandler = Box<dyn Fn(&ScrollEvent, &mut Context<'_>) -> Result<()>>;
type CancelHandler = Box<dyn Fn(PointerId, &mut Context<'_>) -> Result<()>>;

#[derive(Default)]
pub(crate) struct Handlers {
    control: Option<crate::WindowControl>,
    down: Option<DownHandler>,
    moved: Option<PositionHandler>,
    scroll: Option<ScrollHandler>,
    up: Option<PositionHandler>,
    cancel: Option<CancelHandler>,
    pub(crate) click: Option<crate::element::Click>,
}

impl Handlers {
    fn handles_pointer(&self) -> bool {
        self.down.is_some()
            || self.moved.is_some()
            || self.up.is_some()
            || self.cancel.is_some()
            || self.click.is_some()
            || self.control.is_some()
    }
}

macro_rules! pointer_handlers {
    ($($(#[$doc:meta])* $name:ident => $slot:ident ($event:ty, $cx:ty);)+) => {$(
        $(#[$doc])*
        pub fn $name(mut self, handler: impl Fn($event, $cx) -> Result<()> + 'static) -> Self {
            self.handlers.get_or_insert_with(Default::default).$slot = Some(Box::new(handler));
            self
        }
    )+};
}
macro_rules! window_controls {
    ($($(#[$doc:meta])* $method:ident => $control:ident;)+) => {$(
        $(#[$doc])*
        pub fn $method(self) -> Self {
            self.window_control(crate::WindowControl::$control)
        }
    )+};
}

impl<E: crate::Element> crate::Node<E> {
    window_controls! {
        /// Starts a native window drag on primary press.
        window_drag_region => Drag;
        /// Closes the window on a completed click.
        window_close_button => Close;
        /// Minimizes the window on a completed click.
        window_minimize_button => Minimize;
        /// Switches this window's native decorations on or off on a completed click.
        window_decorations_toggle => ToggleDecorations;
        /// Toggles maximization on a completed click.
        window_maximize_button => ToggleMaximize;
    }

    /// Starts an interactive window resize from the given edge or corner on primary press.
    pub fn window_resize_region(self, edge: crate::ResizeEdge) -> Self {
        self.window_control(crate::WindowControl::Resize(edge))
    }

    /// Assigns a native window operation to this element. Interactive children receive their own input.
    pub fn window_control(mut self, control: crate::WindowControl) -> Self {
        self.handlers.get_or_insert_with(Default::default).control = Some(control);
        self
    }

    pointer_handlers! {
        /// Handles positioned scroll input over this element and its descendants without a closer scroll listener.
        on_scroll => scroll (&ScrollEvent, &mut Context<'_>);
        /// Handles a button press on this element. The callback can capture the pointer.
        on_pointer_down => down (&PointerEvent, &mut PointerDownContext<'_>);
        /// Handles pointer motion over this element or during its capture.
        on_pointer_move => moved (&PointerEvent, &mut Context<'_>);
        /// Handles a tracked button release at the current target or capture owner.
        on_pointer_up => up (&PointerEvent, &mut Context<'_>);
        /// Handles the loss of a live press or capture once for this pointer.
        on_pointer_cancel => cancel (PointerId, &mut Context<'_>);
    }
}
