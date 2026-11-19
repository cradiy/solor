//! Presentation of a mounted component on RGBA8 window targets.
use anyhow::Result;
use solor::{
    Render, Ui, ViewId,
    layout::Size,
    platform::{WindowEvent, WindowHandler},
    pointer::PointerFrame,
    raster::{Clip, RasterFrame, RasterScale, Rgba8},
};

use crate::pixels::Surface;

/// A CPU-rendered window that owns its UI and a single mounted root.
pub struct CpuWindow {
    ui: Ui,
    view: ViewId,
    prepared: Option<(PointerFrame, Surface)>,
}

impl CpuWindow {
    /// Mounts a component once. The platform supplies the size for its first layout.
    pub fn mount(mut ui: Ui, root: impl Render) -> Result<Self> {
        let view = ui.mount(root)?;
        Ok(Self {
            ui,
            view,
            prepared: None,
        })
    }
}

impl WindowHandler<Rgba8> for CpuWindow {
    fn event(&mut self, event: WindowEvent) -> Result<bool> {
        if let WindowEvent::Pointer(input) = event {
            self.ui.dispatch_pointer(self.view, input)?;
        }
        Ok(self.ui.has_pending_bindings())
    }

    fn frame(&mut self, size: Size, scale: RasterScale) -> Result<RasterFrame<'_>> {
        self.prepared = None;
        let physical = scale.buffer_size(size)?;
        self.ui.flush()?;
        self.ui.layout(self.view, size)?;
        let frame = self.ui.record_frame(self.view)?;
        let surface = Surface::draw_view(&frame, physical, scale)?;
        let (_, surface) = self.prepared.insert((frame.into_pointer_frame(), surface));
        Ok(RasterFrame {
            size: physical,
            rgba: &surface.rgba,
            damage: Some(Clip::canvas(physical)),
            request_callback: true,
        })
    }

    fn frame_submitted(&mut self) -> Result<()> {
        if let Some((pointer, _)) = self.prepared.take() {
            self.ui.accept_pointer_frame(pointer)?;
        }
        Ok(())
    }
}
