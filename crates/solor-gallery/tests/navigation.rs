//! Pointer navigation through the same mounted window used by the native launcher.
use anyhow::Result;
use solor::{
    WindowControl,
    layout::{Origin, Size},
    platform::{WindowEvent, WindowHandler},
    pointer::{PointerButton, PointerId, PointerInput, PointerKind, PointerSample},
    px,
    raster::RasterScale,
};
use solor_render_cpu::CpuWindow;
use std::time::Duration;

fn gallery() -> Result<(solor::Ui, impl solor::Render)> {
    let font = include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf");
    solor_gallery::gallery(solor::Ui::with_font(font.to_vec())?)
}

fn click(window: &mut CpuWindow, x: f32, y: f32) -> Result<()> {
    let point = PointerSample {
        id: PointerId(1),
        kind: PointerKind::Mouse,
        position: Origin { x: px(x), y: px(y) },
        modifiers: Default::default(),
    };
    for input in [
        PointerInput::Down(point, PointerButton::Primary),
        PointerInput::Up(point, PointerButton::Primary),
    ] {
        assert_eq!(
            window.event(WindowEvent::Pointer(input))?.window_control,
            None
        );
    }
    Ok(())
}

#[test]
fn controls_change_the_specimen_and_choices_survive_navigation() -> Result<()> {
    let size = Size {
        width: px(800.0),
        height: px(560.0),
    };
    let specimen = |window: &mut CpuWindow| -> Result<Vec<u8>> {
        let frame = window.frame(size, RasterScale::ONE)?;
        let stride = frame.size.width as usize * 4;
        let pixels = frame.rgba[200 * stride..350 * stride].to_vec();
        window.frame_submitted()?;
        assert!(
            !window
                .event(WindowEvent::FrameReady(Duration::ZERO))?
                .redraw
        );
        Ok(pixels)
    };
    let (ui, gallery) = gallery()?;
    let mut window = CpuWindow::mount(ui, gallery)?;
    let normal = specimen(&mut window)?;
    click(&mut window, 269.0, 154.0)?;
    let wide = specimen(&mut window)?;
    assert!(normal != wide, "letter spacing must change the specimen");
    click(&mut window, 627.0, 29.0)?;
    let linear = specimen(&mut window)?;
    assert!(linear != wide, "Paint must replace the typography specimen");
    click(&mut window, 167.0, 154.0)?;
    let radial = specimen(&mut window)?;
    assert!(radial != linear, "radial and linear gradients must differ");
    click(&mut window, 683.0, 29.0)?;
    let center = specimen(&mut window)?;
    assert!(center != radial, "Layout must replace the paint specimen");
    click(&mut window, 215.0, 154.0)?;
    assert!(
        center != specimen(&mut window)?,
        "justify must reposition the items"
    );
    click(&mut window, 548.0, 29.0)?;
    assert!(
        wide == specimen(&mut window)?,
        "returning should preserve the selected tracking"
    );
    Ok(())
}

#[test]
fn submitted_header_routes_drag_and_close_separately() -> Result<()> {
    let (ui, gallery) = gallery()?;
    let mut window = CpuWindow::mount(ui, gallery)?;
    let point = |x| PointerSample {
        id: PointerId(1),
        kind: PointerKind::Mouse,
        position: Origin {
            x: px(x),
            y: px(29.0),
        },
        modifiers: Default::default(),
    };
    let drag = PointerInput::Down(point(250.0), PointerButton::Primary);
    window.frame(
        Size {
            width: px(800.0),
            height: px(560.0),
        },
        RasterScale::ONE,
    )?;
    assert_eq!(
        window.event(WindowEvent::Pointer(drag))?.window_control,
        None
    );
    window.frame_submitted()?;
    assert_eq!(
        window.event(WindowEvent::Pointer(drag))?.window_control,
        Some(WindowControl::Drag)
    );

    let toggle = point(150.0);
    let hover = window.event(WindowEvent::Pointer(PointerInput::Move(toggle)))?;
    assert!(hover.redraw);
    window.frame(
        Size {
            width: px(800.0),
            height: px(560.0),
        },
        RasterScale::ONE,
    )?;
    window.frame_submitted()?;
    for _ in 0..2 {
        let down = window.event(WindowEvent::Pointer(PointerInput::Down(
            toggle,
            PointerButton::Primary,
        )))?;
        assert_eq!(down.window_control, None);
        let up = window.event(WindowEvent::Pointer(PointerInput::Up(
            toggle,
            PointerButton::Primary,
        )))?;
        assert_eq!(up.window_control, Some(WindowControl::ToggleDecorations));
        assert!(!up.redraw);
    }

    let close = point(755.0);
    let down = PointerInput::Down(close, PointerButton::Primary);
    let up = PointerInput::Up(close, PointerButton::Primary);
    for input in [down, PointerInput::Move(point(250.0)), up, down] {
        assert_eq!(
            window.event(WindowEvent::Pointer(input))?.window_control,
            None
        );
    }
    assert_eq!(
        window.event(WindowEvent::Pointer(up))?.window_control,
        Some(WindowControl::Close)
    );
    Ok(())
}

fn market_window(scale: RasterScale) -> Result<(CpuWindow, Size)> {
    let size = Size {
        width: px(800.0),
        height: px(560.0),
    };
    let (ui, root) = gallery()?;
    let mut window = CpuWindow::mount(ui, root)?;
    window.frame(size, scale)?;
    window.frame_submitted()?;
    click(&mut window, 461.0, 29.0)?;
    window.frame(size, scale)?;
    window.frame_submitted()?;
    Ok((window, size))
}

#[test]
fn market_advances_freezes_and_retires_its_frame_listener() -> Result<()> {
    let scale = RasterScale::ONE;
    let (mut window, size) = market_window(scale)?;
    let sample = |window: &mut CpuWindow| -> Result<Vec<u8>> {
        let frame = window.frame(size, scale)?;
        let stride = frame.size.width as usize * 4;
        let pixels = frame.rgba[280 * stride..500 * stride].to_vec();
        window.frame_submitted()?;
        Ok(pixels)
    };
    assert!(
        window
            .event(WindowEvent::FrameReady(Duration::ZERO))?
            .redraw
    );
    let previous = sample(&mut window)?;
    assert!(
        window
            .event(WindowEvent::FrameReady(Duration::ZERO))?
            .redraw
    );
    let latest = sample(&mut window)?;
    assert!(previous != latest);
    click(&mut window, 750.0, 85.0)?;
    // The preceding submission may still have a callback in flight at pause.
    window.event(WindowEvent::FrameReady(Duration::from_secs(2)))?;
    let paused = sample(&mut window)?;
    assert!(paused == latest);
    assert!(
        !window
            .event(WindowEvent::FrameReady(Duration::ZERO))?
            .redraw
    );
    assert!(paused == sample(&mut window)?);
    let scroll = |window: &mut CpuWindow, x, dy| -> Result<bool> {
        let sample = PointerSample {
            id: PointerId(1),
            kind: PointerKind::Mouse,
            position: Origin {
                x: px(x),
                y: px(330.0),
            },
            modifiers: Default::default(),
        };
        let delta = solor::pointer::ScrollDelta {
            x: px(0.0),
            y: px(dy),
        };
        Ok(window
            .event(WindowEvent::Pointer(PointerInput::Scroll(sample, delta)))?
            .redraw)
    };
    assert!(!scroll(&mut window, 90.0, -12.0)?);
    let move_to = |window: &mut CpuWindow, x| -> Result<bool> {
        let mouse = PointerSample {
            id: PointerId(1),
            kind: PointerKind::Mouse,
            position: Origin {
                x: px(x),
                y: px(330.0),
            },
            modifiers: Default::default(),
        };
        Ok(window
            .event(WindowEvent::Pointer(PointerInput::Move(mouse)))?
            .redraw)
    };
    assert!(move_to(&mut window, 480.0)?);
    let guide = sample(&mut window)?;
    // Only the horizontal latest-price guide changes on the paused plot.
    let stride = 800 * 4;
    let changed_rows = guide
        .chunks_exact(stride)
        .zip(paused.chunks_exact(stride))
        .filter(|(new, old)| new != old)
        .count();
    assert!((1..=2).contains(&changed_rows));
    assert!(!move_to(&mut window, 490.0)?);
    assert!(scroll(&mut window, 480.0, -200.0)?);
    assert!(guide != sample(&mut window)?);
    assert!(!scroll(&mut window, 480.0, -1.0)?);
    assert!(
        !window
            .event(WindowEvent::FrameReady(Duration::from_secs(3)))?
            .redraw
    );
    assert!(scroll(&mut window, 480.0, 200.0)?);
    assert!(guide == sample(&mut window)?);
    assert!(move_to(&mut window, 90.0)?);
    assert!(paused == sample(&mut window)?);
    click(&mut window, 92.0, 233.0)?;
    assert!(paused != sample(&mut window)?);
    assert!(
        !window
            .event(WindowEvent::FrameReady(Duration::ZERO))?
            .redraw
    );
    click(&mut window, 750.0, 85.0)?;
    sample(&mut window)?;
    assert!(
        window
            .event(WindowEvent::FrameReady(Duration::ZERO))?
            .redraw
    );
    click(&mut window, 550.0, 29.0)?;
    sample(&mut window)?;
    assert!(
        !window
            .event(WindowEvent::FrameReady(Duration::ZERO))?
            .redraw
    );
    Ok(())
}

#[test]
#[ignore = "release throughput measurement"]
fn market_frame_cost() -> Result<()> {
    use std::time::Instant;
    for scale in [RasterScale::ONE, RasterScale(1.5)] {
        let (mut window, size) = market_window(scale)?;
        let mut costs = Vec::with_capacity(360);
        let clock = Instant::now();
        for frame in 0..392 {
            let start = Instant::now();
            assert!(
                window
                    .event(WindowEvent::FrameReady(start.duration_since(clock)))?
                    .redraw
            );
            assert!(!window.frame(size, scale)?.damage.is_empty());
            window.frame_submitted()?;
            if frame >= 32 {
                costs.push(start.elapsed());
            }
        }
        let frames_per_second = costs.len() as f64 / costs.iter().sum::<Duration>().as_secs_f64();
        costs.sort_unstable();
        let over = costs
            .iter()
            .filter(|&&d| d > Duration::from_secs_f64(1.0 / 120.0))
            .count();
        eprintln!(
            "800x560 scale={} unpaced CPU throughput={frames_per_second:.0} frames/s, frame work p50={:?} p95={:?} p99={:?} over 120 Hz budget={over}/{}",
            scale.0,
            costs[179],
            costs[341],
            costs[356],
            costs.len()
        );
    }
    Ok(())
}

#[test]
fn fps_readout_uses_elapsed_callback_intervals() -> Result<()> {
    use solor::{Draw, Ui};

    let (mut ui, root) = gallery()?;
    let view = ui.mount(root)?;
    let submit = |ui: &mut Ui| -> Result<Vec<solor::SharedString>> {
        ui.flush()?;
        ui.layout(
            view,
            Size {
                width: px(800.0),
                height: px(560.0),
            },
        )?;
        let frame = ui.record_frame(view)?;
        let labels = frame
            .draws()
            .iter()
            .filter_map(|draw| match draw {
                Draw::Text { paragraph, .. } => Some(paragraph.text.clone()),
                _ => None,
            })
            .collect();
        ui.accept_pointer_frame(frame.into_pointer_frame())?;
        Ok(labels)
    };
    submit(&mut ui)?;
    let point = PointerSample {
        id: PointerId(1),
        kind: PointerKind::Mouse,
        position: Origin {
            x: px(461.0),
            y: px(29.0),
        },
        modifiers: Default::default(),
    };
    for input in [
        PointerInput::Down(point, PointerButton::Primary),
        PointerInput::Up(point, PointerButton::Primary),
    ] {
        ui.dispatch_pointer(view, input)?;
    }
    assert!(submit(&mut ui)?.iter().any(|label| label.as_str() == "--"));
    for millis in [1000, 1050, 1600, 1900, 2250] {
        ui.dispatch_frame(view, Duration::from_millis(millis))?;
        let labels = submit(&mut ui)?;
        let expected = if millis < 2250 { "--" } else { "3.2" };
        assert!(
            labels
                .windows(2)
                .any(|pair| pair[0] == "Callback FPS" && pair[1] == expected)
        );
    }
    Ok(())
}

#[test]
fn sparse_market_repairs_match_full_redraw_at_fractional_scale() -> Result<()> {
    let scale = RasterScale(1.5);
    let (mut cached, size) = market_window(scale)?;
    let (mut full, _) = market_window(scale)?;
    let mut saw_sparse_damage = false;
    for tick in 1..=24 {
        let time = Duration::from_millis(tick * 8);
        cached.event(WindowEvent::FrameReady(time))?;
        full.event(WindowEvent::FrameReady(time))?;
        let repaired = cached.frame(size, scale)?;
        saw_sparse_damage |= repaired.damage.rects().len() > 1;
        // Discard the prepared reference frame to force a fresh surface at the same tick.
        full.frame(size, scale)?;
        let reference = full.frame(size, scale)?;
        assert!(
            repaired.rgba == reference.rgba,
            "pixel mismatch at tick {tick}"
        );
        cached.frame_submitted()?;
        full.frame_submitted()?;
    }
    assert!(saw_sparse_damage);
    Ok(())
}
