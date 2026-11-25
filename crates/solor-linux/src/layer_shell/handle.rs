use super::{ExclusiveZone, KeyboardInteractivity, extent};
use crate::surface_control::{
    Binding, Control, InputRect, InputRegion, SurfaceRect, region_rectangles,
};
use solor::Pixels;
use std::io;

/// Thread-safe control of a single layer surface. Changes queued before opening are
/// checked against its anchors and protocol version when the surface is created.
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

    /// Requests logical dimensions. An automatic axis stretches between opposite anchors.
    pub fn set_size(&self, width: Option<Pixels>, height: Option<Pixels>) -> io::Result<()> {
        let size = (extent(width)?, extent(height)?);
        self.0.update_layer(|layer| layer.size = Some(size))
    }

    /// Changes how much desktop space the compositor should reserve.
    pub fn set_exclusive_zone(&self, zone: ExclusiveZone) -> io::Result<()> {
        let zone = zone.units()?;
        self.0.update_layer(|layer| layer.zone = Some(zone))
    }

    /// Changes the surface's keyboard focus policy. On-demand focus requires layer-shell v4.
    pub fn set_keyboard_interactivity(&self, keyboard: KeyboardInteractivity) -> io::Result<()> {
        self.0.update_layer(|layer| layer.keyboard = Some(keyboard))
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

    /// Requests compositor blur behind these panel-local rectangles. An empty slice clears it.
    /// The compositor determines the appearance and may leave the backdrop unchanged.
    pub fn set_background_blur_region(&self, rects: &[SurfaceRect]) -> io::Result<()> {
        let rects = region_rectangles(rects)?;
        self.0.update(|state| {
            state.blur = Some(rects);
            true
        })
    }

    /// Requests termination, including while hidden. Closing the surface is final.
    pub fn close(&self) -> io::Result<()> {
        self.0.close()
    }

    pub(crate) fn claim(&self) -> io::Result<Binding> {
        self.0.claim()
    }
}
