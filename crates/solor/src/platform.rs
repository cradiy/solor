//! Window lifecycle and input shared by platform implementations.
use crate::{Pixels, layout::Size, pointer::PointerInput, px, raster::RasterScale};
use anyhow::Result;

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

/// Initial identity and preferred logical size of a window.
#[derive(Clone, Copy, Debug)]
#[must_use]
pub struct WindowOptions<'a> {
    /// Whether to request the native window frame and title bar. Defaults to true.
    pub native_decorations: bool,
    /// Human-readable window title.
    pub title: &'a str,
    /// Optional application identifier used by the window system to group windows.
    pub app_id: Option<&'a str>,
    /// Preferred content size before the host supplies an allocation.
    pub size: Size,
}

impl<'a> WindowOptions<'a> {
    /// Creates a titled window configuration with an 800 × 600 logical-pixel preference.
    pub const fn new(title: &'a str) -> Self {
        Self {
            native_decorations: true,
            title,
            app_id: None,
            size: Size {
                width: px(800.0),
                height: px(600.0),
            },
        }
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
    /// Handles an event and returns any requested redraw and native window operation.
    fn event(&mut self, event: WindowEvent) -> Result<EventResponse>;

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
