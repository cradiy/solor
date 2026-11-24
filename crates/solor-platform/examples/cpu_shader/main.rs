//! Drag a WGSL light band or play it continuously on the CPU.
mod surface;

use anyhow::{Context, Result};
use clap::{Arg, Command, value_parser};
use solor::EffectShader;
use solor::{SharedString, Styled, Ui, component, div, pointer::PointerEvent, px, rgb, text};
use solor_platform::{WindowOptions, current_platform};
use std::{cell::Cell, path::PathBuf, rc::Rc};

fn pointer_position(event: &PointerEvent) -> f32 {
    let width = event.content_size().width.get();
    (event.content_position().x.get() / width.max(f32::EPSILON)).clamp(0., 1.)
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
    let view = component(move |cx| {
        let position = cx.signal(0.4);
        let playing = cx.signal(false);
        let label = cx.signal(SharedString::from("Play"));
        let owner = cx.signal(None);
        let clock = Rc::new(Cell::new(None));
        let reset_clock = clock.clone();
        let surface = surface::shader_surface(shader, position, footprint)
            .on_pointer_down(move |event, cx| {
                if event.is_primary_action() && cx.read(owner)?.is_none() {
                    cx.capture_pointer();
                    cx.set(owner, Some(event.pointer_id()))?;
                    cx.set(playing, false)?;
                    cx.set(label, "Play".into())?;
                    cx.set(position, pointer_position(event))?;
                }
                Ok(())
            })
            .on_pointer_move(move |event, cx| {
                if *cx.read(owner)? == Some(event.pointer_id()) {
                    cx.set(position, pointer_position(event))?;
                }
                Ok(())
            })
            .on_pointer_up(move |event, cx| {
                if event.is_primary_action() && *cx.read(owner)? == Some(event.pointer_id()) {
                    cx.set(owner, None)?;
                }
                Ok(())
            })
            .on_pointer_cancel(move |id, cx| {
                if *cx.read(owner)? == Some(id) {
                    cx.set(owner, None)?;
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
                                cx.set(playing, next)?;
                                cx.set(label, if next { "Pause" } else { "Play" }.into())
                            }),
                    ),
            )
            .child(surface)
            .on_frame_when(playing, move |now, cx| {
                let (start, phase) = match clock.get() {
                    Some(anchor) => anchor,
                    None => {
                        let phase = (1.0_f32 - 2. * *cx.read(position)?).acos();
                        clock.set(Some((now, phase)));
                        return Ok(());
                    }
                };
                let elapsed = now.saturating_sub(start).as_secs_f32();
                cx.set(position, 0.5 - 0.5 * (phase + elapsed * 0.9).cos())?;
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
