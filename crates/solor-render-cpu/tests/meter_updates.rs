//! Mounted meter updates through the RGBA window handler.
use std::{hint::black_box, time::Instant};

use anyhow::Result;
use solor::{
    Context, Element, IntoElement, PaintContext, Signal, Styled, Ui, component, div, element,
    layout::{
        self, LayoutContext, LayoutNode, LayoutOutput, MeasureContext, Metrics, Origin, Query, Size,
    },
    platform::{WindowEvent, WindowHandler},
    pointer::{PointerId, PointerInput, PointerKind, PointerSample},
    px,
    raster::RasterScale,
    rgb, rgba, text,
};
use solor_render_cpu::{CpuWindow, pixels::Surface};

const SIZE: Size = Size {
    width: px(320.0),
    height: px(180.0),
};
const SCALE: RasterScale = RasterScale(1.25);
const FONT: &[u8] = include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf");

struct Meter(Signal<u16>);

impl LayoutNode for Meter {
    fn measure(&mut self, _: &mut MeasureContext<'_, '_>, query: Query) -> layout::Result<Metrics> {
        Ok(Metrics {
            size: Size {
                width: query.resolve(px(240.0)),
                height: px(8.0),
            },
            baseline: None,
        })
    }
    fn layout(
        &mut self,
        _: &mut LayoutContext<'_, '_>,
        size: Size,
    ) -> layout::Result<LayoutOutput> {
        Ok(LayoutOutput {
            content: size,
            baseline: None,
        })
    }
}
impl Element for Meter {
    fn paint(&self, cx: &mut PaintContext) -> Result<()> {
        cx.fill(
            Size {
                width: px(f32::from(*cx.read(self.0)?) + 0.25),
                ..cx.size
            },
            rgba(0x2168c4a0),
        );
        Ok(())
    }
}

fn panel(cx: &mut Context<'_>) -> impl IntoElement + use<> {
    let level = cx.signal(0u16);
    div()
        .flex_col()
        .p(px(8.25))
        .gap(px(4.0))
        .bg(rgb(0xf0f0f0))
        .child(
            element(Meter(level))
                .w(px(240.0))
                .h(px(8.0))
                .bg(rgb(0xbbbbbb))
                .repaint_on(level)
                .on_pointer_move(move |event, cx| {
                    cx.set(level, event.local_position().x.get() as u16)
                }),
        )
        .children(
            [
                "Output level",
                "Hello 世界！",
                "Audio settings",
                "System device",
            ]
            .map(text),
        )
}

fn movement(step: usize) -> PointerInput {
    let x = px(9.0 + ((step * 37) % 237) as f32);
    PointerInput::Move(PointerSample {
        id: PointerId(1),
        position: Origin { x, y: px(12.0) },
        kind: PointerKind::Mouse,
        modifiers: Default::default(),
    })
}

#[test]
fn meter_damage_covers_old_and_new_ink_and_matches_full_replay() -> Result<()> {
    let mut window = CpuWindow::mount(Ui::with_font(FONT.to_vec(), "en")?, component(panel))?;
    let mut ui = Ui::with_font(FONT.to_vec(), "en")?;
    let view = ui.mount(component(panel))?;
    let physical = SCALE.buffer_size(SIZE)?;
    let mut old_width = 0;
    for n in 0..18 {
        if n != 0 {
            let input = movement(n);
            assert!(window.event(WindowEvent::Pointer(input))?);
            ui.dispatch_pointer(view, input)?;
        }
        let frame = window.frame(SIZE, SCALE)?;
        ui.flush()?;
        ui.layout(view, SIZE)?;
        let reference = ui.record_frame(view)?;
        let full = Surface::draw_view(&reference, physical, SCALE)?;
        assert_eq!(frame.rgba, full.rgba);
        let new_width = (n * 37) % 237;
        let area = frame.damage.unwrap();
        if n != 0 {
            assert_eq!((area.left, area.top, area.bottom), (10, 10, 21));
            assert_eq!(
                area.right,
                ((8.5 + new_width.max(old_width) as f64) * 1.25).ceil() as i64
            );
        }
        old_width = new_width;
        ui.accept_pointer_frame(reference.into_pointer_frame())?;
        window.frame_submitted()?;
    }
    assert!(window.frame(SIZE, SCALE)?.damage.is_none());
    window.frame_submitted()?;
    let rescaled = RasterScale(1.2501);
    assert_eq!(rescaled.buffer_size(SIZE)?, physical);
    assert_eq!(
        window.frame(SIZE, rescaled)?.damage,
        Some(solor::raster::Clip::canvas(physical))
    );
    Ok(())
}

#[test]
#[ignore = "release timing workload"]
fn time_mounted_meter_frames() -> Result<()> {
    let mut window = CpuWindow::mount(Ui::with_font(FONT.to_vec(), "en")?, component(panel))?;
    window.frame(SIZE, SCALE)?;
    window.frame_submitted()?;
    let mut times = Vec::new();
    let mut total_area = 0;
    for n in 1..=220 {
        let start = Instant::now();
        window.event(WindowEvent::Pointer(movement(n)))?;
        let frame = window.frame(SIZE, SCALE)?;
        black_box(frame.rgba);
        let area = frame.damage.unwrap();
        window.frame_submitted()?;
        let elapsed = start.elapsed();
        if n > 20 {
            times.push(elapsed);
            total_area += (area.right - area.left) * (area.bottom - area.top);
        }
    }
    times.sort_unstable();
    println!(
        "median_ns={} p95_ns={} damaged_pixels_per_frame={:.1}",
        times[99].as_nanos(),
        times[189].as_nanos(),
        total_area as f64 / 200.0
    );
    Ok(())
}

#[test]
fn repair_replays_overlapping_text_and_rejected_pixels_require_full_damage() -> Result<()> {
    use solor::{Fill, raster::Clip};

    fn overlay(cx: &mut Context<'_>) -> impl IntoElement + use<> {
        let ink = cx.signal(Fill::Solid(rgb(0x445566)));
        div()
            .flex_col()
            .bg(rgb(0xffffff))
            .on_pointer_move(move |p, cx| {
                cx.set(
                    ink,
                    Fill::Solid(if p.local_position().x.get() > 60.0 {
                        rgb(0x556644)
                    } else {
                        rgb(0x445566)
                    }),
                )
            })
            .child(div().w(px(9.5)).h(px(18.0)).rounded(px(2.0)).bg(ink))
            .child(
                text("Output level")
                    .mt(px(-18.0))
                    .text_color(rgba(0x112233b0)),
            )
    }
    let mut window = CpuWindow::mount(Ui::with_font(FONT.to_vec(), "en")?, component(overlay))?;
    let mut ui = Ui::with_font(FONT.to_vec(), "en")?;
    let view = ui.mount(component(overlay))?;
    let physical = SCALE.buffer_size(SIZE)?;
    for (step, accept) in [(0, true), (2, false), (2, true), (1, true)] {
        let input = movement(step);
        window.event(WindowEvent::Pointer(input))?;
        ui.dispatch_pointer(view, input)?;
        let frame = window.frame(SIZE, SCALE)?;
        ui.flush()?;
        ui.layout(view, SIZE)?;
        let reference = ui.record_frame(view)?;
        let full = Surface::draw_view(&reference, physical, SCALE)?;
        assert_eq!(frame.rgba, full.rgba);
        assert_eq!(
            frame.damage,
            Some(if step == 0 || (step == 2 && accept) {
                Clip::canvas(physical)
            } else {
                Clip {
                    left: 0,
                    top: 0,
                    right: 12,
                    bottom: 23,
                }
            })
        );
        if accept {
            ui.accept_pointer_frame(reference.into_pointer_frame())?;
            window.frame_submitted()?;
        }
    }
    Ok(())
}
