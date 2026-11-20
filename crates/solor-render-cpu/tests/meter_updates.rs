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
    panel_with_offscreen_labels(cx, 0)
}

fn panel_with_offscreen_labels(cx: &mut Context<'_>, extra: usize) -> impl IntoElement + use<> {
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
        .children(
            (0..extra).map(|i| text("Output level").when(i == 0, |label| label.mt(SIZE.height))),
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
    time_window(0)
}

fn scaled_window(extra: usize) -> Result<CpuWindow> {
    CpuWindow::mount(
        Ui::with_font(FONT.to_vec(), "en")?,
        component(move |cx| panel_with_offscreen_labels(cx, extra)),
    )
}

fn time_window(extra: usize) -> Result<()> {
    let mut window = scaled_window(extra)?;
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
        "labels={} median_ns={} p95_ns={} damaged_pixels_per_frame={:.1}",
        4 + extra,
        times[99].as_nanos(),
        times[189].as_nanos(),
        total_area as f64 / 200.0
    );
    Ok(())
}

#[test]
fn offscreen_labels_preserve_meter_pixels_and_damage() -> Result<()> {
    let mut baseline = scaled_window(0)?;
    let mut others = [
        scaled_window(60)?,
        scaled_window(252)?,
        scaled_window(1020)?,
    ];
    for n in 0..=220 {
        if n > 0 {
            baseline.event(WindowEvent::Pointer(movement(n)))?;
        }
        let expected = baseline.frame(SIZE, SCALE)?;
        for window in &mut others {
            if n > 0 {
                window.event(WindowEvent::Pointer(movement(n)))?;
            }
            let frame = window.frame(SIZE, SCALE)?;
            assert!(frame.rgba == expected.rgba, "different pixels on frame {n}");
            assert_eq!(frame.damage, expected.damage);
            window.frame_submitted()?;
        }
        baseline.frame_submitted()?;
    }
    Ok(())
}

#[test]
#[ignore = "release timing workload"]
fn time_static_label_scaling() -> Result<()> {
    for extra in [0, 60, 252, 1020, 1020, 252, 60, 0] {
        time_window(extra)?;
        let mut ui = Ui::with_font(FONT.to_vec(), "en")?;
        let view = ui.mount(component(move |cx| panel_with_offscreen_labels(cx, extra)))?;
        ui.layout(view, SIZE)?;
        let first = ui.record_frame(view)?;
        assert_eq!(
            first
                .draws()
                .iter()
                .filter(|d| matches!(d, solor::Draw::Text { .. }))
                .count(),
            4 + extra
        );
        ui.accept_pointer_frame(first.into_pointer_frame())?;
        let mut update = Vec::with_capacity(200);
        let mut record = Vec::with_capacity(200);
        for n in 1..=220 {
            let start = Instant::now();
            ui.dispatch_pointer(view, movement(n))?;
            ui.flush()?;
            let changed = ui.layout(view, SIZE)?;
            let flushed = Instant::now();
            let frame = ui.record_frame(view)?;
            black_box(frame.draws());
            let recorded = Instant::now();
            ui.accept_pointer_frame(frame.into_pointer_frame())?;
            assert_eq!(changed, 0);
            if n > 20 {
                update.push(flushed - start);
                record.push(recorded - flushed);
            }
        }
        // Time snapshot preparation separately so probing it cannot warm the recording pass.
        let mut snapshot = Vec::with_capacity(200);
        for n in 1..=220 {
            let start = Instant::now();
            let pointer = black_box(ui.prepare_pointer_frame(view)?);
            let elapsed = start.elapsed();
            drop(pointer);
            if n > 20 {
                snapshot.push(elapsed);
            }
        }
        snapshot.sort_unstable();
        update.sort_unstable();
        record.sort_unstable();
        println!(
            "labels={} input_flush_layout_ns={} record_ns={} snapshot_probe_ns={}",
            4 + extra,
            update[99].as_nanos(),
            record[99].as_nanos(),
            snapshot[99].as_nanos()
        );
    }
    Ok(())
}

#[test]
fn overlapping_text_matches_full_replay_across_repairs_retries_and_scale_changes() -> Result<()> {
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
    let scaled = RasterScale(1.5);
    for (step, scale, accept, full_damage) in [
        (0, SCALE, true, true),
        (2, SCALE, false, false),
        (2, SCALE, true, true),
        (1, SCALE, true, false),
        (1, scaled, true, true),
        (2, scaled, true, false),
        (2, SCALE, true, true),
    ] {
        let physical = scale.buffer_size(SIZE)?;
        let input = movement(step);
        window.event(WindowEvent::Pointer(input))?;
        ui.dispatch_pointer(view, input)?;
        let frame = window.frame(SIZE, scale)?;
        ui.flush()?;
        ui.layout(view, SIZE)?;
        let reference = ui.record_frame(view)?;
        let full = Surface::draw_view(&reference, physical, scale)?;
        assert_eq!(frame.rgba, full.rgba);
        assert_eq!(
            frame.damage,
            Some(if full_damage {
                Clip::canvas(physical)
            } else {
                Clip {
                    left: 0,
                    top: 0,
                    right: (9.5 * scale.0).ceil() as i64,
                    bottom: (18.0 * scale.0).ceil() as i64,
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

#[test]
fn toggling_a_fill_repairs_only_its_clip_under_unchanged_labels() -> Result<()> {
    use solor::raster::Clip;

    fn panel(cx: &mut Context<'_>) -> impl IntoElement + use<> {
        let shown = cx.signal(true);
        div()
            .bg(rgb(0xf4f4f4))
            .on_pointer_move(move |event, cx| cx.set(shown, event.local_position().x > px(60.0)))
            .child(
                div().w(px(24.0)).h(px(16.0)).overflow_hidden().child(
                    div()
                        .min_w(px(40.0))
                        .h(px(16.0))
                        .bg(rgb(0x2255aa))
                        .visible_when(shown),
                ),
            )
            .child(
                text("Output level")
                    .mt(px(-16.0))
                    .text_color(rgba(0x33001199)),
            )
            .child(text("System device").mt(px(40.0)))
    }

    let mut window = CpuWindow::mount(Ui::with_font(FONT.to_vec(), "en")?, component(panel))?;
    let mut ui = Ui::with_font(FONT.to_vec(), "en")?;
    let view = ui.mount(component(panel))?;
    let physical = SCALE.buffer_size(SIZE)?;
    let meter = Clip {
        left: 0,
        top: 0,
        right: 30,
        bottom: 20,
    };
    for step in [None, Some(1), Some(2), Some(1)] {
        if let Some(n) = step {
            let input = movement(n);
            window.event(WindowEvent::Pointer(input))?;
            ui.dispatch_pointer(view, input)?;
        }
        let frame = window.frame(SIZE, SCALE)?;
        ui.flush()?;
        ui.layout(view, SIZE)?;
        let reference = ui.record_frame(view)?;
        let full = Surface::draw_view(&reference, physical, SCALE)?;
        assert_eq!(frame.rgba, full.rgba);
        assert_eq!(
            frame.damage,
            Some(step.map_or(Clip::canvas(physical), |_| meter))
        );
        ui.accept_pointer_frame(reference.into_pointer_frame())?;
        window.frame_submitted()?;
    }
    Ok(())
}

#[test]
#[ignore = "release timing workload"]
fn time_resizing_meter() -> Result<()> {
    let mut ui = Ui::with_font(FONT.to_vec(), "en")?;
    let view = ui.mount(component(|cx| panel_with_offscreen_labels(cx, 1020)))?;
    let mut recordings = Vec::with_capacity(200);
    let mut updates = Vec::with_capacity(200);
    for n in 0..220 {
        let size = Size {
            width: SIZE.width + px((n % 2) as f32),
            ..SIZE
        };
        let start = Instant::now();
        ui.layout(view, size)?;
        let laid_out = Instant::now();
        let frame = ui.record_frame(view)?;
        black_box(frame.draws());
        let recorded = Instant::now();
        ui.accept_pointer_frame(frame.into_pointer_frame())?;
        if n >= 20 {
            recordings.push(recorded - laid_out);
            updates.push(start.elapsed());
        }
    }
    recordings.sort_unstable();
    updates.sort_unstable();
    println!(
        "labels=1024 resize_record_ns={} resize_update_ns={}",
        recordings[99].as_nanos(),
        updates[99].as_nanos()
    );
    Ok(())
}
