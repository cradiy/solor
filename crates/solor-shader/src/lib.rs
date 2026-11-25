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
    /// One when the operands are equal, zero otherwise.
    Equal(Reg, Reg),
    /// One when the operands differ, zero otherwise.
    NotEqual(Reg, Reg),
    /// One when the first operand is smaller, zero otherwise.
    Less(Reg, Reg),
    /// One when the first operand is smaller than or equal to the second, zero otherwise.
    LessEqual(Reg, Reg),
    /// Selects the second operand when the third is true, otherwise the first.
    Select(Reg, Reg, Reg),
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
    /// Greatest integer less than or equal to the operand.
    Floor(Reg),
    /// Fractional part measured from the floor of the operand.
    Fract(Reg),
    /// Square root.
    Sqrt(Reg),
    /// Base raised to an exponent.
    Pow(Reg, Reg),
    /// Zero when the first operand is negative, otherwise the second operand.
    ZeroIfNegative(Reg, Reg),
    /// Sine of an angle in radians.
    Sin(Reg),
    /// Cosine of an angle in radians.
    Cos(Reg),
    /// Linear interpolation.
    Mix(Reg, Reg, Reg),
    /// Smooth Hermite interpolation.
    SmoothStep(Reg, Reg, Reg),
}

#[derive(Debug)]
struct CoordinateAxes {
    output: u8,
    masks: Vec<u8>,
    column: Vec<usize>,
    row: Vec<usize>,
}

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
                Op::Neg(a)
                | Op::Abs(a)
                | Op::Floor(a)
                | Op::Fract(a)
                | Op::Sqrt(a)
                | Op::Sin(a)
                | Op::Cos(a) => axes[a],
                Op::Add(a, b)
                | Op::Sub(a, b)
                | Op::Mul(a, b)
                | Op::Div(a, b)
                | Op::Pow(a, b)
                | Op::Min(a, b)
                | Op::Max(a, b)
                | Op::Equal(a, b)
                | Op::NotEqual(a, b)
                | Op::Less(a, b)
                | Op::LessEqual(a, b)
                | Op::ZeroIfNegative(a, b) => axes[a] | axes[b],
                Op::Clamp(a, b, c)
                | Op::Mix(a, b, c)
                | Op::SmoothStep(a, b, c)
                | Op::Select(a, b, c) => axes[a] | axes[b] | axes[c],
            };
            axes.push(mask);
        }
        let (column, row) = (0..ops.len()).partition(|&i| axes[i] & Self::Y == 0);
        Self {
            output: rgba.into_iter().fold(0, |mask, r| mask | axes[r]),
            masks: axes,
            column,
            row,
        }
    }
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
        let mut lowerer = Lowerer {
            module: &module,
            info: &info,
            source,
            ops: Vec::new(),
            helper_expressions: 0,
        };
        let rgba = lowerer
            .function(id, None, 0)?
            .try_into()
            .map_err(|_| anyhow!("expected RGBA output"))?;
        let ops = lowerer.ops;
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

    /// Instruction indices reusable down a column, followed by those evaluated for each row.
    pub fn column_plan(&self) -> (&[usize], &[usize]) {
        (&self.axes.column, &self.axes.row)
    }

    /// Whether this result can vary across a horizontal batch of samples.
    pub fn varies_across_row(&self, register: usize) -> bool {
        self.axes.masks[register] & CoordinateAxes::X != 0
    }

    /// Registers containing straight-alpha RGBA output.
    pub fn outputs(&self) -> [usize; 4] {
        self.rgba
    }

    /// Independent sample counts along each axis for an output of this size.
    pub fn sample_extent(&self, width: u32, height: u32) -> [u32; 2] {
        [
            if self.axes.output & CoordinateAxes::X == 0 {
                1
            } else {
                width
            },
            if self.axes.output & CoordinateAxes::Y == 0 {
                1
            } else {
                height
            },
        ]
    }
}

struct Lowerer<'a> {
    module: &'a naga::Module,
    info: &'a naga::valid::ModuleInfo,
    source: &'a str,
    ops: Vec<Op>,
    helper_expressions: usize,
}

impl Lowerer<'_> {
    fn emit(&mut self, op: Op) -> Reg {
        let reg = self.ops.len();
        self.ops.push(op);
        reg
    }

    fn dot(&mut self, a: &[Reg], b: &[Reg]) -> Reg {
        let mut sum = self.emit(Op::Mul(a[0], b[0]));
        for (&x, &y) in a[1..].iter().zip(&b[1..]) {
            let product = self.emit(Op::Mul(x, y));
            sum = self.emit(Op::Add(sum, product));
        }
        sum
    }

    fn magnitude(&mut self, value: &[Reg]) -> Reg {
        if let &[scalar] = value {
            return self.emit(Op::Abs(scalar));
        }
        let squared = self.dot(value, value);
        self.emit(Op::Sqrt(squared))
    }

    fn refract(&mut self, incident: &[Reg], normal: &[Reg], eta: Reg) -> Vec<Reg> {
        let projection = self.dot(normal, incident);
        let one = self.emit(Op::Constant(1.0));
        let cos_squared = self.emit(Op::Mul(projection, projection));
        let sin_squared = self.emit(Op::Sub(one, cos_squared));
        let eta_squared = self.emit(Op::Mul(eta, eta));
        let transmitted_sin_squared = self.emit(Op::Mul(eta_squared, sin_squared));
        let k = self.emit(Op::Sub(one, transmitted_sin_squared));
        let nonnegative_k = self.emit(Op::ZeroIfNegative(k, k));
        let transmitted_cos = self.emit(Op::Sqrt(nonnegative_k));
        let scaled_cos = self.emit(Op::Mul(eta, projection));
        let normal_scale = self.emit(Op::Add(scaled_cos, transmitted_cos));
        incident
            .iter()
            .zip(normal)
            .map(|(&i, &n)| {
                let ray = self.emit(Op::Mul(eta, i));
                let offset = self.emit(Op::Mul(normal_scale, n));
                let component = self.emit(Op::Sub(ray, offset));
                self.emit(Op::ZeroIfNegative(k, component))
            })
            .collect()
    }

    fn function(
        &mut self,
        id: Handle<naga::Function>,
        args: Option<&[&[Reg]]>,
        depth: usize,
    ) -> Result<Vec<Reg>> {
        let module = self.module;
        let function = &module.functions[id];
        // A short call graph can expand exponentially when helpers call each other repeatedly.
        if args.is_some() {
            self.helper_expressions += function.expressions.len().max(1);
            let line = || {
                module
                    .functions
                    .get_span(id)
                    .location(self.source)
                    .line_number
            };
            ensure!(
                depth <= 64,
                "line {}: helper calls exceed 64 levels",
                line()
            );
            ensure!(
                self.helper_expressions <= 65_536,
                "line {}: inlined helpers exceed 65536 expressions",
                line()
            );
        }
        let mut calls = std::collections::HashMap::new();
        let mut result = None;
        for (statement, span) in function.body.span_iter() {
            match statement {
                Statement::Emit(_) => (),
                Statement::Call {
                    arguments,
                    result: Some(output),
                    ..
                } => {
                    calls.insert(*output, arguments);
                }
                Statement::Return { value: Some(value) } => result = Some(*value),
                _ => bail!(
                    "line {}: effect requires straight-line expressions",
                    span.location(self.source).line_number
                ),
            }
        }

        let mut values: Vec<Vec<Reg>> = Vec::with_capacity(function.expressions.len());
        for (handle, expr) in function.expressions.iter() {
            let unsupported = || {
                let loc = function.expressions.get_span(handle).location(self.source);
                anyhow!(
                    "{}:{}: unsupported effect expression {expr:?}",
                    loc.line_number,
                    loc.line_position
                )
            };
            let width = match self.info[id][handle].ty.inner_with(&module.types) {
                TypeInner::Scalar(Scalar::F32 | Scalar::BOOL) => 1,
                TypeInner::Vector {
                    size,
                    scalar: Scalar::F32 | Scalar::BOOL,
                } => *size as usize,
                _ => return Err(unsupported()),
            };
            let registers = |h: Handle<Expr>| &values[h.index()];
            let lowered = match *expr {
                Expr::FunctionArgument(index) => {
                    args.map(|bindings| bindings[index as usize].to_vec())
                }
                Expr::CallResult(callee) => {
                    let arguments = calls.get(&handle).context("missing call arguments")?;
                    let bindings: Vec<_> =
                        arguments.iter().map(|&h| registers(h).as_slice()).collect();
                    Some(self.function(callee, Some(&bindings), depth + 1)?)
                }
                Expr::Math {
                    fun: Math::Length,
                    arg,
                    ..
                } => Some(vec![self.magnitude(registers(arg))]),
                Expr::Math {
                    fun: Math::Distance,
                    arg,
                    arg1: Some(other),
                    ..
                } => {
                    let a = registers(arg);
                    let mut delta = [0; 4];
                    for ((reg, &x), &y) in delta.iter_mut().zip(a).zip(registers(other)) {
                        *reg = self.emit(Op::Sub(x, y));
                    }
                    Some(vec![self.magnitude(&delta[..a.len()])])
                }
                Expr::Math {
                    fun: Math::Dot,
                    arg,
                    arg1: Some(other),
                    ..
                } => Some(vec![self.dot(registers(arg), registers(other))]),
                Expr::Math {
                    fun: Math::Reflect,
                    arg,
                    arg1: Some(normal),
                    ..
                } => {
                    let incident = registers(arg);
                    let normal = registers(normal);
                    let projection = self.dot(normal, incident);
                    let twice = self.emit(Op::Add(projection, projection));
                    Some(
                        incident
                            .iter()
                            .zip(normal)
                            .map(|(&i, &n)| {
                                let offset = self.emit(Op::Mul(twice, n));
                                self.emit(Op::Sub(i, offset))
                            })
                            .collect(),
                    )
                }
                Expr::Math {
                    fun: Math::Refract,
                    arg,
                    arg1: Some(normal),
                    arg2: Some(eta),
                    ..
                } => Some(self.refract(registers(arg), registers(normal), registers(eta)[0])),
                Expr::Math {
                    fun: Math::Normalize,
                    arg,
                    ..
                } => {
                    let value = registers(arg);
                    let length = self.magnitude(value);
                    Some(
                        value
                            .iter()
                            .map(|&x| self.emit(Op::Div(x, length)))
                            .collect(),
                    )
                }
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
                    Expr::Literal(Literal::Bool(v)) => Op::Constant(f32::from(v)),
                    Expr::Select {
                        condition,
                        accept,
                        reject,
                    } => Op::Select(lane(reject, i), lane(accept, i), lane(condition, i)),
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
                            Binary::Equal => Op::Equal(a, b),
                            Binary::NotEqual => Op::NotEqual(a, b),
                            Binary::Less => Op::Less(a, b),
                            Binary::LessEqual => Op::LessEqual(a, b),
                            Binary::Greater => Op::Less(b, a),
                            Binary::GreaterEqual => Op::LessEqual(b, a),
                            _ => return Err(unsupported()),
                        }
                    }
                    Expr::Math {
                        fun,
                        arg,
                        arg1: None,
                        ..
                    } => {
                        let a = lane(arg, i);
                        match fun {
                            Math::Abs => Op::Abs(a),
                            Math::Floor => Op::Floor(a),
                            Math::Fract => Op::Fract(a),
                            Math::Sqrt => Op::Sqrt(a),
                            Math::Sin => Op::Sin(a),
                            Math::Cos => Op::Cos(a),
                            _ => return Err(unsupported()),
                        }
                    }
                    Expr::Math {
                        fun,
                        arg,
                        arg1: Some(b),
                        arg2: None,
                        ..
                    } => {
                        let (a, b) = (lane(arg, i), lane(b, i));
                        match fun {
                            Math::Min => Op::Min(a, b),
                            Math::Max => Op::Max(a, b),
                            Math::Pow if matches!(self.ops[b], Op::Constant(2.0)) => Op::Mul(a, a),
                            Math::Pow => Op::Pow(a, b),
                            _ => return Err(unsupported()),
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
                output.push(self.emit(op));
            }
            values.push(output);
        }
        let result = result.context("missing effect return value")?;
        Ok(std::mem::take(&mut values[result.index()]))
    }
}
