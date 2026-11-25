use solor_shader::{Op, Program};
use std::fmt::Write;

// Locals are generated from validated scalar IR, independent of user function names and signatures.
pub(super) fn body(program: &Program) -> String {
    let mut code = String::new();
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
            Op::Min(a, b) => format!("min(r{a},r{b})"),
            Op::Max(a, b) => format!("max(r{a},r{b})"),
            Op::Clamp(a, b, c) => format!("clamp(r{a},r{b},r{c})"),
            Op::Mix(a, b, c) => format!("mix(r{a},r{b},r{c})"),
            Op::SmoothStep(a, b, c) => format!("smoothstep(r{a},r{b},r{c})"),
        };
        writeln!(code, "let r{i}: f32 = {expr};").unwrap();
    }
    let [r, g, b, a] = program.outputs();
    write!(code, "return vec4f(r{r},r{g},r{b},r{a});").unwrap();
    code
}
