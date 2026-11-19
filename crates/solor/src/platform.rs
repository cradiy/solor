//! Window lifecycle and input shared by platform implementations.
use crate::{Pixels, layout::Size, pointer::PointerInput, raster::RasterScale};
use anyhow::Result;

/// Initial identity and preferred logical size of a window.
pub struct WindowOptions<'a> {
    /// Human-readable window title.
    pub title: &'a str,
    /// Application identifier used by the window system.
    pub app_id: &'a str,
    /// Preferred content size before the host supplies an allocation.
    pub size: Size,
}

/// Input or a frame opportunity delivered on the window's event thread.
pub enum WindowEvent {
    /// A positioned sample or the end of a pointer sequence.
    Pointer(PointerInput),
    /// Vertical scroll distance in logical pixels.
    Scroll(Pixels),
    /// The host is ready for a previously requested frame callback.
    FrameReady,
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

/// Application side of a single window. All calls are serialized on its event thread.
pub trait WindowHandler<T: Presentation> {
    /// Handles an event and reports whether the window needs a frame.
    fn event(&mut self, event: WindowEvent) -> Result<bool>;

    /// Prepares output for the current logical allocation and device scale.
    fn frame(&mut self, size: Size, scale: RasterScale) -> Result<T::Frame<'_>>;

    /// Acknowledges successful submission, including a frame with unchanged pixels.
    /// This is the point at which a view may accept its prepared pointer geometry.
    fn frame_submitted(&mut self) -> Result<()> {
        Ok(())
    }
}
