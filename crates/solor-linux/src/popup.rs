//! Wayland popup positioning and controls for externally assigned roles.
use crate::surface_control::{Control, InputRegion};
use solor::{layout::Size, px};
use std::io;

pub use solor::platform::SurfaceRect;
pub use wayland_client::{Connection, protocol::wl_surface::WlSurface};

/// Controls one popup from the panel or input-method thread.
#[derive(Clone, Debug)]
pub struct PopupHandle(pub(crate) Control);

impl PopupHandle {
    /// Creates an initially hidden popup handle.
    pub fn new() -> io::Result<Self> {
        Ok(Self(Control::new(false)?))
    }

    /// Shows or updates the popup at its measured logical size. The size rounds
    /// outward. Rectangles accept input in surface coordinates; `None` accepts
    /// the entire surface and an empty slice passes all input through.
    pub fn show(&self, size: Size, input: Option<&[SurfaceRect]>) -> io::Result<()> {
        let size = Size {
            width: px(size.width.get().ceil()),
            height: px(size.height.get().ceil()),
        };
        if size.width.get() < 1.0 || size.height.get() < 1.0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "popup size must be positive",
            ));
        }
        crate::wayland::buffer_size(120, size)?;
        let region = InputRegion::from_rects(input)?;
        self.0.update(|state| {
            state.size = Some(size);
            state.region = Some(region);
            state.visible = Some(true);
            true
        })
    }

    /// Requests compositor blur behind these popup-local rectangles. An empty slice clears it.
    pub fn set_background_blur_region(&self, rects: &[SurfaceRect]) -> io::Result<()> {
        self.0.set_blur(rects)
    }

    /// Unmaps the popup while retaining its role and mounted UI.
    pub fn hide(&self) -> io::Result<()> {
        self.0.set_visible(false)
    }

    /// Closes the popup and wakes the panel thread.
    pub fn close(&self) -> io::Result<()> {
        self.0.close()
    }
}

pub use wayland_protocols::xdg::shell::client::xdg_positioner::{
    Anchor, ConstraintAdjustment, Gravity,
};

/// Placement of an XDG popup relative to its parent's content.
#[derive(Clone, Debug)]
pub struct PopupOptions {
    /// Mapped toplevel, layer surface or popup to anchor to. Defaults to the triggering input's window.
    pub parent: Option<solor::platform::WindowHandle>,
    /// Requests a grab using the parent window's triggering pointer, touch or key press.
    pub grab: bool,
    /// Lets pointer and touch input pass through the entire popup surface.
    pub input_transparent: bool,
    /// Origin and extent of the anchor rectangle in parent-local logical pixels.
    pub anchor_rect: SurfaceRect,
    /// Preferred logical popup size.
    pub size: Size,
    /// Point on the anchor rectangle used for placement.
    pub anchor: Anchor,
    /// Direction in which the popup extends from its anchor.
    pub gravity: Gravity,
    /// Adjustments the compositor may make at output edges.
    pub constraints: ConstraintAdjustment,
    /// Composition of the popup background with the desktop.
    pub background: solor::platform::WindowBackground,
}

impl PopupOptions {
    /// Places a popup below the anchor rectangle, aligned with its left edge.
    pub fn new(anchor_rect: SurfaceRect, size: Size) -> Self {
        Self {
            parent: None,
            grab: true,
            input_transparent: false,
            anchor_rect,
            size,
            anchor: Anchor::BottomLeft,
            gravity: Gravity::BottomRight,
            constraints: ConstraintAdjustment::FlipY
                | ConstraintAdjustment::SlideX
                | ConstraintAdjustment::SlideY,
            background: solor::platform::WindowBackground::Opaque,
        }
    }

    /// Opens a context menu at the pointer, allowing it to flip at screen edges.
    pub fn at_pointer(
        event: &solor::pointer::PointerEvent,
        width: solor::Pixels,
        height: solor::Pixels,
    ) -> Self {
        let point = event.sample.position;
        let mut options = Self::new(
            SurfaceRect::new(point.x, point.y, px(1.), px(1.)),
            Size { width, height },
        );
        options.anchor = Anchor::TopLeft;
        options.constraints |= ConstraintAdjustment::FlipX;
        options
    }

    /// Anchors a popup to the element receiving a pointer event.
    pub fn below(
        event: &solor::pointer::PointerEvent,
        width: solor::Pixels,
        height: solor::Pixels,
    ) -> Self {
        Self::new(event.bounds(), Size { width, height })
    }

    /// Places a submenu to the right of its triggering item, with aligned top edges.
    pub fn beside(
        event: &solor::pointer::PointerEvent,
        width: solor::Pixels,
        height: solor::Pixels,
    ) -> Self {
        Self {
            anchor: Anchor::TopRight,
            gravity: Gravity::BottomRight,
            constraints: ConstraintAdjustment::FlipX
                | ConstraintAdjustment::SlideX
                | ConstraintAdjustment::SlideY,
            ..Self::below(event, width, height)
        }
    }
}
