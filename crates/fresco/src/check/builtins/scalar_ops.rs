//! Scalar operation builtins.

use crate::ast::Expr;
use crate::builtin;
use crate::check::{ArgBag, Checker, Value};
use crate::diag::Diag;
use crate::hir::Sx;
use crate::registry::{
    BuiltinArgDecl, BuiltinDecl, BuiltinId, BuiltinLowering, BuiltinSignature, PrimitiveType,
    TypeId, TypeRef,
};

fn sx_hash2(ix: Sx, iy: Sx) -> Sx {
    let dot = Sx::Add(
        Box::new(Sx::Mul(Box::new(ix), Box::new(Sx::Lit(127.1)))),
        Box::new(Sx::Mul(Box::new(iy), Box::new(Sx::Lit(311.7)))),
    );
    let n = Sx::Mul(
        Box::new(Sx::Sin(Box::new(dot))),
        Box::new(Sx::Lit(43_758.547)),
    );
    Sx::Fract(Box::new(n))
}

fn sx_worley2(px: Sx, py: Sx) -> Sx {
    let ix = Sx::Floor(Box::new(px.clone()));
    let iy = Sx::Floor(Box::new(py.clone()));
    let fx = Sx::Fract(Box::new(px));
    let fy = Sx::Fract(Box::new(py));

    let mut best: Option<Sx> = None;
    for ox in -1..=1 {
        for oy in -1..=1 {
            let cell_x = Sx::Add(Box::new(ix.clone()), Box::new(Sx::Lit(ox as f32)));
            let cell_y = Sx::Add(Box::new(iy.clone()), Box::new(Sx::Lit(oy as f32)));

            let jx = sx_hash2(cell_x.clone(), cell_y.clone());
            let jy = sx_hash2(
                Sx::Add(Box::new(cell_x.clone()), Box::new(Sx::Lit(19.19))),
                Sx::Add(Box::new(cell_y.clone()), Box::new(Sx::Lit(73.73))),
            );

            let px = Sx::Add(Box::new(Sx::Lit(ox as f32)), Box::new(jx));
            let py = Sx::Add(Box::new(Sx::Lit(oy as f32)), Box::new(jy));
            let dx = Sx::Sub(Box::new(px), Box::new(fx.clone()));
            let dy = Sx::Sub(Box::new(py), Box::new(fy.clone()));
            let dist2 = Sx::Add(
                Box::new(Sx::Mul(Box::new(dx.clone()), Box::new(dx))),
                Box::new(Sx::Mul(Box::new(dy.clone()), Box::new(dy))),
            );

            best = Some(match best {
                Some(prev) => Sx::Min(Box::new(prev), Box::new(dist2)),
                None => dist2,
            });
        }
    }

    Sx::Sqrt(Box::new(best.expect("worley neighborhood cannot be empty")))
}

fn sx_curvature_from_normal(nx: Sx, ny: Sx, nz: Sx) -> Sx {
    let ddx_nx = Sx::Ddx(Box::new(nx.clone()));
    let ddx_ny = Sx::Ddx(Box::new(ny.clone()));
    let ddx_nz = Sx::Ddx(Box::new(nz.clone()));
    let ddy_nx = Sx::Ddy(Box::new(nx));
    let ddy_ny = Sx::Ddy(Box::new(ny));
    let ddy_nz = Sx::Ddy(Box::new(nz));

    let ddx_sq = Sx::Add(
        Box::new(Sx::Mul(Box::new(ddx_nx.clone()), Box::new(ddx_nx))),
        Box::new(Sx::Add(
            Box::new(Sx::Mul(Box::new(ddx_ny.clone()), Box::new(ddx_ny))),
            Box::new(Sx::Mul(Box::new(ddx_nz.clone()), Box::new(ddx_nz))),
        )),
    );
    let ddy_sq = Sx::Add(
        Box::new(Sx::Mul(Box::new(ddy_nx.clone()), Box::new(ddy_nx))),
        Box::new(Sx::Add(
            Box::new(Sx::Mul(Box::new(ddy_ny.clone()), Box::new(ddy_ny))),
            Box::new(Sx::Mul(Box::new(ddy_nz.clone()), Box::new(ddy_nz))),
        )),
    );

    Sx::Sqrt(Box::new(Sx::Add(Box::new(ddx_sq), Box::new(ddy_sq))))
}

builtin! {
    name = "wrap",
    signature = single {
        args(
            x: Scalar = "input value",
            range: Expr @range = "target numeric source as `lo .. hi` or `[lo, hi]`"
        ),
        result = Scalar,
        caps = PURE,
    },
    check = |ctx, x, range| {
        let (lo, hi) = ctx.require_range_pair(range, "`wrap(x, range: ...)`")?;

        let width = Sx::Sub(Box::new(hi), Box::new(lo.clone()));
        let shifted = Sx::Sub(Box::new(x), Box::new(lo.clone()));
        let u = Sx::Div(Box::new(shifted), Box::new(width.clone()));
        let w = Sx::Fract(Box::new(u));
        let out = Sx::Add(
            Box::new(lo),
            Box::new(Sx::Mul(Box::new(width), Box::new(w))),
        );
        Value::Scalar(out)
    }
}

builtin! {
    name = "len",
    signature = single {
        args(
            items: Expr = "array value"
        ),
        result = Scalar,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, items| {
        let value = ctx.eval(items).unwrap_or(Value::Error);
        match value {
            Value::Array(items) => Value::Scalar(Sx::Lit(items.len() as f32)),
            Value::Error => Value::Error,
            other => {
                ctx.diags.push(
                    Diag::error(
                        items.span.clone(),
                        format!("`len` expects an array value, found {}", other.kind()),
                    )
                    .with_help("use `len([a, b, c])` or `values |> len()`"),
                );
                Value::Error
            }
        }
    }
}

pub fn distribute_check_impl(ctx: &mut Checker, bag: &mut ArgBag<'_>) -> Option<Value> {
    let index_expr = bag.require("index", &mut ctx.diags)?;
    let count_expr = bag.require("count", &mut ctx.diags)?;
    let start_expr = bag.take_named("start");
    let end_expr = bag.take_named("end");
    let width_expr = bag.take_named("width");
    let gap_expr = bag.take_named("gap");
    let anchor_expr = bag.take_named("anchor");

    let uses_spacing_mode = width_expr.is_some() || gap_expr.is_some() || anchor_expr.is_some();
    let uses_bounds_mode = start_expr.is_some() || end_expr.is_some();

    if uses_spacing_mode && uses_bounds_mode {
        ctx.diags.push(
            Diag::error(
                bag.call_span().clone(),
                "`distribute` cannot mix `start/end` with `width/gap/anchor`",
            )
            .with_help("use either `start/end` bounds mode or `width/gap/anchor` spacing mode"),
        );
        return Some(Value::Error);
    }

    let index_value = ctx.eval(index_expr)?;
    let count_value = ctx.eval(count_expr)?;

    let width_value = if uses_spacing_mode {
        match width_expr {
            Some(expr) => ctx.eval(expr)?,
            None => {
                ctx.diags.push(
                    Diag::error(
                        bag.call_span().clone(),
                        "`distribute` spacing mode requires `width`",
                    )
                    .with_help("use `distribute(i, count, width: ..., gap?: ..., anchor?: ...)`"),
                );
                return Some(Value::Error);
            }
        }
    } else {
        Value::Error
    };

    let gap_value = if uses_spacing_mode {
        match gap_expr {
            Some(expr) => ctx.eval(expr)?,
            None => Value::Scalar(Sx::Lit(0.0)),
        }
    } else {
        Value::Error
    };

    let anchor_value = if uses_spacing_mode {
        match anchor_expr {
            Some(expr) => ctx.eval(expr)?,
            None => Value::Scalar(Sx::Lit(0.5)),
        }
    } else {
        Value::Error
    };

    let start_value = if uses_spacing_mode {
        Value::Error
    } else {
        match start_expr {
            Some(expr) => ctx.eval(expr)?,
            None => Value::Scalar(Sx::Lit(0.0)),
        }
    };

    let end_value = if uses_spacing_mode {
        Value::Error
    } else {
        match end_expr {
            Some(expr) => ctx.eval(expr)?,
            None => Value::Scalar(Sx::Lit(1.0)),
        }
    };

    let Some((index_sx, _)) = Checker::as_numeric_scalar(&index_value) else {
        if !matches!(index_value, Value::Error) {
            ctx.diags.push(
                Diag::error(
                    index_expr.span.clone(),
                    format!(
                        "`distribute` index must be scalar, found {}",
                        index_value.kind()
                    ),
                )
                .with_help("use `distribute(i, count, start: ..., end: ...)` with numeric values"),
            );
        }
        return Some(Value::Error);
    };

    let Some((count_sx, _)) = Checker::as_numeric_scalar(&count_value) else {
        if !matches!(count_value, Value::Error) {
            ctx.diags.push(
                Diag::error(
                    count_expr.span.clone(),
                    format!(
                        "`distribute` count must be scalar, found {}",
                        count_value.kind()
                    ),
                )
                .with_help("use `len(values)` or another numeric expression for `count`"),
            );
        }
        return Some(Value::Error);
    };

    if let Some(count_n) = Checker::try_eval_static_scalar(&count_sx) {
        if count_n <= 0.0 {
            ctx.diags.push(
                Diag::error(
                    count_expr.span.clone(),
                    "`distribute` count must be greater than zero",
                )
                .with_help("use a positive item count, for example `len(values)`"),
            );
            return Some(Value::Error);
        }

        if let Some(index_n) = Checker::try_eval_static_scalar(&index_sx)
            && (index_n < 0.0 || index_n >= count_n)
        {
            ctx.diags.push(
                Diag::warning(
                    index_expr.span.clone(),
                    format!("`distribute` index {index_n} is outside count {count_n}"),
                )
                .with_help("check the loop index or adjust the count/start/end values"),
            );
        }
    }

    let (start_sx, end_sx) = if uses_spacing_mode {
        let Some((width_sx, _)) = Checker::as_numeric_scalar(&width_value) else {
            if !matches!(width_value, Value::Error) {
                ctx.diags.push(
                    Diag::error(
                        width_expr.map_or(index_expr.span.clone(), |expr| expr.span.clone()),
                        format!(
                            "`distribute` width must be scalar, found {}",
                            width_value.kind()
                        ),
                    )
                    .with_help("use a numeric width value such as `0.1`"),
                );
            }
            return Some(Value::Error);
        };

        let Some((gap_sx, _)) = Checker::as_numeric_scalar(&gap_value) else {
            if !matches!(gap_value, Value::Error) {
                ctx.diags.push(
                    Diag::error(
                        gap_expr.map_or(index_expr.span.clone(), |expr| expr.span.clone()),
                        format!(
                            "`distribute` gap must be scalar, found {}",
                            gap_value.kind()
                        ),
                    )
                    .with_help("use a numeric gap value such as `0.02` or omit it to use 0"),
                );
            }
            return Some(Value::Error);
        };

        let Some((anchor_sx, _)) = Checker::as_numeric_scalar(&anchor_value) else {
            if !matches!(anchor_value, Value::Error) {
                ctx.diags.push(
                    Diag::error(
                        anchor_expr.map_or(index_expr.span.clone(), |expr| expr.span.clone()),
                        format!(
                            "`distribute` anchor must be scalar, found {}",
                            anchor_value.kind()
                        ),
                    )
                    .with_help("use a numeric anchor value such as `0.5` or omit it to use 0.5"),
                );
            }
            return Some(Value::Error);
        };

        let pitch = Sx::Add(Box::new(width_sx), Box::new(gap_sx));
        let span = Sx::Mul(Box::new(count_sx.clone()), Box::new(pitch));
        let start = Sx::Sub(
            Box::new(anchor_sx),
            Box::new(Sx::Mul(Box::new(Sx::Lit(0.5)), Box::new(span.clone()))),
        );
        let end = Sx::Add(Box::new(start.clone()), Box::new(span));
        (start, end)
    } else {
        let Some((start_sx, _)) = Checker::as_numeric_scalar(&start_value) else {
            if !matches!(start_value, Value::Error) {
                ctx.diags.push(
                    Diag::error(
                        start_expr.map_or(index_expr.span.clone(), |expr| expr.span.clone()),
                        format!(
                            "`distribute` start must be scalar, found {}",
                            start_value.kind()
                        ),
                    )
                    .with_help("use a numeric start value such as `0.08` or omit it to use 0"),
                );
            }
            return Some(Value::Error);
        };

        let Some((end_sx, _)) = Checker::as_numeric_scalar(&end_value) else {
            if !matches!(end_value, Value::Error) {
                ctx.diags.push(
                    Diag::error(
                        end_expr.map_or(index_expr.span.clone(), |expr| expr.span.clone()),
                        format!(
                            "`distribute` end must be scalar, found {}",
                            end_value.kind()
                        ),
                    )
                    .with_help("use a numeric end value such as `0.92` or omit it to use 1"),
                );
            }
            return Some(Value::Error);
        };

        (start_sx, end_sx)
    };

    Some(Value::Slot {
        index: index_sx,
        count: count_sx,
        start: start_sx,
        end: end_sx,
    })
}

inventory::submit! {
    BuiltinDecl {
        id: BuiltinId("distribute"),
        name: "distribute",
        discriminator: None,
        signature: BuiltinSignature {
            receiver: None,
            args: &[
                BuiltinArgDecl::required(
                    "index",
                    TypeRef::Primitive(PrimitiveType::Scalar),
                    "item index",
                ),
                BuiltinArgDecl::required(
                    "count",
                    TypeRef::Primitive(PrimitiveType::Scalar),
                    "total item count",
                ),
                BuiltinArgDecl::optional(
                    "start",
                    TypeRef::Primitive(PrimitiveType::Scalar),
                    "starting bound; defaults to 0",
                ),
                BuiltinArgDecl::optional(
                    "end",
                    TypeRef::Primitive(PrimitiveType::Scalar),
                    "ending bound; defaults to 1",
                ),
                BuiltinArgDecl::optional(
                    "width",
                    TypeRef::Primitive(PrimitiveType::Scalar),
                    "slot width for spacing mode",
                ),
                BuiltinArgDecl::optional(
                    "gap",
                    TypeRef::Primitive(PrimitiveType::Scalar),
                    "slot gap for spacing mode; defaults to 0",
                ),
                BuiltinArgDecl::optional(
                    "anchor",
                    TypeRef::Primitive(PrimitiveType::Scalar),
                    "center anchor for spacing mode; defaults to 0.5",
                ),
            ],
            result: TypeRef::Named(TypeId("slot")),
            result_alternatives: &[],
            caps: crate::builtin_catalog::BuiltinCaps::from_bits_retain(
                crate::builtin_catalog::BuiltinCaps::PURE.bits()
                    | crate::builtin_catalog::BuiltinCaps::PIPEABLE.bits(),
            ),
        },
        lowering: BuiltinLowering::Impl(distribute_check_impl),
        docs: "Distribute an index across a slot interval and derive center/left/right/size fields.",
    }
}

pub fn rand_check_impl(ctx: &mut Checker, bag: &mut ArgBag<'_>) -> Option<Value> {
    let range = bag.require("range", &mut ctx.diags)?;
    let (lo, hi) = ctx.require_range_pair(range, "`rand`")?;

    if let Some(unit) = ctx.scatter_rand_unit(bag.call_span()) {
        let width = Sx::Sub(Box::new(hi), Box::new(lo.clone()));
        let out = Sx::Add(
            Box::new(lo),
            Box::new(Sx::Mul(Box::new(width), Box::new(unit))),
        );
        ctx.hir.notes.push(format!(
            "signal: rand lowered as deterministic per-instance scatter noise at {}..{}",
            bag.call_span().start,
            bag.call_span().end
        ));
        return Some(Value::Scalar(out));
    }

    ctx.diags.push(
        Diag::error(
            bag.call_span().clone(),
            "`rand(...)` is only supported inside `scatter` for now",
        )
        .with_help(
            "use `rand(...)` inside a scatter body or scatter lifecycle, or replace it with an explicit constant until seeded non-scatter randomness is designed",
        ),
    );
    let _ = (lo, hi);
    None
}

inventory::submit! {
    BuiltinDecl {
        id: BuiltinId("rand"),
        name: "rand",
        discriminator: None,
        signature: BuiltinSignature {
            receiver: None,
            args: &[
                BuiltinArgDecl::required_with_role(
                    "range",
                    TypeRef::Primitive(PrimitiveType::Scalar),
                    "range",
                    "numeric source as `lo .. hi` or `[lo, hi]`",
                ),
            ],
            result: TypeRef::Primitive(PrimitiveType::Scalar),
            result_alternatives: &[],
            caps: crate::builtin_catalog::BuiltinCaps::from_bits_retain(
                crate::builtin_catalog::BuiltinCaps::PURE.bits()
            ),
        },
        lowering: BuiltinLowering::Impl(rand_check_impl),
        docs: "Generate a deterministic scatter random scalar.",
    }
}

pub fn hash_check_impl(ctx: &mut Checker, bag: &mut ArgBag<'_>) -> Option<Value> {
    let value_expr = bag.require("value", &mut ctx.diags)?;
    let salt = match bag.take_named("salt") {
        Some(expr) => ctx.as_scalar(expr)?,
        None => Sx::Lit(0.0),
    };

    let value = ctx.eval(value_expr)?;
    let sx = match value {
        Value::Scalar(v) => sx_hash2(v, salt),
        Value::Vec2((x, y)) => {
            let salted_y = Sx::Add(
                Box::new(y),
                Box::new(Sx::Mul(Box::new(salt), Box::new(Sx::Lit(17.0)))),
            );
            sx_hash2(x, salted_y)
        }
        Value::RepeatCell(cell) => {
            let (x, y) = cell.id;
            let salted_y = Sx::Add(
                Box::new(y),
                Box::new(Sx::Mul(Box::new(salt), Box::new(Sx::Lit(17.0)))),
            );
            sx_hash2(x, salted_y)
        }
        other => {
            let found = match other {
                Value::Distance(_) | Value::Coverage(_) | Value::Mask(_) => "scalar-like value",
                Value::Vec3(_) => "vec3",
                Value::Vec4(_) => "vec4",
                Value::Color { .. } | Value::ColorField { .. } => "color",
                Value::Shape(_) => "shape",
                Value::Layer(_) => "layer",
                Value::Space(_) => "space",
                Value::Array(_) => "array",
                Value::Gradient { .. } => "gradient",
                Value::ScatterInstance { .. } => "scatter instance",
                Value::RepeatCell(_) => "repeat cell",
                Value::CellContour { .. } => "contour",
                Value::Slot { .. } => "slot",
                Value::Mat2(_) => "mat2",
                Value::Mat3(_) => "mat3",
                Value::Mat4(_) => "mat4",
                Value::Lambda { .. } => "lambda",
                Value::FnRef(_) => "fn reference",
                Value::TypedTextureSample { .. } => "typed texture",
                Value::PathFuture(_) => "path",
                Value::DynamicArray { .. } => "dynamic array",
                Value::Struct { .. } => "struct",
                Value::Error => return None,
                Value::Scalar(_) | Value::Vec2(_) => unreachable!(),
            };
            ctx.diags.push(
                Diag::error(
                    value_expr.span.clone(),
                    format!(
                        "`hash(...)` expects a scalar, vec2, or repeat cell input, found {found}"
                    ),
                )
                .with_help("use `hash(x)`, `hash((x, y), salt: n)`, or `hash(cell)`"),
            );
            return None;
        }
    };

    Some(Value::Scalar(sx))
}

inventory::submit! {
    BuiltinDecl {
        id: BuiltinId("hash"),
        name: "hash",
        discriminator: None,
        signature: BuiltinSignature {
            receiver: None,
            args: &[
                BuiltinArgDecl::required_with_role(
                    "value",
                    TypeRef::Primitive(PrimitiveType::Scalar),
                    "value",
                    "scalar, vec2, or repeat-cell seed value",
                ),
                BuiltinArgDecl {
                    name: "salt",
                    ty: TypeRef::Primitive(PrimitiveType::Scalar),
                    viz_role: Some("salt"),
                    required: false,
                    docs: "optional scalar salt mixed into the seed",
                },
            ],
            result: TypeRef::Primitive(PrimitiveType::Scalar),
            result_alternatives: &[],
            caps: crate::builtin_catalog::BuiltinCaps::from_bits_retain(
                crate::builtin_catalog::BuiltinCaps::PURE.bits()
            ),
        },
        lowering: BuiltinLowering::Impl(hash_check_impl),
        docs: "Hash a scalar, vec2, or repeat-cell identity.",
    }
}

builtin! {
    name = "noise1",
    signature = single {
        args(p: Scalar = "input coordinate"),
        result = Mask,
        caps = PURE | INTRINSIC,
    },
    check = |ctx, p| {
        ctx.lower_intrinsic_noise1_components(p)
    }
}

builtin! {
    name = "noise2",
    signature = single {
        args(p: Vec2 = "input coordinate"),
        result = Mask,
        caps = PURE | INTRINSIC,
    },
    check = |ctx, p| {
        let (px, py) = p;
        ctx.lower_intrinsic_noise2_components(px, py)
    }
}

builtin! {
    name = "worley2",
    signature = single {
        args(p: Vec2 = "input coordinate"),
        result = Mask,
        caps = PURE | INTRINSIC,
    },
    check = |ctx, p| {
        let (px, py) = p;
        ctx.hir.notes.push(
            "field: worley2 lowered as deterministic cell noise (nearest jittered lattice point distance)"
                .to_string(),
        );
        Value::Mask(sx_worley2(px, py))
    }
}

builtin! {
    name = "noise3",
    signature = single {
        args(p: Vec3 = "input coordinate"),
        result = Mask,
        caps = PURE | INTRINSIC,
    },
    check = |ctx, p| {
        let (px, py, pz) = p;
        ctx.lower_intrinsic_noise3_components(px, py, pz)
    }
}

builtin! {
    name = "fbm",
    signature = single {
        args(
            p: Vec2 = "input coordinate",
            octaves: Optional<Scalar> = "octave count (literal positive integer)",
            lacunarity: Optional<Scalar> = "frequency multiplier between octaves",
            gain: Optional<Scalar> = "amplitude multiplier between octaves"
        ),
        result = Mask,
        caps = PURE | INTRINSIC,
    },
    check = |ctx, p, octaves, lacunarity, gain| {
        let (px0, py0) = p;
        let octaves = if let Some(octaves) = octaves {
            match Checker::try_eval_static_scalar(&octaves) {
                Some(v) if v >= 1.0 => v.round() as usize,
                Some(_) => {
                    ctx.diags.push(
                        Diag::error(0..0, "`fbm(octaves: ...)` expects a positive integer value")
                            .with_help("example: `fbm(coord, octaves: 5)`"),
                    );
                    return None;
                }
                None => {
                    ctx.diags.push(
                        Diag::error(0..0, "`fbm(octaves: ...)` requires a compile-time scalar value")
                            .with_help("example: `fbm(coord, octaves: 5)`"),
                    );
                    return None;
                }
            }
        } else {
            4
        }
        .clamp(1, 8);
        let lacunarity = lacunarity.unwrap_or(Sx::Lit(2.0));
        let gain = gain.unwrap_or(Sx::Lit(0.5));

        ctx.lower_intrinsic_fbm_components(px0, py0, octaves, lacunarity, gain)
    }
}

builtin! {
    name = "curvature",
    signature = single {
        args(sp: Expr = "normal vector"),
        result = Mask,
        caps = PURE | INTRINSIC,
    },
    check = |ctx, sp| {
        let value = ctx.eval(sp).unwrap_or(Value::Error);
        match value {
            Value::Vec3((nx, ny, nz)) => {
                ctx.hir.notes.push(
                    "field: curvature(normal) lowered from provided normal screen-space derivatives"
                        .to_string(),
                );
                Value::Mask(sx_curvature_from_normal(nx, ny, nz))
            }
            Value::Error => Value::Error,
            other => {
                ctx.diags.push(
                    Diag::error(
                        sp.span.clone(),
                        format!(
                            "`curvature` expects a vec3 normal, found {}",
                            other.kind()
                        ),
                    )
                    .with_help("pass the normal vector explicitly, for example `curvature(sp.normal)`"),
                );
                Value::Error
            }
        }
    }
}

builtin! {
    name = "ease",
    signature = single {
        args(
            t: Scalar = "input value in 0..1",
            curve: Expr = "easing curve name (e.g. `in_out_cubic`, `out_quad`, `out_back`)"
        ),
        result = Scalar,
        caps = PURE,
    },
    check = |ctx, t, curve| {
        let curve_name = match &curve.node {
            Expr::Var(name) => name.as_str(),
            _ => {
                ctx.diags.push(
                    Diag::error(
                        curve.span.clone(),
                        "`ease` expects an easing curve name as its second argument",
                    )
                    .with_help(
                        "use a combined name like `in_out_cubic`, `out_quad`, `out_back`, `in_sine`, `out_in_expo`",
                    ),
                );
                return None;
            }
        };
        let Some((transition, mode)) = Checker::parse_signal_ease_name(curve_name) else {
                ctx.diags.push(
                    Diag::error(
                        curve.span.clone(),
                        format!("`ease` does not recognise curve `{curve_name}`"),
                    )
                    .with_help(
                        "use `in_out_cubic|out_quad|out_back|in_sine|out_in_expo|linear` (mode_transition pattern)",
                    ),
                );
                return None;
            };
        let ease_label = Checker::signal_ease_label(transition, mode);
        ctx.hir.notes.push(format!(
            "signal: ease({ease_label}) applied as a closed-form easing curve"
        ));
        Value::Scalar(Checker::sx_ease(t, transition, mode))
    }
}
