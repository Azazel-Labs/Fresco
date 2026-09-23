//! Math function builtins.

use crate::builtin;
use crate::check::Value;
use crate::check::expr::SymbolicFactorOp;
use crate::hir::{Shape, Sx, SxVec};

fn typed_vector_math(
    ctx: &mut crate::check::Checker,
    fun: naga::MathFunction,
    operands: &[(&crate::ast::SExpr, &Value)],
) -> Value {
    use crate::typed_scalar::{Kind, Op, Scalar};
    let kind = operands
        .iter()
        .filter_map(|(_, value)| crate::check::Checker::value_element_kind(value))
        .find(|kind| *kind != Kind::F32)
        .expect("native vector operand");
    if kind == Kind::Bool {
        ctx.diags.push(crate::diag::Diag::error(
            operands[0].0.span.clone(),
            "numeric builtin cannot operate on bool",
        ));
        return Value::Error;
    }
    let mut args = Vec::new();
    let mut template = Value::Scalar(Sx::Lit(0.0));
    let mut width = 1;
    for (expression, value) in operands {
        let value = if matches!(value, Value::Scalar(_)) {
            let Some(value) = ctx.eval_scalar_expected(expression, Some(kind)) else {
                return Value::Error;
            };
            value
        } else {
            (*value).clone()
        };
        let lanes = match &value {
            Value::Scalar(x) => vec![x.clone()],
            Value::Vec2((x, y)) => vec![x.clone(), y.clone()],
            Value::Vec3((x, y, z)) => vec![x.clone(), y.clone(), z.clone()],
            Value::Vec4((x, y, z, w)) => vec![x.clone(), y.clone(), z.clone(), w.clone()],
            _ => {
                ctx.diags.push(crate::diag::Diag::error(
                    expression.span.clone(),
                    "numeric builtin requires scalar or vector operands",
                ));
                return Value::Error;
            }
        };
        if lanes.iter().any(|lane| lane.scalar_kind() != kind)
            || (lanes.len() > 1 && width > 1 && lanes.len() != width)
        {
            ctx.diags.push(crate::diag::Diag::error(
                expression.span.clone(),
                "numeric builtin requires matching native element types and vector dimensions",
            ));
            return Value::Error;
        }
        if lanes.len() > width {
            width = lanes.len();
            template = value;
        }
        args.push(lanes);
    }
    let mut index = 0;
    crate::check::Checker::map_value_lanes(template, |_| {
        let result = Scalar {
            kind,
            op: Op::Math(fun),
            args: args
                .iter()
                .map(|lanes| lanes[index % lanes.len()].clone())
                .collect(),
        }
        .sx();
        index += 1;
        result
    })
}

fn eval_const_i64(
    ctx: &mut crate::check::Checker,
    expr: &crate::ast::SExpr,
    what: &str,
) -> Option<i64> {
    let sx = ctx.as_scalar(expr)?;
    if let Sx::Typed(value) = &sx {
        match value.evaluate(&std::collections::HashMap::new()) {
            Ok(naga::Literal::U32(v)) => return Some(i64::from(v)),
            Ok(naga::Literal::I32(v)) => return Some(i64::from(v)),
            Ok(_) => {}
            Err(message) => {
                ctx.diags.push(crate::diag::Diag::error(
                    expr.span.clone(),
                    format!("{what}: {message}"),
                ));
                return None;
            }
        }
    }
    let v = ctx.eval_const_scalar_sx(&sx, &expr.span, what)?;
    if !v.is_finite() {
        ctx.diags.push(
            crate::diag::Diag::error(expr.span.clone(), format!("{what} must be finite"))
                .with_help("use finite numeric values"),
        );
        return None;
    }
    if v.fract() != 0.0 {
        ctx.diags.push(
            crate::diag::Diag::error(expr.span.clone(), format!("{what} must be an integer"))
                .with_help("use integer-valued expressions for bit operations"),
        );
        return None;
    }
    Some(v as i64)
}

fn require_small_non_negative_int(
    ctx: &mut crate::check::Checker,
    expr: &crate::ast::SExpr,
    what: &str,
    max: i64,
) -> Option<i64> {
    let v = eval_const_i64(ctx, expr, what)?;
    if !(0..=max).contains(&v) {
        ctx.diags.push(
            crate::diag::Diag::error(expr.span.clone(), format!("{what} must be in 0..{max}"))
                .with_help(format!("use an integer literal in the range 0..{max}")),
        );
        return None;
    }
    Some(v)
}

fn sx_mod_floor(x: Sx, m: Sx) -> Sx {
    let q = Sx::Floor(Box::new(Sx::Div(Box::new(x.clone()), Box::new(m.clone()))));
    Sx::Sub(Box::new(x), Box::new(Sx::Mul(Box::new(q), Box::new(m))))
}

fn sx_unpack_u8_from_u32(word: Sx, byte_index: i64) -> Sx {
    if word.scalar_kind() == crate::typed_scalar::Kind::U32 {
        use crate::typed_scalar::{Kind, Op, Scalar};
        let shifted = Scalar {
            kind: Kind::U32,
            op: Op::Binary(naga::BinaryOperator::ShiftRight),
            args: vec![
                word,
                Scalar::from_literal(naga::Literal::U32(
                    u32::try_from(byte_index * 8).expect("checked byte index"),
                )),
            ],
        }
        .sx();
        let byte = Scalar {
            kind: Kind::U32,
            op: Op::Binary(naga::BinaryOperator::And),
            args: vec![shifted, Scalar::from_literal(naga::Literal::U32(255))],
        }
        .sx();
        return Scalar::cast(byte, Kind::F32);
    }
    let shift = 1_i64 << (byte_index * 8);
    let shifted = Sx::Floor(Box::new(Sx::Div(
        Box::new(word),
        Box::new(Sx::Lit(shift as f32)),
    )));
    sx_mod_floor(shifted, Sx::Lit(256.0))
}

fn map_numeric_unary(
    ctx: &mut crate::check::Checker,
    x: &crate::ast::SExpr,
    name: &str,
    op: fn(Sx) -> Sx,
) -> Value {
    match ctx.eval(x).unwrap_or(Value::Error) {
        value
            if crate::check::Checker::value_element_kind(&value)
                .is_some_and(|kind| kind != crate::typed_scalar::Kind::F32) =>
        {
            let fun = match name {
                "abs" => naga::MathFunction::Abs,
                "sign" => naga::MathFunction::Sign,
                _ => {
                    ctx.diags.push(crate::diag::Diag::error(
                        x.span.clone(),
                        format!("{name} requires a floating-point value; use f32(...)"),
                    ));
                    return Value::Error;
                }
            };
            typed_vector_math(ctx, fun, &[(x, &value)])
        }
        Value::Scalar(s) => Value::Scalar(op(s)),
        Value::Distance(s) => Value::Distance(op(s)),
        Value::Coverage(s) => Value::Coverage(op(s)),
        Value::Mask(s) => Value::Mask(op(s)),
        Value::Vec2((a, b)) => Value::Vec2((op(a), op(b))),
        Value::Vec3((a, b, c)) => Value::Vec3((op(a), op(b), op(c))),
        Value::Vec4((a, b, c, d)) => Value::Vec4((op(a), op(b), op(c), op(d))),
        Value::Error => Value::Error,
        other => {
            ctx.diags.push(
                crate::diag::Diag::error(
                    x.span.clone(),
                    format!(
                        "`{name}` expects scalar/vec2/vec3/vec4, found {}",
                        other.kind()
                    ),
                )
                .with_help(format!(
                    "use `{name}` on numeric scalar or vector expressions"
                )),
            );
            Value::Error
        }
    }
}

fn same_simple_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Scalar(sa), Value::Scalar(sb))
        | (Value::Distance(sa), Value::Distance(sb))
        | (Value::Coverage(sa), Value::Coverage(sb))
        | (Value::Mask(sa), Value::Mask(sb)) => sa == sb,
        (Value::Vec2(va), Value::Vec2(vb)) => va == vb,
        (Value::Vec3(va), Value::Vec3(vb)) => va == vb,
        (Value::Vec4(va), Value::Vec4(vb)) => va == vb,
        _ => false,
    }
}

fn color_to_sx(value: Value) -> Option<[Sx; 4]> {
    match value {
        Value::Color {
            rgba: [r, g, b, a], ..
        } => Some([Sx::Lit(r), Sx::Lit(g), Sx::Lit(b), Sx::Lit(a)]),
        Value::ColorField { rgba, .. } => Some(rgba),
        Value::Gradient { kind, stops } => Some(crate::hir::GradientSample::channels(kind, stops)),
        _ => None,
    }
}

fn color_luma([r, g, b, _a]: &[Sx; 4]) -> Sx {
    let wr = Sx::Lit(0.2126);
    let wg = Sx::Lit(0.7152);
    let wb = Sx::Lit(0.0722);
    Sx::Add(
        Box::new(Sx::Add(
            Box::new(Sx::Mul(Box::new(r.clone()), Box::new(wr))),
            Box::new(Sx::Mul(Box::new(g.clone()), Box::new(wg))),
        )),
        Box::new(Sx::Mul(Box::new(b.clone()), Box::new(wb))),
    )
}

fn clamp01(value: Sx) -> Sx {
    Sx::Clamp(
        Box::new(value),
        Box::new(Sx::Lit(0.0)),
        Box::new(Sx::Lit(1.0)),
    )
}

fn color_mix(lhs: Sx, rhs: Sx, t: Sx) -> Sx {
    Sx::Mix(Box::new(lhs), Box::new(rhs), Box::new(t))
}

fn require_color_transform_args(
    ctx: &mut crate::check::Checker,
    color: &crate::ast::SExpr,
    by: Sx,
    name: &str,
) -> Option<[Sx; 4]> {
    let color_value = ctx.eval(color).unwrap_or(Value::Error);
    let Some(rgba) = color_to_sx(color_value.clone()) else {
        if !matches!(color_value, Value::Error) {
            ctx.diags.push(
                crate::diag::Diag::error(
                    color.span.clone(),
                    format!("`{name}` expects a color, found {}", color_value.kind()),
                )
                .with_help(format!(
                    "use `{name}(color, by: amount)` with a color-valued expression"
                )),
            );
        }
        return None;
    };

    let _ = by;
    Some(rgba)
}

fn vec2_min(a: (Sx, Sx), b: (Sx, Sx)) -> (Sx, Sx) {
    let sa = SxVec::V2(Box::new(a));
    let sb = SxVec::V2(Box::new(b));
    (
        Sx::MinComponent {
            a: sa.clone(),
            b: sb.clone(),
            index: 0,
        },
        Sx::MinComponent {
            a: sa,
            b: sb,
            index: 1,
        },
    )
}

fn vec3_min(a: (Sx, Sx, Sx), b: (Sx, Sx, Sx)) -> (Sx, Sx, Sx) {
    let sa = SxVec::V3(Box::new(a));
    let sb = SxVec::V3(Box::new(b));
    (
        Sx::MinComponent {
            a: sa.clone(),
            b: sb.clone(),
            index: 0,
        },
        Sx::MinComponent {
            a: sa.clone(),
            b: sb.clone(),
            index: 1,
        },
        Sx::MinComponent {
            a: sa,
            b: sb,
            index: 2,
        },
    )
}

fn vec4_min(a: (Sx, Sx, Sx, Sx), b: (Sx, Sx, Sx, Sx)) -> (Sx, Sx, Sx, Sx) {
    let sa = SxVec::V4(Box::new(a));
    let sb = SxVec::V4(Box::new(b));
    (
        Sx::MinComponent {
            a: sa.clone(),
            b: sb.clone(),
            index: 0,
        },
        Sx::MinComponent {
            a: sa.clone(),
            b: sb.clone(),
            index: 1,
        },
        Sx::MinComponent {
            a: sa.clone(),
            b: sb.clone(),
            index: 2,
        },
        Sx::MinComponent {
            a: sa,
            b: sb,
            index: 3,
        },
    )
}

fn vec2_max(a: (Sx, Sx), b: (Sx, Sx)) -> (Sx, Sx) {
    let sa = SxVec::V2(Box::new(a));
    let sb = SxVec::V2(Box::new(b));
    (
        Sx::MaxComponent {
            a: sa.clone(),
            b: sb.clone(),
            index: 0,
        },
        Sx::MaxComponent {
            a: sa,
            b: sb,
            index: 1,
        },
    )
}

fn vec3_max(a: (Sx, Sx, Sx), b: (Sx, Sx, Sx)) -> (Sx, Sx, Sx) {
    let sa = SxVec::V3(Box::new(a));
    let sb = SxVec::V3(Box::new(b));
    (
        Sx::MaxComponent {
            a: sa.clone(),
            b: sb.clone(),
            index: 0,
        },
        Sx::MaxComponent {
            a: sa.clone(),
            b: sb.clone(),
            index: 1,
        },
        Sx::MaxComponent {
            a: sa,
            b: sb,
            index: 2,
        },
    )
}

fn vec4_max(a: (Sx, Sx, Sx, Sx), b: (Sx, Sx, Sx, Sx)) -> (Sx, Sx, Sx, Sx) {
    let sa = SxVec::V4(Box::new(a));
    let sb = SxVec::V4(Box::new(b));
    (
        Sx::MaxComponent {
            a: sa.clone(),
            b: sb.clone(),
            index: 0,
        },
        Sx::MaxComponent {
            a: sa.clone(),
            b: sb.clone(),
            index: 1,
        },
        Sx::MaxComponent {
            a: sa.clone(),
            b: sb.clone(),
            index: 2,
        },
        Sx::MaxComponent {
            a: sa,
            b: sb,
            index: 3,
        },
    )
}

fn vec2_clamp(x: (Sx, Sx), lo: (Sx, Sx), hi: (Sx, Sx)) -> (Sx, Sx) {
    let sx = SxVec::V2(Box::new(x));
    let sl = SxVec::V2(Box::new(lo));
    let sh = SxVec::V2(Box::new(hi));
    (
        Sx::ClampVecComponent {
            x: sx.clone(),
            lo: sl.clone(),
            hi: sh.clone(),
            index: 0,
        },
        Sx::ClampVecComponent {
            x: sx,
            lo: sl,
            hi: sh,
            index: 1,
        },
    )
}

fn vec3_clamp(x: (Sx, Sx, Sx), lo: (Sx, Sx, Sx), hi: (Sx, Sx, Sx)) -> (Sx, Sx, Sx) {
    let sx = SxVec::V3(Box::new(x));
    let sl = SxVec::V3(Box::new(lo));
    let sh = SxVec::V3(Box::new(hi));
    (
        Sx::ClampVecComponent {
            x: sx.clone(),
            lo: sl.clone(),
            hi: sh.clone(),
            index: 0,
        },
        Sx::ClampVecComponent {
            x: sx.clone(),
            lo: sl.clone(),
            hi: sh.clone(),
            index: 1,
        },
        Sx::ClampVecComponent {
            x: sx,
            lo: sl,
            hi: sh,
            index: 2,
        },
    )
}

fn vec4_clamp(x: (Sx, Sx, Sx, Sx), lo: (Sx, Sx, Sx, Sx), hi: (Sx, Sx, Sx, Sx)) -> (Sx, Sx, Sx, Sx) {
    let sx = SxVec::V4(Box::new(x));
    let sl = SxVec::V4(Box::new(lo));
    let sh = SxVec::V4(Box::new(hi));
    (
        Sx::ClampVecComponent {
            x: sx.clone(),
            lo: sl.clone(),
            hi: sh.clone(),
            index: 0,
        },
        Sx::ClampVecComponent {
            x: sx.clone(),
            lo: sl.clone(),
            hi: sh.clone(),
            index: 1,
        },
        Sx::ClampVecComponent {
            x: sx.clone(),
            lo: sl.clone(),
            hi: sh.clone(),
            index: 2,
        },
        Sx::ClampVecComponent {
            x: sx,
            lo: sl,
            hi: sh,
            index: 3,
        },
    )
}

// Trigonometric functions

builtin! {
    name = "sin",
    signature = single {
        args(x: Expr = "angle in radians (scalar, vec2, vec3, or vec4)"),
        result = Scalar | Vec2 | Vec3 | Vec4,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, x| {
        map_numeric_unary(ctx, x, "sin", |v| Sx::Sin(Box::new(v)))
    }
}

builtin! {
    name = "cos",
    signature = single {
        args(x: Expr = "angle in radians (scalar, vec2, vec3, or vec4)"),
        result = Scalar | Vec2 | Vec3 | Vec4,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, x| {
        map_numeric_unary(ctx, x, "cos", |v| Sx::Cos(Box::new(v)))
    }
    docs = "Apply cosine component-wise to scalar or vector inputs. Scalar inputs return a scalar; vector inputs return the same vector shape."
}

builtin! {
    name = "tan",
    signature = single {
        args(x: Expr = "angle in radians (scalar, vec2, vec3, or vec4)"),
        result = Scalar | Vec2 | Vec3 | Vec4,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, x| {
        map_numeric_unary(ctx, x, "tan", |v| Sx::Tan(Box::new(v)))
    }
}

builtin! {
    name = "asin",
    signature = single {
        args(x: Expr = "value (scalar, vec2, vec3, or vec4)"),
        result = Scalar | Vec2 | Vec3 | Vec4,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, x| {
        map_numeric_unary(ctx, x, "asin", |v| Sx::Asin(Box::new(v)))
    }
}

builtin! {
    name = "acos",
    signature = single {
        args(x: Expr = "value (scalar, vec2, vec3, or vec4)"),
        result = Scalar | Vec2 | Vec3 | Vec4,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, x| {
        map_numeric_unary(ctx, x, "acos", |v| Sx::Acos(Box::new(v)))
    }
}

builtin! {
    name = "atan",
    signature = single {
        args(x: Expr = "value (scalar, vec2, vec3, or vec4)"),
        result = Scalar | Vec2 | Vec3 | Vec4,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, x| {
        map_numeric_unary(ctx, x, "atan", |v| Sx::Atan(Box::new(v)))
    }
}

// Basic math functions

builtin! {
    name = "fract",
    signature = single {
        args(x: Expr = "value (scalar, vec2, vec3, or vec4)"),
        result = Scalar | Vec2 | Vec3 | Vec4,
        caps = PURE,
    },
    check = |ctx, x| {
        map_numeric_unary(ctx, x, "fract", |v| Sx::Fract(Box::new(v)))
    }
}

builtin! {
    name = "frac",
    signature = single {
        args(x: Expr = "value (scalar, vec2, vec3, or vec4)"),
        result = Scalar | Vec2 | Vec3 | Vec4,
        caps = PURE,
    },
    check = |ctx, x| {
        map_numeric_unary(ctx, x, "frac", |v| Sx::Fract(Box::new(v)))
    }
}

builtin! {
    name = "abs",
    signature = single {
        args(x: Expr = "value (scalar, vec2, vec3, or vec4)"),
        result = Scalar | Vec2 | Vec3 | Vec4,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, x| {
        map_numeric_unary(ctx, x, "abs", |v| Sx::Abs(Box::new(v)))
    }
}

builtin! {
    name = "sign",
    signature = single {
        args(x: Expr = "value (scalar, vec2, vec3, or vec4)"),
        result = Scalar | Vec2 | Vec3 | Vec4,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, x| {
        map_numeric_unary(ctx, x, "sign", |v| Sx::Sign(Box::new(v)))
    }
}

builtin! {
    name = "sqrt",
    signature = single {
        args(x: Expr = "value (scalar, vec2, vec3, or vec4)"),
        result = Scalar | Vec2 | Vec3 | Vec4,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, x| {
        map_numeric_unary(ctx, x, "sqrt", |v| Sx::Sqrt(Box::new(v)))
    }
}

builtin! {
    name = "inversesqrt",
    signature = single {
        args(x: Expr = "value (scalar, vec2, vec3, or vec4)"),
        result = Scalar | Vec2 | Vec3 | Vec4,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, x| {
        map_numeric_unary(ctx, x, "inversesqrt", |v| Sx::InverseSqrt(Box::new(v)))
    }
}

builtin! {
    name = "inverse_sqrt",
    signature = single {
        args(x: Expr = "value (scalar, vec2, vec3, or vec4)"),
        result = Scalar | Vec2 | Vec3 | Vec4,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, x| {
        map_numeric_unary(ctx, x, "inverse_sqrt", |v| Sx::InverseSqrt(Box::new(v)))
    }
}

builtin! {
    name = "inverseSqrt",
    signature = single {
        args(x: Expr = "value (scalar, vec2, vec3, or vec4)"),
        result = Scalar | Vec2 | Vec3 | Vec4,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, x| {
        map_numeric_unary(ctx, x, "inverseSqrt", |v| Sx::InverseSqrt(Box::new(v)))
    }
}

builtin! {
    name = "rsqrt",
    signature = single {
        args(x: Expr = "value (scalar, vec2, vec3, or vec4)"),
        result = Scalar | Vec2 | Vec3 | Vec4,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, x| {
        map_numeric_unary(ctx, x, "rsqrt", |v| Sx::InverseSqrt(Box::new(v)))
    }
}

builtin! {
    name = "rcp",
    signature = single {
        args(x: Scalar = "value"),
        result = Scalar,
        caps = PURE,
    },
    check = |_ctx, x| {
        Value::Scalar(Sx::Div(Box::new(Sx::Lit(1.0)), Box::new(x)))
    }
}

builtin! {
    name = "floor",
    signature = single {
        args(x: Expr = "value (scalar, vec2, vec3, or vec4)"),
        result = Scalar | Vec2 | Vec3 | Vec4,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, x| {
        map_numeric_unary(ctx, x, "floor", |v| Sx::Floor(Box::new(v)))
    }
}

builtin! {
    name = "ceil",
    signature = single {
        args(x: Expr = "value (scalar, vec2, vec3, or vec4)"),
        result = Scalar | Vec2 | Vec3 | Vec4,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, x| {
        map_numeric_unary(ctx, x, "ceil", |v| Sx::Ceil(Box::new(v)))
    }
}

builtin! {
    name = "round",
    signature = single {
        args(x: Expr = "value (scalar, vec2, vec3, or vec4)"),
        result = Scalar | Vec2 | Vec3 | Vec4,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, x| {
        map_numeric_unary(ctx, x, "round", |v| Sx::Round(Box::new(v)))
    }
}

builtin! {
    name = "trunc",
    signature = single {
        args(x: Expr = "value (scalar, vec2, vec3, or vec4)"),
        result = Scalar | Vec2 | Vec3 | Vec4,
        caps = PURE,
    },
    check = |ctx, x| {
        map_numeric_unary(ctx, x, "trunc", |v| Sx::Trunc(Box::new(v)))
    }
}

builtin! {
    name = "exp",
    signature = single {
        args(x: Expr = "value (scalar, vec2, vec3, or vec4)"),
        result = Scalar | Vec2 | Vec3 | Vec4,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, x| {
        map_numeric_unary(ctx, x, "exp", |v| Sx::Exp(Box::new(v)))
    }
}

builtin! {
    name = "exp2",
    signature = single {
        args(x: Expr = "value (scalar, vec2, vec3, or vec4)"),
        result = Scalar | Vec2 | Vec3 | Vec4,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, x| {
        map_numeric_unary(ctx, x, "exp2", |v| Sx::Exp2(Box::new(v)))
    }
}

builtin! {
    name = "log",
    signature = single {
        args(x: Expr = "value (scalar, vec2, vec3, or vec4)"),
        result = Scalar | Vec2 | Vec3 | Vec4,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, x| {
        map_numeric_unary(ctx, x, "log", |v| Sx::Log(Box::new(v)))
    }
}

builtin! {
    name = "log2",
    signature = single {
        args(x: Expr = "value (scalar, vec2, vec3, or vec4)"),
        result = Scalar | Vec2 | Vec3 | Vec4,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, x| {
        map_numeric_unary(ctx, x, "log2", |v| Sx::Log2(Box::new(v)))
    }
}

// Compile-time integer bit helpers (function style, GLSL-port compatibility)

builtin! {
    name = "bit_shr",
    signature = single {
        args(x: Expr = "integer value", shift: Expr = "non-negative shift amount"),
        result = Scalar,
        caps = PURE,
    },
    check = |ctx, x, shift| {
        let xv = ctx.as_scalar(x);
        let sv = ctx.as_scalar(shift);
        match (xv, sv) {
            (Some(xs), Some(Sx::Lit(sv))) => {
                let si = sv as i64;
                if !(0..=62).contains(&si) || (sv.fract() != 0.0) {
                    ctx.diags.push(
                        crate::diag::Diag::error(
                            shift.span.clone(),
                            "`bit_shr` shift must be an integer in the range 0..62",
                        )
                        .with_help("use a non-negative integer shift amount up to 62"),
                    );
                    Value::Error
                } else if let Sx::Lit(xl) = xs {
                    Value::Scalar(Sx::Lit(((xl as i64) >> si) as f32))
                } else {
                    let denom = Sx::Lit((1_i64 << si) as f32);
                    Value::Scalar(Sx::Floor(Box::new(Sx::Div(Box::new(xs), Box::new(denom)))))
                }
            }
            (Some(_), Some(_)) => {
                ctx.diags.push(
                    crate::diag::Diag::error(
                        shift.span.clone(),
                        "`bit_shr` shift must be a compile-time integer literal in v0 helper lowering",
                    )
                    .with_help("use an integer literal shift, for example `bit_shr(x, 1.0)`"),
                );
                Value::Error
            }
            _ => Value::Error,
        }
    }
}

builtin! {
    name = "bit_shl",
    signature = single {
        args(x: Expr = "integer value", shift: Expr = "non-negative shift amount"),
        result = Scalar,
        caps = PURE,
    },
    check = |ctx, x, shift| {
        match (
            eval_const_i64(ctx, x, "`bit_shl` first argument"),
            eval_const_i64(ctx, shift, "`bit_shl` shift argument"),
        ) {
            (Some(xi), Some(si)) => {
                if !(0..=30).contains(&si) {
                    ctx.diags.push(
                        crate::diag::Diag::error(
                            shift.span.clone(),
                            "`bit_shl` shift must be in the range 0..30",
                        )
                        .with_help("use a non-negative integer shift amount up to 30"),
                    );
                    Value::Error
                } else {
                    Value::Scalar(Sx::Lit((xi << si) as f32))
                }
            }
            _ => Value::Error,
        }
    }
}

builtin! {
    name = "bit_and",
    signature = single {
        args(a: Expr = "integer value", b: Expr = "integer value"),
        result = Scalar,
        caps = PURE,
    },
    check = |ctx, a, b| {
        let av = ctx.as_scalar(a);
        let bv = ctx.as_scalar(b);
        match (av, bv) {
            (Some(Sx::Lit(ai)), Some(Sx::Lit(bi))) => {
                Value::Scalar(Sx::Lit(((ai as i64) & (bi as i64)) as f32))
            }
            (Some(a_sx), Some(Sx::Lit(mask_lit))) => {
                let mask = mask_lit as i64;
                if mask > 0 && (mask & (mask - 1)) == 0 {
                    let mask_sx = Sx::Lit(mask as f32);
                    let scaled = Sx::Floor(Box::new(Sx::Div(
                        Box::new(a_sx),
                        Box::new(mask_sx.clone()),
                    )));
                    let parity = Sx::Mul(
                        Box::new(Sx::Lit(2.0)),
                        Box::new(Sx::Fract(Box::new(Sx::Mul(
                            Box::new(scaled),
                            Box::new(Sx::Lit(0.5)),
                        )))),
                    );
                    Value::Scalar(Sx::Mul(Box::new(mask_sx), Box::new(parity)))
                } else {
                    ctx.diags.push(
                        crate::diag::Diag::error(
                            b.span.clone(),
                            "`bit_and` runtime lowering supports only power-of-two masks",
                        )
                        .with_help("use masks like 1.0, 2.0, 4.0, 8.0, ..."),
                    );
                    Value::Error
                }
            }
            _ => Value::Error,
        }
    }
}

builtin! {
    name = "bit_or",
    signature = single {
        args(a: Expr = "integer value", b: Expr = "integer value"),
        result = Scalar,
        caps = PURE,
    },
    check = |ctx, a, b| {
        match (
            eval_const_i64(ctx, a, "`bit_or` first argument"),
            eval_const_i64(ctx, b, "`bit_or` second argument"),
        ) {
            (Some(ai), Some(bi)) => Value::Scalar(Sx::Lit((ai | bi) as f32)),
            _ => Value::Error,
        }
    }
}

builtin! {
    name = "bit_xor",
    signature = single {
        args(a: Expr = "integer value", b: Expr = "integer value"),
        result = Scalar,
        caps = PURE,
    },
    check = |ctx, a, b| {
        match (
            eval_const_i64(ctx, a, "`bit_xor` first argument"),
            eval_const_i64(ctx, b, "`bit_xor` second argument"),
        ) {
            (Some(ai), Some(bi)) => Value::Scalar(Sx::Lit((ai ^ bi) as f32)),
            _ => Value::Error,
        }
    }
}

builtin! {
    name = "is_multiple_of",
    signature = single {
        args(x: Expr = "integer value", n: Expr = "positive integer divisor"),
        result = Scalar,
        caps = PURE,
    },
    check = |ctx, x, n| {
        let Some(xi) = eval_const_i64(ctx, x, "`is_multiple_of` first argument") else {
            return Some(Value::Error);
        };
        let Some(ni) = eval_const_i64(ctx, n, "`is_multiple_of` second argument") else {
            return Some(Value::Error);
        };

        if ni == 0 {
            ctx.diags.push(
                crate::diag::Diag::error(
                    n.span.clone(),
                    "`is_multiple_of` divisor must be non-zero",
                )
                .with_help("use a positive integer divisor, for example `is_multiple_of(i, 2)`"),
            );
            return Some(Value::Error);
        }

        Value::Scalar(Sx::Lit(if xi % ni == 0 { 1.0 } else { 0.0 }))
    }
}

builtin! {
    name = "remainder",
    signature = single {
        args(x: Expr = "integer-like value", n: Expr = "non-zero divisor"),
        result = Scalar,
        caps = PURE,
    },
    check = |ctx, x, n| {
        let Some(xv) = ctx.as_scalar(x) else { return Some(Value::Error) };
        let Some(nv) = ctx.as_scalar(n) else { return Some(Value::Error) };

        if let Some(n_const) = ctx.eval_const_scalar_sx(&nv, &n.span, "`remainder` divisor")
            && n_const == 0.0 {
                ctx.diags.push(
                    crate::diag::Diag::error(
                        n.span.clone(),
                        "`remainder` divisor must be non-zero",
                    )
                    .with_help("use a non-zero divisor, for example `remainder(i, 2)`"),
                );
                return Some(Value::Error);
            }

        Value::Scalar(sx_mod_floor(xv, nv))
    }
}

builtin! {
    name = "bit_extract",
    signature = single {
        args(
            x: Expr = "packed integer value",
            lsb: Expr = "least-significant bit offset (0..31)",
            bits: Expr = "field bit width (1..16)"
        ),
        result = Scalar,
        caps = PURE,
    },
    check = |ctx, x, lsb, bits| {
        let Some(lsb_i) = require_small_non_negative_int(ctx, lsb, "`bit_extract` lsb", 31) else { return Some(Value::Error) };
        let bits_i = match require_small_non_negative_int(ctx, bits, "`bit_extract` bits", 16) {
            Some(v) if v >= 1 => v,
            Some(_) => {
                ctx.diags.push(
                    crate::diag::Diag::error(bits.span.clone(), "`bit_extract` bits must be at least 1")
                        .with_help("use a bit width between 1 and 16"),
                );
                return Some(Value::Error);
            }
            None => return Some(Value::Error),
        };

        let Some(xv) = ctx.as_scalar(x) else { return Some(Value::Error) };

        let two_pow_lsb = (1_i64 << lsb_i) as f32;
        let two_pow_bits = (1_i64 << bits_i) as f32;
        let shifted = Sx::Floor(Box::new(Sx::Div(
            Box::new(xv),
            Box::new(Sx::Lit(two_pow_lsb)),
        )));
        Value::Scalar(sx_mod_floor(shifted, Sx::Lit(two_pow_bits)))
    }
}

builtin! {
    name = "unpack_unorm8",
    signature = single {
        args(
            packed: Expr = "u32-like packed value",
            byte: Expr = "byte index (0..3)"
        ),
        result = Scalar,
        caps = PURE,
    },
    check = |ctx, packed, byte| {
        let Some(byte_i) = require_small_non_negative_int(ctx, byte, "`unpack_unorm8` byte", 3) else { return Some(Value::Error) };
        let Some(packed_sx) = ctx.as_scalar(packed) else { return Some(Value::Error) };
        let raw = sx_unpack_u8_from_u32(packed_sx, byte_i);
        Value::Scalar(Sx::Div(Box::new(raw), Box::new(Sx::Lit(255.0))))
    }
}

builtin! {
    name = "unpack_snorm8",
    signature = single {
        args(
            packed: Expr = "u32-like packed value",
            byte: Expr = "byte index (0..3)"
        ),
        result = Scalar,
        caps = PURE,
    },
    check = |ctx, packed, byte| {
        let Some(byte_i) = require_small_non_negative_int(ctx, byte, "`unpack_snorm8` byte", 3) else { return Some(Value::Error) };
        let Some(packed_sx) = ctx.as_scalar(packed) else { return Some(Value::Error) };

        let raw = sx_unpack_u8_from_u32(packed_sx, byte_i);
        let is_signed = Sx::Ge(Box::new(raw.clone()), Box::new(Sx::Lit(128.0)));
        let signed = Sx::Add(
            Box::new(Sx::Mul(
                Box::new(is_signed.clone()),
                Box::new(Sx::Sub(Box::new(raw.clone()), Box::new(Sx::Lit(256.0)))),
            )),
            Box::new(Sx::Mul(
                Box::new(Sx::Sub(Box::new(Sx::Lit(1.0)), Box::new(is_signed))),
                Box::new(raw),
            )),
        );

        Value::Scalar(Sx::Clamp(
            Box::new(Sx::Div(Box::new(signed), Box::new(Sx::Lit(127.0)))),
            Box::new(Sx::Lit(-1.0)),
            Box::new(Sx::Lit(1.0)),
        ))
    }
}

builtin! {
    name = "pack_unorm8x4",
    signature = single {
        args(
            r: Expr = "red in [0,1]",
            g: Expr = "green in [0,1]",
            b: Expr = "blue in [0,1]",
            a: Expr = "alpha in [0,1]"
        ),
        result = Scalar,
        caps = PURE,
    },
    check = |ctx, r, g, b, a| {
        let Some(rs) = ctx.as_scalar(r) else { return Some(Value::Error); };
        let Some(gs) = ctx.as_scalar(g) else { return Some(Value::Error); };
        let Some(bs) = ctx.as_scalar(b) else { return Some(Value::Error); };
        let Some(as_) = ctx.as_scalar(a) else { return Some(Value::Error); };

        let to_byte = |v: Sx| {
            let clamped = Sx::Clamp(Box::new(v), Box::new(Sx::Lit(0.0)), Box::new(Sx::Lit(1.0)));
            Sx::Floor(Box::new(Sx::Add(
                Box::new(Sx::Mul(Box::new(clamped), Box::new(Sx::Lit(255.0)))),
                Box::new(Sx::Lit(0.5)),
            )))
        };

        let rb = to_byte(rs);
        let gb = to_byte(gs);
        let bb = to_byte(bs);
        let ab = to_byte(as_);

        let packed = Sx::Add(
            Box::new(Sx::Add(
                Box::new(Sx::Add(
                    Box::new(rb),
                    Box::new(Sx::Mul(Box::new(gb), Box::new(Sx::Lit(256.0)))),
                )),
                Box::new(Sx::Mul(Box::new(bb), Box::new(Sx::Lit(65_536.0)))),
            )),
            Box::new(Sx::Mul(Box::new(ab), Box::new(Sx::Lit(16_777_216.0)))),
        );

        Value::Scalar(packed)
    }
}

builtin! {
    name = "min",
    signature = single {
        args(a: Expr = "first value", b: Expr = "second value"),
        result = Scalar | Vec2 | Vec3 | Vec4,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, a, b| {
        let a_span = a.span.clone();
        let b_span = b.span.clone();
        let av = ctx.eval(a).unwrap_or(Value::Error);
        let bv = ctx.eval(b).unwrap_or(Value::Error);
        let av_kind = av.kind();
        let bv_kind = bv.kind();

        if [&av, &bv].iter().any(|value| crate::check::Checker::value_element_kind(value).is_some_and(|kind| kind != crate::typed_scalar::Kind::F32)) {
            return Some(typed_vector_math(ctx, naga::MathFunction::Min, &[(a, &av), (b, &bv)]));
        }
        if matches!(av, Value::Error) || matches!(bv, Value::Error) {
            Value::Error
        } else if same_simple_value(&av, &bv) {
            av
        } else if let (Some((sa, _)), Some((sb, _))) = (
            crate::check::Checker::as_numeric_scalar(&av),
            crate::check::Checker::as_numeric_scalar(&bv),
        ) {
            if let Some(factored) = crate::check::Checker::factor_symbolic_pair(
                SymbolicFactorOp::Min,
                sa.clone(),
                sb.clone(),
            ) {
                Value::Scalar(factored)
            } else {
                Value::Scalar(Sx::Min(Box::new(sa), Box::new(sb)))
            }
        } else {
            match (av, bv) {
                (Value::Vec2(a), Value::Vec2(b)) => Value::Vec2(vec2_min(a, b)),
                (Value::Vec2((ax, ay)), b) => {
                    if let Some((s, _)) = crate::check::Checker::as_numeric_scalar(&b) {
                        Value::Vec2(vec2_min((ax, ay), (s.clone(), s)))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                b_span,
                                "`min` expects vec2 with vec2 or scalar for second argument",
                            )
                            .with_help("use `min(vec2, vec2)` or `min(vec2, scalar)`"),
                        );
                        Value::Error
                    }
                }
                (a, Value::Vec2((bx, by))) => {
                    if let Some((s, _)) = crate::check::Checker::as_numeric_scalar(&a) {
                        Value::Vec2(vec2_min((s.clone(), s), (bx, by)))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                a_span,
                                "`min` expects vec2 with vec2 or scalar for first argument",
                            )
                            .with_help("use `min(vec2, vec2)` or `min(scalar, vec2)`"),
                        );
                        Value::Error
                    }
                }
                (Value::Vec3(a), Value::Vec3(b)) => Value::Vec3(vec3_min(a, b)),
                (Value::Vec3((ax, ay, az)), b) => {
                    if let Some((s, _)) = crate::check::Checker::as_numeric_scalar(&b) {
                        Value::Vec3(vec3_min((ax, ay, az), (s.clone(), s.clone(), s)))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                b_span,
                                "`min` expects vec3 with vec3 or scalar for second argument",
                            )
                            .with_help("use `min(vec3, vec3)` or `min(vec3, scalar)`"),
                        );
                        Value::Error
                    }
                }
                (a, Value::Vec3((bx, by, bz))) => {
                    if let Some((s, _)) = crate::check::Checker::as_numeric_scalar(&a) {
                        Value::Vec3(vec3_min((s.clone(), s.clone(), s), (bx, by, bz)))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                a_span,
                                "`min` expects vec3 with vec3 or scalar for first argument",
                            )
                            .with_help("use `min(vec3, vec3)` or `min(scalar, vec3)`"),
                        );
                        Value::Error
                    }
                }
                (Value::Vec4(a), Value::Vec4(b)) => Value::Vec4(vec4_min(a, b)),
                (Value::Vec4((ax, ay, az, aw)), b) => {
                    if let Some((s, _)) = crate::check::Checker::as_numeric_scalar(&b) {
                        Value::Vec4(vec4_min((ax, ay, az, aw), (s.clone(), s.clone(), s.clone(), s)))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                b_span,
                                "`min` expects vec4 with vec4 or scalar for second argument",
                            )
                            .with_help("use `min(vec4, vec4)` or `min(vec4, scalar)`"),
                        );
                        Value::Error
                    }
                }
                (a, Value::Vec4((bx, by, bz, bw))) => {
                    if let Some((s, _)) = crate::check::Checker::as_numeric_scalar(&a) {
                        Value::Vec4(vec4_min((s.clone(), s.clone(), s.clone(), s), (bx, by, bz, bw)))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                a_span,
                                "`min` expects vec4 with vec4 or scalar for first argument",
                            )
                            .with_help("use `min(vec4, vec4)` or `min(scalar, vec4)`"),
                        );
                        Value::Error
                    }
                }
                _ => {
                    ctx.diags.push(
                        crate::diag::Diag::error(
                            a.span.clone(),
                            format!(
                                "`min` expects scalar, vec2, vec3, or vec4 operands; found {} and {}",
                                av_kind,
                                bv_kind
                            ),
                        )
                        .with_help("supported forms: min(s,s), min(v2,v2), min(v2,s), min(v3,v3), min(v3,s), min(v4,v4), min(v4,s)"),
                    );
                    Value::Error
                }
            }
        }
    }
}

builtin! {
    name = "max",
    signature = single {
        args(a: Expr = "first value", b: Expr = "second value"),
        result = Scalar | Vec2 | Vec3 | Vec4,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, a, b| {
        let a_span = a.span.clone();
        let b_span = b.span.clone();
        let av = ctx.eval(a).unwrap_or(Value::Error);
        let bv = ctx.eval(b).unwrap_or(Value::Error);
        let av_kind = av.kind();
        let bv_kind = bv.kind();

        if [&av, &bv].iter().any(|value| crate::check::Checker::value_element_kind(value).is_some_and(|kind| kind != crate::typed_scalar::Kind::F32)) {
            return Some(typed_vector_math(ctx, naga::MathFunction::Max, &[(a, &av), (b, &bv)]));
        }
        if matches!(av, Value::Error) || matches!(bv, Value::Error) {
            Value::Error
        } else if same_simple_value(&av, &bv) {
            av
        } else if let (Some((sa, _)), Some((sb, _))) = (
            crate::check::Checker::as_numeric_scalar(&av),
            crate::check::Checker::as_numeric_scalar(&bv),
        ) {
            if let Some(factored) = crate::check::Checker::factor_symbolic_pair(
                SymbolicFactorOp::Max,
                sa.clone(),
                sb.clone(),
            ) {
                Value::Scalar(factored)
            } else {
                Value::Scalar(Sx::Max(Box::new(sa), Box::new(sb)))
            }
        } else {
            match (av, bv) {
                (Value::Vec2(a), Value::Vec2(b)) => Value::Vec2(vec2_max(a, b)),
                (Value::Vec2((ax, ay)), b) => {
                    if let Some((s, _)) = crate::check::Checker::as_numeric_scalar(&b) {
                        Value::Vec2(vec2_max((ax, ay), (s.clone(), s)))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                b_span,
                                "`max` expects vec2 with vec2 or scalar for second argument",
                            )
                            .with_help("use `max(vec2, vec2)` or `max(vec2, scalar)`"),
                        );
                        Value::Error
                    }
                }
                (a, Value::Vec2((bx, by))) => {
                    if let Some((s, _)) = crate::check::Checker::as_numeric_scalar(&a) {
                        Value::Vec2(vec2_max((s.clone(), s), (bx, by)))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                a_span,
                                "`max` expects vec2 with vec2 or scalar for first argument",
                            )
                            .with_help("use `max(vec2, vec2)` or `max(scalar, vec2)`"),
                        );
                        Value::Error
                    }
                }
                (Value::Vec3(a), Value::Vec3(b)) => Value::Vec3(vec3_max(a, b)),
                (Value::Vec3((ax, ay, az)), b) => {
                    if let Some((s, _)) = crate::check::Checker::as_numeric_scalar(&b) {
                        Value::Vec3(vec3_max((ax, ay, az), (s.clone(), s.clone(), s)))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                b_span,
                                "`max` expects vec3 with vec3 or scalar for second argument",
                            )
                            .with_help("use `max(vec3, vec3)` or `max(vec3, scalar)`"),
                        );
                        Value::Error
                    }
                }
                (a, Value::Vec3((bx, by, bz))) => {
                    if let Some((s, _)) = crate::check::Checker::as_numeric_scalar(&a) {
                        Value::Vec3(vec3_max((s.clone(), s.clone(), s), (bx, by, bz)))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                a_span,
                                "`max` expects vec3 with vec3 or scalar for first argument",
                            )
                            .with_help("use `max(vec3, vec3)` or `max(scalar, vec3)`"),
                        );
                        Value::Error
                    }
                }
                (Value::Vec4(a), Value::Vec4(b)) => Value::Vec4(vec4_max(a, b)),
                (Value::Vec4((ax, ay, az, aw)), b) => {
                    if let Some((s, _)) = crate::check::Checker::as_numeric_scalar(&b) {
                        Value::Vec4(vec4_max((ax, ay, az, aw), (s.clone(), s.clone(), s.clone(), s)))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                b_span,
                                "`max` expects vec4 with vec4 or scalar for second argument",
                            )
                            .with_help("use `max(vec4, vec4)` or `max(vec4, scalar)`"),
                        );
                        Value::Error
                    }
                }
                (a, Value::Vec4((bx, by, bz, bw))) => {
                    if let Some((s, _)) = crate::check::Checker::as_numeric_scalar(&a) {
                        Value::Vec4(vec4_max((s.clone(), s.clone(), s.clone(), s), (bx, by, bz, bw)))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                a_span,
                                "`max` expects vec4 with vec4 or scalar for first argument",
                            )
                            .with_help("use `max(vec4, vec4)` or `max(scalar, vec4)`"),
                        );
                        Value::Error
                    }
                }
                _ => {
                    ctx.diags.push(
                        crate::diag::Diag::error(
                            a.span.clone(),
                            format!(
                                "`max` expects scalar, vec2, vec3, or vec4 operands; found {} and {}",
                                av_kind,
                                bv_kind
                            ),
                        )
                        .with_help("supported forms: max(s,s), max(v2,v2), max(v2,s), max(v3,v3), max(v3,s), max(v4,v4), max(v4,s)"),
                    );
                    Value::Error
                }
            }
        }
    }
}

builtin! {
    name = "pow",
    signature = single {
        args(
            x: Expr = "base (scalar, vec2, vec3, or vec4)",
            e: Expr = "exponent (scalar, vec2, vec3, or vec4 to match base)"
        ),
        result = Scalar | Vec2 | Vec3 | Vec4,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, x, e| {
        let xv = ctx.eval(x).unwrap_or(Value::Error);
        let ev = ctx.eval(e).unwrap_or(Value::Error);

        if matches!(xv, Value::Error) || matches!(ev, Value::Error) {
            Value::Error
        } else if let (Some((sx, _)), Some((se, _))) = (
            crate::check::Checker::as_numeric_scalar(&xv),
            crate::check::Checker::as_numeric_scalar(&ev),
        ) {
            Value::Scalar(Sx::Pow(Box::new(sx), Box::new(se)))
        } else {
            match (xv, ev) {
                (Value::Vec2((x0, x1)), Value::Vec2((e0, e1))) => Value::Vec2((
                    Sx::Pow(Box::new(x0), Box::new(e0)),
                    Sx::Pow(Box::new(x1), Box::new(e1)),
                )),
                (Value::Vec2((x0, x1)), ev) => {
                    if let Some((se, _)) = crate::check::Checker::as_numeric_scalar(&ev) {
                        Value::Vec2((
                            Sx::Pow(Box::new(x0), Box::new(se.clone())),
                            Sx::Pow(Box::new(x1), Box::new(se)),
                        ))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                e.span.clone(),
                                format!("`pow` vec2 exponent must be scalar or vec2, found {}", ev.kind()),
                            )
                            .with_help("use `pow(vec2, scalar)` or `pow(vec2, vec2)`"),
                        );
                        Value::Error
                    }
                }
                (Value::Vec3((x0, x1, x2)), Value::Vec3((e0, e1, e2))) => Value::Vec3((
                    Sx::Pow(Box::new(x0), Box::new(e0)),
                    Sx::Pow(Box::new(x1), Box::new(e1)),
                    Sx::Pow(Box::new(x2), Box::new(e2)),
                )),
                (Value::Vec3((x0, x1, x2)), ev) => {
                    if let Some((se, _)) = crate::check::Checker::as_numeric_scalar(&ev) {
                        Value::Vec3((
                            Sx::Pow(Box::new(x0), Box::new(se.clone())),
                            Sx::Pow(Box::new(x1), Box::new(se.clone())),
                            Sx::Pow(Box::new(x2), Box::new(se)),
                        ))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                e.span.clone(),
                                format!("`pow` vec3 exponent must be scalar or vec3, found {}", ev.kind()),
                            )
                            .with_help("use `pow(vec3, scalar)` or `pow(vec3, vec3)`"),
                        );
                        Value::Error
                    }
                }
                (Value::Vec4((x0, x1, x2, x3)), Value::Vec4((e0, e1, e2, e3))) => Value::Vec4((
                    Sx::Pow(Box::new(x0), Box::new(e0)),
                    Sx::Pow(Box::new(x1), Box::new(e1)),
                    Sx::Pow(Box::new(x2), Box::new(e2)),
                    Sx::Pow(Box::new(x3), Box::new(e3)),
                )),
                (Value::Vec4((x0, x1, x2, x3)), ev) => {
                    if let Some((se, _)) = crate::check::Checker::as_numeric_scalar(&ev) {
                        Value::Vec4((
                            Sx::Pow(Box::new(x0), Box::new(se.clone())),
                            Sx::Pow(Box::new(x1), Box::new(se.clone())),
                            Sx::Pow(Box::new(x2), Box::new(se.clone())),
                            Sx::Pow(Box::new(x3), Box::new(se)),
                        ))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                e.span.clone(),
                                format!("`pow` vec4 exponent must be scalar or vec4, found {}", ev.kind()),
                            )
                            .with_help("use `pow(vec4, scalar)` or `pow(vec4, vec4)`"),
                        );
                        Value::Error
                    }
                }
                (xv, ev) => {
                    ctx.diags.push(
                        crate::diag::Diag::error(
                            x.span.clone(),
                            format!(
                                "`pow` expects scalar/scalar, vec2/(scalar|vec2), vec3/(scalar|vec3), or vec4/(scalar|vec4), found {}/{}",
                                xv.kind(),
                                ev.kind(),
                            ),
                        )
                        .with_help("examples: `pow(2.0, 3.0)`, `pow((1,2), 2.0)`, `pow((1,2,3), 2.0)`, `pow((1,2,3,4), 2.0)`"),
                    );
                    Value::Error
                }
            }
        }
    }
}

builtin! {
    name = "atan2",
    signature = single {
        args(y: Scalar = "y coordinate", x: Scalar = "x coordinate"),
        result = Scalar,
        caps = PURE,
    },
    check = |_ctx, y, x| {
        Value::Scalar(Sx::Atan2(Box::new(y), Box::new(x)))
    }
}

builtin! {
    name = "step",
    signature = single {
        args(
            edge: Expr = "edge value (scalar, vec2, vec3, or vec4)",
            x: Expr = "input value (scalar, vec2, vec3, or vec4)"
        ),
        result = Scalar | Vec2 | Vec3 | Vec4,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, edge, x| {
        let ev = ctx.eval(edge).unwrap_or(Value::Error);
        let xv = ctx.eval(x).unwrap_or(Value::Error);

        if matches!(ev, Value::Error) || matches!(xv, Value::Error) {
            Value::Error
        } else if let (Some((se, _)), Some((sx, _))) = (
            crate::check::Checker::as_numeric_scalar(&ev),
            crate::check::Checker::as_numeric_scalar(&xv),
        ) {
            Value::Scalar(Sx::Step(Box::new(se), Box::new(sx)))
        } else {
            let edge_scalar = crate::check::Checker::as_numeric_scalar(&ev).map(|(s, _)| s);
            let x_scalar = crate::check::Checker::as_numeric_scalar(&xv).map(|(s, _)| s);
            match (ev, xv) {
                (Value::Vec2((e0, e1)), Value::Vec2((x0, x1))) => Value::Vec2((
                    Sx::Step(Box::new(e0), Box::new(x0)),
                    Sx::Step(Box::new(e1), Box::new(x1)),
                )),
                (Value::Vec2((e0, e1)), _) => {
                    if let Some(xs) = x_scalar {
                        Value::Vec2((
                            Sx::Step(Box::new(e0), Box::new(xs.clone())),
                            Sx::Step(Box::new(e1), Box::new(xs)),
                        ))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                x.span.clone(),
                                "`step` vec2 form expects x as scalar or vec2",
                            )
                            .with_help("use `step(vec2, scalar)` or `step(vec2, vec2)`"),
                        );
                        Value::Error
                    }
                }
                (_, Value::Vec2((x0, x1))) => {
                    if let Some(es) = edge_scalar {
                        Value::Vec2((
                            Sx::Step(Box::new(es.clone()), Box::new(x0)),
                            Sx::Step(Box::new(es), Box::new(x1)),
                        ))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                edge.span.clone(),
                                "`step` vec2 form expects edge as scalar or vec2",
                            )
                            .with_help("use `step(scalar, vec2)` or `step(vec2, vec2)`"),
                        );
                        Value::Error
                    }
                }
                (Value::Vec3((e0, e1, e2)), Value::Vec3((x0, x1, x2))) => Value::Vec3((
                    Sx::Step(Box::new(e0), Box::new(x0)),
                    Sx::Step(Box::new(e1), Box::new(x1)),
                    Sx::Step(Box::new(e2), Box::new(x2)),
                )),
                (Value::Vec3((e0, e1, e2)), _) => {
                    if let Some(xs) = x_scalar {
                        Value::Vec3((
                            Sx::Step(Box::new(e0), Box::new(xs.clone())),
                            Sx::Step(Box::new(e1), Box::new(xs.clone())),
                            Sx::Step(Box::new(e2), Box::new(xs)),
                        ))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                x.span.clone(),
                                "`step` vec3 form expects x as scalar or vec3",
                            )
                            .with_help("use `step(vec3, scalar)` or `step(vec3, vec3)`"),
                        );
                        Value::Error
                    }
                }
                (_, Value::Vec3((x0, x1, x2))) => {
                    if let Some(es) = edge_scalar {
                        Value::Vec3((
                            Sx::Step(Box::new(es.clone()), Box::new(x0)),
                            Sx::Step(Box::new(es.clone()), Box::new(x1)),
                            Sx::Step(Box::new(es), Box::new(x2)),
                        ))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                edge.span.clone(),
                                "`step` vec3 form expects edge as scalar or vec3",
                            )
                            .with_help("use `step(scalar, vec3)` or `step(vec3, vec3)`"),
                        );
                        Value::Error
                    }
                }
                (Value::Vec4((e0, e1, e2, e3)), Value::Vec4((x0, x1, x2, x3))) => Value::Vec4((
                    Sx::Step(Box::new(e0), Box::new(x0)),
                    Sx::Step(Box::new(e1), Box::new(x1)),
                    Sx::Step(Box::new(e2), Box::new(x2)),
                    Sx::Step(Box::new(e3), Box::new(x3)),
                )),
                (Value::Vec4((e0, e1, e2, e3)), _) => {
                    if let Some(xs) = x_scalar {
                        Value::Vec4((
                            Sx::Step(Box::new(e0), Box::new(xs.clone())),
                            Sx::Step(Box::new(e1), Box::new(xs.clone())),
                            Sx::Step(Box::new(e2), Box::new(xs.clone())),
                            Sx::Step(Box::new(e3), Box::new(xs)),
                        ))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                x.span.clone(),
                                "`step` vec4 form expects x as scalar or vec4",
                            )
                            .with_help("use `step(vec4, scalar)` or `step(vec4, vec4)`"),
                        );
                        Value::Error
                    }
                }
                (_, Value::Vec4((x0, x1, x2, x3))) => {
                    if let Some(es) = edge_scalar {
                        Value::Vec4((
                            Sx::Step(Box::new(es.clone()), Box::new(x0)),
                            Sx::Step(Box::new(es.clone()), Box::new(x1)),
                            Sx::Step(Box::new(es.clone()), Box::new(x2)),
                            Sx::Step(Box::new(es), Box::new(x3)),
                        ))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                edge.span.clone(),
                                "`step` vec4 form expects edge as scalar or vec4",
                            )
                            .with_help("use `step(scalar, vec4)` or `step(vec4, vec4)`"),
                        );
                        Value::Error
                    }
                }
                (ev, xv) => {
                    ctx.diags.push(
                        crate::diag::Diag::error(
                            edge.span.clone(),
                            format!(
                                "`step` expects scalar/scalar, vec2/vec3/vec4 with scalar/vector counterparts; found {}/{}",
                                ev.kind(),
                                xv.kind(),
                            ),
                        )
                        .with_help("examples: `step(0.5, x)`, `step((0,0), v2)`, `step((0,0,0), v3)`, `step(v4, 0.5)`"),
                    );
                    Value::Error
                }
            }
        }
    }
}

builtin! {
    name = "clamp",
    signature = single {
        args(
            x: Expr = "value to clamp (scalar, vec2, vec3, or vec4)",
            lo: Expr = "minimum value (scalar or matching vector)",
            hi: Expr = "maximum value (scalar or matching vector)"
        ),
        result = Scalar | Vec2 | Vec3 | Vec4,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, x, lo, hi| {
        let xv = ctx.eval(x).unwrap_or(Value::Error);
        let lov = ctx.eval(lo).unwrap_or(Value::Error);
        let hiv = ctx.eval(hi).unwrap_or(Value::Error);

        if [&xv, &lov, &hiv].iter().any(|value| crate::check::Checker::value_element_kind(value).is_some_and(|kind| kind != crate::typed_scalar::Kind::F32)) {
            return Some(typed_vector_math(ctx, naga::MathFunction::Clamp, &[(x, &xv), (lo, &lov), (hi, &hiv)]));
        }
        if matches!(xv, Value::Error) || matches!(lov, Value::Error) || matches!(hiv, Value::Error) {
            Value::Error
        } else if same_simple_value(&xv, &lov) && same_simple_value(&lov, &hiv) {
            xv
        } else if let (Some((sx, _)), Some((slo, _)), Some((shi, _))) = (
            crate::check::Checker::as_numeric_scalar(&xv),
            crate::check::Checker::as_numeric_scalar(&lov),
            crate::check::Checker::as_numeric_scalar(&hiv),
        ) {
            Value::Scalar(Sx::Clamp(Box::new(sx), Box::new(slo), Box::new(shi)))
        } else {
            let scalar_lo = crate::check::Checker::as_numeric_scalar(&lov).map(|(s, _)| s);
            let scalar_hi = crate::check::Checker::as_numeric_scalar(&hiv).map(|(s, _)| s);
            match (xv, lov, hiv) {
                (Value::Vec2(x2), Value::Vec2(l2), Value::Vec2(h2)) => Value::Vec2(vec2_clamp(x2, l2, h2)),
                (Value::Vec2((x0, x1)), _, _) => {
                    if let (Some(lo_s), Some(hi_s)) = (scalar_lo, scalar_hi) {
                        Value::Vec2(vec2_clamp((x0, x1), (lo_s.clone(), lo_s), (hi_s.clone(), hi_s)))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                x.span.clone(),
                                "`clamp` vec2 form expects vec2/vec2/vec2 or vec2/scalar/scalar",
                            )
                            .with_help("use `clamp(vec2, vec2, vec2)` or `clamp(vec2, 0.0, 1.0)`"),
                        );
                        Value::Error
                    }
                }
                (Value::Vec3(x3), Value::Vec3(l3), Value::Vec3(h3)) => Value::Vec3(vec3_clamp(x3, l3, h3)),
                (Value::Vec3((x0, x1, x2)), _, _) => {
                    if let (Some(lo_s), Some(hi_s)) = (scalar_lo, scalar_hi) {
                        Value::Vec3(vec3_clamp(
                            (x0, x1, x2),
                            (lo_s.clone(), lo_s.clone(), lo_s),
                            (hi_s.clone(), hi_s.clone(), hi_s),
                        ))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                x.span.clone(),
                                "`clamp` vec3 form expects vec3/vec3/vec3 or vec3/scalar/scalar",
                            )
                            .with_help("use `clamp(vec3, vec3, vec3)` or `clamp(vec3, 0.0, 1.0)`"),
                        );
                        Value::Error
                    }
                }
                (Value::Vec4(x4), Value::Vec4(l4), Value::Vec4(h4)) => Value::Vec4(vec4_clamp(x4, l4, h4)),
                (Value::Vec4((x0, x1, x2, x3)), _, _) => {
                    if let (Some(lo_s), Some(hi_s)) = (scalar_lo, scalar_hi) {
                        Value::Vec4(vec4_clamp(
                            (x0, x1, x2, x3),
                            (lo_s.clone(), lo_s.clone(), lo_s.clone(), lo_s),
                            (hi_s.clone(), hi_s.clone(), hi_s.clone(), hi_s),
                        ))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                x.span.clone(),
                                "`clamp` vec4 form expects vec4/vec4/vec4 or vec4/scalar/scalar",
                            )
                            .with_help("use `clamp(vec4, vec4, vec4)` or `clamp(vec4, 0.0, 1.0)`"),
                        );
                        Value::Error
                    }
                }
                (xv, lov, hiv) => {
                    ctx.diags.push(
                        crate::diag::Diag::error(
                            x.span.clone(),
                            format!(
                                "`clamp` expects scalar/scalar/scalar, vec2/vec3/vec4 with scalar/vector bounds; found {}/{}/{}",
                                xv.kind(),
                                lov.kind(),
                                hiv.kind(),
                            ),
                        )
                        .with_help("supported forms include clamp(s,lo,hi), clamp(v2,0,1), clamp(v3,0,1), clamp(v4,0,1), clamp(v4,v4,v4)"),
                    );
                    Value::Error
                }
            }
        }
    }
}

builtin! {
    name = "mix",
    signature = single {
        args(
            a: Expr = "first value (scalar, vec2, vec3, vec4, or shape)",
            b: Expr = "second value (scalar, vec2, vec3, vec4, or shape)",
            t: Expr = "interpolation factor (scalar, vec2, vec3, or vec4 for vector mix; scalar-like for shape mix)"
        ),
        result = Scalar | Vec2 | Vec3 | Vec4 | Shape,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, a, b, t| {
        let av = ctx.eval(a).unwrap_or(Value::Error);
        let bv = ctx.eval(b).unwrap_or(Value::Error);
        let tv = ctx.eval(t).unwrap_or(Value::Error);

        if matches!(av, Value::Error) || matches!(bv, Value::Error) || matches!(tv, Value::Error) {
            Value::Error
        } else if let (Some((sa, _)), Some((sb, _)), Some((st, _))) = (
            crate::check::Checker::as_numeric_scalar(&av),
            crate::check::Checker::as_numeric_scalar(&bv),
            crate::check::Checker::as_numeric_scalar(&tv),
        ) {
            Value::Scalar(Sx::Mix(Box::new(sa), Box::new(sb), Box::new(st)))
        } else if let (Value::Shape(a_shape), Value::Shape(b_shape)) = (&av, &bv) {
            if let Some((st, _)) = crate::check::Checker::as_numeric_scalar(&tv) {
                let out = ctx.hir.shape(Shape::Mix(*a_shape, *b_shape, st));
                ctx.note_shape_exactness(out, "mix");
                Value::Shape(out)
            } else {
                ctx.diags.push(
                    crate::diag::Diag::error(
                        t.span.clone(),
                        format!(
                            "`mix` shape interpolation factor must be scalar-like, found {}",
                            tv.kind()
                        ),
                    )
                    .with_help("use `mix(shape_a, shape_b, 0.5)` or another scalar expression for `t`"),
                );
                Value::Error
            }
        } else {
            match (av, bv, tv) {
                (Value::Vec2((ax, ay)), Value::Vec2((bx, by)), Value::Vec2((tx, ty))) => {
                    Value::Vec2((
                        Sx::Mix(Box::new(ax), Box::new(bx), Box::new(tx)),
                        Sx::Mix(Box::new(ay), Box::new(by), Box::new(ty)),
                    ))
                }
                (Value::Vec2((ax, ay)), Value::Vec2((bx, by)), tv) => {
                    if let Some((st, _)) = crate::check::Checker::as_numeric_scalar(&tv) {
                        Value::Vec2((
                            Sx::Mix(Box::new(ax), Box::new(bx), Box::new(st.clone())),
                            Sx::Mix(Box::new(ay), Box::new(by), Box::new(st)),
                        ))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                t.span.clone(),
                                format!(
                                    "`mix` vec2 interpolation factor must be scalar or vec2, found {}",
                                    tv.kind()
                                ),
                            )
                            .with_help("use `mix(vec2, vec2, scalar)` or `mix(vec2, vec2, vec2)`"),
                        );
                        Value::Error
                    }
                }
                (Value::Vec3((ax, ay, az)), Value::Vec3((bx, by, bz)), Value::Vec3((tx, ty, tz))) => {
                    Value::Vec3((
                        Sx::Mix(Box::new(ax), Box::new(bx), Box::new(tx)),
                        Sx::Mix(Box::new(ay), Box::new(by), Box::new(ty)),
                        Sx::Mix(Box::new(az), Box::new(bz), Box::new(tz)),
                    ))
                }
                (Value::Vec3((ax, ay, az)), Value::Vec3((bx, by, bz)), tv) => {
                    if let Some((st, _)) = crate::check::Checker::as_numeric_scalar(&tv) {
                        Value::Vec3((
                            Sx::Mix(Box::new(ax), Box::new(bx), Box::new(st.clone())),
                            Sx::Mix(Box::new(ay), Box::new(by), Box::new(st.clone())),
                            Sx::Mix(Box::new(az), Box::new(bz), Box::new(st)),
                        ))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                t.span.clone(),
                                format!(
                                    "`mix` vec3 interpolation factor must be scalar or vec3, found {}",
                                    tv.kind()
                                ),
                            )
                            .with_help("use `mix(vec3, vec3, scalar)` or `mix(vec3, vec3, vec3)`"),
                        );
                        Value::Error
                    }
                }
                (Value::Vec4((ax, ay, az, aw)), Value::Vec4((bx, by, bz, bw)), Value::Vec4((tx, ty, tz, tw))) => {
                    Value::Vec4((
                        Sx::Mix(Box::new(ax), Box::new(bx), Box::new(tx)),
                        Sx::Mix(Box::new(ay), Box::new(by), Box::new(ty)),
                        Sx::Mix(Box::new(az), Box::new(bz), Box::new(tz)),
                        Sx::Mix(Box::new(aw), Box::new(bw), Box::new(tw)),
                    ))
                }
                (Value::Vec4((ax, ay, az, aw)), Value::Vec4((bx, by, bz, bw)), tv) => {
                    if let Some((st, _)) = crate::check::Checker::as_numeric_scalar(&tv) {
                        Value::Vec4((
                            Sx::Mix(Box::new(ax), Box::new(bx), Box::new(st.clone())),
                            Sx::Mix(Box::new(ay), Box::new(by), Box::new(st.clone())),
                            Sx::Mix(Box::new(az), Box::new(bz), Box::new(st.clone())),
                            Sx::Mix(Box::new(aw), Box::new(bw), Box::new(st)),
                        ))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                t.span.clone(),
                                format!(
                                    "`mix` vec4 interpolation factor must be scalar or vec4, found {}",
                                    tv.kind()
                                ),
                            )
                            .with_help("use `mix(vec4, vec4, scalar)` or `mix(vec4, vec4, vec4)`"),
                        );
                        Value::Error
                    }
                }
                (av, bv, tv) => {
                    let av_kind = av.kind();
                    let bv_kind = bv.kind();
                    let tv_kind = tv.kind();
                    // Color/ColorField mixing: mix(color_a, color_b, t) — per-channel linear blend.
                    if let (Some(ca), Some(cb), Some((st, _))) = (
                        color_to_sx(av),
                        color_to_sx(bv),
                        crate::check::Checker::as_numeric_scalar(&tv),
                    ) {
                        let [ar, ag, ab, aa] = ca;
                        let [br, bg, bb, ba] = cb;
                        return Some(Value::ColorField {
                            rgba: [
                                Sx::Mix(Box::new(ar), Box::new(br), Box::new(st.clone())),
                                Sx::Mix(Box::new(ag), Box::new(bg), Box::new(st.clone())),
                                Sx::Mix(Box::new(ab), Box::new(bb), Box::new(st.clone())),
                                Sx::Mix(Box::new(aa), Box::new(ba), Box::new(st)),
                            ],
                            space: crate::check::ColorSpace::Linear,
                        });
                    }
                    ctx.diags.push(
                        crate::diag::Diag::error(
                            a.span.clone(),
                            format!(
                                "`mix` expects scalar/scalar/scalar, vec2/vec2/(scalar|vec2), vec3/vec3/(scalar|vec3), vec4/vec4/(scalar|vec4), color/color/scalar, or shape/shape/scalar, found {}/{}/{}",
                                av_kind,
                                bv_kind,
                                tv_kind,
                            ),
                        )
                        .with_help("examples: `mix(0.0, 1.0, 0.5)`, `mix((0,0,0), (1,1,1), 0.5)`, `mix(#ff0000, #0000ff, 0.5)`, or `mix(circle(...), box(...), 0.5)`"),
                    );
                    Value::Error
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::same_simple_value;
    use crate::check::Value;
    use crate::hir::Sx;

    #[test]
    fn same_simple_value_matches_scalar_and_vector_shapes() {
        assert!(same_simple_value(
            &Value::Scalar(Sx::Lit(1.0)),
            &Value::Scalar(Sx::Lit(1.0))
        ));
        assert!(same_simple_value(
            &Value::Vec2((Sx::Lit(1.0), Sx::Lit(2.0))),
            &Value::Vec2((Sx::Lit(1.0), Sx::Lit(2.0)))
        ));
        assert!(same_simple_value(
            &Value::Vec4((Sx::Lit(1.0), Sx::Lit(2.0), Sx::Lit(3.0), Sx::Lit(4.0))),
            &Value::Vec4((Sx::Lit(1.0), Sx::Lit(2.0), Sx::Lit(3.0), Sx::Lit(4.0)))
        ));
    }

    #[test]
    fn same_simple_value_rejects_mismatched_variants() {
        assert!(!same_simple_value(
            &Value::Scalar(Sx::Lit(1.0)),
            &Value::Distance(Sx::Lit(1.0))
        ));
        assert!(!same_simple_value(
            &Value::Vec2((Sx::Lit(1.0), Sx::Lit(2.0))),
            &Value::Vec3((Sx::Lit(1.0), Sx::Lit(2.0), Sx::Lit(3.0)))
        ));
    }
}

builtin! {
    name = "lerp",
    signature = single {
        args(
            a: Expr = "first value (scalar, vec2, vec3, vec4, or shape)",
            b: Expr = "second value (scalar, vec2, vec3, vec4, or shape)",
            t: Expr = "interpolation factor (scalar, vec2, vec3, or vec4 for vector lerp; scalar-like for shape lerp)"
        ),
        result = Scalar | Vec2 | Vec3 | Vec4 | Shape,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, a, b, t| {
        let av = ctx.eval(a).unwrap_or(Value::Error);
        let bv = ctx.eval(b).unwrap_or(Value::Error);
        let tv = ctx.eval(t).unwrap_or(Value::Error);

        if matches!(av, Value::Error) || matches!(bv, Value::Error) || matches!(tv, Value::Error) {
            Value::Error
        } else if let (Some((sa, _)), Some((sb, _)), Some((st, _))) = (
            crate::check::Checker::as_numeric_scalar(&av),
            crate::check::Checker::as_numeric_scalar(&bv),
            crate::check::Checker::as_numeric_scalar(&tv),
        ) {
            Value::Scalar(Sx::Mix(Box::new(sa), Box::new(sb), Box::new(st)))
        } else if let (Value::Shape(a_shape), Value::Shape(b_shape)) = (&av, &bv) {
            if let Some((st, _)) = crate::check::Checker::as_numeric_scalar(&tv) {
                let out = ctx.hir.shape(Shape::Mix(*a_shape, *b_shape, st));
                ctx.note_shape_exactness(out, "lerp");
                Value::Shape(out)
            } else {
                ctx.diags.push(
                    crate::diag::Diag::error(
                        t.span.clone(),
                        format!(
                            "`lerp` shape interpolation factor must be scalar-like, found {}",
                            tv.kind()
                        ),
                    )
                    .with_help("use `lerp(shape_a, shape_b, 0.5)` or another scalar expression for `t`"),
                );
                Value::Error
            }
        } else {
            match (av, bv, tv) {
                (Value::Vec2((ax, ay)), Value::Vec2((bx, by)), Value::Vec2((tx, ty))) => {
                    Value::Vec2((
                        Sx::Mix(Box::new(ax), Box::new(bx), Box::new(tx)),
                        Sx::Mix(Box::new(ay), Box::new(by), Box::new(ty)),
                    ))
                }
                (Value::Vec2((ax, ay)), Value::Vec2((bx, by)), tv) => {
                    if let Some((st, _)) = crate::check::Checker::as_numeric_scalar(&tv) {
                        Value::Vec2((
                            Sx::Mix(Box::new(ax), Box::new(bx), Box::new(st.clone())),
                            Sx::Mix(Box::new(ay), Box::new(by), Box::new(st)),
                        ))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                t.span.clone(),
                                format!(
                                    "`lerp` vec2 interpolation factor must be scalar or vec2, found {}",
                                    tv.kind()
                                ),
                            )
                            .with_help("use `lerp(vec2, vec2, scalar)` or `lerp(vec2, vec2, vec2)`"),
                        );
                        Value::Error
                    }
                }
                (Value::Vec3((ax, ay, az)), Value::Vec3((bx, by, bz)), Value::Vec3((tx, ty, tz))) => {
                    Value::Vec3((
                        Sx::Mix(Box::new(ax), Box::new(bx), Box::new(tx)),
                        Sx::Mix(Box::new(ay), Box::new(by), Box::new(ty)),
                        Sx::Mix(Box::new(az), Box::new(bz), Box::new(tz)),
                    ))
                }
                (Value::Vec3((ax, ay, az)), Value::Vec3((bx, by, bz)), tv) => {
                    if let Some((st, _)) = crate::check::Checker::as_numeric_scalar(&tv) {
                        Value::Vec3((
                            Sx::Mix(Box::new(ax), Box::new(bx), Box::new(st.clone())),
                            Sx::Mix(Box::new(ay), Box::new(by), Box::new(st.clone())),
                            Sx::Mix(Box::new(az), Box::new(bz), Box::new(st)),
                        ))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                t.span.clone(),
                                format!(
                                    "`lerp` vec3 interpolation factor must be scalar or vec3, found {}",
                                    tv.kind()
                                ),
                            )
                            .with_help("use `lerp(vec3, vec3, scalar)` or `lerp(vec3, vec3, vec3)`"),
                        );
                        Value::Error
                    }
                }
                (Value::Vec4((ax, ay, az, aw)), Value::Vec4((bx, by, bz, bw)), Value::Vec4((tx, ty, tz, tw))) => {
                    Value::Vec4((
                        Sx::Mix(Box::new(ax), Box::new(bx), Box::new(tx)),
                        Sx::Mix(Box::new(ay), Box::new(by), Box::new(ty)),
                        Sx::Mix(Box::new(az), Box::new(bz), Box::new(tz)),
                        Sx::Mix(Box::new(aw), Box::new(bw), Box::new(tw)),
                    ))
                }
                (Value::Vec4((ax, ay, az, aw)), Value::Vec4((bx, by, bz, bw)), tv) => {
                    if let Some((st, _)) = crate::check::Checker::as_numeric_scalar(&tv) {
                        Value::Vec4((
                            Sx::Mix(Box::new(ax), Box::new(bx), Box::new(st.clone())),
                            Sx::Mix(Box::new(ay), Box::new(by), Box::new(st.clone())),
                            Sx::Mix(Box::new(az), Box::new(bz), Box::new(st.clone())),
                            Sx::Mix(Box::new(aw), Box::new(bw), Box::new(st)),
                        ))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                t.span.clone(),
                                format!(
                                    "`lerp` vec4 interpolation factor must be scalar or vec4, found {}",
                                    tv.kind()
                                ),
                            )
                            .with_help("use `lerp(vec4, vec4, scalar)` or `lerp(vec4, vec4, vec4)`"),
                        );
                        Value::Error
                    }
                }
                (av, bv, tv) => {
                    ctx.diags.push(
                        crate::diag::Diag::error(
                            a.span.clone(),
                            format!(
                                    "`lerp` expects scalar/scalar/scalar, vec2/vec2/(scalar|vec2), vec3/vec3/(scalar|vec3), vec4/vec4/(scalar|vec4), or shape/shape/scalar, found {}/{}/{}",
                                av.kind(),
                                bv.kind(),
                                tv.kind(),
                            ),
                        )
                            .with_help("examples: `lerp(0.0, 1.0, 0.5)`, `lerp((0,0,0), (1,1,1), 0.5)`, or `lerp(circle(...), box(...), 0.5)`"),
                    );
                    Value::Error
                }
            }
        }
    }
}

builtin! {
    name = "saturate",
    signature = single {
        args(x: Expr = "value (scalar, vec2, vec3, or vec4)"),
        result = Scalar | Vec2 | Vec3 | Vec4,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, x| {
        let xv = ctx.eval(x).unwrap_or(Value::Error);

        if matches!(xv, Value::Error) {
            Value::Error
        } else if let Some((sx, _)) = crate::check::Checker::as_numeric_scalar(&xv) {
            Value::Scalar(Sx::Clamp(
                Box::new(sx),
                Box::new(Sx::Lit(0.0)),
                Box::new(Sx::Lit(1.0)),
            ))
        } else {
            match xv {
                Value::Vec2((x0, y0)) => Value::Vec2((
                    Sx::Clamp(
                        Box::new(x0),
                        Box::new(Sx::Lit(0.0)),
                        Box::new(Sx::Lit(1.0)),
                    ),
                    Sx::Clamp(
                        Box::new(y0),
                        Box::new(Sx::Lit(0.0)),
                        Box::new(Sx::Lit(1.0)),
                    ),
                )),
                Value::Vec3((x0, y0, z0)) => Value::Vec3((
                    Sx::Clamp(
                        Box::new(x0),
                        Box::new(Sx::Lit(0.0)),
                        Box::new(Sx::Lit(1.0)),
                    ),
                    Sx::Clamp(
                        Box::new(y0),
                        Box::new(Sx::Lit(0.0)),
                        Box::new(Sx::Lit(1.0)),
                    ),
                    Sx::Clamp(
                        Box::new(z0),
                        Box::new(Sx::Lit(0.0)),
                        Box::new(Sx::Lit(1.0)),
                    ),
                )),
                Value::Vec4((x0, y0, z0, w0)) => Value::Vec4((
                    Sx::Clamp(
                        Box::new(x0),
                        Box::new(Sx::Lit(0.0)),
                        Box::new(Sx::Lit(1.0)),
                    ),
                    Sx::Clamp(
                        Box::new(y0),
                        Box::new(Sx::Lit(0.0)),
                        Box::new(Sx::Lit(1.0)),
                    ),
                    Sx::Clamp(
                        Box::new(z0),
                        Box::new(Sx::Lit(0.0)),
                        Box::new(Sx::Lit(1.0)),
                    ),
                    Sx::Clamp(
                        Box::new(w0),
                        Box::new(Sx::Lit(0.0)),
                        Box::new(Sx::Lit(1.0)),
                    ),
                )),
                v => {
                    ctx.diags.push(
                        crate::diag::Diag::error(
                            x.span.clone(),
                            format!("`saturate` expects scalar, vec2, vec3, or vec4, found {}", v.kind()),
                        )
                        .with_help("use `saturate(scalar)`, `saturate(vec2)`, `saturate(vec3)`, or `saturate(vec4)`"),
                    );
                    Value::Error
                }
            }
        }
    }
}

builtin! {
    name = "desaturate",
    signature = single {
        args(
            color: ColorExpr = "base color",
            by: Scalar = "desaturation amount"
        ),
        result = Color,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, color, by| {
        match require_color_transform_args(ctx, color, by.clone(), "desaturate") {
            Some([r, g, b, a]) => {
                let grey = color_luma(&[r.clone(), g.clone(), b.clone(), a.clone()]);
                Value::ColorField {
                    rgba: [
                        color_mix(r, grey.clone(), by.clone()),
                        color_mix(g, grey.clone(), by.clone()),
                        color_mix(b, grey, by),
                        a,
                    ],
                    space: crate::check::ColorSpace::Linear,
                }
            }
            None => Value::Error,
        }
    }
}

builtin! {
    name = "darken",
    signature = single {
        args(
            color: ColorExpr = "base color",
            by: Scalar = "darkening amount"
        ),
        result = Color,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, color, by| {
        match require_color_transform_args(ctx, color, by.clone(), "darken") {
            Some([r, g, b, a]) => Value::ColorField {
                rgba: [
                    color_mix(r, Sx::Lit(0.0), by.clone()),
                    color_mix(g, Sx::Lit(0.0), by.clone()),
                    color_mix(b, Sx::Lit(0.0), by),
                    a,
                ],
                space: crate::check::ColorSpace::Linear,
            },
            None => Value::Error,
        }
    }
}

builtin! {
    name = "lighten",
    signature = single {
        args(
            color: ColorExpr = "base color",
            by: Scalar = "lightening amount"
        ),
        result = Color,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, color, by| {
        match require_color_transform_args(ctx, color, by.clone(), "lighten") {
            Some([r, g, b, a]) => Value::ColorField {
                rgba: [
                    color_mix(r, Sx::Lit(1.0), by.clone()),
                    color_mix(g, Sx::Lit(1.0), by.clone()),
                    color_mix(b, Sx::Lit(1.0), by),
                    a,
                ],
                space: crate::check::ColorSpace::Linear,
            },
            None => Value::Error,
        }
    }
}

fn saturate_color_check_impl(
    ctx: &mut crate::check::Checker,
    bag: &mut crate::check::ArgBag<'_>,
) -> Option<Value> {
    let color = bag.require("color", &mut ctx.diags)?;
    let by_expr = bag.require("by", &mut ctx.diags)?;
    let by_value = ctx.eval(by_expr)?;
    let Some((by, _kind)) = crate::check::Checker::as_numeric_scalar(&by_value) else {
        if !matches!(by_value, Value::Error) {
            ctx.diags.push(
                crate::diag::Diag::error(
                    by_expr.span.clone(),
                    format!(
                        "`saturate` amount must be scalar, found {}",
                        by_value.kind()
                    ),
                )
                .with_help("use `saturate(color, by: amount)` with a scalar amount"),
            );
        }
        return Some(Value::Error);
    };

    Some(
        match require_color_transform_args(ctx, color, by.clone(), "saturate") {
            Some([r, g, b, a]) => {
                let grey = color_luma(&[r.clone(), g.clone(), b.clone(), a.clone()]);
                let factor = Sx::Add(Box::new(Sx::Lit(1.0)), Box::new(by));
                let sat = |channel: Sx| {
                    let delta = Sx::Sub(Box::new(channel), Box::new(grey.clone()));
                    clamp01(Sx::Add(
                        Box::new(grey.clone()),
                        Box::new(Sx::Mul(Box::new(delta), Box::new(factor.clone()))),
                    ))
                };
                Value::ColorField {
                    rgba: [sat(r), sat(g), sat(b), a],
                    space: crate::check::ColorSpace::Linear,
                }
            }
            None => Value::Error,
        },
    )
}

inventory::submit! {
    crate::registry::BuiltinDecl {
        id: crate::registry::BuiltinId("saturate_color"),
        name: "saturate",
        signature: crate::registry::BuiltinSignature {
            receiver: None,
            args: &[
                crate::registry::BuiltinArgDecl::required(
                    "color",
                    crate::registry::TypeRef::Primitive(crate::registry::PrimitiveType::Color),
                    "base color",
                ),
                crate::registry::BuiltinArgDecl::required(
                    "by",
                    crate::registry::TypeRef::Primitive(crate::registry::PrimitiveType::Scalar),
                    "saturation boost amount",
                ),
            ],
            result: crate::registry::TypeRef::Primitive(crate::registry::PrimitiveType::Color),
            result_alternatives: &[],
            caps: crate::builtin_catalog::BuiltinCaps::from_bits_retain(
                crate::builtin_catalog::BuiltinCaps::PURE.bits()
                    | crate::builtin_catalog::BuiltinCaps::PIPEABLE.bits()
                    | crate::builtin_catalog::BuiltinCaps::COLOR_OUT.bits(),
            ),
        },
        lowering: crate::registry::BuiltinLowering::Impl(saturate_color_check_impl),
        discriminator: None,
        docs: "Adjust color saturation by blending away from luminance grey.",
    }
}

builtin! {
    name = "ddx",
    signature = single {
        args(x: Scalar = "value"),
        result = Scalar,
        caps = PURE,
    },
    check = |_ctx, x| {
        Value::Scalar(Sx::Ddx(Box::new(x)))
    }
}

builtin! {
    name = "ddy",
    signature = single {
        args(x: Scalar = "value"),
        result = Scalar,
        caps = PURE,
    },
    check = |_ctx, x| {
        Value::Scalar(Sx::Ddy(Box::new(x)))
    }
}

builtin! {
    name = "fwidth",
    signature = single {
        args(x: Scalar = "value"),
        result = Scalar,
        caps = PURE,
    },
    check = |_ctx, x| {
        Value::Scalar(Sx::Fwidth(Box::new(x)))
    }
}

builtin! {
    name = "smoothstep",
    signature = single {
        args(
            lo: Expr = "lower edge (scalar, vec2, vec3, or vec4)",
            hi: Expr = "upper edge (scalar, vec2, vec3, or vec4)",
            x: Expr = "input value (scalar, vec2, vec3, or vec4)"
        ),
        result = Scalar | Vec2 | Vec3 | Vec4,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, lo, hi, x| {
        let lov = ctx.eval(lo).unwrap_or(Value::Error);
        let hiv = ctx.eval(hi).unwrap_or(Value::Error);
        let xv = ctx.eval(x).unwrap_or(Value::Error);

        if matches!(lov, Value::Error) || matches!(hiv, Value::Error) || matches!(xv, Value::Error) {
            Value::Error
        } else if let (Some((slo, _)), Some((shi, _)), Some((sx, _))) = (
            crate::check::Checker::as_numeric_scalar(&lov),
            crate::check::Checker::as_numeric_scalar(&hiv),
            crate::check::Checker::as_numeric_scalar(&xv),
        ) {
            Value::Scalar(Sx::SmoothStep(Box::new(slo), Box::new(shi), Box::new(sx)))
        } else {
            let lo_s = crate::check::Checker::as_numeric_scalar(&lov).map(|(s, _)| s);
            let hi_s = crate::check::Checker::as_numeric_scalar(&hiv).map(|(s, _)| s);
            let x_s = crate::check::Checker::as_numeric_scalar(&xv).map(|(s, _)| s);

            match (lov, hiv, xv) {
                (Value::Vec2((l0, l1)), Value::Vec2((h0, h1)), Value::Vec2((x0, x1))) => Value::Vec2((
                    Sx::SmoothStep(Box::new(l0), Box::new(h0), Box::new(x0)),
                    Sx::SmoothStep(Box::new(l1), Box::new(h1), Box::new(x1)),
                )),
                (Value::Vec2((l0, l1)), Value::Vec2((h0, h1)), _) => {
                    if let Some(xs) = x_s {
                        Value::Vec2((
                            Sx::SmoothStep(Box::new(l0), Box::new(h0), Box::new(xs.clone())),
                            Sx::SmoothStep(Box::new(l1), Box::new(h1), Box::new(xs)),
                        ))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                x.span.clone(),
                                "`smoothstep` vec2 form expects x as scalar or vec2",
                            )
                            .with_help("use `smoothstep(vec2, vec2, scalar)` or `smoothstep(vec2, vec2, vec2)`"),
                        );
                        Value::Error
                    }
                }
                (Value::Vec3((l0, l1, l2)), Value::Vec3((h0, h1, h2)), Value::Vec3((x0, x1, x2))) => Value::Vec3((
                    Sx::SmoothStep(Box::new(l0), Box::new(h0), Box::new(x0)),
                    Sx::SmoothStep(Box::new(l1), Box::new(h1), Box::new(x1)),
                    Sx::SmoothStep(Box::new(l2), Box::new(h2), Box::new(x2)),
                )),
                (Value::Vec3((l0, l1, l2)), Value::Vec3((h0, h1, h2)), _) => {
                    if let Some(xs) = x_s {
                        Value::Vec3((
                            Sx::SmoothStep(Box::new(l0), Box::new(h0), Box::new(xs.clone())),
                            Sx::SmoothStep(Box::new(l1), Box::new(h1), Box::new(xs.clone())),
                            Sx::SmoothStep(Box::new(l2), Box::new(h2), Box::new(xs)),
                        ))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                x.span.clone(),
                                "`smoothstep` vec3 form expects x as scalar or vec3",
                            )
                            .with_help("use `smoothstep(vec3, vec3, scalar)` or `smoothstep(vec3, vec3, vec3)`"),
                        );
                        Value::Error
                    }
                }
                (Value::Vec4((l0, l1, l2, l3)), Value::Vec4((h0, h1, h2, h3)), Value::Vec4((x0, x1, x2, x3))) => {
                    Value::Vec4((
                        Sx::SmoothStep(Box::new(l0), Box::new(h0), Box::new(x0)),
                        Sx::SmoothStep(Box::new(l1), Box::new(h1), Box::new(x1)),
                        Sx::SmoothStep(Box::new(l2), Box::new(h2), Box::new(x2)),
                        Sx::SmoothStep(Box::new(l3), Box::new(h3), Box::new(x3)),
                    ))
                }
                (Value::Vec4((l0, l1, l2, l3)), Value::Vec4((h0, h1, h2, h3)), _) => {
                    if let Some(xs) = x_s {
                        Value::Vec4((
                            Sx::SmoothStep(Box::new(l0), Box::new(h0), Box::new(xs.clone())),
                            Sx::SmoothStep(Box::new(l1), Box::new(h1), Box::new(xs.clone())),
                            Sx::SmoothStep(Box::new(l2), Box::new(h2), Box::new(xs.clone())),
                            Sx::SmoothStep(Box::new(l3), Box::new(h3), Box::new(xs)),
                        ))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                x.span.clone(),
                                "`smoothstep` vec4 form expects x as scalar or vec4",
                            )
                            .with_help("use `smoothstep(vec4, vec4, scalar)` or `smoothstep(vec4, vec4, vec4)`"),
                        );
                        Value::Error
                    }
                }
                (lov, hiv, xv) => {
                    if let (Some(ls), Some(hs), Some(xs)) = (lo_s, hi_s, x_s) {
                        Value::Scalar(Sx::SmoothStep(Box::new(ls), Box::new(hs), Box::new(xs)))
                    } else {
                        ctx.diags.push(
                            crate::diag::Diag::error(
                                lo.span.clone(),
                                format!(
                                        "`smoothstep` expects scalar/scalar/scalar, vec2/vec2/(scalar|vec2), vec3/vec3/(scalar|vec3), or vec4/vec4/(scalar|vec4), found {}/{}/{}",
                                    lov.kind(),
                                    hiv.kind(),
                                    xv.kind(),
                                ),
                            )
                                .with_help("examples: `smoothstep(0,1,x)`, `smoothstep(v2a,v2b,x)`, `smoothstep(v3a,v3b,x)`, `smoothstep(v4a,v4b,v4x)`"),
                        );
                        Value::Error
                    }
                }
            }
        }
    }
}

builtin! {
    name = "select",
    signature = single {
        args(
            false_value: Expr = "value selected when condition is false",
            true_value: Expr = "value selected when condition is true",
            condition: Expr = "scalar or matching vector condition"
        ),
        result = Scalar | Vec2 | Vec3 | Vec4 | Color,
        caps = PURE,
    },
    check = |ctx, false_value, true_value, condition| {
        let mut a = ctx.eval(false_value)?;
        let mut b = ctx.eval(true_value)?;
        if let (Some(ak), Some(bk)) = (crate::check::Checker::value_element_kind(&a), crate::check::Checker::value_element_kind(&b))
            && ak != bk {
                let kind = if ak != crate::typed_scalar::Kind::F32 { ak } else { bk };
                if matches!(a, Value::Scalar(_)) && matches!(b, Value::Scalar(_)) {
                    a = ctx.eval_scalar_expected(false_value, Some(kind))?;
                    b = ctx.eval_scalar_expected(true_value, Some(kind))?;
                } else {
                    ctx.diags.push(crate::diag::Diag::error(false_value.span.clone(), "select requires matching element types"));
                    return Some(Value::Error);
                }
        }
        let cond = ctx.eval(condition)?;
        let lane = crate::typed_scalar::Scalar::select;
        match (a, b, cond) {
            (a @ (Value::Color { .. } | Value::ColorField { .. }), b @ (Value::Color { .. } | Value::ColorField { .. }), Value::Scalar(c)) => ctx.merge_conditional_values("select", "", c, b, a, &false_value.span)?,
            (Value::Scalar(a), Value::Scalar(b), Value::Scalar(c)) => Value::Scalar(lane(a,b,c)),
            (Value::Vec2((ax,ay)), Value::Vec2((bx,by)), Value::Scalar(c)) => Value::Vec2((lane(ax,bx,c.clone()),lane(ay,by,c))),
            (Value::Vec3((ax,ay,az)), Value::Vec3((bx,by,bz)), Value::Scalar(c)) => Value::Vec3((lane(ax,bx,c.clone()),lane(ay,by,c.clone()),lane(az,bz,c))),
            (Value::Vec4((ax,ay,az,aw)), Value::Vec4((bx,by,bz,bw)), Value::Scalar(c)) => Value::Vec4((lane(ax,bx,c.clone()),lane(ay,by,c.clone()),lane(az,bz,c.clone()),lane(aw,bw,c))),
            (Value::Vec2((ax,ay)), Value::Vec2((bx,by)), Value::Vec2((cx,cy))) => Value::Vec2((lane(ax,bx,cx),lane(ay,by,cy))),
            (Value::Vec3((ax,ay,az)), Value::Vec3((bx,by,bz)), Value::Vec3((cx,cy,cz))) => Value::Vec3((lane(ax,bx,cx),lane(ay,by,cy),lane(az,bz,cz))),
            (Value::Vec4((ax,ay,az,aw)), Value::Vec4((bx,by,bz,bw)), Value::Vec4((cx,cy,cz,cw))) => Value::Vec4((lane(ax,bx,cx),lane(ay,by,cy),lane(az,bz,cz),lane(aw,bw,cw))),
            (a,b,c) => {
                ctx.diags.push(crate::diag::Diag::error(false_value.span.clone(), format!("select expects matching scalar/vector values and a scalar or matching vector condition, found {}/{}/{}", a.kind(),b.kind(),c.kind())));
                Value::Error
            }
        }
    }
}
