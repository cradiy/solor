//! Retained and host-paced elements sharing one mounted CPU window.
use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

use anyhow::Result;
use solor::{
    Element, Node, PaintContext, RefreshMode, Styled, Ui, component, div, element,
    layout::{
        self, LayoutContext, LayoutNode, LayoutOutput, MeasureContext, Metrics, Origin, Query, Size,
    },
    platform::{WindowEvent, WindowHandler},
    pointer::{PointerId, PointerInput, PointerKind, PointerSample},
    px,
    raster::RasterScale,
    rgb, text,
};
use solor_render_cpu::CpuWindow;

const SIZE: Size = Size {
    width: px(320.0),
    height: px(180.0),
};
const FONT: &[u8] = include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf");

struct Bar<F>(F);
impl<F: 'static> LayoutNode for Bar<F> {
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
impl<F: Fn(&mut PaintContext) -> Result<u16> + 'static> Element for Bar<F> {
    fn paint(&self, cx: &mut PaintContext) -> Result<()> {
        let width = px(f32::from((self.0)(cx)?));
        cx.fill(Size { width, ..cx.size }, rgb(0x2168c4));
        Ok(())
    }
}

fn bar(sample: impl Fn(&mut PaintContext) -> Result<u16> + 'static) -> Node<impl Element> {
    element(Bar(sample)).h(px(8.0))
}

#[derive(Default)]
struct Samples {
    latest: Cell<u16>,
    slow_paints: Cell<usize>,
    fast_paints: Cell<usize>,
}

fn refresh_mode(active: bool) -> RefreshMode {
    if active {
        RefreshMode::Continuous
    } else {
        RefreshMode::OnDemand
    }
}

fn mixed_window(samples: &Rc<Samples>, active: bool, labels: usize) -> Result<CpuWindow> {
    let slow = samples.clone();
    let fast = samples.clone();
    CpuWindow::mount(
        Ui::with_font(FONT.to_vec(), "en")?,
        component(move |cx| {
            let level = cx.signal(10u16);
            let mode = cx.signal(refresh_mode(active));
            div()
                .flex_col()
                .child(bar(move |cx| {
                    slow.slow_paints.update(|n| n + 1);
                    Ok(*cx.read(level)?)
                }))
                .child(
                    bar(move |_| {
                        fast.fast_paints.update(|n| n + 1);
                        Ok(fast.latest.get())
                    })
                    .refresh_mode(mode),
                )
                .on_pointer_move(move |event, cx| {
                    let x = event.local_position().x.get() as u16;
                    if x < 2 {
                        cx.set(mode, refresh_mode(x == 1))
                    } else {
                        cx.set(level, x)
                    }
                })
                .children((0..labels).map(|_| text("Output level")))
        }),
    )
}

fn pointer(x: u16) -> WindowEvent {
    WindowEvent::Pointer(PointerInput::Move(PointerSample {
        id: PointerId(1),
        kind: PointerKind::Mouse,
        position: Origin {
            x: px(f32::from(x)),
            y: px(4.0),
        },
        modifiers: Default::default(),
    }))
}

fn pixel(rgba: &[u8], x: usize, y: usize) -> &[u8] {
    let at = 4 * (320 * y + x);
    &rgba[at..at + 4]
}

#[test]
fn paint_dependencies_follow_successful_branches_and_retire_with_the_element() -> Result<()> {
    let mut ui = Ui::new();
    let first = ui.context().signal(10u16);
    let second = ui.context().signal(20u16);
    let use_second = ui.context().signal(false);
    let visible = ui.context().signal(true);
    let event_only = ui.context().signal(0u8);
    let fail = Rc::new(Cell::new(false));
    let view = ui.mount(component(|_| {
        div()
            .child(bar(move |cx| Ok(*cx.read(first)?)))
            .child_when(visible, move || {
                let fail_for_click = fail.clone();
                let fail = fail.clone();
                bar(move |cx| {
                    let source = if *cx.read(use_second)? { second } else { first };
                    let value = *cx.read(source)?;
                    anyhow::ensure!(!fail.replace(false), "paint interrupted");
                    Ok(value)
                })
                .on_click(move |cx| {
                    cx.read(event_only)?;
                    fail_for_click.set(true);
                    Ok(())
                })
            })
    }))?;
    ui.flush()?;
    ui.layout(view, SIZE)?;
    ui.record_frame(view)?;
    let root = ui.root(view)?;
    let children: Vec<_> = ui.children(view, root)?.collect();
    let mut ready = Vec::new();

    ui.context().set(first, 12)?;
    ui.context().set(first, 14)?;
    ui.flush()?;
    ui.collect_repaints(view, &mut ready)?;
    assert_eq!(ready.len(), 2);
    assert!(children.iter().all(|child| ready.contains(child)));
    ui.record_frame(view)?;

    ui.click(view, children[1])?;
    ui.context().set(event_only, 1)?;
    ui.context().set(first, 14)?;
    assert!(!ui.has_pending_bindings());

    ui.context().set(use_second, true)?;
    ui.flush()?;
    assert!(ui.record_frame(view).is_err());
    ui.context().set(second, 21)?;
    assert!(!ui.has_pending_bindings());
    ui.record_frame(view)?;

    ui.context().set(first, 15)?;
    ui.flush()?;
    ready.clear();
    ui.collect_repaints(view, &mut ready)?;
    assert_eq!(ready, [children[0]]);
    ui.record_frame(view)?;

    ui.context().set(second, 22)?;
    ui.flush()?;
    ready.clear();
    ui.collect_repaints(view, &mut ready)?;
    assert_eq!(ready, [children[1]]);

    ui.context().set(visible, false)?;
    ui.flush()?;
    ui.context().set(second, 23)?;
    assert!(!ui.has_pending_bindings());
    ready.clear();
    ui.collect_repaints(view, &mut ready)?;
    assert!(ready.is_empty());
    Ok(())
}

#[test]
fn retained_sibling_repaints_only_on_change_while_meter_starts_and_stops() -> Result<()> {
    let samples = Rc::new(Samples::default());
    let mut window = mixed_window(&samples, false, 4)?;
    window.frame(SIZE, RasterScale::ONE)?;
    window.frame_submitted()?;
    assert!(!window.event(WindowEvent::FrameReady)?);
    assert!(window.event(pointer(1))?);
    for width in [16, 16, 37, 28] {
        samples.latest.set(width - 1);
        samples.latest.set(width);
        let frame = window.frame(SIZE, RasterScale::ONE)?;
        assert!(frame.request_callback);
        assert_eq!(
            pixel(frame.rgba, usize::from(width) - 1, 10),
            [0x21, 0x68, 0xc4, 255]
        );
        assert_eq!(pixel(frame.rgba, usize::from(width), 10), [255; 4]);
        window.frame_submitted()?;
        assert!(window.event(WindowEvent::FrameReady)?);
        assert!(!window.event(WindowEvent::FrameReady)?);
    }
    assert_eq!(
        (samples.slow_paints.get(), samples.fast_paints.get()),
        (1, 5)
    );
    window.event(pointer(15))?;
    window.event(pointer(23))?;
    window.event(pointer(0))?;
    let last = window.frame(SIZE, RasterScale::ONE)?;
    assert_eq!(pixel(last.rgba, 22, 4), [0x21, 0x68, 0xc4, 255]);
    window.frame_submitted()?;
    assert!(!window.event(WindowEvent::FrameReady)?);
    assert!(window.frame(SIZE, RasterScale::ONE)?.damage.is_none());
    window.frame_submitted()?;
    assert_eq!(
        (samples.slow_paints.get(), samples.fast_paints.get()),
        (2, 6)
    );
    // A burst before the next frame resolves to the final mode.
    for x in [1, 0, 1] {
        window.event(pointer(x))?;
    }
    window.frame(SIZE, RasterScale::ONE)?;
    window.frame_submitted()?;
    assert!(window.event(WindowEvent::FrameReady)?);
    assert_eq!(
        (samples.slow_paints.get(), samples.fast_paints.get()),
        (2, 7)
    );
    Ok(())
}

#[test]
fn returning_to_on_demand_keeps_the_current_paint_subscription() -> Result<()> {
    let mut ui = Ui::new();
    let level = ui.context().signal(8u16);
    let mode = ui.context().signal(RefreshMode::Continuous);
    let paints = Rc::new(Cell::new(0));
    let count = paints.clone();
    let view = ui.mount(component(|_| {
        bar(move |cx| {
            count.update(|n| n + 1);
            Ok(*cx.read(level)?)
        })
        .refresh_mode(mode)
    }))?;
    ui.layout(view, SIZE)?;
    for _ in 0..2 {
        assert!(ui.record_frame(view)?.requests_animation_frame());
    }
    ui.context().set(mode, RefreshMode::OnDemand)?;
    ui.flush()?;
    for _ in 0..2 {
        assert!(!ui.record_frame(view)?.requests_animation_frame());
    }
    assert_eq!(paints.get(), 3);
    ui.context().set(level, 19)?;
    ui.flush()?;
    let frame = ui.record_frame(view)?;
    assert!(!frame.requests_animation_frame());
    assert!(matches!(frame.draws(), [solor::Draw::Fill { size, .. }] if size.width == px(19.0)));
    assert_eq!(paints.get(), 4);
    Ok(())
}

#[test]
fn animation_wakeup_belongs_to_the_submitted_frame() -> Result<()> {
    let mut window = mixed_window(&Rc::new(Samples::default()), false, 0)?;
    window.frame(SIZE, RasterScale::ONE)?;
    window.frame_submitted()?;

    window.event(pointer(1))?;
    window.frame(SIZE, RasterScale::ONE)?;
    assert!(!window.event(WindowEvent::FrameReady)?);
    window.frame(SIZE, RasterScale::ONE)?;
    window.frame_submitted()?;

    // The host drops the first stopped frame. The previously accepted wakeup is still live.
    window.event(pointer(0))?;
    window.frame(SIZE, RasterScale::ONE)?;
    assert!(window.event(WindowEvent::FrameReady)?);
    window.frame(SIZE, RasterScale::ONE)?;
    window.frame_submitted()?;
    assert!(!window.event(WindowEvent::FrameReady)?);
    Ok(())
}

#[test]
#[ignore = "paced release workload"]
fn mixed_updates_at_120_hz() -> Result<()> {
    let samples = Rc::new(Samples::default());
    let mut window = mixed_window(&samples, true, 1024)?;
    window.frame(SIZE, RasterScale::ONE)?;
    window.frame_submitted()?;
    let start = Instant::now();
    let tick = |n| Duration::from_secs_f64(f64::from(n) / 120.0);
    let mut times = Vec::new();
    let mut late = 0;
    for n in 1u32..=260 {
        std::thread::sleep((start + tick(n)).saturating_duration_since(Instant::now()));
        let begin = Instant::now();
        samples.latest.set((n * 37 % 237) as u16);
        assert!(window.event(WindowEvent::FrameReady)?);
        if n.is_multiple_of(60) {
            window.event(pointer(20 + n as u16 / 60))?;
        }
        let frame = window.frame(SIZE, RasterScale::ONE)?;
        std::hint::black_box(frame.rgba);
        window.frame_submitted()?;
        let end = Instant::now();
        if n > 20 {
            times.push(end - begin);
            late += usize::from(end > start + tick(n + 1));
        }
    }
    times.sort_unstable();
    assert_eq!(
        (samples.slow_paints.get(), samples.fast_paints.get()),
        (5, 261)
    );
    println!(
        "frames=240 cpu_p50_us={} cpu_p95_us={} cpu_p99_us={} cpu_max_us={} late={late}",
        times[119].as_micros(),
        times[227].as_micros(),
        times[237].as_micros(),
        times[239].as_micros()
    );
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires a visible Wayland surface"]
fn wayland_mixed_frame_cadence() -> Result<()> {
    use solor::{
        platform::{Platform, WindowOptions},
        raster::{RasterFrame, Rgba8},
    };
    use std::cell::RefCell;

    #[derive(Debug)]
    struct Finished;
    impl std::fmt::Display for Finished {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("frame sample complete")
        }
    }
    impl std::error::Error for Finished {}

    #[derive(Default)]
    struct Timings {
        callbacks: Vec<Instant>,
        submissions: Vec<Duration>,
    }
    struct Probe {
        window: CpuWindow,
        samples: Rc<Samples>,
        timings: Rc<RefCell<Timings>>,
        pending: Option<Instant>,
    }
    impl WindowHandler<Rgba8> for Probe {
        fn event(&mut self, event: WindowEvent) -> Result<bool> {
            if matches!(event, WindowEvent::FrameReady) {
                let now = Instant::now();
                let mut timings = self.timings.borrow_mut();
                timings.callbacks.push(now);
                self.pending = Some(now);
                let n = timings.callbacks.len();
                if n == 261 {
                    return Err(Finished.into());
                }
                self.samples.latest.set((n * 37 % 237) as u16);
                if n.is_multiple_of(60) {
                    self.window.event(pointer(20 + n as u16 / 60))?;
                }
                return self.window.event(event);
            }
            Ok(false)
        }
        fn frame(&mut self, size: Size, scale: RasterScale) -> Result<RasterFrame<'_>> {
            let frame = self.window.frame(size, scale)?;
            if self.timings.borrow().callbacks.len() == 20 {
                println!(
                    "native logical={size:?} scale={} buffer={:?}",
                    scale.0, frame.size
                );
            }
            Ok(frame)
        }
        fn frame_submitted(&mut self) -> Result<()> {
            self.window.frame_submitted()?;
            if let Some(start) = self.pending.take() {
                self.timings.borrow_mut().submissions.push(start.elapsed());
            }
            Ok(())
        }
    }
    let samples = Rc::new(Samples::default());
    let timings = Rc::new(RefCell::new(Timings::default()));
    let probe = Probe {
        window: mixed_window(&samples, true, 1024)?,
        samples,
        timings: timings.clone(),
        pending: None,
    };
    let platform = solor_linux::Wayland {
        trace_submissions: std::env::var_os("SOLOR_TRACE_SUBMISSIONS").is_some(),
        ..Default::default()
    };
    let outcome = platform.run_window(
        WindowOptions {
            title: "Solor frame cadence",
            app_id: "solor.refresh-measurement",
            size: SIZE,
        },
        Box::new(probe),
    );
    match outcome {
        Err(e) if e.is::<Finished>() => {}
        Err(e) => return Err(e),
        Ok(()) => anyhow::bail!("window closed before collecting the sample"),
    }
    let timings = timings.borrow();
    let callbacks = &timings.callbacks;
    let mut submissions = timings.submissions[20..].to_vec();
    assert_eq!(submissions.len(), 240);
    submissions.sort_unstable();
    println!(
        "callback_to_submission_p95_us={} max_us={}",
        submissions[227].as_micros(),
        submissions[239].as_micros()
    );
    let mut gaps: Vec<_> = callbacks[20..].windows(2).map(|w| w[1] - w[0]).collect();
    gaps.sort_unstable();
    let budget = Duration::from_secs_f64(1.0 / 120.0);
    println!(
        "callbacks=240 hz={:.1} interval_p50_us={} interval_p95_us={} interval_max_us={} over_120hz_budget={}",
        240.0 / (callbacks[260] - callbacks[20]).as_secs_f64(),
        gaps[119].as_micros(),
        gaps[227].as_micros(),
        gaps[239].as_micros(),
        gaps.iter().filter(|gap| **gap > budget).count()
    );
    Ok(())
}
