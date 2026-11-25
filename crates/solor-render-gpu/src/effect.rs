use solor_shader::{Op, Program};
use std::fmt::Write;

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

// Locals are generated from validated scalar IR, independent of user function names and signatures.
pub(super) fn paint(program: &Program, target: Target) -> String {
    let mut code = String::from(match target {
        Target::Fill | Target::Underline => {
            "fn paint(local: vec2f, size: vec2f, q: Quad) -> vec4f {\n"
        }
        Target::Mask => {
            "fn foreground(local: vec2f, size: vec2f, q: Quad, ramp: vec4f) -> vec4f {\n"
        }
    });
    code.push_str("let uv = clamp(local / size, vec2f(0), vec2f(1));\n");
    for (i, op) in program.instructions().iter().copied().enumerate() {
        let expr = match op {
            Op::U => "uv.x".into(),
            Op::V => "uv.y".into(),
            Op::Size(i) => format!("size[{i}]"),
            Op::Param(i) => format!("q.params[{i}]"),
            Op::Constant(x) => format!("{x:?}f"),
            Op::Abs(a) => format!("abs(r{a})"),
            Op::Floor(a) => format!("floor(r{a})"),
            Op::Fract(a) => format!("fract(r{a})"),
            Op::Sqrt(a) => format!("sqrt(r{a})"),
            Op::Pow(a, b) => format!("pow(r{a},r{b})"),
            Op::ZeroIfNegative(a, b) => format!("select(r{b},0.,r{a}<0.)"),
            Op::Sin(a) => format!("sin(r{a})"),
            Op::Cos(a) => format!("cos(r{a})"),
            Op::Neg(a) => format!("-r{a}"),
            Op::Add(a, b) => format!("r{a}+r{b}"),
            Op::Sub(a, b) => format!("r{a}-r{b}"),
            Op::Mul(a, b) => format!("r{a}*r{b}"),
            Op::Div(a, b) => format!("r{a}/r{b}"),
            Op::Equal(a, b) => format!("select(0.,1.,r{a}==r{b})"),
            Op::NotEqual(a, b) => format!("select(0.,1.,r{a}!=r{b})"),
            Op::Less(a, b) => format!("select(0.,1.,r{a}<r{b})"),
            Op::LessEqual(a, b) => format!("select(0.,1.,r{a}<=r{b})"),
            Op::Select(a, b, condition) => format!("select(r{a},r{b},r{condition}!=0.)"),
            Op::Min(a, b) => format!("min(r{a},r{b})"),
            Op::Max(a, b) => format!("max(r{a},r{b})"),
            Op::Clamp(a, b, c) => format!("clamp(r{a},r{b},r{c})"),
            Op::Mix(a, b, c) => format!("mix(r{a},r{b},r{c})"),
            Op::SmoothStep(a, b, c) => format!("smoothstep(r{a},r{b},r{c})"),
        };
        writeln!(code, "let r{i}: f32 = {expr};").unwrap();
    }
    let [r, g, b, a] = program.outputs();
    write!(code, "return vec4f(r{r},r{g},r{b},r{a}); }}").unwrap();
    if target == Target::Mask {
        code.push_str(include_str!("mask.wgsl"));
    }
    code
}
