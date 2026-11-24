//! Shareable fragment effects and their per-draw parameters.
use std::{ops::Range, sync::Arc};

/// A compiled WGSL effect shared by all fills created from this handle.
#[derive(Clone, Debug)]
pub struct EffectShader(pub(crate) Arc<solor_shader::Program>);

impl PartialEq for EffectShader {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl EffectShader {
    /// Compiles a straight-line `effect(uv: vec2<f32>, params: vec4<f32>) -> vec4<f32>`.
    /// UV runs from the top left to the bottom right of the fill. Return straight sRGB and alpha.
    /// The second argument may also be a single float. Declare an optional third
    /// `size: vec2<f32>` argument to read the full fill dimensions in logical pixels.
    /// Supported operations include float and vector arithmetic, swizzles,
    /// `abs`, `min`, `max`, `clamp`, `length`, `distance`, `mix` and `smoothstep`.
    pub fn wgsl(source: &str) -> anyhow::Result<Self> {
        Ok(Self(Arc::new(solor_shader::Program::compile(source)?)))
    }

    /// Binds a float or four floats for one draw. Backgrounds also accept signals of either type.
    pub fn with_params<P>(&self, params: P) -> ShaderFill<P> {
        ShaderFill {
            shader: self.clone(),
            params,
            region: None,
        }
    }
}

/// An effect and its parameters for a single fill.
#[derive(Clone, Debug, PartialEq)]
pub struct ShaderFill<P = [f32; 4]> {
    /// Shared fragment program.
    pub shader: EffectShader,
    /// Values passed to the effect function.
    pub params: P,
    region: Option<Range<[f32; 2]>>,
}

impl<P> ShaderFill<P> {
    /// Identifies the UV rectangle containing all parameter-dependent pixels at this value.
    /// When parameters change, both the previous and current rectangles are repainted.
    pub fn changing_region(mut self, region: Range<[f32; 2]>) -> Self {
        self.region = Some(region);
        self
    }

    /// Region declared in normalized fill coordinates, or the whole fill when absent.
    pub fn region(&self) -> Option<&Range<[f32; 2]>> {
        self.region.as_ref()
    }
}

impl From<ShaderFill> for crate::Fill {
    fn from(fill: ShaderFill) -> Self {
        Self::Shader(fill)
    }
}

impl From<ShaderFill<f32>> for crate::Fill {
    fn from(fill: ShaderFill<f32>) -> Self {
        Self::Shader(ShaderFill {
            params: [fill.params, 0., 0., 0.],
            shader: fill.shader,
            region: fill.region,
        })
    }
}

impl<T: Clone + 'static> From<ShaderFill<crate::Signal<T>>>
    for crate::style::Property<crate::style::Background>
where
    crate::style::Property<T>: From<crate::Signal<T>>,
    ShaderFill<T>: Into<crate::Fill>,
{
    fn from(fill: ShaderFill<crate::Signal<T>>) -> Self {
        crate::style::Property::<T>::from(fill.params).map(move |params| {
            crate::style::Background::Fixed(
                ShaderFill {
                    shader: fill.shader.clone(),
                    params,
                    region: fill.region.clone(),
                }
                .into(),
            )
        })
    }
}
