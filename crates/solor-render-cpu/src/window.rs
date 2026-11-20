//! Presentation of a mounted component on RGBA8 window targets.
use anyhow::Result;
use solor::{
    Draw, Render, Ui, ViewId,
    layout::Size,
    platform::{EventResponse, WindowEvent, WindowHandler},
    pointer::PointerFrame,
    raster::{Clip, RasterFrame, RasterScale, Rgba8},
};

use crate::{glyph_masks::GlyphMasks, pixels::Surface};

/// A CPU-rendered window that owns its UI and a single mounted root.
pub struct CpuWindow {
    ui: Ui,
    view: ViewId,
    prepared: Option<(PointerFrame, bool)>,
    animate: bool,
    retained: Option<RetainedView>,
    // The mounted UI owns one immutable font face for the lifetime of this cache.
    glyph_masks: GlyphMasks,
}

struct RetainedView {
    logical: Size,
    scale: RasterScale,
    surface: Surface,
    draws: Vec<Draw>,
}

impl CpuWindow {
    /// Mounts a component once. The platform supplies the size for its first layout.
    pub fn mount(mut ui: Ui, root: impl Render) -> Result<Self> {
        let view = ui.mount(root)?;
        Ok(Self {
            ui,
            view,
            prepared: None,
            animate: false,
            retained: None,
            glyph_masks: GlyphMasks::new(64, 32 * 1024),
        })
    }
}

impl WindowHandler<Rgba8> for CpuWindow {
    fn event(&mut self, event: WindowEvent) -> Result<EventResponse> {
        let mut response = EventResponse::default();
        match event {
            WindowEvent::Pointer(input) => {
                response.window_control = self.ui.dispatch_pointer(self.view, input)?
            }
            WindowEvent::FrameReady => response.redraw = std::mem::take(&mut self.animate),
            _ => {}
        }
        response.redraw |= self.ui.has_pending_bindings();
        Ok(response)
    }

    fn frame(&mut self, size: Size, scale: RasterScale) -> Result<RasterFrame<'_>> {
        // An unacknowledged image cannot be the baseline for the platform's next damage.
        if self.prepared.take().is_some() {
            self.retained = None;
        }
        let physical = scale.buffer_size(size)?;
        self.ui.flush()?;
        self.ui.layout(self.view, size)?;
        let frame = self.ui.record_frame(self.view)?;
        let canvas = Clip::canvas(physical);
        // Take ownership before mutating pixels. On a raster error, retry from a clean image.
        let (surface, damage) = match self.retained.take() {
            Some(mut old) if old.logical == size && old.scale == scale => {
                let damage = crate::view::damage(&old.draws, frame.draws(), scale, canvas)?;
                if let Some(area) = damage {
                    old.surface
                        .repair(&frame, scale, area, &mut self.glyph_masks)?;
                }
                (old.surface, damage)
            }
            _ => (
                Surface::raster_view(&frame, physical, scale, Some(&mut self.glyph_masks))?,
                Some(canvas),
            ),
        };
        let animate = frame.requests_animation_frame();
        let (draws, pointer) = frame.into_parts();
        self.prepared = Some((pointer, animate));
        let retained = self.retained.insert(RetainedView {
            logical: size,
            scale,
            surface,
            draws,
        });
        Ok(RasterFrame {
            size: physical,
            rgba: &retained.surface.rgba,
            damage,
            request_callback: animate || damage.is_some(),
        })
    }

    fn frame_submitted(&mut self) -> Result<()> {
        if let Some((pointer, animate)) = self.prepared.take() {
            self.animate = animate;
            self.ui.accept_pointer_frame(pointer)?;
        }
        Ok(())
    }
}
