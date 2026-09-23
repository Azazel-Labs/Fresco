//! Jacobian engine and derivative computation (design doc §24.1, §25).
//!
//! This module implements identity-driven rewriting to compute:
//!
//! 1. **Jacobians** (`J(p) : mat2x2`) — the derivative of composed sample maps
//!    active at each DAG node. The columns are the arrows that show where the
//!    sample point moves when the output pixel steps one unit right (resp. up).
//!    The parallelogram spanned by these columns, scaled to one output pixel,
//!    is the node's **footprint** — the region of local space one pixel covers.
//!
//! 2. **Gradients** (`∇f : vec2`) — the spatial derivative of scalar fields,
//!    used both for warp-space Jacobians (`J(warp) = I + ∇f`) and for
//!    gradient-driven antialiasing width computation (`‖∇f · footprint‖`).
//!
//! The engine is not autodiff over emitted code. It is identity-driven rewriting
//! over typed HIR, producing `J` and `∇` as ordinary HIR expressions — which
//! makes them fuse, CSE, constant-fold, hoist to uniform rate, and appear in
//! the workbook like any other value.

use crate::hir::{Hir, Sx, SxVec, Xform};
use std::collections::BTreeSet;

#[derive(Clone, Copy)]
enum Axis {
    X,
    Y,
}

fn d(expr: &Sx, axis: Axis) -> Sx {
    match expr {
        Sx::Typed(_) => Sx::Lit(0.0),
        Sx::Lit(_)
        | Sx::PxLit(_)
        | Sx::EntryInput(_) | Sx::UniformField { .. }
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
        | Sx::CellQuery { .. } | Sx::CellContour { .. }
            | Sx::GradientChannel { .. }
        | Sx::TexChannel { .. }
        | Sx::EffectInputChannel { .. }
        | Sx::PathDist { .. }
        | Sx::PathAlong { .. }
        | Sx::PathTangentComponent { .. }
        | Sx::PathPointAtComponent { .. }
        | Sx::PathTangentAtComponent { .. }
        | Sx::Var(_)
        // Dynamic array reads are uniform — treat as zero derivative wrt sample coord.
        | Sx::DynamicArrayIndex { .. } => Sx::Lit(0.0),
        Sx::CoordX => match axis {
            Axis::X => Sx::Lit(1.0),
            Axis::Y => Sx::Lit(0.0),
        },
        Sx::CoordY => match axis {
            Axis::X => Sx::Lit(0.0),
            Axis::Y => Sx::Lit(1.0),
        },
        Sx::Neg(a) => Sx::Neg(Box::new(d(a, axis))),
        Sx::Add(a, b) => Sx::Add(Box::new(d(a, axis)), Box::new(d(b, axis))),
        Sx::Sub(a, b) => Sx::Sub(Box::new(d(a, axis)), Box::new(d(b, axis))),
        Sx::Mul(a, b) => {
            let da = d(a, axis);
            let db = d(b, axis);
            Sx::Add(
                Box::new(Sx::Mul(Box::new(da), Box::new((**b).clone()))),
                Box::new(Sx::Mul(Box::new((**a).clone()), Box::new(db))),
            )
        }
        Sx::Div(a, b) => {
            let da = d(a, axis);
            let db = d(b, axis);
            let numerator = Sx::Sub(
                Box::new(Sx::Mul(Box::new(da), Box::new((**b).clone()))),
                Box::new(Sx::Mul(Box::new((**a).clone()), Box::new(db))),
            );
            let denominator = Sx::Mul(Box::new((**b).clone()), Box::new((**b).clone()));
            Sx::Div(Box::new(numerator), Box::new(denominator))
        }
        Sx::Sin(a) => Sx::Mul(
            Box::new(Sx::Cos(Box::new((**a).clone()))),
            Box::new(d(a, axis)),
        ),
        Sx::Cos(a) => Sx::Neg(Box::new(Sx::Mul(
            Box::new(Sx::Sin(Box::new((**a).clone()))),
            Box::new(d(a, axis)),
        ))),
        Sx::Tan(a) => {
            let cos_a = Sx::Cos(Box::new((**a).clone()));
            let cos_sq = Sx::Mul(Box::new(cos_a.clone()), Box::new(cos_a));
            Sx::Div(Box::new(d(a, axis)), Box::new(cos_sq))
        }
        Sx::Asin(a) => {
            let one_minus = Sx::Sub(
                Box::new(Sx::Lit(1.0)),
                Box::new(Sx::Mul(Box::new((**a).clone()), Box::new((**a).clone()))),
            );
            Sx::Div(
                Box::new(d(a, axis)),
                Box::new(Sx::Sqrt(Box::new(one_minus))),
            )
        }
        Sx::Acos(a) => {
            let one_minus = Sx::Sub(
                Box::new(Sx::Lit(1.0)),
                Box::new(Sx::Mul(Box::new((**a).clone()), Box::new((**a).clone()))),
            );
            Sx::Neg(Box::new(Sx::Div(
                Box::new(d(a, axis)),
                Box::new(Sx::Sqrt(Box::new(one_minus))),
            )))
        }
        Sx::Atan(a) => {
            let denom = Sx::Add(
                Box::new(Sx::Lit(1.0)),
                Box::new(Sx::Mul(Box::new((**a).clone()), Box::new((**a).clone()))),
            );
            Sx::Div(Box::new(d(a, axis)), Box::new(denom))
        }
        Sx::Sqrt(a) => Sx::Div(
            Box::new(d(a, axis)),
            Box::new(Sx::Mul(
                Box::new(Sx::Lit(2.0)),
                Box::new(Sx::Sqrt(Box::new((**a).clone()))),
            )),
        ),
        Sx::InverseSqrt(a) => {
            let sqrt_a = Sx::Sqrt(Box::new((**a).clone()));
            let denom = Sx::Mul(
                Box::new(Sx::Lit(2.0)),
                Box::new(Sx::Mul(Box::new((**a).clone()), Box::new(sqrt_a))),
            );
            Sx::Neg(Box::new(Sx::Div(Box::new(d(a, axis)), Box::new(denom))))
        }
        Sx::Fract(a) => d(a, axis),
        Sx::Abs(a) => Sx::Mul(
            Box::new(Sx::Sign(Box::new((**a).clone()))),
            Box::new(d(a, axis)),
        ),
        Sx::Sign(_)
        | Sx::Floor(_)
        | Sx::Ceil(_)
        | Sx::Round(_)
        | Sx::Trunc(_)
        | Sx::Step(_, _)
        | Sx::Lt(_, _)
        | Sx::Le(_, _)
        | Sx::Gt(_, _)
        | Sx::Ge(_, _)
        | Sx::Eq(_, _)
        | Sx::Ne(_, _)
        | Sx::Ddx(_)
        | Sx::Ddy(_)
        | Sx::Fwidth(_) => Sx::Lit(0.0),
        Sx::Exp(a) => Sx::Mul(
            Box::new(Sx::Exp(Box::new((**a).clone()))),
            Box::new(d(a, axis)),
        ),
        Sx::Exp2(a) => Sx::Mul(
            Box::new(Sx::Mul(
                Box::new(Sx::Lit(std::f32::consts::LN_2)),
                Box::new(Sx::Exp2(Box::new((**a).clone()))),
            )),
            Box::new(d(a, axis)),
        ),
        Sx::Log(a) => Sx::Div(Box::new(d(a, axis)), Box::new((**a).clone())),
        Sx::Log2(a) => {
            let denom = Sx::Mul(
                Box::new((**a).clone()),
                Box::new(Sx::Lit(std::f32::consts::LN_2)),
            );
            Sx::Div(Box::new(d(a, axis)), Box::new(denom))
        }
        Sx::Atan2(a, b) => {
            let da = d(a, axis);
            let db = d(b, axis);
            let numerator = Sx::Sub(
                Box::new(Sx::Mul(Box::new((**b).clone()), Box::new(da))),
                Box::new(Sx::Mul(Box::new((**a).clone()), Box::new(db))),
            );
            let denominator = Sx::Add(
                Box::new(Sx::Mul(Box::new((**a).clone()), Box::new((**a).clone()))),
                Box::new(Sx::Mul(Box::new((**b).clone()), Box::new((**b).clone()))),
            );
            // atan2 has no derivative at the origin. Bound its denominator so
            // callers can handle the singular footprint without NaNs.
            let denominator = Sx::Max(Box::new(denominator), Box::new(Sx::Lit(1.0e-12)));
            Sx::Div(Box::new(numerator), Box::new(denominator))
        }
        Sx::Pow(a, b) | Sx::Min(a, b) | Sx::Max(a, b) => {
            let da = d(a, axis);
            let db = d(b, axis);
            Sx::Max(Box::new(da), Box::new(db))
        }
        Sx::Dot { a, b } => d_dot(a, b, axis),
        Sx::NormalizeComponent { v, index } => d_normalize_component(v, *index, axis),
        Sx::Length(v) => d_length(v, axis),
        Sx::MinComponent { a, b, index } => d_minmax_component(a, b, *index, axis),
        Sx::MaxComponent { a, b, index } => d_minmax_component(a, b, *index, axis),
        Sx::ClampVecComponent { .. } => Sx::Lit(0.0),
        Sx::Clamp(a, _, _) => d(a, axis),
        Sx::Select(a, b, cond) => Sx::Select(Box::new(d(a, axis)), Box::new(d(b, axis)), cond.clone()),
        Sx::Mix(a, b, t) => {
            let one_minus_t = Sx::Sub(Box::new(Sx::Lit(1.0)), Box::new((**t).clone()));
            let lerp_deriv = Sx::Add(
                Box::new(Sx::Mul(Box::new(d(a, axis)), Box::new(one_minus_t))),
                Box::new(Sx::Mul(Box::new(d(b, axis)), Box::new((**t).clone()))),
            );
            let dt_term = Sx::Mul(
                Box::new(Sx::Sub(Box::new((**b).clone()), Box::new((**a).clone()))),
                Box::new(d(t, axis)),
            );
            Sx::Add(Box::new(lerp_deriv), Box::new(dt_term))
        }
        Sx::SmoothStep(edge0, edge1, x) => d_smoothstep(edge0, edge1, x, axis),
        Sx::SrgbToLinear(_)
        | Sx::LinearToSrgb(_)
        | Sx::UserCall { .. } => Sx::Lit(0.0),
        // d/dx of `let name = value in body` equals d/dx of `body` with
        // `name` substituted back to `value` — inline before differentiating
        // rather than inventing a distinct calculus rule for `Let`.
        Sx::Let { name, value, body } => {
            let inlined = (**body)
                .clone()
                .subst_vars(&std::collections::HashMap::from([(
                    name.clone(),
                    (**value).clone(),
                )]));
            d(&inlined, axis)
        }
    }
}

fn vec_component(v: &SxVec, index: u8) -> Option<Sx> {
    match (v, index) {
        (SxVec::V2(v), 0) => Some(v.0.clone()),
        (SxVec::V2(v), 1) => Some(v.1.clone()),
        (SxVec::V3(v), 0) => Some(v.0.clone()),
        (SxVec::V3(v), 1) => Some(v.1.clone()),
        (SxVec::V3(v), 2) => Some(v.2.clone()),
        (SxVec::V4(v), 0) => Some(v.0.clone()),
        (SxVec::V4(v), 1) => Some(v.1.clone()),
        (SxVec::V4(v), 2) => Some(v.2.clone()),
        (SxVec::V4(v), 3) => Some(v.3.clone()),
        _ => None,
    }
}

fn d_minmax_component(a: &SxVec, b: &SxVec, index: u8, axis: Axis) -> Sx {
    let Some(a_component) = vec_component(a, index) else {
        return Sx::Lit(0.0);
    };
    let Some(b_component) = vec_component(b, index) else {
        return Sx::Lit(0.0);
    };
    let da = d(&a_component, axis);
    let db = d(&b_component, axis);
    Sx::Max(Box::new(da), Box::new(db))
}

fn d_smoothstep(edge0: &Sx, edge1: &Sx, x: &Sx, axis: Axis) -> Sx {
    // smoothstep(e0, e1, x) = t*t*(3 - 2*t), where t = clamp((x-e0)/(e1-e0), 0, 1)
    let t = Sx::Clamp(
        Box::new(Sx::Div(
            Box::new(Sx::Sub(Box::new(x.clone()), Box::new(edge0.clone()))),
            Box::new(Sx::Sub(Box::new(edge1.clone()), Box::new(edge0.clone()))),
        )),
        Box::new(Sx::Lit(0.0)),
        Box::new(Sx::Lit(1.0)),
    );
    let smoothstep_poly = Sx::Mul(
        Box::new(Sx::Mul(Box::new(t.clone()), Box::new(t.clone()))),
        Box::new(Sx::Sub(
            Box::new(Sx::Lit(3.0)),
            Box::new(Sx::Mul(Box::new(Sx::Lit(2.0)), Box::new(t))),
        )),
    );
    d(&smoothstep_poly, axis)
}

fn add_terms(mut terms: Vec<Sx>) -> Sx {
    if terms.is_empty() {
        return Sx::Lit(0.0);
    }
    let mut acc = terms.remove(0);
    for term in terms {
        acc = Sx::Add(Box::new(acc), Box::new(term));
    }
    acc
}

fn d_dot(a: &SxVec, b: &SxVec, axis: Axis) -> Sx {
    match (a, b) {
        (SxVec::V2(va), SxVec::V2(vb)) => add_terms(vec![
            Sx::Add(
                Box::new(Sx::Mul(Box::new(d(&va.0, axis)), Box::new(vb.0.clone()))),
                Box::new(Sx::Mul(Box::new(va.0.clone()), Box::new(d(&vb.0, axis)))),
            ),
            Sx::Add(
                Box::new(Sx::Mul(Box::new(d(&va.1, axis)), Box::new(vb.1.clone()))),
                Box::new(Sx::Mul(Box::new(va.1.clone()), Box::new(d(&vb.1, axis)))),
            ),
        ]),
        (SxVec::V3(va), SxVec::V3(vb)) => add_terms(vec![
            Sx::Add(
                Box::new(Sx::Mul(Box::new(d(&va.0, axis)), Box::new(vb.0.clone()))),
                Box::new(Sx::Mul(Box::new(va.0.clone()), Box::new(d(&vb.0, axis)))),
            ),
            Sx::Add(
                Box::new(Sx::Mul(Box::new(d(&va.1, axis)), Box::new(vb.1.clone()))),
                Box::new(Sx::Mul(Box::new(va.1.clone()), Box::new(d(&vb.1, axis)))),
            ),
            Sx::Add(
                Box::new(Sx::Mul(Box::new(d(&va.2, axis)), Box::new(vb.2.clone()))),
                Box::new(Sx::Mul(Box::new(va.2.clone()), Box::new(d(&vb.2, axis)))),
            ),
        ]),
        (SxVec::V4(va), SxVec::V4(vb)) => add_terms(vec![
            Sx::Add(
                Box::new(Sx::Mul(Box::new(d(&va.0, axis)), Box::new(vb.0.clone()))),
                Box::new(Sx::Mul(Box::new(va.0.clone()), Box::new(d(&vb.0, axis)))),
            ),
            Sx::Add(
                Box::new(Sx::Mul(Box::new(d(&va.1, axis)), Box::new(vb.1.clone()))),
                Box::new(Sx::Mul(Box::new(va.1.clone()), Box::new(d(&vb.1, axis)))),
            ),
            Sx::Add(
                Box::new(Sx::Mul(Box::new(d(&va.2, axis)), Box::new(vb.2.clone()))),
                Box::new(Sx::Mul(Box::new(va.2.clone()), Box::new(d(&vb.2, axis)))),
            ),
            Sx::Add(
                Box::new(Sx::Mul(Box::new(d(&va.3, axis)), Box::new(vb.3.clone()))),
                Box::new(Sx::Mul(Box::new(va.3.clone()), Box::new(d(&vb.3, axis)))),
            ),
        ]),
        _ => Sx::Lit(0.0),
    }
}

fn d_length(v: &SxVec, axis: Axis) -> Sx {
    let len = Sx::Length(v.clone());
    let len_safe = Sx::Max(Box::new(len), Box::new(Sx::Lit(1.0e-6)));

    let numerator = match v {
        SxVec::V2(v) => add_terms(vec![
            Sx::Mul(Box::new(v.0.clone()), Box::new(d(&v.0, axis))),
            Sx::Mul(Box::new(v.1.clone()), Box::new(d(&v.1, axis))),
        ]),
        SxVec::V3(v) => add_terms(vec![
            Sx::Mul(Box::new(v.0.clone()), Box::new(d(&v.0, axis))),
            Sx::Mul(Box::new(v.1.clone()), Box::new(d(&v.1, axis))),
            Sx::Mul(Box::new(v.2.clone()), Box::new(d(&v.2, axis))),
        ]),
        SxVec::V4(v) => add_terms(vec![
            Sx::Mul(Box::new(v.0.clone()), Box::new(d(&v.0, axis))),
            Sx::Mul(Box::new(v.1.clone()), Box::new(d(&v.1, axis))),
            Sx::Mul(Box::new(v.2.clone()), Box::new(d(&v.2, axis))),
            Sx::Mul(Box::new(v.3.clone()), Box::new(d(&v.3, axis))),
        ]),
    };

    Sx::Div(Box::new(numerator), Box::new(len_safe))
}

fn d_normalize_component(v: &SxVec, index: u8, axis: Axis) -> Sx {
    let Some(component) = vec_component(v, index) else {
        return Sx::Lit(0.0);
    };
    let d_component = d(&component, axis);
    let len = Sx::Length(v.clone());
    let len_safe = Sx::Max(Box::new(len), Box::new(Sx::Lit(1.0e-6)));
    let d_len = d_length(v, axis);

    let numerator = Sx::Sub(
        Box::new(Sx::Mul(Box::new(d_component), Box::new(len_safe.clone()))),
        Box::new(Sx::Mul(Box::new(component), Box::new(d_len))),
    );
    let denom = Sx::Mul(Box::new(len_safe.clone()), Box::new(len_safe));

    Sx::Div(Box::new(numerator), Box::new(denom))
}

fn transform_point(xform: &Xform, x: &Sx, y: &Sx) -> (Sx, Sx) {
    match xform {
        Xform::Rotate { angle, around } => {
            let angle = angle.clone().subst_coord(x, y);
            let cx = around.0.clone().subst_coord(x, y);
            let cy = around.1.clone().subst_coord(x, y);
            let qx = Sx::Sub(Box::new(x.clone()), Box::new(cx.clone()));
            let qy = Sx::Sub(Box::new(y.clone()), Box::new(cy.clone()));
            let c = Sx::Cos(Box::new(angle.clone()));
            let s = Sx::Sin(Box::new(angle));
            let nx = Sx::Add(
                Box::new(cx),
                Box::new(Sx::Add(
                    Box::new(Sx::Mul(Box::new(c.clone()), Box::new(qx.clone()))),
                    Box::new(Sx::Mul(Box::new(s.clone()), Box::new(qy.clone()))),
                )),
            );
            let ny = Sx::Add(
                Box::new(cy),
                Box::new(Sx::Sub(
                    Box::new(Sx::Mul(Box::new(c), Box::new(qy))),
                    Box::new(Sx::Mul(Box::new(s), Box::new(qx))),
                )),
            );
            (nx, ny)
        }
        Xform::Translate((tx, ty)) => (
            Sx::Sub(Box::new(x.clone()), Box::new(tx.clone().subst_coord(x, y))),
            Sx::Sub(Box::new(y.clone()), Box::new(ty.clone().subst_coord(x, y))),
        ),
        Xform::Translate3 { by, .. } => (
            Sx::Sub(
                Box::new(x.clone()),
                Box::new(by.0.clone().subst_coord(x, y)),
            ),
            Sx::Sub(
                Box::new(y.clone()),
                Box::new(by.1.clone().subst_coord(x, y)),
            ),
        ),
        Xform::Scale { factor, around } => {
            let k = factor.clone().subst_coord(x, y);
            let cx = around.0.clone().subst_coord(x, y);
            let cy = around.1.clone().subst_coord(x, y);
            let nx = Sx::Add(
                Box::new(cx.clone()),
                Box::new(Sx::Div(
                    Box::new(Sx::Sub(Box::new(x.clone()), Box::new(cx))),
                    Box::new(k.clone()),
                )),
            );
            let ny = Sx::Add(
                Box::new(cy.clone()),
                Box::new(Sx::Div(
                    Box::new(Sx::Sub(Box::new(y.clone()), Box::new(cy))),
                    Box::new(k),
                )),
            );
            (nx, ny)
        }
        Xform::Orientation {
            y: crate::hir::VerticalAxis::Up,
        } => (x.clone(), y.clone()),
        Xform::Orientation {
            y: crate::hir::VerticalAxis::Down,
        } => (
            x.clone(),
            Sx::Sub(Box::new(Sx::Lit(1.0)), Box::new(y.clone())),
        ),
        Xform::RepeatX(every) => {
            let every = every.clone().subst_coord(x, y);
            let turns = Sx::Div(Box::new(x.clone()), Box::new(every.clone()));
            let x_wrapped = Sx::Mul(Box::new(Sx::Fract(Box::new(turns))), Box::new(every));
            (x_wrapped, y.clone())
        }
        Xform::RepeatY(every) => {
            let every = every.clone().subst_coord(x, y);
            let turns = Sx::Div(Box::new(y.clone()), Box::new(every.clone()));
            let y_wrapped = Sx::Mul(Box::new(Sx::Fract(Box::new(turns))), Box::new(every));
            (x.clone(), y_wrapped)
        }
        Xform::Repeat2D { every, .. } => {
            let ex = every.0.clone().subst_coord(x, y);
            let ey = every.1.clone().subst_coord(x, y);
            let turns_x = Sx::Div(Box::new(x.clone()), Box::new(ex.clone()));
            let turns_y = Sx::Div(Box::new(y.clone()), Box::new(ey.clone()));
            (
                Sx::Mul(Box::new(Sx::Fract(Box::new(turns_x))), Box::new(ex)),
                Sx::Mul(Box::new(Sx::Fract(Box::new(turns_y))), Box::new(ey)),
            )
        }
        Xform::Polar {
            center,
            from,
            clockwise,
        } => {
            let qx = Sx::Sub(
                Box::new(x.clone()),
                Box::new(center.0.clone().subst_coord(x, y)),
            );
            let qy = Sx::Sub(
                Box::new(y.clone()),
                Box::new(center.1.clone().subst_coord(x, y)),
            );
            let theta = Sx::Atan2(Box::new(qy.clone()), Box::new(qx.clone()));
            let from = from.clone().subst_coord(x, y);
            let phase = if *clockwise {
                Sx::Sub(Box::new(theta), Box::new(from))
            } else {
                Sx::Sub(Box::new(from), Box::new(theta))
            };
            let turns = Sx::Div(Box::new(phase), Box::new(Sx::Lit(std::f32::consts::TAU)));
            (
                Sx::Fract(Box::new(turns)),
                Sx::Length(SxVec::V2(Box::new((qx, qy)))),
            )
        }
        Xform::Cellular(_)
        | Xform::RepeatRadial { .. }
        | Xform::Warp { .. }
        | Xform::RotateX { .. }
        | Xform::RotateY { .. }
        | Xform::Perspective { .. }
        | Xform::Aspect(_)
        | Xform::Centered { .. } => (x.clone(), y.clone()),
    }
}

fn jacobian_single_at(xform: &Xform, x: &Sx, y: &Sx) -> (Sx, Sx, Sx, Sx) {
    match xform {
        Xform::Scale { .. } | Xform::Polar { .. } => {
            let (tx, ty) = transform_point(xform, &Sx::CoordX, &Sx::CoordY);
            (
                d(&tx, Axis::X).subst_coord(x, y),
                d(&tx, Axis::Y).subst_coord(x, y),
                d(&ty, Axis::X).subst_coord(x, y),
                d(&ty, Axis::Y).subst_coord(x, y),
            )
        }
        Xform::Rotate { angle, .. } => {
            let angle = angle.clone().subst_coord(x, y);
            let c = Sx::Cos(Box::new(angle.clone()));
            let s = Sx::Sin(Box::new(angle));
            let neg_s = Sx::Neg(Box::new(s.clone()));
            (c.clone(), s, neg_s, c)
        }
        Xform::Translate(_) | Xform::Translate3 { .. } => {
            (Sx::Lit(1.0), Sx::Lit(0.0), Sx::Lit(0.0), Sx::Lit(1.0))
        }
        Xform::Orientation { y } => match y {
            crate::hir::VerticalAxis::Up => {
                (Sx::Lit(1.0), Sx::Lit(0.0), Sx::Lit(0.0), Sx::Lit(1.0))
            }
            crate::hir::VerticalAxis::Down => {
                (Sx::Lit(1.0), Sx::Lit(0.0), Sx::Lit(0.0), Sx::Lit(-1.0))
            }
        },
        Xform::RepeatX(_)
        | Xform::RepeatY(_)
        | Xform::Repeat2D { .. }
        | Xform::Cellular(_)
        | Xform::RepeatRadial { .. }
        | Xform::Warp { .. }
        | Xform::RotateX { .. }
        | Xform::RotateY { .. }
        | Xform::Perspective { .. }
        | Xform::Aspect(_)
        | Xform::Centered { .. } => (Sx::Lit(1.0), Sx::Lit(0.0), Sx::Lit(0.0), Sx::Lit(1.0)),
    }
}

/// Entry point: compute the Jacobian matrix for a composed space chain.
///
/// Returns four `Sx` nodes representing the 2×2 matrix as `[J11, J12, J21, J22]`,
/// where columns are the right/up arrows in local space per output pixel step.
///
/// The identity table (§2.2):
/// ```text
/// J(identity)                = I
/// J(translate(v))            = I
/// J(rotate(θ))               = R(−θ)
/// J(scale(k))                = diag(1/k)
/// J(orientation(y: down))    = diag(1, −1)
/// J(A . B)                   = J_B(A(p)) · J_A(p)   // chain rule
/// J(polar(...))              = analytic from (r, θ) partials — singular at center
/// J(repeat(every: e))        = I                    // piecewise-identity
/// J(mirror / kaleido)        = ±I per wedge         // piecewise
/// J(warp(by: f))             = I + ∇f               // recursion into gradient
/// J(quantize(step))          = 0                    // piecewise-constant
/// ```
///
/// Returns `(j11, j12, j21, j22)` as separate `Sx` nodes for CSE and rate analysis.
pub fn jacobian(space_chain: &[Xform]) -> (Sx, Sx, Sx, Sx) {
    // Start with identity matrix: J = I = [[1, 0], [0, 1]]
    let mut j11 = Sx::Lit(1.0);
    let mut j12 = Sx::Lit(0.0);
    let mut j21 = Sx::Lit(0.0);
    let mut j22 = Sx::Lit(1.0);
    let mut point_x = Sx::CoordX;
    let mut point_y = Sx::CoordY;

    // Apply the chain rule in authored order. Each transform's Jacobian is evaluated
    // at the point produced by all earlier transforms in the chain.
    for xform in space_chain {
        let (a11, a12, a21, a22) = jacobian_single_at(xform, &point_x, &point_y);
        let next_point = transform_point(xform, &point_x, &point_y);

        // Check if A is identity to avoid building unnecessary expression trees
        let a_is_identity = matches!(
            (&a11, &a12, &a21, &a22),
            (Sx::Lit(a), Sx::Lit(b), Sx::Lit(c), Sx::Lit(d))
                if (*a - 1.0).abs() < 1e-9
                    && b.abs() < 1e-9
                    && c.abs() < 1e-9
                    && (*d - 1.0).abs() < 1e-9
        );

        if a_is_identity {
            // A is identity, so A · J = J; skip the multiplication
            (point_x, point_y) = next_point;
            continue;
        }

        // Check if J is identity to simplify
        let j_is_identity = matches!(
            (&j11, &j12, &j21, &j22),
            (Sx::Lit(a), Sx::Lit(b), Sx::Lit(c), Sx::Lit(d))
                if (*a - 1.0).abs() < 1e-9
                    && b.abs() < 1e-9
                    && c.abs() < 1e-9
                    && (*d - 1.0).abs() < 1e-9
        );

        if j_is_identity {
            // J is identity, so A · J = A
            j11 = a11;
            j12 = a12;
            j21 = a21;
            j22 = a22;
            (point_x, point_y) = next_point;
            continue;
        }

        // Matrix multiply: J' = A · J
        // [a11 a12] · [j11 j12] = [a11*j11 + a12*j21,  a11*j12 + a12*j22]
        // [a21 a22]   [j21 j22]   [a21*j11 + a22*j21,  a21*j12 + a22*j22]
        let new_j11 = Sx::Add(
            Box::new(Sx::Mul(Box::new(a11.clone()), Box::new(j11.clone()))),
            Box::new(Sx::Mul(Box::new(a12.clone()), Box::new(j21.clone()))),
        );
        let new_j12 = Sx::Add(
            Box::new(Sx::Mul(Box::new(a11), Box::new(j12.clone()))),
            Box::new(Sx::Mul(Box::new(a12), Box::new(j22.clone()))),
        );
        let new_j21 = Sx::Add(
            Box::new(Sx::Mul(Box::new(a21.clone()), Box::new(j11.clone()))),
            Box::new(Sx::Mul(Box::new(a22.clone()), Box::new(j21.clone()))),
        );
        let new_j22 = Sx::Add(
            Box::new(Sx::Mul(Box::new(a21), Box::new(j12))),
            Box::new(Sx::Mul(Box::new(a22), Box::new(j22))),
        );

        j11 = new_j11;
        j12 = new_j12;
        j21 = new_j21;
        j22 = new_j22;

        (point_x, point_y) = next_point;
    }

    (j11, j12, j21, j22)
}

/// Entry point: compute the gradient of a scalar field expression.
///
/// Returns `(∇x, ∇y)` as two `Sx` nodes representing the spatial derivative.
///
/// The identity table (§2.3):
/// ```text
/// ∇(a + b)        = ∇a + ∇b
/// ∇(a · b)        = a∇b + b∇a
/// ∇(g(h(p)))      = g′(h(p)) · ∇h(p)       // chain rule, scalar outer
/// ∇length(p − c)  = (p − c)/|p − c|
/// ∇dist_prim      = analytic per primitive  // circle/box/capsule
/// ∇min(a, b)      = ∇a or ∇b by branch      // subgradient at seam
/// ∇noise2 / ∇fbm  = analytic (stdlib ships closed-form gradients)
/// ```
///
/// When a field includes nodes without trustworthy symbolic identities,
/// this returns explicit hardware derivatives (`ddx`/`ddy`) as a guardrail
/// instead of silently returning placeholder zeros.
fn guardrail_reason(node: &Sx) -> Option<&'static str> {
    match node {
        Sx::Pow(_, _) => Some("pow"),
        Sx::Min(_, _) => Some("min"),
        Sx::Max(_, _) => Some("max"),
        Sx::ClampVecComponent { .. } => Some("clamp_vec_component"),
        Sx::SrgbToLinear(_) => Some("srgb_to_linear"),
        Sx::LinearToSrgb(_) => Some("linear_to_srgb"),
        Sx::UserCall { .. } => Some("user_call"),
        Sx::CellQuery { .. } | Sx::CellContour { .. } => Some("cell_geometry"),
        Sx::GradientChannel { .. } => Some("gradient_channel"),
        Sx::TexChannel { .. } => Some("tex_channel"),
        Sx::EffectInputChannel { .. } => Some("effect_input_channel"),
        Sx::PathDist { .. } => Some("path_dist"),
        Sx::PathAlong { .. } => Some("path_along"),
        Sx::PathTangentComponent { .. } => Some("path_tangent_component"),
        Sx::PathPointAtComponent { .. } => Some("path_point_at_component"),
        Sx::PathTangentAtComponent { .. } => Some("path_tangent_at_component"),
        _ => None,
    }
}

fn gradient_guardrail_reasons(field_expr: &Sx) -> BTreeSet<&'static str> {
    let mut reasons = BTreeSet::new();
    field_expr.walk_preorder(&mut |node| {
        if let Some(reason) = guardrail_reason(node) {
            reasons.insert(reason);
        }
    });
    reasons
}

#[allow(
    dead_code,
    reason = "Reserved for warp/filtering call sites that are still being wired"
)]
pub fn gradient(hir: &mut Hir, field_expr: &Sx) -> (Sx, Sx) {
    let reasons = gradient_guardrail_reasons(field_expr);
    if !reasons.is_empty() {
        let reasons_joined = reasons.iter().copied().collect::<Vec<_>>().join(", ");
        let note = format!(
            "gradient guardrail: symbolic differentiation bypassed; using ddx/ddy fallback due to unsupported nodes [{reasons_joined}]"
        );
        if !hir.notes.iter().any(|existing| existing == &note) {
            hir.notes.push(note);
        }
        return (
            Sx::Ddx(Box::new(field_expr.clone())),
            Sx::Ddy(Box::new(field_expr.clone())),
        );
    }

    (d(field_expr, Axis::X), d(field_expr, Axis::Y))
}

/// Compute the footprint span (bounding box width/height) for axis-aligned
/// filtering. Returns `(span_x, span_y)` as the width and height of the
/// footprint parallelogram's axis-aligned bounding box.
///
/// For a Jacobian matrix J = [[j11, j12], [j21, j22]], the footprint spans are:
/// ```text
/// span_x = |j11| + |j12|
/// span_y = |j21| + |j22|
/// ```
/// (This is a conservative over-approximation for sheared footprints.)
pub fn footprint_spans(j11: &Sx, j12: &Sx, j21: &Sx, j22: &Sx) -> (Sx, Sx) {
    let abs_j11 = Sx::Abs(Box::new(j11.clone()));
    let abs_j12 = Sx::Abs(Box::new(j12.clone()));
    let abs_j21 = Sx::Abs(Box::new(j21.clone()));
    let abs_j22 = Sx::Abs(Box::new(j22.clone()));

    let span_x = Sx::Add(Box::new(abs_j11), Box::new(abs_j12));
    let span_y = Sx::Add(Box::new(abs_j21), Box::new(abs_j22));

    (span_x, span_y)
}

//
// ════════════════════════════════════════════════════════════════════════════
// Analytic prefiltering (§3, band-limited procedurals)
// ════════════════════════════════════════════════════════════════════════════
//

/// Compute the triangle wave antiderivative S(x) = 0.5 - |fract(x) - 0.5|.
///
/// This is the antiderivative of the unit square wave s(x), where:
/// - s(x) = +1 for fract(x) < 0.5, -1 otherwise
/// - S'(x) = s(x)
///
/// The triangle wave is used for box-filtering procedural patterns (§3.3).
pub fn triangle_wave_antiderivative(x: Sx) -> Sx {
    // S(x) = 0.5 - |fract(x) - 0.5|
    let fract_x = Sx::Fract(Box::new(x));
    let offset = Sx::Sub(Box::new(fract_x), Box::new(Sx::Lit(0.5)));
    let abs_offset = Sx::Abs(Box::new(offset));
    Sx::Sub(Box::new(Sx::Lit(0.5)), Box::new(abs_offset))
}

/// Compute the box-filtered square wave s̄(x, w) using the antiderivative method.
///
/// Returns the filtered square wave as:
/// ```text
/// s̄(x, w) = (S(x + w/2) - S(x - w/2)) / w
/// ```
/// where S is the triangle wave antiderivative (§3.3).
///
/// This degrades gracefully:
/// - As w → 0, s̄(x, w) → s(x) (the unfiltered square wave)
/// - As w → ∞, s̄(x, w) → 0 (the pattern grays out instead of shimmering)
pub fn filtered_square_wave(x: Sx, width: Sx) -> Sx {
    // Guard against zero width (causes division by zero)
    let width_clamped = Sx::Max(Box::new(width), Box::new(Sx::Lit(1e-5)));

    // Compute half-width
    let half_width = Sx::Mul(Box::new(width_clamped.clone()), Box::new(Sx::Lit(0.5)));

    // S(x + w/2)
    let x_plus_hw = Sx::Add(Box::new(x.clone()), Box::new(half_width.clone()));
    let s_plus = triangle_wave_antiderivative(x_plus_hw);

    // S(x - w/2)
    let x_minus_hw = Sx::Sub(Box::new(x), Box::new(half_width));
    let s_minus = triangle_wave_antiderivative(x_minus_hw);

    // (S(x + w/2) - S(x - w/2)) / w
    let diff = Sx::Sub(Box::new(s_plus), Box::new(s_minus));
    Sx::Div(Box::new(diff), Box::new(width_clamped))
}

/// Compute a 2D filtered checker pattern using separable box filtering.
///
/// For axis-aligned footprints, the filtered checker is exact:
/// ```text
/// checker(x, y) = 0.5 * (1 - s(x) * s(y))
/// filtered_checker(x, y, wx, wy) = 0.5 * (1 - s̄(x, wx) * s̄(y, wy))
/// ```
///
/// For sheared/rotated footprints, the function uses the footprint's axis-aligned
/// bounding box, which over-blurs by a bounded factor (declared ε ≤ 0.02, §3.3).
pub fn filtered_checker_pattern(x: Sx, y: Sx, width_x: Sx, width_y: Sx) -> Sx {
    // Compute filtered square waves for each axis
    let filtered_x = filtered_square_wave(x, width_x);
    let filtered_y = filtered_square_wave(y, width_y);

    // Multiply the two filtered waves
    let product = Sx::Mul(Box::new(filtered_x), Box::new(filtered_y));

    // checker = 0.5 * (1 - product)
    let one_minus_prod = Sx::Sub(Box::new(Sx::Lit(1.0)), Box::new(product));
    Sx::Mul(Box::new(Sx::Lit(0.5)), Box::new(one_minus_prod))
}

/// Compute a 1D filtered stripe pattern using box filtering along one axis.
///
/// A stripe is a 1D square wave, filtered using the same antiderivative method:
/// ```text
/// stripe(x) = s(x)
/// filtered_stripe(x, w) = s̄(x, w)
/// ```
pub fn filtered_stripe_pattern(x: Sx, width: Sx) -> Sx {
    filtered_square_wave(x, width)
}

/// Compute a filtered periodic pulse train with configurable duty cycle.
///
/// `x` is in cell units, `width` is the box-filter width in cell units,
/// and `duty` is the on-interval fraction per cell (clamped to [0, 1]).
///
/// This is used for analytically filtered line families where line thickness
/// is independent from line spacing.
pub fn filtered_pulse_train(x: Sx, width: Sx, duty: Sx) -> Sx {
    let eps = Sx::Lit(1.0e-5);
    let width_clamped = Sx::Max(Box::new(width), Box::new(eps));
    let duty_clamped = Sx::Clamp(
        Box::new(duty),
        Box::new(Sx::Lit(0.0)),
        Box::new(Sx::Lit(1.0)),
    );

    // Shift so each pulse is centered on integer lattice points.
    let half = Sx::Lit(0.5);
    let half_duty = Sx::Mul(Box::new(Sx::Lit(0.5)), Box::new(duty_clamped.clone()));
    let t = Sx::Sub(
        Box::new(Sx::Add(Box::new(x), Box::new(half))),
        Box::new(half_duty),
    );

    let half_width = Sx::Mul(Box::new(Sx::Lit(0.5)), Box::new(width_clamped.clone()));

    let pulse_antiderivative = |u: Sx| {
        let whole = Sx::Floor(Box::new(u.clone()));
        let frac = Sx::Fract(Box::new(u));
        let ramp = Sx::Min(Box::new(frac), Box::new(duty_clamped.clone()));
        Sx::Add(
            Box::new(Sx::Mul(Box::new(whole), Box::new(duty_clamped.clone()))),
            Box::new(ramp),
        )
    };

    let s_plus = pulse_antiderivative(Sx::Add(Box::new(t.clone()), Box::new(half_width.clone())));
    let s_minus = pulse_antiderivative(Sx::Sub(Box::new(t), Box::new(half_width)));
    let diff = Sx::Sub(Box::new(s_plus), Box::new(s_minus));
    Sx::Div(Box::new(diff), Box::new(width_clamped))
}

/// Box-filter a finite interval. Unlike a single SDF edge ramp, this integrates
/// both edges and therefore gives zero coverage to a zero-width interval.
pub(crate) fn filtered_interval(x: Sx, width: Sx, lo: Sx, hi: Sx) -> Sx {
    let width = Sx::Max(Box::new(width), Box::new(Sx::Lit(1.0e-7)));
    let half = Sx::Mul(Box::new(width.clone()), Box::new(Sx::Lit(0.5)));
    let right = Sx::Min(
        Box::new(Sx::Sub(Box::new(hi), Box::new(x.clone()))),
        Box::new(half.clone()),
    );
    let left = Sx::Max(
        Box::new(Sx::Sub(Box::new(lo), Box::new(x))),
        Box::new(Sx::Neg(Box::new(half))),
    );
    Sx::Clamp(
        Box::new(Sx::Div(
            Box::new(Sx::Sub(Box::new(right), Box::new(left))),
            Box::new(width),
        )),
        Box::new(Sx::Lit(0.0)),
        Box::new(Sx::Lit(1.0)),
    )
}

/// Filter the interval's intersection with the canonical domain [0, 1),
/// extended periodically. Clipping BEFORE repetition is essential: a shape
/// outside the authored domain must not become visible through wrapping.
pub(crate) fn filtered_periodic_interval(x: Sx, width: Sx, lo: Sx, hi: Sx) -> Sx {
    let clamp01 = |v| Sx::Clamp(Box::new(v), Box::new(Sx::Lit(0.0)), Box::new(Sx::Lit(1.0)));
    let lo = clamp01(lo);
    let hi = clamp01(hi);
    let duty = Sx::Max(
        Box::new(Sx::Sub(Box::new(hi), Box::new(lo.clone()))),
        Box::new(Sx::Lit(0.0)),
    );
    let width = Sx::Max(Box::new(width), Box::new(Sx::Lit(1.0e-7)));
    let half = Sx::Mul(Box::new(width.clone()), Box::new(Sx::Lit(0.5)));
    // Reducing the sample first limits cancellation for translated domains.
    let x = Sx::Fract(Box::new(x));
    let integral = |t: Sx| {
        let whole = Sx::Mul(
            Box::new(Sx::Floor(Box::new(t.clone()))),
            Box::new(duty.clone()),
        );
        let tail = Sx::Clamp(
            Box::new(Sx::Sub(
                Box::new(Sx::Fract(Box::new(t))),
                Box::new(lo.clone()),
            )),
            Box::new(Sx::Lit(0.0)),
            Box::new(duty.clone()),
        );
        Sx::Add(Box::new(whole), Box::new(tail))
    };
    let a = integral(Sx::Sub(Box::new(x.clone()), Box::new(half.clone())));
    let b = integral(Sx::Add(Box::new(x), Box::new(half)));
    let coverage = clamp01(Sx::Div(
        Box::new(Sx::Sub(Box::new(b), Box::new(a))),
        Box::new(width),
    ));
    // Preserve the full-domain identity exactly, even at very small footprints.
    Sx::Mix(
        Box::new(coverage),
        Box::new(Sx::Lit(1.0)),
        Box::new(Sx::Step(Box::new(Sx::Lit(1.0)), Box::new(duty))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::{ShapeAaStyle, UserFnCall, VerticalAxis};
    use std::collections::HashMap;

    fn eval_vec(expr: &SxVec, x: f32, y: f32) -> Vec<f32> {
        match expr {
            SxVec::V2(v) => vec![eval(&v.0, x, y), eval(&v.1, x, y)],
            SxVec::V3(v) => vec![eval(&v.0, x, y), eval(&v.1, x, y), eval(&v.2, x, y)],
            SxVec::V4(v) => vec![
                eval(&v.0, x, y),
                eval(&v.1, x, y),
                eval(&v.2, x, y),
                eval(&v.3, x, y),
            ],
        }
    }

    fn dummy_hir() -> Hir {
        Hir {
            entry_context: None,
            name: "test".to_string(),
            params: Vec::new(),
            rendering_policy: Some(crate::hir::RenderingPolicy {
                shape_aa_min_px: 0.75,
                shape_aa_max_px: 1.5,
                projective_footprint_max_px: 64.0,
                shape_aa_style: ShapeAaStyle::Gradient,
            }),
            canvas_space: None,
            canvas_jacobian: None,
            shapes: Vec::new(),
            layers: Vec::new(),
            layer_locality: Vec::new(),
            root: 0,
            notes: Vec::new(),
            specialization_notes: Vec::new(),
            textures: Vec::new(),
            texture_index: HashMap::new(),
            texture_metadata: HashMap::new(),
            texture_type_defs: HashMap::new(),
            user_helpers: HashMap::new(),
            path_profiles: Vec::new(),
            effects: Vec::new(),
            effect_by_name: HashMap::new(),
            global_uniforms: Vec::new(),
        }
    }

    fn eval(expr: &Sx, x: f32, y: f32) -> f32 {
        match expr {
            Sx::Lit(v) => *v,
            Sx::CoordX => x,
            Sx::CoordY => y,
            Sx::Neg(a) => -eval(a, x, y),
            Sx::Add(a, b) => eval(a, x, y) + eval(b, x, y),
            Sx::Sub(a, b) => eval(a, x, y) - eval(b, x, y),
            Sx::Mul(a, b) => eval(a, x, y) * eval(b, x, y),
            Sx::Div(a, b) => eval(a, x, y) / eval(b, x, y),
            Sx::Sin(a) => eval(a, x, y).sin(),
            Sx::Cos(a) => eval(a, x, y).cos(),
            Sx::Sqrt(a) => eval(a, x, y).sqrt(),
            Sx::Fract(a) => {
                let a = eval(a, x, y);
                a - a.floor()
            }
            Sx::Floor(a) => eval(a, x, y).floor(),
            Sx::Atan2(a, b) => eval(a, x, y).atan2(eval(b, x, y)),
            Sx::Step(edge, a) => {
                if eval(a, x, y) < eval(edge, x, y) {
                    0.0
                } else {
                    1.0
                }
            }
            Sx::Select(a, b, cond) => {
                if eval(cond, x, y) != 0.0 {
                    eval(b, x, y)
                } else {
                    eval(a, x, y)
                }
            }
            Sx::Mix(a, b, t) => {
                let t = eval(t, x, y);
                eval(a, x, y) * (1.0 - t) + eval(b, x, y) * t
            }
            Sx::Abs(a) => eval(a, x, y).abs(),
            Sx::Sign(a) => eval(a, x, y).signum(),
            Sx::Max(a, b) => eval(a, x, y).max(eval(b, x, y)),
            Sx::Min(a, b) => eval(a, x, y).min(eval(b, x, y)),
            Sx::Clamp(v, lo, hi) => eval(v, x, y).clamp(eval(lo, x, y), eval(hi, x, y)),
            Sx::Dot { a, b } => {
                let av = eval_vec(a, x, y);
                let bv = eval_vec(b, x, y);
                av.into_iter().zip(bv).map(|(lhs, rhs)| lhs * rhs).sum()
            }
            Sx::Length(v) => {
                let vv = eval_vec(v, x, y);
                vv.into_iter().map(|c| c * c).sum::<f32>().sqrt()
            }
            Sx::NormalizeComponent { v, index } => {
                let vv = eval_vec(v, x, y);
                let i = usize::from(*index);
                let len = vv.iter().map(|c| c * c).sum::<f32>().sqrt();
                vv.get(i).copied().unwrap_or(0.0) / len.max(1.0e-6)
            }
            other => panic!("test evaluator does not support {other:?}"),
        }
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1.0e-5,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn finite_interval_filter_preserves_empty_and_subpixel_area() {
        for size in [0.0_f32, 0.0001, 0.01, 0.4] {
            let expr = filtered_interval(
                Sx::CoordX,
                Sx::Lit(0.02),
                Sx::Lit(-size / 2.0),
                Sx::Lit(size / 2.0),
            );
            assert_close(eval(&expr, 0.0, 0.0), (size / 0.02).min(1.0));
            assert_close(eval(&expr, 1.0, 0.0), 0.0);
        }
        let reversed = filtered_interval(Sx::CoordX, Sx::Lit(0.02), Sx::Lit(0.6), Sx::Lit(0.4));
        assert_close(eval(&reversed, 0.5, 0.0), 0.0);
    }

    #[test]
    fn polar_interval_filter_matches_pixel_overlap_across_seam() {
        // Independent geometric reference: intersect each repeated canonical
        // interval with the pixel. Covers both endpoints and clipped shapes.
        for (lo, hi) in [
            (0.0_f32, 0.0_f32),
            (0.0, 0.0001),
            (0.0, 0.72),
            (0.0, 0.9999),
            (0.0, 1.0),
            (-0.2, 0.3),
            (0.8, 1.2),
            (1.1, 1.4),
            (-0.4, 1.4),
            (0.6, 0.4),
        ] {
            for width in [0.001_f32, 0.02, 0.4, 1.0, 2.3] {
                let expr = filtered_periodic_interval(
                    Sx::CoordX,
                    Sx::Lit(width),
                    Sx::Lit(lo),
                    Sx::Lit(hi),
                );
                for x in [-0.001_f32, 0.0, 0.0001, 0.2, 0.7199, 0.9999, 1.0, 1.001] {
                    let left = f64::from(x) - f64::from(width) / 2.0;
                    let right = f64::from(x) + f64::from(width) / 2.0;
                    let expected: f64 = (-4..=4)
                        .map(|cell| {
                            let a = f64::from(lo.clamp(0.0, 1.0)) + f64::from(cell);
                            let b = f64::from(hi.clamp(0.0, 1.0)) + f64::from(cell);
                            (right.min(b) - left.max(a)).max(0.0)
                        })
                        .sum::<f64>()
                        / f64::from(width);
                    let actual = eval(&expr, x, 0.0);
                    assert!(
                        (f64::from(actual) - expected).abs() < 0.0002,
                        "interval {lo}..{hi}, x={x}, width={width}: {actual} != {expected}"
                    );
                    if lo == 0.0 && hi == 1.0 {
                        assert_eq!(actual, 1.0);
                    }
                    if lo == hi {
                        assert_eq!(actual, 0.0);
                    }
                }
            }
        }
    }

    #[test]
    fn polar_jacobian_matches_finite_differences_and_is_finite_at_center() {
        for clockwise in [false, true] {
            let polar = Xform::Polar {
                center: (Sx::Lit(0.5), Sx::Lit(0.5)),
                from: Sx::Lit(-std::f32::consts::FRAC_PI_2),
                clockwise,
            };
            let (tx, ty) = transform_point(&polar, &Sx::CoordX, &Sx::CoordY);
            let (a, b, c, d) = jacobian(&[polar]);
            for (x, y) in [
                (0.5_f32, 0.18_f32),
                (0.499, 0.18),
                (0.501, 0.18),
                (0.2, 0.5),
                (0.8, 0.7),
            ] {
                let h = 0.0001;
                let angular_difference = |v: f32| (v + 0.5).rem_euclid(1.0) - 0.5;
                let ax = angular_difference(eval(&tx, x + h, y) - eval(&tx, x - h, y)) / (2.0 * h);
                let ay = angular_difference(eval(&tx, x, y + h) - eval(&tx, x, y - h)) / (2.0 * h);
                let rx = (eval(&ty, x + h, y) - eval(&ty, x - h, y)) / (2.0 * h);
                let ry = (eval(&ty, x, y + h) - eval(&ty, x, y - h)) / (2.0 * h);
                for (expr, expected) in [(&a, ax), (&b, ay), (&c, rx), (&d, ry)] {
                    assert!((eval(expr, x, y) - expected).abs() < 0.001);
                    assert!(eval(expr, 0.5, 0.5).is_finite());
                }
            }
        }
    }

    #[test]
    fn polar_jacobian_composes_at_transformed_points() {
        let chain = [
            Xform::Scale {
                factor: Sx::Lit(2.0),
                around: (Sx::Lit(0.0), Sx::Lit(0.0)),
            },
            Xform::Polar {
                center: (Sx::Lit(0.0), Sx::Lit(0.0)),
                from: Sx::Lit(0.0),
                clockwise: true,
            },
            // A coordinate-dependent scale verifies that transforms AFTER the
            // polar map are evaluated at (turns, radius), not Cartesian input.
            Xform::Scale {
                factor: Sx::Add(Box::new(Sx::Lit(1.0)), Box::new(Sx::CoordY)),
                around: (Sx::Lit(0.0), Sx::Lit(0.0)),
            },
        ];
        let mut point = (Sx::CoordX, Sx::CoordY);
        for xf in &chain {
            point = transform_point(xf, &point.0, &point.1);
        }
        let (a, b, c, d) = jacobian(&chain);
        let (x, y, h) = (0.6, 0.4, 0.0001);
        for (expr, expected) in [
            (
                a,
                (eval(&point.0, x + h, y) - eval(&point.0, x - h, y)) / (2.0 * h),
            ),
            (
                b,
                (eval(&point.0, x, y + h) - eval(&point.0, x, y - h)) / (2.0 * h),
            ),
            (
                c,
                (eval(&point.1, x + h, y) - eval(&point.1, x - h, y)) / (2.0 * h),
            ),
            (
                d,
                (eval(&point.1, x, y + h) - eval(&point.1, x, y - h)) / (2.0 * h),
            ),
        ] {
            assert!((eval(&expr, x, y) - expected).abs() < 0.001);
        }
    }

    #[test]
    fn jacobian_identity_for_translate() {
        let xforms = vec![Xform::Translate((Sx::Lit(1.0), Sx::Lit(2.0)))];
        let (j11, j12, j21, j22) = jacobian(&xforms);

        // Should be identity matrix
        assert!(matches!(j11, Sx::Lit(x) if (x - 1.0).abs() < 1e-6));
        assert!(matches!(j12, Sx::Lit(x) if x.abs() < 1e-6));
        assert!(matches!(j21, Sx::Lit(x) if x.abs() < 1e-6));
        assert!(matches!(j22, Sx::Lit(x) if (x - 1.0).abs() < 1e-6));
    }

    #[test]
    fn jacobian_orientation_y_down() {
        let xforms = vec![Xform::Orientation {
            y: VerticalAxis::Down,
        }];
        let (j11, j12, j21, j22) = jacobian(&xforms);

        // Should be diag(1, -1)
        assert!(matches!(j11, Sx::Lit(x) if (x - 1.0).abs() < 1e-6));
        assert!(matches!(j12, Sx::Lit(x) if x.abs() < 1e-6));
        assert!(matches!(j21, Sx::Lit(x) if x.abs() < 1e-6));
        assert!(matches!(j22, Sx::Lit(x) if (x + 1.0).abs() < 1e-6));
    }

    #[test]
    fn jacobian_scale_uniform() {
        let xforms = vec![Xform::Scale {
            factor: Sx::Lit(2.0),
            around: (Sx::Lit(0.5), Sx::Lit(0.5)),
        }];
        let (j11, j12, j21, j22) = jacobian(&xforms);

        assert_close(eval(&j11, 0.8, 0.2), 0.5);
        assert_close(eval(&j12, 0.8, 0.2), 0.0);
        assert_close(eval(&j21, 0.8, 0.2), 0.0);
        assert_close(eval(&j22, 0.8, 0.2), 0.5);
    }

    #[test]
    fn jacobian_chain_translate_scale() {
        let xforms = vec![
            Xform::Translate((Sx::Lit(0.5), Sx::Lit(0.5))),
            Xform::Scale {
                factor: Sx::Lit(2.0),
                around: (Sx::Lit(0.5), Sx::Lit(0.5)),
            },
        ];
        let (j11, j12, j21, j22) = jacobian(&xforms);

        assert_close(eval(&j11, 0.8, 0.2), 0.5);
        assert_close(eval(&j12, 0.8, 0.2), 0.0);
        assert_close(eval(&j21, 0.8, 0.2), 0.0);
        assert_close(eval(&j22, 0.8, 0.2), 0.5);
    }

    #[test]
    fn test_triangle_wave_properties() {
        // Test that triangle wave is bounded [0, 0.5]
        // At x = 0.0, fract(0) = 0, S(0) = 0.5 - |0 - 0.5| = 0.5 - 0.5 = 0
        let s_at_0 = triangle_wave_antiderivative(Sx::Lit(0.0));
        assert!(matches!(s_at_0, Sx::Sub(..)));

        // At x = 0.25, fract(0.25) = 0.25, S(0.25) = 0.5 - |0.25 - 0.5| = 0.5 - 0.25 = 0.25
        let s_at_quarter = triangle_wave_antiderivative(Sx::Lit(0.25));
        assert!(matches!(s_at_quarter, Sx::Sub(..)));

        // At x = 0.5, fract(0.5) = 0.5, S(0.5) = 0.5 - |0.5 - 0.5| = 0.5 - 0 = 0.5
        let s_at_half = triangle_wave_antiderivative(Sx::Lit(0.5));
        assert!(matches!(s_at_half, Sx::Sub(..)));
    }

    #[test]
    fn test_filtered_square_wave_structure() {
        // Test that filtered square wave produces the expected expression structure
        let x = Sx::Lit(0.5);
        let w = Sx::Lit(0.1);
        let result = filtered_square_wave(x, w);

        // Should be a division of (S(x + w/2) - S(x - w/2)) by (2 * w)
        assert!(matches!(result, Sx::Div(..)));
    }

    #[test]
    fn filtered_square_wave_preserves_full_contrast_within_single_lobe() {
        let result = filtered_square_wave(Sx::Lit(0.25), Sx::Lit(0.5));
        let value = eval(&result, 0.0, 0.0);
        assert_close(value, 1.0);
    }

    #[test]
    fn test_filtered_checker_structure() {
        // Test that filtered checker produces the expected expression structure
        let x = Sx::Lit(0.5);
        let y = Sx::Lit(0.5);
        let wx = Sx::Lit(0.1);
        let wy = Sx::Lit(0.1);
        let result = filtered_checker_pattern(x, y, wx, wy);

        // Should be 0.5 * (1 - filtered_x * filtered_y)
        assert!(matches!(result, Sx::Mul(..)));
    }

    #[test]
    fn test_filtered_stripe_structure() {
        // Test that filtered stripe produces the expected expression structure
        let x = Sx::Lit(0.5);
        let w = Sx::Lit(0.1);
        let result = filtered_stripe_pattern(x, w);

        // Should be same as filtered square wave
        assert!(matches!(result, Sx::Div(..)));
    }

    #[test]
    fn footprint_spans_sum_rotated_column_contributions() {
        let (span_x, span_y) =
            footprint_spans(&Sx::Lit(1.0), &Sx::Lit(1.0), &Sx::Lit(0.0), &Sx::Lit(1.0));

        assert_close(eval(&span_x, 0.0, 0.0), 2.0);
        assert_close(eval(&span_y, 0.0, 0.0), 1.0);
    }

    #[test]
    fn jacobian_dynamic_scale_includes_scale_gradient_terms() {
        let xforms = vec![Xform::Scale {
            factor: Sx::Add(Box::new(Sx::Lit(1.0)), Box::new(Sx::CoordY)),
            around: (Sx::Lit(0.5), Sx::Lit(0.5)),
        }];
        let (_j11, j12, _j21, _j22) = jacobian(&xforms);

        let value = eval(&j12, 1.0, 0.75);
        assert_close(value, -0.1632653);
    }

    #[test]
    fn jacobian_evaluates_dynamic_scale_at_transformed_point() {
        let xforms = vec![
            Xform::Translate((Sx::Lit(0.0), Sx::Lit(0.25))),
            Xform::Scale {
                factor: Sx::Add(Box::new(Sx::Lit(1.0)), Box::new(Sx::CoordY)),
                around: (Sx::Lit(0.5), Sx::Lit(0.5)),
            },
        ];
        let (j11, _j12, _j21, _j22) = jacobian(&xforms);

        let value = eval(&j11, 0.5, 0.75);
        assert_close(value, 2.0 / 3.0);
    }

    #[test]
    fn gradient_uses_symbolic_identities_for_polynomial_fields() {
        let expr = Sx::Add(
            Box::new(Sx::Mul(Box::new(Sx::CoordX), Box::new(Sx::CoordX))),
            Box::new(Sx::Mul(Box::new(Sx::Lit(3.0)), Box::new(Sx::CoordY))),
        );
        let mut hir = dummy_hir();
        let (grad_x, grad_y) = gradient(&mut hir, &expr);

        assert_close(eval(&grad_x, 2.0, 4.0), 4.0);
        assert_close(eval(&grad_y, 2.0, 4.0), 3.0);
    }

    #[test]
    fn gradient_falls_back_to_hardware_derivatives_for_unsupported_nodes() {
        let expr = Sx::UserCall {
            call: std::rc::Rc::new(UserFnCall {
                helper_id: "helper".to_string(),
                args: vec![Sx::CoordX],
                ret_components: 1,
            }),
            component: None,
        };
        let mut hir = dummy_hir();
        let (grad_x, grad_y) = gradient(&mut hir, &expr);

        assert!(matches!(grad_x, Sx::Ddx(_)));
        assert!(matches!(grad_y, Sx::Ddy(_)));
        assert!(
            hir.notes
                .iter()
                .any(|note| note.contains("gradient guardrail:") && note.contains("user_call"))
        );
    }

    #[test]
    fn gradient_dot_derivative_matches_linear_coefficients() {
        let expr = Sx::Dot {
            a: SxVec::V2(Box::new((Sx::CoordX, Sx::CoordY))),
            b: SxVec::V2(Box::new((Sx::Lit(2.0), Sx::Lit(3.0)))),
        };
        let mut hir = dummy_hir();
        let (grad_x, grad_y) = gradient(&mut hir, &expr);

        assert_close(eval(&grad_x, 5.0, -7.0), 2.0);
        assert_close(eval(&grad_y, 5.0, -7.0), 3.0);
        assert!(
            hir.notes
                .iter()
                .all(|note| !note.contains("gradient guardrail:"))
        );
    }

    #[test]
    fn gradient_length_derivative_is_normalized_vector() {
        let expr = Sx::Length(SxVec::V2(Box::new((Sx::CoordX, Sx::CoordY))));
        let mut hir = dummy_hir();
        let (grad_x, grad_y) = gradient(&mut hir, &expr);

        assert_close(eval(&grad_x, 3.0, 4.0), 0.6);
        assert_close(eval(&grad_y, 3.0, 4.0), 0.8);
    }

    #[test]
    fn gradient_normalize_component_uses_quotient_rule() {
        let expr = Sx::NormalizeComponent {
            v: SxVec::V2(Box::new((Sx::CoordX, Sx::CoordY))),
            index: 0,
        };
        let mut hir = dummy_hir();
        let (grad_x, grad_y) = gradient(&mut hir, &expr);

        assert_close(eval(&grad_x, 3.0, 4.0), 16.0 / 125.0);
        assert_close(eval(&grad_y, 3.0, 4.0), -12.0 / 125.0);
    }

    #[test]
    fn gradient_smoothstep_uses_symbolic_identity() {
        let expr = Sx::SmoothStep(
            Box::new(Sx::Lit(0.0)),
            Box::new(Sx::Lit(1.0)),
            Box::new(Sx::CoordX),
        );
        let mut hir = dummy_hir();
        let (grad_x, grad_y) = gradient(&mut hir, &expr);

        assert_close(eval(&grad_x, 0.25, 0.0), 1.125);
        assert_close(eval(&grad_y, 0.25, 0.0), 0.0);
        assert!(
            hir.notes
                .iter()
                .all(|note| !note.contains("gradient guardrail:"))
        );
    }

    #[test]
    fn gradient_minmax_component_uses_symbolic_path() {
        let min_expr = Sx::MinComponent {
            a: SxVec::V2(Box::new((Sx::CoordX, Sx::Lit(0.0)))),
            b: SxVec::V2(Box::new((Sx::Lit(2.0), Sx::Lit(0.0)))),
            index: 0,
        };
        let max_expr = Sx::MaxComponent {
            a: SxVec::V2(Box::new((Sx::CoordX, Sx::Lit(0.0)))),
            b: SxVec::V2(Box::new((Sx::Lit(2.0), Sx::Lit(0.0)))),
            index: 0,
        };
        let mut hir = dummy_hir();
        let (min_grad_x, _) = gradient(&mut hir, &min_expr);
        let (max_grad_x, _) = gradient(&mut hir, &max_expr);

        assert_close(eval(&min_grad_x, 1.0, 0.0), 1.0);
        assert_close(eval(&max_grad_x, 1.0, 0.0), 1.0);
        assert!(
            hir.notes
                .iter()
                .all(|note| !note.contains("min_component") && !note.contains("max_component"))
        );
    }
}
