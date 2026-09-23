//! Constant evaluation shared by signal checking, folding, and rewrite guards.
//! Runtime dependencies remain explicit failures; callers choose whether to report them.
use crate::hir::{Sx, SxVec};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Error {
    pub message: &'static str,
    pub help: &'static str,
}
impl Error {
    fn runtime() -> Self {
        Self {
            message: " must be compile-time constant",
            help: "use a numeric literal or constant arithmetic",
        }
    }
    fn domain(message: &'static str) -> Self {
        Self {
            message,
            help: "use values within the operation's domain",
        }
    }
}
fn clamp(value: f32, lo: f32, hi: f32) -> Result<f32, Error> {
    if lo.is_nan() || hi.is_nan() || lo > hi {
        return Err(Error::domain(
            ": clamp lower bound must not exceed upper bound",
        ));
    }
    Ok(value.clamp(lo, hi))
}
fn vector(v: &SxVec, vars: &HashMap<&str, f32>) -> Result<Vec<f32>, Error> {
    match v {
        SxVec::V2(v) => Ok(vec![evaluate(&v.0, vars)?, evaluate(&v.1, vars)?]),
        SxVec::V3(v) => Ok(vec![
            evaluate(&v.0, vars)?,
            evaluate(&v.1, vars)?,
            evaluate(&v.2, vars)?,
        ]),
        SxVec::V4(v) => Ok(vec![
            evaluate(&v.0, vars)?,
            evaluate(&v.1, vars)?,
            evaluate(&v.2, vars)?,
            evaluate(&v.3, vars)?,
        ]),
    }
}

pub(crate) fn evaluate(sx: &Sx, vars: &HashMap<&str, f32>) -> Result<f32, Error> {
    match sx {
        Sx::Typed(value) => {
            let result = value
                .evaluate(vars)
                .map_err(|_| Error::domain(": invalid typed constant expression"))?;
            match result {
                naga::Literal::F32(v) => Ok(v),
                naga::Literal::U32(v) => Ok(v as f32),
                naga::Literal::I32(v) => Ok(v as f32),
                naga::Literal::Bool(v) => Ok(if v { 1.0 } else { 0.0 }),
                _ => unreachable!("concrete scalar type"),
            }
        }
        Sx::Lit(v) | Sx::PxLit(v) => Ok(*v),
        Sx::Var(name) => vars.get(name.as_str()).copied().ok_or(Error::runtime()),
        Sx::Let { name, value, body } => {
            let value = evaluate(value, vars)?;
            let mut nested = vars.clone();
            nested.insert(name.as_str(), value);
            evaluate(body, &nested)
        }
        Sx::Neg(v) => Ok(-evaluate(v, vars)?),
        Sx::Add(a, b) => Ok(evaluate(a, vars)? + evaluate(b, vars)?),
        Sx::Sub(a, b) => Ok(evaluate(a, vars)? - evaluate(b, vars)?),
        Sx::Mul(a, b) => Ok(evaluate(a, vars)? * evaluate(b, vars)?),
        Sx::Div(a, b) => {
            let den = evaluate(b, vars)?;
            if den == 0.0 {
                return Err(Error {
                    message: " cannot divide by zero",
                    help: "use a non-zero constant value",
                });
            }
            Ok(evaluate(a, vars)? / den)
        }
        Sx::Lt(a, b) => Ok(if evaluate(a, vars)? < evaluate(b, vars)? {
            1.0
        } else {
            0.0
        }),
        Sx::Le(a, b) => Ok(if evaluate(a, vars)? <= evaluate(b, vars)? {
            1.0
        } else {
            0.0
        }),
        Sx::Gt(a, b) => Ok(if evaluate(a, vars)? > evaluate(b, vars)? {
            1.0
        } else {
            0.0
        }),
        Sx::Ge(a, b) => Ok(if evaluate(a, vars)? >= evaluate(b, vars)? {
            1.0
        } else {
            0.0
        }),
        Sx::Eq(a, b) => Ok(if evaluate(a, vars)? == evaluate(b, vars)? {
            1.0
        } else {
            0.0
        }),
        Sx::Ne(a, b) => Ok(if evaluate(a, vars)? != evaluate(b, vars)? {
            1.0
        } else {
            0.0
        }),
        Sx::Sin(v) => Ok(evaluate(v, vars)?.sin()),
        Sx::Cos(v) => Ok(evaluate(v, vars)?.cos()),
        Sx::Tan(v) => Ok(evaluate(v, vars)?.tan()),
        Sx::Asin(v) => {
            let x = evaluate(v, vars)?;
            if x.abs() > 1.0 {
                return Err(Error {
                    message: ": asin argument out of [-1, 1]",
                    help: "asin is only defined for inputs in [-1, 1]",
                });
            }
            Ok(x.asin())
        }
        Sx::Acos(v) => {
            let x = evaluate(v, vars)?;
            if x.abs() > 1.0 {
                return Err(Error {
                    message: ": acos argument out of [-1, 1]",
                    help: "acos is only defined for inputs in [-1, 1]",
                });
            }
            Ok(x.acos())
        }
        Sx::Atan(v) => Ok(evaluate(v, vars)?.atan()),
        Sx::Sqrt(v) => {
            let x = evaluate(v, vars)?;
            if x < 0.0 {
                return Err(Error {
                    message: " cannot take sqrt of a negative value",
                    help: "use a non-negative constant value",
                });
            }
            Ok(x.sqrt())
        }
        Sx::InverseSqrt(v) => {
            let x = evaluate(v, vars)?;
            if x <= 0.0 {
                return Err(Error {
                    message: ": inversesqrt requires a positive value",
                    help: "use a positive constant value",
                });
            }
            Ok(1.0 / x.sqrt())
        }
        Sx::Fract(v) => {
            let x = evaluate(v, vars)?;
            Ok(x - x.floor())
        }
        Sx::Abs(v) => Ok(evaluate(v, vars)?.abs()),
        Sx::Sign(v) => Ok(evaluate(v, vars)?.signum()),
        Sx::Floor(v) => Ok(evaluate(v, vars)?.floor()),
        Sx::Ceil(v) => Ok(evaluate(v, vars)?.ceil()),
        Sx::Round(v) => Ok(evaluate(v, vars)?.round()),
        Sx::Trunc(v) => Ok(evaluate(v, vars)?.trunc()),
        Sx::Exp(v) => Ok(evaluate(v, vars)?.exp()),
        Sx::Exp2(v) => Ok(evaluate(v, vars)?.exp2()),
        Sx::Log(v) => {
            let x = evaluate(v, vars)?;
            if x <= 0.0 {
                return Err(Error {
                    message: ": log requires a positive value",
                    help: "use a positive constant value",
                });
            }
            Ok(x.ln())
        }
        Sx::Log2(v) => {
            let x = evaluate(v, vars)?;
            if x <= 0.0 {
                return Err(Error {
                    message: ": log2 requires a positive value",
                    help: "use a positive constant value",
                });
            }
            Ok(x.log2())
        }
        Sx::Atan2(y, x) => Ok(evaluate(y, vars)?.atan2(evaluate(x, vars)?)),
        Sx::Pow(x, e) => Ok(evaluate(x, vars)?.powf(evaluate(e, vars)?)),
        Sx::Min(a, b) => Ok(evaluate(a, vars)?.min(evaluate(b, vars)?)),
        Sx::Max(a, b) => Ok(evaluate(a, vars)?.max(evaluate(b, vars)?)),
        Sx::Step(edge, x) => {
            let edge = evaluate(edge, vars)?;
            let x = evaluate(x, vars)?;
            Ok(if x < edge { 0.0 } else { 1.0 })
        }
        Sx::Dot { a, b } => {
            let av = vector(a, vars)?;
            let bv = vector(b, vars)?;
            Ok(av.iter().zip(bv.iter()).map(|(a, b)| a * b).sum())
        }
        Sx::Length(v) => {
            let comps = vector(v, vars)?;
            Ok(comps.iter().map(|x| x * x).sum::<f32>().sqrt())
        }
        Sx::NormalizeComponent { v, index } => {
            let comps = vector(v, vars)?;
            let len = comps.iter().map(|x| x * x).sum::<f32>().sqrt();
            if len <= f32::EPSILON {
                Err(Error::domain(": normalize requires a nonzero vector"))
            } else {
                comps
                    .get(*index as usize)
                    .map(|c| c / len)
                    .ok_or(Error::domain(": invalid vector component"))
            }
        }
        Sx::MinComponent { a, b, index } => {
            let av = vector(a, vars)?;
            let bv = vector(b, vars)?;
            let i = *index as usize;
            Ok(av
                .get(i)
                .ok_or(Error::domain(": invalid vector component"))?
                .min(
                    *bv.get(i)
                        .ok_or(Error::domain(": invalid vector component"))?,
                ))
        }
        Sx::MaxComponent { a, b, index } => {
            let av = vector(a, vars)?;
            let bv = vector(b, vars)?;
            let i = *index as usize;
            Ok(av
                .get(i)
                .ok_or(Error::domain(": invalid vector component"))?
                .max(
                    *bv.get(i)
                        .ok_or(Error::domain(": invalid vector component"))?,
                ))
        }
        Sx::ClampVecComponent { x, lo, hi, index } => {
            let xv = vector(x, vars)?;
            let lv = vector(lo, vars)?;
            let hv = vector(hi, vars)?;
            let i = *index as usize;
            clamp(
                *xv.get(i)
                    .ok_or(Error::domain(": invalid vector component"))?,
                *lv.get(i)
                    .ok_or(Error::domain(": invalid vector component"))?,
                *hv.get(i)
                    .ok_or(Error::domain(": invalid vector component"))?,
            )
        }
        Sx::Clamp(x, lo, hi) => {
            let x = evaluate(x, vars)?;
            let lo = evaluate(lo, vars)?;
            let hi = evaluate(hi, vars)?;
            clamp(x, lo, hi)
        }
        Sx::Select(a, b, cond) => {
            let cond = evaluate(cond, vars)?;
            evaluate(if cond != 0.0 { b } else { a }, vars)
        }
        Sx::Mix(a, b, t) => {
            let a = evaluate(a, vars)?;
            let b = evaluate(b, vars)?;
            let t = evaluate(t, vars)?;
            Ok(a + t * (b - a))
        }
        Sx::SmoothStep(lo, hi, x) => {
            let lo = evaluate(lo, vars)?;
            let hi = evaluate(hi, vars)?;
            let x = evaluate(x, vars)?;
            if (hi - lo).abs() < f32::EPSILON {
                return Err(Error {
                    message: ": smoothstep lo and hi are too close",
                    help: "lo and hi must be distinct values",
                });
            }
            let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
            Ok(t * t * (3.0 - 2.0 * t))
        }
        Sx::Ddx(_) | Sx::Ddy(_) | Sx::Fwidth(_) => Err(Error {
            message: " cannot use screen-space derivatives in compile-time constants",
            help: "use a numeric literal or constant arithmetic",
        }),
        Sx::SrgbToLinear(v) => {
            let x = evaluate(v, vars)?;
            if x <= 0.04045 {
                Ok(x / 12.92)
            } else {
                Ok(((x + 0.055) / 1.055).powf(2.4))
            }
        }
        Sx::LinearToSrgb(v) => {
            let x = evaluate(v, vars)?;
            if x <= 0.0031308 {
                Ok(x * 12.92)
            } else {
                Ok(1.055 * x.powf(1.0 / 2.4) - 0.055)
            }
        }
        Sx::EntryInput(_)
        | Sx::UniformField { .. }
        | Sx::CoordX
        | Sx::CoordY
        | Sx::FootprintJ11
        | Sx::FootprintJ12
        | Sx::FootprintJ21
        | Sx::FootprintJ22
        | Sx::PostColorR
        | Sx::PostColorG
        | Sx::PostColorB
        | Sx::PostColorA
        | Sx::Param(_)
        | Sx::ScatterInstanceId
        | Sx::ScatterInstanceIndex01
        | Sx::ScatterInstanceAgeNorm
        | Sx::ScatterInstancePosX
        | Sx::ScatterInstancePosY
        | Sx::RepeatCellIdX(_)
        | Sx::RepeatCellIdY(_)
        | Sx::RepeatCellCenterX(_)
        | Sx::RepeatCellCenterY(_)
        | Sx::RepeatCellUvX(_)
        | Sx::RepeatCellUvY(_)
        | Sx::RepeatCellRand(_)
        | Sx::UserCall { .. }
        | Sx::CellQuery { .. }
        | Sx::CellContour { .. }
        | Sx::GradientChannel { .. }
        | Sx::TexChannel { .. }
        | Sx::EffectInputChannel { .. }
        | Sx::PathDist { .. }
        | Sx::PathAlong { .. }
        | Sx::PathTangentComponent { .. }
        | Sx::PathPointAtComponent { .. }
        | Sx::PathTangentAtComponent { .. }
        | Sx::DynamicArrayIndex { .. } => Err(Error::runtime()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    #[test]
    fn lexical_bindings_restore_outer_values() {
        let expression = Sx::Add(
            Box::new(Sx::Let {
                name: "x".into(),
                value: Rc::new(Sx::Lit(3.0)),
                body: Box::new(Sx::Mul(
                    Box::new(Sx::Var("x".into())),
                    Box::new(Sx::Lit(2.0)),
                )),
            }),
            Box::new(Sx::Var("x".into())),
        );
        let vars = HashMap::from([("x", 10.0)]);
        assert_eq!(evaluate(&expression, &vars), Ok(16.0));
        assert_eq!(vars["x"], 10.0);
        assert!(evaluate(&expression, &HashMap::new()).is_err());
    }

    #[test]
    fn invalid_clamp_bounds_are_errors_instead_of_panics() {
        for lo in [2.0, f32::NAN] {
            let expression = Sx::Clamp(
                Box::new(Sx::Lit(0.5)),
                Box::new(Sx::Lit(lo)),
                Box::new(Sx::Lit(1.0)),
            );
            assert!(
                evaluate(&expression, &HashMap::new())
                    .unwrap_err()
                    .message
                    .contains("clamp")
            );
            let v = |n| SxVec::V2(Box::new((Sx::Lit(n), Sx::Lit(n))));
            let expression = Sx::ClampVecComponent {
                x: v(0.5),
                lo: v(lo),
                hi: v(1.0),
                index: 0,
            };
            assert!(evaluate(&expression, &HashMap::new()).is_err());
        }
    }

    #[test]
    fn select_does_not_evaluate_the_rejected_branch() {
        let invalid = Sx::Div(Box::new(Sx::Lit(1.0)), Box::new(Sx::Lit(0.0)));
        for (a, b, cond) in [
            (invalid.clone(), Sx::Lit(7.0), 1.0),
            (Sx::Lit(7.0), invalid, 0.0),
        ] {
            assert_eq!(
                evaluate(
                    &Sx::Select(Box::new(a), Box::new(b), Box::new(Sx::Lit(cond))),
                    &HashMap::new()
                ),
                Ok(7.0)
            );
        }
    }

    #[test]
    fn runtime_and_domain_failures_stay_distinct() {
        assert_eq!(
            evaluate(&Sx::CoordX, &HashMap::new()),
            Err(Error::runtime())
        );
        let error = evaluate(&Sx::Sqrt(Box::new(Sx::Lit(-1.0))), &HashMap::new()).unwrap_err();
        assert!(error.message.contains("sqrt"));
        let error = evaluate(&Sx::Ddx(Box::new(Sx::Lit(1.0))), &HashMap::new()).unwrap_err();
        assert!(error.message.contains("derivatives"));
    }
}
