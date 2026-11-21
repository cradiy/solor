//! Pointer navigation through the same mounted window used by the native launcher.
use anyhow::Result;
use solor::{
    WindowControl,
    keyboard::{Key, KeyEvent, KeyState, NamedKey},
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
    let point = mouse_at(x, y);
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
fn startup_keyboard_navigation_keeps_focus_on_the_activated_tab() -> Result<()> {
    let (ui, root) = gallery()?;
    let mut window = CpuWindow::mount(ui, root)?;
    let size = Size {
        width: px(800.),
        height: px(560.),
    };
    let submit = |w: &mut CpuWindow| -> Result<_> {
        let frame = w.frame(size, RasterScale::ONE)?;
        let pixels = frame.rgba.to_vec();
        w.frame_submitted()?;
        Ok(pixels)
    };
    let key = |w: &mut CpuWindow, key: Key| -> Result<()> {
        for state in [KeyState::Down, KeyState::Up] {
            w.event(WindowEvent::Key(KeyEvent {
                key: key.clone(),
                state,
                modifiers: Default::default(),
            }))?;
        }
        Ok(())
    };
    let original = submit(&mut window)?;
    key(&mut window, Key::Tab)?;
    let focused = submit(&mut window)?;
    let header_end = 800 * 58 * 4;
    assert!(focused[..header_end] != original[..header_end]);
    assert!(focused[header_end..] == original[header_end..]);
    key(&mut window, Key::Enter)?;
    let lists = submit(&mut window)?;
    assert!(lists[header_end..] != original[header_end..]);
    key(&mut window, Key::Tab)?;
    key(&mut window, Key::Space)?;
    let markets = window.frame(size, RasterScale::ONE)?;
    assert!(
        markets.request_callback,
        "Markets should start its live feed"
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
        let sample = mouse_at(x, 330.0);
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
        let mouse = mouse_at(x, 330.0);
        Ok(window
            .event(WindowEvent::Pointer(PointerInput::Move(mouse)))?
            .redraw)
    };
    assert!(move_to(&mut window, 480.0)?);
    let guide = sample(&mut window)?;
    let changed: Vec<_> = guide
        .as_chunks::<4>()
        .0
        .iter()
        .zip(paused.as_chunks::<4>().0)
        .enumerate()
        .filter_map(|(i, (a, b))| (a != b).then_some(i % 800))
        .collect();
    assert!(changed.len() >= 100);
    assert!(changed.iter().all(|x| x.abs_diff(480) <= 8));
    assert!(!move_to(&mut window, 480.01)?);
    assert!(move_to(&mut window, 550.0)?);
    assert!(guide != sample(&mut window)?);
    assert!(move_to(&mut window, 480.0)?);
    sample(&mut window)?;
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
    for key in [NamedKey::Tab, NamedKey::Enter] {
        assert!(
            window
                .event(WindowEvent::Key(KeyEvent {
                    key: key.into(),
                    state: KeyState::Down,
                    modifiers: solor::keyboard::Modifiers {
                        shift: key == NamedKey::Tab,
                        ..Default::default()
                    },
                }))?
                .redraw
        );
        sample(&mut window)?;
    }
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
        for inspecting in [false, true] {
            let (mut window, size) = market_window(scale)?;
            if inspecting {
                window.event(WindowEvent::Pointer(PointerInput::Move(mouse_at(
                    480.0, 330.0,
                ))))?;
                window.frame(size, scale)?;
                window.frame_submitted()?;
            }
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
            let frames_per_second =
                costs.len() as f64 / costs.iter().sum::<Duration>().as_secs_f64();
            costs.sort_unstable();
            let over = costs
                .iter()
                .filter(|&&d| d > Duration::from_secs_f64(1.0 / 120.0))
                .count();
            eprintln!(
                "800x560 scale={} inspecting={inspecting} unpaced CPU throughput={frames_per_second:.0} frames/s, frame work p50={:?} p95={:?} p99={:?} over 120 Hz budget={over}/{}",
                scale.0,
                costs[179],
                costs[341],
                costs[356],
                costs.len()
            );
        }
    }
    Ok(())
}

fn submit_labels(ui: &mut solor::Ui, view: solor::ViewId) -> Result<Vec<solor::SharedString>> {
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
            solor::Draw::Text { paragraph, .. } => Some(paragraph.text.clone()),
            _ => None,
        })
        .collect();
    ui.accept_pointer_frame(frame.into_pointer_frame())?;
    Ok(labels)
}

fn mouse_at(x: f32, y: f32) -> PointerSample {
    PointerSample {
        id: PointerId(1),
        kind: PointerKind::Mouse,
        position: Origin { x: px(x), y: px(y) },
        modifiers: Default::default(),
    }
}

fn label_target(frame: &solor::ViewFrame, name: &str) -> Option<PointerSample> {
    frame.draws().iter().find_map(|draw| {
        let solor::Draw::Text {
            paragraph,
            origin,
            clip,
            ..
        } = draw
        else {
            return None;
        };
        if paragraph.text.as_str() != name {
            return None;
        }
        let target = mouse_at(origin.x.get() + 3.0, origin.y.get() + 5.0);
        let visible = clip.as_ref().is_none_or(|clip| {
            clip.x
                .as_ref()
                .is_none_or(|x| x.contains(&target.position.x))
                && clip
                    .y
                    .as_ref()
                    .is_none_or(|y| y.contains(&target.position.y))
        });
        visible.then_some(target)
    })
}

fn click_label(ui: &mut solor::Ui, view: solor::ViewId, name: &str) -> Result<PointerSample> {
    let target = label_target(&ui.record_frame(view)?, name)
        .ok_or_else(|| anyhow::anyhow!("missing visible label {name}"))?;
    for input in [
        PointerInput::Down(target, PointerButton::Primary),
        PointerInput::Up(target, PointerButton::Primary),
    ] {
        ui.dispatch_pointer(view, input)?;
    }
    Ok(target)
}

#[test]
fn atlas_scrubs_across_the_color_space_and_returns_to_idle() -> Result<()> {
    use solor::{Draw, pointer::ScrollDelta};
    let (mut ui, root) = gallery()?;
    let view = ui.mount(root)?;
    submit_labels(&mut ui, view)?;
    click_label(&mut ui, view, "Lists")?;
    let labels = submit_labels(&mut ui, view)?;
    assert!(labels.iter().any(|s| s.as_str() == "#000000"));
    assert!((2..20).contains(&labels.iter().filter(|s| s.starts_with('#')).count()));
    let rail = ui
        .record_frame(view)?
        .draws()
        .iter()
        .find_map(|draw| match draw {
            Draw::Fill { origin, size, .. } if size.height == px(4.0) && size.width > px(100.0) => {
                Some((*origin, *size))
            }
            _ => None,
        })
        .expect("seek rail");
    let (start, size) = rail;
    let midpoint = mouse_at((start.x + size.width / 2.0).get(), start.y.get() + 2.0);
    ui.dispatch_pointer(view, PointerInput::Down(midpoint, PointerButton::Primary))?;
    let halfway = submit_labels(&mut ui, view)?;
    assert!(halfway.iter().any(|s| s.as_str() == "#000001"));
    let outside = mouse_at(860.0, 400.0);
    ui.dispatch_pointer(view, PointerInput::Move(outside))?;
    assert!(
        submit_labels(&mut ui, view)?
            .iter()
            .any(|s| s.as_str() == "#FFFFFF")
    );
    ui.dispatch_pointer(view, PointerInput::Up(outside, PointerButton::Primary))?;
    submit_labels(&mut ui, view)?;
    ui.dispatch_pointer(view, PointerInput::Move(midpoint))?;
    assert!(
        submit_labels(&mut ui, view)?
            .iter()
            .any(|s| s.as_str() == "#FFFFFF")
    );
    ui.dispatch_pointer(
        view,
        PointerInput::Scroll(
            mouse_at(240.0, 220.0),
            ScrollDelta {
                x: px(0.0),
                y: px(-167_772_160.0), // One quarter of the color rows.
            },
        ),
    )?;
    assert!(
        !submit_labels(&mut ui, view)?
            .iter()
            .any(|s| s.as_str() == "#FFFFFF")
    );
    submit_labels(&mut ui, view)?;
    let thumb = ui
        .record_frame(view)?
        .draws()
        .iter()
        .find_map(|draw| match draw {
            Draw::Fill { origin, size, .. }
                if size.width == px(12.0) && size.height == px(12.0) =>
            {
                Some(origin.x + size.width / 2.0)
            }
            _ => None,
        })
        .expect("seek thumb");
    assert!((thumb - start.x - size.width * 0.75).get().abs() < 0.5);
    assert!(ui.next_redraw(view).is_none());
    for (key, swatch) in [(Key::Home, "#000000"), (Key::End, "#FFFFFF")] {
        let press = KeyEvent {
            key,
            state: KeyState::Down,
            modifiers: Default::default(),
        };
        assert!(ui.dispatch_key(view, &press)?);
        assert!(
            submit_labels(&mut ui, view)?
                .iter()
                .any(|s| s.as_str() == swatch)
        );
    }
    click_label(&mut ui, view, "Lists")?;
    let home = KeyEvent {
        key: Key::Home,
        state: KeyState::Down,
        modifiers: Default::default(),
    };
    assert!(!ui.dispatch_key(view, &home)?);
    assert!(
        submit_labels(&mut ui, view)?
            .iter()
            .any(|s| s.as_str() == "#FFFFFF")
    );
    Ok(())
}

fn market_view() -> Result<(solor::Ui, solor::ViewId)> {
    let (mut ui, root) = gallery()?;
    let view = ui.mount(root)?;
    open_market(&mut ui, view)?;
    Ok((ui, view))
}

fn open_market(ui: &mut solor::Ui, view: solor::ViewId) -> Result<()> {
    submit_labels(ui, view)?;
    let point = mouse_at(461.0, 29.0);
    for input in [
        PointerInput::Down(point, PointerButton::Primary),
        PointerInput::Up(point, PointerButton::Primary),
    ] {
        ui.dispatch_pointer(view, input)?;
    }
    Ok(())
}

#[test]
fn market_zoom_passes_unused_scroll_to_its_container() -> Result<()> {
    use solor::{Styled, component, div, pointer::ScrollDelta};
    use std::{cell::RefCell, rc::Rc};

    let (mut ui, root) = gallery()?;
    let passed = Rc::new(RefCell::new(Vec::new()));
    let view = ui.mount(component(|_| {
        let passed = passed.clone();
        div()
            .items_stretch()
            .on_scroll(move |event, _| {
                passed.borrow_mut().push(event.delta);
                Ok(())
            })
            .child(root)
    }))?;
    open_market(&mut ui, view)?;
    submit_labels(&mut ui, view)?;
    let frame = ui.record_frame(view)?;
    let point = frame
        .draws()
        .iter()
        .find_map(|draw| match draw {
            solor::Draw::Polyline { origin, points, .. } => {
                let p = points[points.len() / 2];
                Some(mouse_at((origin.x + p.x).get(), (origin.y + p.y).get()))
            }
            _ => None,
        })
        .expect("market price line");
    let delta = |x, y| ScrollDelta { x: px(x), y: px(y) };
    // The first half pixel adjusts continuous zoom without changing the rounded label.
    for (input, labels, remainder) in [
        (delta(1.25, -0.5), "96 samples", Some(delta(1.25, 0.0))),
        (delta(0.0, -0.75), "95 samples", None),
        (delta(-3.5, 4.0), "96 samples", Some(delta(-3.5, 2.75))),
        (delta(0.0, -170.25), "12 samples", Some(delta(0.0, -2.25))),
        (delta(-0.5, -0.75), "12 samples", Some(delta(-0.5, -0.75))),
    ] {
        ui.dispatch_pointer(view, PointerInput::Scroll(point, input))?;
        assert_eq!(passed.take().as_slice(), remainder.as_slice());
        assert!(
            submit_labels(&mut ui, view)?
                .iter()
                .any(|s| s.as_str() == labels)
        );
    }
    Ok(())
}

#[test]
fn resized_market_keeps_the_inspection_marker_under_a_stationary_mouse() -> Result<()> {
    use solor::Draw;

    let (mut ui, view) = market_view()?;
    submit_labels(&mut ui, view)?;
    let pause = mouse_at(750.0, 85.0);
    ui.dispatch_pointer(view, PointerInput::Down(pause, PointerButton::Primary))?;
    ui.dispatch_pointer(view, PointerInput::Up(pause, PointerButton::Primary))?;
    let mouse = mouse_at(550.0, 330.0);
    ui.dispatch_pointer(view, PointerInput::Move(mouse))?;
    submit_labels(&mut ui, view)?;

    let resized = Size {
        width: px(1000.0),
        height: px(560.0),
    };
    ui.layout(view, resized)?;
    let frame = ui.record_frame(view)?;
    ui.accept_pointer_frame(frame.into_pointer_frame())?;
    ui.flush()?;
    ui.layout(view, resized)?;
    let frame = ui.record_frame(view)?;
    let expected = frame
        .draws()
        .iter()
        .find_map(|draw| {
            let Draw::Polyline { origin, points, .. } = draw else {
                return None;
            };
            points.iter().map(|p| origin.x + p.x).min_by(|a, b| {
                (a.get() - mouse.position.x.get())
                    .abs()
                    .total_cmp(&(b.get() - mouse.position.x.get()).abs())
            })
        })
        .expect("nearest market sample");
    let marker = frame
        .draws()
        .iter()
        .find_map(|draw| match draw {
            Draw::Fill { origin, size, .. } if size.width == px(5.0) && size.height == px(5.0) => {
                Some(origin.x + px(2.5))
            }
            _ => None,
        })
        .expect("inspection marker");
    assert_eq!(marker, expected);
    ui.accept_pointer_frame(frame.into_pointer_frame())?;
    assert!(ui.next_redraw(view).is_none());
    Ok(())
}

#[test]
fn inspected_price_and_average_follow_the_sample_as_it_moves_through_history() -> Result<()> {
    let (mut ui, view) = market_view()?;
    submit_labels(&mut ui, view)?;
    let frame = ui.record_frame(view)?;
    let (left, right) = frame
        .draws()
        .iter()
        .find_map(|draw| {
            let solor::Draw::Polyline { origin, points, .. } = draw else {
                return None;
            };
            Some((origin.x + points.first()?.x, origin.x + points.last()?.x))
        })
        .expect("market price line");
    let right = mouse_at(right.get(), 330.0);
    let left = mouse_at(left.get(), 330.0);
    let readout = |ui: &mut solor::Ui| -> Result<_> {
        submit_labels(ui, view)?
            .into_iter()
            .find(|text| text.starts_with("Price ") && text.contains("MA "))
            .ok_or_else(|| anyhow::anyhow!("missing inspected price and moving average"))
    };
    ui.dispatch_pointer(view, PointerInput::Move(right))?;
    let initial = readout(&mut ui)?;
    for tick in 1..=11 {
        ui.dispatch_frame(view, Duration::from_millis(tick * 8))?;
        submit_labels(&mut ui, view)?;
    }
    assert_ne!(initial, readout(&mut ui)?);
    ui.dispatch_pointer(view, PointerInput::Move(left))?;
    ui.dispatch_pointer(
        view,
        PointerInput::Scroll(
            left,
            solor::pointer::ScrollDelta {
                x: px(0.0),
                y: px(-200.0),
            },
        ),
    )?;
    assert_eq!(initial, readout(&mut ui)?);
    Ok(())
}

#[test]
fn fps_readout_uses_elapsed_callback_intervals() -> Result<()> {
    let (mut ui, view) = market_view()?;
    assert!(
        submit_labels(&mut ui, view)?
            .iter()
            .any(|label| label.as_str() == "--")
    );
    for millis in [1000, 1050, 1600, 1900, 2250] {
        ui.dispatch_frame(view, Duration::from_millis(millis))?;
        let labels = submit_labels(&mut ui, view)?;
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

#[test]
fn compact_market_scrolls_the_watchlist_below_its_header() -> Result<()> {
    let (mut ui, view) = market_view()?;
    let viewport = Size {
        width: px(800.0),
        height: px(320.0),
    };
    let submit = |ui: &mut solor::Ui| -> Result<_> {
        ui.flush()?;
        ui.layout(view, viewport)?;
        let frame = ui.record_frame(view)?;
        let label = |name| {
            frame.draws().iter().find_map(|draw| match draw {
                solor::Draw::Text {
                    origin, paragraph, ..
                } if paragraph.text.as_str() == name => Some(*origin),
                _ => None,
            })
        };
        let positions = (label("Watchlist").expect("fixed header"), label("QBFG"));
        ui.accept_pointer_frame(frame.into_pointer_frame())?;
        Ok(positions)
    };
    let (header, last) = submit(&mut ui)?;
    assert!(last.is_none());
    ui.dispatch_pointer(
        view,
        PointerInput::Scroll(
            mouse_at(header.x.get(), header.y.get() + 60.0),
            solor::pointer::ScrollDelta {
                x: px(0.0),
                y: px(400.0),
            },
        ),
    )?;
    let (scrolled_header, scrolled_last) = submit(&mut ui)?;
    let scrolled_last = scrolled_last.expect("last quote enters the viewport");
    assert_eq!(scrolled_header, header);
    assert!(scrolled_last.y < viewport.height && scrolled_last.y > header.y);
    let target = mouse_at(scrolled_last.x.get() + 2.0, scrolled_last.y.get() + 4.0);
    for input in [
        PointerInput::Down(target, PointerButton::Primary),
        PointerInput::Up(target, PointerButton::Primary),
    ] {
        ui.dispatch_pointer(view, input)?;
    }
    let labels = submit_labels(&mut ui, view)?;
    assert_eq!(labels.iter().filter(|s| s.as_str() == "QBFG").count(), 2);
    Ok(())
}

fn open_tone_sheet(ui: &mut solor::Ui, view: solor::ViewId) -> Result<[PointerSample; 2]> {
    submit_labels(ui, view)?;
    let lists = click_label(ui, view, "Lists")?;
    submit_labels(ui, view)?;
    let sheet = click_label(ui, view, "Tone sheet")?;
    Ok([lists, sheet])
}

#[test]
fn tone_sheet_selects_the_last_swatch_after_scrolling_both_axes() -> Result<()> {
    use solor::{Draw, pointer::ScrollDelta};
    use solor_render_cpu::pixels::{DeviceSize, Surface};
    let (mut ui, root) = gallery()?;
    let view = ui.mount(root)?;
    open_tone_sheet(&mut ui, view)?;
    let labels = submit_labels(&mut ui, view)?;
    assert!(labels.iter().any(|s| s.as_str() == "Clay 100"));
    assert!(label_target(&ui.record_frame(view)?, "Rose 900").is_none());
    let chip = |ui: &solor::Ui| -> Result<Origin> {
        ui.record_frame(view)?
            .draws()
            .iter()
            .find_map(|d| match d {
                Draw::Fill { origin, size, .. }
                    if size.width == px(20.0) && size.height == px(20.0) =>
                {
                    Some(*origin)
                }
                _ => None,
            })
            .ok_or_else(|| anyhow::anyhow!("missing selection chip"))
    };
    let fixed = chip(&ui)?;
    let pan = |ui: &mut solor::Ui, distance| -> Result<()> {
        ui.dispatch_pointer(
            view,
            PointerInput::Scroll(
                mouse_at(240.0, 250.0),
                ScrollDelta {
                    x: px(distance),
                    y: px(distance),
                },
            ),
        )?;
        Ok(())
    };
    pan(&mut ui, 2000.0)?;
    submit_labels(&mut ui, view)?;
    click_label(&mut ui, view, "Rose 900")?;
    let labels = submit_labels(&mut ui, view)?;
    assert!(labels.iter().any(|s| s.as_str() == "Rose 900  /  #503D3F"));
    pan(&mut ui, -2000.0)?;
    let labels = submit_labels(&mut ui, view)?;
    assert!(labels.iter().any(|s| s.as_str() == "Clay 100"));
    assert!(label_target(&ui.record_frame(view)?, "Rose 900").is_none());
    click_label(&mut ui, view, "Show selection")?;
    assert!(ui.next_redraw(view).is_some());
    submit_labels(&mut ui, view)?;
    assert!(label_target(&ui.record_frame(view)?, "Rose 900").is_some());
    assert_eq!(chip(&ui)?, fixed);
    let surface = Surface::draw_view(
        &ui.record_frame(view)?,
        DeviceSize {
            width: 800,
            height: 560,
        },
        RasterScale::ONE,
    )?;
    let at =
        ((fixed.y.get() as usize + 10) * surface.size.width as usize + fixed.x.get() as usize + 10)
            * 4;
    assert_eq!(&surface.rgba[at..at + 4], &[0x50, 0x3d, 0x3f, 255]);
    assert!(ui.next_redraw(view).is_none());
    Ok(())
}

#[test]
#[ignore = "release frame recording measurement"]
fn tone_sheet_recording_cost() -> Result<()> {
    use solor::Draw;
    use std::{hint::black_box, time::Instant};

    let (mut ui, root) = gallery()?;
    let view = ui.mount(root)?;
    open_tone_sheet(&mut ui, view)?;
    submit_labels(&mut ui, view)?;
    for (width, height, scrolling) in [
        (800.0, 560.0, true),
        (800.0, 560.0, false),
        (1440.0, 900.0, false),
    ] {
        let size = Size {
            width: px(width),
            height: px(height),
        };
        let mut geometry = Vec::new();
        let mut painting = Vec::new();
        let mut recording = Vec::new();
        let mut total = Vec::new();
        let mut text_commands = 0;
        for i in 0..832 {
            let start = Instant::now();
            if scrolling {
                ui.dispatch_pointer(view, tone_sheet_pan(i))?;
            }
            ui.flush()?;
            ui.layout(view, size)?;
            let record = Instant::now();
            drop(black_box(ui.prepare_pointer_frame(view)?));
            let prepared = Instant::now();
            let frame = black_box(ui.record_frame(view)?);
            let recorded = Instant::now();
            let (draws, pointer) = frame.into_parts();
            ui.accept_pointer_frame(pointer)?;
            let through_accept = start.elapsed();
            if i >= 64 {
                total.push(through_accept);
                geometry.push(prepared - record);
                painting.push(recorded - prepared);
                recording.push(recorded - record);
                text_commands += draws
                    .iter()
                    .filter(|d| matches!(d, Draw::Text { .. }))
                    .count();
            }
        }
        geometry.sort_unstable();
        painting.sort_unstable();
        recording.sort_unstable();
        total.sort_unstable();
        let mid = recording.len() / 2;
        eprintln!(
            "{width}x{height} scrolling={scrolling} text/frame={:.1} geometry_p50_ns={} paint_p50_ns={} record_p50_ns={} input_to_accept_p50_ns={}",
            text_commands as f64 / recording.len() as f64,
            geometry[mid].as_nanos(),
            painting[mid].as_nanos(),
            recording[mid].as_nanos(),
            total[mid].as_nanos()
        );
    }
    Ok(())
}

fn tone_sheet_pan(frame: usize) -> PointerInput {
    let sign = if frame % 128 < 64 { 1.0 } else { -1.0 };
    PointerInput::Scroll(
        mouse_at(320.0, 280.0),
        solor::pointer::ScrollDelta {
            x: px(3.25 * sign),
            y: px(2.75 * sign),
        },
    )
}

#[test]
#[ignore = "release CPU scroll measurement"]
fn tone_sheet_cpu_frame_cost() -> Result<()> {
    use std::{hint::black_box, time::Instant};

    let size = Size {
        width: px(800.0),
        height: px(560.0),
    };
    let navigation = {
        let (mut ui, root) = gallery()?;
        let view = ui.mount(root)?;
        open_tone_sheet(&mut ui, view)?
    };
    for scale in [RasterScale::ONE, RasterScale(1.5)] {
        let (ui, root) = gallery()?;
        let mut window = CpuWindow::mount(ui, root)?;
        window.frame(size, scale)?;
        window.frame_submitted()?;
        for target in navigation {
            click(
                &mut window,
                target.position.x.get(),
                target.position.y.get(),
            )?;
            window.frame(size, scale)?;
            window.frame_submitted()?;
        }
        let mut times = Vec::with_capacity(256);
        for i in 0..320 {
            let start = Instant::now();
            window.event(WindowEvent::Pointer(tone_sheet_pan(i)))?;
            let frame = window.frame(size, scale)?;
            assert!(!frame.damage.is_empty());
            black_box(frame.rgba);
            window.frame_submitted()?;
            if i >= 64 {
                times.push(start.elapsed());
            }
        }
        times.sort_unstable();
        let over_budget = times
            .iter()
            .filter(|t| **t > Duration::from_secs_f64(1.0 / 120.0))
            .count();
        eprintln!(
            "800x560 scale={} scroll CPU frame p50_us={} p95_us={} over_budget={over_budget}/{}",
            scale.0,
            times[127].as_micros(),
            times[243].as_micros(),
            times.len(),
        );
    }
    Ok(())
}
