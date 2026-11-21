use crate::surface_control::{Binding, Control, InputRect, InputRegion};
use std::io;

/// Thread-safe control of a single layer surface, including while its event loop is asleep.
#[derive(Clone, Debug)]
pub struct LayerSurfaceHandle(Control);

impl LayerSurfaceHandle {
    /// Creates a handle for one run and selects whether the surface should initially appear.
    pub fn new(visible: bool) -> io::Result<Self> {
        Ok(Self(Control::new(visible)?))
    }

    /// Requests the latest visibility. A hidden surface keeps its mounted component and signals.
    pub fn set_visible(&self, visible: bool) -> io::Result<()> {
        self.0.set_visible(visible)
    }

    /// Sets the surface-local pointer and touch input region. Use `None` for the whole surface
    /// or `Some(&[])` to pass all input through. Keyboard focus is controlled separately.
    /// Nonempty rectangles round outward to whole logical pixels. Invalid coordinates or
    /// extents return an error and preserve the previous request.
    pub fn set_input_region(&self, rects: Option<&[InputRect]>) -> io::Result<()> {
        let region = InputRegion::from_rects(rects)?;
        self.0.update(|state| {
            state.region = Some(region);
            true
        })
    }

    /// Requests termination, including while hidden. Closing the surface is final.
    pub fn close(&self) -> io::Result<()> {
        self.0.close()
    }

    pub(crate) fn claim(&self) -> io::Result<Binding<'_>> {
        self.0.claim()
    }
}
