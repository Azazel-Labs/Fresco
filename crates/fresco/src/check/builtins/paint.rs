//! Paint-oriented builtins.

use crate::builtin;
use crate::check::{ArgBag, Checker};
use crate::check::{FillStyle, Value};
use crate::hir::{FillRule, GradientAnchor, GradientKind, Layer, PATH_SEG_KIND_CUBIC, Shape, Sx};
use crate::registry::{
    BuiltinArgDecl, BuiltinDecl, BuiltinId, BuiltinImplReceiverFn, BuiltinLowering,
    BuiltinSignature, PrimitiveType, TypeRef,
};

const PATH_CONTOUR_EPSILON: f32 = 1.0e-5;

fn points_match(a: (f32, f32), b: (f32, f32)) -> bool {
    (a.0 - b.0).abs() <= PATH_CONTOUR_EPSILON && (a.1 - b.1).abs() <= PATH_CONTOUR_EPSILON
}

fn shape_from_closed_path(ctx: &mut Checker, profile_id: usize) -> Option<usize> {
    let profile = ctx.hir.path_profiles.get(profile_id)?;
    let rows = profile.flattened_rows();
    if rows.is_empty() {
        return None;
    }

    let mut contours: Vec<Vec<(Sx, Sx)>> = Vec::new();
    let mut contour_points: Vec<(f32, f32)> = Vec::new();

    for row in rows {
        let seg_start = row.p0;
        let seg_end = if row.kind == PATH_SEG_KIND_CUBIC {
            row.p3
        } else {
            row.p2
        };

        if contour_points.is_empty() {
            contour_points.push(seg_start);
            if !points_match(seg_start, seg_end) {
                contour_points.push(seg_end);
            }
            continue;
        }

        let prev_end = *contour_points
            .last()
            .expect("contour has at least one point");
        if !points_match(prev_end, seg_start) {
            if !points_match(contour_points[0], prev_end) {
                return None;
            }
            contour_points.pop();
            if contour_points.len() < 3 {
                return None;
            }
            contours.push(
                contour_points
                    .iter()
                    .map(|(x, y)| (Sx::Lit(*x), Sx::Lit(*y)))
                    .collect(),
            );
            contour_points.clear();
            contour_points.push(seg_start);
        }

        let current_end = *contour_points
            .last()
            .expect("contour has at least one point");
        if !points_match(current_end, seg_end) {
            contour_points.push(seg_end);
        }
    }

    if contour_points.is_empty() {
        return None;
    }
    let last = *contour_points
        .last()
        .expect("contour has at least one point");
    if !points_match(contour_points[0], last) {
        return None;
    }
    contour_points.pop();
    if contour_points.len() < 3 {
        return None;
    }
    contours.push(
        contour_points
            .iter()
            .map(|(x, y)| (Sx::Lit(*x), Sx::Lit(*y)))
            .collect(),
    );

    Some(ctx.hir.shape(Shape::Polygon {
        contours,
        fill_rule: FillRule::NonZero,
    }))
}

fn path_stroke_coverage(ctx: &mut Checker, path_id: usize, width: Sx) -> Sx {
    ctx.record_path_channel_demand(path_id, "dist");
    let dist = Sx::PathDist { path_id };
    let half = Sx::Mul(Box::new(width), Box::new(Sx::Lit(0.5)));
    let aa = Sx::Max(
        Box::new(Sx::PxLit(1.0)),
        Box::new(Sx::Fwidth(Box::new(dist.clone()))),
    );
    let hi = Sx::Add(Box::new(half.clone()), Box::new(aa));
    let edge = Sx::SmoothStep(Box::new(half), Box::new(hi), Box::new(dist));
    Sx::Sub(Box::new(Sx::Lit(1.0)), Box::new(edge))
}

fn color_to_expr(color: [f32; 4]) -> [Sx; 4] {
    [
        Sx::Lit(color[0]),
        Sx::Lit(color[1]),
        Sx::Lit(color[2]),
        Sx::Lit(color[3]),
    ]
}

fn premultiply_color(color: [Sx; 4], coverage: Sx) -> Layer {
    let [r, g, b, a] = color;
    Layer::ColorExpr {
        r: Sx::Mul(Box::new(r), Box::new(coverage.clone())),
        g: Sx::Mul(Box::new(g), Box::new(coverage.clone())),
        b: Sx::Mul(Box::new(b), Box::new(coverage.clone())),
        a: Sx::Mul(Box::new(a), Box::new(coverage)),
    }
}

fn fill_path_check_impl(ctx: &mut Checker, bag: &mut ArgBag<'_>) -> Option<Value> {
    let Some(path) = bag.path_receiver().cloned() else {
        return Some(Value::Error);
    };
    let color_expr = bag.require("color", &mut ctx.diags)?;
    let Some(shape) = shape_from_closed_path(ctx, path.profile_id()) else {
        ctx.diags.push(
            crate::diag::Diag::error(
                color_expr.span.clone(),
                "`fill` on a path receiver requires a closed contour",
            )
            .with_help("close the path before calling `some_path |> fill(...)`"),
        );
        return Some(Value::Error);
    };

    match ctx.as_fill_style(color_expr)? {
        FillStyle::Solid(color) => Some(Value::Layer(ctx.hir.layer(Layer::Fill { shape, color }))),
        FillStyle::Dynamic([r, g, b, a]) => Some(Value::Layer(ctx.hir.layer(Layer::FillExpr {
            shape,
            r,
            g,
            b,
            a,
        }))),
        FillStyle::Gradient { kind, stops } => {
            Some(Value::Layer(ctx.hir.layer(Layer::FillGradient {
                shape,
                kind,
                stops,
            })))
        }
    }
}

fn stroke_path_check_impl(ctx: &mut Checker, bag: &mut ArgBag<'_>) -> Option<Value> {
    let Some(path) = bag.path_receiver().cloned() else {
        return Some(Value::Error);
    };
    let width_expr = bag.require("width", &mut ctx.diags)?;
    let width_value = ctx.eval(width_expr)?;
    let Some((width, _)) = Checker::as_numeric_scalar(&width_value) else {
        if !matches!(width_value, Value::Error) {
            ctx.diags.push(
                crate::diag::Diag::error(
                    width_expr.span.clone(),
                    format!(
                        "argument `width` to `stroke` expected scalar, found {}",
                        width_value.kind()
                    ),
                )
                .with_label("type mismatch in path stroke width"),
            );
        }
        return Some(Value::Error);
    };

    let coverage = path_stroke_coverage(ctx, path.profile_id(), width);

    let layer = if let Some(color_expr) = bag.take_named("color") {
        match ctx.as_fill_style(color_expr)? {
            FillStyle::Solid(color) => premultiply_color(color_to_expr(color), coverage),
            FillStyle::Dynamic(rgba) => premultiply_color(rgba, coverage),
            FillStyle::Gradient { kind, stops } => {
                premultiply_color(crate::hir::GradientSample::channels(kind, stops), coverage)
            }
        }
    } else {
        Layer::Grey { value: coverage }
    };

    Some(Value::Layer(ctx.hir.layer(layer)))
}

builtin! {
    name = "fill",
    signature = single {
        args(
            color: ColorExpr = "color or gradient fill style"
        ),
        result = Layer,
        caps = PURE,
    },
    check = |ctx, color| {
        let source = ctx.eval(color)?;
        if let Value::PathFuture(_) = source {
            ctx.diags.push(
                crate::diag::Diag::error(
                    color.span.clone(),
                    "`fill(path, ...)` is no longer supported",
                )
                .with_help(
                    "use `some_path |> fill(color)` for closed fills or `some_path |> stroke(width: ..., color: ...)` for path strokes",
                ),
            );
            return Some(Value::Error);
        }

        match ctx.as_fill_style(color)? {
            FillStyle::Solid(color) => Value::Layer(ctx.hir.layer(Layer::Solid(color))),
            FillStyle::Dynamic([r, g, b, a]) => {
                Value::Layer(ctx.hir.layer(Layer::ColorExpr { r, g, b, a }))
            }
            FillStyle::Gradient { kind, stops } => {
                if matches!(kind, GradientKind::Linear { anchor: GradientAnchor::Shape, .. }) {
                    ctx.diags.push(
                        crate::diag::Diag::error(
                            color.span.clone(),
                            "`anchor: shape` requires a shape receiver",
                        )
                        .with_help("use `some_shape |> fill(gradient(..., anchor: shape, ...))`"),
                    );
                    return None;
                }
                let full = ctx.hir.shape(Shape::RBox {
                    center: (Sx::Lit(0.5), Sx::Lit(0.5)),
                    half: (Sx::Lit(0.5), Sx::Lit(0.5)),
                    round: Sx::Lit(0.0),
                });
                Value::Layer(ctx.hir.layer(Layer::FillGradient {
                    shape: full,
                    kind,
                    stops,
                }))
            }
        }
    }
}

#[doc(hidden)]
inventory::submit! {
    BuiltinDecl {
        id: BuiltinId("fill_path"),
        name: "fill",
        discriminator: None,
        signature: BuiltinSignature {
            receiver: Some(TypeRef::Primitive(PrimitiveType::Path)),
            args: &[
                BuiltinArgDecl::required_with_role(
                    "color",
                    TypeRef::Primitive(PrimitiveType::Color),
                    "color",
                    "color or gradient fill style for a closed path",
                ),
            ],
            result: TypeRef::Primitive(PrimitiveType::Layer),
            result_alternatives: &[],
            caps: crate::builtin_catalog::BuiltinCaps::from_bits_retain(
                crate::builtin_catalog::BuiltinCaps::PURE.bits(),
            ),
        },
        lowering: BuiltinLowering::ImplReceiver(BuiltinImplReceiverFn::path(fill_path_check_impl)),
        docs: "Fill a path receiver with a color or gradient.",
    }
}

#[doc(hidden)]
inventory::submit! {
    BuiltinDecl {
        id: BuiltinId("stroke_path"),
        name: "stroke",
        discriminator: None,
        signature: BuiltinSignature {
            receiver: Some(TypeRef::Primitive(PrimitiveType::Path)),
            args: &[
                BuiltinArgDecl::required_with_role(
                    "width",
                    TypeRef::Primitive(PrimitiveType::Scalar),
                    "stroke width",
                    "path stroke width",
                ),
                BuiltinArgDecl::optional(
                    "color",
                    TypeRef::Primitive(PrimitiveType::Color),
                    "optional stroke color; omitted returns coverage layer",
                ),
            ],
            result: TypeRef::Primitive(PrimitiveType::Layer),
            result_alternatives: &[],
            caps: crate::builtin_catalog::BuiltinCaps::from_bits_retain(
                crate::builtin_catalog::BuiltinCaps::PURE.bits(),
            ),
        },
        lowering: BuiltinLowering::ImplReceiver(BuiltinImplReceiverFn::path(stroke_path_check_impl)),
        docs: "Stroke a path receiver.",
    }
}

builtin! {
    name = "stroke",
    signature = single {
        receiver = Shape,
        args(
            width: Scalar = "stroke width",
            color: Optional<ColorExpr> = "optional color or gradient; if omitted returns outlined shape"
        ),
        result = Shape | Layer,
        caps = PURE,
    },
    check = |ctx, width, color| {
        let outline = ctx.hir.shape(Shape::Outline {
            inner: _receiver,
            width,
        });

        if let Some(color_expr) = color {
            let layer = match ctx.as_fill_style(color_expr)? {
                FillStyle::Solid(color) => Layer::Fill { shape: outline, color },
                FillStyle::Dynamic([r, g, b, a]) => Layer::FillExpr { shape: outline, r, g, b, a },
                FillStyle::Gradient { kind, stops } => Layer::FillGradient { shape: outline, kind, stops },
            };
            Value::Layer(ctx.hir.layer(layer))
        } else {
            Value::Shape(outline)
        }
    }
}

builtin! {
    name = "fill",
    signature = single {
        receiver = Shape,
        args(color: ColorExpr = "color or gradient fill style"),
        result = Layer,
        caps = PURE,
    },
    check = |ctx, color| {
        let shape = _receiver;
        match ctx.as_fill_style(color)? {
            FillStyle::Solid(color) => Value::Layer(ctx.hir.layer(Layer::Fill { shape, color })),
            FillStyle::Dynamic([r, g, b, a]) => {
                Value::Layer(ctx.hir.layer(Layer::FillExpr { shape, r, g, b, a }))
            }
            FillStyle::Gradient { kind, stops } => {
                Value::Layer(ctx.hir.layer(Layer::FillGradient { shape, kind, stops }))
            }
        }
    }
}
