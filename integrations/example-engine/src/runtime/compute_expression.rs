//! Checked evaluation of CPU-known compute allocation and dispatch expressions.
use super::RuntimeError;
use fresco_artifact::{
    ComputeBinaryOp as Op, ComputeScalar as Scalar, ComputeScalarType as Ty,
    ManifestComputeExpression as Expression,
};

fn invalid(message: &str) -> RuntimeError {
    RuntimeError::PassPlan(format!("compute expression: {message}"))
}

fn kind(value: Scalar) -> Ty {
    match value {
        Scalar::Bool(_) => Ty::Bool,
        Scalar::U32(_) => Ty::U32,
        Scalar::I32(_) => Ty::I32,
        Scalar::F32(_) => Ty::F32,
    }
}

fn finite(value: Scalar) -> Result<Scalar, RuntimeError> {
    if matches!(value, Scalar::F32(v) if !v.is_finite()) {
        Err(invalid("non-finite scalar"))
    } else {
        Ok(value)
    }
}

fn convert(value: Scalar, ty: Ty) -> Result<Scalar, RuntimeError> {
    let overflow = || invalid("scalar conversion exceeds its destination range");
    let result = match (value, ty) {
        (value, ty) if kind(value) == ty => value,
        (Scalar::U32(v), Ty::I32) => Scalar::I32(i32::try_from(v).map_err(|_| overflow())?),
        (Scalar::I32(v), Ty::U32) => Scalar::U32(u32::try_from(v).map_err(|_| overflow())?),
        (Scalar::U32(v), Ty::F32) => Scalar::F32(v as f32),
        (Scalar::I32(v), Ty::F32) => Scalar::F32(v as f32),
        (Scalar::F32(v), Ty::U32) => {
            let truncated = f64::from(v).trunc();
            if !(0.0..=f64::from(u32::MAX)).contains(&truncated) {
                return Err(overflow());
            }
            Scalar::U32(truncated as u32)
        }
        (Scalar::F32(v), Ty::I32) => {
            let truncated = f64::from(v).trunc();
            if !(f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&truncated) {
                return Err(overflow());
            }
            Scalar::I32(truncated as i32)
        }
        _ => return Err(invalid("boolean and numeric conversions cannot be mixed")),
    };
    finite(result)
}

fn binary(op: Op, left: Scalar, right: Scalar) -> Result<Scalar, RuntimeError> {
    if kind(left) != kind(right) {
        return Err(invalid("binary operands have different scalar types"));
    }
    if matches!(op, Op::Equal | Op::NotEqual) {
        return Ok(Scalar::Bool(if op == Op::Equal {
            left == right
        } else {
            left != right
        }));
    }
    macro_rules! numeric {
        ($a:expr, $b:expr, $variant:ident, $calculate:expr) => {{
            match op {
                Op::Less => return Ok(Scalar::Bool($a < $b)),
                Op::LessEqual => return Ok(Scalar::Bool($a <= $b)),
                Op::Greater => return Ok(Scalar::Bool($a > $b)),
                Op::GreaterEqual => return Ok(Scalar::Bool($a >= $b)),
                _ => {}
            }
            let result = $calculate;
            finite(Scalar::$variant(result.ok_or_else(|| {
                invalid("arithmetic overflow, division by zero, or invalid operator")
            })?))
        }};
    }
    match (left, right) {
        (Scalar::U32(a), Scalar::U32(b)) => numeric!(
            a,
            b,
            U32,
            match op {
                Op::Add => a.checked_add(b),
                Op::Subtract => a.checked_sub(b),
                Op::Multiply => a.checked_mul(b),
                Op::Divide => a.checked_div(b),
                Op::Remainder => a.checked_rem(b),
                _ => None,
            }
        ),
        (Scalar::I32(a), Scalar::I32(b)) => numeric!(
            a,
            b,
            I32,
            match op {
                Op::Add => a.checked_add(b),
                Op::Subtract => a.checked_sub(b),
                Op::Multiply => a.checked_mul(b),
                Op::Divide => a.checked_div(b),
                Op::Remainder => a.checked_rem(b),
                _ => None,
            }
        ),
        (Scalar::F32(a), Scalar::F32(b)) => numeric!(
            a,
            b,
            F32,
            match op {
                Op::Add => Some(a + b),
                Op::Subtract => Some(a - b),
                Op::Multiply => Some(a * b),
                Op::Divide => Some(a / b),
                Op::Remainder => Some(a % b),
                _ => None,
            }
        ),
        _ => Err(invalid("operator requires numeric operands")),
    }
}

/// `read` supplies validated settings and logical resource dimensions. It never
/// maps or reads GPU buffers. Integer arithmetic is checked at every operation.
pub fn evaluate(
    expression: &Expression,
    read: &impl Fn(&str, Option<&str>) -> Result<Scalar, RuntimeError>,
) -> Result<Scalar, RuntimeError> {
    fn visit(
        expression: &Expression,
        read: &impl Fn(&str, Option<&str>) -> Result<Scalar, RuntimeError>,
        depth: usize,
    ) -> Result<Scalar, RuntimeError> {
        if depth > 128 {
            return Err(invalid("expression nesting exceeds the host limit"));
        }
        let next = |value| visit(value, read, depth + 1);
        match expression {
            Expression::Constant { value } => finite(*value),
            Expression::Input {
                parameter,
                member,
                ty,
            } => {
                let value = finite(read(parameter, member.as_deref())?)?;
                if kind(value) != *ty {
                    return Err(invalid("input does not match the declared scalar type"));
                }
                Ok(value)
            }
            Expression::Negate { value } => match next(value)? {
                Scalar::I32(value) => Ok(Scalar::I32(
                    value
                        .checked_neg()
                        .ok_or_else(|| invalid("signed negation overflow"))?,
                )),
                Scalar::F32(value) => finite(Scalar::F32(-value)),
                _ => Err(invalid("negation requires a signed scalar")),
            },
            Expression::Convert { ty, value } => convert(next(value)?, *ty),
            Expression::Binary { op, left, right } => {
                let left = next(left)?;
                if matches!(op, Op::And | Op::Or) {
                    let Scalar::Bool(left) = left else {
                        return Err(invalid("logical operands require bool"));
                    };
                    if (*op == Op::And && !left) || (*op == Op::Or && left) {
                        return Ok(Scalar::Bool(left));
                    }
                    let Scalar::Bool(right) = next(right)? else {
                        return Err(invalid("logical operands require bool"));
                    };
                    return Ok(Scalar::Bool(right));
                }
                binary(*op, left, next(right)?)
            }
        }
    }
    visit(expression, read, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn constant(value: Scalar) -> Box<Expression> {
        Box::new(Expression::Constant { value })
    }
    fn run(expression: &Expression) -> Result<Scalar, RuntimeError> {
        evaluate(expression, &|_, _| Err(invalid("missing input")))
    }

    #[test]
    fn compute_expressions_preserve_integer_precision_and_reject_overflow() {
        let expression = |op, a, b| Expression::Binary {
            op,
            left: constant(a),
            right: constant(b),
        };
        assert_eq!(
            run(&expression(
                Op::Add,
                Scalar::U32(16_777_217),
                Scalar::U32(1)
            ))
            .unwrap(),
            Scalar::U32(16_777_218)
        );
        for (op, a, b) in [
            (Op::Add, Scalar::U32(u32::MAX), Scalar::U32(1)),
            (Op::Subtract, Scalar::U32(0), Scalar::U32(1)),
            (Op::Multiply, Scalar::U32(65536), Scalar::U32(65536)),
            (Op::Divide, Scalar::U32(1), Scalar::U32(0)),
            (Op::Divide, Scalar::I32(i32::MIN), Scalar::I32(-1)),
            (Op::Remainder, Scalar::I32(i32::MIN), Scalar::I32(-1)),
            (Op::Multiply, Scalar::F32(f32::MAX), Scalar::F32(2.0)),
        ] {
            assert!(run(&expression(op, a, b)).is_err());
        }
    }

    #[test]
    fn compute_expressions_check_conversions_inputs_and_short_circuit() {
        for value in [
            Scalar::F32(u32::MAX as f32),
            Scalar::F32(-1.0),
            Scalar::F32(f32::NAN),
            Scalar::I32(-1),
        ] {
            assert!(
                run(&Expression::Convert {
                    ty: Ty::U32,
                    value: constant(value)
                })
                .is_err()
            );
        }
        let missing = Box::new(Expression::Input {
            parameter: "missing".into(),
            member: None,
            ty: Ty::Bool,
        });
        assert_eq!(
            run(&Expression::Binary {
                op: Op::And,
                left: constant(Scalar::Bool(false)),
                right: missing
            })
            .unwrap(),
            Scalar::Bool(false)
        );
        let input = Expression::Input {
            parameter: "mesh".into(),
            member: Some("count".into()),
            ty: Ty::U32,
        };
        assert!(evaluate(&input, &|_, _| Ok(Scalar::F32(1.0))).is_err());
        assert_eq!(
            evaluate(&input, &|name, member| {
                assert_eq!((name, member), ("mesh", Some("count")));
                Ok(Scalar::U32(7))
            })
            .unwrap(),
            Scalar::U32(7)
        );
    }
}
