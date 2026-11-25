use super::{Expr, Lowerer, Op, Reg};
use anyhow::{Context, Result, bail, ensure};
use naga::{Block, Function, Handle, Scalar, Statement, TypeInner};

struct Invocation {
    id: Handle<Function>,
    depth: usize,
    values: Vec<Vec<Reg>>,
    locals: Vec<Vec<Reg>>,
    returns: Vec<(Option<Reg>, Vec<Reg>)>,
}

impl Lowerer<'_> {
    pub(super) fn function(
        &mut self,
        id: Handle<Function>,
        args: Option<&[&[Reg]]>,
        depth: usize,
    ) -> Result<Vec<Reg>> {
        let function = &self.module.functions[id];
        // Repeated helper calls can expand a small call graph exponentially.
        if args.is_some() {
            self.helper_expressions += function.expressions.len().max(1);
            let line = self
                .module
                .functions
                .get_span(id)
                .location(self.source)
                .line_number;
            ensure!(depth <= 64, "line {line}: helper calls exceed 64 levels");
            ensure!(
                self.helper_expressions <= 65_536,
                "line {line}: inlined helpers exceed 65536 expressions"
            );
        }
        let mut invocation = Invocation {
            id,
            depth,
            values: vec![Vec::new(); function.expressions.len()],
            locals: vec![Vec::new(); function.local_variables.len()],
            returns: Vec::new(),
        };
        for (handle, expr) in function.expressions.iter() {
            if !expr.needs_pre_emit() || matches!(expr, Expr::LocalVariable(_)) {
                continue;
            }
            invocation.values[handle.index()] = match (expr, args) {
                (Expr::FunctionArgument(i), Some(args)) => args[*i as usize].to_vec(),
                _ => self.expression(id, handle, &invocation.values)?,
            };
        }
        self.block(&mut invocation, &function.body, None)?;

        // The first reached return wins. Its registers snapshot the value before later stores.
        let mut returns = invocation.returns.into_iter().rev();
        let (_, mut result) = returns.next().context("missing effect return value")?;
        for (guard, value) in returns {
            self.assign(&mut result, &value, guard);
        }
        Ok(result)
    }

    fn assign(&mut self, destination: &mut [Reg], value: &[Reg], guard: Option<Reg>) {
        for (dest, &src) in destination.iter_mut().zip(value) {
            if *dest != src {
                *dest = guard.map_or(src, |g| self.emit(Op::Select(*dest, src, g)));
            }
        }
    }

    fn block(
        &mut self,
        invocation: &mut Invocation,
        block: &Block,
        guard: Option<Reg>,
    ) -> Result<()> {
        let function = &self.module.functions[invocation.id];
        for (statement, span) in block.span_iter() {
            match statement {
                Statement::Emit(range) => {
                    for handle in range.clone() {
                        let expr = &function.expressions[handle];
                        let ty = self.info[invocation.id][handle]
                            .ty
                            .inner_with(&self.module.types);
                        if matches!(
                            ty,
                            TypeInner::ValuePointer { .. } | TypeInner::Pointer { .. }
                        ) {
                            continue;
                        }
                        invocation.values[handle.index()] = match *expr {
                            Expr::Load { pointer } => self.local(invocation, pointer)?.to_vec(),
                            _ => self.expression(invocation.id, handle, &invocation.values)?,
                        };
                    }
                }
                Statement::Call {
                    function,
                    arguments,
                    result: Some(output),
                } => {
                    let args: Vec<_> = arguments
                        .iter()
                        .map(|h| invocation.values[h.index()].as_slice())
                        .collect();
                    ensure!(
                        args.iter().all(|a| !a.is_empty()),
                        "effect helpers require value arguments"
                    );
                    invocation.values[output.index()] =
                        self.function(*function, Some(&args), invocation.depth + 1)?;
                }
                Statement::Store { pointer, value } => {
                    let value = invocation.values[value.index()].clone();
                    let dst = self.local(invocation, *pointer)?;
                    self.assign(dst, &value, guard);
                }
                Statement::If {
                    condition,
                    accept,
                    reject,
                } => {
                    let test = invocation.values[condition.index()][0];
                    if !accept.is_empty() {
                        let pass = guard.map_or(test, |g| self.emit(Op::Min(g, test)));
                        self.block(invocation, accept, Some(pass))?;
                    }
                    if !reject.is_empty() {
                        let zero = self.emit(Op::Constant(0.));
                        let fail = self.emit(Op::Equal(test, zero));
                        let fail = guard.map_or(fail, |g| self.emit(Op::Min(g, fail)));
                        self.block(invocation, reject, Some(fail))?;
                    }
                }
                Statement::Block(inner) => self.block(invocation, inner, guard)?,
                Statement::Return { value: Some(value) } => {
                    invocation
                        .returns
                        .push((guard, invocation.values[value.index()].clone()));
                    break;
                }
                _ => bail!(
                    "line {}: unsupported effect statement",
                    span.location(self.source).line_number
                ),
            }
        }
        Ok(())
    }

    fn local<'a>(
        &mut self,
        invocation: &'a mut Invocation,
        pointer: Handle<Expr>,
    ) -> Result<&'a mut [Reg]> {
        let function = &self.module.functions[invocation.id];
        let (base, lane) = match function.expressions[pointer] {
            Expr::AccessIndex { base, index } => (base, Some(index as usize)),
            _ => (pointer, None),
        };
        let Expr::LocalVariable(id) = function.expressions[base] else {
            bail!("effect assignments and loads require a local variable or vector component");
        };
        let value = &mut invocation.locals[id.index()];
        if value.is_empty() {
            let var = &function.local_variables[id];
            let width = match self.module.types[var.ty].inner {
                TypeInner::Scalar(Scalar::BOOL | Scalar::F32) => 1,
                TypeInner::Vector {
                    size,
                    scalar: Scalar::BOOL | Scalar::F32,
                } => size as usize,
                _ => bail!("effect locals require float or boolean scalars or vectors"),
            };
            *value = match var.init {
                Some(h) => invocation.values[h.index()].clone(),
                None => vec![self.emit(Op::Constant(0.)); width],
            };
            ensure!(value.len() == width, "unsupported effect local initializer");
        }
        Ok(match lane {
            Some(i) => &mut value[i..=i],
            None => value,
        })
    }
}
