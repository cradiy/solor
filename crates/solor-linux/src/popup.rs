//! Client-sized Wayland popups with an application-supplied surface role.
use crate::surface_control::{Control, InputRegion};
use solor::{layout::Size, px};
use std::io;

pub use crate::surface_control::InputRect;
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
    pub fn show(&self, size: Size, input: Option<&[InputRect]>) -> io::Result<()> {
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

    /// Unmaps the popup while retaining its role and mounted UI.
    pub fn hide(&self) -> io::Result<()> {
        self.0.set_visible(false)
    }

    /// Closes the popup and wakes the panel thread.
    pub fn close(&self) -> io::Result<()> {
        self.0.close()
    }
}
