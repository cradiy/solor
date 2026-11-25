//! Animated glass and translucent fills in a single paint stack.
use anyhow::Result;
use solor::{
    EffectShader, Styled, Ui, color_stop, component, div,
    layout::{Origin, Size},
    linear_gradient, px, rgb, rgba,
};
use solor_platform::{WindowOptions, current_platform};

fn main() -> Result<()> {
    let lens = EffectShader::wgsl(include_str!("shaders/pearl.wgsl"))?;
    let ramp = linear_gradient(
        118.,
        [
            color_stop(rgb(0x23374b), 0.),
            color_stop(rgb(0x537481), 0.47),
            color_stop(rgb(0x182936), 1.),
        ],
    )?;
    let scene = component(move |cx| {
        let phase = cx.signal(0.5_f32);
        let mut start = None;
        div()
            .w_full()
            .h_full()
            .paint(move |cx| {
                let phase = *cx.read(phase)?;
                let w = cx.size.width.get();
                let h = cx.size.height.get();
                let inset = w.min(h) * 0.065;
                cx.fill(cx.size, ramp.clone());
                cx.fill_rounded_rect(
                    Origin {
                        x: px(inset),
                        y: px(inset),
                    },
                    Size {
                        width: px(w - 2. * inset),
                        height: px(h - 2. * inset),
                    },
                    px(inset * 0.72),
                    lens.with_params(phase),
                )?;
                cx.fill_rounded_rect(
                    Origin {
                        x: px(0.13 * w),
                        y: px(0.72 * h),
                    },
                    Size {
                        width: px(w * 0.38),
                        height: px(h * 0.12),
                    },
                    px(inset * 0.35),
                    rgba(0xaadace46),
                )
            })
            .on_frame(move |now, cx| {
                let elapsed = now.saturating_sub(*start.get_or_insert(now));
                cx.set(phase, 0.5 + 0.5 * (elapsed.as_secs_f32() * 0.7).sin())
            })
    });
    current_platform()?.run_window(
        WindowOptions::new("Solor · Glass")
            .app_id("org.solor.GpuFills")
            .size(px(640.), px(440.)),
        Ui::new(),
        scene,
    )
}
