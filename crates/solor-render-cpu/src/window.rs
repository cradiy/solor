//! Presentation of a mounted component on RGBA8 window targets.
use anyhow::Result;
use solor::{
    Draw, Render, Ui, ViewId,
    layout::Size,
    platform::{EventResponse, WindowBackground, WindowEvent, WindowHandler},
    pointer::PointerFrame,
    raster::{Clip, Damage, RasterFrame, RasterScale, Rgba8},
};

use crate::{
    pixels::Surface,
    view::{RepairIndex, ViewCaches},
};

/// A CPU-rendered window that owns its UI and a single mounted root.
pub struct CpuWindow {
    ui: Ui,
    view: ViewId,
    prepared: Option<(PointerFrame, bool)>,
    animate: bool,
    clear_byte: u8,
    retained: Option<RetainedView>,
    caches: ViewCaches,
}

struct RetainedView {
    logical: Size,
    scale: RasterScale,
    surface: Surface,
    draws: Vec<Draw>,
    index: RepairIndex,
}

impl CpuWindow {
    /// Mounts a component once. The platform supplies the size for its first layout.
    pub fn mount(ui: Ui, root: impl Render) -> Result<Self> {
        Self::mount_with_background(ui, root, WindowBackground::Opaque)
    }

    /// Mounts a component with an opaque white or transparent canvas for full and partial draws.
    pub fn mount_with_background(
        mut ui: Ui,
        root: impl Render,
        background: WindowBackground,
    ) -> Result<Self> {
        let view = ui.mount(root)?;
        Ok(Self {
            ui,
            view,
            prepared: None,
            animate: false,
            clear_byte: match background {
                WindowBackground::Opaque => 255,
                WindowBackground::Transparent => 0,
            },
            retained: None,
            caches: ViewCaches::default(),
        })
    }
}

impl WindowHandler<Rgba8> for CpuWindow {
    fn poll_tasks(&mut self, waker: &std::task::Waker) -> Result<()> {
        self.ui.poll_tasks(waker)
    }

    fn set_window_resizable(&mut self, resizable: bool) -> Result<()> {
        self.ui.set_window_resizable(self.view, resizable)
    }

    fn event(&mut self, event: WindowEvent) -> Result<EventResponse> {
        let mut response = EventResponse::default();
        match event {
            WindowEvent::Pointer(input) => {
                response.window_control = self.ui.dispatch_pointer(self.view, input)?
            }
            WindowEvent::Key(event) => {
                self.ui.dispatch_key(self.view, &event)?;
            }
            WindowEvent::FrameReady(time) if std::mem::take(&mut self.animate) => {
                self.ui.dispatch_frame(self.view, time)?;
                response.redraw = true;
            }
            _ => {}
        }
        response.redraw |= self
            .next_redraw()
            .is_some_and(|deadline| deadline <= std::time::Instant::now());
        Ok(response)
    }

    fn pointer_cursor(&self, pointer: solor::pointer::PointerId) -> solor::style::Cursor {
        self.ui.pointer_cursor(self.view, pointer)
    }

    fn next_redraw(&self) -> Option<std::time::Instant> {
        self.ui.next_redraw(self.view)
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
        let (surface, damage, index) = match self.retained.take() {
            Some(mut old) if old.logical == size && old.scale == scale => {
                let damage = old.index.damage(&old.draws, frame.draws(), scale, canvas)?;
                for &area in damage.rects() {
                    old.index.repair(
                        &mut old.surface,
                        frame.draws(),
                        scale,
                        area,
                        self.clear_byte,
                        &mut self.caches,
                    )?;
                }
                (old.surface, damage, old.index)
            }
            Some(mut old) => {
                old.surface.reset(physical, scale, self.clear_byte)?;
                old.surface
                    .replay(frame.draws(), scale, canvas, &mut self.caches)?;
                (
                    old.surface,
                    Damage::from_rect(canvas),
                    RepairIndex::default(),
                )
            }
            None => (
                Surface::raster_view(&frame, physical, scale, self.clear_byte, &mut self.caches)?,
                Damage::from_rect(canvas),
                RepairIndex::default(),
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
            index,
        });
        Ok(RasterFrame {
            size: physical,
            rgba: &retained.surface.rgba,
            request_callback: animate || !damage.is_empty(),
            damage,
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
