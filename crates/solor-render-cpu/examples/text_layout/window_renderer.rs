use anyhow::Result;
use solor::{
    layout::Size,
    platform::{EventResponse, WindowEvent, WindowHandler},
    pointer::{PointerButton, PointerInput, PointerSample},
    raster::{Clip, RasterFrame, RasterScale, Rgba8},
};
use solor_render_cpu::{damage::RetainedPixels, glyph_masks::GlyphMasks, pixels::Surface};

use crate::{
    input::{Point, PointerEvent},
    window_content::Content,
};

pub(super) struct Renderer {
    view: Content,
    masks: GlyphMasks,
    pixels: Option<(Size, RasterScale, RetainedPixels)>,
    audit: bool,
}

impl WindowHandler<Rgba8> for Renderer {
    fn event(&mut self, event: WindowEvent) -> Result<EventResponse> {
        let point = |p: PointerSample| Point {
            x: p.position.x.into(),
            y: p.position.y.into(),
        };
        let redraw = match event {
            WindowEvent::FrameReady(_) => self.view.frame_ready(),
            WindowEvent::Scroll(delta) => self.view.scroll_by(delta.into()),
            WindowEvent::Pointer(input) => {
                let event = match input {
                    PointerInput::Down(p, PointerButton::Primary) => {
                        PointerEvent::Down(p.id.0, point(p))
                    }
                    PointerInput::Move(p) => PointerEvent::Move(p.id.0, point(p)),
                    PointerInput::Up(p, PointerButton::Primary) => {
                        PointerEvent::Up(p.id.0, point(p))
                    }
                    PointerInput::Cancel(id) | PointerInput::Leave(id) => {
                        PointerEvent::Cancel(id.0)
                    }
                    _ => return Ok(EventResponse::default()),
                };
                self.view.pointer(event)
            }
        };
        Ok(EventResponse {
            redraw,
            ..Default::default()
        })
    }

    fn frame(&mut self, logical: Size, scale: RasterScale) -> Result<RasterFrame<'_>> {
        let frame = self.view.frame(logical)?;
        let size = scale.buffer_size(logical)?;
        let clip = Clip::canvas(size);
        let damage = match &mut self.pixels {
            Some((previous, density, retained)) if (*previous, *density) == (logical, scale) => {
                retained.update(frame.clone(), size, clip, &mut self.masks)?
            }
            _ => {
                let retained =
                    RetainedPixels::new_scaled(frame.clone(), size, clip, scale, &mut self.masks)?;
                self.pixels = Some((logical, scale, retained));
                Some(clip)
            }
        };
        let pixels = self.pixels.as_ref().expect("prepared pixels").2.surface();
        if self.audit {
            let oracle = Surface::draw_scaled(&frame, size, clip, scale, None)?;
            anyhow::ensure!(
                oracle.rgba == pixels.rgba,
                "retained pixels differ from full redraw"
            );
        }
        Ok(RasterFrame {
            size,
            rgba: &pixels.rgba,
            damage: damage
                .map(solor::raster::Damage::from_rect)
                .unwrap_or_default(),
            request_callback: self.view.wants_frame_callback(damage.is_some()),
        })
    }
}

impl Renderer {
    pub(super) fn new(view: Content, audit: bool) -> Self {
        Self {
            view,
            audit,
            pixels: None,
            masks: GlyphMasks::new(64, 32 * 1024),
        }
    }
}
