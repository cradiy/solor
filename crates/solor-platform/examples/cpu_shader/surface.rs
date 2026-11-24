use solor::{Div, EffectShader, Signal, Styled, div};

pub(super) fn shader_surface(
    shader: EffectShader,
    position: Signal<f32>,
    built_in_sweep: bool,
) -> Div {
    if !built_in_sweep {
        return div().flex_1().bg(shader.with_params(position));
    }
    div().flex_1().paint(move |cx| {
        let p = *cx.read(position)?;
        // Radius 0.16 plus a maximum tilt of 0.06 in sweep.wgsl.
        let fill = shader
            .with_params(p)
            .changing_region([p - 0.22, 0.]..[p + 0.22, 1.]);
        cx.fill(cx.size, fill);
        Ok(())
    })
}
