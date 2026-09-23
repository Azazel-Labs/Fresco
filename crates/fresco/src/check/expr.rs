use super::*;
use std::hash::{Hash, Hasher};
use tracing::info;
use tracing::trace_span;
use web_time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScalarKind {
    Plain,
    Distance,
    Coverage,
    Mask,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SymbolicFactorOp {
    Max,
    Min,
    Sub,
    Div,
}

impl Checker {
    pub(super) fn map_value_lanes(value: Value, mut map: impl FnMut(Sx) -> Sx) -> Value {
        match value {
            Value::Scalar(s) => Value::Scalar(map(s)),
            Value::Vec2((x, y)) => Value::Vec2((map(x), map(y))),
            Value::Vec3((x, y, z)) => Value::Vec3((map(x), map(y), map(z))),
            Value::Vec4((x, y, z, w)) => Value::Vec4((map(x), map(y), map(z), map(w))),
            other => other,
        }
    }

    pub(super) fn value_element_kind(value: &Value) -> Option<crate::typed_scalar::Kind> {
        match value {
            Value::Scalar(s)
            | Value::Vec2((s, _))
            | Value::Vec3((s, _, _))
            | Value::Vec4((s, _, _, _)) => Some(s.scalar_kind()),
            _ => None,
        }
    }

    pub(super) fn eval_expected(&mut self, expression: &SExpr, ty: &str) -> Option<Value> {
        if let Some((element, _)) = hir::parse_array_param_type(ty)
            && let Expr::Array(items) = &expression.node
        {
            return items
                .iter()
                .map(|item| self.eval_expected(item, element))
                .collect::<Option<Vec<_>>>()
                .map(Value::Array);
        }
        self.eval_scalar_expected(expression, crate::typed_scalar::Kind::parse(ty))
    }

    pub(super) fn eval_scalar_expected(
        &mut self,
        expression: &SExpr,
        kind: Option<crate::typed_scalar::Kind>,
    ) -> Option<Value> {
        use crate::typed_scalar::Scalar;
        let Some(kind) = kind else {
            return self.eval(expression);
        };
        let number = match &expression.node {
            Expr::Num(value, Unit::None) => Some(*value),
            Expr::Unary(UnOp::Neg, value) => match value.node {
                Expr::Num(v, Unit::None) => Some(-v),
                _ => None,
            },
            _ => None,
        };
        if let Some(number) = number {
            if (matches!(
                kind,
                crate::typed_scalar::Kind::I32 | crate::typed_scalar::Kind::U32
            ) && number.fract() != 0.0)
                || kind == crate::typed_scalar::Kind::Bool
            {
                self.diags.push(Diag::error(
                    expression.span.clone(),
                    format!(
                        "literal cannot be implicitly converted to {}; use an explicit conversion",
                        kind.name()
                    ),
                ));
                return None;
            }
            return match Scalar::number(number, kind) {
                Ok(value) => Some(Value::Scalar(value)),
                Err(message) => {
                    self.diags
                        .push(Diag::error(expression.span.clone(), message));
                    None
                }
            };
        }
        let value = if let Expr::Binary(op, left, right) = &expression.node {
            if matches!(
                op,
                BinOp::Add
                    | BinOp::Sub
                    | BinOp::Mul
                    | BinOp::Div
                    | BinOp::Mod
                    | BinOp::Shl
                    | BinOp::Shr
                    | BinOp::BitXor
                    | BinOp::Union
                    | BinOp::Intersect
            ) {
                self.eval_binary_expected(*op, left, right, &expression.span, Some(kind))?
            } else {
                self.eval(expression)?
            }
        } else {
            self.eval(expression)?
        };
        if let Value::Scalar(scalar) = &value
            && scalar.scalar_kind() != kind
        {
            self.diags.push(Diag::error(
                expression.span.clone(),
                format!(
                    "expected {}, found {}; use an explicit conversion",
                    kind.name(),
                    scalar.scalar_kind().name()
                ),
            ));
            return None;
        }
        Some(value)
    }

    fn eval_static_scalar_from_expr(&mut self, expr: &SExpr, what: &str) -> Option<f32> {
        let value = self.eval(expr)?;
        let Some((sx, _kind)) = Self::as_numeric_scalar(&value) else {
            self.diags.push(
                Diag::error(
                    expr.span.clone(),
                    format!("{what} expected scalar expression"),
                )
                .with_label("type mismatch in path command"),
            );
            return None;
        };
        let Some(v) = Self::try_eval_static_scalar(&sx) else {
            self.diags.push(
                Diag::error(
                    expr.span.clone(),
                    format!("{what} must be compile-time constant for path lowering"),
                )
                .with_help("use numeric literals or constant scalar expressions"),
            );
            return None;
        };
        Some(v)
    }

    fn eval_static_vec2_from_expr(&mut self, expr: &SExpr, what: &str) -> Option<(f32, f32)> {
        let value = self.eval(expr)?;
        let Value::Vec2((x, y)) = value else {
            self.diags.push(
                Diag::error(
                    expr.span.clone(),
                    format!("{what} expected vec2 expression"),
                )
                .with_label("type mismatch in path command"),
            );
            return None;
        };
        let Some(xv) = Self::try_eval_static_scalar(&x) else {
            self.diags.push(
                Diag::error(
                    expr.span.clone(),
                    format!("{what} x component must be compile-time constant"),
                )
                .with_help("use numeric literals or constant scalar expressions"),
            );
            return None;
        };
        let Some(yv) = Self::try_eval_static_scalar(&y) else {
            self.diags.push(
                Diag::error(
                    expr.span.clone(),
                    format!("{what} y component must be compile-time constant"),
                )
                .with_help("use numeric literals or constant scalar expressions"),
            );
            return None;
        };
        Some((xv, yv))
    }

    fn build_path_profile(&mut self, commands: &[PathCommand], span: &Span) -> Option<PathValue> {
        let mut primitives: Vec<hir::PathPrimitive> = Vec::new();
        let mut cursor: Option<(f32, f32)> = None;

        for cmd in commands {
            match &cmd.kind {
                PathCommandKind::Move { to } => {
                    let p = self.eval_static_vec2_from_expr(to, "`move` target")?;
                    cursor = Some(p);
                }
                PathCommandKind::Line { to } => {
                    let Some(cur) = cursor else {
                        self.diags.push(
                            Diag::error(cmd.span.clone(), "`line` requires a preceding `move`")
                                .with_help("start path blocks with `move (x, y)`"),
                        );
                        return Some(
                            PathValue::from_points(0, vec![(0.0, 0.0), (1.0, 0.0)])
                                .expect("static fallback path"),
                        );
                    };
                    let p = self.eval_static_vec2_from_expr(to, "`line` target")?;
                    primitives.push(hir::PathPrimitive::Line { from: cur, to: p });
                    cursor = Some(p);
                }
                PathCommandKind::Cubic { c1, c2, to } => {
                    let Some(p0) = cursor else {
                        self.diags.push(
                            Diag::error(cmd.span.clone(), "`cubic` requires a preceding `move`")
                                .with_help("start path blocks with `move (x, y)`"),
                        );
                        return Some(
                            PathValue::from_points(0, vec![(0.0, 0.0), (1.0, 0.0)])
                                .expect("static fallback path"),
                        );
                    };
                    let p1 = self.eval_static_vec2_from_expr(c1, "`cubic` control point c1")?;
                    let p2 = self.eval_static_vec2_from_expr(c2, "`cubic` control point c2")?;
                    let p3 = self.eval_static_vec2_from_expr(to, "`cubic` target")?;
                    primitives.push(hir::PathPrimitive::Cubic { p0, p1, p2, p3 });
                    cursor = Some(p3);
                }
                PathCommandKind::Arc {
                    center,
                    radius,
                    sweep,
                } => {
                    let Some(cur) = cursor else {
                        self.diags.push(
                            Diag::error(cmd.span.clone(), "`arc` requires a preceding `move`")
                                .with_help("start path blocks with `move (x, y)`"),
                        );
                        return Some(
                            PathValue::from_points(0, vec![(0.0, 0.0), (1.0, 0.0)])
                                .expect("static fallback path"),
                        );
                    };
                    let c = self.eval_static_vec2_from_expr(center, "`arc` center")?;
                    let r = self.eval_static_scalar_from_expr(radius, "`arc` radius")?;
                    let sweep_rad = self.eval_static_scalar_from_expr(sweep, "`arc` sweep")?;
                    let r = r.abs().max(f32::EPSILON);
                    primitives.push(hir::PathPrimitive::Arc {
                        from: cur,
                        center: c,
                        radius: r,
                        sweep: sweep_rad,
                    });
                    let end = {
                        let start = (cur.1 - c.1).atan2(cur.0 - c.0);
                        let a = start + sweep_rad;
                        (c.0 + r * a.cos(), c.1 + r * a.sin())
                    };
                    cursor = Some(end);
                }
            }
        }

        if let Some(path) =
            self.register_path_profile_from_primitives(primitives, span, "path block")
        {
            return Some(path);
        }

        self.diags.push(
            Diag::error(
                span.clone(),
                "path block must yield at least two distinct points",
            )
            .with_help("include `move` followed by one or more drawing commands"),
        );
        Some(PathValue::from_points(0, vec![(0.0, 0.0), (1.0, 0.0)]).expect("static fallback path"))
    }

    pub(super) fn is_anchor_variant_name(name: &str) -> bool {
        matches!(
            name,
            "center"
                | "top_center"
                | "bottom_center"
                | "left_center"
                | "right_center"
                | "top_left"
                | "top_right"
                | "bottom_left"
                | "bottom_right"
        )
    }

    fn anchor_variant_value(name: &str) -> Option<Value> {
        anchor_variant_coords(name).map(|(x, y)| Value::Vec2((Sx::Lit(x), Sx::Lit(y))))
    }

    fn resolve_anchor_value_name(&mut self, name: &str, span: &Span) -> Option<Value> {
        if let Some((enum_name, variant_name)) = name.split_once('.') {
            if enum_name != "Anchor" {
                return None;
            }
            if variant_name.contains('.') {
                self.diags.push(
                    Diag::error(span.clone(), format!("invalid enum variant path `{name}`"))
                        .with_help("use `Anchor.<variant>`, for example `Anchor.center`"),
                );
                return Some(Value::Error);
            }

            let Some(anchor_def) = self.enum_defs.get("Anchor") else {
                self.diags.push(
                    Diag::error(span.clone(), "enum `Anchor` is not available")
                        .with_help("ensure stdlib enums are loaded before using anchor literals"),
                );
                return Some(Value::Error);
            };

            if !anchor_def.variants.contains_key(variant_name) {
                self.diags.push(
                    Diag::error(
                        span.clone(),
                        format!("unknown anchor variant `{variant_name}`"),
                    )
                    .with_help("supported anchor variants: center, top_center, bottom_center, left_center, right_center, top_left, top_right, bottom_left, bottom_right"),
                );
                return Some(Value::Error);
            }

            return Some(Self::anchor_variant_value(variant_name).unwrap_or(Value::Error));
        }

        if !Self::is_anchor_variant_name(name) {
            return None;
        }

        let owners: Vec<&str> = self
            .enum_defs
            .iter()
            .filter(|(enum_name, _)| crate::registry::enum_allows_unqualified_values(enum_name))
            .filter_map(|(enum_name, enum_def)| {
                enum_def
                    .variants
                    .contains_key(name)
                    .then_some(enum_name.as_str())
            })
            .collect();
        if owners.is_empty() {
            return None;
        }
        if owners.len() > 1 {
            self.diags.push(
                Diag::error(span.clone(), format!("ambiguous enum variant `{name}`")).with_help(
                    format!(
                        "qualify the variant: {}",
                        owners
                            .iter()
                            .map(|owner| format!("`{owner}.{name}`"))
                            .collect::<Vec<_>>()
                            .join(" or ")
                    ),
                ),
            );
            return Some(Value::Error);
        }

        if owners[0] != "Anchor" {
            return None;
        }

        Some(Self::anchor_variant_value(name).unwrap_or(Value::Error))
    }

    fn enum_variant_scalar_value(enum_def: &EnumDef, variant_name: &str) -> Value {
        let mut variants = enum_def
            .variants
            .iter()
            .map(|(name, span)| (name.as_str(), span.start))
            .collect::<Vec<_>>();
        variants.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(b.0)));

        let ordinal = variants
            .iter()
            .position(|(name, _)| *name == variant_name)
            .unwrap_or(0) as f32;
        Value::Scalar(Sx::Lit(ordinal))
    }

    fn resolve_enum_value_name(&mut self, name: &str, span: &Span) -> Option<Value> {
        if let Some((enum_name, variant_name)) = name.split_once('.') {
            if !self.enum_defs.contains_key(enum_name) {
                return None;
            }
            if variant_name.contains('.') {
                self.diags.push(
                    Diag::error(span.clone(), format!("invalid enum variant path `{name}`"))
                        .with_help("use `<Enum>.<variant>`, for example `YAxis.down`"),
                );
                return Some(Value::Error);
            }

            let Some(enum_def) = self.enum_defs.get(enum_name) else {
                self.diags.push(
                    Diag::error(span.clone(), format!("unknown enum `{enum_name}`"))
                        .with_help("declare the enum before using it"),
                );
                return Some(Value::Error);
            };

            if !enum_def.variants.contains_key(variant_name) {
                self.diags.push(
                    Diag::error(
                        span.clone(),
                        format!("unknown variant `{variant_name}` on enum `{enum_name}`"),
                    )
                    .with_help("use a declared enum variant name"),
                );
                return Some(Value::Error);
            }

            return Some(Self::enum_variant_scalar_value(enum_def, variant_name));
        }

        let owners: Vec<&str> = self
            .enum_defs
            .iter()
            .filter(|(enum_name, _)| crate::registry::enum_allows_unqualified_values(enum_name))
            .filter_map(|(enum_name, enum_def)| {
                enum_def
                    .variants
                    .contains_key(name)
                    .then_some(enum_name.as_str())
            })
            .collect();

        if owners.is_empty() {
            return None;
        }

        if owners.len() > 1 {
            self.diags.push(
                Diag::error(span.clone(), format!("ambiguous enum variant `{name}`")).with_help(
                    format!(
                        "qualify the variant: {}",
                        owners
                            .iter()
                            .map(|owner| format!("`{owner}.{name}`"))
                            .collect::<Vec<_>>()
                            .join(" or ")
                    ),
                ),
            );
            return Some(Value::Error);
        }

        let enum_name = owners[0];
        let Some(enum_def) = self.enum_defs.get(enum_name) else {
            return Some(Value::Error);
        };
        Some(Self::enum_variant_scalar_value(enum_def, name))
    }

    fn fold_static_sx(sx: Sx) -> Sx {
        Self::try_eval_static_scalar(&sx).map_or(sx, Sx::Lit)
    }

    fn is_literal_like_sx(sx: &Sx) -> bool {
        matches!(sx, Sx::Lit(_) | Sx::PxLit(_))
    }

    fn is_exact_literal_value(sx: &Sx, expected: f32) -> bool {
        matches!(sx, Sx::Lit(value) if value.to_bits() == expected.to_bits())
    }

    fn sx_sort_rank(sx: &Sx) -> u8 {
        match sx {
            Sx::Typed(_) => 4,
            Sx::Lit(_) | Sx::PxLit(_) => 0,
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
            | Sx::TexChannel { .. }
            | Sx::PathDist { .. }
            | Sx::PathAlong { .. }
            | Sx::PathTangentComponent { .. }
            | Sx::Var(_)
            | Sx::DynamicArrayIndex { .. } => 1,
            Sx::Neg(_)
            | Sx::Sin(_)
            | Sx::Cos(_)
            | Sx::Tan(_)
            | Sx::Asin(_)
            | Sx::Acos(_)
            | Sx::Atan(_)
            | Sx::Sqrt(_)
            | Sx::InverseSqrt(_)
            | Sx::Fract(_)
            | Sx::Abs(_)
            | Sx::Sign(_)
            | Sx::Floor(_)
            | Sx::Ceil(_)
            | Sx::Round(_)
            | Sx::Trunc(_)
            | Sx::Exp(_)
            | Sx::Exp2(_)
            | Sx::Log(_)
            | Sx::Log2(_)
            | Sx::Ddx(_)
            | Sx::Ddy(_)
            | Sx::Fwidth(_)
            | Sx::SrgbToLinear(_)
            | Sx::LinearToSrgb(_) => 2,
            Sx::Add(_, _)
            | Sx::Sub(_, _)
            | Sx::Mul(_, _)
            | Sx::Div(_, _)
            | Sx::Lt(_, _)
            | Sx::Le(_, _)
            | Sx::Gt(_, _)
            | Sx::Ge(_, _)
            | Sx::Eq(_, _)
            | Sx::Ne(_, _)
            | Sx::Atan2(_, _)
            | Sx::Pow(_, _)
            | Sx::Min(_, _)
            | Sx::Max(_, _)
            | Sx::Step(_, _) => 3,
            Sx::Dot { .. }
            | Sx::NormalizeComponent { .. }
            | Sx::MinComponent { .. }
            | Sx::MaxComponent { .. }
            | Sx::ClampVecComponent { .. }
            | Sx::Length(_)
            | Sx::Clamp(_, _, _)
            | Sx::Mix(_, _, _)
            | Sx::Select(_, _, _)
            | Sx::SmoothStep(_, _, _)
            | Sx::UserCall { .. }
            | Sx::EffectInputChannel { .. }
            | Sx::PathPointAtComponent { .. }
            | Sx::PathTangentAtComponent { .. }
            | Sx::CellQuery { .. }
            | Sx::CellContour { .. }
            | Sx::GradientChannel { .. }
            | Sx::Let { .. } => 4,
        }
    }

    fn sx_sort_key(sx: &Sx) -> (u8, u64) {
        struct StableHasher(u64);

        impl StableHasher {
            fn new() -> Self {
                Self(0xcbf29ce484222325)
            }
        }

        impl Hasher for StableHasher {
            fn finish(&self) -> u64 {
                self.0
            }

            fn write(&mut self, bytes: &[u8]) {
                for byte in bytes {
                    self.0 ^= u64::from(*byte);
                    self.0 = self.0.wrapping_mul(0x100000001b3);
                }
            }
        }

        let mut hasher = StableHasher::new();
        sx.hash(&mut hasher);
        (Self::sx_sort_rank(sx), hasher.finish())
    }

    fn normalize_commutative_pair(a: Sx, b: Sx) -> (Sx, Sx) {
        if Self::sx_sort_key(&a) <= Self::sx_sort_key(&b) {
            (a, b)
        } else {
            (b, a)
        }
    }

    fn fold_neg_if_literal(sx: Sx) -> Sx {
        let expr = Sx::Neg(Box::new(sx.clone()));
        if Self::is_literal_like_sx(&sx) {
            Self::fold_static_sx(expr)
        } else {
            expr
        }
    }

    fn positive_constant(sx: &Sx) -> bool {
        matches!(sx, Sx::Lit(v) | Sx::PxLit(v) if *v > 0.0)
    }

    fn symbolic_factor_sign(sx: &Sx) -> Option<bool> {
        if Self::is_literal_like_sx(sx) {
            return None;
        }

        match sx {
            Sx::Abs(_) | Sx::Length(_) | Sx::Sqrt(_) => Some(false),
            Sx::Max(a, b) => {
                if Self::positive_constant(a) || Self::positive_constant(b) {
                    Some(true)
                } else if Self::symbolic_factor_sign(a).is_some()
                    || Self::symbolic_factor_sign(b).is_some()
                {
                    Some(false)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn split_symbolic_factor(sx: &Sx) -> Option<(f32, Sx, bool)> {
        match sx {
            Sx::Mul(a, b) => {
                if let Sx::Lit(coef) = **a {
                    let sign = Self::symbolic_factor_sign(b)?;
                    return Some((coef, (*b.clone()), sign));
                }
                if let Sx::Lit(coef) = **b {
                    let sign = Self::symbolic_factor_sign(a)?;
                    return Some((coef, (*a.clone()), sign));
                }
                None
            }
            _ => {
                let sign = Self::symbolic_factor_sign(sx)?;
                Some((1.0, sx.clone(), sign))
            }
        }
    }

    pub(super) fn factor_symbolic_pair(op: SymbolicFactorOp, a: Sx, b: Sx) -> Option<Sx> {
        let (ca, fa, a_positive) = Self::split_symbolic_factor(&a)?;
        let (cb, fb, b_positive) = Self::split_symbolic_factor(&b)?;
        if fa != fb {
            return None;
        }

        match op {
            SymbolicFactorOp::Max => {
                if !(a_positive || b_positive || Self::symbolic_factor_sign(&fa).is_some()) {
                    return None;
                }
                let coef =
                    Self::fold_static_sx(Sx::Max(Box::new(Sx::Lit(ca)), Box::new(Sx::Lit(cb))));
                Some(Self::fold_bin_if_literals(BinOp::Mul, coef, fa))
            }
            SymbolicFactorOp::Min => {
                if !(a_positive || b_positive || Self::symbolic_factor_sign(&fa).is_some()) {
                    return None;
                }
                let coef =
                    Self::fold_static_sx(Sx::Min(Box::new(Sx::Lit(ca)), Box::new(Sx::Lit(cb))));
                Some(Self::fold_bin_if_literals(BinOp::Mul, coef, fa))
            }
            SymbolicFactorOp::Sub => {
                if !(a_positive || b_positive || Self::symbolic_factor_sign(&fa).is_some()) {
                    return None;
                }
                let coef = Self::fold_bin_if_literals(BinOp::Sub, Sx::Lit(ca), Sx::Lit(cb));
                Some(Self::fold_bin_if_literals(BinOp::Mul, coef, fa))
            }
            SymbolicFactorOp::Div => {
                if !(a_positive && b_positive) {
                    return None;
                }
                Some(Self::fold_bin_if_literals(
                    BinOp::Div,
                    Sx::Lit(ca),
                    Sx::Lit(cb),
                ))
            }
        }
    }

    pub(super) fn fold_bin_if_literals(op: BinOp, a: Sx, b: Sx) -> Sx {
        match op {
            BinOp::Sub if Self::is_exact_literal_value(&b, 0.0) => return a,
            BinOp::Mul if Self::is_exact_literal_value(&a, 1.0) => return b,
            BinOp::Mul if Self::is_exact_literal_value(&b, 1.0) => return a,
            BinOp::Div if Self::is_exact_literal_value(&b, 1.0) => return a,
            _ => {}
        }

        let (a, b) = match op {
            BinOp::Add | BinOp::Mul => Self::normalize_commutative_pair(a, b),
            _ => (a, b),
        };

        let expr = match op {
            BinOp::Add => Sx::Add(Box::new(a.clone()), Box::new(b.clone())),
            BinOp::Sub => Sx::Sub(Box::new(a.clone()), Box::new(b.clone())),
            BinOp::Mul => Sx::Mul(Box::new(a.clone()), Box::new(b.clone())),
            BinOp::Div => Sx::Div(Box::new(a.clone()), Box::new(b.clone())),
            _ => unreachable!("non-arithmetic op used in fold_bin_if_literals"),
        };
        if Self::is_literal_like_sx(&a) && Self::is_literal_like_sx(&b) {
            Self::fold_static_sx(expr)
        } else {
            expr
        }
    }

    pub(super) fn color_to_field(color: [f32; 4]) -> [Sx; 4] {
        [
            Sx::Lit(color[0]),
            Sx::Lit(color[1]),
            Sx::Lit(color[2]),
            Sx::Lit(color[3]),
        ]
    }

    fn color_bin(op: BinOp, a: [Sx; 4], b: [Sx; 4]) -> Option<[Sx; 4]> {
        let mk = |la: Sx, lb: Sx| match op {
            BinOp::Add => Some(Sx::Add(Box::new(la), Box::new(lb))),
            BinOp::Sub => Some(Sx::Sub(Box::new(la), Box::new(lb))),
            BinOp::Mul => Some(Sx::Mul(Box::new(la), Box::new(lb))),
            BinOp::Div => Some(Sx::Div(Box::new(la), Box::new(lb))),
            _ => None,
        };

        Some([
            mk(a[0].clone(), b[0].clone())?,
            mk(a[1].clone(), b[1].clone())?,
            mk(a[2].clone(), b[2].clone())?,
            mk(a[3].clone(), b[3].clone())?,
        ])
    }

    fn as_color_field_value(v: &Value) -> Option<([Sx; 4], ColorSpace)> {
        match v {
            Value::Error => None,
            Value::Color { rgba, space } => Some((Self::color_to_field(*rgba), *space)),
            Value::ColorField { rgba, space } => Some((rgba.clone(), *space)),
            _ => None,
        }
    }

    pub(super) fn as_numeric_scalar(v: &Value) -> Option<(Sx, ScalarKind)> {
        match v {
            Value::Error => None,
            Value::Scalar(s) => Some((s.clone(), ScalarKind::Plain)),
            Value::Distance(s) => Some((s.clone(), ScalarKind::Distance)),
            Value::Coverage(s) => Some((s.clone(), ScalarKind::Coverage)),
            Value::Mask(s) => Some((s.clone(), ScalarKind::Mask)),
            _ => None,
        }
    }

    /// Wrap a scalar operand once behind a cheap `Sx::Let`/`Sx::Var` pair
    /// before it's broadcast (`.clone()`d) across multiple vector-component
    /// results, so the (potentially large, e.g. an accumulator chain)
    /// expression is stored once and shared via `Rc`, rather than physically
    /// duplicated once per vector component. Leaves that are already O(1) to
    /// clone are returned unchanged to avoid pointless wrapper overhead.
    fn hoist_for_broadcast(&mut self, sx: Sx) -> Sx {
        let mut has_child = false;
        sx.for_each_child(|_| has_child = true);
        if !has_child {
            return sx;
        }
        self.let_counter += 1;
        let name = format!("__bcast_{}", self.let_counter);
        Sx::Let {
            name: name.clone(),
            value: std::rc::Rc::new(sx),
            body: Box::new(Sx::Var(name)),
        }
    }

    /// Hoist `sx` once (see [`Self::hoist_for_broadcast`]) and return two
    /// cheap-to-clone copies for a `vec2` broadcast.
    fn broadcast2(&mut self, sx: Sx) -> [Sx; 2] {
        let hoisted = self.hoist_for_broadcast(sx);
        [hoisted.clone(), hoisted]
    }

    /// Hoist `sx` once and return three cheap-to-clone copies for a `vec3`
    /// broadcast.
    fn broadcast3(&mut self, sx: Sx) -> [Sx; 3] {
        let hoisted = self.hoist_for_broadcast(sx);
        [hoisted.clone(), hoisted.clone(), hoisted]
    }

    /// Hoist `sx` once and return four cheap-to-clone copies for a `vec4`
    /// broadcast.
    fn broadcast4(&mut self, sx: Sx) -> [Sx; 4] {
        let hoisted = self.hoist_for_broadcast(sx);
        [hoisted.clone(), hoisted.clone(), hoisted.clone(), hoisted]
    }

    pub(super) fn scalar_kind_name(kind: ScalarKind) -> &'static str {
        match kind {
            ScalarKind::Plain => "scalar",
            ScalarKind::Distance => "distance",
            ScalarKind::Coverage => "coverage",
            ScalarKind::Mask => "mask",
        }
    }

    pub(super) fn promoted_scalar_kind(
        op: BinOp,
        lhs: ScalarKind,
        rhs: ScalarKind,
    ) -> Option<ScalarKind> {
        if lhs == rhs {
            return Some(lhs);
        }
        if lhs == ScalarKind::Plain {
            return Some(rhs);
        }
        if rhs == ScalarKind::Plain {
            return Some(lhs);
        }

        if matches!(op, BinOp::Mul | BinOp::Div) {
            match (lhs, rhs) {
                (ScalarKind::Distance, ScalarKind::Coverage)
                | (ScalarKind::Coverage, ScalarKind::Distance)
                | (ScalarKind::Distance, ScalarKind::Mask)
                | (ScalarKind::Mask, ScalarKind::Distance) => return Some(ScalarKind::Distance),
                (ScalarKind::Coverage, ScalarKind::Mask)
                | (ScalarKind::Mask, ScalarKind::Coverage) => return Some(ScalarKind::Coverage),
                _ => {}
            }
        }

        None
    }

    pub(super) fn scalar_value_from_kind(kind: ScalarKind, sx: Sx) -> Value {
        match kind {
            ScalarKind::Plain => Value::Scalar(sx),
            ScalarKind::Distance => Value::Distance(sx),
            ScalarKind::Coverage => Value::Coverage(sx),
            ScalarKind::Mask => Value::Mask(sx),
        }
    }

    fn sx_mod_floor(x: Sx, m: Sx) -> Sx {
        let q = Sx::Floor(Box::new(Sx::Div(Box::new(x.clone()), Box::new(m.clone()))));
        let q_times_m = Self::fold_bin_if_literals(BinOp::Mul, q, m);
        Self::fold_bin_if_literals(BinOp::Sub, x, q_times_m)
    }

    pub(super) fn to_working_color_space(rgba: [Sx; 4], space: ColorSpace) -> [Sx; 4] {
        if space == ColorSpace::Linear {
            return rgba;
        }

        [
            Sx::SrgbToLinear(Box::new(rgba[0].clone())),
            Sx::SrgbToLinear(Box::new(rgba[1].clone())),
            Sx::SrgbToLinear(Box::new(rgba[2].clone())),
            rgba[3].clone(),
        ]
    }

    fn select_component(
        &mut self,
        base_name: &str,
        field: &str,
        base: Value,
        span: &Span,
    ) -> Option<Value> {
        let swizzle_index = |ch: char, size: usize| -> Option<usize> {
            match ch {
                'x' | 'r' if size >= 1 => Some(0),
                'y' | 'g' if size >= 2 => Some(1),
                'z' | 'b' if size >= 3 => Some(2),
                'w' | 'a' if size >= 4 => Some(3),
                _ => None,
            }
        };

        let parse_swizzle = |field: &str, size: usize| -> Option<Vec<usize>> {
            if field.is_empty() || field.len() > 4 {
                return None;
            }
            let mut out = Vec::with_capacity(field.len());
            for ch in field.chars() {
                let idx = swizzle_index(ch, size)?;
                out.push(idx);
            }
            Some(out)
        };

        match base {
            Value::Error => Some(Value::Error),
            Value::Struct { ty_name, fields } => {
                if let Some(value) = fields.get(field) {
                    Some(value.clone())
                } else {
                    let mut available = fields.keys().cloned().collect::<Vec<_>>();
                    available.sort();
                    self.diags.push(
                        Diag::error(
                            span.clone(),
                            format!("unknown field `{field}` on `{base_name}`"),
                        )
                        .with_help(format!(
                            "`{ty_name}` fields: {}",
                            if available.is_empty() {
                                "<none>".to_string()
                            } else {
                                available.join(", ")
                            }
                        )),
                    );
                    None
                }
            }
            Value::PathFuture(path) => match field {
                "length" => Some(Value::Scalar(path.total_length_sx())),
                "dist" => {
                    self.record_path_channel_demand(path.profile_id(), "dist");
                    Some(Value::Scalar(Sx::PathDist {
                        path_id: path.profile_id(),
                    }))
                }
                "along" => {
                    self.record_path_channel_demand(path.profile_id(), "along");
                    Some(Value::Scalar(Sx::PathAlong {
                        path_id: path.profile_id(),
                    }))
                }
                "tangent" => {
                    self.record_path_channel_demand(path.profile_id(), "tangent");
                    Some(Value::Vec2((
                        Sx::PathTangentComponent {
                            path_id: path.profile_id(),
                            component: 0,
                        },
                        Sx::PathTangentComponent {
                            path_id: path.profile_id(),
                            component: 1,
                        },
                    )))
                }
                _ => {
                    self.diags.push(
                        Diag::error(
                            span.clone(),
                            format!("unknown path component `{field}` on `{base_name}`"),
                        )
                        .with_help(
                            "valid planned path components: .length, .dist, .along, .tangent",
                        ),
                    );
                    None
                }
            },
            Value::ScatterInstance {
                pos,
                id,
                index01,
                age_norm,
            } => match field {
                "pos" => Some(Value::Vec2(pos)),
                "id" => Some(Value::Scalar(id)),
                "index01" => Some(Value::Scalar(index01)),
                "age_norm" => Some(Value::Scalar(age_norm)),
                "rand" => {
                    let Some(unit) = self.scatter_rand_unit(span) else {
                        self.diags.push(
                            Diag::error(
                                span.clone(),
                                "`instance.rand` is only available inside scatter evaluation",
                            )
                            .with_help("use this alias inside a `scatter { ... }` body"),
                        );
                        return None;
                    };
                    Some(Value::Scalar(unit))
                }
                "rand2" => {
                    let Some(unit) = self.scatter_rand_unit(span) else {
                        self.diags.push(
                            Diag::error(
                                span.clone(),
                                "`instance.rand2` is only available inside scatter evaluation",
                            )
                            .with_help("use this alias inside a `scatter { ... }` body"),
                        );
                        return None;
                    };
                    let v = Sx::Fract(Box::new(Sx::Mul(
                        Box::new(Sx::Sin(Box::new(Sx::Add(
                            Box::new(unit),
                            Box::new(Sx::Lit(17.173)),
                        )))),
                        Box::new(Sx::Lit(43_758.547)),
                    )));
                    Some(Value::Scalar(v))
                }
                "jitter" => {
                    let Some(unit) = self.scatter_rand_unit(span) else {
                        self.diags.push(
                            Diag::error(
                                span.clone(),
                                "`instance.jitter` is only available inside scatter evaluation",
                            )
                            .with_help("use this alias inside a `scatter { ... }` body"),
                        );
                        return None;
                    };
                    let other = Sx::Fract(Box::new(Sx::Mul(
                        Box::new(Sx::Sin(Box::new(Sx::Add(
                            Box::new(unit.clone()),
                            Box::new(Sx::Lit(31.331)),
                        )))),
                        Box::new(Sx::Lit(91_821.734)),
                    )));
                    let jx = Sx::Sub(
                        Box::new(Sx::Mul(Box::new(unit), Box::new(Sx::Lit(2.0)))),
                        Box::new(Sx::Lit(1.0)),
                    );
                    let jy = Sx::Sub(
                        Box::new(Sx::Mul(Box::new(other), Box::new(Sx::Lit(2.0)))),
                        Box::new(Sx::Lit(1.0)),
                    );
                    Some(Value::Vec2((jx, jy)))
                }
                _ => {
                    self.diags.push(
                        Diag::error(
                            span.clone(),
                            format!(
                                "unknown scatter instance component `{field}` on `{base_name}`"
                            ),
                        )
                        .with_help(
                            "valid scatter instance components: .pos, .id, .index01, .age_norm, .rand, .rand2, .jitter",
                        ),
                    );
                    None
                }
            },
            Value::CellContour { scope_id, inset } => {
                if field == "edge_distance" {
                    return Some(Value::Scalar(Sx::CellQuery {
                        scope_id,
                        inset: Box::new(inset),
                        angle: None,
                    }));
                }

                let channel = match field {
                    "distance" => 0,
                    "progress" => 1,
                    "length" => 2,
                    _ => {
                        self.diags.push(Diag::error(
                            span.clone(),
                            "contour members: distance, edge_distance, progress, length, point(at: ...)",
                        ));
                        return None;
                    }
                };
                Some(Value::Scalar(Sx::CellContour {
                    scope_id,
                    inset: Box::new(inset),
                    at: None,
                    channel,
                }))
            }
            Value::RepeatCell(cell) => match field {
                "local" | "angle" | "edge_distance" => {
                    self.cell_geometry_member(&cell, field, span)
                }
                "id" => Some(Value::Vec2(cell.id)),
                "center" => Some(Value::Vec2(cell.center)),
                "uv" => Some(Value::Vec2(cell.uv)),
                "rand" => Some(Value::Scalar(cell.rand)),
                _ => {
                    self.diags.push(
                        Diag::error(
                            span.clone(),
                            format!("unknown repeat cell component `{field}` on `{base_name}`"),
                        )
                        .with_help("cell components: .id, .center, .uv, .rand; cells bindings also expose .local, .angle, .edge_distance, .inset_distance(by: ...), and .boundary_point(angle: ...)"),
                    );
                    None
                }
            },
            Value::Slot {
                index,
                count,
                start,
                end,
            } => {
                let slot_size = Sx::Div(
                    Box::new(Sx::Sub(Box::new(end), Box::new(start.clone()))),
                    Box::new(count.clone()),
                );
                let slot_left = Sx::Add(
                    Box::new(start),
                    Box::new(Sx::Mul(
                        Box::new(index.clone()),
                        Box::new(slot_size.clone()),
                    )),
                );
                let slot_right = Sx::Add(Box::new(slot_left.clone()), Box::new(slot_size.clone()));
                let slot_center = Sx::Add(
                    Box::new(slot_left.clone()),
                    Box::new(Sx::Mul(Box::new(slot_size.clone()), Box::new(Sx::Lit(0.5)))),
                );

                match field {
                    "index" => Some(Value::Scalar(index)),
                    "count" => Some(Value::Scalar(count)),
                    "start" | "left" => Some(Value::Scalar(slot_left)),
                    "end" | "right" => Some(Value::Scalar(slot_right)),
                    "center" => Some(Value::Scalar(slot_center)),
                    "size" => Some(Value::Scalar(slot_size)),
                    _ => {
                        self.diags.push(
                            Diag::error(
                                span.clone(),
                                format!("unknown slot component `{field}` on `{base_name}`"),
                            )
                            .with_help(
                                "valid slot components: .center, .left, .right, .size, .index, .count",
                            ),
                        );
                        None
                    }
                }
            }
            Value::Color { rgba, .. } => {
                if let Some(ixs) = parse_swizzle(field, 4) {
                    let comps = [
                        Sx::Lit(rgba[0]),
                        Sx::Lit(rgba[1]),
                        Sx::Lit(rgba[2]),
                        Sx::Lit(rgba[3]),
                    ];
                    match ixs.as_slice() {
                        [i] => {
                            if *i == 3 {
                                Some(Value::Coverage(comps[*i].clone()))
                            } else {
                                Some(Value::Scalar(comps[*i].clone()))
                            }
                        }
                        [a, b] => Some(Value::Vec2((comps[*a].clone(), comps[*b].clone()))),
                        [a, b, c] => Some(Value::Vec3((
                            comps[*a].clone(),
                            comps[*b].clone(),
                            comps[*c].clone(),
                        ))),
                        [a, b, c, d] => Some(Value::Vec4((
                            comps[*a].clone(),
                            comps[*b].clone(),
                            comps[*c].clone(),
                            comps[*d].clone(),
                        ))),
                        _ => unreachable!("swizzle length already constrained"),
                    }
                } else {
                    self.diags.push(
                        Diag::error(
                            span.clone(),
                            format!("unknown color component `{field}` on `{base_name}`"),
                        )
                        .with_help("valid color components: .r, .g, .b, .a and swizzles like .rgb/.bgr/.rgba"),
                    );
                    None
                }
            }
            Value::ColorField { rgba, .. } => {
                if let Some(ixs) = parse_swizzle(field, 4) {
                    let comps = [
                        rgba[0].clone(),
                        rgba[1].clone(),
                        rgba[2].clone(),
                        rgba[3].clone(),
                    ];
                    match ixs.as_slice() {
                        [i] => {
                            if *i == 3 {
                                Some(Value::Coverage(comps[*i].clone()))
                            } else {
                                Some(Value::Scalar(comps[*i].clone()))
                            }
                        }
                        [a, b] => Some(Value::Vec2((comps[*a].clone(), comps[*b].clone()))),
                        [a, b, c] => Some(Value::Vec3((
                            comps[*a].clone(),
                            comps[*b].clone(),
                            comps[*c].clone(),
                        ))),
                        [a, b, c, d] => Some(Value::Vec4((
                            comps[*a].clone(),
                            comps[*b].clone(),
                            comps[*c].clone(),
                            comps[*d].clone(),
                        ))),
                        _ => unreachable!("swizzle length already constrained"),
                    }
                } else {
                    self.diags.push(
                        Diag::error(
                            span.clone(),
                            format!("unknown color component `{field}` on `{base_name}`"),
                        )
                        .with_help("valid color components: .r, .g, .b, .a and swizzles like .rgb/.bgr/.rgba"),
                    );
                    None
                }
            }
            Value::Vec2((x, y)) => {
                if let Some(ixs) = parse_swizzle(field, 2) {
                    let comps = [x, y];
                    match ixs.as_slice() {
                        [i] => Some(Value::Scalar(comps[*i].clone())),
                        [a, b] => Some(Value::Vec2((comps[*a].clone(), comps[*b].clone()))),
                        [a, b, c] => Some(Value::Vec3((
                            comps[*a].clone(),
                            comps[*b].clone(),
                            comps[*c].clone(),
                        ))),
                        [a, b, c, d] => Some(Value::Vec4((
                            comps[*a].clone(),
                            comps[*b].clone(),
                            comps[*c].clone(),
                            comps[*d].clone(),
                        ))),
                        _ => unreachable!("swizzle length already constrained"),
                    }
                } else {
                    self.diags.push(
                        Diag::error(
                            span.clone(),
                            format!("unknown vec2 component `{field}` on `{base_name}`"),
                        )
                        .with_help("valid vec2 components: .x, .y (aliases: .r, .g) and swizzles like .xy/.yx/.rr"),
                    );
                    None
                }
            }
            Value::Vec3((x, y, z)) => {
                if let Some(ixs) = parse_swizzle(field, 3) {
                    let comps = [x, y, z];
                    match ixs.as_slice() {
                        [i] => Some(Value::Scalar(comps[*i].clone())),
                        [a, b] => Some(Value::Vec2((comps[*a].clone(), comps[*b].clone()))),
                        [a, b, c] => Some(Value::Vec3((
                            comps[*a].clone(),
                            comps[*b].clone(),
                            comps[*c].clone(),
                        ))),
                        [a, b, c, d] => Some(Value::Vec4((
                            comps[*a].clone(),
                            comps[*b].clone(),
                            comps[*c].clone(),
                            comps[*d].clone(),
                        ))),
                        _ => unreachable!("swizzle length already constrained"),
                    }
                } else {
                    self.diags.push(
                        Diag::error(
                            span.clone(),
                            format!("unknown vec3 component `{field}` on `{base_name}`"),
                        )
                        .with_help("valid vec3 components: .x, .y, .z (aliases: .r, .g, .b) and swizzles like .xyz/.yzx/.bgr"),
                    );
                    None
                }
            }
            Value::Vec4((x, y, z, w)) => {
                if let Some(ixs) = parse_swizzle(field, 4) {
                    let comps = [x, y, z, w];
                    match ixs.as_slice() {
                        [i] => Some(Value::Scalar(comps[*i].clone())),
                        [a, b] => Some(Value::Vec2((comps[*a].clone(), comps[*b].clone()))),
                        [a, b, c] => Some(Value::Vec3((
                            comps[*a].clone(),
                            comps[*b].clone(),
                            comps[*c].clone(),
                        ))),
                        [a, b, c, d] => Some(Value::Vec4((
                            comps[*a].clone(),
                            comps[*b].clone(),
                            comps[*c].clone(),
                            comps[*d].clone(),
                        ))),
                        _ => unreachable!("swizzle length already constrained"),
                    }
                } else {
                    self.diags.push(
                        Diag::error(
                            span.clone(),
                            format!("unknown vec4 component `{field}` on `{base_name}`"),
                        )
                        .with_help(
                            "valid vec4 components: .x, .y, .z, .w (aliases: .r, .g, .b, .a) and swizzles like .xyz/.rgba/.bgr",
                        ),
                    );
                    None
                }
            }
            v @ Value::TypedTextureSample { .. } => {
                // Allow raw r/g/b/a channel access by index OR by semantic name.
                let Value::TypedTextureSample {
                    layer_id: _,
                    tex_name,
                    sample_at,
                } = v
                else {
                    unreachable!()
                };
                if sample_at.is_none()
                    && !self.implicit_texture_uv_allowed(
                        span.clone(),
                        &tex_name,
                        "sample explicitly first",
                        format!(
                            "use `{tex_name}.at(uv).{field}` or `image({tex_name}, at: uv).{field}`; staged rollout: `#pragma check.warn_implicit_texture_uv = true` for warnings or `#pragma check.allow_implicit_texture_uv = true` to suppress the diagnostic"
                        ),
                    )
                {
                    return None;
                }
                // First try raw channel letter (r/g/b/a)
                let raw_idx: Option<u8> = match field {
                    "r" => Some(0),
                    "g" => Some(1),
                    "b" => Some(2),
                    "a" => Some(3),
                    _ => None,
                };
                if let Some(ch) = raw_idx {
                    return Some(Value::Scalar(Sx::TexChannel {
                        tex_name,
                        channel: ch,
                        sample_at: sample_at.map(Box::new),
                        decode_mul: 1.0,
                        decode_add: 0.0,
                        decode_expr: None,
                    }));
                }
                // Try semantic name lookup via texture_type_defs
                let type_name = self
                    .hir
                    .texture_metadata
                    .get(&tex_name)
                    .and_then(|m| m.texture_type_name.clone());
                if let Some(type_name) = type_name
                    && let Some(def) = self.hir.texture_type_defs.get(&type_name)
                {
                    if let Some(ch_def) = def.channel_def_for(field) {
                        return Some(Value::Scalar(Sx::TexChannel {
                            tex_name,
                            channel: ch_def.channel_idx,
                            sample_at: sample_at.map(Box::new),
                            decode_mul: ch_def.decode_mul,
                            decode_add: ch_def.decode_add,
                            decode_expr: ch_def.decode_expr.clone().map(Box::new),
                        }));
                    }
                    // Build help string from known names
                    let known: Vec<String> = def
                        .channels
                        .iter()
                        .map(|c| format!(".{}", c.semantic_name))
                        .collect();
                    self.diags.push(
                        Diag::error(
                            span.clone(),
                            format!(
                                "unknown channel `{field}` on typed texture `{tex_name}` (type `{type_name}`)"
                            ),
                        )
                        .with_help(format!(
                            "valid channels: {}; or raw: .r .g .b .a",
                            known.join(", ")
                        )),
                    );
                    return None;
                }
                // No type — only raw channels allowed
                self.diags.push(
                    Diag::error(
                        span.clone(),
                        format!("unknown channel `{field}` on texture `{tex_name}`"),
                    )
                    .with_help("valid channels: .r .g .b .a"),
                );
                None
            }
            v => {
                self.diags.push(
                    Diag::error(
                        span.clone(),
                        format!("`{base_name}` is a {}, so `{base_name}.{field}` is invalid", v.kind()),
                    )
                    .with_help("component selectors are supported on colors (.r/.g/.b/.a), vec2 (.x/.y), vec3 (.x/.y/.z), and vec4 (.x/.y/.z/.w)"),
                );
                None
            }
        }
    }

    fn resolve_var_name(&mut self, name: &str, span: &Span) -> Option<Value> {
        if name == "true" {
            return Some(Value::Scalar(
                crate::typed_scalar::Scalar::number(1.0, crate::typed_scalar::Kind::Bool)
                    .expect("boolean literal"),
            ));
        }
        if name == "false" {
            return Some(Value::Scalar(
                crate::typed_scalar::Scalar::number(0.0, crate::typed_scalar::Kind::Bool)
                    .expect("boolean literal"),
            ));
        }

        if let Some(v) = self.lookup(name) {
            return Some(v);
        }

        if let Some(v) = self.resolve_anchor_value_name(name, span) {
            return Some(v);
        }

        if let Some(v) = self.resolve_enum_value_name(name, span) {
            return Some(v);
        }

        if name.contains('.') {
            let mut iter = name.split('.');
            let head = iter.next()?;

            let mut current_name = head.to_string();
            let mut value = self
                .lookup(head)
                .or_else(|| self.resolve_anchor_value_name(head, span))?;

            for field in iter {
                let next_name = format!("{current_name}.{field}");
                let selected = self.select_component(&current_name, field, value, span)?;
                current_name = next_name;
                value = selected;
            }

            return Some(value);
        }

        // If the name is a known fn declaration, produce a fn reference.
        // This allows `fn_name` to appear as an argument where a callable is expected (§16.5).
        if let Some(defs) = self.fn_defs.get(name) {
            let can_access_internal = self.can_access_internal_functions();
            if defs
                .iter()
                .any(|def| !def.is_internal || can_access_internal)
            {
                return Some(Value::FnRef(name.to_string()));
            }

            self.diags.push(
                Diag::error(
                    span.clone(),
                    format!("`{name}` is internal and cannot be referenced directly"),
                )
                .with_help("use a public stdlib wrapper function instead"),
            );
            return Some(Value::Error);
        }

        // User-defined effect declarations can also be referenced as callable values (§16.1 Phase 6).
        // An effect ref carries the effect name and is dispatched through the effect path at call sites.
        if self.effect_defs.contains_key(name) {
            return Some(Value::FnRef(name.to_string()));
        }

        None
    }

    pub(super) fn eval(&mut self, e: &SExpr) -> Option<Value> {
        with_checker_stack(|| self.eval_on_stack(e))
    }

    // Keep this frame behind the stack-growth checkpoint in optimized builds too.
    #[inline(never)]
    fn eval_on_stack(&mut self, e: &SExpr) -> Option<Value> {
        let _span = trace_span!("check.eval", kind = ?e.node).entered();
        if self.timeout_exceeded(e) {
            return Some(Value::Error);
        }
        if self.expr_watchdog_every > 0 {
            self.expr_watchdog_counter = self.expr_watchdog_counter.saturating_add(1);
            if self
                .expr_watchdog_counter
                .is_multiple_of(self.expr_watchdog_every)
            {
                info!(
                    evals = self.expr_watchdog_counter,
                    span_start = e.span.start,
                    span_end = e.span.end,
                    expr_kind = ?e.node,
                    scope_depth = self.scopes.len(),
                    diags = self.diags.len(),
                    "checker expr watchdog heartbeat"
                );
            }
        }
        let t0 = Instant::now();
        let expr_kind_key = match &e.node {
            Expr::Num(_, _) => "eval_expr_num",
            Expr::Color(_) => "eval_expr_color",
            Expr::Str(_) => "eval_expr_str",
            Expr::Var(_) => "eval_expr_var",
            Expr::Vec2(_, _) => "eval_expr_vec2",
            Expr::Vec3(_, _, _) => "eval_expr_vec3",
            Expr::Vec4(_, _, _, _) => "eval_expr_vec4",
            Expr::Range(_, _) => "eval_expr_range",
            Expr::Member(_, _) => "eval_expr_member",
            Expr::Array(_) => "eval_expr_array",
            Expr::ArrayComp { .. } => "eval_expr_array",
            Expr::Lambda { .. } => "eval_expr_lambda",
            Expr::Field(_) => "eval_expr_field",
            Expr::FieldAt { .. } => "eval_expr_field_at",
            Expr::Layer(_) => "eval_expr_layer",
            Expr::PathFuture { .. } => "eval_expr_path_future",
            Expr::Unary(_, _) => "eval_expr_unary",
            Expr::Binary(_, _, _) => "eval_expr_binary",
            Expr::Call { .. } => "eval_expr_call",
            Expr::Pipe { .. } => "eval_expr_pipe",
            Expr::Through { .. } => "eval_expr_through",
            Expr::Index { .. } => "eval_expr_index",
        };
        let result = self.eval_inner(e);
        let elapsed = t0.elapsed();
        self.record_check_profile_timing("eval_expr", elapsed);
        self.record_check_profile_timing(expr_kind_key, elapsed);
        self.record_expr_hotspot(e.span.start, e.span.end, expr_kind_key, elapsed);
        if let Some(ref v) = result {
            self.record_span_value(e.span.start, e.span.end, v);
        }
        result
    }

    // Keep expression-specific temporaries out of the recursive dispatcher frame.
    // These helpers stay out of line: WASM cannot use the native stack-growth
    // checkpoint, and the unoptimized evaluator otherwise exhausts its stack
    // when compiling ordinary nested expressions in an emitter.
    #[inline(never)]
    fn eval_call_expr(&mut self, e: &SExpr) -> Option<Value> {
        let Expr::Call {
            name,
            name_span,
            const_args,
            args,
        } = &e.node
        else {
            unreachable!("call evaluator requires a call expression");
        };
        if let Some(kind) = crate::typed_scalar::Kind::parse(name) {
            if args.len() != 1 || args[0].name.is_some() || !const_args.is_empty() {
                self.diags.push(Diag::error(
                    e.span.clone(),
                    "scalar conversion requires one positional argument",
                ));
                return None;
            }
            let argument = &args[0].value;
            let literal = match &argument.node {
                Expr::Num(value, Unit::None) => Some(*value),
                Expr::Unary(UnOp::Neg, inner) => match inner.node {
                    Expr::Num(value, Unit::None) => Some(-value),
                    _ => None,
                },
                _ => None,
            };
            if let Some(value) = literal {
                return match crate::typed_scalar::Scalar::number(value, kind) {
                    Ok(value) => Some(Value::Scalar(value)),
                    Err(message) => {
                        self.diags.push(Diag::error(argument.span.clone(), message));
                        None
                    }
                };
            }
            let value = self.as_scalar(argument)?;
            return Some(Value::Scalar(crate::typed_scalar::Scalar::cast(
                value, kind,
            )));
        }
        if name == "context" {
            if const_args.is_empty()
                && args.len() == 1
                && args[0].name.is_none()
                && let Expr::Var(role) = &args[0].value.node
            {
                return self.context_role(role, &e.span);
            }
            self.diags.push(Diag::error(
                e.span.clone(),
                "expected context(role), with a semantic role name",
            ));
            return None;
        }
        if name == "angle" {
            let mut theta_expr: Option<&SExpr> = None;
            for arg in args {
                match arg.name.as_deref() {
                    Some("theta") | Some("a") => theta_expr = Some(&arg.value),
                    Some(other) => {
                        self.diags.push(
                            Diag::error(
                                arg.value.span.clone(),
                                format!("unknown argument `{other}` to `angle`"),
                            )
                            .with_help("use `angle(theta)` or `angle(theta: ...)`"),
                        );
                        return Some(Value::Error);
                    }
                    None => {
                        if theta_expr.is_none() {
                            theta_expr = Some(&arg.value);
                        } else {
                            self.diags.push(
                                Diag::error(
                                    arg.value.span.clone(),
                                    "`angle` accepts exactly one argument",
                                )
                                .with_help("use `angle(theta)`"),
                            );
                            return Some(Value::Error);
                        }
                    }
                }
            }

            let Some(theta_expr) = theta_expr else {
                self.diags.push(
                    Diag::error(e.span.clone(), "`angle` is missing required argument")
                        .with_help("use `angle(theta)`"),
                );
                return Some(Value::Error);
            };

            let theta_val = self.eval(theta_expr)?;
            let Some((theta, _kind)) = Self::as_numeric_scalar(&theta_val) else {
                if !matches!(theta_val, Value::Error) {
                    self.diags.push(
                        Diag::error(
                            theta_expr.span.clone(),
                            format!(
                                "argument to `angle` expected scalar, found {}",
                                theta_val.kind()
                            ),
                        )
                        .with_label("type mismatch in angle constructor"),
                    );
                }
                return Some(Value::Error);
            };

            return Some(Value::Vec2((
                Sx::Cos(Box::new(theta.clone())),
                Sx::Sin(Box::new(theta)),
            )));
        }

        if matches!(name.as_str(), "point_at" | "tangent_at") {
            let mut path_arg: Option<&SExpr> = None;
            let mut s_arg: Option<&SExpr> = None;

            for arg in args {
                match arg.name.as_deref() {
                    Some("path") => path_arg = Some(&arg.value),
                    Some("s") => s_arg = Some(&arg.value),
                    Some(other) => {
                        self.diags.push(
                            Diag::error(
                                arg.value.span.clone(),
                                format!("unknown argument `{other}` to `{name}`"),
                            )
                            .with_help("expected named arguments: `path: ...`, `s: ...`"),
                        );
                        return Some(Value::Error);
                    }
                    None => {
                        if path_arg.is_none() {
                            path_arg = Some(&arg.value);
                        } else if s_arg.is_none() {
                            s_arg = Some(&arg.value);
                        } else {
                            self.diags.push(
                                Diag::error(
                                    arg.value.span.clone(),
                                    format!("`{name}` accepts at most two arguments"),
                                )
                                .with_help("use `{name}(path: <path>, s: <scalar>)`"),
                            );
                            return Some(Value::Error);
                        }
                    }
                }
            }

            let Some(path_expr) = path_arg else {
                self.diags.push(
                    Diag::error(
                        e.span.clone(),
                        format!("`{name}` is missing required argument `path`"),
                    )
                    .with_help("use `{name}(path: <path>, s: <scalar>)`"),
                );
                return Some(Value::Error);
            };
            let Some(s_expr) = s_arg else {
                self.diags.push(
                    Diag::error(
                        e.span.clone(),
                        format!("`{name}` is missing required argument `s`"),
                    )
                    .with_help("use `{name}(path: <path>, s: <scalar>)`"),
                );
                return Some(Value::Error);
            };

            let path_value = self.eval(path_expr)?;
            if !matches!(path_value, Value::PathFuture(_) | Value::Error) {
                self.diags.push(
                    Diag::error(
                        path_expr.span.clone(),
                        format!(
                            "argument `path` to `{name}` expected path, found {}",
                            path_value.kind()
                        ),
                    )
                    .with_label("type mismatch in path evaluator call"),
                );
                return Some(Value::Error);
            }

            let s_value = self.eval(s_expr)?;
            if !matches!(s_value, Value::Error)
                && !matches!(Self::as_numeric_scalar(&s_value), Some((_sx, _kind)))
            {
                self.diags.push(
                    Diag::error(
                        s_expr.span.clone(),
                        format!(
                            "argument `s` to `{name}` expected scalar, found {}",
                            s_value.kind()
                        ),
                    )
                    .with_label("type mismatch in path evaluator call"),
                );
                return Some(Value::Error);
            }

            let Some((s_sx, _kind)) = Self::as_numeric_scalar(&s_value) else {
                return Some(Value::Error);
            };

            return match path_value {
                Value::PathFuture(path) => {
                    if name == "point_at" {
                        self.record_path_eval_demand(path.profile_id(), "point_at");
                        Some(Value::Vec2((
                            Sx::PathPointAtComponent {
                                path_id: path.profile_id(),
                                s: Box::new(s_sx.clone()),
                                component: 0,
                            },
                            Sx::PathPointAtComponent {
                                path_id: path.profile_id(),
                                s: Box::new(s_sx),
                                component: 1,
                            },
                        )))
                    } else {
                        self.record_path_eval_demand(path.profile_id(), "tangent_at");
                        Some(Value::Vec2((
                            Sx::PathTangentAtComponent {
                                path_id: path.profile_id(),
                                s: Box::new(s_sx.clone()),
                                component: 0,
                            },
                            Sx::PathTangentAtComponent {
                                path_id: path.profile_id(),
                                s: Box::new(s_sx),
                                component: 1,
                            },
                        )))
                    }
                }
                Value::Error => Some(Value::Error),
                _ => Some(Value::Error),
            };
        }

        if matches!(
            name.as_str(),
            "vec2"
                | "vec3"
                | "vec4"
                | "uvec2"
                | "uvec3"
                | "uvec4"
                | "ivec2"
                | "ivec3"
                | "ivec4"
                | "bvec2"
                | "bvec3"
                | "bvec4"
        ) {
            let element =
                crate::typed_scalar::Kind::element(name).expect("vector constructor element");
            let vector_name = if name.starts_with(['u', 'i', 'b']) {
                &name[1..]
            } else {
                name.as_str()
            };
            let mut vals = Vec::with_capacity(args.len());
            for arg in args {
                let value = if matches!(arg.value.node, Expr::Num(..)) {
                    self.eval_scalar_expected(&arg.value, Some(element))?
                } else {
                    self.eval(&arg.value)?
                };
                vals.push(Self::map_value_lanes(value, |lane| {
                    crate::typed_scalar::Scalar::cast(lane, element)
                }));
            }

            if vals.iter().any(|v| matches!(v, Value::Error)) {
                return Some(Value::Error);
            }

            let scalar_of = |v: &Value| Checker::as_numeric_scalar(v).map(|(sx, _kind)| sx);

            let out = match vector_name {
                "vec2" => {
                    if vals.len() == 1 {
                        match &vals[0] {
                            Value::Vec2(v) => Some(Value::Vec2(v.clone())),
                            v => scalar_of(v).map(|s| Value::Vec2((s.clone(), s))),
                        }
                    } else if vals.len() == 2 {
                        if let (Some(x), Some(y)) = (scalar_of(&vals[0]), scalar_of(&vals[1])) {
                            Some(Value::Vec2((x, y)))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                "vec3" => {
                    if vals.len() == 1 {
                        match &vals[0] {
                            Value::Vec3(v) => Some(Value::Vec3(v.clone())),
                            v => scalar_of(v).map(|s| Value::Vec3((s.clone(), s.clone(), s))),
                        }
                    } else if vals.len() == 2 {
                        match (&vals[0], &vals[1]) {
                            (Value::Vec2((x, y)), v) => {
                                scalar_of(v).map(|z| Value::Vec3((x.clone(), y.clone(), z)))
                            }
                            (v, Value::Vec2((y, z))) => {
                                scalar_of(v).map(|x| Value::Vec3((x, y.clone(), z.clone())))
                            }
                            _ => None,
                        }
                    } else if vals.len() == 3 {
                        if let (Some(x), Some(y), Some(z)) = (
                            scalar_of(&vals[0]),
                            scalar_of(&vals[1]),
                            scalar_of(&vals[2]),
                        ) {
                            Some(Value::Vec3((x, y, z)))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                "vec4" => {
                    if vals.len() == 1 {
                        match &vals[0] {
                            Value::Vec4(v) => Some(Value::Vec4(v.clone())),
                            v => scalar_of(v)
                                .map(|s| Value::Vec4((s.clone(), s.clone(), s.clone(), s))),
                        }
                    } else if vals.len() == 2 {
                        match (&vals[0], &vals[1]) {
                            (Value::Vec3((x, y, z)), v) => scalar_of(v)
                                .map(|w| Value::Vec4((x.clone(), y.clone(), z.clone(), w))),
                            (v, Value::Vec3((y, z, w))) => scalar_of(v)
                                .map(|x| Value::Vec4((x, y.clone(), z.clone(), w.clone()))),
                            (Value::Vec2((x, y)), Value::Vec2((z, w))) => {
                                Some(Value::Vec4((x.clone(), y.clone(), z.clone(), w.clone())))
                            }
                            _ => None,
                        }
                    } else if vals.len() == 3 {
                        match (&vals[0], &vals[1], &vals[2]) {
                            (Value::Vec2((x, y)), b, c) => match (scalar_of(b), scalar_of(c)) {
                                (Some(z), Some(w)) => {
                                    Some(Value::Vec4((x.clone(), y.clone(), z, w)))
                                }
                                _ => None,
                            },
                            (a, Value::Vec2((y, z)), c) => match (scalar_of(a), scalar_of(c)) {
                                (Some(x), Some(w)) => {
                                    Some(Value::Vec4((x, y.clone(), z.clone(), w)))
                                }
                                _ => None,
                            },
                            (a, b, Value::Vec2((z, w))) => match (scalar_of(a), scalar_of(b)) {
                                (Some(x), Some(y)) => {
                                    Some(Value::Vec4((x, y, z.clone(), w.clone())))
                                }
                                _ => None,
                            },
                            _ => None,
                        }
                    } else if vals.len() == 4 {
                        if let (Some(x), Some(y), Some(z), Some(w)) = (
                            scalar_of(&vals[0]),
                            scalar_of(&vals[1]),
                            scalar_of(&vals[2]),
                            scalar_of(&vals[3]),
                        ) {
                            Some(Value::Vec4((x, y, z, w)))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            };

            if let Some(v) = out {
                return Some(v);
            }

            self.diags.push(
                Diag::error(
                    e.span.clone(),
                    format!("`{name}` constructor arguments are invalid"),
                )
                .with_help("vec2: vec2(x), vec2(x,y); vec3: vec3(x), vec3(v2,z), vec3(x,v2), vec3(x,y,z), vec3(v3); vec4: vec4(x), vec4(v3,w), vec4(w,v3), vec4(v2,v2), vec4(v2,y,z), vec4(x,v2,z), vec4(x,y,v2), vec4(x,y,z,w), vec4(v4)"),
            );
            return Some(Value::Error);
        }

        if name == "mat2" || name == "mat3" || name == "mat4" {
            let mut vals = Vec::with_capacity(args.len());
            for arg in args {
                vals.push(self.eval(&arg.value).unwrap_or(Value::Error));
            }

            if vals.iter().any(|v| matches!(v, Value::Error)) {
                return Some(Value::Error);
            }

            let out = match name.as_str() {
                "mat2" => {
                    if vals.len() == 2 {
                        match (&vals[0], &vals[1]) {
                            (Value::Vec2(c0), Value::Vec2(c1)) => {
                                Some(Value::Mat2((c0.clone(), c1.clone())))
                            }
                            _ => None,
                        }
                    } else if vals.len() == 4 {
                        let s = vals
                            .iter()
                            .map(Checker::as_numeric_scalar)
                            .collect::<Vec<_>>();
                        if s.iter().all(Option::is_some) {
                            let sx =
                                |i: usize| s[i].clone().map(|(v, _)| v).unwrap_or(Sx::Lit(0.0));
                            Some(Value::Mat2(((sx(0), sx(1)), (sx(2), sx(3)))))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                "mat3" => {
                    if vals.len() == 3 {
                        match (&vals[0], &vals[1], &vals[2]) {
                            (Value::Vec3(c0), Value::Vec3(c1), Value::Vec3(c2)) => {
                                Some(Value::Mat3(Box::new((c0.clone(), c1.clone(), c2.clone()))))
                            }
                            _ => None,
                        }
                    } else if vals.len() == 9 {
                        let s = vals
                            .iter()
                            .map(Checker::as_numeric_scalar)
                            .collect::<Vec<_>>();
                        if s.iter().all(Option::is_some) {
                            let sx =
                                |i: usize| s[i].clone().map(|(v, _)| v).unwrap_or(Sx::Lit(0.0));
                            Some(Value::Mat3(Box::new((
                                (sx(0), sx(1), sx(2)),
                                (sx(3), sx(4), sx(5)),
                                (sx(6), sx(7), sx(8)),
                            ))))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                "mat4" => {
                    if vals.len() == 4 {
                        match (&vals[0], &vals[1], &vals[2], &vals[3]) {
                            (
                                Value::Vec4(c0),
                                Value::Vec4(c1),
                                Value::Vec4(c2),
                                Value::Vec4(c3),
                            ) => Some(Value::Mat4(Box::new((
                                c0.clone(),
                                c1.clone(),
                                c2.clone(),
                                c3.clone(),
                            )))),
                            _ => None,
                        }
                    } else if vals.len() == 16 {
                        let s = vals
                            .iter()
                            .map(Checker::as_numeric_scalar)
                            .collect::<Vec<_>>();
                        if s.iter().all(Option::is_some) {
                            let sx =
                                |i: usize| s[i].clone().map(|(v, _)| v).unwrap_or(Sx::Lit(0.0));
                            Some(Value::Mat4(Box::new((
                                (sx(0), sx(1), sx(2), sx(3)),
                                (sx(4), sx(5), sx(6), sx(7)),
                                (sx(8), sx(9), sx(10), sx(11)),
                                (sx(12), sx(13), sx(14), sx(15)),
                            ))))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            };

            if let Some(v) = out {
                return Some(v);
            }

            self.diags.push(
                Diag::error(
                    e.span.clone(),
                    format!("`{name}` constructor arguments are invalid"),
                )
                .with_help("use columns: mat2(vec2, vec2), mat3(vec3, vec3, vec3), mat4(vec4, vec4, vec4, vec4) or scalar lists of 4/9/16"),
            );
            return Some(Value::Error);
        }

        if let Some(defs) = self.fn_defs.get(name) {
            // Authored overloads keep precedence at their declared arities.
            // An explicitly registered expression callable can supply another arity.
            let callable_overload = const_args.is_empty()
                && !defs.iter().any(|d| d.params.len() == args.len())
                && crate::registry::callable_decl_by_name(name).is_some_and(|decl| {
                    decl.context == "expr-call"
                        && args.len() >= decl.args.iter().filter(|a| a.required).count()
                        && args.len() <= decl.args.len()
                        && args.iter().all(|a| {
                            a.name
                                .as_ref()
                                .is_none_or(|name| decl.args.iter().any(|d| d.name == name))
                        })
                });
            if defs.iter().all(|d| d.is_builtin) || callable_overload {
                self.builtin(name, name_span, None, args, &e.span)
            } else {
                self.eval_user_fn_call(name, name_span, &e.span, const_args, args)
            }
        } else if let Some(struct_def) = self.struct_defs.get(name).cloned() {
            let mut field_values: HashMap<String, Value> = HashMap::new();
            for arg in args {
                let Some(field_name) = arg.name.as_ref() else {
                    self.diags.push(
                        Diag::error(
                            arg.value.span.clone(),
                            format!("struct constructor `{name}` requires named arguments"),
                        )
                        .with_help("use `TypeName(field: value, ...)`"),
                    );
                    return Some(Value::Error);
                };
                if field_values.contains_key(field_name) {
                    self.diags.push(
                        Diag::error(
                            arg.value.span.clone(),
                            format!(
                                "duplicate field argument `{field_name}` in `{name}` constructor"
                            ),
                        )
                        .with_help("provide each struct field at most once"),
                    );
                    return Some(Value::Error);
                }
                let Some(field_def) = struct_def.fields.get(field_name) else {
                    let mut available = struct_def.fields.keys().cloned().collect::<Vec<_>>();
                    available.sort();
                    self.diags.push(
                        Diag::error(
                            arg.value.span.clone(),
                            format!("unknown field `{field_name}` for struct `{name}`"),
                        )
                        .with_help(format!(
                            "available fields: {}",
                            if available.is_empty() {
                                "<none>".to_string()
                            } else {
                                available.join(", ")
                            }
                        )),
                    );
                    return Some(Value::Error);
                };

                let value = self.eval_expected(&arg.value, &field_def.ty_name)?;
                if !local_decl_type_matches(
                    &value,
                    &field_def.ty_name,
                    &self.enum_defs,
                    &self.struct_defs,
                    &[],
                ) {
                    self.diags.push(
                        Diag::error(
                            arg.value.span.clone(),
                            format!(
                                "field `{field_name}` expected {}, found {}",
                                local_decl_expected_kind(
                                    &field_def.ty_name,
                                    &self.enum_defs,
                                    &self.struct_defs,
                                    &[]
                                ),
                                value.kind()
                            ),
                        )
                        .with_help("adjust the argument to match the field type"),
                    );
                    return Some(Value::Error);
                }

                field_values.insert(field_name.clone(), value);
            }

            let mut missing = Vec::new();
            for field_name in struct_def.fields.keys() {
                if !field_values.contains_key(field_name) {
                    missing.push(field_name.clone());
                }
            }
            if !missing.is_empty() {
                missing.sort();
                self.diags.push(
                    Diag::error(
                        e.span.clone(),
                        format!(
                            "struct constructor `{name}` is missing field arguments: {}",
                            missing.join(", ")
                        ),
                    )
                    .with_help("provide values for all struct fields"),
                );
                return Some(Value::Error);
            }

            Some(Value::Struct {
                ty_name: name.clone(),
                fields: field_values,
            })
        } else if let Some(Value::FnRef(target)) = self.lookup(name) {
            // Callable parameter dispatch (§16.5/§16.1 Phase 6): `f(args)` where `f` is
            // bound to a fn reference or an effect reference.
            if self.effect_defs.contains_key(target.as_str()) {
                // Effect reference: route through the builtin dispatcher, which checks
                // effect_defs before the builtin registry and emits Layer::UserEffect.
                let target = target.clone();
                self.builtin(&target, name_span, None, args, &e.span)
            } else {
                self.eval_user_fn_call(&target, name_span, &e.span, &[], args)
            }
        } else {
            self.builtin(name, name_span, None, args, &e.span)
        }
    }

    #[inline(never)]
    fn eval_vec2_expr(&mut self, e: &SExpr) -> Option<Value> {
        let Expr::Vec2(a, b) = &e.node else {
            unreachable!("expression dispatcher must match evaluator");
        };
        let av = self.eval(a)?;
        let bv = self.eval(b)?;
        if matches!(av, Value::Error) || matches!(bv, Value::Error) {
            return Some(Value::Error);
        }
        if let (Value::Vec2(va), Value::Vec2(_)) = (&av, &bv)
            && a.span == b.span
        {
            return Some(Value::Vec2(va.clone()));
        }

        let Some((x, _)) = Self::as_numeric_scalar(&av) else {
            self.diags.push(
                Diag::error(a.span.clone(), format!("expected a scalar, found a {}", av.kind()))
                    .with_help("scalars: numbers with optional units (0.5, 12px, 20deg, 2s), time, arithmetic thereof"),
            );
            return None;
        };
        let Some((y, _)) = Self::as_numeric_scalar(&bv) else {
            self.diags.push(
                Diag::error(b.span.clone(), format!("expected a scalar, found a {}", bv.kind()))
                    .with_help("scalars: numbers with optional units (0.5, 12px, 20deg, 2s), time, arithmetic thereof"),
            );
            return None;
        };
        Some(Value::Vec2((x, y)))
    }

    #[inline(never)]
    fn eval_vec3_expr(&mut self, e: &SExpr) -> Option<Value> {
        let Expr::Vec3(a, b, c) = &e.node else {
            unreachable!("expression dispatcher must match evaluator");
        };
        let av = self.eval(a)?;
        let bv = self.eval(b)?;
        let cv = self.eval(c)?;
        if matches!(av, Value::Error) || matches!(bv, Value::Error) || matches!(cv, Value::Error) {
            return Some(Value::Error);
        }
        if let (Value::Vec3(va), Value::Vec3(_), Value::Vec3(_)) = (&av, &bv, &cv)
            && a.span == b.span
            && b.span == c.span
        {
            return Some(Value::Vec3(va.clone()));
        }

        let Some((x, _)) = Self::as_numeric_scalar(&av) else {
            self.diags.push(
                Diag::error(a.span.clone(), format!("expected a scalar, found a {}", av.kind()))
                    .with_help("scalars: numbers with optional units (0.5, 12px, 20deg, 2s), time, arithmetic thereof"),
            );
            return None;
        };
        let Some((y, _)) = Self::as_numeric_scalar(&bv) else {
            self.diags.push(
                Diag::error(b.span.clone(), format!("expected a scalar, found a {}", bv.kind()))
                    .with_help("scalars: numbers with optional units (0.5, 12px, 20deg, 2s), time, arithmetic thereof"),
            );
            return None;
        };
        let Some((z, _)) = Self::as_numeric_scalar(&cv) else {
            self.diags.push(
                Diag::error(c.span.clone(), format!("expected a scalar, found a {}", cv.kind()))
                    .with_help("scalars: numbers with optional units (0.5, 12px, 20deg, 2s), time, arithmetic thereof"),
            );
            return None;
        };
        Some(Value::Vec3((x, y, z)))
    }

    #[inline(never)]
    fn eval_vec4_expr(&mut self, e: &SExpr) -> Option<Value> {
        let Expr::Vec4(a, b, c, d) = &e.node else {
            unreachable!("expression dispatcher must match evaluator");
        };
        let av = self.eval(a)?;
        let bv = self.eval(b)?;
        let cv = self.eval(c)?;
        let dv = self.eval(d)?;
        if matches!(av, Value::Error)
            || matches!(bv, Value::Error)
            || matches!(cv, Value::Error)
            || matches!(dv, Value::Error)
        {
            return Some(Value::Error);
        }
        if let (Value::Vec4(va), Value::Vec4(_), Value::Vec4(_), Value::Vec4(_)) =
            (&av, &bv, &cv, &dv)
            && a.span == b.span
            && b.span == c.span
            && c.span == d.span
        {
            return Some(Value::Vec4(va.clone()));
        }

        let Some((x, _)) = Self::as_numeric_scalar(&av) else {
            self.diags.push(
                Diag::error(a.span.clone(), format!("expected a scalar, found a {}", av.kind()))
                    .with_help("scalars: numbers with optional units (0.5, 12px, 20deg, 2s), time, arithmetic thereof"),
            );
            return None;
        };
        let Some((y, _)) = Self::as_numeric_scalar(&bv) else {
            self.diags.push(
                Diag::error(b.span.clone(), format!("expected a scalar, found a {}", bv.kind()))
                    .with_help("scalars: numbers with optional units (0.5, 12px, 20deg, 2s), time, arithmetic thereof"),
            );
            return None;
        };
        let Some((z, _)) = Self::as_numeric_scalar(&cv) else {
            self.diags.push(
                Diag::error(c.span.clone(), format!("expected a scalar, found a {}", cv.kind()))
                    .with_help("scalars: numbers with optional units (0.5, 12px, 20deg, 2s), time, arithmetic thereof"),
            );
            return None;
        };
        let Some((w, _)) = Self::as_numeric_scalar(&dv) else {
            self.diags.push(
                Diag::error(d.span.clone(), format!("expected a scalar, found a {}", dv.kind()))
                    .with_help("scalars: numbers with optional units (0.5, 12px, 20deg, 2s), time, arithmetic thereof"),
            );
            return None;
        };
        Some(Value::Vec4((x, y, z, w)))
    }

    #[inline(never)]
    fn eval_array_comp_expr(&mut self, e: &SExpr) -> Option<Value> {
        let Expr::ArrayComp {
            name,
            iterable,
            body,
            ..
        } = &e.node
        else {
            unreachable!("expression dispatcher must match evaluator");
        };
        let iter_values = self.eval(iterable)?;
        let items = match iter_values {
            Value::Array(items) => items,
            Value::Vec2((start, end)) => {
                let start =
                    self.eval_const_scalar_sx(&start, &iterable.span, "comprehension range start")?;
                let end =
                    self.eval_const_scalar_sx(&end, &iterable.span, "comprehension range end")?;
                if start.fract() != 0.0 || end.fract() != 0.0 {
                    self.diags.push(
                        Diag::error(
                            iterable.span.clone(),
                            "comprehension range bounds must be integers",
                        )
                        .with_help("use integer literals, for example `[for i in 0 .. 8 => ...]`"),
                    );
                    return None;
                }

                let start_i = start as i32;
                let end_i = end as i32;
                if start_i <= end_i {
                    (start_i..end_i)
                        .map(|i| Value::Scalar(Sx::Lit(i as f32)))
                        .collect::<Vec<_>>()
                } else {
                    (end_i..start_i)
                        .rev()
                        .map(|i| Value::Scalar(Sx::Lit(i as f32)))
                        .collect::<Vec<_>>()
                }
            }
            other => {
                self.diags.push(
                    Diag::error(
                        iterable.span.clone(),
                        format!(
                            "array comprehension expects an array or range iterable, found {}",
                            other.kind()
                        ),
                    )
                    .with_help("use `[for i in 0 .. n => ...]` or `[for x in some_array => ...]`"),
                );
                return None;
            }
        };

        let mut out = Vec::with_capacity(items.len());
        for item in items {
            self.scopes.push(HashMap::new());
            self.bind(name.clone(), item);
            let value = self.eval(body);
            self.scopes.pop();

            let value = value?;
            if matches!(value, Value::Error) {
                return Some(Value::Error);
            }
            if matches!(
                value,
                Value::Layer(_) | Value::Space(_) | Value::ScatterInstance { .. }
            ) {
                self.diags.push(
                    Diag::error(
                        body.span.clone(),
                        format!(
                            "array comprehension body cannot produce {} values",
                            value.kind()
                        ),
                    )
                    .with_help(
                        "return scalar, vec2, color, or shape values from the comprehension body",
                    ),
                );
                return None;
            }
            out.push(value);
        }

        Some(Value::Array(out))
    }

    #[inline(never)]
    fn eval_field_at_expr(&mut self, e: &SExpr) -> Option<Value> {
        let Expr::FieldAt { inner, coord } = &e.node else {
            unreachable!("expression dispatcher must match evaluator");
        };
        let inner_value = self.eval(inner)?;
        // Evaluate the new coordinate expression.
        let coord_val = self.eval(coord)?;
        let (new_x, new_y) = match coord_val {
            Value::Vec2(v) => v,
            Value::Error => return Some(Value::Error),
            other => {
                self.diags.push(
                    Diag::error(
                        coord.span.clone(),
                        format!("`at` expects a vec2 coordinate, found {}", other.kind()),
                    )
                    .with_help("provide a vec2 expression such as `coord + offset`"),
                );
                return None;
            }
        };
        match inner_value {
            Value::Scalar(s) | Value::Distance(s) | Value::Coverage(s) | Value::Mask(s) => {
                Some(Value::Distance(s.subst_coord(&new_x, &new_y)))
            }
            Value::Slot {
                index,
                count,
                start,
                end,
            } => Some(Value::Slot {
                index: index.subst_coord(&new_x, &new_y),
                count: count.subst_coord(&new_x, &new_y),
                start: start.subst_coord(&new_x, &new_y),
                end: end.subst_coord(&new_x, &new_y),
            }),
            Value::ColorField { rgba, space } => Some(Value::ColorField {
                rgba: [
                    rgba[0].clone().subst_coord(&new_x, &new_y),
                    rgba[1].clone().subst_coord(&new_x, &new_y),
                    rgba[2].clone().subst_coord(&new_x, &new_y),
                    rgba[3].clone().subst_coord(&new_x, &new_y),
                ],
                space,
            }),
            Value::Color { rgba, space } => Some(Value::Color { rgba, space }),
            other => {
                self.diags.push(
                    Diag::error(
                        inner.span.clone(),
                        format!(
                            "`at` expects a field-like scalar or color expression, found {}",
                            other.kind()
                        ),
                    )
                    .with_help(
                        "use `field expr at coord` for scalar fields, or resample a color field such as `self at coord` inside an effect",
                    ),
                );
                None
            }
        }
    }

    #[inline(never)]
    fn eval_layer_expr(&mut self, e: &SExpr) -> Option<Value> {
        let Expr::Layer(inner) = &e.node else {
            unreachable!("expression dispatcher must match evaluator");
        };
        match self.eval(inner)? {
            Value::Error => Some(Value::Error),
            Value::Layer(l) => Some(Value::Layer(l)),
            Value::Color { rgba, .. } => Some(Value::Layer(self.hir.layer(Layer::Solid(rgba)))),
            Value::ColorField { rgba, .. } => {
                let [r, g, b, a] = rgba;
                Some(Value::Layer(self.hir.layer(Layer::ColorExpr {
                    r,
                    g,
                    b,
                    a,
                })))
            }
            Value::Scalar(_) => {
                self.diags.push(
                    Diag::error(
                        inner.span.clone(),
                        "`layer` expects a color/layer expression, found scalar",
                    )
                    .with_help("use `grey(...)` to lift a scalar into a layer"),
                );
                None
            }
            v => {
                self.diags.push(Diag::error(
                    inner.span.clone(),
                    format!(
                        "`layer` expects a color/layer expression, found {}",
                        v.kind()
                    ),
                ));
                None
            }
        }
    }

    #[inline(never)]
    fn eval_unary_expr(&mut self, e: &SExpr) -> Option<Value> {
        let Expr::Unary(UnOp::Neg, inner) = &e.node else {
            unreachable!("expression dispatcher must match evaluator");
        };
        let value = self.eval(inner)?;
        if let Some(kind) = Self::value_element_kind(&value) {
            if kind == crate::typed_scalar::Kind::I32 {
                return Some(Self::map_value_lanes(value, |lane| {
                    crate::typed_scalar::Scalar {
                        kind,
                        op: crate::typed_scalar::Op::Negate,
                        args: vec![lane],
                    }
                    .sx()
                }));
            }
            if matches!(
                kind,
                crate::typed_scalar::Kind::U32 | crate::typed_scalar::Kind::Bool
            ) {
                self.diags.push(Diag::error(
                    e.span.clone(),
                    "cannot negate an unsigned integer or bool",
                ));
                return None;
            }
        }
        match value {
            Value::Error => Some(Value::Error),
            Value::Scalar(s) => Some(Value::Scalar(Self::fold_neg_if_literal(s))),
            Value::Distance(s) => Some(Value::Distance(Self::fold_neg_if_literal(s))),
            Value::Coverage(s) => Some(Value::Coverage(Self::fold_neg_if_literal(s))),
            Value::Mask(s) => Some(Value::Mask(Self::fold_neg_if_literal(s))),
            Value::Vec2((x, y)) => Some(Value::Vec2((
                Self::fold_neg_if_literal(x),
                Self::fold_neg_if_literal(y),
            ))),
            Value::Vec3((x, y, z)) => Some(Value::Vec3((
                Self::fold_neg_if_literal(x),
                Self::fold_neg_if_literal(y),
                Self::fold_neg_if_literal(z),
            ))),
            Value::Vec4((x, y, z, w)) => Some(Value::Vec4((
                Self::fold_neg_if_literal(x),
                Self::fold_neg_if_literal(y),
                Self::fold_neg_if_literal(z),
                Self::fold_neg_if_literal(w),
            ))),
            v => {
                self.diags.push(Diag::error(
                    e.span.clone(),
                    format!("cannot negate a {}", v.kind()),
                ));
                None
            }
        }
    }

    #[inline(never)]
    fn eval_pipe_expr(&mut self, e: &SExpr) -> Option<Value> {
        let Expr::Pipe {
            recv,
            name,
            name_span,
            args,
        } = &e.node
        else {
            unreachable!("expression dispatcher must match evaluator");
        };
        let recv = self.eval(recv)?;
        if matches!(recv, Value::Error) {
            return Some(Value::Error);
        }
        if let Some(style) = self.lookup_style(name) {
            let mut style_stack = Vec::new();
            return self.apply_style_chain(name, &style, recv, args, &e.span, &mut style_stack);
        }

        if self.fn_defs.contains_key(name) && crate::registry::builtin_decl_by_name(name).is_empty()
        {
            return self.with_piped_arg(recv, args, &e.span, |ctx, piped_args| {
                ctx.eval_user_fn_call(name, name_span, &e.span, &[], piped_args)
            });
        }

        if let Some(Value::FnRef(target)) = self.lookup(name)
            && !self.effect_defs.contains_key(target.as_str())
        {
            return self.with_piped_arg(recv, args, &e.span, |ctx, piped_args| {
                ctx.eval_user_fn_call(&target, name_span, &e.span, &[], piped_args)
            });
        }

        self.builtin(name, name_span, Some(recv), args, &e.span)
    }

    #[inline(never)]
    fn eval_through_expr(&mut self, e: &SExpr) -> Option<Value> {
        let Expr::Through { layer, chain } = &e.node else {
            unreachable!("expression dispatcher must match evaluator");
        };
        let recv = self.eval(layer)?;
        if matches!(recv, Value::Error) {
            return Some(Value::Error);
        }
        let Value::Layer(inner_id) = recv else {
            self.diags.push(
                Diag::error(
                    layer.span.clone(),
                    format!("`through space` expects a layer, found {}", recv.kind()),
                )
                .with_help(
                    "use `layer <expr>` to lift a color to a layer, or name a layer with `let`",
                ),
            );
            return None;
        };
        let resolved = self.eval_space_chain(chain)?;
        let new_id = self.hir.layer(Layer::InSpace {
            xforms: resolved.xforms,
            inner: inner_id,
        });
        Some(Value::Layer(new_id))
    }

    #[inline(never)]
    fn eval_index_expr(&mut self, e: &SExpr) -> Option<Value> {
        let Expr::Index { array, index } = &e.node else {
            unreachable!("expression dispatcher must match evaluator");
        };
        let arr_val = self.eval(array)?;
        if matches!(arr_val, Value::Error) {
            return Some(Value::Error);
        }

        // Dynamic array parameter: allow runtime (non-constant) indices.
        if let Value::DynamicArray {
            param_name,
            elem_type,
        } = arr_val
        {
            let idx_val = self.eval(index)?;
            if matches!(idx_val, Value::Error) {
                return Some(Value::Error);
            }
            let Some((idx_sx, _kind)) = Self::as_numeric_scalar(&idx_val) else {
                self.diags.push(
                    Diag::error(
                        index.span.clone(),
                        format!("array index must be a scalar, found {}", idx_val.kind()),
                    )
                    .with_help("use an integer or scalar expression as the index"),
                );
                return None;
            };
            use crate::hir::ArrayElemType;
            return match elem_type {
                ArrayElemType::F32
                | ArrayElemType::I32
                | ArrayElemType::U32
                | ArrayElemType::Bool => {
                    let kind = match elem_type {
                        ArrayElemType::F32 => crate::typed_scalar::Kind::F32,
                        ArrayElemType::I32 => crate::typed_scalar::Kind::I32,
                        ArrayElemType::U32 => crate::typed_scalar::Kind::U32,
                        ArrayElemType::Bool => crate::typed_scalar::Kind::Bool,
                        _ => unreachable!("scalar array element"),
                    };
                    Some(Value::Scalar(crate::typed_scalar::Scalar::cast(
                        Sx::DynamicArrayIndex {
                            param_name,
                            index: Box::new(idx_sx),
                            component: None,
                        },
                        kind,
                    )))
                }
                ArrayElemType::Vec2 => Some(Value::Vec2((
                    Sx::DynamicArrayIndex {
                        param_name: param_name.clone(),
                        index: Box::new(idx_sx.clone()),
                        component: Some(0),
                    },
                    Sx::DynamicArrayIndex {
                        param_name,
                        index: Box::new(idx_sx),
                        component: Some(1),
                    },
                ))),
                ArrayElemType::Vec3 => Some(Value::Vec3((
                    Sx::DynamicArrayIndex {
                        param_name: param_name.clone(),
                        index: Box::new(idx_sx.clone()),
                        component: Some(0),
                    },
                    Sx::DynamicArrayIndex {
                        param_name: param_name.clone(),
                        index: Box::new(idx_sx.clone()),
                        component: Some(1),
                    },
                    Sx::DynamicArrayIndex {
                        param_name,
                        index: Box::new(idx_sx),
                        component: Some(2),
                    },
                ))),
                ArrayElemType::Vec4 => Some(Value::Vec4((
                    Sx::DynamicArrayIndex {
                        param_name: param_name.clone(),
                        index: Box::new(idx_sx.clone()),
                        component: Some(0),
                    },
                    Sx::DynamicArrayIndex {
                        param_name: param_name.clone(),
                        index: Box::new(idx_sx.clone()),
                        component: Some(1),
                    },
                    Sx::DynamicArrayIndex {
                        param_name: param_name.clone(),
                        index: Box::new(idx_sx.clone()),
                        component: Some(2),
                    },
                    Sx::DynamicArrayIndex {
                        param_name,
                        index: Box::new(idx_sx),
                        component: Some(3),
                    },
                ))),
                _ => {
                    self.diags.push(
                        Diag::error(
                            array.span.clone(),
                            format!(
                                "indexing a dynamic array with element type `{}` is not yet supported",
                                elem_type.as_str()
                            ),
                        )
                        .with_help("supported element types for indexing: f32, i32, u32, bool, vec2, vec3, vec4"),
                    );
                    None
                }
            };
        }

        let Value::Array(items) = arr_val else {
            self.diags.push(
                Diag::error(
                    array.span.clone(),
                    format!(
                        "index operator `[...]` requires an array, found {}",
                        arr_val.kind()
                    ),
                )
                .with_help(
                    "only array values can be indexed; use `[a, b, c]` or a fixed-size array param",
                ),
            );
            return None;
        };

        let idx_val = self.eval(index)?;
        if matches!(idx_val, Value::Error) {
            return Some(Value::Error);
        }
        let Some((idx_sx, _kind)) = Self::as_numeric_scalar(&idx_val) else {
            self.diags.push(
                Diag::error(
                    index.span.clone(),
                    format!("array index must be a scalar, found {}", idx_val.kind()),
                )
                .with_help("use an integer literal or compile-time scalar expression as the index"),
            );
            return None;
        };
        let Some(idx_f) = Self::try_eval_static_scalar(&idx_sx) else {
            self.diags.push(
                Diag::error(
                    index.span.clone(),
                    "array index must be a compile-time constant",
                )
                .with_help("only integer literals or constant scalar expressions are supported as array indices"),
            );
            return None;
        };
        if idx_f.fract() != 0.0 {
            self.diags.push(
                Diag::error(
                    index.span.clone(),
                    format!("array index must be an integer, found {idx_f}"),
                )
                .with_help("use a whole-number index such as `0`, `1`, `2`"),
            );
            return None;
        }
        let idx = idx_f as i64;
        let len = items.len() as i64;
        if idx < 0 || idx >= len {
            self.diags.push(
                Diag::error(
                    index.span.clone(),
                    format!("array index {idx} is out of bounds for an array of length {len}"),
                )
                .with_help(format!(
                    "valid indices are 0 through {}",
                    len.saturating_sub(1)
                )),
            );
            return None;
        }
        Some(items[idx as usize].clone())
    }

    fn eval_inner(&mut self, e: &SExpr) -> Option<Value> {
        match &e.node {
            Expr::Num(v, unit) => Some(Value::Scalar(self.scalar_lit_runtime(*v, *unit))),
            Expr::Color(c) => Some(Value::Color {
                rgba: *c,
                space: ColorSpace::Srgb,
            }),
            Expr::Str(_) => {
                self.diags.push(
                    Diag::error(
                        e.span.clone(),
                        "string literals are only supported in specific compile-time constructors",
                    )
                    .with_help("use string literals directly with constructors like `svg(...)` and `svg_path(...)`"),
                );
                Some(Value::Error)
            }
            Expr::Var(name) => {
                if let Some(v) = self.resolve_var_name(name, &e.span) {
                    return Some(v);
                }

                self.diags.push(
                    Diag::error(e.span.clone(), format!("unknown name `{name}`"))
                        .with_help("use declared names in this canvas scope (include one `: coord` parameter and optional `: signal` / `: delta` / `: resolution` parameters)"),
                );
                Some(Value::Error)
            }
            Expr::Vec2(..) => self.eval_vec2_expr(e),
            Expr::Vec3(..) => self.eval_vec3_expr(e),
            Expr::Vec4(..) => self.eval_vec4_expr(e),
            Expr::Range(a, b) => {
                let lo = self.as_scalar(a)?;
                let hi = self.as_scalar(b)?;
                Some(Value::Vec2((lo, hi)))
            }
            Expr::Member(recv, field) => {
                let base = self.eval(recv)?;
                self.select_component("<expr>", field, base, &e.span)
            }
            Expr::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    let value = self.eval(item)?;
                    if matches!(value, Value::Error) {
                        return Some(Value::Error);
                    }
                    if matches!(
                        value,
                        Value::Layer(_) | Value::Space(_) | Value::ScatterInstance { .. }
                    ) {
                        self.diags.push(
                            Diag::error(
                                item.span.clone(),
                                format!("array elements cannot be {} values", value.kind()),
                            )
                            .with_help(
                                "arrays currently support scalar, vec2, vec3, vec4, color, and shape elements",
                            ),
                        );
                        return None;
                    }
                    out.push(value);
                }
                Some(Value::Array(out))
            }
            Expr::ArrayComp { .. } => self.eval_array_comp_expr(e),
            Expr::Lambda { params, body } => Some(Value::Lambda {
                params: params.iter().map(|p| p.node.clone()).collect::<Vec<_>>(),
                body: body.clone(),
            }),
            Expr::Field(inner) => {
                let s = self.as_scalar(inner)?;
                Some(Value::Distance(s))
            }
            Expr::FieldAt { .. } => self.eval_field_at_expr(e),
            Expr::Layer(..) => self.eval_layer_expr(e),
            Expr::PathFuture { commands } => {
                let path = self.build_path_profile(commands, &e.span)?;
                self.hir.notes.push(format!(
                    "path: lowered {} command(s) into sampled polyline profile (length ~ {:.4})",
                    commands.len(),
                    Self::try_eval_static_scalar(&path.total_length_sx()).unwrap_or_default()
                ));
                Some(Value::PathFuture(path))
            }
            Expr::Unary(..) => self.eval_unary_expr(e),
            Expr::Binary(op, l, r) => self.eval_binary(*op, l, r, &e.span),
            Expr::Call { .. } => self.eval_call_expr(e),
            Expr::Pipe { .. } => self.eval_pipe_expr(e),
            Expr::Through { .. } => self.eval_through_expr(e),
            Expr::Index { .. } => self.eval_index_expr(e),
        }
    }

    fn apply_style_chain(
        &mut self,
        style_name: &str,
        style: &StyleDef,
        mut recv: Value,
        style_args: &[Arg],
        call_span: &Span,
        style_stack: &mut Vec<String>,
    ) -> Option<Value> {
        if style_stack.iter().any(|name| name == style_name) {
            let cycle = style_stack
                .iter()
                .cloned()
                .chain(std::iter::once(style_name.to_string()))
                .collect::<Vec<_>>()
                .join(" -> ");
            self.diags.push(
                Diag::error(
                    style.span.clone(),
                    format!("recursive style expansion: {cycle}"),
                )
                .with_help("remove the recursive style reference cycle"),
            );
            return None;
        }

        let stage_chain = style
            .stages
            .iter()
            .map(|stage| {
                if stage.args.is_empty() {
                    stage.name.clone()
                } else {
                    format!("{}({} arg(s))", stage.name, stage.args.len())
                }
            })
            .collect::<Vec<_>>()
            .join(" |> ");

        let mut bag = ArgBag::new(style_name, style_args, call_span.clone());

        style_stack.push(style_name.to_string());
        self.scopes.push(HashMap::new());
        for param in &style.params {
            let arg_expr = bag.take_named(&param.name).unwrap_or(&param.default);
            let Some(value) = self.eval(arg_expr) else {
                self.scopes.pop();
                style_stack.pop();
                return None;
            };
            if matches!(value, Value::Error) {
                self.scopes.pop();
                style_stack.pop();
                return Some(Value::Error);
            }
            self.bind(param.name.clone(), value);
        }
        bag.finish(&mut self.diags);

        for stage in &style.stages {
            if let Some(nested_style) = self.lookup_style(&stage.name) {
                recv = self.apply_style_chain(
                    &stage.name,
                    &nested_style,
                    recv,
                    &stage.args,
                    call_span,
                    style_stack,
                )?;
                continue;
            }

            recv = self.builtin(
                &stage.name,
                &stage.name_span,
                Some(recv),
                &stage.args,
                call_span,
            )?;
            if matches!(recv, Value::Error) {
                self.scopes.pop();
                style_stack.pop();
                return Some(Value::Error);
            }
        }

        self.scopes.pop();
        self.hir.notes.push(format!(
            "style: expanded `{style_name}` as {} ({} stage(s))",
            stage_chain,
            style.stages.len(),
        ));
        style_stack.pop();
        Some(recv)
    }

    fn eval_binary(&mut self, op: BinOp, l: &SExpr, r: &SExpr, span: &Span) -> Option<Value> {
        self.eval_binary_expected(op, l, r, span, None)
    }

    fn eval_binary_expected(
        &mut self,
        op: BinOp,
        l: &SExpr,
        r: &SExpr,
        span: &Span,
        expected: Option<crate::typed_scalar::Kind>,
    ) -> Option<Value> {
        let _span = trace_span!("check.eval_binary", op = ?op).entered();
        let lv = self.eval_scalar_expected(l, expected)?;
        let right_kind = if matches!(op, BinOp::Shl | BinOp::Shr)
            && matches!(
                expected,
                Some(crate::typed_scalar::Kind::I32 | crate::typed_scalar::Kind::U32)
            ) {
            Some(crate::typed_scalar::Kind::U32)
        } else {
            expected
        };
        let rv = self.eval_scalar_expected(r, right_kind)?;
        self.eval_binary_values(op, l, r, span, lv, rv)
    }

    fn eval_binary_values(
        &mut self,
        op: BinOp,
        l: &SExpr,
        r: &SExpr,
        span: &Span,
        lv: Value,
        rv: Value,
    ) -> Option<Value> {
        let t0 = Instant::now();
        let native = [Self::value_element_kind(&lv), Self::value_element_kind(&rv)]
            .into_iter()
            .flatten()
            .any(|kind| kind != crate::typed_scalar::Kind::F32);
        if native
            && (matches!(lv, Value::Vec2(_) | Value::Vec3(_) | Value::Vec4(_))
                || matches!(rv, Value::Vec2(_) | Value::Vec3(_) | Value::Vec4(_)))
        {
            fn lanes(value: &Value) -> Option<Vec<Sx>> {
                match value {
                    Value::Scalar(s) => Some(vec![s.clone()]),
                    Value::Vec2((x, y)) => Some(vec![x.clone(), y.clone()]),
                    Value::Vec3((x, y, z)) => Some(vec![x.clone(), y.clone(), z.clone()]),
                    Value::Vec4((x, y, z, w)) => {
                        Some(vec![x.clone(), y.clone(), z.clone(), w.clone()])
                    }
                    _ => None,
                }
            }
            if let (Some(left), Some(right)) = (lanes(&lv), lanes(&rv)) {
                let width = left.len().max(right.len());
                if (left.len() == 1 || left.len() == width)
                    && (right.len() == 1 || right.len() == width)
                {
                    let mut result = Vec::with_capacity(width);
                    for i in 0..width {
                        let Value::Scalar(value) = self.eval_binary_values(
                            op,
                            l,
                            r,
                            span,
                            Value::Scalar(left[i % left.len()].clone()),
                            Value::Scalar(right[i % right.len()].clone()),
                        )?
                        else {
                            return None;
                        };
                        result.push(value);
                    }
                    return Some(match result.as_slice() {
                        [x, y] => Value::Vec2((x.clone(), y.clone())),
                        [x, y, z] => Value::Vec3((x.clone(), y.clone(), z.clone())),
                        [x, y, z, w] => Value::Vec4((x.clone(), y.clone(), z.clone(), w.clone())),
                        _ => unreachable!("checked vector width"),
                    });
                }
            }
            self.diags.push(Diag::error(
                span.clone(),
                "native vector operands must have compatible dimensions",
            ));
            return None;
        }
        if matches!(lv, Value::Error) || matches!(rv, Value::Error) {
            self.record_check_profile_timing("eval_binary", t0.elapsed());
            return Some(Value::Error);
        }
        if let (Value::Scalar(a), Value::Scalar(b)) = (&lv, &rv) {
            use crate::typed_scalar::{Kind, Op, Scalar};
            let ak = a.scalar_kind();
            let bk = b.scalar_kind();
            if ak != Kind::F32
                || bk != Kind::F32
                || matches!(
                    op,
                    BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::Eq | BinOp::Ne
                )
            {
                let kind = if ak != Kind::F32 { ak } else { bk };
                fn abstract_numeric(expression: &SExpr) -> bool {
                    match &expression.node {
                        Expr::Num(_, Unit::None) => true,
                        Expr::Unary(UnOp::Neg, inner) => abstract_numeric(inner),
                        Expr::Binary(
                            BinOp::Add
                            | BinOp::Sub
                            | BinOp::Mul
                            | BinOp::Div
                            | BinOp::Mod
                            | BinOp::Shl
                            | BinOp::Shr
                            | BinOp::BitXor,
                            left,
                            right,
                        ) => abstract_numeric(left) && abstract_numeric(right),
                        _ => false,
                    }
                }
                let mut materialize = |value: &Sx, expr: &SExpr, kind| -> Result<Sx, String> {
                    if value.scalar_kind() == kind {
                        return Ok(value.clone());
                    }
                    if abstract_numeric(expr) {
                        return match self.eval_scalar_expected(expr, Some(kind)) {
                            Some(Value::Scalar(value)) => Ok(value),
                            _ => Err(format!(
                                "literal expression is not representable as {}",
                                kind.name()
                            )),
                        };
                    }
                    Err(format!(
                        "operator requires matching scalar types, found {} and {}",
                        ak.name(),
                        bk.name()
                    ))
                };
                let operands = materialize(a, l, kind).and_then(|a| {
                    materialize(
                        b,
                        r,
                        if matches!(op, BinOp::Shl | BinOp::Shr) {
                            Kind::U32
                        } else {
                            kind
                        },
                    )
                    .map(|b| vec![a, b])
                });
                let comparison = matches!(
                    op,
                    BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::Eq | BinOp::Ne
                );
                let operation = match op {
                    BinOp::LogicalAnd => naga::BinaryOperator::LogicalAnd,
                    BinOp::LogicalOr => naga::BinaryOperator::LogicalOr,
                    BinOp::Add => naga::BinaryOperator::Add,
                    BinOp::Sub => naga::BinaryOperator::Subtract,
                    BinOp::Mul => naga::BinaryOperator::Multiply,
                    BinOp::Div => naga::BinaryOperator::Divide,
                    BinOp::Mod => naga::BinaryOperator::Modulo,
                    BinOp::Shl => naga::BinaryOperator::ShiftLeft,
                    BinOp::Shr => naga::BinaryOperator::ShiftRight,
                    BinOp::BitXor => naga::BinaryOperator::ExclusiveOr,
                    BinOp::Union => naga::BinaryOperator::InclusiveOr,
                    BinOp::Intersect => naga::BinaryOperator::And,
                    BinOp::Lt => naga::BinaryOperator::Less,
                    BinOp::Le => naga::BinaryOperator::LessEqual,
                    BinOp::Gt => naga::BinaryOperator::Greater,
                    BinOp::Ge => naga::BinaryOperator::GreaterEqual,
                    BinOp::Eq => naga::BinaryOperator::Equal,
                    BinOp::Ne => naga::BinaryOperator::NotEqual,
                };
                if kind == Kind::Bool
                    && !matches!(
                        op,
                        BinOp::Eq
                            | BinOp::Ne
                            | BinOp::Union
                            | BinOp::Intersect
                            | BinOp::BitXor
                            | BinOp::LogicalAnd
                            | BinOp::LogicalOr
                    )
                {
                    self.diags.push(Diag::error(
                        span.clone(),
                        "operator is not defined for bool",
                    ));
                    return None;
                }
                return match operands {
                    Ok(args) => Some(Value::Scalar(
                        Scalar {
                            kind: if comparison { Kind::Bool } else { kind },
                            op: Op::Binary(operation),
                            args,
                        }
                        .sx(),
                    )),
                    Err(message) => {
                        self.diags.push(Diag::error(span.clone(), message));
                        None
                    }
                };
            }
        }
        let lv_kind = lv.kind();
        let rv_kind = rv.kind();
        let lhs_numeric = Self::as_numeric_scalar(&lv);
        let rhs_numeric = Self::as_numeric_scalar(&rv);
        // Computed once and reused below — `as_color_field_value` takes `&Value`
        // specifically so this doesn't deep-clone `lv`/`rv` (which may carry an
        // arbitrarily large accumulated expression tree) just to immediately
        // discard the clone for the overwhelmingly common non-color case.
        let lv_color = Self::as_color_field_value(&lv);
        let rv_color = Self::as_color_field_value(&rv);

        if let (Some((ca_raw, sa)), Some((cb_raw, sb))) = (lv_color.clone(), rv_color.clone()) {
            let ca = Self::to_working_color_space(ca_raw, sa);
            let cb = Self::to_working_color_space(cb_raw, sb);
            if let Some(out) = Self::color_bin(op, ca, cb) {
                return Some(Value::ColorField {
                    rgba: out,
                    space: ColorSpace::Linear,
                });
            }
        }

        if let (Some((ca_raw, sa)), Some(sb)) = (
            lv_color.clone(),
            rhs_numeric.as_ref().map(|(sx, _)| sx.clone()),
        ) {
            let ca = Self::to_working_color_space(ca_raw, sa);
            if let Some(out) = Self::color_bin(op, ca, [sb.clone(), sb.clone(), sb.clone(), sb]) {
                return Some(Value::ColorField {
                    rgba: out,
                    space: ColorSpace::Linear,
                });
            }
        }

        if let (Some(sa), Some((cb_raw, sb_space))) = (
            lhs_numeric.as_ref().map(|(sx, _)| sx.clone()),
            rv_color.clone(),
        ) {
            let cb = Self::to_working_color_space(cb_raw, sb_space);
            if let Some(out) = Self::color_bin(op, [sa.clone(), sa.clone(), sa.clone(), sa], cb) {
                return Some(Value::ColorField {
                    rgba: out,
                    space: ColorSpace::Linear,
                });
            }
        }

        if (lv_color.is_some() || rv_color.is_some())
            && !(lhs_numeric.is_some() || rhs_numeric.is_some())
        {
            self.diags.push(
                Diag::error(
                    span.clone(),
                    format!(
                        "color arithmetic `{op:?}` only supports color/color or color/scalar operands, found {lv_kind} and {rv_kind}"
                    ),
                )
                .with_help("use scalar factors or color expressions on both sides before applying this operator"),
            );
            return None;
        }

        let scalar_bin = |checker: &mut Checker,
                          lhs: &Value,
                          rhs: &Value,
                          sx: Sx|
         -> Option<Value> {
            let (_, lhs_kind) = Self::as_numeric_scalar(lhs)?;
            let (_, rhs_kind) = Self::as_numeric_scalar(rhs)?;
            let Some(out_kind) = Self::promoted_scalar_kind(op, lhs_kind, rhs_kind) else {
                checker.diags.push(
                        Diag::error(
                            span.clone(),
                            format!(
                                "unsafe cross-kind arithmetic: {} {op:?} {}",
                                Self::scalar_kind_name(lhs_kind),
                                Self::scalar_kind_name(rhs_kind)
                            ),
                        )
                        .with_help(
                            "allowed promotions: matching kinds, scalar with any kind, and multiplicative attenuation between distance/coverage/mask",
                        ),
                    );
                return None;
            };
            Some(Self::scalar_value_from_kind(out_kind, sx))
        };

        let out = match (op, lv, rv) {
            (BinOp::Lt, a, b) if lhs_numeric.is_some() && rhs_numeric.is_some() => {
                let sa = Self::as_numeric_scalar(&a)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let sb = Self::as_numeric_scalar(&b)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                Some(Value::Scalar(Sx::Lt(Box::new(sa), Box::new(sb))))
            }
            (BinOp::Le, a, b) if lhs_numeric.is_some() && rhs_numeric.is_some() => {
                let sa = Self::as_numeric_scalar(&a)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let sb = Self::as_numeric_scalar(&b)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                Some(Value::Scalar(Sx::Le(Box::new(sa), Box::new(sb))))
            }
            (BinOp::Gt, a, b) if lhs_numeric.is_some() && rhs_numeric.is_some() => {
                let sa = Self::as_numeric_scalar(&a)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let sb = Self::as_numeric_scalar(&b)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                Some(Value::Scalar(Sx::Gt(Box::new(sa), Box::new(sb))))
            }
            (BinOp::Ge, a, b) if lhs_numeric.is_some() && rhs_numeric.is_some() => {
                let sa = Self::as_numeric_scalar(&a)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let sb = Self::as_numeric_scalar(&b)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                Some(Value::Scalar(Sx::Ge(Box::new(sa), Box::new(sb))))
            }
            (BinOp::Eq, a, b) if lhs_numeric.is_some() && rhs_numeric.is_some() => {
                let sa = Self::as_numeric_scalar(&a)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let sb = Self::as_numeric_scalar(&b)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                Some(Value::Scalar(Sx::Eq(Box::new(sa), Box::new(sb))))
            }
            (BinOp::Ne, a, b) if lhs_numeric.is_some() && rhs_numeric.is_some() => {
                let sa = Self::as_numeric_scalar(&a)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let sb = Self::as_numeric_scalar(&b)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                Some(Value::Scalar(Sx::Ne(Box::new(sa), Box::new(sb))))
            }
            (BinOp::Add, a, b) if lhs_numeric.is_some() && rhs_numeric.is_some() => {
                let sa = Self::as_numeric_scalar(&a)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let sb = Self::as_numeric_scalar(&b)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                scalar_bin(self, &a, &b, Self::fold_bin_if_literals(BinOp::Add, sa, sb))
            }
            (BinOp::Add, Value::Vec2((ax, ay)), Value::Vec2((bx, by))) => Some(Value::Vec2((
                Self::fold_bin_if_literals(BinOp::Add, ax, bx),
                Self::fold_bin_if_literals(BinOp::Add, ay, by),
            ))),
            (BinOp::Add, Value::Vec2((ax, ay)), b) if rhs_numeric.is_some() => {
                let sb = Self::as_numeric_scalar(&b)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let [sb0, sb1] = self.broadcast2(sb);
                Some(Value::Vec2((
                    Self::fold_bin_if_literals(BinOp::Add, ax, sb0),
                    Self::fold_bin_if_literals(BinOp::Add, ay, sb1),
                )))
            }
            (BinOp::Add, a, Value::Vec2((bx, by))) if lhs_numeric.is_some() => {
                let sa = Self::as_numeric_scalar(&a)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let [sa0, sa1] = self.broadcast2(sa);
                Some(Value::Vec2((
                    Self::fold_bin_if_literals(BinOp::Add, sa0, bx),
                    Self::fold_bin_if_literals(BinOp::Add, sa1, by),
                )))
            }
            (BinOp::Add, Value::Vec3((ax, ay, az)), Value::Vec3((bx, by, bz))) => {
                Some(Value::Vec3((
                    Self::fold_bin_if_literals(BinOp::Add, ax, bx),
                    Self::fold_bin_if_literals(BinOp::Add, ay, by),
                    Self::fold_bin_if_literals(BinOp::Add, az, bz),
                )))
            }
            (BinOp::Add, Value::Vec3((ax, ay, az)), b) if rhs_numeric.is_some() => {
                let sb = Self::as_numeric_scalar(&b)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let [sb0, sb1, sb2] = self.broadcast3(sb);
                Some(Value::Vec3((
                    Self::fold_bin_if_literals(BinOp::Add, ax, sb0),
                    Self::fold_bin_if_literals(BinOp::Add, ay, sb1),
                    Self::fold_bin_if_literals(BinOp::Add, az, sb2),
                )))
            }
            (BinOp::Add, a, Value::Vec3((bx, by, bz))) if lhs_numeric.is_some() => {
                let sa = Self::as_numeric_scalar(&a)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let [sa0, sa1, sa2] = self.broadcast3(sa);
                Some(Value::Vec3((
                    Self::fold_bin_if_literals(BinOp::Add, sa0, bx),
                    Self::fold_bin_if_literals(BinOp::Add, sa1, by),
                    Self::fold_bin_if_literals(BinOp::Add, sa2, bz),
                )))
            }
            (BinOp::Add, Value::Vec4((ax, ay, az, aw)), Value::Vec4((bx, by, bz, bw))) => {
                Some(Value::Vec4((
                    Self::fold_bin_if_literals(BinOp::Add, ax, bx),
                    Self::fold_bin_if_literals(BinOp::Add, ay, by),
                    Self::fold_bin_if_literals(BinOp::Add, az, bz),
                    Self::fold_bin_if_literals(BinOp::Add, aw, bw),
                )))
            }
            (BinOp::Add, Value::Vec4((ax, ay, az, aw)), b) if rhs_numeric.is_some() => {
                let sb = Self::as_numeric_scalar(&b)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let [sb0, sb1, sb2, sb3] = self.broadcast4(sb);
                Some(Value::Vec4((
                    Self::fold_bin_if_literals(BinOp::Add, ax, sb0),
                    Self::fold_bin_if_literals(BinOp::Add, ay, sb1),
                    Self::fold_bin_if_literals(BinOp::Add, az, sb2),
                    Self::fold_bin_if_literals(BinOp::Add, aw, sb3),
                )))
            }
            (BinOp::Add, a, Value::Vec4((bx, by, bz, bw))) if lhs_numeric.is_some() => {
                let sa = Self::as_numeric_scalar(&a)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let [sa0, sa1, sa2, sa3] = self.broadcast4(sa);
                Some(Value::Vec4((
                    Self::fold_bin_if_literals(BinOp::Add, sa0, bx),
                    Self::fold_bin_if_literals(BinOp::Add, sa1, by),
                    Self::fold_bin_if_literals(BinOp::Add, sa2, bz),
                    Self::fold_bin_if_literals(BinOp::Add, sa3, bw),
                )))
            }
            (BinOp::Sub, a, b) if lhs_numeric.is_some() && rhs_numeric.is_some() => {
                let sa = Self::as_numeric_scalar(&a)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let sb = Self::as_numeric_scalar(&b)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                if let Some(factored) =
                    Self::factor_symbolic_pair(SymbolicFactorOp::Sub, sa.clone(), sb.clone())
                {
                    scalar_bin(self, &a, &b, factored)
                } else {
                    scalar_bin(self, &a, &b, Self::fold_bin_if_literals(BinOp::Sub, sa, sb))
                }
            }
            (BinOp::Sub, Value::Vec2((ax, ay)), Value::Vec2((bx, by))) => Some(Value::Vec2((
                Self::fold_bin_if_literals(BinOp::Sub, ax, bx),
                Self::fold_bin_if_literals(BinOp::Sub, ay, by),
            ))),
            (BinOp::Sub, Value::Vec2((ax, ay)), b) if rhs_numeric.is_some() => {
                let sb = Self::as_numeric_scalar(&b)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let [sb0, sb1] = self.broadcast2(sb);
                Some(Value::Vec2((
                    Self::fold_bin_if_literals(BinOp::Sub, ax, sb0),
                    Self::fold_bin_if_literals(BinOp::Sub, ay, sb1),
                )))
            }
            (BinOp::Sub, a, Value::Vec2((bx, by))) if lhs_numeric.is_some() => {
                let sa = Self::as_numeric_scalar(&a)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let [sa0, sa1] = self.broadcast2(sa);
                Some(Value::Vec2((
                    Self::fold_bin_if_literals(BinOp::Sub, sa0, bx),
                    Self::fold_bin_if_literals(BinOp::Sub, sa1, by),
                )))
            }
            (BinOp::Sub, Value::Vec3((ax, ay, az)), Value::Vec3((bx, by, bz))) => {
                Some(Value::Vec3((
                    Self::fold_bin_if_literals(BinOp::Sub, ax, bx),
                    Self::fold_bin_if_literals(BinOp::Sub, ay, by),
                    Self::fold_bin_if_literals(BinOp::Sub, az, bz),
                )))
            }
            (BinOp::Sub, Value::Vec3((ax, ay, az)), b) if rhs_numeric.is_some() => {
                let sb = Self::as_numeric_scalar(&b)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let [sb0, sb1, sb2] = self.broadcast3(sb);
                Some(Value::Vec3((
                    Self::fold_bin_if_literals(BinOp::Sub, ax, sb0),
                    Self::fold_bin_if_literals(BinOp::Sub, ay, sb1),
                    Self::fold_bin_if_literals(BinOp::Sub, az, sb2),
                )))
            }
            (BinOp::Sub, a, Value::Vec3((bx, by, bz))) if lhs_numeric.is_some() => {
                let sa = Self::as_numeric_scalar(&a)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let [sa0, sa1, sa2] = self.broadcast3(sa);
                Some(Value::Vec3((
                    Self::fold_bin_if_literals(BinOp::Sub, sa0, bx),
                    Self::fold_bin_if_literals(BinOp::Sub, sa1, by),
                    Self::fold_bin_if_literals(BinOp::Sub, sa2, bz),
                )))
            }
            (BinOp::Sub, Value::Vec4((ax, ay, az, aw)), Value::Vec4((bx, by, bz, bw))) => {
                Some(Value::Vec4((
                    Self::fold_bin_if_literals(BinOp::Sub, ax, bx),
                    Self::fold_bin_if_literals(BinOp::Sub, ay, by),
                    Self::fold_bin_if_literals(BinOp::Sub, az, bz),
                    Self::fold_bin_if_literals(BinOp::Sub, aw, bw),
                )))
            }
            (BinOp::Sub, Value::Vec4((ax, ay, az, aw)), b) if rhs_numeric.is_some() => {
                let sb = Self::as_numeric_scalar(&b)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let [sb0, sb1, sb2, sb3] = self.broadcast4(sb);
                Some(Value::Vec4((
                    Self::fold_bin_if_literals(BinOp::Sub, ax, sb0),
                    Self::fold_bin_if_literals(BinOp::Sub, ay, sb1),
                    Self::fold_bin_if_literals(BinOp::Sub, az, sb2),
                    Self::fold_bin_if_literals(BinOp::Sub, aw, sb3),
                )))
            }
            (BinOp::Sub, a, Value::Vec4((bx, by, bz, bw))) if lhs_numeric.is_some() => {
                let sa = Self::as_numeric_scalar(&a)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let [sa0, sa1, sa2, sa3] = self.broadcast4(sa);
                Some(Value::Vec4((
                    Self::fold_bin_if_literals(BinOp::Sub, sa0, bx),
                    Self::fold_bin_if_literals(BinOp::Sub, sa1, by),
                    Self::fold_bin_if_literals(BinOp::Sub, sa2, bz),
                    Self::fold_bin_if_literals(BinOp::Sub, sa3, bw),
                )))
            }
            (BinOp::Mul, a, b) if lhs_numeric.is_some() && rhs_numeric.is_some() => {
                let sa = Self::as_numeric_scalar(&a)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let sb = Self::as_numeric_scalar(&b)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                scalar_bin(self, &a, &b, Self::fold_bin_if_literals(BinOp::Mul, sa, sb))
            }
            (BinOp::Mul, Value::Vec2((ax, ay)), Value::Vec2((bx, by))) => Some(Value::Vec2((
                Self::fold_bin_if_literals(BinOp::Mul, ax, bx),
                Self::fold_bin_if_literals(BinOp::Mul, ay, by),
            ))),
            (BinOp::Mul, Value::Vec3((ax, ay, az)), Value::Vec3((bx, by, bz))) => {
                Some(Value::Vec3((
                    Self::fold_bin_if_literals(BinOp::Mul, ax, bx),
                    Self::fold_bin_if_literals(BinOp::Mul, ay, by),
                    Self::fold_bin_if_literals(BinOp::Mul, az, bz),
                )))
            }
            (BinOp::Mul, Value::Vec4((ax, ay, az, aw)), Value::Vec4((bx, by, bz, bw))) => {
                Some(Value::Vec4((
                    Self::fold_bin_if_literals(BinOp::Mul, ax, bx),
                    Self::fold_bin_if_literals(BinOp::Mul, ay, by),
                    Self::fold_bin_if_literals(BinOp::Mul, az, bz),
                    Self::fold_bin_if_literals(BinOp::Mul, aw, bw),
                )))
            }
            (BinOp::Mul, Value::Vec2((ax, ay)), b) if rhs_numeric.is_some() => {
                let sb = Self::as_numeric_scalar(&b)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let [sb0, sb1] = self.broadcast2(sb);
                Some(Value::Vec2((
                    Self::fold_bin_if_literals(BinOp::Mul, ax, sb0),
                    Self::fold_bin_if_literals(BinOp::Mul, ay, sb1),
                )))
            }
            (BinOp::Mul, Value::Vec3((ax, ay, az)), b) if rhs_numeric.is_some() => {
                let sb = Self::as_numeric_scalar(&b)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let [sb0, sb1, sb2] = self.broadcast3(sb);
                Some(Value::Vec3((
                    Self::fold_bin_if_literals(BinOp::Mul, ax, sb0),
                    Self::fold_bin_if_literals(BinOp::Mul, ay, sb1),
                    Self::fold_bin_if_literals(BinOp::Mul, az, sb2),
                )))
            }
            (BinOp::Mul, Value::Vec4((ax, ay, az, aw)), b) if rhs_numeric.is_some() => {
                let sb = Self::as_numeric_scalar(&b)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let [sb0, sb1, sb2, sb3] = self.broadcast4(sb);
                Some(Value::Vec4((
                    Self::fold_bin_if_literals(BinOp::Mul, ax, sb0),
                    Self::fold_bin_if_literals(BinOp::Mul, ay, sb1),
                    Self::fold_bin_if_literals(BinOp::Mul, az, sb2),
                    Self::fold_bin_if_literals(BinOp::Mul, aw, sb3),
                )))
            }
            (BinOp::Mul, a, Value::Vec2((bx, by))) if lhs_numeric.is_some() => {
                let sa = Self::as_numeric_scalar(&a)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let [sa0, sa1] = self.broadcast2(sa);
                Some(Value::Vec2((
                    Self::fold_bin_if_literals(BinOp::Mul, sa0, bx),
                    Self::fold_bin_if_literals(BinOp::Mul, sa1, by),
                )))
            }
            (BinOp::Mul, a, Value::Vec3((bx, by, bz))) if lhs_numeric.is_some() => {
                let sa = Self::as_numeric_scalar(&a)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let [sa0, sa1, sa2] = self.broadcast3(sa);
                Some(Value::Vec3((
                    Self::fold_bin_if_literals(BinOp::Mul, sa0, bx),
                    Self::fold_bin_if_literals(BinOp::Mul, sa1, by),
                    Self::fold_bin_if_literals(BinOp::Mul, sa2, bz),
                )))
            }
            (BinOp::Mul, a, Value::Vec4((bx, by, bz, bw))) if lhs_numeric.is_some() => {
                let sa = Self::as_numeric_scalar(&a)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let [sa0, sa1, sa2, sa3] = self.broadcast4(sa);
                Some(Value::Vec4((
                    Self::fold_bin_if_literals(BinOp::Mul, sa0, bx),
                    Self::fold_bin_if_literals(BinOp::Mul, sa1, by),
                    Self::fold_bin_if_literals(BinOp::Mul, sa2, bz),
                    Self::fold_bin_if_literals(BinOp::Mul, sa3, bw),
                )))
            }
            (BinOp::Mul, Value::Mat2((c0, c1)), Value::Vec2((vx, vy))) => {
                let (c0x, c0y) = c0;
                let (c1x, c1y) = c1;
                Some(Value::Vec2((
                    Self::fold_bin_if_literals(
                        BinOp::Add,
                        Self::fold_bin_if_literals(BinOp::Mul, c0x, vx.clone()),
                        Self::fold_bin_if_literals(BinOp::Mul, c1x, vy.clone()),
                    ),
                    Self::fold_bin_if_literals(
                        BinOp::Add,
                        Self::fold_bin_if_literals(BinOp::Mul, c0y, vx),
                        Self::fold_bin_if_literals(BinOp::Mul, c1y, vy),
                    ),
                )))
            }
            (BinOp::Mul, Value::Vec2((vx, vy)), Value::Mat2((c0, c1))) => {
                let (c0x, c0y) = c0;
                let (c1x, c1y) = c1;
                Some(Value::Vec2((
                    Self::fold_bin_if_literals(
                        BinOp::Add,
                        Self::fold_bin_if_literals(BinOp::Mul, vx.clone(), c0x),
                        Self::fold_bin_if_literals(BinOp::Mul, vy.clone(), c0y),
                    ),
                    Self::fold_bin_if_literals(
                        BinOp::Add,
                        Self::fold_bin_if_literals(BinOp::Mul, vx, c1x),
                        Self::fold_bin_if_literals(BinOp::Mul, vy, c1y),
                    ),
                )))
            }
            (BinOp::Mul, Value::Mat3(mat3), Value::Vec3((vx, vy, vz))) => {
                let (c0, c1, c2) = *mat3;
                let (c0x, c0y, c0z) = c0;
                let (c1x, c1y, c1z) = c1;
                let (c2x, c2y, c2z) = c2;
                let out = |a0: Sx, a1: Sx, a2: Sx| {
                    Self::fold_bin_if_literals(
                        BinOp::Add,
                        Self::fold_bin_if_literals(
                            BinOp::Add,
                            Self::fold_bin_if_literals(BinOp::Mul, a0, vx.clone()),
                            Self::fold_bin_if_literals(BinOp::Mul, a1, vy.clone()),
                        ),
                        Self::fold_bin_if_literals(BinOp::Mul, a2, vz.clone()),
                    )
                };
                Some(Value::Vec3((
                    out(c0x, c1x, c2x),
                    out(c0y, c1y, c2y),
                    out(c0z, c1z, c2z),
                )))
            }
            (BinOp::Mul, Value::Vec3((vx, vy, vz)), Value::Mat3(mat3)) => {
                let (c0, c1, c2) = *mat3;
                let (c0x, c0y, c0z) = c0;
                let (c1x, c1y, c1z) = c1;
                let (c2x, c2y, c2z) = c2;
                let dot = |x: Sx, y: Sx, z: Sx| {
                    Self::fold_bin_if_literals(
                        BinOp::Add,
                        Self::fold_bin_if_literals(
                            BinOp::Add,
                            Self::fold_bin_if_literals(BinOp::Mul, vx.clone(), x),
                            Self::fold_bin_if_literals(BinOp::Mul, vy.clone(), y),
                        ),
                        Self::fold_bin_if_literals(BinOp::Mul, vz.clone(), z),
                    )
                };
                Some(Value::Vec3((
                    dot(c0x, c0y, c0z),
                    dot(c1x, c1y, c1z),
                    dot(c2x, c2y, c2z),
                )))
            }
            (BinOp::Mul, Value::Mat4(matrix), Value::Vec4((vx, vy, vz, vw))) => {
                let (c0, c1, c2, c3) = *matrix;
                let (c0x, c0y, c0z, c0w) = c0;
                let (c1x, c1y, c1z, c1w) = c1;
                let (c2x, c2y, c2z, c2w) = c2;
                let (c3x, c3y, c3z, c3w) = c3;
                let out = |a0: Sx, a1: Sx, a2: Sx, a3: Sx| {
                    Self::fold_bin_if_literals(
                        BinOp::Add,
                        Self::fold_bin_if_literals(
                            BinOp::Add,
                            Self::fold_bin_if_literals(
                                BinOp::Add,
                                Self::fold_bin_if_literals(BinOp::Mul, a0, vx.clone()),
                                Self::fold_bin_if_literals(BinOp::Mul, a1, vy.clone()),
                            ),
                            Self::fold_bin_if_literals(BinOp::Mul, a2, vz.clone()),
                        ),
                        Self::fold_bin_if_literals(BinOp::Mul, a3, vw.clone()),
                    )
                };
                Some(Value::Vec4((
                    out(c0x, c1x, c2x, c3x),
                    out(c0y, c1y, c2y, c3y),
                    out(c0z, c1z, c2z, c3z),
                    out(c0w, c1w, c2w, c3w),
                )))
            }
            (BinOp::Mul, Value::Vec4((vx, vy, vz, vw)), Value::Mat4(matrix)) => {
                let (c0, c1, c2, c3) = *matrix;
                let (c0x, c0y, c0z, c0w) = c0;
                let (c1x, c1y, c1z, c1w) = c1;
                let (c2x, c2y, c2z, c2w) = c2;
                let (c3x, c3y, c3z, c3w) = c3;
                let dot = |x: Sx, y: Sx, z: Sx, w: Sx| {
                    Self::fold_bin_if_literals(
                        BinOp::Add,
                        Self::fold_bin_if_literals(
                            BinOp::Add,
                            Self::fold_bin_if_literals(
                                BinOp::Add,
                                Self::fold_bin_if_literals(BinOp::Mul, vx.clone(), x),
                                Self::fold_bin_if_literals(BinOp::Mul, vy.clone(), y),
                            ),
                            Self::fold_bin_if_literals(BinOp::Mul, vz.clone(), z),
                        ),
                        Self::fold_bin_if_literals(BinOp::Mul, vw.clone(), w),
                    )
                };
                Some(Value::Vec4((
                    dot(c0x, c0y, c0z, c0w),
                    dot(c1x, c1y, c1z, c1w),
                    dot(c2x, c2y, c2z, c2w),
                    dot(c3x, c3y, c3z, c3w),
                )))
            }
            (BinOp::Div, a, b) if lhs_numeric.is_some() && rhs_numeric.is_some() => {
                let sa = Self::as_numeric_scalar(&a)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let sb = Self::as_numeric_scalar(&b)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(1.0));
                if let Some(factored) =
                    Self::factor_symbolic_pair(SymbolicFactorOp::Div, sa.clone(), sb.clone())
                {
                    scalar_bin(self, &a, &b, factored)
                } else {
                    scalar_bin(self, &a, &b, Self::fold_bin_if_literals(BinOp::Div, sa, sb))
                }
            }
            (BinOp::Div, Value::Vec2((ax, ay)), Value::Vec2((bx, by))) => Some(Value::Vec2((
                Self::fold_bin_if_literals(BinOp::Div, ax, bx),
                Self::fold_bin_if_literals(BinOp::Div, ay, by),
            ))),
            (BinOp::Div, Value::Vec3((ax, ay, az)), Value::Vec3((bx, by, bz))) => {
                Some(Value::Vec3((
                    Self::fold_bin_if_literals(BinOp::Div, ax, bx),
                    Self::fold_bin_if_literals(BinOp::Div, ay, by),
                    Self::fold_bin_if_literals(BinOp::Div, az, bz),
                )))
            }
            (BinOp::Div, Value::Vec4((ax, ay, az, aw)), Value::Vec4((bx, by, bz, bw))) => {
                Some(Value::Vec4((
                    Self::fold_bin_if_literals(BinOp::Div, ax, bx),
                    Self::fold_bin_if_literals(BinOp::Div, ay, by),
                    Self::fold_bin_if_literals(BinOp::Div, az, bz),
                    Self::fold_bin_if_literals(BinOp::Div, aw, bw),
                )))
            }
            (BinOp::Div, Value::Vec2((ax, ay)), b) if rhs_numeric.is_some() => {
                let sb = Self::as_numeric_scalar(&b)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(1.0));
                let [sb0, sb1] = self.broadcast2(sb);
                Some(Value::Vec2((
                    Self::fold_bin_if_literals(BinOp::Div, ax, sb0),
                    Self::fold_bin_if_literals(BinOp::Div, ay, sb1),
                )))
            }
            (BinOp::Div, Value::Vec3((ax, ay, az)), b) if rhs_numeric.is_some() => {
                let sb = Self::as_numeric_scalar(&b)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(1.0));
                let [sb0, sb1, sb2] = self.broadcast3(sb);
                Some(Value::Vec3((
                    Self::fold_bin_if_literals(BinOp::Div, ax, sb0),
                    Self::fold_bin_if_literals(BinOp::Div, ay, sb1),
                    Self::fold_bin_if_literals(BinOp::Div, az, sb2),
                )))
            }
            (BinOp::Div, Value::Vec4((ax, ay, az, aw)), b) if rhs_numeric.is_some() => {
                let sb = Self::as_numeric_scalar(&b)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(1.0));
                let [sb0, sb1, sb2, sb3] = self.broadcast4(sb);
                Some(Value::Vec4((
                    Self::fold_bin_if_literals(BinOp::Div, ax, sb0),
                    Self::fold_bin_if_literals(BinOp::Div, ay, sb1),
                    Self::fold_bin_if_literals(BinOp::Div, az, sb2),
                    Self::fold_bin_if_literals(BinOp::Div, aw, sb3),
                )))
            }
            (BinOp::Div, a, Value::Vec2((bx, by))) if lhs_numeric.is_some() => {
                let sa = Self::as_numeric_scalar(&a)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let [sa0, sa1] = self.broadcast2(sa);
                Some(Value::Vec2((
                    Self::fold_bin_if_literals(BinOp::Div, sa0, bx),
                    Self::fold_bin_if_literals(BinOp::Div, sa1, by),
                )))
            }
            (BinOp::Div, a, Value::Vec3((bx, by, bz))) if lhs_numeric.is_some() => {
                let sa = Self::as_numeric_scalar(&a)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let [sa0, sa1, sa2] = self.broadcast3(sa);
                Some(Value::Vec3((
                    Self::fold_bin_if_literals(BinOp::Div, sa0, bx),
                    Self::fold_bin_if_literals(BinOp::Div, sa1, by),
                    Self::fold_bin_if_literals(BinOp::Div, sa2, bz),
                )))
            }
            (BinOp::Div, a, Value::Vec4((bx, by, bz, bw))) if lhs_numeric.is_some() => {
                let sa = Self::as_numeric_scalar(&a)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let [sa0, sa1, sa2, sa3] = self.broadcast4(sa);
                Some(Value::Vec4((
                    Self::fold_bin_if_literals(BinOp::Div, sa0, bx),
                    Self::fold_bin_if_literals(BinOp::Div, sa1, by),
                    Self::fold_bin_if_literals(BinOp::Div, sa2, bz),
                    Self::fold_bin_if_literals(BinOp::Div, sa3, bw),
                )))
            }
            (BinOp::Mod, a, b) if lhs_numeric.is_some() && rhs_numeric.is_some() => {
                let sa = Self::as_numeric_scalar(&a)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let sb = Self::as_numeric_scalar(&b)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(1.0));
                scalar_bin(self, &a, &b, Self::sx_mod_floor(sa, sb))
            }
            (BinOp::Mod, Value::Vec2((ax, ay)), Value::Vec2((bx, by))) => Some(Value::Vec2((
                Self::sx_mod_floor(ax, bx),
                Self::sx_mod_floor(ay, by),
            ))),
            (BinOp::Mod, Value::Vec3((ax, ay, az)), Value::Vec3((bx, by, bz))) => {
                Some(Value::Vec3((
                    Self::sx_mod_floor(ax, bx),
                    Self::sx_mod_floor(ay, by),
                    Self::sx_mod_floor(az, bz),
                )))
            }
            (BinOp::Mod, Value::Vec4((ax, ay, az, aw)), Value::Vec4((bx, by, bz, bw))) => {
                Some(Value::Vec4((
                    Self::sx_mod_floor(ax, bx),
                    Self::sx_mod_floor(ay, by),
                    Self::sx_mod_floor(az, bz),
                    Self::sx_mod_floor(aw, bw),
                )))
            }
            (BinOp::Mod, Value::Vec2((ax, ay)), b) if rhs_numeric.is_some() => {
                let sb = Self::as_numeric_scalar(&b)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(1.0));
                let [sb0, sb1] = self.broadcast2(sb);
                Some(Value::Vec2((
                    Self::sx_mod_floor(ax, sb0),
                    Self::sx_mod_floor(ay, sb1),
                )))
            }
            (BinOp::Mod, Value::Vec3((ax, ay, az)), b) if rhs_numeric.is_some() => {
                let sb = Self::as_numeric_scalar(&b)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(1.0));
                let [sb0, sb1, sb2] = self.broadcast3(sb);
                Some(Value::Vec3((
                    Self::sx_mod_floor(ax, sb0),
                    Self::sx_mod_floor(ay, sb1),
                    Self::sx_mod_floor(az, sb2),
                )))
            }
            (BinOp::Mod, Value::Vec4((ax, ay, az, aw)), b) if rhs_numeric.is_some() => {
                let sb = Self::as_numeric_scalar(&b)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(1.0));
                let [sb0, sb1, sb2, sb3] = self.broadcast4(sb);
                Some(Value::Vec4((
                    Self::sx_mod_floor(ax, sb0),
                    Self::sx_mod_floor(ay, sb1),
                    Self::sx_mod_floor(az, sb2),
                    Self::sx_mod_floor(aw, sb3),
                )))
            }
            (BinOp::Mod, a, Value::Vec2((bx, by))) if lhs_numeric.is_some() => {
                let sa = Self::as_numeric_scalar(&a)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let [sa0, sa1] = self.broadcast2(sa);
                Some(Value::Vec2((
                    Self::sx_mod_floor(sa0, bx),
                    Self::sx_mod_floor(sa1, by),
                )))
            }
            (BinOp::Mod, a, Value::Vec3((bx, by, bz))) if lhs_numeric.is_some() => {
                let sa = Self::as_numeric_scalar(&a)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let [sa0, sa1, sa2] = self.broadcast3(sa);
                Some(Value::Vec3((
                    Self::sx_mod_floor(sa0, bx),
                    Self::sx_mod_floor(sa1, by),
                    Self::sx_mod_floor(sa2, bz),
                )))
            }
            (BinOp::Mod, a, Value::Vec4((bx, by, bz, bw))) if lhs_numeric.is_some() => {
                let sa = Self::as_numeric_scalar(&a)
                    .map(|(sx, _)| sx)
                    .unwrap_or(Sx::Lit(0.0));
                let [sa0, sa1, sa2, sa3] = self.broadcast4(sa);
                Some(Value::Vec4((
                    Self::sx_mod_floor(sa0, bx),
                    Self::sx_mod_floor(sa1, by),
                    Self::sx_mod_floor(sa2, bz),
                    Self::sx_mod_floor(sa3, bw),
                )))
            }
            (BinOp::Union, a, b)
                if Self::as_numeric_scalar(&a).is_some()
                    && Self::as_numeric_scalar(&b).is_some() =>
            {
                self.diags.push(
                    Diag::error(
                        span.clone(),
                        "operator `|` on scalars is not supported; use bit helpers",
                    )
                    .with_help("use `bit_or(a, b)` for bitwise OR on integer-like scalar values"),
                );
                None
            }
            (BinOp::Union, Value::Shape(a), Value::Shape(b)) => {
                let out = self.hir.shape(Shape::Union(a, b));
                self.note_shape_exactness(out, "union");
                Some(Value::Shape(out))
            }
            (BinOp::Intersect, Value::Shape(a), Value::Shape(b)) => {
                let out = self.hir.shape(Shape::Intersect(a, b));
                self.note_shape_exactness(out, "intersect");
                Some(Value::Shape(out))
            }
            (BinOp::Sub, Value::Shape(a), Value::Shape(b)) => {
                let out = self.hir.shape(Shape::Subtract(a, b));
                self.note_shape_exactness(out, "subtract");
                Some(Value::Shape(out))
            }
            (BinOp::Sub, Value::Shape(seed), Value::Array(items)) => {
                let mut acc = seed;
                let depth = items.len();
                for item in items {
                    let Value::Shape(cut) = item else {
                        self.diags.push(
                            Diag::error(
                                span.clone(),
                                "shape subtraction with an array requires shape elements",
                            )
                            .with_help("use `shape - [shape1, shape2, ...]`"),
                        );
                        return None;
                    };
                    acc = self.hir.shape(Shape::Subtract(acc, cut));
                    self.note_shape_exactness(acc, "subtract");
                }
                self.hir.notes.push(format!(
                    "collection-subtract: lowered to ordered subtract chain, depth={depth}, collapsed=false"
                ));
                Some(Value::Shape(acc))
            }
            (BinOp::Intersect, a, b)
                if Self::as_numeric_scalar(&a).is_some()
                    && Self::as_numeric_scalar(&b).is_some() =>
            {
                self.diags.push(
                    Diag::error(
                        span.clone(),
                        "operator `&` on scalars is not supported; use bit helpers",
                    )
                    .with_help("use `bit_and(a, b)` for bitwise AND on integer-like scalar values"),
                );
                None
            }
            (BinOp::Shl, _, _) => {
                self.diags.push(
                    Diag::error(
                        span.clone(),
                        "operator `<<` is not supported in expressions",
                    )
                    .with_help("use `bit_shl(x, shift)` instead"),
                );
                None
            }
            (BinOp::Shr, _, _) => {
                self.diags.push(
                    Diag::error(
                        span.clone(),
                        "operator `>>` is not supported in expressions",
                    )
                    .with_help("use `bit_shr(x, shift)` instead"),
                );
                None
            }
            (BinOp::BitXor, _, _) => {
                self.diags.push(
                    Diag::error(span.clone(), "operator `^` is not supported in expressions")
                        .with_help("use `bit_xor(a, b)` instead"),
                );
                None
            }
            (op, a, b) => {
                self.diags.push(
                    Diag::error(
                        span.clone(),
                        format!("operator `{op:?}` is not defined for {} and {}", a.kind(), b.kind()),
                    )
                    .with_help("scalars support + - * / % and comparisons (< <= > >= == !=); shapes support | & - (union, intersect, subtract)"),
                );
                None
            }
        };
        self.record_check_profile_timing("eval_binary", t0.elapsed());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::Checker;
    use crate::ast::BinOp;
    use crate::check::expr::SymbolicFactorOp;
    use crate::hir::{Sx, SxVec};

    #[test]
    fn static_folding_and_rewrite_guards_share_vector_and_binding_semantics() {
        let expression = Sx::Let {
            name: "n".into(),
            value: std::rc::Rc::new(Sx::Lit(4.0)),
            body: Box::new(Sx::Length(SxVec::V2(Box::new((
                Sx::Lit(3.0),
                Sx::Var("n".into()),
            ))))),
        };
        assert_eq!(Checker::try_eval_static_scalar(&expression), Some(5.0));
        assert_eq!(
            expression.try_eval_with_vars(&std::collections::HashMap::new()),
            Some(5.0)
        );
        let invalid = Sx::Clamp(
            Box::new(Sx::Lit(0.5)),
            Box::new(Sx::Lit(2.0)),
            Box::new(Sx::Lit(1.0)),
        );
        assert_eq!(Checker::try_eval_static_scalar(&invalid), None);
        assert_eq!(
            invalid.try_eval_with_vars(&std::collections::HashMap::new()),
            None
        );
    }

    #[test]
    fn fold_bin_if_literals_folds_literal_subgraphs() {
        let folded = Checker::fold_bin_if_literals(BinOp::Add, Sx::Lit(1.25), Sx::Lit(2.75));
        assert_eq!(folded, Sx::Lit(4.0));
    }

    #[test]
    fn fold_bin_if_literals_eliminates_safe_identities() {
        assert_eq!(
            Checker::fold_bin_if_literals(BinOp::Sub, Sx::CoordX, Sx::Lit(0.0)),
            Sx::CoordX
        );
        assert_eq!(
            Checker::fold_bin_if_literals(BinOp::Mul, Sx::CoordY, Sx::Lit(1.0)),
            Sx::CoordY
        );
        assert_eq!(
            Checker::fold_bin_if_literals(BinOp::Div, Sx::CoordY, Sx::Lit(1.0)),
            Sx::CoordY
        );
    }

    #[test]
    fn fold_bin_if_literals_normalizes_commutative_operands() {
        match Checker::fold_bin_if_literals(BinOp::Add, Sx::CoordX, Sx::Lit(0.0)) {
            Sx::Add(left, right) => {
                assert!(matches!(*left, Sx::Lit(_)));
                assert!(matches!(*right, Sx::CoordX));
            }
            other => panic!("expected Add, found {other:?}"),
        }

        match Checker::fold_bin_if_literals(BinOp::Mul, Sx::CoordY, Sx::Lit(2.0)) {
            Sx::Mul(left, right) => {
                assert!(matches!(*left, Sx::Lit(_)));
                assert!(matches!(*right, Sx::CoordY));
            }
            other => panic!("expected Mul, found {other:?}"),
        }
    }

    #[test]
    fn factor_symbolic_pair_factors_shared_positive_symbol_through_max() {
        let length = Sx::Length(SxVec::V2(Box::new((Sx::CoordX, Sx::CoordY))));
        let lhs = Sx::Mul(Box::new(Sx::Lit(1.5)), Box::new(length.clone()));
        let rhs = Sx::Mul(Box::new(Sx::Lit(2.0)), Box::new(length.clone()));

        let factored = Checker::factor_symbolic_pair(SymbolicFactorOp::Max, lhs, rhs)
            .expect("expected shared-factor max rewrite");

        assert_eq!(factored, Sx::Mul(Box::new(Sx::Lit(2.0)), Box::new(length)));
    }

    #[test]
    fn factor_symbolic_pair_collapses_shared_nonnegative_symbol_through_sub() {
        let shape = Sx::Abs(Box::new(Sx::CoordX));
        let lhs = Sx::Mul(Box::new(Sx::Lit(1.5)), Box::new(shape.clone()));
        let rhs = Sx::Mul(Box::new(Sx::Lit(0.5)), Box::new(shape.clone()));

        let factored = Checker::factor_symbolic_pair(SymbolicFactorOp::Sub, lhs, rhs)
            .expect("expected shared-factor subtraction rewrite");

        assert_eq!(factored, shape);
    }

    #[test]
    fn factor_symbolic_pair_cancels_shared_positive_symbol_through_div() {
        let symbol = Sx::Max(
            Box::new(Sx::Length(SxVec::V2(Box::new((Sx::CoordX, Sx::CoordY))))),
            Box::new(Sx::Lit(1.0)),
        );
        let lhs = Sx::Mul(Box::new(Sx::Lit(3.0)), Box::new(symbol.clone()));
        let rhs = Sx::Mul(Box::new(Sx::Lit(2.0)), Box::new(symbol));

        let factored = Checker::factor_symbolic_pair(SymbolicFactorOp::Div, lhs, rhs)
            .expect("expected shared-factor division rewrite");

        assert_eq!(factored, Sx::Lit(1.5));
    }
}
