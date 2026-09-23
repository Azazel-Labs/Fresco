//! Typed host expressions retained in compute invocation allocation plans.
use crate::{ast::*, diag::Diag};
use fresco_artifact::{
    ComputeBinaryOp as Op, ComputeScalar as Scalar, ComputeScalarType as Ty,
    ManifestComputeExpression as Expression,
};

fn scalar_type(name: &str) -> Option<Ty> {
    match name {
        "bool" => Some(Ty::Bool),
        "u32" => Some(Ty::U32),
        "i32" => Some(Ty::I32),
        "f32" => Some(Ty::F32),
        _ => None,
    }
}
fn type_name(ty: Ty) -> &'static str {
    match ty {
        Ty::Bool => "bool",
        Ty::U32 => "u32",
        Ty::I32 => "i32",
        Ty::F32 => "f32",
    }
}

pub(super) fn lower(
    program: &Program,
    operation: &StyleOperation,
    value: &SExpr,
    expected: Option<Ty>,
) -> Result<Expression, Vec<Diag>> {
    let error = |message: &str| vec![Diag::error(value.span.clone(), message)];
    let ty = super::compute_operations::host_type(program, operation, value)?;
    let literal = matches!(value.node, Expr::Num(..))
        || matches!(&value.node, Expr::Unary(UnOp::Neg, inner) if matches!(inner.node, Expr::Num(..)));
    if matches!(ty.as_str(), "number" | "f32") && literal {
        let target = scalar_type(&ty).or(expected).unwrap_or(Ty::F32);
        let parameter = GlobalParamDecl {
            name: "__compute_constant".into(),
            name_span: value.span.clone(),
            ty_name: type_name(target).into(),
            ty_span: value.span.clone(),
            default: None,
            range: None,
            span: value.span.clone(),
        };
        let constant = crate::check::compute::style_parameter_value(program, &parameter, value)?;
        let value = match target {
            Ty::Bool => Scalar::Bool(
                constant
                    .as_bool()
                    .ok_or_else(|| error("expected a boolean host constant"))?,
            ),
            Ty::U32 => Scalar::U32(
                constant
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| error("host constant exceeds u32"))?,
            ),
            Ty::I32 => Scalar::I32(
                constant
                    .as_i64()
                    .and_then(|n| i32::try_from(n).ok())
                    .ok_or_else(|| error("host constant exceeds i32"))?,
            ),
            Ty::F32 => Scalar::F32(
                constant
                    .as_f64()
                    .ok_or_else(|| error("expected a floating host constant"))?
                    as f32,
            ),
        };
        return Ok(Expression::Constant { value });
    }
    if let Ok(path) = super::style_graph::path(value) {
        if matches!(path.as_str(), "true" | "false") {
            return Ok(Expression::Constant {
                value: Scalar::Bool(path == "true"),
            });
        }
        let (parameter, member) = path
            .split_once('.')
            .map_or((path.as_str(), None), |(parameter, member)| {
                (parameter, Some(member.into()))
            });
        return Ok(Expression::Input {
            parameter: parameter.into(),
            member,
            ty: scalar_type(&ty).ok_or_else(|| error("unsupported host scalar type"))?,
        });
    }
    match &value.node {
        Expr::Unary(UnOp::Neg, inner) => Ok(Expression::Negate {
            value: Box::new(lower(
                program,
                operation,
                inner,
                scalar_type(&ty).or(expected),
            )?),
        }),
        Expr::Binary(op, left, right) => {
            let operands = scalar_type(&super::compute_operations::host_type(
                program, operation, left,
            )?)
            .or(scalar_type(&super::compute_operations::host_type(
                program, operation, right,
            )?))
            .or(expected.filter(|ty| *ty != Ty::Bool));
            let op = match op {
                BinOp::Add => Op::Add,
                BinOp::Sub => Op::Subtract,
                BinOp::Mul => Op::Multiply,
                BinOp::Div => Op::Divide,
                BinOp::Mod => Op::Remainder,
                BinOp::Eq => Op::Equal,
                BinOp::Ne => Op::NotEqual,
                BinOp::Lt => Op::Less,
                BinOp::Le => Op::LessEqual,
                BinOp::Gt => Op::Greater,
                BinOp::Ge => Op::GreaterEqual,
                BinOp::LogicalAnd => Op::And,
                BinOp::LogicalOr => Op::Or,
                _ => return Err(error("unsupported host binary operator")),
            };
            Ok(Expression::Binary {
                op,
                left: Box::new(lower(program, operation, left, operands)?),
                right: Box::new(lower(program, operation, right, operands)?),
            })
        }
        Expr::Call { name, args, .. } if matches!(name.as_str(), "checked_mul" | "checked_add") => {
            Ok(Expression::Binary {
                op: if name == "checked_mul" {
                    Op::Multiply
                } else {
                    Op::Add
                },
                left: Box::new(lower(program, operation, &args[0].value, Some(Ty::U32))?),
                right: Box::new(lower(program, operation, &args[1].value, Some(Ty::U32))?),
            })
        }
        Expr::Call { name, args, .. } => {
            let ty = scalar_type(name).ok_or_else(|| error("unsupported host conversion"))?;
            Ok(Expression::Convert {
                ty,
                value: Box::new(lower(program, operation, &args[0].value, Some(ty))?),
            })
        }
        _ => Err(error("unsupported host expression")),
    }
}
