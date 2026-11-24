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
    /// The effect can call straight-line WGSL helpers with float or vector parameters and results.
    /// Supported operations include float and vector arithmetic, swizzles,
    /// `abs`, `min`, `max`, `clamp`, `length`, `distance`, `sin`, `cos`, `mix` and `smoothstep`.
    pub fn wgsl(source: &str) -> anyhow::Result<Self> {
        Ok(Self(Arc::new(solor_shader::Program::compile(source)?)))
    }

    /// Binds a float or four floats for one draw. Style setters also accept a signal of either type.
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

impl From<ShaderFill<f32>> for ShaderFill {
    fn from(fill: ShaderFill<f32>) -> Self {
        Self {
            shader: fill.shader,
            params: [fill.params, 0., 0., 0.],
            region: fill.region,
        }
    }
}

impl<P> From<ShaderFill<P>> for crate::Ink
where
    ShaderFill<P>: Into<ShaderFill>,
{
    fn from(fill: ShaderFill<P>) -> Self {
        Self::Shader(fill.into())
    }
}

impl<P> From<ShaderFill<P>> for crate::Fill
where
    ShaderFill<P>: Into<ShaderFill>,
{
    fn from(fill: ShaderFill<P>) -> Self {
        Self::Shader(fill.into())
    }
}

impl<T: Copy + 'static> ShaderFill<crate::Signal<T>> {
    pub(crate) fn sample(&self, cx: &crate::PaintContext<'_>) -> crate::Result<crate::Ink>
    where
        ShaderFill<T>: Into<crate::Ink>,
    {
        Ok(ShaderFill {
            shader: self.shader.clone(),
            params: *cx.read(self.params)?,
            region: self.region.clone(),
        }
        .into())
    }
}

impl<T: Clone + 'static, P: From<ShaderFill<T>> + 'static> From<ShaderFill<crate::Signal<T>>>
    for crate::style::Property<P>
where
    crate::style::Property<T>: From<crate::Signal<T>>,
{
    fn from(fill: ShaderFill<crate::Signal<T>>) -> Self {
        crate::style::Property::<T>::from(fill.params).map(move |params| {
            P::from(ShaderFill {
                shader: fill.shader.clone(),
                params,
                region: fill.region.clone(),
            })
        })
    }
}
