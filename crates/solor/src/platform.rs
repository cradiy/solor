//! Window lifecycle and input shared by platform implementations.
mod mounted;
pub use mounted::{UiWindow, ViewRenderer};

use crate::{
    Pixels,
    layout::{Origin, Size},
    pointer::PointerInput,
    px,
    raster::RasterScale,
};
use anyhow::Result;
use std::rc::{Rc, Weak};

/// A rectangle in surface-local logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceRect {
    /// Top-left corner relative to the surface.
    pub origin: Origin,
    /// Nonnegative width and height.
    pub size: Size,
}

impl SurfaceRect {
    /// Creates a rectangle with the given offset and extent in logical pixels.
    pub const fn new(x: Pixels, y: Pixels, width: Pixels, height: Pixels) -> Self {
        Self {
            origin: Origin { x, y },
            size: Size { width, height },
        }
    }

    pub(crate) fn contains(self, p: Origin) -> bool {
        let x = p.x - self.origin.x;
        let y = p.y - self.origin.y;
        x.is_extent() && y.is_extent() && x < self.size.width && y < self.size.height
    }
}

/// Presentation state confirmed by the host for a top-level window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowState {
    /// The window should display its active appearance.
    pub active: bool,
    /// The compositor has maximized the window.
    pub maximized: bool,
    /// The window occupies its output in fullscreen mode.
    pub fullscreen: bool,
}

/// A UI-thread observation that is cancelled when its owner drops it.
#[must_use = "keep the subscription or bind it to a component"]
#[derive(Default)]
pub struct WindowSubscription(Option<Box<dyn FnOnce()>>);

impl WindowSubscription {
    /// Creates a subscription with a host-provided cancellation callback.
    pub fn new(cancel: impl FnOnce() + 'static) -> Self {
        Self(Some(Box::new(cancel)))
    }

    /// Keeps the observation until the current element unmounts or the owning UI closes.
    pub fn bind_to(self, cx: &mut crate::Context<'_>) {
        if self.0.is_some() {
            cx.signal(self);
        }
    }
}

impl Drop for WindowSubscription {
    fn drop(&mut self) {
        if let Some(cancel) = self.0.take() {
            cancel();
        }
    }
}

/// Host-side lifecycle of a window on its event thread.
pub trait PlatformWindow {
    /// Returns true while the window accepts input and frame requests, including during startup.
    fn is_open(&self) -> bool;
    /// Asks the host to activate this window if supported.
    fn activate(&self) {}
    /// Requests compositor blur behind the given surface-local rectangles when supported.
    fn set_background_blur_region(&self, _rects: &[SurfaceRect]) -> std::io::Result<()> {
        Ok(())
    }
    /// Returns whether the window should draw client-side decorations.
    fn client_decorations(&self) -> bool {
        false
    }
    /// Registers a UI-thread listener invoked when negotiated client-side decorations change.
    fn observe_client_decorations(&self, _listener: Box<dyn FnMut(bool)>) -> WindowSubscription {
        WindowSubscription::default()
    }
    /// Returns the most recently confirmed top-level state, when available.
    fn state(&self) -> Option<WindowState> {
        None
    }
    /// Observes confirmed top-level state changes on the UI thread.
    fn observe_state(&self, _listener: Box<dyn FnMut(WindowState)>) -> WindowSubscription {
        WindowSubscription::default()
    }
    /// Sets the predicate for user-initiated top-level close requests.
    fn set_close_guard(&self, handler: Rc<dyn Fn() -> bool>);
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

    /// Returns true from successful creation until the window closes or the host exits.
    pub fn is_open(&self) -> bool {
        self.0.upgrade().is_some_and(|window| window.is_open())
    }

    /// Asks the window system to bring this window to the user's attention.
    /// If a modal dialog blocks this window, activation targets the dialog that accepts input.
    /// The window system decides whether to raise it or transfer keyboard focus.
    /// Closed windows and hosts without activation support ignore the request.
    pub fn activate(&self) {
        if let Some(window) = self.0.upgrade().filter(|window| window.is_open()) {
            window.activate();
        }
    }

    /// Requests compositor blur behind these window-local rectangles. An empty slice clears it.
    /// The effect is visible through transparent content on hosts that support background blur.
    pub fn set_background_blur_region(&self, rects: &[SurfaceRect]) -> std::io::Result<()> {
        let window = self
            .0
            .upgrade()
            .filter(|window| window.is_open())
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "window closed"))?;
        window.set_background_blur_region(rects)
    }

    /// Returns whether this window should render client-side decorations.
    ///
    /// Returns `false` when the compositor negotiates server-side decorations or
    /// requests undecorated content via `xdg-decoration`.
    pub fn client_decorations(&self) -> bool {
        self.0
            .upgrade()
            .is_some_and(|window| window.client_decorations())
    }

    /// Registers a UI-thread callback invoked when the compositor updates whether
    /// client-side decorations should be rendered.
    pub fn observe_client_decorations(
        &self,
        listener: impl FnMut(bool) + 'static,
    ) -> WindowSubscription {
        self.0
            .upgrade()
            .filter(|window| window.is_open())
            .map(|window| window.observe_client_decorations(Box::new(listener)))
            .unwrap_or_default()
    }

    /// Returns the state confirmed for this open top-level window.
    pub fn state(&self) -> Option<WindowState> {
        self.0
            .upgrade()
            .filter(|w| w.is_open())
            .and_then(|w| w.state())
    }

    /// Receives subsequent state changes on the UI thread.
    pub fn observe_state(&self, listener: impl FnMut(WindowState) + 'static) -> WindowSubscription {
        self.0
            .upgrade()
            .filter(|window| window.is_open())
            .map(|window| window.observe_state(Box::new(listener)))
            .unwrap_or_default()
    }

    /// Replaces the handler for native and title-bar close requests on this top-level window.
    /// Return true to close, or false to keep it open. Requests are accepted by default.
    pub fn set_close_guard(&self, handler: impl Fn() -> bool + 'static) {
        if let Some(window) = self.0.upgrade().filter(|window| window.is_open()) {
            window.set_close_guard(Rc::new(handler));
        }
    }

    /// Closes the window directly, including after application confirmation.
    /// All clones immediately observe it as closed.
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

impl std::hash::Hash for WindowHandle {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.as_ptr().cast::<()>().hash(state);
    }
}

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

/// Rendering backend requested by an application.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Renderer {
    /// Rasterize on the CPU.
    Cpu,
    /// Render on the GPU. Requires the platform's `gpu` feature.
    Gpu,
}

/// Allocation preference for the GPU device shared by an application's windows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GpuMemoryPolicy {
    /// Favor allocation performance.
    #[default]
    Performance,
    /// Favor a smaller memory footprint.
    MemoryUsage,
}

/// Options applied when opening a window.
#[derive(Clone, Copy, Debug)]
#[must_use]
pub struct WindowOptions<'a> {
    /// Overrides the platform's default renderer for this window.
    pub renderer: Option<Renderer>,
    /// Optional parent for an auxiliary window in the same application.
    pub parent: Option<&'a WindowHandle>,
    /// Blocks input to the parent while this window is open. Requires an open parent.
    pub modal: bool,
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
            renderer: None,
            parent: None,
            modal: false,
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

    /// Selects the renderer for this window.
    pub const fn renderer(mut self, renderer: Renderer) -> Self {
        self.renderer = Some(renderer);
        self
    }

    /// Associates this window with an open parent. Each window can be closed independently.
    pub const fn parent(mut self, parent: &'a WindowHandle) -> Self {
        self.parent = Some(parent);
        self.modal = false;
        self
    }

    /// Blocks input to the given parent until the dialog closes.
    /// Closing the parent also closes its modal dialogs.
    pub const fn modal_for(mut self, parent: &'a WindowHandle) -> Self {
        self.parent = Some(parent);
        self.modal = true;
        self
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
    /// The host confirmed a new top-level presentation state.
    StateChanged(WindowState),
    /// A positioned sample or the end of a pointer sequence.
    Pointer(PointerInput),
    /// A file-drop recipient gained its first hovering drag or lost its last one.
    FileDropHover {
        /// Recipient in the submitted frame.
        target: crate::NodeId,
        /// Whether an external file drag is over the target.
        hovered: bool,
    },
    /// Whether the window currently has native keyboard focus.
    KeyboardFocus(bool),
    /// A layout-resolved key press or release for this window.
    Key(crate::keyboard::KeyEvent),
    /// A text edit from the active platform input method.
    TextInput(crate::text::TextInputEvent),
    /// A requested frame opportunity with a monotonic animation timestamp.
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
    /// Finds a file-drop recipient at a logical position in the submitted frame.
    fn file_drop_target(&self, _position: crate::layout::Origin) -> Option<crate::NodeId> {
        None
    }

    /// Delivers local paths to the recipient captured at release. Returns whether it handled the drop.
    fn drop_files(
        &mut self,
        _target: crate::NodeId,
        _paths: &[std::path::PathBuf],
    ) -> Result<bool> {
        Ok(false)
    }

    /// Identifies the current text-input focus within this window. The token changes
    /// when a field regains focus so queued edits can be matched to their session.
    fn text_input_session(&self) -> Option<u64> {
        None
    }

    /// Returns the focused insertion point in logical window coordinates.
    fn caret_bounds(&self) -> Option<crate::text::CaretBounds> {
        None
    }

    /// Queries the focused editor for committed context within the host's byte budget.
    fn surrounding_text(&mut self, _max_bytes: usize) -> Option<crate::text::SurroundingText<'_>> {
        None
    }

    /// Informs the view whether native resize controls are available before the first frame.
    fn set_window_resizable(&mut self, _resizable: bool) -> Result<()> {
        Ok(())
    }

    /// Supplies the system clipboard for this window.
    fn set_clipboard(&mut self, _clipboard: std::rc::Rc<dyn crate::clipboard::PlatformClipboard>) {}

    /// Polls foreground work before checking redraw deadlines. The waker interrupts
    /// the event loop when a pending task becomes ready.
    fn poll_tasks(&mut self, _waker: &std::task::Waker) -> Result<()> {
        Ok(())
    }

    /// Handles an event and returns any requested redraw and native window operation.
    fn event(&mut self, event: WindowEvent) -> Result<EventResponse>;

    /// Cursor requested for this mouse after input delivery or frame submission.
    fn pointer_cursor(&self, _pointer: crate::pointer::PointerId) -> crate::style::Cursor {
        crate::style::Cursor::Default
    }

    /// Earliest monotonic deadline for an on-demand redraw. Recheck after input or submission.
    fn next_redraw(&self) -> Option<web_time::Instant> {
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
