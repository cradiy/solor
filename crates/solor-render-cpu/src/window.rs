use crate::{
    pixels::Surface,
    view::{RepairIndex, ViewCaches},
};
use anyhow::Result;
use solor::{
    ViewFrame,
    layout::Size,
    platform::{UiWindow, ViewRenderer, WindowBackground},
    raster::{Clip, Damage, RasterFrame, RasterScale, Rgba8},
};

/// A mounted window rendered into retained RGBA8 pixels.
pub type CpuWindow = UiWindow<CpuRenderer>;

/// Retained CPU pixels, damage index and resource caches for one window.
pub struct CpuRenderer {
    clear_byte: u8,
    retained: Option<RetainedView>,
    caches: ViewCaches,
}
struct RetainedView {
    logical: Size,
    scale: RasterScale,
    surface: Surface,
    frame: ViewFrame,
    index: RepairIndex,
}

impl From<WindowBackground> for CpuRenderer {
    fn from(background: WindowBackground) -> Self {
        Self {
            clear_byte: if background == WindowBackground::Opaque {
                255
            } else {
                0
            },
            retained: None,
            caches: Default::default(),
        }
    }
}
impl ViewRenderer for CpuRenderer {
    type Target = Rgba8;
    fn render(
        &mut self,
        frame: ViewFrame,
        size: Size,
        scale: RasterScale,
        reset: bool,
    ) -> Result<RasterFrame<'_>> {
        if reset {
            self.retained = None;
        }
        let physical = scale.buffer_size(size)?;
        let canvas = Clip::canvas(physical);
        // Take ownership before mutating pixels. On a raster error, retry from a clean image.
        let (surface, damage, index) = match self.retained.take() {
            Some(mut old) if old.logical == size && old.scale == scale => {
                let damage = old.index.damage(&old.frame, &frame, scale, canvas)?;
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
                    RepairIndex::new(frame.draws(), scale, canvas)?,
                )
            }
            None => (
                Surface::raster_view(&frame, physical, scale, self.clear_byte, &mut self.caches)?,
                Damage::from_rect(canvas),
                RepairIndex::new(frame.draws(), scale, canvas)?,
            ),
        };
        let animate = frame.requests_animation_frame();
        let retained = self.retained.insert(RetainedView {
            logical: size,
            scale,
            surface,
            frame,
            index,
        });
        Ok(RasterFrame {
            size: physical,
            rgba: &retained.surface.rgba,
            request_callback: animate || !damage.is_empty(),
            damage,
        })
    }
}
