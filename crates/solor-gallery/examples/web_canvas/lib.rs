//! A browser canvas driven by Solor's mounted CPU window.
#![cfg(target_arch = "wasm32")]

use solor::{
    SharedString, Styled, Ui, component, div,
    layout::{Origin, Size},
    platform::{WindowEvent, WindowHandler},
    pointer::{Modifiers, PointerButton, PointerId, PointerInput, PointerKind, PointerSample},
    px,
    raster::RasterScale,
    rgb, text,
};
use solor_render_cpu::CpuWindow;
use wasm_bindgen::{Clamped, prelude::*};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, ImageData, PointerEvent};

fn js_error(error: anyhow::Error) -> JsError {
    JsError::new(&format!("{error:#}"))
}

/// The mounted view and its browser drawing surface.
#[wasm_bindgen]
pub struct CanvasView {
    window: CpuWindow,
    canvas: HtmlCanvasElement,
    context: CanvasRenderingContext2d,
}

#[wasm_bindgen]
impl CanvasView {
    /// Mounts a click counter on an existing canvas.
    #[wasm_bindgen(constructor)]
    pub fn new(canvas: HtmlCanvasElement) -> Result<Self, JsValue> {
        let ui = Ui::with_font(
            include_bytes!("../../../../fixtures/fonts/NotoSansCJKsc-fixture.otf").to_vec(),
        )
        .map_err(js_error)?;
        let root = component(|cx| {
            let count = cx.signal(0_u32);
            let label = cx.signal(SharedString::from("Clicks: 0"));
            div()
                .w_full()
                .h_full()
                .items_center()
                .justify_center()
                .bg(rgb(0xf8f7f2))
                .child(
                    div()
                        .rounded_lg()
                        .p(px(16.))
                        .bg(rgb(0x253c34))
                        .text_color(rgb(0xf4f6e4))
                        .child(text(label).text_size(px(22.)))
                        .on_click(move |cx| {
                            let next = cx.read(count)?.wrapping_add(1);
                            cx.set(count, next)?;
                            cx.set(label, format!("Clicks: {next}").into())
                        }),
                )
        });
        let context = canvas
            .get_context("2d")?
            .and_then(|c| c.dyn_into().ok())
            .ok_or_else(|| JsError::new("Canvas 2D context unavailable"))?;
        Ok(Self {
            window: CpuWindow::mount(ui, root).map_err(js_error)?,
            canvas,
            context,
        })
    }

    /// Submits the view at its CSS size and current device scale.
    pub fn draw(&mut self, width: f32, height: f32, scale: f64) -> Result<(), JsValue> {
        let frame = self
            .window
            .frame(
                Size {
                    width: px(width),
                    height: px(height),
                },
                RasterScale(scale),
            )
            .map_err(js_error)?;
        if self.canvas.width() != frame.size.width {
            self.canvas.set_width(frame.size.width);
        }
        if self.canvas.height() != frame.size.height {
            self.canvas.set_height(frame.size.height);
        }
        if let Some(area) = frame.damage.bounds() {
            let image = ImageData::new_with_u8_clamped_array_and_sh(
                Clamped(frame.rgba),
                frame.size.width,
                frame.size.height,
            )?;
            self.context
                .put_image_data_with_dirty_x_and_dirty_y_and_dirty_width_and_dirty_height(
                    &image,
                    0.,
                    0.,
                    area.left as f64,
                    area.top as f64,
                    (area.right - area.left) as f64,
                    (area.bottom - area.top) as f64,
                )?;
        }
        self.window.frame_submitted().map_err(js_error)?;
        Ok(())
    }

    /// Routes a browser pointer event and reports whether the view needs a frame.
    pub fn pointer(&mut self, event: PointerEvent) -> Result<bool, JsError> {
        let id = PointerId(event.pointer_id() as u64);
        let sample = PointerSample {
            id,
            kind: match event.pointer_type().as_str() {
                "touch" => PointerKind::Touch,
                "pen" => PointerKind::Pen,
                _ => PointerKind::Mouse,
            },
            position: Origin {
                x: px(event.offset_x() as f32),
                y: px(event.offset_y() as f32),
            },
            modifiers: Modifiers {
                shift: event.shift_key(),
                control: event.ctrl_key(),
                alt: event.alt_key(),
                meta: event.meta_key(),
            },
        };
        let input = match event.type_().as_str() {
            "pointerdown" if event.button() == 0 => {
                PointerInput::Down(sample, PointerButton::Primary)
            }
            "pointerup" if event.button() == 0 => PointerInput::Up(sample, PointerButton::Primary),
            "pointermove" => PointerInput::Move(sample),
            "pointercancel" => PointerInput::Cancel(id),
            "pointerleave" => PointerInput::Leave(id),
            _ => return Ok(false),
        };
        Ok(self
            .window
            .event(WindowEvent::Pointer(input))
            .map_err(js_error)?
            .redraw)
    }
}
