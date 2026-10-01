use super::*;
use ast::{BinOp, Expr, UnOp};

impl Compiler<'_> {
    pub(super) fn load(
        &mut self,
        function: &mut Function,
        mut value: Value,
        span: &ast::Span,
    ) -> Result<Value> {
        if let Some(place) = value.place.take() {
            if !matches!(value.ty, Type::Pointer { .. })
                && self.contains_pointer(&value.ty, span)?
            {
                return Err(Error::new(
                    span.clone(),
                    "loading an aggregate containing pointers is not supported; project its fields first",
                ));
            }
            self.requirements.reads_device_memory |= place.physical;
            // Each load is appended at its use, never cached across stores or calls.
            value.expression = function.expr(
                Ex::Load {
                    pointer: value.expression,
                },
                true,
            );
        }
        Ok(value)
    }

    fn dereference(
        &mut self,
        function: &mut Function,
        value: Value,
        span: &ast::Span,
    ) -> Result<Value> {
        let value = self.load(function, value, span)?;
        let Type::Pointer { pointee, access } = value.ty else {
            return Err(Error::new(
                span.clone(),
                "dereference requires a device pointer",
            ));
        };
        Ok(Value {
            expression: value.expression,
            ty: *pointee,
            place: Some(Place {
                writable: access == Access::ReadWrite,
                physical: true,
            }),
        })
    }

    pub(super) fn value(&mut self, function: &mut Function, expr: &SExpr) -> Result<Value> {
        let value = self.expression(function, expr)?;
        self.load(function, value, &expr.span)
    }

    pub(super) fn expression(&mut self, function: &mut Function, expr: &SExpr) -> Result<Value> {
        let error = |message| Error::new(expr.span.clone(), message);
        match &expr.node {
            Expr::Var(name) => {
                if name.contains('.') {
                    let mut parts = name.split('.');
                    let mut projected = SExpr {
                        node: Expr::Var(parts.next().expect("nonempty path").into()),
                        span: expr.span.clone(),
                    };
                    for field in parts {
                        projected = SExpr {
                            node: Expr::Member(Box::new(projected), field.into()),
                            span: expr.span.clone(),
                        };
                    }
                    return self.expression(function, &projected);
                }
                if let Some(value) = function.variables.get(name) {
                    return Ok(value.clone());
                }
                if let Some((global, ty)) = self.globals.get(name) {
                    return Ok(Value {
                        expression: function.expr(Ex::GlobalVariable(*global), false),
                        ty: ty.clone(),
                        place: Some(Place {
                            writable: false,
                            physical: false,
                        }),
                    });
                }
                match name.as_str() {
                    "true" | "false" => Ok(Value {
                        expression: function
                            .expr(Ex::Literal(naga::Literal::Bool(name == "true")), false),
                        ty: Type::Bool,
                        place: None,
                    }),
                    _ => Err(error(format!("unknown native value `{name}`"))),
                }
            }
            Expr::Num(n, crate::lexer::Unit::None) => {
                if !n.is_finite() || !(*n as f32).is_finite() {
                    return Err(error("number exceeds finite f32 range".into()));
                }
                Ok(Value {
                    expression: function.expr(Ex::Literal(naga::Literal::F32(*n as f32)), false),
                    ty: Type::F32,
                    place: None,
                })
            }
            Expr::Member(base, field) => {
                let mut base = self.expression(function, base)?;
                if matches!(base.ty, Type::Pointer { .. }) {
                    base = self.dereference(function, base, &expr.span)?;
                }
                let layout = self.resolve(&base.ty, &expr.span)?.layout;
                let (index, member) = layout
                    .fields
                    .iter()
                    .enumerate()
                    .find(|(_, m)| &m.name == field)
                    .ok_or_else(|| error(format!("type {:?} has no field `{field}`", base.ty)))?;
                let index =
                    u32::try_from(index).map_err(|_| error("record has too many fields".into()))?;
                Ok(Value {
                    expression: function.expr(
                        Ex::AccessIndex {
                            base: base.expression,
                            index,
                        },
                        true,
                    ),
                    ty: member.ty.clone(),
                    place: base.place,
                })
            }
            Expr::Index { array, index } => {
                let base = self.expression(function, array)?;
                if matches!(base.ty, Type::Pointer { .. }) {
                    return Err(error("raw pointer indexing requires element-address support; use (*p)[i] for a pointer to a fixed array".into()));
                }
                let Type::Array(element, count) = base.ty else {
                    return Err(error("indexing requires an array".into()));
                };
                let constant = match index.node {
                    Expr::Num(n, crate::lexer::Unit::None)
                        if n >= 0.0 && n.fract() == 0.0 && n <= f64::from(u32::MAX) =>
                    {
                        Some(n as u32)
                    }
                    _ => None,
                };
                let expression = if let Some(index) = constant {
                    if index >= count {
                        return Err(error("constant array index is out of bounds".into()));
                    }
                    function.expr(
                        Ex::AccessIndex {
                            base: base.expression,
                            index,
                        },
                        true,
                    )
                } else {
                    let index = self.value(function, index)?;
                    if !matches!(index.ty, Type::U32 | Type::I32) {
                        return Err(error("array index must be an integer".into()));
                    }
                    function.expr(
                        Ex::Access {
                            base: base.expression,
                            index: index.expression,
                        },
                        true,
                    )
                };
                Ok(Value {
                    expression,
                    ty: *element,
                    place: base.place,
                })
            }
            Expr::Unary(UnOp::Deref, inner) => {
                let value = self.expression(function, inner)?;
                self.dereference(function, value, &expr.span)
            }
            Expr::Unary(UnOp::AddressOf, inner) => {
                let value = self.expression(function, inner)?;
                let Some(Place {
                    writable,
                    physical: true,
                }) = value.place
                else {
                    return Err(error("address-of requires a physical device memory location; locals and descriptor roots are not device pointers".into()));
                };
                let ty = Type::Pointer {
                    pointee: Box::new(value.ty),
                    access: if writable {
                        Access::ReadWrite
                    } else {
                        Access::Read
                    },
                };
                self.resolve(&ty, &expr.span)?;
                Ok(Value {
                    expression: value.expression,
                    ty,
                    place: None,
                })
            }
            Expr::Unary(UnOp::Neg, inner) => {
                let value = self.value(function, inner)?;
                if !matches!(value.ty, Type::I32 | Type::F32) {
                    return Err(error("negation requires i32 or f32".into()));
                }
                Ok(Value {
                    expression: function.expr(
                        Ex::Unary {
                            op: naga::UnaryOperator::Negate,
                            expr: value.expression,
                        },
                        true,
                    ),
                    ty: value.ty,
                    place: None,
                })
            }
            Expr::Binary(op, left, right) => {
                let left = self.value(function, left)?;
                let right = self.value(function, right)?;
                if matches!(left.ty, Type::Pointer { .. })
                    || matches!(right.ty, Type::Pointer { .. })
                {
                    return Err(error(
                        "pointer arithmetic and comparisons require additional Naga support".into(),
                    ));
                }
                if left.ty != right.ty {
                    return Err(error(format!(
                        "binary operands have different types: {:?} and {:?}",
                        left.ty, right.ty
                    )));
                }
                let op = match op {
                    BinOp::Add => naga::BinaryOperator::Add,
                    BinOp::Sub => naga::BinaryOperator::Subtract,
                    BinOp::Mul => naga::BinaryOperator::Multiply,
                    BinOp::Div => naga::BinaryOperator::Divide,
                    BinOp::Mod => naga::BinaryOperator::Modulo,
                    BinOp::Lt => naga::BinaryOperator::Less,
                    BinOp::Le => naga::BinaryOperator::LessEqual,
                    BinOp::Gt => naga::BinaryOperator::Greater,
                    BinOp::Ge => naga::BinaryOperator::GreaterEqual,
                    BinOp::Eq => naga::BinaryOperator::Equal,
                    BinOp::Ne => naga::BinaryOperator::NotEqual,
                    _ => {
                        return Err(error(
                            "this binary operator is not supported in native functions yet".into(),
                        ));
                    }
                };
                let ty = if matches!(
                    op,
                    naga::BinaryOperator::Less
                        | naga::BinaryOperator::LessEqual
                        | naga::BinaryOperator::Greater
                        | naga::BinaryOperator::GreaterEqual
                        | naga::BinaryOperator::Equal
                        | naga::BinaryOperator::NotEqual
                ) {
                    Type::Bool
                } else {
                    left.ty
                };
                Ok(Value {
                    expression: function.expr(
                        Ex::Binary {
                            op,
                            left: left.expression,
                            right: right.expression,
                        },
                        true,
                    ),
                    ty,
                    place: None,
                })
            }
            Expr::Call {
                name,
                args,
                const_args,
                ..
            } if matches!(name.as_str(), "u32" | "i32" | "f32") => {
                if args.len() != 1 || !const_args.is_empty() || args[0].name.is_some() {
                    return Err(error(
                        "scalar conversion takes one positional argument".into(),
                    ));
                }
                let ty = Type::parse(name).map_err(error)?;
                if let Some(n) = numeric_literal(&args[0].value) {
                    let literal = match ty {
                        Type::U32 if n >= 0.0 && n <= f64::from(u32::MAX) && n.fract() == 0.0 => {
                            naga::Literal::U32(n as u32)
                        }
                        Type::I32
                            if n >= f64::from(i32::MIN)
                                && n <= f64::from(i32::MAX)
                                && n.fract() == 0.0 =>
                        {
                            naga::Literal::I32(n as i32)
                        }
                        Type::F32 if (n as f32).is_finite() => naga::Literal::F32(n as f32),
                        _ => {
                            return Err(error(
                                "literal is outside the target scalar type's range".into(),
                            ));
                        }
                    };
                    return Ok(Value {
                        expression: function.expr(Ex::Literal(literal), false),
                        ty,
                        place: None,
                    });
                }
                let value = self.value(function, &args[0].value)?;
                if !matches!(value.ty, Type::I32 | Type::U32 | Type::F32) {
                    return Err(error(
                        "integer/address and pointer conversions require additional Naga support"
                            .into(),
                    ));
                }
                let kind = match ty {
                    Type::U32 => naga::ScalarKind::Uint,
                    Type::I32 => naga::ScalarKind::Sint,
                    _ => naga::ScalarKind::Float,
                };
                Ok(Value {
                    expression: function.expr(
                        Ex::As {
                            expr: value.expression,
                            kind,
                            convert: Some(4),
                        },
                        true,
                    ),
                    ty,
                    place: None,
                })
            }
            Expr::Call {
                name,
                args,
                const_args,
                ..
            } => {
                if !const_args.is_empty() || args.iter().any(|a| a.name.is_some()) {
                    return Err(error("native helper calls require positional arguments without template arguments".into()));
                }
                self.call(function, name, args, &expr.span)?
                    .ok_or_else(|| error("void helper call must be a statement".into()))
            }
            _ => Err(error(
                "expression is not supported in native functions yet".into(),
            )),
        }
    }
}

fn numeric_literal(expr: &SExpr) -> Option<f64> {
    match &expr.node {
        Expr::Num(value, crate::lexer::Unit::None) => Some(*value),
        Expr::Unary(UnOp::Neg, inner) => numeric_literal(inner).map(|value| -value),
        _ => None,
    }
}
