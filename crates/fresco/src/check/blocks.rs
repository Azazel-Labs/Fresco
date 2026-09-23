use super::*;
use crate::hir::ScatterLifecycleParams;
use crate::hir::ScatterLoweringStrategy;
use crate::hir::VerticalAxis;
use tracing::{debug, info};

pub(super) const SPACE_TRANSFORM_NAMES: &[&str] = &[
    "rotate",
    "rotate_x",
    "rotate_y",
    "rotate_z",
    "translate",
    "translate3",
    "scale",
    "repeat",
    "cells",
    "repeat_x",
    "repeat_y",
    "repeat_radial",
    "aspect",
    "centered",
    "orientation",
    "polar",
    "warp",
    "perspective",
];

#[derive(Clone, Copy, Debug, Default)]
struct ScatterFootprint {
    shape_radius: f32,
    effect_extent: f32,
}

#[derive(Clone, Copy, Debug)]
struct ScalarBounds {
    min: f32,
    max: f32,
}

#[derive(Clone, Debug)]
struct RepeatCellBinding {
    every: Option<[f32; 2]>,
    name: String,
    scope_id: u32,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ResolvedSpaceChain {
    pub(super) xforms: Vec<Xform>,
    repeat_bindings: Vec<RepeatCellBinding>,
}

type StaticRegionBounds = ((f32, f32), (f32, f32));
type DynamicRegionBounds = ((Sx, Sx), (Sx, Sx), Option<StaticRegionBounds>);

impl ScalarBounds {
    fn from_values(values: &[f32]) -> Self {
        let mut min = f32::INFINITY;
        let mut max = f32::NEG_INFINITY;
        for value in values {
            min = min.min(*value);
            max = max.max(*value);
        }
        Self { min, max }
    }

    fn midpoint(self) -> f32 {
        f32::midpoint(self.min, self.max)
    }

    fn abs_max(self) -> f32 {
        self.min.abs().max(self.max.abs())
    }
}

impl ScatterFootprint {
    fn score(self) -> f32 {
        (self.shape_radius + self.effect_extent).max(0.01)
    }
}

impl Checker {
    /// Replace a nested record field or vector swizzle, preserving sibling values.
    pub(super) fn assign_field_path(
        &self,
        base: Value,
        path: &str,
        value: Value,
    ) -> Result<Value, String> {
        match base {
            Value::Struct {
                ty_name,
                mut fields,
            } => {
                let (field, rest) = path
                    .split_once('.')
                    .map_or((path, None), |(a, b)| (a, Some(b)));
                let definition = self
                    .struct_defs
                    .get(&ty_name)
                    .and_then(|d| d.fields.get(field))
                    .ok_or_else(|| format!("`{ty_name}` has no field `{field}`"))?;
                let updated = if let Some(rest) = rest {
                    let old = fields
                        .get(field)
                        .cloned()
                        .ok_or_else(|| format!("missing field `{field}`"))?;
                    self.assign_field_path(old, rest, value)?
                } else {
                    if !local_decl_type_matches(
                        &value,
                        &definition.ty_name,
                        &self.enum_defs,
                        &self.struct_defs,
                        &[],
                    ) {
                        return Err(format!(
                            "cannot assign {} to field `{field}` of type {}",
                            value.kind(),
                            definition.ty_name
                        ));
                    }
                    value
                };
                fields.insert(field.to_owned(), updated);
                Ok(Value::Struct { ty_name, fields })
            }
            other => apply_swizzle_assignment(other, path, value),
        }
    }

    fn stmt_kind(stmt: &Stmt) -> &'static str {
        match stmt {
            Stmt::Param { .. } => "param",
            Stmt::TextureBinding { .. } => "uniform texture binding",
            Stmt::Let { .. } => "let",
            Stmt::Const { .. } => "const",
            Stmt::Assign { .. } | Stmt::Store { .. } => "assignment",
            Stmt::For { .. } => "for-loop",
            Stmt::If { .. } => "if",
            Stmt::Match { .. } => "match",
            Stmt::LetScatter { .. } => "scatter",
            Stmt::SpaceDecl { .. } => "space declaration",
            Stmt::StyleDecl { .. } => "style declaration",
            Stmt::CanvasSpace { .. } => "canvas_space declaration",
            Stmt::Compose { .. } => "compose",
            Stmt::ComposePiped { .. } => "compose with pipes",
            Stmt::SurfaceVertex { .. } => "surface vertex",
            Stmt::Return { .. } | Stmt::ReturnVoid { .. } => "return",
            Stmt::Break { .. } => "break",
            Stmt::InSpace { .. } => "in-space block",
            Stmt::InContext { .. } => "in-context block",
            Stmt::Block { .. } => "grouped block",
            Stmt::Seq { .. } => "statement sequence",
            Stmt::LocalFnDecl(_) => "local fn declaration",
            Stmt::Expr(_) => "expression",
        }
    }

    fn compose_entry_kind(entry: &ComposeEntry) -> &'static str {
        match entry {
            ComposeEntry::Expr { .. } => "compose expr",
            ComposeEntry::Block { .. } => "compose block",
            ComposeEntry::If { .. } => "compose if",
            ComposeEntry::InSpace { .. } => "compose in-space",
            ComposeEntry::For { .. } => "compose for-loop",
        }
    }

    fn is_circle_like_shape_call(name: &str) -> bool {
        matches!(name, "circle" | "capsule")
    }

    fn expr_contains_rand(expr: &SExpr) -> bool {
        match &expr.node {
            Expr::Call { name, args, .. } => {
                name == "rand" || args.iter().any(|arg| Self::expr_contains_rand(&arg.value))
            }
            Expr::Pipe { recv, args, .. } => {
                Self::expr_contains_rand(recv)
                    || args.iter().any(|arg| Self::expr_contains_rand(&arg.value))
            }
            Expr::Vec2(a, b) | Expr::Range(a, b) | Expr::Binary(_, a, b) => {
                Self::expr_contains_rand(a) || Self::expr_contains_rand(b)
            }
            Expr::Vec3(a, b, c) => {
                Self::expr_contains_rand(a)
                    || Self::expr_contains_rand(b)
                    || Self::expr_contains_rand(c)
            }
            Expr::Vec4(a, b, c, d) => {
                Self::expr_contains_rand(a)
                    || Self::expr_contains_rand(b)
                    || Self::expr_contains_rand(c)
                    || Self::expr_contains_rand(d)
            }
            Expr::Array(items) => items.iter().any(Self::expr_contains_rand),
            Expr::ArrayComp { iterable, body, .. } => {
                Self::expr_contains_rand(iterable) || Self::expr_contains_rand(body)
            }
            Expr::Lambda { body, .. } => match body {
                LambdaBody::Expr(expr) | LambdaBody::BlockReturn(expr) => {
                    Self::expr_contains_rand(expr)
                }
            },
            Expr::Unary(_, v) | Expr::Member(v, _) | Expr::Field(v) | Expr::Layer(v) => {
                Self::expr_contains_rand(v)
            }
            Expr::Through { layer, .. } => Self::expr_contains_rand(layer),
            Expr::FieldAt { inner, coord } => {
                Self::expr_contains_rand(inner) || Self::expr_contains_rand(coord)
            }
            Expr::Index { array, index } => {
                Self::expr_contains_rand(array) || Self::expr_contains_rand(index)
            }
            Expr::Num(_, _)
            | Expr::Color(_)
            | Expr::Str(_)
            | Expr::Var(_)
            | Expr::PathFuture { .. } => false,
        }
    }

    fn lifecycle_scalar_expr(&mut self, expr: &SExpr, what: &str) -> Option<(Sx, ScalarBounds)> {
        let sx = self.as_scalar(expr)?;
        if !Self::expr_contains_rand(expr) {
            let value = self.eval_const_scalar_sx(&sx, &expr.span, what)?;
            return Some((
                sx,
                ScalarBounds {
                    min: value,
                    max: value,
                },
            ));
        }

        let Some(bounds) = Self::scalar_bounds_quiet(expr) else {
            self.diags.push(
                Diag::error(
                    expr.span.clone(),
                    format!("{what} with `rand(...)` must use literal arithmetic over numeric ranges"),
                )
                .with_help("use literals, `rand(lo .. hi)`, and basic arithmetic in scatter lifecycle values"),
            );
            return None;
        };

        Some((sx, bounds))
    }

    const MAX_SCATTER_INSTANCES: usize = 4096;
    const MAX_SCATTER_BINS_AXIS: usize = 16;
    const MAX_SCATTER_CANDIDATES_PER_BIN: usize = 8;

    fn next_repeat_cell_scope_id(&mut self) -> u32 {
        let scope_id = self.next_repeat_cell_scope_id;
        self.next_repeat_cell_scope_id += 1;
        scope_id
    }

    fn repeat_cell_value(scope_id: u32, every: Option<[f32; 2]>) -> Value {
        Value::RepeatCell(Box::new(RepeatCellValue {
            scope_id,
            every,
            id: (Sx::RepeatCellIdX(scope_id), Sx::RepeatCellIdY(scope_id)),
            center: (
                Sx::RepeatCellCenterX(scope_id),
                Sx::RepeatCellCenterY(scope_id),
            ),
            uv: (Sx::RepeatCellUvX(scope_id), Sx::RepeatCellUvY(scope_id)),
            rand: Sx::RepeatCellRand(scope_id),
        }))
    }

    fn bind_repeat_cells(&mut self, bindings: &[RepeatCellBinding], span: &Span) -> bool {
        let mut ok = true;
        let Some(scope) = self.scopes.last_mut() else {
            return false;
        };
        for binding in bindings {
            if scope.contains_key(&binding.name) {
                self.diags.push(
                    Diag::error(
                        span.clone(),
                        format!(
                            "duplicate repeat cell binding `{}` in this scope",
                            binding.name
                        ),
                    )
                    .with_help("rename one of the repeat bindings so nested scopes stay distinct"),
                );
                ok = false;
                continue;
            }
            scope.insert(
                binding.name.clone(),
                Self::repeat_cell_value(binding.scope_id, binding.every),
            );
        }
        ok
    }

    fn apply_layer_pipes(
        &mut self,
        mut layer: LayerId,
        pipes: &[ScatterPipeCall],
        span: &Span,
    ) -> Option<LayerId> {
        for pipe in pipes {
            let recv = Value::Layer(layer);
            match self.builtin(&pipe.name, &pipe.name_span, Some(recv), &pipe.args, span) {
                Some(Value::Layer(next)) => layer = next,
                Some(v) => {
                    self.diags.push(
                        Diag::error(
                            span.clone(),
                            format!(
                                "`{}` must return a layer when piping layer values, found {}",
                                pipe.name,
                                v.kind()
                            ),
                        )
                        .with_help(
                            "use layer effects in this position, for example `opacity(...)`",
                        ),
                    );
                    return None;
                }
                None => return None,
            }
        }
        Some(layer)
    }

    pub(super) fn eval_const_scalar_sx(&mut self, sx: &Sx, span: &Span, what: &str) -> Option<f32> {
        match crate::signal_eval::evaluate(sx, &HashMap::new()) {
            Ok(value) => Some(value),
            Err(error) => {
                self.diags.push(
                    Diag::error(span.clone(), format!("{what}{}", error.message))
                        .with_help(error.help),
                );
                None
            }
        }
    }

    fn eval_const_scalar_expr(&mut self, expr: &SExpr, what: &str) -> Option<f32> {
        let sx = self.as_scalar(expr)?;
        self.eval_const_scalar_sx(&sx, &expr.span, what)
    }

    pub(super) fn eval_compile_time_const_value(
        &mut self,
        value: &Value,
        span: &Span,
        what: &str,
    ) -> Option<Value> {
        match value {
            Value::Scalar(Sx::Typed(value)) => match value.evaluate(&HashMap::new()) {
                Ok(literal) => Some(Value::Scalar(crate::typed_scalar::Scalar::from_literal(
                    literal,
                ))),
                Err(message) => {
                    self.diags
                        .push(Diag::error(span.clone(), format!("{what}: {message}")));
                    None
                }
            },
            Value::Scalar(sx) => Some(Value::Scalar(Sx::Lit(
                self.eval_const_scalar_sx(sx, span, what)?,
            ))),
            Value::Distance(sx) => Some(Value::Distance(Sx::Lit(
                self.eval_const_scalar_sx(sx, span, what)?,
            ))),
            Value::Coverage(sx) => Some(Value::Coverage(Sx::Lit(
                self.eval_const_scalar_sx(sx, span, what)?,
            ))),
            Value::Mask(sx) => Some(Value::Mask(Sx::Lit(
                self.eval_const_scalar_sx(sx, span, what)?,
            ))),
            Value::Vec2((x, y)) => Some(Value::Vec2((
                Sx::Lit(self.eval_const_scalar_sx(x, span, what)?),
                Sx::Lit(self.eval_const_scalar_sx(y, span, what)?),
            ))),
            Value::Vec3((x, y, z)) => Some(Value::Vec3((
                Sx::Lit(self.eval_const_scalar_sx(x, span, what)?),
                Sx::Lit(self.eval_const_scalar_sx(y, span, what)?),
                Sx::Lit(self.eval_const_scalar_sx(z, span, what)?),
            ))),
            Value::Vec4((x, y, z, w)) => Some(Value::Vec4((
                Sx::Lit(self.eval_const_scalar_sx(x, span, what)?),
                Sx::Lit(self.eval_const_scalar_sx(y, span, what)?),
                Sx::Lit(self.eval_const_scalar_sx(z, span, what)?),
                Sx::Lit(self.eval_const_scalar_sx(w, span, what)?),
            ))),
            Value::Mat2((c0, c1)) => Some(Value::Mat2((
                (
                    Sx::Lit(self.eval_const_scalar_sx(&c0.0, span, what)?),
                    Sx::Lit(self.eval_const_scalar_sx(&c0.1, span, what)?),
                ),
                (
                    Sx::Lit(self.eval_const_scalar_sx(&c1.0, span, what)?),
                    Sx::Lit(self.eval_const_scalar_sx(&c1.1, span, what)?),
                ),
            ))),
            Value::Mat3(m) => Some(Value::Mat3(Box::new((
                (
                    Sx::Lit(self.eval_const_scalar_sx(&m.0.0, span, what)?),
                    Sx::Lit(self.eval_const_scalar_sx(&m.0.1, span, what)?),
                    Sx::Lit(self.eval_const_scalar_sx(&m.0.2, span, what)?),
                ),
                (
                    Sx::Lit(self.eval_const_scalar_sx(&m.1.0, span, what)?),
                    Sx::Lit(self.eval_const_scalar_sx(&m.1.1, span, what)?),
                    Sx::Lit(self.eval_const_scalar_sx(&m.1.2, span, what)?),
                ),
                (
                    Sx::Lit(self.eval_const_scalar_sx(&m.2.0, span, what)?),
                    Sx::Lit(self.eval_const_scalar_sx(&m.2.1, span, what)?),
                    Sx::Lit(self.eval_const_scalar_sx(&m.2.2, span, what)?),
                ),
            )))),
            Value::Mat4(m) => Some(Value::Mat4(Box::new((
                (
                    Sx::Lit(self.eval_const_scalar_sx(&m.0.0, span, what)?),
                    Sx::Lit(self.eval_const_scalar_sx(&m.0.1, span, what)?),
                    Sx::Lit(self.eval_const_scalar_sx(&m.0.2, span, what)?),
                    Sx::Lit(self.eval_const_scalar_sx(&m.0.3, span, what)?),
                ),
                (
                    Sx::Lit(self.eval_const_scalar_sx(&m.1.0, span, what)?),
                    Sx::Lit(self.eval_const_scalar_sx(&m.1.1, span, what)?),
                    Sx::Lit(self.eval_const_scalar_sx(&m.1.2, span, what)?),
                    Sx::Lit(self.eval_const_scalar_sx(&m.1.3, span, what)?),
                ),
                (
                    Sx::Lit(self.eval_const_scalar_sx(&m.2.0, span, what)?),
                    Sx::Lit(self.eval_const_scalar_sx(&m.2.1, span, what)?),
                    Sx::Lit(self.eval_const_scalar_sx(&m.2.2, span, what)?),
                    Sx::Lit(self.eval_const_scalar_sx(&m.2.3, span, what)?),
                ),
                (
                    Sx::Lit(self.eval_const_scalar_sx(&m.3.0, span, what)?),
                    Sx::Lit(self.eval_const_scalar_sx(&m.3.1, span, what)?),
                    Sx::Lit(self.eval_const_scalar_sx(&m.3.2, span, what)?),
                    Sx::Lit(self.eval_const_scalar_sx(&m.3.3, span, what)?),
                ),
            )))),
            Value::Array(items) => {
                let mut folded = Vec::with_capacity(items.len());
                for item in items {
                    folded.push(self.eval_compile_time_const_value(item, span, what)?);
                }
                Some(Value::Array(folded))
            }
            Value::Color { rgba, space } => Some(Value::Color {
                rgba: *rgba,
                space: *space,
            }),
            Value::ColorField { rgba, space } => Some(Value::Color {
                rgba: [
                    self.eval_const_scalar_sx(&rgba[0], span, what)?,
                    self.eval_const_scalar_sx(&rgba[1], span, what)?,
                    self.eval_const_scalar_sx(&rgba[2], span, what)?,
                    self.eval_const_scalar_sx(&rgba[3], span, what)?,
                ],
                space: *space,
            }),
            Value::Struct { ty_name, fields } => {
                let mut folded_fields = HashMap::new();
                for (field_name, field_value) in fields {
                    let folded = self.eval_compile_time_const_value(field_value, span, what)?;
                    folded_fields.insert(field_name.clone(), folded);
                }
                Some(Value::Struct {
                    ty_name: ty_name.clone(),
                    fields: folded_fields,
                })
            }
            _ => {
                self.diags.push(
                    Diag::error(
                        span.clone(),
                        format!("{what} must be compile-time constant"),
                    )
                    .with_help(
                        "use literal arithmetic, const values, and compile-time pure expressions",
                    ),
                );
                None
            }
        }
    }

    fn scatter_region_bounds(&mut self, region: &SExpr) -> Option<DynamicRegionBounds> {
        match &region.node {
            Expr::Var(name) if name == "screen" => Some((
                (Sx::Lit(0.0), Sx::Lit(0.0)),
                (Sx::Lit(1.0), Sx::Lit(1.0)),
                Some(((0.0, 0.0), (1.0, 1.0))),
            )),
            Expr::Call { name, args, .. } if name == "region" => {
                if args.len() != 1 {
                    self.diags.push(
                        Diag::error(
                            region.span.clone(),
                            "`region(...)` expects exactly one range argument",
                        )
                        .with_help("write `region((x0, y0) .. (x1, y1))`"),
                    );
                    return None;
                }
                let bounds = &args[0].value;
                match &bounds.node {
                    Expr::Range(min_expr, max_expr) => {
                        let min = self.as_vec2(min_expr)?;
                        let max = self.as_vec2(max_expr)?;

                        let min_const = Self::const_vec2_expr_quiet(min_expr);
                        let max_const = Self::const_vec2_expr_quiet(max_expr);
                        let const_bounds = match (min_const, max_const) {
                            (Some(min), Some(max)) => Some((min, max)),
                            _ => None,
                        };
                        if let Some((min, max)) = const_bounds
                            && (max.0 <= min.0 || max.1 <= min.1)
                        {
                            self.diags.push(
                                Diag::error(
                                    region.span.clone(),
                                    "scatter region must have positive area",
                                )
                                .with_help("ensure max bounds are greater than min bounds"),
                            );
                            return None;
                        }
                        Some((min, max, const_bounds))
                    }
                    _ => {
                        self.diags.push(
                            Diag::error(region.span.clone(), "`region(...)` expects a vec2 range")
                                .with_help("write `region((x0, y0) .. (x1, y1))`"),
                        );
                        None
                    }
                }
            }
            _ => {
                self.diags.push(
                    Diag::error(
                        region.span.clone(),
                        "scatter `within` expects `screen` or `region((x0, y0) .. (x1, y1))`",
                    )
                    .with_help(
                        "example: `scatter 64 within region((0,0) .. (1,1)) seed 7 strategy compact { ... }`",
                    ),
                );
                None
            }
        }
    }

    fn hash01(seed: u32) -> f32 {
        let mut x = seed.wrapping_add(0x9E37_79B9);
        x ^= x >> 16;
        x = x.wrapping_mul(0x7FEB_352D);
        x ^= x >> 15;
        x = x.wrapping_mul(0x846C_A68B);
        x ^= x >> 16;
        (x as f32) / (u32::MAX as f32)
    }

    fn const_scalar_expr_quiet(expr: &SExpr) -> Option<f32> {
        match &expr.node {
            Expr::Num(v, unit) => {
                let s = scalar_lit(*v, *unit);
                match s {
                    Sx::Lit(x) | Sx::PxLit(x) => Some(x),
                    _ => None,
                }
            }
            Expr::Unary(UnOp::Neg, inner) => Some(-Self::const_scalar_expr_quiet(inner)?),
            Expr::Field(inner) => Self::const_scalar_expr_quiet(inner),
            // `field expr at coord` is never a compile-time constant (coord is dynamic).
            Expr::FieldAt { .. } => None,
            Expr::Binary(BinOp::Add, a, b) => {
                Some(Self::const_scalar_expr_quiet(a)? + Self::const_scalar_expr_quiet(b)?)
            }
            Expr::Binary(BinOp::Sub, a, b) => {
                Some(Self::const_scalar_expr_quiet(a)? - Self::const_scalar_expr_quiet(b)?)
            }
            Expr::Binary(BinOp::Mul, a, b) => {
                Some(Self::const_scalar_expr_quiet(a)? * Self::const_scalar_expr_quiet(b)?)
            }
            Expr::Binary(BinOp::Div, a, b) => {
                let den = Self::const_scalar_expr_quiet(b)?;
                if den == 0.0 {
                    None
                } else {
                    Some(Self::const_scalar_expr_quiet(a)? / den)
                }
            }
            Expr::Call { name, args, .. } if name == "rand" && args.len() == 1 => {
                match &args[0].value.node {
                    Expr::Range(lo, hi) => {
                        let lo = Self::const_scalar_expr_quiet(lo)?;
                        let hi = Self::const_scalar_expr_quiet(hi)?;
                        Some(f32::midpoint(lo, hi))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn const_vec2_expr_quiet(expr: &SExpr) -> Option<(f32, f32)> {
        match &expr.node {
            Expr::Vec2(a, b) => Some((
                Self::const_scalar_expr_quiet(a)?,
                Self::const_scalar_expr_quiet(b)?,
            )),
            _ => None,
        }
    }

    fn scalar_bounds_quiet(expr: &SExpr) -> Option<ScalarBounds> {
        match &expr.node {
            Expr::Num(v, unit) => {
                let s = scalar_lit(*v, *unit);
                match s {
                    Sx::Lit(x) | Sx::PxLit(x) => Some(ScalarBounds { min: x, max: x }),
                    _ => None,
                }
            }
            Expr::Unary(UnOp::Neg, inner) => {
                let inner = Self::scalar_bounds_quiet(inner)?;
                Some(ScalarBounds {
                    min: -inner.max,
                    max: -inner.min,
                })
            }
            Expr::Field(inner) => Self::scalar_bounds_quiet(inner),
            // `field expr at coord` re-samples at a different position; bounds are unknown.
            Expr::FieldAt { .. } => None,
            Expr::Binary(BinOp::Add, a, b) => {
                let a = Self::scalar_bounds_quiet(a)?;
                let b = Self::scalar_bounds_quiet(b)?;
                Some(ScalarBounds {
                    min: a.min + b.min,
                    max: a.max + b.max,
                })
            }
            Expr::Binary(BinOp::Sub, a, b) => {
                let a = Self::scalar_bounds_quiet(a)?;
                let b = Self::scalar_bounds_quiet(b)?;
                Some(ScalarBounds {
                    min: a.min - b.max,
                    max: a.max - b.min,
                })
            }
            Expr::Binary(BinOp::Mul, a, b) => {
                let a = Self::scalar_bounds_quiet(a)?;
                let b = Self::scalar_bounds_quiet(b)?;
                Some(ScalarBounds::from_values(&[
                    a.min * b.min,
                    a.min * b.max,
                    a.max * b.min,
                    a.max * b.max,
                ]))
            }
            Expr::Binary(BinOp::Div, a, b) => {
                let a = Self::scalar_bounds_quiet(a)?;
                let b = Self::scalar_bounds_quiet(b)?;
                if b.min <= 0.0 && b.max >= 0.0 {
                    return None;
                }
                Some(ScalarBounds::from_values(&[
                    a.min / b.min,
                    a.min / b.max,
                    a.max / b.min,
                    a.max / b.max,
                ]))
            }
            Expr::Call { name, args, .. } if name == "rand" && args.len() == 1 => {
                match &args[0].value.node {
                    Expr::Range(lo, hi) => {
                        let lo = Self::scalar_bounds_quiet(lo)?;
                        let hi = Self::scalar_bounds_quiet(hi)?;
                        Some(ScalarBounds {
                            min: lo.min.min(lo.max).min(hi.min).min(hi.max),
                            max: lo.min.max(lo.max).max(hi.min).max(hi.max),
                        })
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn extract_named_arg<'a>(args: &'a [Arg], name: &str) -> Option<&'a SExpr> {
        args.iter()
            .find(|a| a.name.as_deref() == Some(name))
            .map(|a| &a.value)
            .or_else(|| args.iter().find(|a| a.name.is_none()).map(|a| &a.value))
    }

    fn shadow_extent(args: &[Arg]) -> f32 {
        let soften = Self::extract_named_arg(args, "soften")
            .and_then(Self::const_scalar_expr_quiet)
            .map(f32::abs)
            .unwrap_or(0.0);
        let offset = Self::extract_named_arg(args, "offset")
            .and_then(Self::const_vec2_expr_quiet)
            .map(|(x, y)| x.hypot(y))
            .unwrap_or(0.0);
        soften + offset
    }

    fn expr_footprint_hint(expr: &SExpr) -> ScatterFootprint {
        fn walk(e: &SExpr, acc: &mut ScatterFootprint) {
            match &e.node {
                Expr::Call { name, args, .. } => {
                    if Checker::is_circle_like_shape_call(name)
                        && let Some(radius) = Checker::extract_named_arg(args, "radius")
                        && let Some(v) =
                            Checker::scalar_bounds_quiet(radius).map(ScalarBounds::abs_max)
                    {
                        acc.shape_radius = acc.shape_radius.max(v);
                    }
                    if name == "box"
                        && let Some(size) = Checker::extract_named_arg(args, "size")
                        && let Some((sx, sy)) = Checker::const_vec2_expr_quiet(size)
                    {
                        let hx = sx.abs() * 0.5;
                        let hy = sy.abs() * 0.5;
                        acc.shape_radius = acc.shape_radius.max(hx.hypot(hy));
                    }
                    if name == "glow"
                        && let Some(reach) = Checker::extract_named_arg(args, "reach")
                        && let Some(v) =
                            Checker::scalar_bounds_quiet(reach).map(ScalarBounds::abs_max)
                    {
                        acc.effect_extent = acc.effect_extent.max(v);
                    }
                    if name == "soften"
                        && let Some(radius) = Checker::extract_named_arg(args, "radius")
                        && let Some(v) =
                            Checker::scalar_bounds_quiet(radius).map(ScalarBounds::abs_max)
                    {
                        acc.effect_extent = acc.effect_extent.max(v);
                    }
                    if name == "shadow" {
                        acc.effect_extent = acc.effect_extent.max(Checker::shadow_extent(args));
                    }
                    for a in args {
                        walk(&a.value, acc);
                    }
                }
                Expr::Pipe {
                    recv, name, args, ..
                } => {
                    walk(recv, acc);
                    if name == "glow"
                        && let Some(reach) = Checker::extract_named_arg(args, "reach")
                        && let Some(v) =
                            Checker::scalar_bounds_quiet(reach).map(ScalarBounds::abs_max)
                    {
                        acc.effect_extent = acc.effect_extent.max(v);
                    }
                    if name == "soften"
                        && let Some(radius) = Checker::extract_named_arg(args, "radius")
                        && let Some(v) =
                            Checker::scalar_bounds_quiet(radius).map(ScalarBounds::abs_max)
                    {
                        acc.effect_extent = acc.effect_extent.max(v);
                    }
                    if name == "shadow" {
                        acc.effect_extent = acc.effect_extent.max(Checker::shadow_extent(args));
                    }
                    for a in args {
                        walk(&a.value, acc);
                    }
                }
                Expr::Vec2(a, b) | Expr::Range(a, b) | Expr::Binary(_, a, b) => {
                    walk(a, acc);
                    walk(b, acc);
                }
                Expr::Vec3(a, b, c) => {
                    walk(a, acc);
                    walk(b, acc);
                    walk(c, acc);
                }
                Expr::Vec4(a, b, c, d) => {
                    walk(a, acc);
                    walk(b, acc);
                    walk(c, acc);
                    walk(d, acc);
                }
                Expr::Array(items) => {
                    for item in items {
                        walk(item, acc);
                    }
                }
                Expr::ArrayComp { iterable, body, .. } => {
                    walk(iterable, acc);
                    walk(body, acc);
                }
                Expr::Lambda { body, .. } => match body {
                    LambdaBody::Expr(expr) | LambdaBody::BlockReturn(expr) => walk(expr, acc),
                },
                Expr::Unary(_, v) | Expr::Member(v, _) | Expr::Field(v) | Expr::Layer(v) => {
                    walk(v, acc);
                }
                Expr::Through { layer, .. } => walk(layer, acc),
                Expr::FieldAt { inner, coord } => {
                    walk(inner, acc);
                    walk(coord, acc);
                }
                Expr::Index { array, index } => {
                    walk(array, acc);
                    walk(index, acc);
                }
                Expr::Num(_, _)
                | Expr::Color(_)
                | Expr::Str(_)
                | Expr::Var(_)
                | Expr::PathFuture { .. } => {}
            }
        }

        let mut acc = ScatterFootprint::default();
        walk(expr, &mut acc);
        acc
    }

    fn scatter_body_footprint_hint(stmts: &[Stmt]) -> ScatterFootprint {
        fn walk_stmt(stmt: &Stmt, acc: &mut ScatterFootprint) {
            match stmt {
                Stmt::Let { value, .. }
                | Stmt::Const { value, .. }
                | Stmt::Assign { value, .. }
                | Stmt::Store { value, .. }
                | Stmt::Expr(value)
                | Stmt::Return { value, .. } => {
                    let f = Checker::expr_footprint_hint(value);
                    acc.shape_radius = acc.shape_radius.max(f.shape_radius);
                    acc.effect_extent = acc.effect_extent.max(f.effect_extent);
                }
                Stmt::Compose { entries, .. } | Stmt::ComposePiped { entries, .. } => {
                    for entry in entries {
                        match entry {
                            ComposeEntry::Expr { expr, .. } => {
                                let f = Checker::expr_footprint_hint(expr);
                                acc.shape_radius = acc.shape_radius.max(f.shape_radius);
                                acc.effect_extent = acc.effect_extent.max(f.effect_extent);
                            }
                            ComposeEntry::Block { body, .. } => {
                                for s in body {
                                    walk_stmt(s, acc);
                                }
                            }
                            ComposeEntry::If {
                                cond,
                                then_body,
                                else_body,
                                ..
                            } => {
                                let f = Checker::expr_footprint_hint(cond);
                                acc.shape_radius = acc.shape_radius.max(f.shape_radius);
                                acc.effect_extent = acc.effect_extent.max(f.effect_extent);
                                for s in then_body {
                                    walk_stmt(s, acc);
                                }
                                if let Some(else_body) = else_body {
                                    for s in else_body {
                                        walk_stmt(s, acc);
                                    }
                                }
                            }
                            ComposeEntry::InSpace { body, .. } => {
                                for s in body {
                                    walk_stmt(s, acc);
                                }
                            }
                            ComposeEntry::For { body, .. } => {
                                for s in body {
                                    walk_stmt(s, acc);
                                }
                            }
                        }
                    }
                }
                Stmt::InSpace { body, .. } | Stmt::InContext { body, .. } => {
                    for s in body {
                        walk_stmt(s, acc);
                    }
                }
                Stmt::Seq { body, .. } => {
                    for s in body {
                        walk_stmt(s, acc);
                    }
                }
                Stmt::Block { body, .. } => {
                    for s in body {
                        walk_stmt(s, acc);
                    }
                }
                Stmt::LetScatter { scatter, .. } => {
                    for s in &scatter.body {
                        walk_stmt(s, acc);
                    }
                }
                Stmt::For { body, .. } => {
                    for s in body {
                        walk_stmt(s, acc);
                    }
                }
                Stmt::If {
                    cond,
                    then_body,
                    else_body,
                    ..
                } => {
                    let f = Checker::expr_footprint_hint(cond);
                    acc.shape_radius = acc.shape_radius.max(f.shape_radius);
                    acc.effect_extent = acc.effect_extent.max(f.effect_extent);
                    for s in then_body {
                        walk_stmt(s, acc);
                    }
                    if let Some(else_body) = else_body {
                        for s in else_body {
                            walk_stmt(s, acc);
                        }
                    }
                }
                Stmt::Match {
                    value,
                    arms,
                    default_body,
                    ..
                } => {
                    let f = Checker::expr_footprint_hint(value);
                    acc.shape_radius = acc.shape_radius.max(f.shape_radius);
                    acc.effect_extent = acc.effect_extent.max(f.effect_extent);
                    for arm in arms {
                        for s in &arm.body {
                            walk_stmt(s, acc);
                        }
                    }
                    if let Some(default_body) = default_body {
                        for s in default_body {
                            walk_stmt(s, acc);
                        }
                    }
                }
                Stmt::TextureBinding { .. }
                | Stmt::Param { .. }
                | Stmt::SpaceDecl { .. }
                | Stmt::StyleDecl { .. }
                | Stmt::CanvasSpace { .. }
                | Stmt::SurfaceVertex { .. }
                | Stmt::LocalFnDecl(_)
                | Stmt::ReturnVoid { .. }
                | Stmt::Break { .. } => {}
            }
        }

        let mut acc = ScatterFootprint::default();
        for s in stmts {
            walk_stmt(s, &mut acc);
        }
        acc
    }

    fn scatter_instance_pos(
        idx: usize,
        count: usize,
        seed: u32,
        min: (f32, f32),
        max: (f32, f32),
    ) -> (f32, f32) {
        let cols = (count as f32).sqrt().ceil().max(1.0) as usize;
        let rows = count.div_ceil(cols).max(1);
        let col = idx % cols;
        let row = idx.div_euclid(cols);

        let jx = Self::hash01(seed ^ (idx as u32).wrapping_mul(0xA24B_AED4));
        let jy = Self::hash01(seed ^ (idx as u32).wrapping_mul(0x9FB2_1C65));

        let fx = ((col as f32) + jx) / (cols as f32);
        let fy = ((row as f32) + jy) / (rows as f32);

        (min.0 + fx * (max.0 - min.0), min.1 + fy * (max.1 - min.1))
    }

    fn scatter_bin_index(
        pos: (f32, f32),
        min: (f32, f32),
        max: (f32, f32),
        bins_x: usize,
        bins_y: usize,
    ) -> usize {
        let wx = (max.0 - min.0).max(1.0e-6);
        let wy = (max.1 - min.1).max(1.0e-6);

        let nx = ((pos.0 - min.0) / wx).clamp(0.0, 0.999_999);
        let ny = ((pos.1 - min.1) / wy).clamp(0.0, 0.999_999);

        let bx = (nx * bins_x as f32).floor() as usize;
        let by = (ny * bins_y as f32).floor() as usize;
        by * bins_x + bx
    }

    pub(super) fn eval_compile_time_iter_values(
        &mut self,
        iterable: &SExpr,
        context_name: &str,
    ) -> Option<Vec<Value>> {
        match &iterable.node {
            Expr::Range(start_expr, end_expr) => {
                let start_sx = self.as_scalar(start_expr)?;
                let end_sx = self.as_scalar(end_expr)?;

                let start = self.eval_const_scalar_sx(&start_sx, &start_expr.span, context_name)?;
                let end = self.eval_const_scalar_sx(&end_sx, &end_expr.span, context_name)?;

                if start.fract() != 0.0 || end.fract() != 0.0 {
                    self.diags.push(
                        Diag::error(
                            iterable.span.clone(),
                            format!("{context_name} range bounds must be integers"),
                        )
                        .with_help("use integer literals, for example `for i in 0 .. 8 { ... }`"),
                    );
                    return None;
                }

                let start_i = start as i32;
                let end_i = end as i32;
                let count = (i64::from(end_i) - i64::from(start_i)).unsigned_abs();
                if count > 4096 {
                    self.diags.push(
                        Diag::error(
                            iterable.span.clone(),
                            format!(
                                "{context_name} expands to {count} iterations, exceeding v1 limit of 4096"
                            ),
                        )
                        .with_help("reduce the range span or split the loop into smaller chunks"),
                    );
                    return None;
                }

                let values = if start_i <= end_i {
                    (start_i..end_i)
                        .map(|i| Value::Scalar(Sx::Lit(i as f32)))
                        .collect::<Vec<_>>()
                } else {
                    (end_i..start_i)
                        .rev()
                        .map(|i| Value::Scalar(Sx::Lit(i as f32)))
                        .collect::<Vec<_>>()
                };

                self.hir.notes.push(format!(
                    "loop: unrolled ({context_name}), iterations={}, reason=compile-time integer range",
                    values.len()
                ));

                Some(values)
            }
            _ => {
                let values = self.eval(iterable)?;
                match values {
                    Value::Array(items) => {
                        self.hir.notes.push(format!(
                            "loop: unrolled ({context_name}), iterations={}, reason=compile-time array",
                            items.len()
                        ));
                        Some(items)
                    }
                    other => {
                        self.diags.push(
                            Diag::error(
                                iterable.span.clone(),
                                format!(
                                    "{context_name} expects an array or range iterable, found {}",
                                    other.kind()
                                ),
                            )
                            .with_help(
                                "use `for i in 0 .. n { ... }` or `for x in some_array { ... }`",
                            ),
                        );
                        None
                    }
                }
            }
        }
    }

    /// Evaluate a statement block; its value is the last layer-producing
    /// statement (compose, in-space block, or layer expression).
    pub(super) fn eval_block(
        &mut self,
        stmts: &[Stmt],
        block_span: &Span,
        is_canvas_root: bool,
        allow_empty: bool,
    ) -> Option<LayerId> {
        let diags_before_block = self.diags.len();
        let mut last: Option<(LayerId, Span)> = None;
        let mut produced_layers: Vec<(LayerId, Blend)> = Vec::new();
        if is_canvas_root {
            info!(stmts = stmts.len(), "checker block start: canvas root");
        } else {
            debug!(stmts = stmts.len(), "checker block start");
        }

        for (stmt_idx, stmt) in stmts.iter().enumerate() {
            let stmt_kind = Self::stmt_kind(stmt);
            if is_canvas_root {
                info!(idx = stmt_idx, kind = stmt_kind, "checker stmt start");
            } else {
                debug!(idx = stmt_idx, kind = stmt_kind, "checker stmt start");
            }

            match stmt {
                Stmt::Store { span, .. } => self.diags.push(Diag::error(
                    span.clone(),
                    "indexed writes require an explicit GPU stage program",
                )),
                Stmt::TextureBinding {
                    name,
                    ty_name,
                    default_asset,
                    span,
                    ..
                } => {
                    if !is_canvas_root {
                        self.diags.push(
                            Diag::error(
                                span.clone(),
                                "texture bindings are only allowed at canvas top level",
                            )
                            .with_help("move this declaration to the top-level canvas body"),
                        );
                        continue;
                    }
                    if ty_name != "texture" {
                        self.diags.push(
                            Diag::error(
                                span.clone(),
                                format!(
                                    "uniform `{name}` must declare type `texture`, found `{ty_name}`"
                                ),
                            )
                            .with_help("use `uniform <name>: texture`")
                        );
                        continue;
                    }

                    self.hir.register_texture(name);
                    if let Some(path) = default_asset {
                        self.hir.set_texture_default_asset(name, path.clone());
                    }
                }
                Stmt::Param {
                    name,
                    name_span,
                    ty_name,
                    default,
                    range,
                    ..
                } => {
                    self.declare_param(name, name_span, ty_name, default, range.as_ref());
                }
                Stmt::Let {
                    name,
                    value,
                    declared_ty_name,
                    declared_ty_span,
                    ..
                } => {
                    if let Some(v) = self.eval_scalar_expected(
                        value,
                        declared_ty_name
                            .as_deref()
                            .and_then(crate::typed_scalar::Kind::parse),
                    ) {
                        if let (Some(ty_name), Some(ty_span)) =
                            (declared_ty_name.as_ref(), declared_ty_span.as_ref())
                            && !local_decl_type_matches(
                                &v,
                                ty_name,
                                &self.enum_defs,
                                &self.struct_defs,
                                &[],
                            )
                        {
                            self.diags.push(
                                Diag::error(
                                    ty_span.clone(),
                                    format!(
                                        "typed local `{name}` expected {}, but got {}",
                                        local_decl_expected_kind(
                                            ty_name,
                                            &self.enum_defs,
                                            &self.struct_defs,
                                            &[],
                                        ),
                                        v.kind()
                                    ),
                                )
                                .with_label("declared type does not match initializer")
                                .with_help("adjust the declared type or initializer expression"),
                            );
                            continue;
                        }
                        self.bind(name.clone(), v);
                    }
                }
                Stmt::Const {
                    name,
                    value,
                    ty_name,
                    ty_span,
                    ..
                } => {
                    if let Some(v) = self.eval_expected(value, ty_name) {
                        if !local_decl_type_matches(
                            &v,
                            ty_name,
                            &self.enum_defs,
                            &self.struct_defs,
                            &[],
                        ) {
                            self.diags.push(
                                Diag::error(
                                    ty_span.clone(),
                                    format!(
                                        "const `{name}` expected {}, but got {}",
                                        local_decl_expected_kind(
                                            ty_name,
                                            &self.enum_defs,
                                            &self.struct_defs,
                                            &[],
                                        ),
                                        v.kind()
                                    ),
                                )
                                .with_label("declared type does not match initializer")
                                .with_help(
                                    "adjust the declared type or const initializer expression",
                                ),
                            );
                            continue;
                        }

                        let what = format!("const `{name}` initializer");
                        if let Some(folded) =
                            self.eval_compile_time_const_value(&v, &value.span, &what)
                        {
                            self.bind(name.clone(), folded);
                        }
                    }
                }
                Stmt::Assign {
                    name,
                    name_span,
                    field_path,
                    value,
                    ..
                } => {
                    if let Some(v) = self.eval(value) {
                        if let Some(field_path) = field_path {
                            if field_path.contains('.') {
                                let result = self
                                    .lookup(name)
                                    .ok_or_else(|| format!("undeclared local `{name}`"))
                                    .and_then(|base| self.assign_field_path(base, field_path, v));
                                match result {
                                    Ok(updated) => {
                                        self.assign(name, updated);
                                    }
                                    Err(message) => {
                                        self.diags.push(Diag::error(name_span.clone(), format!("invalid assignment target `{name}.{field_path}`")).with_help(message));
                                        continue;
                                    }
                                }
                                continue;
                            }

                            let Some(base) = self.lookup(name) else {
                                self.diags.push(
                                    Diag::error(
                                        name_span.clone(),
                                        format!("cannot assign to undeclared local `{name}`"),
                                    )
                                    .with_label("unknown assignment target")
                                    .with_help(
                                        "declare the variable first with `let name = ...` or `type name = ...`",
                                    ),
                                );
                                continue;
                            };

                            let updated = match base {
                                Value::Struct {
                                    ty_name,
                                    mut fields,
                                } => {
                                    let Some(field_def) = self
                                        .struct_defs
                                        .get(&ty_name)
                                        .and_then(|def| def.fields.get(field_path))
                                    else {
                                        self.diags.push(
                                            Diag::error(
                                                name_span.clone(),
                                                format!(
                                                    "invalid assignment target `{name}.{field_path}`"
                                                ),
                                            )
                                            .with_help(format!(
                                                "`{ty_name}` has no field `{field_path}`"
                                            )),
                                        );
                                        continue;
                                    };

                                    if !local_decl_type_matches(
                                        &v,
                                        &field_def.ty_name,
                                        &self.enum_defs,
                                        &self.struct_defs,
                                        &[],
                                    ) {
                                        self.diags.push(
                                            Diag::error(
                                                name_span.clone(),
                                                format!(
                                                    "cannot assign {} to `{name}.{field_path}`",
                                                    v.kind()
                                                ),
                                            )
                                            .with_help(format!(
                                                "expected {} for field `{field_path}`",
                                                local_decl_expected_kind(
                                                    &field_def.ty_name,
                                                    &self.enum_defs,
                                                    &self.struct_defs,
                                                    &[]
                                                )
                                            )),
                                        );
                                        continue;
                                    }

                                    fields.insert(field_path.clone(), v);
                                    Value::Struct { ty_name, fields }
                                }
                                other => match apply_swizzle_assignment(other, field_path, v) {
                                    Ok(updated) => updated,
                                    Err(msg) => {
                                        self.diags.push(
                                            Diag::error(
                                                name_span.clone(),
                                                format!(
                                                    "invalid assignment target `{name}.{field_path}`"
                                                ),
                                            )
                                            .with_help(msg),
                                        );
                                        continue;
                                    }
                                },
                            };
                            let _ = self.assign(name, updated);
                        } else if !self.assign(name, v) {
                            self.diags.push(
                                Diag::error(
                                    name_span.clone(),
                                    format!("cannot assign to undeclared local `{name}`"),
                                )
                                .with_label("unknown assignment target")
                                .with_help(
                                    "declare the variable first with `let name = ...` or `type name = ...`",
                                ),
                            );
                        }
                    }
                }
                Stmt::For {
                    name,
                    iterable,
                    body,
                    span,
                    index_name,
                    ..
                } => {
                    let Some(values) = self.eval_compile_time_iter_values(iterable, "for-loop")
                    else {
                        continue;
                    };

                    for (iter_idx, value) in values.into_iter().enumerate() {
                        self.scopes.push(HashMap::new());
                        self.style_scopes.push(HashMap::new());
                        self.bind(name.clone(), value);
                        if let Some((idx_name, _)) = index_name {
                            self.bind(idx_name.clone(), Value::Scalar(Sx::Lit(iter_idx as f32)));
                        }
                        if let Some(id) = self.eval_block(body, span, false, false) {
                            last = Some((id, span.clone()));
                            produced_layers.push((id, Blend::Over));
                        }
                        self.scopes.pop();
                        self.style_scopes.pop();
                    }
                }
                Stmt::If {
                    cond,
                    then_body,
                    else_body,
                    span,
                } => {
                    let Some(cond_sx) = self.as_scalar(cond) else {
                        self.diags.push(
                            Diag::error(
                                cond.span.clone(),
                                "`if` condition must be a scalar expression",
                            )
                            .with_help(
                                "use a scalar condition like `if cos(angle) { ... } else { ... }`",
                            ),
                        );
                        continue;
                    };

                    self.scopes.push(HashMap::new());
                    self.style_scopes.push(HashMap::new());
                    let then_layer = self.eval_block(then_body, span, false, false);
                    self.scopes.pop();
                    self.style_scopes.pop();

                    let Some(then_layer) = then_layer else {
                        continue;
                    };

                    let else_layer = if let Some(else_body) = else_body {
                        self.scopes.push(HashMap::new());
                        self.style_scopes.push(HashMap::new());
                        let else_layer = self.eval_block(else_body, span, false, false);
                        self.scopes.pop();
                        self.style_scopes.pop();
                        let Some(else_layer) = else_layer else {
                            continue;
                        };
                        else_layer
                    } else {
                        self.hir.layer(Layer::Solid([0.0, 0.0, 0.0, 0.0]))
                    };

                    let id = self.hir.layer(Layer::If {
                        cond: cond_sx,
                        then_layer,
                        else_layer,
                    });
                    last = Some((id, span.clone()));
                    produced_layers.push((id, Blend::Over));
                }
                Stmt::Match { span, .. } => {
                    self.diags.push(
                        Diag::error(
                            span.clone(),
                            "`match` is not yet supported in canvas/surface layer blocks",
                        )
                        .with_help("use `if` chains in layer blocks for now"),
                    );
                    continue;
                }
                Stmt::LetScatter { name, scatter, .. } => {
                    #[derive(Clone)]
                    struct ScatterCandidate {
                        idx: usize,
                        index01: f32,
                        pos: (f32, f32),
                        bin: usize,
                        score: f32,
                    }

                    let Some(count_raw) =
                        self.eval_const_scalar_expr(&scatter.count, "scatter count")
                    else {
                        continue;
                    };
                    if !count_raw.is_finite() || count_raw <= 0.0 {
                        self.diags.push(
                            Diag::error(
                                scatter.count.span.clone(),
                                "scatter count must be a positive constant",
                            )
                            .with_help("use a positive numeric literal like `scatter 64 ...`"),
                        );
                        continue;
                    }
                    let count = count_raw.round() as usize;

                    let seed = match self.eval_const_scalar_expr(&scatter.seed, "scatter seed") {
                        Some(v) => v.max(0.0).round() as u32,
                        None => continue,
                    };

                    let strategy = match scatter.strategy.node.as_str() {
                        "compact" => ScatterLoweringStrategy::Compact,
                        "branch" | "branch_tree" => ScatterLoweringStrategy::BranchTree,
                        "procedural" => ScatterLoweringStrategy::Procedural,
                        _ => {
                            self.diags.push(
                                Diag::error(
                                    scatter.strategy.span.clone(),
                                    format!("unknown scatter strategy `{}`", scatter.strategy.node),
                                )
                                .with_help("use one of: `compact`, `branch`, or `procedural`"),
                            );
                            continue;
                        }
                    };

                    if strategy != ScatterLoweringStrategy::Procedural
                        && count > Self::MAX_SCATTER_INSTANCES
                    {
                        self.diags.push(
                            Diag::error(
                                scatter.count.span.clone(),
                                format!(
                                    "scatter count {count} exceeds v0 bound of {}",
                                    Self::MAX_SCATTER_INSTANCES
                                ),
                            )
                            .with_help("lower the count, use `strategy procedural`, or wait for compute-backed scatter"),
                        );
                        continue;
                    }

                    let Some((min, max, const_bounds)) =
                        self.scatter_region_bounds(&scatter.region)
                    else {
                        continue;
                    };

                    self.push_scatter_rand_scope(seed);

                    let lifecycle = if let Some(lifecycle) = &scatter.lifecycle {
                        let lifetime =
                            self.lifecycle_scalar_expr(&lifecycle.lifetime, "scatter lifetime");
                        let respawn = self.lifecycle_scalar_expr(
                            &lifecycle.respawn_every,
                            "scatter respawn interval",
                        );
                        if let (
                            Some((lifetime_sx, lifetime_bounds)),
                            Some((respawn_sx, respawn_bounds)),
                        ) = (lifetime, respawn)
                            && lifetime_bounds.min.is_finite()
                            && lifetime_bounds.max.is_finite()
                            && respawn_bounds.min.is_finite()
                            && respawn_bounds.max.is_finite()
                            && lifetime_bounds.min > 0.0
                            && respawn_bounds.min > 0.0
                        {
                            let lifetime = lifetime_bounds.midpoint();
                            let respawn = respawn_bounds.midpoint();
                            let duty = (lifetime / respawn).clamp(0.0, 1.0);
                            let expected_live = (count as f32) * duty;
                            self.hir.notes.push(format!(
                                "scatter lifecycle: lifetime≈{lifetime:.3}s, respawn≈{respawn:.3}s, duty≈{duty:.3}, expected concurrently active≈{expected_live:.2}/{count}",
                            ));
                            Some(ScatterLifecycleParams {
                                lifetime: lifetime_sx,
                                respawn_every: respawn_sx,
                            })
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    if strategy == ScatterLoweringStrategy::Procedural && lifecycle.is_some() {
                        self.diags.push(
                            Diag::error(
                                scatter.strategy.span.clone(),
                                "`procedural` scatter strategy does not support lifecycle bindings in v0",
                            )
                            .with_help("use `strategy compact` or `strategy branch` for lifecycle scatter"),
                        );
                        continue;
                    }

                    // Size bins so average occupancy stays near the runtime per-bin loop cap.
                    let bins_axis = (((count as f32 / Self::MAX_SCATTER_CANDIDATES_PER_BIN as f32)
                        .sqrt())
                    .ceil() as usize)
                        .clamp(1, Self::MAX_SCATTER_BINS_AXIS);
                    let bins_x = bins_axis;
                    let bins_y = bins_axis;

                    let footprint = Self::scatter_body_footprint_hint(&scatter.body);
                    let footprint_base = footprint.score();

                    let mut bins: Vec<Vec<ScatterCandidate>> = vec![Vec::new(); bins_x * bins_y];
                    let unit_min = (0.0, 0.0);
                    let unit_max = (1.0, 1.0);
                    for idx in 0..count {
                        let pos = Self::scatter_instance_pos(idx, count, seed, unit_min, unit_max);
                        let bin = Self::scatter_bin_index(pos, unit_min, unit_max, bins_x, bins_y);
                        let denom = if count <= 1 { 1.0 } else { (count - 1) as f32 };
                        bins[bin].push(ScatterCandidate {
                            idx,
                            index01: (idx as f32) / denom,
                            pos,
                            bin,
                            score: footprint_base,
                        });
                    }

                    let occupancy_before: Vec<usize> = bins.iter().map(Vec::len).collect();

                    let mut kept: Vec<ScatterCandidate> = Vec::new();
                    for bin in &mut bins {
                        bin.sort_by(|a, b| {
                            b.score
                                .partial_cmp(&a.score)
                                .unwrap_or(std::cmp::Ordering::Equal)
                                .then_with(|| a.idx.cmp(&b.idx))
                        });
                        kept.extend(bin.iter().cloned());
                    }

                    kept.sort_by_key(|c| c.idx);

                    let mut bin_instances: Vec<Vec<hir::ScatterInstance>> =
                        vec![Vec::new(); bins_x * bins_y];
                    for candidate in kept {
                        bin_instances[candidate.bin].push(hir::ScatterInstance {
                            id: candidate.idx as u32,
                            index01: candidate.index01,
                            pos: candidate.pos,
                            footprint: candidate.score.max(0.01),
                        });
                    }

                    self.scopes.push(HashMap::new());
                    self.style_scopes.push(HashMap::new());
                    let scatter_instance = Value::ScatterInstance {
                        pos: (Sx::ScatterInstancePosX, Sx::ScatterInstancePosY),
                        id: Sx::ScatterInstanceId,
                        index01: Sx::ScatterInstanceIndex01,
                        age_norm: Sx::ScatterInstanceAgeNorm,
                    };
                    if let Some(lifecycle) = &scatter.lifecycle {
                        self.bind(lifecycle.instance_name.node.clone(), scatter_instance);
                    } else {
                        self.bind("instance".to_string(), scatter_instance);
                    }

                    let template_inner =
                        self.eval_block(&scatter.body, &scatter.span, false, false);
                    self.scopes.pop();
                    self.style_scopes.pop();
                    self.pop_scatter_rand_scope();

                    let Some(template_inner) = template_inner else {
                        continue;
                    };
                    let Some(body_layer) =
                        self.apply_layer_pipes(template_inner, &scatter.post_pipes, &scatter.span)
                    else {
                        continue;
                    };

                    let kept_in_bins = bin_instances.iter().map(Vec::len).sum::<usize>();
                    if kept_in_bins == 0 {
                        self.diags.push(
                            Diag::error(
                                scatter.span.clone(),
                                "scatter produced no drawable instance layers",
                            )
                            .with_help(
                                "ensure the scatter body ends with a layer-producing expression",
                            ),
                        );
                        continue;
                    }

                    let layer = self.hir.layer(Layer::ScatterBins {
                        min,
                        max,
                        bins_x,
                        bins_y,
                        strategy,
                        lifecycle,
                        bins: bin_instances,
                        body: body_layer,
                    });
                    if let Some((const_min, const_max)) = const_bounds {
                        self.hir.notes.push(format!(
                            "scatter: binned {count} instance(s) into {}x{} bins, kept {} (no candidate truncation), footprint≈{:.3}, bounds ({:.3},{:.3})..({:.3},{:.3}), seed {seed}",
                            bins_x,
                            bins_y,
                            kept_in_bins,
                            footprint_base,
                            const_min.0,
                            const_min.1,
                            const_max.0,
                            const_max.1
                        ));
                    } else {
                        self.hir.notes.push(format!(
                            "scatter: binned {count} instance(s) into {}x{} bins, kept {} (no candidate truncation), footprint≈{:.3}, bounds <dynamic>, seed {seed}",
                            bins_x,
                            bins_y,
                            kept_in_bins,
                            footprint_base
                        ));
                    }
                    self.hir
                        .notes
                        .push(format!("scatter bins occupancy: {:?}", occupancy_before));
                    self.bind(name.clone(), Value::Layer(layer));
                }
                Stmt::SpaceDecl {
                    name, chain, span, ..
                } => {
                    if let Some(resolved) = self.eval_space_chain(chain) {
                        if !resolved.repeat_bindings.is_empty() {
                            self.diags.push(
                                Diag::error(
                                    span.clone(),
                                    "named repeat cell bindings are only valid on immediate `in space` blocks",
                                )
                                .with_help(
                                    "remove the repeat-cell binding (`cell: ...`, `id: ...`, or `as: ...`) from the space declaration and bind it where the space is used",
                                ),
                            );
                            continue;
                        }
                        let value = Value::Space(resolved.xforms.clone());
                        self.record_span_value(span.start, span.end, &value);
                        self.bind(name.clone(), value);
                    }
                }
                Stmt::StyleDecl {
                    name,
                    name_span,
                    params,
                    stages,
                    span,
                } => {
                    self.bind_style(
                        name.clone(),
                        StyleDef {
                            params: params.clone(),
                            stages: stages.clone(),
                            span: span.clone(),
                        },
                        name_span.clone(),
                    );
                }
                Stmt::CanvasSpace { chain, span } => {
                    let _ = (chain, is_canvas_root);
                    self.diags.push(
                        Diag::error(
                            span.clone(),
                            "`canvas_space` has been removed; use explicit `space` + `in space`",
                        )
                        .with_help(
                            "define a named space (`space warped = ...`) and wrap affected layers in `in space warped { ... }`",
                        ),
                    );
                    continue;
                }
                Stmt::Compose { entries, span } => {
                    if let Some(id) = self.eval_compose(entries, span) {
                        last = Some((id, span.clone()));
                        produced_layers.push((id, Blend::Over));
                    }
                }
                Stmt::ComposePiped {
                    entries,
                    pipes,
                    span,
                } => {
                    if let Some(id) = self.eval_compose(entries, span)
                        && let Some(layer) = self.apply_layer_pipes(id, pipes, span)
                    {
                        last = Some((layer, span.clone()));
                        produced_layers.push((layer, Blend::Over));
                    }
                }
                Stmt::SurfaceVertex { span, .. } => {
                    self.diags.push(
                        Diag::error(
                            span.clone(),
                            "`vertex { ... }` is only valid inside `surface` declarations",
                        )
                        .with_help("move this block into a `surface ... { ... }` body"),
                    );
                }
                Stmt::InContext { value, body, span } => {
                    let Some(value) = self.eval(value) else {
                        continue;
                    };
                    let roles = self.context_roles(&value, span);
                    let previous = self.evaluation_context.replace(roles);
                    let cache = std::mem::take(&mut self.runtime_channel_cache);
                    let coord = self.activate_context_coordinate();
                    self.scopes.push(HashMap::new());
                    self.style_scopes.push(HashMap::new());
                    let inner = self.eval_block(body, span, false, false);
                    self.style_scopes.pop();
                    self.scopes.pop();
                    self.evaluation_context = previous;
                    self.runtime_channel_cache = cache;
                    if let Some(inner) = inner {
                        let id = self.context_layer(inner, coord);
                        last = Some((id, span.clone()));
                        produced_layers.push((id, Blend::Over));
                    }
                }
                Stmt::InSpace { chain, body, span } => {
                    let resolved = self.eval_space_chain(chain);
                    self.scopes.push(HashMap::new());
                    self.style_scopes.push(HashMap::new());
                    let bindings_ok = resolved.as_ref().is_none_or(|resolved| {
                        self.bind_repeat_cells(&resolved.repeat_bindings, span)
                    });
                    let diags_before_inner = self.diags.len();
                    let inner = self.eval_block(body, span, false, true);
                    let inner_had_errors = self
                        .diags
                        .iter()
                        .skip(diags_before_inner)
                        .any(|d| d.severity == Severity::Error);
                    self.scopes.pop();
                    self.style_scopes.pop();
                    if let (Some(inner), Some(resolved), true) = (inner, resolved, bindings_ok) {
                        let id = self.hir.layer(Layer::InSpace {
                            xforms: resolved.xforms,
                            inner,
                        });
                        last = Some((id, span.clone()));
                        produced_layers.push((id, Blend::Over));
                    } else if inner_had_errors {
                        continue;
                    }
                }
                Stmt::Block { body, span } => {
                    self.scopes.push(HashMap::new());
                    self.style_scopes.push(HashMap::new());
                    let grouped = self.eval_block(body, span, is_canvas_root, false);
                    self.scopes.pop();
                    self.style_scopes.pop();

                    if let Some(id) = grouped {
                        last = Some((id, span.clone()));
                        produced_layers.push((id, Blend::Over));
                    }
                }
                Stmt::Seq { body, .. } => {
                    for inner in body {
                        if let Stmt::Let {
                            name,
                            value,
                            declared_ty_name,
                            declared_ty_span,
                            ..
                        } = inner
                            && let Some(v) = self.eval(value)
                        {
                            if let (Some(ty_name), Some(ty_span)) =
                                (declared_ty_name.as_ref(), declared_ty_span.as_ref())
                                && !local_decl_type_matches(
                                    &v,
                                    ty_name,
                                    &self.enum_defs,
                                    &self.struct_defs,
                                    &[],
                                )
                            {
                                self.diags.push(
                                    Diag::error(
                                        ty_span.clone(),
                                        format!(
                                            "typed local `{name}` expected {}, but got {}",
                                            local_decl_expected_kind(
                                                ty_name,
                                                &self.enum_defs,
                                                &self.struct_defs,
                                                &[],
                                            ),
                                            v.kind()
                                        ),
                                    )
                                    .with_label("declared type does not match initializer")
                                    .with_help(
                                        "adjust the declared type or initializer expression",
                                    ),
                                );
                                continue;
                            }
                            self.bind(name.clone(), v);
                        }
                    }
                }
                Stmt::Expr(e) => {
                    let span = e.span.clone();
                    let mut stmt_blend = Blend::Over;
                    let expr = match &e.node {
                        Expr::Pipe {
                            recv,
                            name,
                            name_span,
                            args,
                        } if name == "blend" => {
                            let mut arg_error = |msg: &str| {
                                self.diags.push(
                                    Diag::error(name_span.clone(), msg)
                                        .with_help(
                                            "use `blend(<mode>)` where mode is one of: over, add, screen, multiply",
                                        ),
                                );
                            };

                            if args.is_empty() {
                                arg_error("`blend(...)` expects one mode argument");
                                continue;
                            }
                            if args.len() > 1 {
                                self.diags.push(
                                    Diag::error(
                                        args[1].value.span.clone(),
                                        "`blend(...)` accepts only one mode argument",
                                    )
                                    .with_help(
                                        "use `blend(over)`, `blend(add)`, `blend(screen)`, or `blend(multiply)`",
                                    ),
                                );
                                continue;
                            }

                            let arg = &args[0];
                            if let Some(name) = arg.name.as_deref()
                                && name != "mode"
                            {
                                self.diags.push(
                                    Diag::error(
                                        arg.value.span.clone(),
                                        format!(
                                            "unknown argument `{name}` to `blend`; expected `mode`"
                                        ),
                                    )
                                    .with_help("use `blend(mode: add)` or `blend(add)`"),
                                );
                                continue;
                            }

                            let mode = match &arg.value.node {
                                Expr::Var(mode) => mode.as_str(),
                                _ => {
                                    self.diags.push(
                                        Diag::error(
                                            arg.value.span.clone(),
                                            "`blend(...)` expects a mode identifier",
                                        )
                                        .with_help("use one of: over, add, screen, multiply"),
                                    );
                                    continue;
                                }
                            };

                            if !matches!(mode, "over" | "add" | "screen" | "multiply") {
                                self.diags.push(
                                    Diag::error(
                                        arg.value.span.clone(),
                                        format!("unknown blend mode `{mode}`"),
                                    )
                                    .with_help("available blends: over, add, screen, multiply"),
                                );
                                continue;
                            }

                            stmt_blend = match mode {
                                "over" => Blend::Over,
                                "add" => Blend::Add,
                                "screen" => Blend::Screen,
                                "multiply" => Blend::Multiply,
                                _ => Blend::Over,
                            };

                            recv.as_ref()
                        }
                        _ => e,
                    };

                    match self.eval(expr) {
                        Some(Value::Error) => {}
                        Some(Value::Color { rgba, .. }) => {
                            let id = self.hir.layer(Layer::Solid(rgba));
                            last = Some((id, span));
                            produced_layers.push((id, stmt_blend));
                        }
                        Some(Value::ColorField { rgba, .. }) => {
                            let [r, g, b, a] = rgba;
                            let id = self.hir.layer(Layer::ColorExpr { r, g, b, a });
                            last = Some((id, span));
                            produced_layers.push((id, stmt_blend));
                        }
                        Some(Value::Layer(id)) => {
                            last = Some((id, span));
                            produced_layers.push((id, stmt_blend));
                        }
                        Some(Value::TypedTextureSample {
                            layer_id,
                            tex_name,
                            sample_at,
                        }) => {
                            if sample_at.is_none()
                                && !self.implicit_texture_uv_allowed(
                                    span.clone(),
                                    &tex_name,
                                    "sample explicitly",
                                    format!(
                                        "use `{tex_name}.at(uv)` or `image({tex_name}, at: uv)`; staged rollout: `#pragma check.warn_implicit_texture_uv = true` for warnings or `#pragma check.allow_implicit_texture_uv = true` to suppress the diagnostic"
                                    ),
                                )
                            {
                                continue;
                            } else {
                                last = Some((layer_id, span));
                                produced_layers.push((layer_id, stmt_blend));
                            }
                        }
                        Some(Value::Shape(_)) => self.diags.push(
                            Diag::error(span, "a bare shape is not drawable")
                                .with_label("this is a shape, not a layer")
                                .with_help("give it ink: `|> fill(#ff2d78)`"),
                        ),
                        Some(_) | None => {}
                    }
                }
                Stmt::Return { span, .. } | Stmt::ReturnVoid { span } => {
                    self.diags.push(
                        Diag::error(
                            span.clone(),
                            "`return` is only valid inside function bodies",
                        )
                        .with_help("remove `return` from canvas/compose blocks"),
                    );
                }
                Stmt::Break { span } => {
                    self.diags.push(
                        Diag::error(
                            span.clone(),
                            "`break` is only valid inside function loop bodies",
                        )
                        .with_help("use `break` only inside a function `for` loop"),
                    );
                }
                Stmt::LocalFnDecl(fn_decl) => {
                    self.register_local_fn_decl(fn_decl);
                }
            }

            if is_canvas_root {
                info!(
                    idx = stmt_idx,
                    kind = stmt_kind,
                    diags = self.diags.len(),
                    produced_layers = produced_layers.len(),
                    "checker stmt done"
                );
            } else {
                debug!(
                    idx = stmt_idx,
                    kind = stmt_kind,
                    diags = self.diags.len(),
                    produced_layers = produced_layers.len(),
                    "checker stmt done"
                );
            }
        }
        let has_new_errors = self
            .diags
            .iter()
            .skip(diags_before_block)
            .any(|d| d.severity == Severity::Error);

        if !allow_empty && last.is_none() && !has_new_errors {
            self.diags.push(
                Diag::error(block_span.clone(), "this block produces no layer")
                    .with_help("blocks must end in a `compose { ... }` or a layer expression"),
            );
        }
        if self
            .diags
            .iter()
            .skip(diags_before_block)
            .any(|d| d.severity == Severity::Error)
        {
            if is_canvas_root {
                info!(
                    diags = self.diags.len(),
                    "checker block failed: canvas root"
                );
            } else {
                debug!(diags = self.diags.len(), "checker block failed");
            }
            return None;
        }
        if produced_layers.len() > 1 {
            if is_canvas_root {
                info!(
                    layers = produced_layers.len(),
                    "checker block done: composed root output"
                );
            } else {
                debug!(
                    layers = produced_layers.len(),
                    "checker block done: composed output"
                );
            }
            return Some(self.hir.layer(Layer::Compose(produced_layers)));
        }
        if is_canvas_root {
            info!(has_last = last.is_some(), "checker block done: canvas root");
        } else {
            debug!(has_last = last.is_some(), "checker block done");
        }
        last.map(|(id, _)| id)
    }

    fn eval_compose(&mut self, entries: &[ComposeEntry], span: &Span) -> Option<LayerId> {
        info!(entries = entries.len(), "checker compose start");
        let mut out = Vec::new();
        let mut had_error = false;

        let parse_blend =
            |checker: &mut Checker, blend_spec: &Option<Spanned<String>>| match blend_spec {
                None => Blend::Over,
                Some(b) => match b.node.as_str() {
                    "over" => Blend::Over,
                    "add" => Blend::Add,
                    "screen" => Blend::Screen,
                    "multiply" => Blend::Multiply,
                    other => {
                        checker.diags.push(
                            Diag::error(b.span.clone(), format!("unknown blend mode `{other}`"))
                                .with_help("available blends: over, add, screen, multiply"),
                        );
                        Blend::Over
                    }
                },
            };

        for (entry_idx, e) in entries.iter().enumerate() {
            let entry_kind = Self::compose_entry_kind(e);
            info!(
                idx = entry_idx,
                kind = entry_kind,
                "checker compose entry start"
            );

            if let ComposeEntry::For {
                name,
                iterable,
                body,
                blend,
                span,
                index_name,
                ..
            } = e
            {
                let Some(values) = self.eval_compile_time_iter_values(iterable, "compose for-loop")
                else {
                    had_error = true;
                    info!(
                        idx = entry_idx,
                        kind = entry_kind,
                        "checker compose entry failed: iterable eval"
                    );
                    continue;
                };

                info!(
                    idx = entry_idx,
                    iterations = values.len(),
                    "checker compose for-loop resolved iterations"
                );

                for (iter_idx, value) in values.into_iter().enumerate() {
                    if iter_idx < 3 || iter_idx % 64 == 0 {
                        info!(
                            idx = entry_idx,
                            iter = iter_idx,
                            "checker compose for-loop iteration start"
                        );
                    }

                    self.scopes.push(HashMap::new());
                    self.style_scopes.push(HashMap::new());
                    self.bind(name.clone(), value);
                    if let Some((idx_name, _)) = index_name {
                        self.bind(idx_name.clone(), Value::Scalar(Sx::Lit(iter_idx as f32)));
                    }
                    let layer = self.eval_block(body, span, false, false);
                    self.scopes.pop();
                    self.style_scopes.pop();

                    let Some(layer) = layer else {
                        had_error = true;
                        info!(
                            idx = entry_idx,
                            iter = iter_idx,
                            "checker compose for-loop iteration failed"
                        );
                        continue;
                    };
                    out.push((layer, parse_blend(self, blend)));

                    if iter_idx < 3 || iter_idx % 64 == 0 {
                        info!(
                            idx = entry_idx,
                            iter = iter_idx,
                            total_layers = out.len(),
                            "checker compose for-loop iteration done"
                        );
                    }
                }
                info!(
                    idx = entry_idx,
                    kind = entry_kind,
                    total_layers = out.len(),
                    "checker compose entry done"
                );
                continue;
            }

            let Some((layer, blend_spec)) = (match e {
                ComposeEntry::Expr { expr, blend } => {
                    let Some(value) = self.eval(expr) else {
                        had_error = true;
                        info!(
                            idx = entry_idx,
                            kind = entry_kind,
                            "checker compose entry failed: expr eval"
                        );
                        continue;
                    };
                    if matches!(value, Value::Error) {
                        had_error = true;
                        info!(
                            idx = entry_idx,
                            kind = entry_kind,
                            "checker compose entry failed: expr yielded error"
                        );
                        continue;
                    }
                    let layer = match value {
                        Value::Layer(id) => id,
                        Value::TypedTextureSample {
                            layer_id,
                            tex_name,
                            sample_at,
                        } => {
                            if sample_at.is_none()
                                && !self.implicit_texture_uv_allowed(
                                    expr.span.clone(),
                                    &tex_name,
                                    "sample explicitly",
                                    format!(
                                        "use `{tex_name}.at(uv)` or `image({tex_name}, at: uv)`; staged rollout: `#pragma check.warn_implicit_texture_uv = true` for warnings or `#pragma check.allow_implicit_texture_uv = true` to suppress the diagnostic"
                                    ),
                                )
                            {
                                had_error = true;
                                continue;
                            }
                            layer_id
                        }
                        Value::Color { rgba, .. } => self.hir.layer(Layer::Solid(rgba)),
                        Value::ColorField { rgba, .. } => {
                            let [r, g, b, a] = rgba;
                            self.hir.layer(Layer::ColorExpr { r, g, b, a })
                        }
                        v => {
                            self.diags.push(
                                Diag::error(expr.span.clone(), format!("compose entries must be layers, found a {}", v.kind()))
                                    .with_help(if matches!(v, Value::Shape(_)) {
                                        "shapes need ink before they can be composed: `|> fill(...)`"
                                    } else {
                                        "wrap this in a layer-producing expression"
                                    }),
                            );
                            had_error = true;
                            info!(
                                idx = entry_idx,
                                kind = entry_kind,
                                "checker compose entry failed: non-layer value"
                            );
                            continue;
                        }
                    };
                    Some((layer, blend))
                }
                ComposeEntry::Block { body, blend, span } => {
                    self.scopes.push(HashMap::new());
                    self.style_scopes.push(HashMap::new());
                    let grouped = self.eval_block(body, span, false, false);
                    self.scopes.pop();
                    self.style_scopes.pop();

                    let Some(layer) = grouped else {
                        had_error = true;
                        info!(
                            idx = entry_idx,
                            kind = entry_kind,
                            "checker compose entry failed: grouped block"
                        );
                        continue;
                    };
                    Some((layer, blend))
                }
                ComposeEntry::If {
                    cond,
                    then_body,
                    else_body,
                    blend,
                    span,
                } => {
                    let Some(cond_sx) = self.as_scalar(cond) else {
                        self.diags.push(
                            Diag::error(
                                cond.span.clone(),
                                "compose `if` condition must be a scalar expression",
                            )
                            .with_help(
                                "use a scalar condition like `if cos(angle) { ... } else { ... }`",
                            ),
                        );
                        had_error = true;
                        continue;
                    };

                    self.scopes.push(HashMap::new());
                    self.style_scopes.push(HashMap::new());
                    let then_layer = self.eval_block(then_body, span, false, false);
                    self.scopes.pop();
                    self.style_scopes.pop();

                    let Some(then_layer) = then_layer else {
                        had_error = true;
                        info!(
                            idx = entry_idx,
                            kind = entry_kind,
                            "checker compose entry failed: if then branch"
                        );
                        continue;
                    };

                    let else_layer = if let Some(else_body) = else_body {
                        self.scopes.push(HashMap::new());
                        self.style_scopes.push(HashMap::new());
                        let else_layer = self.eval_block(else_body, span, false, false);
                        self.scopes.pop();
                        self.style_scopes.pop();
                        let Some(else_layer) = else_layer else {
                            had_error = true;
                            info!(
                                idx = entry_idx,
                                kind = entry_kind,
                                "checker compose entry failed: if else branch"
                            );
                            continue;
                        };
                        else_layer
                    } else {
                        self.hir.layer(Layer::Solid([0.0, 0.0, 0.0, 0.0]))
                    };

                    let layer = self.hir.layer(Layer::If {
                        cond: cond_sx,
                        then_layer,
                        else_layer,
                    });
                    Some((layer, blend))
                }
                ComposeEntry::InSpace {
                    chain,
                    body,
                    blend,
                    span,
                } => {
                    let resolved = self.eval_space_chain(chain);
                    self.scopes.push(HashMap::new());
                    self.style_scopes.push(HashMap::new());
                    let bindings_ok = resolved.as_ref().is_none_or(|resolved| {
                        self.bind_repeat_cells(&resolved.repeat_bindings, span)
                    });
                    let diags_before_inner = self.diags.len();
                    let inner = self.eval_block(body, span, false, true);
                    let inner_had_errors = self
                        .diags
                        .iter()
                        .skip(diags_before_inner)
                        .any(|d| d.severity == Severity::Error);
                    self.scopes.pop();
                    self.style_scopes.pop();
                    let layer = match (inner, resolved, bindings_ok) {
                        (Some(inner), Some(resolved), true) => self.hir.layer(Layer::InSpace {
                            xforms: resolved.xforms,
                            inner,
                        }),
                        _ => {
                            if inner_had_errors {
                                had_error = true;
                                info!(
                                    idx = entry_idx,
                                    kind = entry_kind,
                                    "checker compose entry failed: in-space inner"
                                );
                            }
                            continue;
                        }
                    };
                    Some((layer, blend))
                }
                ComposeEntry::For { .. } => unreachable!(),
            }) else {
                had_error = true;
                continue;
            };

            let blend = parse_blend(self, blend_spec);
            out.push((layer, blend));
            info!(
                idx = entry_idx,
                kind = entry_kind,
                total_layers = out.len(),
                "checker compose entry done"
            );
        }
        if had_error {
            info!("checker compose failed");
            return None;
        }
        if out.is_empty() {
            self.diags
                .push(Diag::error(span.clone(), "empty compose block"));
            info!("checker compose failed: empty block");
            return None;
        }
        info!(layers = out.len(), "checker compose done");
        Some(self.hir.layer(Layer::Compose(out)))
    }

    pub(super) fn eval_space_chain(&mut self, chain: &[SpaceItem]) -> Option<ResolvedSpaceChain> {
        let mut xforms = Vec::new();
        let mut repeat_bindings = Vec::new();
        for item in chain {
            match item {
                SpaceItem::Call(sc) => {
                    let mut bag = ArgBag::new(&sc.name, &sc.args, sc.name_span.clone());
                    let xf = match sc.name.as_str() {
                        "cells" => {
                            let mut cells = self.cellular_space(&mut bag)?;
                            self.hir.notes.push(format!(
                                "cells: {:?}, clipped ownership, {} samples per outermost pixel footprint; nested cells share quadrature; {} candidate sites per sample",
                                cells.layout,
                                cells.samples_axis * cells.samples_axis,
                                if matches!(cells.layout, hir::CellLayout::Hex | hir::CellLayout::Voronoi) { 25 } else { 1 },
                            ));
                            if let Some(expr) = bag.take_exact_named("cell") {
                                let Expr::Var(name) = &expr.node else {
                                    self.diags.push(Diag::error(
                                        expr.span.clone(),
                                        "cells cell expects a binding name",
                                    ));
                                    return None;
                                };
                                let scope_id = self.next_repeat_cell_scope_id();
                                cells.cell_scope = Some(scope_id);
                                repeat_bindings.push(RepeatCellBinding {
                                    name: name.clone(),
                                    scope_id,
                                    every: Some(cells.every),
                                });
                            }
                            Xform::Cellular(cells)
                        }
                        "rotate" => {
                            let a = bag.require("angle", &mut self.diags)?;
                            let angle = self.as_scalar(a)?;
                            let around = match bag.take("around") {
                                Some(v) => self.as_vec2(v)?,
                                None => (Sx::Lit(0.5), Sx::Lit(0.5)),
                            };
                            Xform::Rotate { angle, around }
                        }
                        "rotate_x" => {
                            let a = bag.require("angle", &mut self.diags)?;
                            let angle = self.as_scalar(a)?;
                            let around = match bag.take("around") {
                                Some(v) => self.as_vec2(v)?,
                                None => (Sx::Lit(0.5), Sx::Lit(0.5)),
                            };
                            Xform::RotateX { angle, around }
                        }
                        "rotate_y" => {
                            let a = bag.require("angle", &mut self.diags)?;
                            let angle = self.as_scalar(a)?;
                            let around = match bag.take("around") {
                                Some(v) => self.as_vec2(v)?,
                                None => (Sx::Lit(0.5), Sx::Lit(0.5)),
                            };
                            Xform::RotateY { angle, around }
                        }
                        "rotate_z" => {
                            let a = bag.require("angle", &mut self.diags)?;
                            let angle = self.as_scalar(a)?;
                            let around = match bag.take("around") {
                                Some(v) => self.as_vec2(v)?,
                                None => (Sx::Lit(0.5), Sx::Lit(0.5)),
                            };
                            Xform::Rotate { angle, around }
                        }
                        "translate" => {
                            let by = bag.require("by", &mut self.diags)?;
                            Xform::Translate(self.as_vec2(by)?)
                        }
                        "translate3" => {
                            let x = bag.require("x", &mut self.diags)?;
                            let y = bag.require("y", &mut self.diags)?;
                            let z = bag.require("z", &mut self.diags)?;
                            Xform::Translate3 {
                                by: (self.as_scalar(x)?, self.as_scalar(y)?),
                                z: self.as_scalar(z)?,
                            }
                        }
                        "scale" => {
                            let f = bag.require("factor", &mut self.diags)?;
                            let factor = self.as_scalar(f)?;
                            self.validate_scale_factor(&factor, sc.name_span.clone())?;
                            let around = match bag.take("around") {
                                Some(v) => self.as_vec2(v)?,
                                None => (Sx::Lit(0.5), Sx::Lit(0.5)),
                            };
                            Xform::Scale { factor, around }
                        }
                        "repeat_x" => {
                            let every = bag.require("every", &mut self.diags)?;
                            let every = self.as_scalar(every)?;
                            self.validate_repeat_every(&every, sc.name_span.clone(), "repeat_x")?;
                            Xform::RepeatX(every)
                        }
                        "repeat" => {
                            let every = bag.require("every", &mut self.diags)?;
                            let every_v = self.eval(every)?;
                            let cell_binding = bag.take_exact_named("cell");
                            let as_binding = bag.take_exact_named("as");
                            let id_binding = bag.take_exact_named("id");
                            let binding_count = usize::from(cell_binding.is_some())
                                + usize::from(as_binding.is_some())
                                + usize::from(id_binding.is_some());
                            if binding_count > 1 {
                                let conflict_span = cell_binding
                                    .as_ref()
                                    .or(id_binding.as_ref())
                                    .or(as_binding.as_ref())
                                    .expect("repeat binding conflict should have a span");
                                self.diags.push(
                                    Diag::error(
                                        conflict_span.span.clone(),
                                        "`repeat(...)` accepts at most one of `cell: ...`, `id: ...`, or `as: ...`",
                                    )
                                    .with_label("remove the extra repeat-cell binding aliases"),
                                );
                                return None;
                            }
                            let (binding_expr, binding_alias) =
                                match (cell_binding, id_binding, as_binding) {
                                    (Some(expr), None, None) => (Some(expr), Some("cell")),
                                    (None, Some(expr), None) => (Some(expr), Some("id")),
                                    (None, None, Some(expr)) => (Some(expr), Some("as")),
                                    (None, None, None) => (None, None),
                                    _ => unreachable!("binding conflicts already handled"),
                                };
                            let binding_span = binding_expr.map(|expr| expr.span.clone());
                            let binding_name = match binding_expr {
                                None => None,
                                Some(expr) => match &expr.node {
                                    Expr::Var(name) => Some(name.clone()),
                                    _ => {
                                        let binding_label = match binding_alias {
                                            Some("cell") => {
                                                "`repeat(..., cell: ...)` expects a binding name"
                                            }
                                            Some("id") => {
                                                "`repeat(..., id: ...)` expects a binding name"
                                            }
                                            Some("as") => {
                                                "`repeat(..., as: ...)` expects a binding name"
                                            }
                                            _ => "`repeat(...)` expects a binding name",
                                        };
                                        self.diags.push(
                                            Diag::error(
                                                expr.span.clone(),
                                                binding_label,
                                            )
                                            .with_help(
                                                "use an identifier like `repeat(every: 0.1, cell: tile)`",
                                            ),
                                        );
                                        return None;
                                    }
                                },
                            };
                            if let Some(alias) = binding_alias
                                && alias != "cell"
                            {
                                self.diags.push(
                                    Diag::warning(
                                        binding_span
                                            .clone()
                                            .expect("legacy repeat binding alias should have a span"),
                                        format!(
                                            "`repeat(..., {alias}: ...)` is deprecated; use `cell:`"
                                        ),
                                    )
                                    .with_help(
                                        "rename the repeat-cell binding to `cell: name` to match the bound value",
                                    ),
                                );
                            }
                            let (ex, ey) = match every_v {
                                Value::Vec2((x, y)) => (x, y),
                                Value::Scalar(sx) => (sx.clone(), sx),
                                other => {
                                    self.diags.push(
                                        Diag::error(
                                            every.span.clone(),
                                            format!(
                                                "`repeat(every: ...)` expects a scalar or vec2 period, found {}",
                                                other.kind()
                                            ),
                                        )
                                        .with_help("use `repeat(every: 0.1)` or `repeat(every: (0.1, 0.08))`")
                                    );
                                    return None;
                                }
                            };
                            self.validate_repeat_every(&ex, sc.name_span.clone(), "repeat")?;
                            self.validate_repeat_every(&ey, sc.name_span.clone(), "repeat")?;
                            let cell_scope = binding_name
                                .as_ref()
                                .map(|_| self.next_repeat_cell_scope_id());
                            if let (Some(name), Some(scope_id)) = (binding_name, cell_scope) {
                                repeat_bindings.push(RepeatCellBinding {
                                    name,
                                    scope_id,
                                    every: None,
                                });
                            }
                            bag.finish(&mut self.diags);
                            xforms.push(Xform::Repeat2D {
                                every: (ex, ey),
                                cell_scope,
                            });
                            continue;
                        }
                        "repeat_y" => {
                            let every = bag.require("every", &mut self.diags)?;
                            let every = self.as_scalar(every)?;
                            self.validate_repeat_every(&every, sc.name_span.clone(), "repeat_y")?;
                            Xform::RepeatY(every)
                        }
                        "repeat_radial" => {
                            let around = bag.require("around", &mut self.diags)?;
                            let around = self.as_vec2(around)?;

                            let (count, from, to, angles) = if let Some(angles_expr) =
                                bag.take("angles")
                            {
                                if bag.take("from").is_some() || bag.take("to").is_some() {
                                    self.diags.push(
                                        Diag::error(
                                            sc.name_span.clone(),
                                            "`repeat_radial(angles: ...)` cannot be combined with `from`/`to`",
                                        )
                                        .with_help(
                                            "use either `angles: ...` or `count: ..., from: ..., to: ...`",
                                        ),
                                    );
                                    return None;
                                }

                                match self.require_number_source(
                                    angles_expr,
                                    "`repeat_radial(angles: ...)`",
                                )? {
                                    NumberSource::Range { lo, hi, .. } => {
                                        let count_expr = bag.require("count", &mut self.diags)?;
                                        let count = self.as_scalar(count_expr)?;
                                        self.validate_repeat_radial_count(
                                            &count,
                                            sc.name_span.clone(),
                                        )?;
                                        self.validate_repeat_radial_span(
                                            &lo,
                                            &hi,
                                            sc.name_span.clone(),
                                        )?;
                                        (count, lo, hi, None)
                                    }
                                    NumberSource::List { items, span } => {
                                        if bag.take("count").is_some() {
                                            self.diags.push(
                                                Diag::error(
                                                    sc.name_span.clone(),
                                                    "`repeat_radial(angles: [...])` does not accept `count`",
                                                )
                                                .with_help(
                                                    "remove `count` and let sectors be derived from the angle list",
                                                ),
                                            );
                                            return None;
                                        }
                                        if items.len() < 3 {
                                            self.diags.push(
                                                Diag::error(
                                                    span,
                                                    "`repeat_radial(angles: [...])` requires at least 3 angles",
                                                )
                                                .with_help(
                                                    "provide a sequence such as `angles: [-90deg, -45deg, 0deg, 45deg, 90deg]`",
                                                ),
                                            );
                                            return None;
                                        }

                                        let mut values = Vec::with_capacity(items.len());
                                        for angle in &items {
                                            let Some(v) = Checker::try_eval_static_scalar(angle)
                                            else {
                                                self.diags.push(
                                                    Diag::error(
                                                        sc.name_span.clone(),
                                                        "`repeat_radial(angles: [...])` requires compile-time scalar angles",
                                                    )
                                                    .with_help(
                                                        "use literal angles (for example `-90deg`, `0deg`, `90deg`) in the list",
                                                    ),
                                                );
                                                return None;
                                            };
                                            values.push(v);
                                        }

                                        for pair in values.windows(2) {
                                            let step = pair[1] - pair[0];
                                            if step.abs() <= f32::EPSILON {
                                                self.diags.push(
                                                    Diag::error(
                                                        sc.name_span.clone(),
                                                        "`repeat_radial(angles: [...])` requires distinct neighboring angles",
                                                    )
                                                    .with_help("ensure each adjacent angle differs"),
                                                );
                                                return None;
                                            }
                                        }

                                        let increasing =
                                            values.windows(2).all(|pair| pair[1] > pair[0]);
                                        if !increasing {
                                            self.diags.push(
                                                Diag::error(
                                                    sc.name_span.clone(),
                                                    "`repeat_radial(angles: [...])` currently requires strictly increasing angles",
                                                )
                                                .with_help(
                                                    "author angles in ascending order, for example `[-90deg, -20deg, 0deg, 90deg]`",
                                                ),
                                            );
                                            return None;
                                        }

                                        let count = Sx::Lit((values.len() - 1) as f32);
                                        let from = Sx::Lit(values[0]);
                                        let to = Sx::Lit(values[values.len() - 1]);
                                        self.validate_repeat_radial_count(
                                            &count,
                                            sc.name_span.clone(),
                                        )?;
                                        self.validate_repeat_radial_span(
                                            &from,
                                            &to,
                                            sc.name_span.clone(),
                                        )?;
                                        self.hir.notes.push(
                                            "space: repeat_radial angles list lowered with explicit non-uniform sector boundaries in v0"
                                                .to_string(),
                                        );
                                        (count, from, to, Some(values))
                                    }
                                }
                            } else {
                                let count = bag.require("count", &mut self.diags)?;
                                let from = bag.require("from", &mut self.diags)?;
                                let to = bag.require("to", &mut self.diags)?;

                                let count = self.as_scalar(count)?;
                                self.validate_repeat_radial_count(&count, sc.name_span.clone())?;

                                let from = self.as_scalar(from)?;
                                let to = self.as_scalar(to)?;
                                self.validate_repeat_radial_span(&from, &to, sc.name_span.clone())?;
                                (count, from, to, None)
                            };

                            Xform::RepeatRadial {
                                count,
                                around,
                                from,
                                to,
                                angles,
                            }
                        }
                        "aspect" => {
                            let ratio = bag.require("ratio", &mut self.diags)?;
                            let ratio = self.as_scalar(ratio)?;
                            self.validate_aspect_ratio(&ratio, sc.name_span.clone())?;
                            Xform::Aspect(ratio)
                        }
                        "centered" => {
                            let mode = match bag.take("aspect") {
                                None => CenteredMode::Preserve,
                                Some(v) => match &v.node {
                                    Expr::Var(name) if name == "preserve" => CenteredMode::Preserve,
                                    Expr::Var(name) if name == "fit" => CenteredMode::Fit,
                                    Expr::Var(name) if name == "fill" => CenteredMode::Fill,
                                    _ => {
                                        self.diags.push(
                                            Diag::error(
                                                v.span.clone(),
                                                "`centered(aspect: ...)` expects `preserve`, `fit`, or `fill`",
                                            )
                                            .with_help(
                                                "use `centered(aspect: preserve|fit|fill)` or `aspect(ratio: <number>)`",
                                            ),
                                        );
                                        return None;
                                    }
                                },
                            };
                            self.hir.notes.push(match mode {
                                CenteredMode::Preserve => {
                                    "space: centered(aspect: preserve) keeps authored framing visible (letter/pillarbox as needed)".to_string()
                                }
                                CenteredMode::Fit => {
                                    "space: centered(aspect: fit) scales to fit entirely within viewport bounds".to_string()
                                }
                                CenteredMode::Fill => {
                                    "space: centered(aspect: fill) scales to fill viewport, allowing edge crop".to_string()
                                }
                            });
                            Xform::Centered { mode }
                        }
                        "orientation" => {
                            let y_expr = bag.require("y", &mut self.diags)?;
                            let y_name = self.resolve_enum_symbol_name(y_expr, "orientation")?;
                            let y = match y_name.as_str() {
                                "up" => VerticalAxis::Up,
                                "down" => VerticalAxis::Down,
                                _ => {
                                    self.diags.push(
                                        Diag::error(
                                            y_expr.span.clone(),
                                            "`orientation(y: ...)` expects `up` or `down`",
                                        )
                                        .with_help(
                                            "use `orientation(y: up|down)` or an enum variant like `Axis.down`",
                                        ),
                                    );
                                    return None;
                                }
                            };
                            self.hir.notes.push(match y {
                                VerticalAxis::Up => {
                                    "space: orientation(y: up) uses canonical Fresco y-up coordinates"
                                        .to_string()
                                }
                                VerticalAxis::Down => {
                                    "space: orientation(y: down) flips vertical axis for top-left style coordinates".to_string()
                                }
                            });
                            Xform::Orientation { y }
                        }
                        "polar" => {
                            let center = bag.require("center", &mut self.diags)?;
                            let from = bag.require("from", &mut self.diags)?;
                            let direction = bag.require("direction", &mut self.diags)?;
                            let clockwise = self.parse_polar_direction(direction)?;
                            Xform::Polar {
                                center: self.as_vec2(center)?,
                                from: self.as_scalar(from)?,
                                clockwise,
                            }
                        }
                        "warp" => {
                            let by = bag.require("by", &mut self.diags)?;
                            self.hir.notes.push(
                                "space: warp(by: ...) lowers as inverse point-local displacement in sampling space"
                                    .to_string(),
                            );
                            Xform::Warp {
                                by: self.as_vec2(by)?,
                            }
                        }
                        "perspective" => {
                            let fov = bag.require("fov", &mut self.diags)?;
                            let near = bag.require("near", &mut self.diags)?;
                            let far = bag.require("far", &mut self.diags)?;
                            let origin = bag.require("origin", &mut self.diags)?;

                            let fov = self.as_scalar(fov)?;
                            let near = self.as_scalar(near)?;
                            let far = self.as_scalar(far)?;
                            let origin = self.as_vec2(origin)?;

                            self.validate_perspective_fov(&fov, sc.name_span.clone())?;
                            self.validate_perspective_planes(&near, &far, sc.name_span.clone())?;

                            self.hir.notes.push(
                                "space: perspective(...) is flat-only in v1 (projective warp, painter-order composition)"
                                    .to_string(),
                            );

                            Xform::Perspective {
                                fov,
                                near,
                                far,
                                origin,
                            }
                        }
                        other => {
                            self.diags.push(
                                Diag::error(sc.name_span.clone(), format!("unknown space transform `{other}`"))
                                    .with_help("v0 spaces: rotate(angle, around?), rotate_x(angle, around?), rotate_y(angle, around?), rotate_z(angle, around?), translate(by), translate3(x, y, z), scale(factor), repeat(every), repeat_x(every), repeat_y(every), repeat_radial(count, around, from, to), repeat_radial(around, angles), aspect(ratio), centered(aspect: preserve), orientation(y: up|down), polar(center, from, direction), warp(by), perspective(fov, near, far, origin)"),
                            );
                            return None;
                        }
                    };
                    bag.finish(&mut self.diags);
                    xforms.push(xf);
                }
                SpaceItem::Ref { name, name_span } => match self.lookup(name) {
                    Some(Value::Space(saved)) => xforms.extend(saved),
                    Some(v) => {
                        self.diags.push(
                            Diag::error(name_span.clone(), format!("`{name}` is a {}, not a space", v.kind()))
                                .with_help("declare named spaces as `space name = rotate(...) . translate(...)`"),
                        );
                        return None;
                    }
                    None => {
                        self.diags.push(
                            Diag::error(name_span.clone(), format!("unknown space `{name}`"))
                                .with_help("declare it first: `space name = ...`"),
                        );
                        return None;
                    }
                },
            }
        }
        if xforms
            .iter()
            .enumerate()
            .any(|(index, xf)| matches!(xf, Xform::Cellular(_)) && index + 1 != xforms.len())
        {
            let span = match chain.last().expect("a cellular chain is nonempty") {
                SpaceItem::Call(call) => call.name_span.clone(),
                SpaceItem::Ref { name_span, .. } => name_span.clone(),
            };
            self.diags.push(Diag::error(span, "cells must be the final transform in a space chain")
                .with_help("nest a new `in space` block inside the cellular body for subsequent transforms"));
            return None;
        }
        Some(ResolvedSpaceChain {
            xforms,
            repeat_bindings,
        })
    }

    fn parse_polar_direction(&mut self, e: &SExpr) -> Option<bool> {
        let direction = self.resolve_enum_symbol_name(e, "polar direction")?;
        match direction.as_str() {
            "clockwise" => Some(true),
            "counterclockwise" => Some(false),
            _ => {
                self.diags.push(
                    Diag::error(
                        e.span.clone(),
                        "`polar` direction must be `clockwise` or `counterclockwise`",
                    )
                    .with_help(
                        "example: `polar(center: center, from: -90deg, direction: clockwise)` or enum variant `Direction.clockwise`",
                    ),
                );
                None
            }
        }
    }

    fn resolve_enum_symbol_name(&mut self, e: &SExpr, context: &str) -> Option<String> {
        let Expr::Var(name) = &e.node else {
            self.diags.push(
                Diag::error(
                    e.span.clone(),
                    format!("`{context}` expects an identifier or enum variant"),
                )
                .with_help("use forms like `up` or `Axis.down`"),
            );
            return None;
        };

        if let Some((enum_name, variant_name)) = name.split_once('.') {
            if variant_name.contains('.') {
                self.diags.push(
                    Diag::error(
                        e.span.clone(),
                        format!("invalid enum variant path `{name}`"),
                    )
                    .with_help("use `<Enum>.<variant>`, for example `Axis.down`"),
                );
                return None;
            }

            let Some(enum_def) = self.enum_defs.get(enum_name) else {
                self.diags.push(
                    Diag::error(e.span.clone(), format!("unknown enum `{enum_name}`"))
                        .with_help("declare the enum before using it"),
                );
                return None;
            };

            if !enum_def.variants.contains_key(variant_name) {
                self.diags.push(
                    Diag::error(
                        e.span.clone(),
                        format!("unknown variant `{variant_name}` on enum `{enum_name}`"),
                    )
                    .with_help("use a declared enum variant name"),
                );
                return None;
            }
            return Some(variant_name.to_string());
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
        if owners.len() > 1 {
            self.diags.push(
                Diag::error(e.span.clone(), format!("ambiguous enum variant `{name}`")).with_help(
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
            return None;
        }

        Some(name.clone())
    }

    fn validate_repeat_every(&mut self, every: &Sx, span: Span, name: &str) -> Option<()> {
        match every {
            Sx::Lit(v) if *v <= 0.0 => {
                self.diags.push(
                    Diag::error(span, format!("`{name}` period must be > 0")).with_help(format!(
                        "use a positive value, for example `{name}(every: 1/12)`"
                    )),
                );
                None
            }
            Sx::PxLit(v) if *v <= 0.0 => {
                self.diags.push(
                    Diag::error(span, format!("`{name}` period must be > 0")).with_help(format!(
                        "use a positive value, for example `{name}(every: 1/12)`"
                    )),
                );
                None
            }
            _ => Some(()),
        }
    }

    fn validate_aspect_ratio(&mut self, ratio: &Sx, span: Span) -> Option<()> {
        match ratio {
            Sx::Lit(v) if *v <= 0.0 => {
                self.diags.push(
                    Diag::error(span, "`aspect` ratio must be > 0")
                        .with_help("use a positive ratio, for example `aspect(ratio: 16/9)`"),
                );
                None
            }
            Sx::PxLit(v) if *v <= 0.0 => {
                self.diags.push(
                    Diag::error(span, "`aspect` ratio must be > 0")
                        .with_help("use a positive ratio, for example `aspect(ratio: 16/9)`"),
                );
                None
            }
            _ => Some(()),
        }
    }

    fn validate_scale_factor(&mut self, factor: &Sx, span: Span) -> Option<()> {
        match factor {
            Sx::Lit(v) if v.abs() <= f32::EPSILON => {
                self.diags.push(
                    Diag::error(span, "`scale` factor must be non-zero")
                        .with_help("use a non-zero scale factor, for example `scale(factor: 1.2)`"),
                );
                None
            }
            Sx::PxLit(v) if v.abs() <= f32::EPSILON => {
                self.diags.push(
                    Diag::error(span, "`scale` factor must be non-zero")
                        .with_help("use a non-zero scale factor, for example `scale(factor: 1.2)`"),
                );
                None
            }
            _ => Some(()),
        }
    }

    fn validate_repeat_radial_count(&mut self, count: &Sx, span: Span) -> Option<()> {
        match count {
            Sx::Lit(v) if *v < 2.0 => {
                self.diags.push(
                    Diag::error(span, "`repeat_radial` count must be >= 2").with_help(
                        "use at least 2 sectors, for example `repeat_radial(count: 8, ...)`",
                    ),
                );
                None
            }
            Sx::PxLit(v) if *v < 2.0 => {
                self.diags.push(
                    Diag::error(span, "`repeat_radial` count must be >= 2").with_help(
                        "use at least 2 sectors, for example `repeat_radial(count: 8, ...)`",
                    ),
                );
                None
            }
            _ => Some(()),
        }
    }

    fn validate_repeat_radial_span(&mut self, from: &Sx, to: &Sx, span: Span) -> Option<()> {
        match (from, to) {
            (Sx::Lit(a), Sx::Lit(b)) if (*b - *a).abs() <= f32::EPSILON => {
                self.diags.push(
                    Diag::error(span, "`repeat_radial` requires `to` and `from` to differ")
                        .with_help(
                            "use a non-zero angular span, for example `from: -90deg, to: 90deg`",
                        ),
                );
                None
            }
            _ => Some(()),
        }
    }

    fn validate_perspective_fov(&mut self, fov: &Sx, span: Span) -> Option<()> {
        match fov {
            Sx::Lit(v) if *v <= 0.0 || *v >= std::f32::consts::PI => {
                self.diags.push(
                    Diag::error(span, "`perspective` fov must be in (0, PI) radians")
                        .with_help("use a typical value like `45deg` or `60deg`"),
                );
                None
            }
            _ => Some(()),
        }
    }

    fn validate_perspective_planes(&mut self, near: &Sx, far: &Sx, span: Span) -> Option<()> {
        match (near, far) {
            (Sx::Lit(n), Sx::Lit(_f)) if *n <= 0.0 => {
                self.diags.push(
                    Diag::error(span, "`perspective` near plane must be > 0")
                        .with_help("use a small positive value, for example `near: 0.01`"),
                );
                None
            }
            (Sx::Lit(n), Sx::Lit(f)) if *f <= *n => {
                self.diags.push(
                    Diag::error(span, "`perspective` far plane must be greater than near")
                        .with_help("use values like `near: 0.01, far: 100.0`"),
                );
                None
            }
            _ => Some(()),
        }
    }
}

// ─── Local fn declaration registration ───────────────────────────────────────

impl Checker {
    /// Register a `fn` declaration encountered as a statement inside a canvas or fn body.
    ///
    /// Local fns are resolved with the same rules as top-level fns: they go into
    /// `self.fn_defs` and are available from the point of declaration forward (sequential
    /// evaluation naturally enforces this for inline-evaluated fn bodies).
    pub(super) fn register_local_fn_decl(&mut self, fn_decl: &FnDecl) {
        let incoming_is_stdlib = fn_decl.source_file.starts_with("<stdlib:");
        let type_param_names: Vec<String> = fn_decl
            .type_params
            .iter()
            .map(|param| param.name.clone())
            .collect();
        let type_params: Vec<(String, Vec<String>)> = fn_decl
            .type_params
            .iter()
            .map(|param| (param.name.clone(), param.bounds.clone()))
            .collect();

        let Some(ret) = fn_decl.ret_ty.as_ref().and_then(|(name, span)| {
            parse_local_fn_type(
                name,
                span,
                &mut self.diags,
                "return type",
                None,
                &self.enum_defs,
                &self.struct_defs,
                &type_param_names,
                &self.interface_names,
            )
        }) else {
            self.diags.push(
                    Diag::error(
                        fn_decl.name_span.clone(),
                        format!(
                            "local function `{}` must declare a return type",
                            fn_decl.name
                        ),
                    )
                    .with_help(
                        "supported return types: f32, vec2, vec3, color, shape, layer, or a defined enum",
                    ),
                );
            return;
        };

        let mut params = Vec::with_capacity(fn_decl.params.len());
        let mut bad_param = false;
        let mut seen_param_names: HashMap<String, Span> = HashMap::new();

        for p in &fn_decl.params {
            let Some(ty) = parse_local_fn_type(
                &p.ty_name,
                &p.ty_span,
                &mut self.diags,
                "parameter type",
                None,
                &self.enum_defs,
                &self.struct_defs,
                &type_param_names,
                &self.interface_names,
            ) else {
                bad_param = true;
                continue;
            };

            if seen_param_names.contains_key(&p.name) {
                self.diags.push(
                    Diag::error(
                        p.name_span.clone(),
                        format!(
                            "duplicate parameter `{}` in function `{}`",
                            p.name, fn_decl.name
                        ),
                    )
                    .with_label("parameter name redefined")
                    .with_help(format!(
                        "rename this parameter; `{}` was already declared earlier",
                        p.name
                    )),
                );
                bad_param = true;
                continue;
            }
            seen_param_names.insert(p.name.clone(), p.name_span.clone());

            params.push(FnParamDef {
                is_context: false,
                name: p.name.clone(),
                ty,
                scalar_kind: crate::typed_scalar::Kind::element(&p.ty_name)
                    .unwrap_or(crate::typed_scalar::Kind::F32),
                scalar_specialization: scalar_specialization_from_type_name(&p.ty_name),
                keyword_only: p.keyword_only,
            });
        }

        if bad_param {
            return;
        }

        let def = FnDef {
            infer_return: false,
            ret_kind: fn_decl
                .ret_ty
                .as_ref()
                .and_then(|(ty, _)| crate::typed_scalar::Kind::element(ty))
                .unwrap_or(crate::typed_scalar::Kind::F32),
            params,
            ret,
            is_internal: false,
            is_builtin: false,
            source_file: fn_decl.source_file.clone(),
            body: fn_decl.body.clone(),
            span: fn_decl.span.clone(),
            type_params,
            const_params: Vec::new(),
            const_bindings: Vec::new(),
        };

        let entry = self.fn_defs.entry(fn_decl.name.clone()).or_default();

        if !incoming_is_stdlib && entry.iter().all(|e| e.source_file.starts_with("<stdlib:")) {
            entry.clear();
        }

        if entry.iter().any(|e| e.same_overload_signature(&def)) {
            // If the existing entry has the same span it's a re-registration of the same local fn
            // (the body is being evaluated multiple times across call sites). Silently update it
            // so callers always see the latest def, without producing a spurious duplicate error.
            if entry
                .iter()
                .any(|e| e.same_overload_signature(&def) && e.span == def.span)
            {
                // Replace the existing entry so it stays current.
                if let Some(slot) = entry.iter_mut().find(|e| e.same_overload_signature(&def)) {
                    *slot = def;
                }
                return;
            }
            self.diags.push(
                Diag::error(
                    fn_decl.name_span.clone(),
                    format!(
                        "duplicate function declaration `{}` with matching overload signature",
                        fn_decl.name,
                    ),
                )
                .with_help("function overloads must differ by parameter type specialization, arity, or keyword-only placement"),
            );
            return;
        }

        entry.push(def);
    }
}

/// Parse a type name for a local fn declaration parameter or return type.
/// Handles simple types and canonical callable type strings (`fn(T1,T2)->T`).
#[expect(
    clippy::too_many_arguments,
    reason = "This compiler boundary explicitly threads independent checking or lowering inputs."
)]
fn parse_local_fn_type(
    name: &str,
    span: &Span,
    diags: &mut Vec<Diag>,
    context: &str,
    source_file: Option<&str>,
    enum_defs: &HashMap<String, EnumDef>,
    struct_defs: &HashMap<String, StructDef>,
    type_params: &[String],
    interface_names: &HashSet<String>,
) -> Option<FnValueTy> {
    let name = strip_spatial_type_suffix(name);
    if let Some(resource) = crate::resource_type::shader_resource_type(name) {
        return match resource {
            Ok(resource) => Some(FnValueTy::ShaderResource(resource)),
            Err(message) => {
                let mut diagnostic = Diag::error(span.clone(), message);
                if let Some(file) = source_file {
                    diagnostic = diagnostic.with_file(file);
                }
                diags.push(diagnostic);
                None
            }
        };
    }
    if let Some((element, length)) = hir::parse_array_param_type(name) {
        if length == 0 {
            diags.push(Diag::error(
                span.clone(),
                "function array types require a nonzero length",
            ));
            return None;
        }
        parse_local_fn_type(
            element,
            span,
            diags,
            "array element type",
            source_file,
            enum_defs,
            struct_defs,
            &[],
            interface_names,
        )?;
        return Some(FnValueTy::Array(
            name.chars().filter(|c| !c.is_whitespace()).collect(),
        ));
    }

    if let Some(callable) = try_parse_callable_ty(name) {
        let param_tys: Option<Vec<FnValueTy>> = callable
            .0
            .iter()
            .map(|p| {
                parse_local_fn_type(
                    p,
                    span,
                    diags,
                    "callable parameter type",
                    source_file,
                    enum_defs,
                    struct_defs,
                    type_params,
                    interface_names,
                )
            })
            .collect();
        let param_tys = param_tys?;
        let ret_ty = Box::new(parse_local_fn_type(
            &callable.1,
            span,
            diags,
            "callable return type",
            source_file,
            enum_defs,
            struct_defs,
            type_params,
            interface_names,
        )?);
        return Some(FnValueTy::Callable {
            params: param_tys,
            ret: ret_ty,
        });
    }

    if let Some((base, args)) = try_parse_type_application(name)
        && base == "field"
    {
        if args.len() != 1 {
            let mut d = Diag::error(
                span.clone(),
                format!(
                    "`field<...>` expects exactly one type argument, found {}",
                    args.len()
                ),
            )
            .with_help("use `field<T>` with a single element type parameter");
            if let Some(file) = source_file {
                d = d.with_file(file);
            }
            diags.push(d);
            return None;
        }
        return parse_local_fn_type(
            &args[0],
            span,
            diags,
            context,
            source_file,
            enum_defs,
            struct_defs,
            type_params,
            interface_names,
        );
    }

    if let Some((base, _args)) = try_parse_type_application(name)
        && base == "texture"
    {
        return Some(FnValueTy::Texture);
    }

    match name {
        "f32" | "f64" | "half" | "i32" | "u32" | "signal" | "mask" | "coverage" | "delta"
        | "bool" | "angle" | "length" => Some(FnValueTy::Scalar),
        "vec2" | "uvec2" | "ivec2" | "bvec2" | "coord" | "resolution" => Some(FnValueTy::Vec2),
        "vec3" | "uvec3" | "ivec3" | "bvec3" => Some(FnValueTy::Vec3),
        "vec4" | "uvec4" | "ivec4" | "bvec4" => Some(FnValueTy::Vec4),
        "mat2" => Some(FnValueTy::Mat2),
        "mat3" => Some(FnValueTy::Mat3),
        "mat4" => Some(FnValueTy::Mat4),
        "coord_like" => Some(FnValueTy::CoordLike),
        "color" => Some(FnValueTy::Color),
        "shape" => Some(FnValueTy::Shape),
        "layer" => Some(FnValueTy::Layer),
        other => {
            if type_params.iter().any(|param| param == other) {
                return Some(FnValueTy::TypeVar(other.to_string()));
            }
            if interface_names.contains(other) {
                let mut d = Diag::error(
                    span.clone(),
                    format!("interface `{other}` cannot be used as a value type in v1"),
                )
                .with_help(format!(
                    "use a generic type parameter with an interface bound instead, e.g. `<T: {other}>`"
                ));
                if let Some(file) = source_file {
                    d = d.with_file(file);
                }
                diags.push(d);
                return None;
            }
            if enum_defs.contains_key(other) {
                return Some(FnValueTy::Enum(other.to_string()));
            }
            if struct_defs.contains_key(other) {
                return Some(FnValueTy::Struct(other.to_string()));
            }
            let mut d = Diag::error(
                span.clone(),
                format!("unsupported function {context} `{other}`"),
            )
            .with_help(
                "supported types are family-first: scalar aliases (f32/f64/half/i32/u32/bool/signal/delta/mask/coverage/angle/length), vector aliases (vec2/uvec2/coord/coord_like/resolution, vec3/uvec3, vec4/uvec4), matrix aliases (mat2/mat3/mat4), texture<T>, color, shape, layer, fixed-size arrays, records, or a defined enum",
            );
            if let Some(file) = source_file {
                d = d.with_file(file);
            }
            diags.push(d);
            None
        }
    }
}

fn strip_spatial_type_suffix(name: &str) -> &str {
    let trimmed = name.trim();

    if let Some((base, suffix)) = trimmed.rsplit_once(" in ")
        && !base.trim().is_empty()
        && !suffix.trim().is_empty()
    {
        return base.trim();
    }

    if let Some((base, suffix)) = trimmed.rsplit_once(" from ")
        && !base.trim().is_empty()
        && !suffix.trim().is_empty()
        && let Some((_src, dst)) = suffix.rsplit_once(" to ")
        && !dst.trim().is_empty()
    {
        return base.trim();
    }

    trimmed
}

#[cfg(test)]
mod numeric_tests {
    use super::ScalarBounds;

    #[test]
    fn bounds_midpoint_stays_finite_near_float_limits() {
        for value in [f32::MAX, -f32::MAX] {
            let bounds = ScalarBounds::from_values(&[value, value]);
            assert_eq!(bounds.midpoint(), value);
        }
        assert_eq!(
            ScalarBounds::from_values(&[-f32::MAX, f32::MAX]).midpoint(),
            0.0
        );
        let subnormal = f32::from_bits(1);
        assert_eq!(
            ScalarBounds::from_values(&[subnormal, subnormal]).midpoint(),
            subnormal
        );
    }
}
