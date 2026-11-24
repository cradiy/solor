//! Drag a WGSL light band or play it continuously on the CPU.
mod surface;

use anyhow::{Context, Result};
use clap::{Arg, Command, value_parser};
use solor::{
    EffectShader, Pixels, SharedString, Styled, Ui, component, div,
    pointer::{PointerEvent, PointerId},
    px, rgb, text,
};
use solor_platform::{WindowOptions, current_platform};
use std::{cell::Cell, path::PathBuf, rc::Rc, time::Duration};

#[derive(Clone, Copy)]
struct Playback {
    last: Duration,
    phase: f64,
    interval: Option<f64>,
}

impl Playback {
    fn advance(&mut self, now: Duration) -> f32 {
        let delta = now.saturating_sub(self.last);
        self.last = now;
        if delta > Duration::from_millis(250) {
            self.interval = None;
        } else {
            let seconds = delta.as_secs_f64();
            // Spread a missed refresh over several steps of this free-running motion.
            let interval = self.interval.get_or_insert(seconds);
            *interval += (seconds - *interval) * 0.2;
            self.phase = (self.phase + *interval * 0.9) % std::f64::consts::TAU;
        }
        (0.5 - 0.5 * self.phase.cos()) as f32
    }
}

#[derive(Clone, Copy, PartialEq)]
struct Grab {
    pointer: PointerId,
    offset: Pixels,
}

fn grab_offset(
    event: &PointerEvent,
    footprint: Option<surface::Footprint>,
    center: f32,
) -> Option<Pixels> {
    let surface::Footprint::Centered {
        half_width,
        half_height,
    } = footprint?
    else {
        return None;
    };
    let size = event.content_size();
    let point = event.content_position();
    let dx = point.x - size.width * center;
    let dy = point.y - size.height * 0.5;
    (dx.get().abs() <= half_width && dy.get().abs() <= half_height).then_some(dx)
}

fn pointer_position(event: &PointerEvent, offset: Pixels) -> f32 {
    let width = event.content_size().width.get();
    ((event.content_position().x - offset).get() / width.max(f32::EPSILON)).clamp(0., 1.)
}

fn main() -> Result<()> {
    let args = Command::new("cpu_shader")
        .arg(
            Arg::new("shader")
                .value_parser(value_parser!(PathBuf))
                .help("WGSL effect file (dragging changes the first parameter)"),
        )
        .get_matches();
    let path = args.get_one::<PathBuf>("shader");
    let title = path
        .and_then(|path| path.file_stem())
        .and_then(|stem| stem.to_str())
        .unwrap_or("Light sweep")
        .to_owned();
    let source = path
        .map(|path| {
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
        })
        .transpose()?;
    let source = source.as_deref().unwrap_or(surface::SWEEP);
    let shader = EffectShader::wgsl(source)?;
    let footprint = surface::Footprint::for_source(source);
    let samples_motion = source == surface::ROUNDED_RECT;
    let view = component(move |cx| {
        let params = cx.signal([0.4, 0., 0., 0.]);
        let playing = cx.signal(false);
        let label = cx.signal(SharedString::from("Play"));
        let grab = cx.signal(None::<Grab>);
        let clock = Rc::new(Cell::new(None::<Playback>));
        let reset_clock = clock.clone();
        let surface = surface::shader_surface(shader, params, footprint)
            .on_pointer_down(move |event, cx| {
                if event.is_primary_action() && cx.read(grab)?.is_none() {
                    let center = cx.read(params)?[0];
                    let offset = grab_offset(event, footprint, center);
                    let center = offset.map_or_else(|| pointer_position(event, px(0.)), |_| center);
                    cx.set(params, [center, 0., 0., 0.])?;
                    cx.capture_pointer();
                    cx.set(
                        grab,
                        Some(Grab {
                            pointer: event.pointer_id(),
                            offset: offset.unwrap_or_default(),
                        }),
                    )?;
                    cx.set(playing, false)?;
                    cx.set(label, "Play".into())?;
                }
                Ok(())
            })
            .on_pointer_move(move |event, cx| {
                if let Some(active) = *cx.read(grab)?
                    && active.pointer == event.pointer_id()
                {
                    cx.set(params, [pointer_position(event, active.offset), 0., 0., 0.])?;
                }
                Ok(())
            })
            .on_pointer_up(move |event, cx| {
                if event.is_primary_action()
                    && cx
                        .read(grab)?
                        .is_some_and(|g| g.pointer == event.pointer_id())
                {
                    cx.set(grab, None)?;
                }
                Ok(())
            })
            .on_pointer_cancel(move |id, cx| {
                if cx.read(grab)?.is_some_and(|g| g.pointer == id) {
                    cx.set(grab, None)?;
                }
                Ok(())
            });
        div()
            .items_stretch()
            .p(px(20.))
            .gap(px(12.))
            .bg(rgb(0x111a29))
            .text_color(rgb(0xebf0f6))
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(text(title).text_size(px(24.)))
                    .child(
                        div()
                            .px(px(18.))
                            .py(px(6.))
                            .rounded_full()
                            .bg(rgb(0x314456))
                            .cursor_pointer()
                            .child(text(label))
                            .on_click(move |cx| {
                                let next = !*cx.read(playing)?;
                                reset_clock.set(None);
                                let center = cx.read(params)?[0];
                                cx.set(params, [center, 0., 0., 0.])?;
                                cx.set(playing, next)?;
                                cx.set(label, if next { "Pause" } else { "Play" }.into())
                            }),
                    ),
            )
            .child(surface)
            .on_frame_when(playing, move |now, cx| {
                if let Some(mut playback) = clock.get() {
                    let center = playback.advance(now);
                    let travel = if samples_motion {
                        center - cx.read(params)?[0]
                    } else {
                        0.
                    };
                    cx.set(params, [center, travel, 0., 0.])?;
                    clock.set(Some(playback));
                } else {
                    clock.set(Some(Playback {
                        last: now,
                        phase: (1. - 2. * f64::from(cx.read(params)?[0])).acos(),
                        interval: None,
                    }));
                }
                Ok(())
            })
    });
    current_platform()?.run_window(
        WindowOptions::new("Solor · CPU shader")
            .app_id("org.solor.CpuShader")
            .size(px(560.), px(320.)),
        Ui::new(),
        view,
    )
}
