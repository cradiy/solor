//! WGSL fragment effects for Solor renderers.
#[cfg(feature = "cpu")]
mod cpu;

#[cfg(feature = "cpu")]
pub use cpu::{CpuProgram, Op};
#[cfg(feature = "cpu")]
use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow, ensure};
use naga::{
    Handle, Scalar, TypeInner, VectorSize,
    valid::{Capabilities, ValidationFlags, Validator},
};

/// A validated WGSL fragment effect.
#[derive(Debug)]
pub struct Program {
    source: Box<str>,
    scalar_parameter: bool,
    takes_size: bool,
    #[cfg(feature = "cpu")]
    cpu: OnceLock<Result<CpuProgram, String>>,
}

impl Program {
    /// Compiles `effect(uv: vec2<f32>, params: vec4<f32>) -> vec4<f32>`.
    /// A scalar second argument reads the first parameter. An optional third
    /// `vec2<f32>` receives the logical fill size in pixels.
    pub fn compile(source: &str) -> Result<Self> {
        let (module, info) = parse(source)?;
        let (id, function) = effect(&module)?;
        let uv = TypeInner::Vector {
            size: VectorSize::Bi,
            scalar: Scalar::F32,
        };
        let color = TypeInner::Vector {
            size: VectorSize::Quad,
            scalar: Scalar::F32,
        };
        let args = &function.arguments;
        let valid_args = match args.as_slice() {
            [uv_arg, params, rest @ ..] => {
                let param_type = &module.types[params.ty].inner;
                module.types[uv_arg.ty].inner == uv
                    && (*param_type == TypeInner::Scalar(Scalar::F32) || *param_type == color)
                    && (rest.is_empty()
                        || matches!(rest, [size] if module.types[size.ty].inner == uv))
            }
            _ => false,
        };
        ensure!(
            valid_args
                && function
                    .result
                    .as_ref()
                    .is_some_and(|r| module.types[r.ty].inner == color),
            "expected effect(uv: vec2<f32>, params: f32 or vec4<f32>[, size: vec2<f32>]) -> vec4<f32>"
        );
        ensure!(
            info[id]
                .available_stages
                .contains(naga::valid::ShaderStages::FRAGMENT),
            "effect must be callable in the fragment stage"
        );
        for (handle, global) in module.global_variables.iter() {
            ensure!(
                global.binding.is_none() || info[id][handle].is_empty(),
                "effect uses an unbound resource: {}",
                global.name.as_deref().unwrap_or("unnamed")
            );
        }
        Ok(Self {
            source: source.into(),
            scalar_parameter: module.types[args[1].ty].inner == TypeInner::Scalar(Scalar::F32),
            takes_size: args.len() == 3,
            #[cfg(feature = "cpu")]
            cpu: OnceLock::new(),
        })
    }

    /// Original WGSL module.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Calls the effect with the arguments its WGSL signature declares.
    pub fn wgsl_call(&self, uv: &str, params: &str, size: &str) -> String {
        let channel = if self.scalar_parameter { ".x" } else { "" };
        if self.takes_size {
            format!("effect({uv}, {params}{channel}, {size})")
        } else {
            format!("effect({uv}, {params}{channel})")
        }
    }

    /// Lowers the effect on first CPU use and shares the result across draws.
    #[cfg(feature = "cpu")]
    pub fn cpu(&self) -> Result<&CpuProgram> {
        self.cpu
            .get_or_init(|| CpuProgram::compile(&self.source).map_err(|e| format!("{e:#}")))
            .as_ref()
            .map_err(|e| anyhow!("CPU renderer cannot execute this effect: {e}"))
    }
}

fn parse(source: &str) -> Result<(naga::Module, naga::valid::ModuleInfo)> {
    let module =
        naga::front::wgsl::parse_str(source).map_err(|err| anyhow!(err.emit_to_string(source)))?;
    let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .map_err(|err| anyhow!(err.emit_to_string(source)))?;
    Ok((module, info))
}

fn effect(module: &naga::Module) -> Result<(Handle<naga::Function>, &naga::Function)> {
    module
        .functions
        .iter()
        .find(|(_, f)| f.name.as_deref() == Some("effect"))
        .context("missing effect function")
}
