//! Window lifecycle and input shared by platform implementations.
use crate::{Pixels, layout::Size, pointer::PointerInput, px, raster::RasterScale};
use anyhow::Result;
use std::rc::Weak;

/// Host-side lifecycle of a window on its event thread.
pub trait PlatformWindow {
    /// Returns true while the window accepts input and frame requests, including during startup.
    fn is_open(&self) -> bool;
    /// Asks the host to activate this window if supported.
    fn activate(&self) {}
    /// Marks the window closed and schedules native teardown before the event loop sleeps.
    fn close(&self);
}

/// A cloneable UI-thread reference to one window. Handles compare equal when they
/// refer to the same window, including after it closes.
#[derive(Clone, Debug)]
pub struct WindowHandle(Weak<dyn PlatformWindow>);

impl WindowHandle {
    /// Creates a handle for a host-owned window. The host retains the target for its lifetime.
    pub fn new(window: Weak<dyn PlatformWindow>) -> Self {
        Self(window)
    }

    /// Returns true from successful creation until closure is requested or the host exits.
    pub fn is_open(&self) -> bool {
        self.0.upgrade().is_some_and(|window| window.is_open())
    }

    /// Asks the window system to bring this window to the user's attention.
    /// The window system decides whether to raise it or transfer keyboard focus.
    /// Closed windows and hosts without activation support ignore the request.
    pub fn activate(&self) {
        if let Some(window) = self.0.upgrade().filter(|window| window.is_open()) {
            window.activate();
        }
    }

    /// Requests closure of this window. All clones immediately observe it as closed.
    /// Repeated requests and requests after the window is gone have no effect.
    pub fn close(&self) {
        if let Some(window) = self.0.upgrade() {
            window.close();
        }
    }
}

impl PartialEq for WindowHandle {
    fn eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }
}

impl Eq for WindowHandle {}

/// Edge or corner moved during an interactive window resize.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeEdge {
    /// Top edge.
    Top,
    /// Upper right corner.
    TopRight,
    /// Right edge.
    Right,
    /// Lower right corner.
    BottomRight,
    /// Bottom edge.
    Bottom,
    /// Lower left corner.
    BottomLeft,
    /// Left edge.
    Left,
    /// Upper left corner.
    TopLeft,
}

/// Native window operation assigned to an element in a custom window frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowControl {
    /// Begin an interactive move on primary press.
    Drag,
    /// Begin resizing from an edge or corner on primary press.
    Resize(ResizeEdge),
    /// Minimize this window on click.
    Minimize,
    /// Switch between maximized and restored on click.
    ToggleMaximize,
    /// Toggle the preference for native decorations on click.
    ToggleDecorations,
    /// Close this window on click.
    Close,
}

/// How the window's pixels compose with the desktop.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowBackground {
    /// Draw onto an opaque canvas.
    #[default]
    Opaque,
    /// Preserve per-pixel alpha, including fully transparent unpainted areas.
    Transparent,
}

/// Initial identity and preferred logical size of a window.
#[derive(Clone, Copy, Debug)]
#[must_use]
pub struct WindowOptions<'a> {
    /// Whether to request stacking above other ordinary windows.
    pub always_on_top: bool,
    /// Composition of the window background with the desktop.
    pub background: WindowBackground,
    /// Whether to request the native window frame and title bar. Defaults to true.
    pub native_decorations: bool,
    /// Human-readable window title.
    pub title: &'a str,
    /// Optional application identifier used by the window system to group windows.
    pub app_id: Option<&'a str>,
    /// Whether the user can resize this window. Defaults to true.
    pub resizable: bool,
    /// Optional lower bound on the requested content size in logical pixels.
    pub min_size: Option<Size>,
    /// Optional upper bound on the requested content size in logical pixels.
    pub max_size: Option<Size>,
    /// Preferred content size before the host supplies an allocation.
    pub size: Size,
}

impl<'a> WindowOptions<'a> {
    /// Creates a titled window configuration with an 800 × 600 logical-pixel preference.
    pub const fn new(title: &'a str) -> Self {
        Self {
            always_on_top: false,
            native_decorations: true,
            background: WindowBackground::Opaque,
            title,
            app_id: None,
            resizable: true,
            min_size: None,
            max_size: None,
            size: Size {
                width: px(800.0),
                height: px(600.0),
            },
        }
    }

    /// Requests that this window stay above other ordinary windows.
    /// Window systems without this capability open the window at its usual level.
    pub const fn always_on_top(mut self) -> Self {
        self.always_on_top = true;
        self
    }

    /// Enables per-pixel transparency for the window content.
    pub const fn transparent(mut self) -> Self {
        self.background = WindowBackground::Transparent;
        self
    }

    /// Requests the native frame and title bar when enabled.
    pub const fn decorations(mut self, enabled: bool) -> Self {
        self.native_decorations = enabled;
        self
    }

    /// Requests a window without the native frame or title bar.
    pub const fn without_decorations(self) -> Self {
        self.decorations(false)
    }

    /// Associates the window with an application identifier.
    pub const fn app_id(mut self, id: &'a str) -> Self {
        self.app_id = Some(id);
        self
    }

    /// Allows user resizing when enabled. A fixed window requests its bounded initial size.
    pub const fn resizable(mut self, enabled: bool) -> Self {
        self.resizable = enabled;
        self
    }

    /// Sets the minimum content width and height in logical pixels.
    pub const fn min_size(mut self, width: Pixels, height: Pixels) -> Self {
        self.min_size = Some(Size { width, height });
        self
    }

    /// Sets the maximum content width and height in logical pixels.
    pub const fn max_size(mut self, width: Pixels, height: Pixels) -> Self {
        self.max_size = Some(Size { width, height });
        self
    }

    /// Sets the preferred content size. The window system supplies the actual allocation.
    pub const fn size(mut self, width: Pixels, height: Pixels) -> Self {
        self.size = Size { width, height };
        self
    }
}

/// Input or a frame opportunity delivered on the window's event thread.
pub enum WindowEvent {
    /// A positioned sample or the end of a pointer sequence.
    Pointer(PointerInput),
    /// A layout-resolved key press or release for this window.
    Key(crate::keyboard::KeyEvent),
    /// A requested frame opportunity, timestamped at dispatch from a monotonic host clock.
    /// Timestamps in one window share an origin and can be subtracted to measure cadence.
    FrameReady(std::time::Duration),
}

/// The format a renderer submits to a compatible window.
pub trait Presentation {
    /// One submission, which may borrow the renderer's output.
    type Frame<'a>;
}

/// Hosts a window and owns native events and presentation resources.
pub trait Platform {
    /// Presentation format accepted by this implementation.
    type Target: Presentation;

    /// Opens a window and runs or registers its event handler.
    fn run_window(
        &self,
        options: WindowOptions<'_>,
        handler: Box<dyn WindowHandler<Self::Target>>,
    ) -> Result<()>;
}

/// Work requested by a handler while processing one window event.
#[derive(Default)]
pub struct EventResponse {
    /// Schedule a frame for pending UI changes.
    pub redraw: bool,
    /// Apply this operation to the originating window during dispatch.
    pub window_control: Option<WindowControl>,
}

/// Application side of a single window. All calls are serialized on its event thread.
pub trait WindowHandler<T: Presentation> {
    /// Informs the view whether native resize controls are available before the first frame.
    fn set_window_resizable(&mut self, _resizable: bool) -> Result<()> {
        Ok(())
    }

    /// Handles an event and returns any requested redraw and native window operation.
    fn event(&mut self, event: WindowEvent) -> Result<EventResponse>;

    /// Cursor requested for this mouse after input delivery or frame submission.
    fn pointer_cursor(&self, _pointer: crate::pointer::PointerId) -> crate::style::Cursor {
        crate::style::Cursor::Default
    }

    /// Earliest monotonic deadline for an on-demand redraw. Recheck after input or submission.
    fn next_redraw(&self) -> Option<std::time::Instant> {
        None
    }

    /// Prepares output for the current logical allocation and device scale.
    fn frame(&mut self, size: Size, scale: RasterScale) -> Result<T::Frame<'_>>;

    /// Acknowledges successful submission, including a frame with unchanged pixels.
    /// This is the point at which a view may accept its prepared pointer geometry.
    fn frame_submitted(&mut self) -> Result<()> {
        Ok(())
    }
}
