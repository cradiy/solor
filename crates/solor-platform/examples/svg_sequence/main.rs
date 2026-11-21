//! Play a loop of independently parsed SVG poses at up to 120 poses per second.
use anyhow::Result;
use solor::{
    Div, Render, SharedString, Styled, SvgAsset, Ui, component, div, percent, px, rgb, svg, text,
};
use solor_platform::{WindowOptions, current_platform};
use std::{cell::Cell, f64::consts::TAU, rc::Rc, time::Duration};

const POSES: usize = 32;
const RATE: u128 = 120;

fn poses() -> Result<Vec<SvgAsset>> {
    (0..POSES)
        .map(|i| {
            let phase = TAU * i as f64 / POSES as f64;
            let source = format!(
                include_str!("moth.svg.in"),
                span = 0.68 + 0.32 * phase.cos(),
                hind = 0.75 + 0.25 * (phase - 0.5).cos(),
                sweep = -4.0 + 7.0 * phase.sin(),
                trail = 7.0 * (phase - 0.5).sin(),
                lift = 2.5 * phase.sin()
            );
            SvgAsset::from_bytes(source.as_bytes())
        })
        .collect()
}

fn button() -> Div {
    div()
        .px(px(16.))
        .py(px(8.))
        .rounded_full()
        .bg(rgb(0xcee4aa))
        .text_color(rgb(0x21312b))
        .cursor_pointer()
}

fn sequence(assets: Vec<SvgAsset>) -> impl Render {
    component(move |cx| {
        let playing = cx.signal(true);
        let label = cx.signal(SharedString::from("Pause"));
        let fps = cx.signal(SharedString::from("— fps"));
        let visible = std::array::from_fn::<_, POSES, _>(|i| cx.signal(i == 0));
        let mut stage = div()
            .flex_1()
            .items_center()
            .justify_center()
            .rounded_xl()
            .bg(rgb(0x1c3738));
        for (asset, flag) in assets.into_iter().zip(visible) {
            stage = stage.child_when(flag, move || {
                svg(asset.clone())
                    .preserve_colors()
                    .w(px(288.))
                    .max_w(percent(100.0))
            });
        }

        let last = Rc::new(Cell::new(None));
        let clock = Rc::clone(&last);
        let mut elapsed = Duration::ZERO;
        let mut sample = Duration::ZERO;
        let mut intervals = 0;
        let mut current = 0;
        div()
            .items_stretch()
            .p(px(20.))
            .gap(px(12.))
            .bg(rgb(0x112626))
            .text_color(rgb(0xf3efdc))
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(text("Luna moth").text_size(px(27.)))
                    .child(text(fps).text_color(rgb(0xc5d4ba))),
            )
            .child(stage)
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child("32 SVG poses")
                    .child(button().child(text(label)).on_click(move |cx| {
                        let next = !*cx.read(playing)?;
                        clock.set(None);
                        cx.set(playing, next)?;
                        cx.set(label, if next { "Pause" } else { "Play" }.into())?;
                        cx.set(fps, if next { "— fps" } else { "Paused" }.into())
                    })),
            )
            .on_frame_when(playing, move |now, cx| {
                let Some(previous) = last.replace(Some(now)) else {
                    sample = Duration::ZERO;
                    intervals = 0;
                    return Ok(());
                };
                let delta = now.saturating_sub(previous);
                elapsed += delta;
                let index = ((elapsed.as_nanos() * RATE / 1_000_000_000) % POSES as u128) as usize;
                if current != index {
                    cx.set(visible[current], false)?;
                    cx.set(visible[index], true)?;
                    current = index;
                }
                sample += delta;
                intervals += 1;
                if sample >= Duration::from_secs(1) {
                    cx.set(
                        fps,
                        format!("{:.0} fps", f64::from(intervals) / sample.as_secs_f64()).into(),
                    )?;
                    sample = Duration::ZERO;
                    intervals = 0;
                }
                Ok(())
            })
    })
}

fn main() -> Result<()> {
    let window = WindowOptions::new("Solor · SVG sequence")
        .app_id("org.solor.SvgSequence")
        .size(px(456.), px(336.));
    current_platform()?.run_window(window, Ui::new(), sequence(poses()?))
}
