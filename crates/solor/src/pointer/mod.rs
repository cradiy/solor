//! View-local pointer input, capture and accepted-frame hit testing.
mod frame;
mod hover;
mod routing;
mod scroll;
pub(crate) use scroll::{ScrollAxes, TrackedScroll};
pub use scroll::{ScrollHandle, ScrollMetrics};
mod tooltip;
pub use tooltip::{Tooltip, TooltipAlignment, TooltipPlacement, TooltipStyle};

use crate::{
    Context, Result,
    layout::{Origin, Size},
    platform::SurfaceRect,
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
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

/// Physical mouse location in an element's accepted content box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HoverPosition {
    /// Logical coordinates relative to the content origin. Padding lies outside the content.
    pub position: Origin,
    /// Logical dimensions of the content box used by the paint callback.
    pub size: Size,
}

/// Scroll delivered to a listener on the hit branch of the accepted frame.
#[derive(Clone, Copy, Debug)]
pub struct ScrollEvent {
    /// Location and bounds in the listening element's accepted frame.
    pub pointer: PointerEvent,
    /// Logical distance available to this listener after descendants have handled their share.
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
    bounds: SurfaceRect,
    padding: crate::styled_body::ContentInsets,
}

impl PointerEvent {
    /// Identifies the pointer producing this event.
    pub fn pointer_id(&self) -> PointerId {
        self.sample.id
    }
    /// Returns the position relative to the recipient's border-box origin.
    pub fn local_position(&self) -> Origin {
        Origin {
            x: self.sample.position.x - self.bounds.origin.x,
            y: self.sample.position.y - self.bounds.origin.y,
        }
    }
    /// Returns the recipient's border box in view coordinates from the accepted frame.
    pub fn bounds(&self) -> SurfaceRect {
        self.bounds
    }
    /// Position relative to the content origin used by the recipient's paint callback.
    /// Points in the padding or outside the box during capture can lie outside the content.
    pub fn content_position(&self) -> Origin {
        let local = self.local_position();
        Origin {
            x: local.x - self.padding.origin.x,
            y: local.y - self.padding.origin.y,
        }
    }
    /// Content dimensions used for drawing in the accepted frame, excluding padding.
    pub fn content_size(&self) -> Size {
        self.padding.content_size(self.bounds.size)
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
/// State access and event routing during a scroll callback.
pub struct ScrollContext<'a> {
    context: Context<'a>,
    received: ScrollDelta,
    remaining: ScrollDelta,
}

impl ScrollContext<'_> {
    /// Forwards all the distance received by this listener to the next listening ancestor.
    pub fn propagate(&mut self) {
        self.remaining = self.received;
    }

    /// Forwards the unconsumed distance after this callback succeeds. Each signed component
    /// must be between zero and the corresponding received delta. A zero remainder ends delivery.
    /// The last successful propagation request in a callback determines what is forwarded.
    pub fn propagate_remaining(&mut self, remaining: ScrollDelta) -> Result<()> {
        let within = |received: crate::Pixels, part: crate::Pixels| {
            (received.get().min(0.0)..=received.get().max(0.0)).contains(&part.get())
        };
        anyhow::ensure!(
            within(self.received.x, remaining.x) && within(self.received.y, remaining.y),
            "scroll remainder outside the received delta"
        );
        self.remaining = remaining;
        Ok(())
    }
}

impl_event_context!(PointerDownContext);
impl_event_context!(ScrollContext);

type HoverHandler = Box<dyn Fn(bool, &mut Context<'_>) -> Result<()>>;

type HoverPositionHandler = Box<dyn Fn(Option<HoverPosition>, &mut Context<'_>) -> Result<()>>;

type DownHandler = Box<dyn Fn(&PointerEvent, &mut PointerDownContext<'_>) -> Result<()>>;
type PositionHandler = Box<dyn Fn(&PointerEvent, &mut Context<'_>) -> Result<()>>;
type FileDropHandler = Box<dyn Fn(&[std::path::PathBuf], &mut Context<'_>) -> Result<()>>;
type ScrollHandler = Box<dyn Fn(&ScrollEvent, &mut ScrollContext<'_>) -> Result<()>>;
type CancelHandler = Box<dyn Fn(PointerId, &mut Context<'_>) -> Result<()>>;

#[derive(Default)]
pub(crate) struct Handlers {
    pub(crate) focus_handle: Option<crate::FocusHandle>,
    pub(crate) text_input: Option<crate::Signal<crate::text::TextInputState>>,
    pub(crate) tab_stop: Option<crate::style::Property<bool>>,
    pub(crate) tab_index: Option<crate::style::Property<i32>>,
    pub(crate) tab_group: Option<crate::style::Property<i32>>,
    pub(crate) focus_indicator: Option<crate::element::FocusIndicator>,
    pub(crate) padding: crate::styled_body::ContentInsets,
    hover: Option<HoverHandler>,
    hover_position: Option<HoverPositionHandler>,
    pub(crate) tooltip: Option<Box<Tooltip>>,
    control: Option<crate::WindowControl>,
    down: Option<DownHandler>,
    moved: Option<PositionHandler>,
    scroll: Option<ScrollHandler>,
    file_drop: Option<FileDropHandler>,
    file_drop_hover: Option<HoverHandler>,
    up: Option<PositionHandler>,
    cancel: Option<CancelHandler>,
    pub(crate) key_context: Option<crate::SharedString>,
    pub(crate) actions: Vec<crate::keyboard::ActionHandler>,
    pub(crate) key_down: Option<crate::keyboard::KeyDownHandler>,
    pub(crate) key_up: Option<crate::keyboard::KeyHandler>,
    pub(crate) click: Option<crate::element::Click>,
}

impl Handlers {
    fn window_control(&self, fixed_size: bool) -> Option<crate::WindowControl> {
        use crate::WindowControl::{Resize, ToggleMaximize};
        self.control
            .filter(|control| !fixed_size || !matches!(control, Resize(_) | ToggleMaximize))
    }

    fn handles_pointer(&self) -> bool {
        self.down.is_some()
            || self.moved.is_some()
            || self.up.is_some()
            || self.cancel.is_some()
            || self.click.is_some()
            || self.control.is_some()
    }
}

macro_rules! event_handlers {
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
        /// Requests window closure on a completed click.
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

    /// Shows a text hint after a mouse dwells on this element or its descendants.
    /// The hint is drawn above the view and leaves pointer targeting unchanged.
    pub fn tooltip(mut self, tooltip: impl Into<Tooltip>) -> Self {
        let tooltip = tooltip.into();
        self.handlers.get_or_insert_with(Default::default).tooltip =
            (!tooltip.text.is_empty()).then(|| Box::new(tooltip));
        self
    }

    event_handlers! {
        /// Handles a key press while this element holds focus.
        on_key_down => key_down (&crate::keyboard::KeyEvent, &mut crate::keyboard::KeyDownContext<'_>);
        /// Handles a key release while this element holds focus.
        on_key_up => key_up (&crate::keyboard::KeyEvent, &mut Context<'_>);
        /// Notifies when the first mouse enters this subtree or the last mouse leaves it.
        on_hover => hover (bool, &mut Context<'_>);
        /// Observes the most recently sampled mouse over this subtree, independently of capture.
        /// Reports changes in content coordinates or dimensions after input or frame acceptance,
        /// and reports `None` when the last mouse leaves.
        on_hover_position => hover_position (Option<HoverPosition>, &mut Context<'_>);
        /// Observes when this element becomes or ceases to be the target of an external file drag.
        /// Attach alongside `on_file_drop` on the receiving element.
        on_file_drop_hover => file_drop_hover (bool, &mut Context<'_>);
        /// Receives local paths dropped on this element or its descendants.
        /// The nearest listening ancestor of the hit element receives the drop.
        on_file_drop => file_drop (&[std::path::PathBuf], &mut Context<'_>);
        /// Overrides wheel scrolling on this element. Call `cx.propagate()` to pass the delta to an ancestor.
        on_scroll => scroll (&ScrollEvent, &mut ScrollContext<'_>);
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
