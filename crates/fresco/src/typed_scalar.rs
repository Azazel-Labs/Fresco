//! Concrete scalar operations shared with the GPU IR. Signal-specific leaves
//! stay in Sx; integer operations never pass through an f32 representation.
use crate::hir::Sx;
use naga::{BinaryOperator, Expression, Handle, Literal};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    F32,
    I32,
    U32,
    Bool,
}
impl Kind {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "f32" | "float" | "half" => Some(Self::F32),
            "i32" | "int" => Some(Self::I32),
            "u32" => Some(Self::U32),
            "bool" => Some(Self::Bool),
            _ => None,
        }
    }
    pub fn element(name: &str) -> Option<Self> {
        Self::parse(name).or(match name {
            "uvec2" | "uvec3" | "uvec4" => Some(Self::U32),
            "ivec2" | "ivec3" | "ivec4" => Some(Self::I32),
            "bvec2" | "bvec3" | "bvec4" => Some(Self::Bool),
            "vec2" | "vec3" | "vec4" => Some(Self::F32),
            _ => None,
        })
    }
    pub fn naga(self) -> naga::ScalarKind {
        match self {
            Self::F32 => naga::ScalarKind::Float,
            Self::I32 => naga::ScalarKind::Sint,
            Self::U32 => naga::ScalarKind::Uint,
            Self::Bool => naga::ScalarKind::Bool,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::F32 => "f32",
            Self::I32 => "i32",
            Self::U32 => "u32",
            Self::Bool => "bool",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Op {
    I32(i32),
    U32(u32),
    Bool(bool),
    Cast,
    Binary(BinaryOperator),
    Negate,
    Select,
    Input,
    Math(naga::MathFunction),
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Scalar {
    pub kind: Kind,
    pub op: Op,
    pub args: Vec<Sx>,
}
impl Scalar {
    pub fn sx(self) -> Sx {
        Sx::Typed(Box::new(self))
    }
    pub fn map(mut self, mut f: impl FnMut(Sx) -> Sx) -> Self {
        self.args = self.args.into_iter().map(&mut f).collect();
        self
    }
    pub fn number(value: f64, kind: Kind) -> Result<Sx, String> {
        if !value.is_finite() {
            return Err("scalar literal must be finite".into());
        }
        let op = match kind {
            Kind::F32 => return Ok(Sx::Lit(value as f32)),
            Kind::I32
                if value.trunc() >= f64::from(i32::MIN) && value.trunc() <= f64::from(i32::MAX) =>
            {
                Op::I32(value as i32)
            }
            Kind::U32 if value.trunc() >= 0.0 && value.trunc() <= f64::from(u32::MAX) => {
                Op::U32(value as u32)
            }
            Kind::Bool => Op::Bool(value != 0.0),
            _ => return Err(format!("literal is outside the range of {}", kind.name())),
        };
        Ok(Self {
            kind,
            op,
            args: vec![],
        }
        .sx())
    }
    pub fn cast(value: Sx, kind: Kind) -> Sx {
        if value.scalar_kind() == kind {
            return value;
        }
        Self {
            kind,
            op: Op::Cast,
            args: vec![value],
        }
        .sx()
    }
    pub fn emit<E>(
        &self,
        args: &[Handle<Expression>],
        mut append: impl FnMut(Expression) -> Result<Handle<Expression>, E>,
    ) -> Result<Handle<Expression>, E> {
        if self.op == Op::Cast
            && (self.kind == Kind::Bool || self.args[0].scalar_kind() == Kind::Bool)
        {
            let literal = |kind, one| match kind {
                Kind::F32 => Literal::F32(if one { 1.0 } else { 0.0 }),
                Kind::I32 => Literal::I32(i32::from(one)),
                Kind::U32 => Literal::U32(u32::from(one)),
                Kind::Bool => Literal::Bool(one),
            };
            return if self.kind == Kind::Bool {
                let zero = append(Expression::Literal(literal(
                    self.args[0].scalar_kind(),
                    false,
                )))?;
                append(Expression::Binary {
                    op: BinaryOperator::NotEqual,
                    left: args[0],
                    right: zero,
                })
            } else {
                let reject = append(Expression::Literal(literal(self.kind, false)))?;
                let accept = append(Expression::Literal(literal(self.kind, true)))?;
                append(Expression::Select {
                    condition: args[0],
                    reject,
                    accept,
                })
            };
        }
        if self.op == Op::Input {
            return Ok(args[0]);
        }
        let expression = match self.op {
            Op::I32(v) => Expression::Literal(Literal::I32(v)),
            Op::U32(v) => Expression::Literal(Literal::U32(v)),
            Op::Bool(v) => Expression::Literal(Literal::Bool(v)),
            Op::Cast => Expression::As {
                expr: args[0],
                kind: self.kind.naga(),
                convert: Some(4),
            },
            Op::Binary(op) => Expression::Binary {
                op,
                left: args[0],
                right: args[1],
            },
            Op::Math(fun) => Expression::Math {
                fun,
                arg: args[0],
                arg1: args.get(1).copied(),
                arg2: args.get(2).copied(),
                arg3: None,
            },
            Op::Negate => Expression::Unary {
                op: naga::UnaryOperator::Negate,
                expr: args[0],
            },
            Op::Input => unreachable!("input handled above"),
            Op::Select => Expression::Select {
                reject: args[0],
                accept: args[1],
                condition: args[2],
            },
        };
        append(expression)
    }
}
impl Sx {
    pub(crate) fn scalar_kind(&self) -> Kind {
        match self {
            Sx::Typed(value) => value.kind,
            _ => Kind::F32,
        }
    }
}

impl Scalar {
    pub fn evaluate(&self, vars: &std::collections::HashMap<&str, f32>) -> Result<Literal, String> {
        let args = self
            .args
            .iter()
            .map(|arg| match arg {
                Sx::Typed(value) => value.evaluate(vars),
                value => crate::signal_eval::evaluate(value, vars)
                    .map(Literal::F32)
                    .map_err(|e| e.message.to_string()),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut module = naga::Module::default();
        let mut tracker = naga::proc::ExpressionKindTracker::new();
        let mut layouter = naga::proc::Layouter::default();
        let mut evaluator = naga::proc::ConstantEvaluator::for_wgsl_module(
            &mut module,
            &mut tracker,
            &mut layouter,
            false,
        );
        let args = args
            .into_iter()
            .map(|v| evaluator.try_eval_and_append(Expression::Literal(v), naga::Span::UNDEFINED))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let handle = self
            .emit(&args, |e| {
                evaluator.try_eval_and_append(e, naga::Span::UNDEFINED)
            })
            .map_err(|e| e.to_string())?;
        match module.global_expressions[handle] {
            Expression::Literal(value) => Ok(value),
            _ => Err("scalar constant did not reduce to a literal".into()),
        }
    }
}

impl Scalar {
    pub fn input(value: Sx, kind: Kind) -> Sx {
        if value.scalar_kind() == kind {
            value
        } else {
            Self {
                kind,
                op: Op::Input,
                args: vec![value],
            }
            .sx()
        }
    }
    pub fn select(a: Sx, b: Sx, cond: Sx) -> Sx {
        let kind = a.scalar_kind();
        if kind == Kind::F32 && b.scalar_kind() == Kind::F32 {
            Sx::Select(Box::new(a), Box::new(b), Box::new(cond))
        } else {
            Self {
                kind,
                op: Op::Select,
                args: vec![a, b, Self::cast(cond, Kind::Bool)],
            }
            .sx()
        }
    }
}

impl Scalar {
    pub fn from_literal(literal: Literal) -> Sx {
        let (kind, op) = match literal {
            Literal::F32(v) => return Sx::Lit(v),
            Literal::I32(v) => (Kind::I32, Op::I32(v)),
            Literal::U32(v) => (Kind::U32, Op::U32(v)),
            Literal::Bool(v) => (Kind::Bool, Op::Bool(v)),
            _ => unreachable!("concrete scalar literal"),
        };
        Self {
            kind,
            op,
            args: vec![],
        }
        .sx()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn evaluate(value: Sx) -> Literal {
        let Sx::Typed(value) = value else {
            panic!("expected native scalar")
        };
        value
            .evaluate(&HashMap::new())
            .expect("valid typed constant")
    }

    #[test]
    fn integer_arithmetic_preserves_bits_beyond_float_precision() {
        let left = Scalar::number(16_777_217.0, Kind::U32).unwrap();
        let right = Scalar::number(2.0, Kind::U32).unwrap();
        let sum = Scalar {
            kind: Kind::U32,
            op: Op::Binary(BinaryOperator::Add),
            args: vec![left, right],
        }
        .sx();
        assert_eq!(evaluate(sum), Literal::U32(16_777_219));
    }

    #[test]
    fn signed_shift_and_boolean_selection_use_native_types() {
        let shifted = Scalar {
            kind: Kind::I32,
            op: Op::Binary(BinaryOperator::ShiftRight),
            args: vec![
                Scalar::number(-16_777_218.0, Kind::I32).unwrap(),
                Scalar::number(1.0, Kind::U32).unwrap(),
            ],
        }
        .sx();
        let selected = Scalar::select(
            Scalar::number(0.0, Kind::I32).unwrap(),
            shifted,
            Scalar::from_literal(Literal::Bool(true)),
        );
        assert_eq!(evaluate(selected), Literal::I32(-8_388_609));
    }

    #[test]
    fn native_literal_range_is_checked_before_conversion() {
        assert!(Scalar::number(4_294_967_296.0, Kind::U32).is_err());
        assert!(Scalar::number(-2_147_483_649.0, Kind::I32).is_err());
        assert_eq!(
            evaluate(Scalar::number(4_294_967_295.0, Kind::U32).unwrap()),
            Literal::U32(u32::MAX)
        );
    }
}
