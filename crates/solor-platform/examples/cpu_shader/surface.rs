use solor::{Div, EffectShader, Signal, Styled, div, layout::Size};

pub(super) const SWEEP: &str = include_str!("sweep.wgsl");
pub(super) const ROUNDED_RECT: &str = include_str!("rounded_rect.wgsl");
pub(super) const SPOTLIGHT: &str = include_str!("spotlight.wgsl");

#[derive(Clone, Copy)]
pub(super) enum Footprint {
    Sweep,
    Centered { half_width: f32, half_height: f32 },
}

impl Footprint {
    pub(super) fn for_source(source: &str) -> Option<Self> {
        match source {
            SWEEP => Some(Self::Sweep),
            SPOTLIGHT => Some(Self::Centered {
                half_width: 96.,
                half_height: 96.,
            }),
            // The 1.5 px rim extends past the 72 x 44 half-size.
            ROUNDED_RECT => Some(Self::Centered {
                half_width: 73.5,
                half_height: 45.5,
            }),
            _ => None,
        }
    }

    fn bounds(self, p: f32, size: Size) -> std::ops::Range<[f32; 2]> {
        match self {
            // Radius 0.16 plus a maximum tilt of 0.06 in sweep.wgsl.
            Self::Sweep => [p - 0.22, 0.]..[p + 0.22, 1.],
            Self::Centered {
                half_width,
                half_height,
            } => {
                let rx = half_width / size.width.get();
                let ry = half_height / size.height.get();
                [p - rx, 0.5 - ry]..[p + rx, 0.5 + ry]
            }
        }
    }
}

pub(super) fn shader_surface(
    shader: EffectShader,
    position: Signal<f32>,
    footprint: Option<Footprint>,
) -> Div {
    let Some(footprint) = footprint else {
        return div().flex_1().bg(shader.with_params(position));
    };
    div().flex_1().paint(move |cx| {
        if cx.size.width.get() > 0. && cx.size.height.get() > 0. {
            let p = *cx.read(position)?;
            cx.fill(
                cx.size,
                shader
                    .with_params(p)
                    .changing_region(footprint.bounds(p, cx.size)),
            );
        }
        Ok(())
    })
}
