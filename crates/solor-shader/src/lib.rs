//! WGSL compilation and scalar instructions shared by Solor renderers.
use anyhow::{Context, Result, anyhow, bail, ensure};
use naga::{
    BinaryOperator as Binary, Expression as Expr, Handle, Literal, MathFunction as Math, Scalar,
    Statement, TypeInner, UnaryOperator, VectorSize,
    valid::{Capabilities, ValidationFlags, Validator},
};

type Reg = usize;

/// One scalar instruction. Operands index earlier instruction results.
#[derive(Clone, Copy, Debug)]
pub enum Op {
    /// Normalized horizontal coordinate.
    U,
    /// Normalized vertical coordinate.
    V,
    /// Component of the logical fill size in pixels.
    Size(usize),
    /// Component of the draw parameters.
    Param(usize),
    /// Literal float.
    Constant(f32),
    /// Sum.
    Add(Reg, Reg),
    /// Difference.
    Sub(Reg, Reg),
    /// Product.
    Mul(Reg, Reg),
    /// Quotient.
    Div(Reg, Reg),
    /// Arithmetic negation.
    Neg(Reg),
    /// Smaller operand.
    Min(Reg, Reg),
    /// Larger operand.
    Max(Reg, Reg),
    /// Value limited to the given lower and upper bounds.
    Clamp(Reg, Reg, Reg),
    /// Absolute value.
    Abs(Reg),
    /// Square root.
    Sqrt(Reg),
    /// Linear interpolation.
    Mix(Reg, Reg, Reg),
    /// Smooth Hermite interpolation.
    SmoothStep(Reg, Reg, Reg),
}

#[derive(Debug)]
struct CoordinateAxes(u8);

impl CoordinateAxes {
    const X: u8 = 1;
    const Y: u8 = 2;

    fn for_output(ops: &[Op], rgba: [Reg; 4]) -> Self {
        let mut axes = Vec::with_capacity(ops.len());
        for op in ops {
            let mask = match *op {
                Op::U => Self::X,
                Op::V => Self::Y,
                Op::Size(_) | Op::Param(_) | Op::Constant(_) => 0,
                Op::Neg(a) | Op::Abs(a) | Op::Sqrt(a) => axes[a],
                Op::Add(a, b)
                | Op::Sub(a, b)
                | Op::Mul(a, b)
                | Op::Div(a, b)
                | Op::Min(a, b)
                | Op::Max(a, b) => axes[a] | axes[b],
                Op::Clamp(a, b, c) | Op::Mix(a, b, c) | Op::SmoothStep(a, b, c) => {
                    axes[a] | axes[b] | axes[c]
                }
            };
            axes.push(mask);
        }
        Self(rgba.into_iter().fold(0, |mask, r| mask | axes[r]))
    }
}

// WGSL length and distance each reduce all vector components to a single float.
fn magnitude(ops: &mut Vec<Op>, a: &[Reg], b: Option<&[Reg]>) -> Reg {
    fn emit(ops: &mut Vec<Op>, op: Op) -> Reg {
        let reg = ops.len();
        ops.push(op);
        reg
    }
    let delta = |i: usize, ops: &mut Vec<Op>| match b {
        Some(other) => emit(ops, Op::Sub(a[i], other[i])),
        None => a[i],
    };
    let first = delta(0, ops);
    if a.len() == 1 {
        return emit(ops, Op::Abs(first));
    }
    let mut sum = emit(ops, Op::Mul(first, first));
    for i in 1..a.len() {
        let d = delta(i, ops);
        let square = emit(ops, Op::Mul(d, d));
        sum = emit(ops, Op::Add(sum, square));
    }
    emit(ops, Op::Sqrt(sum))
}

/// A validated, straight-line WGSL fragment effect.
#[derive(Debug)]
pub struct Program {
    source: Box<str>,
    ops: Vec<Op>,
    axes: CoordinateAxes,
    rgba: [Reg; 4],
}
impl Program {
    /// Compiles `effect(uv: vec2<f32>, params: vec4<f32>) -> vec4<f32>`.
    /// A scalar second argument reads the first parameter. An optional third
    /// `vec2<f32>` receives the logical fill size in pixels.
    pub fn compile(source: &str) -> Result<Self> {
        let module = naga::front::wgsl::parse_str(source)
            .map_err(|err| anyhow!(err.emit_to_string(source)))?;
        let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
            .validate(&module)
            .map_err(|err| anyhow!(err.emit_to_string(source)))?;
        let (id, function) = module
            .functions
            .iter()
            .find(|(_, f)| f.name.as_deref() == Some("effect"))
            .context("missing effect function")?;
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
        let mut result = None;
        for (statement, span) in function.body.span_iter() {
            match statement {
                Statement::Emit(_) => (),
                Statement::Return { value: Some(value) } => result = Some(*value),
                _ => bail!(
                    "line {}: effect requires straight-line expressions",
                    span.location(source).line_number
                ),
            }
        }

        let mut ops = Vec::new();
        let mut values: Vec<Vec<Reg>> = Vec::new();
        for (handle, expr) in function.expressions.iter() {
            let loc = function.expressions.get_span(handle).location(source);
            let unsupported = || {
                anyhow!(
                    "{}:{}: unsupported effect expression {expr:?}",
                    loc.line_number,
                    loc.line_position
                )
            };
            let width = match info[id][handle].ty.inner_with(&module.types) {
                TypeInner::Scalar(Scalar::F32) => 1,
                TypeInner::Vector {
                    size,
                    scalar: Scalar::F32,
                } => *size as usize,
                _ => return Err(unsupported()),
            };
            let registers = |h: Handle<Expr>| &values[h.index()];
            let lowered = match *expr {
                Expr::Math {
                    fun: Math::Length | Math::Distance,
                    arg,
                    arg1,
                    ..
                } => Some(vec![magnitude(
                    &mut ops,
                    registers(arg),
                    arg1.map(|handle| registers(handle).as_slice()),
                )]),
                Expr::Compose { ref components, .. } => Some(
                    components
                        .iter()
                        .flat_map(|&h| registers(h))
                        .copied()
                        .collect(),
                ),
                Expr::AccessIndex { base, index } => Some(vec![registers(base)[index as usize]]),
                Expr::Splat { value, .. } => Some(vec![registers(value)[0]; width]),
                Expr::Swizzle {
                    vector, pattern, ..
                } => Some(
                    pattern[..width]
                        .iter()
                        .map(|c| registers(vector)[*c as usize])
                        .collect(),
                ),
                _ => None,
            };
            if let Some(output) = lowered {
                values.push(output);
                continue;
            }
            let lane = |h, i| {
                let regs = registers(h);
                regs[if regs.len() == 1 { 0 } else { i }]
            };
            let mut output = Vec::with_capacity(width);
            for i in 0..width {
                let op = match *expr {
                    Expr::FunctionArgument(0) => {
                        if i == 0 {
                            Op::U
                        } else {
                            Op::V
                        }
                    }
                    Expr::FunctionArgument(1) => Op::Param(i),
                    Expr::FunctionArgument(2) => Op::Size(i),
                    Expr::Literal(Literal::F32(v)) => Op::Constant(v),
                    Expr::Unary {
                        op: UnaryOperator::Negate,
                        expr,
                    } => Op::Neg(lane(expr, i)),
                    Expr::Binary { op, left, right } => {
                        let (a, b) = (lane(left, i), lane(right, i));
                        match op {
                            Binary::Add => Op::Add(a, b),
                            Binary::Subtract => Op::Sub(a, b),
                            Binary::Multiply => Op::Mul(a, b),
                            Binary::Divide => Op::Div(a, b),
                            _ => return Err(unsupported()),
                        }
                    }
                    Expr::Math {
                        fun: Math::Abs,
                        arg,
                        ..
                    } => Op::Abs(lane(arg, i)),
                    Expr::Math {
                        fun: fun @ (Math::Min | Math::Max),
                        arg,
                        arg1: Some(b),
                        ..
                    } => {
                        let (a, b) = (lane(arg, i), lane(b, i));
                        if fun == Math::Min {
                            Op::Min(a, b)
                        } else {
                            Op::Max(a, b)
                        }
                    }
                    Expr::Math {
                        fun,
                        arg,
                        arg1: Some(b),
                        arg2: Some(c),
                        ..
                    } => {
                        let (a, b, c) = (lane(arg, i), lane(b, i), lane(c, i));
                        match fun {
                            Math::Clamp => Op::Clamp(a, b, c),
                            Math::Mix => Op::Mix(a, b, c),
                            Math::SmoothStep => Op::SmoothStep(a, b, c),
                            _ => return Err(unsupported()),
                        }
                    }
                    _ => return Err(unsupported()),
                };
                output.push(ops.len());
                ops.push(op);
            }
            values.push(output);
        }
        let result = result.context("missing effect return value")?;
        let rgba = values[result.index()]
            .as_slice()
            .try_into()
            .context("expected RGBA output")?;
        let axes = CoordinateAxes::for_output(&ops, rgba);
        Ok(Self {
            source: source.into(),
            ops,
            axes,
            rgba,
        })
    }

    /// Canonical WGSL source.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Scalar operations in dependency order.
    pub fn instructions(&self) -> &[Op] {
        &self.ops
    }

    /// Registers containing straight-alpha RGBA output.
    pub fn outputs(&self) -> [usize; 4] {
        self.rgba
    }

    /// Independent sample counts along each axis for an output of this size.
    pub fn sample_extent(&self, width: u32, height: u32) -> [u32; 2] {
        [
            if self.axes.0 & CoordinateAxes::X == 0 {
                1
            } else {
                width
            },
            if self.axes.0 & CoordinateAxes::Y == 0 {
                1
            } else {
                height
            },
        ]
    }
}
