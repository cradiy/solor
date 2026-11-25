use anyhow::{Result, ensure};
use solor_shader::Program;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Target {
    Fill,
    Underline,
    Mask,
}

pub(super) struct Pipeline {
    pub shader: solor::EffectShader,
    pub target: Target,
    pub pipeline: wgpu::RenderPipeline,
    pub used: bool,
}

impl Target {
    pub(super) fn module(self, body: &str) -> String {
        let fragment = match self {
            Self::Underline => include_str!("underline.wgsl"),
            Self::Fill | Self::Mask => include_str!("quad.wgsl"),
        };
        [include_str!("vertex.wgsl"), body, fragment].join("\n")
    }
}

pub(super) fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    program: &Program,
    target: Target,
) -> Result<wgpu::RenderPipeline> {
    let call = program.wgsl_call("uv", "q.params", "size");
    let (signature, mask) = match target {
        Target::Fill | Target::Underline => {
            ("solor_paint(local: vec2f, size: vec2f, q: solor_Quad)", "")
        }
        Target::Mask => (
            "solor_foreground(local: vec2f, size: vec2f, q: solor_Quad, ramp: vec4f)",
            include_str!("mask.wgsl"),
        ),
    };
    let paint = format!(
        "fn {signature} -> vec4f {{ let uv = clamp(local / size, vec2f(0), vec2f(1)); return {call}; }}\n{mask}"
    );
    let mut host = target.module(&paint);
    let mut prefix = String::from("solor_");
    while program.source().contains(&prefix) {
        prefix.push('x');
    }
    if prefix != "solor_" {
        host = host.replace("solor_", &prefix);
    }
    let source = [program.source(), &host].join("\n");
    let mut module = naga::front::wgsl::parse_str(&source)
        .map_err(|e| anyhow::anyhow!(e.emit_to_string(&source)))?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .map_err(|e| anyhow::anyhow!(e.emit_to_string(&source)))?;
    if module.overrides.iter().any(|(_, o)| o.init.is_none()) {
        let vertex = format!("{prefix}vertex");
        let fragment = format!("{prefix}fragment");
        module
            .entry_points
            .retain(|ep| ep.name == vertex || ep.name == fragment);
        naga::compact::compact(&mut module, naga::compact::KeepUnused::No);
        for (_, constant) in module.overrides.iter() {
            ensure!(
                constant.init.is_some(),
                "effect requires a value for override {}",
                constant.name.as_deref().unwrap_or("constant")
            );
        }
    }
    Ok(super::module_pipeline(
        device, layout, format, source, &prefix,
    ))
}
