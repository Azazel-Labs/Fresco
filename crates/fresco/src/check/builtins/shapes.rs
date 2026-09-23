//! Shape constructor builtins.

use crate::builtin;
use crate::check::Value;
use crate::hir::{FillRule, Shape, ShapeId, Sx};

fn sx_add(a: Sx, b: Sx) -> Sx {
    Sx::Add(Box::new(a), Box::new(b))
}

fn sx_sub(a: Sx, b: Sx) -> Sx {
    Sx::Sub(Box::new(a), Box::new(b))
}

fn sx_mul(a: Sx, b: Sx) -> Sx {
    Sx::Mul(Box::new(a), Box::new(b))
}

fn sx_div(a: Sx, b: Sx) -> Sx {
    Sx::Div(Box::new(a), Box::new(b))
}

fn sx_cos(a: Sx) -> Sx {
    Sx::Cos(Box::new(a))
}

fn sx_sin(a: Sx) -> Sx {
    Sx::Sin(Box::new(a))
}

fn sx_abs(a: Sx) -> Sx {
    Sx::Abs(Box::new(a))
}

fn sx_sign(a: Sx) -> Sx {
    Sx::Sign(Box::new(a))
}

fn sx_pow(a: Sx, b: Sx) -> Sx {
    Sx::Pow(Box::new(a), Box::new(b))
}

fn union_many(ctx: &mut crate::check::Checker, ids: Vec<ShapeId>) -> Option<ShapeId> {
    let mut iter = ids.into_iter();
    let first = iter.next()?;
    Some(iter.fold(first, |acc, id| ctx.hir.shape(Shape::Union(acc, id))))
}

fn parse_fill_rule(
    ctx: &mut crate::check::Checker,
    expr: Option<&crate::ast::SExpr>,
) -> Option<FillRule> {
    let Some(expr) = expr else {
        return Some(FillRule::NonZero);
    };
    match &expr.node {
        crate::ast::Expr::Var(v) if v == "non_zero" || v == "FillRule.non_zero" => {
            Some(FillRule::NonZero)
        }
        crate::ast::Expr::Var(v) if v == "even_odd" || v == "FillRule.even_odd" => {
            Some(FillRule::EvenOdd)
        }
        _ => {
            ctx.diags.push(
                crate::diag::Diag::error(
                    expr.span.clone(),
                    "polygon fill_rule expects `non_zero` or `even_odd`",
                )
                .with_help("use `fill_rule: non_zero` or `fill_rule: even_odd`"),
            );
            None
        }
    }
}

fn parse_plane_mode(
    ctx: &mut crate::check::Checker,
    expr: Option<&crate::ast::SExpr>,
) -> Option<&'static str> {
    let Some(expr) = expr else {
        return Some("auto");
    };
    match &expr.node {
        crate::ast::Expr::Var(v) if v == "auto" || v == "PlaneMode.auto" => Some("auto"),
        crate::ast::Expr::Var(v) if v == "explicit" || v == "PlaneMode.explicit" => {
            Some("explicit")
        }
        _ => {
            ctx.diags.push(
                crate::diag::Diag::error(
                    expr.span.clone(),
                    "polygon3 plane_mode expects `auto` or `explicit`",
                )
                .with_help("use `plane_mode: auto` or `plane_mode: explicit`"),
            );
            None
        }
    }
}

fn collect_polygon_points(
    ctx: &mut crate::check::Checker,
    points: &crate::ast::SExpr,
    allow_vec3: bool,
) -> Option<(Vec<(Sx, Sx)>, bool)> {
    let values = ctx.eval(points)?;
    let Value::Array(items) = values else {
        ctx.diags.push(
            crate::diag::Diag::error(points.span.clone(), "points expects an array literal")
                .with_help("use `points: [(x0, y0), (x1, y1), ...]`"),
        );
        return None;
    };

    let mut out = Vec::with_capacity(items.len());
    let mut had_vec3 = false;
    for item in items {
        match item {
            Value::Vec2(v) => out.push(v),
            Value::Vec3((x, y, _z)) if allow_vec3 => {
                had_vec3 = true;
                out.push((x, y));
            }
            Value::Vec3(_) => {
                ctx.diags.push(
                    crate::diag::Diag::error(
                        points.span.clone(),
                        "polygon2 points must be vec2 values",
                    )
                    .with_help("use points like `(x, y)` for polygon2"),
                );
                return None;
            }
            other => {
                ctx.diags.push(
                    crate::diag::Diag::error(
                        points.span.clone(),
                        format!(
                            "polygon points must be vec2/vec3 values, found {}",
                            other.kind()
                        ),
                    )
                    .with_help("use points like `(x, y)` or `(x, y, z)`"),
                );
                return None;
            }
        }
    }

    if out.len() < 3 {
        ctx.diags.push(
            crate::diag::Diag::error(points.span.clone(), "polygon expects at least 3 points")
                .with_help("provide three or more vertices"),
        );
        return None;
    }

    Some((out, had_vec3))
}

fn collect_polyline_points(
    ctx: &mut crate::check::Checker,
    points: &crate::ast::SExpr,
) -> Option<Vec<(Sx, Sx)>> {
    let values = ctx.eval(points)?;
    let Value::Array(items) = values else {
        ctx.diags.push(
            crate::diag::Diag::error(points.span.clone(), "points expects an array literal")
                .with_help("use `points: [(x0, y0), (x1, y1), ...]`"),
        );
        return None;
    };

    let mut out = Vec::with_capacity(items.len());
    for item in items {
        match item {
            Value::Vec2(v) => out.push(v),
            other => {
                ctx.diags.push(
                    crate::diag::Diag::error(
                        points.span.clone(),
                        format!(
                            "polyline points must be vec2 values, found {}",
                            other.kind()
                        ),
                    )
                    .with_help("use points like `(x, y)` for polyline"),
                );
                return None;
            }
        }
    }

    if out.len() < 2 {
        ctx.diags.push(
            crate::diag::Diag::error(points.span.clone(), "polyline expects at least 2 points")
                .with_help("provide two or more vertices"),
        );
        return None;
    }

    Some(out)
}

fn parse_bool_like(
    ctx: &mut crate::check::Checker,
    expr: Option<&crate::ast::SExpr>,
    arg_name: &str,
    default: bool,
) -> Option<bool> {
    let Some(expr) = expr else {
        return Some(default);
    };
    match &expr.node {
        crate::ast::Expr::Var(v) if v == "true" => Some(true),
        crate::ast::Expr::Var(v) if v == "false" => Some(false),
        _ => {
            ctx.diags.push(
                crate::diag::Diag::error(
                    expr.span.clone(),
                    format!("{arg_name} expects `true` or `false`"),
                )
                .with_help(format!("use `{arg_name}: true` or `{arg_name}: false`")),
            );
            None
        }
    }
}

fn parse_steps_literal(
    ctx: &mut crate::check::Checker,
    expr: Option<&crate::ast::SExpr>,
    arg_name: &str,
    default_steps: usize,
    min_steps: usize,
    max_steps: usize,
) -> Option<usize> {
    let Some(expr) = expr else {
        return Some(default_steps);
    };
    let value = ctx.eval(expr)?;
    let Value::Scalar(Sx::Lit(raw)) = value else {
        ctx.diags.push(
            crate::diag::Diag::error(
                expr.span.clone(),
                format!("{arg_name} expects a literal integer"),
            )
            .with_help(format!("use `{arg_name}: {default_steps}`")),
        );
        return None;
    };
    if raw.fract() != 0.0 || raw < min_steps as f32 || raw > max_steps as f32 {
        ctx.diags.push(crate::diag::Diag::error(
            expr.span.clone(),
            format!("{arg_name} must be a literal integer between {min_steps} and {max_steps}"),
        ));
        return None;
    }
    Some(raw as usize)
}

builtin! {
    name = "circle",
    signature = single {
        args(at: Vec2 = "circle center", radius: Scalar = "circle radius"),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, at, radius| {
        Value::Shape(ctx.hir.shape(Shape::Circle {
            center: at,
            radius,
        }))
    }
}

builtin! {
    name = "ring",
    signature = single {
        args(
            at: Vec2 = "ring center",
            radius: Scalar = "ring radius",
            width: Scalar = "ring width"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, at, radius, width| {
        let base = ctx.hir.shape(Shape::Circle { center: at, radius });
        Value::Shape(ctx.hir.shape(Shape::Outline { inner: base, width }))
    }
}

builtin! {
    name = "line",
    signature = single {
        args(
            from: Vec2 = "line start point",
            to: Vec2 = "line end point",
            thickness: Scalar = "line thickness"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, from, to, thickness| {
        let radius = sx_mul(thickness, Sx::Lit(0.5));
        Value::Shape(ctx.hir.shape(Shape::Capsule { from, to, radius }))
    }
}

builtin! {
    name = "polyline",
    signature = single {
        args(
            points: Expr = "polyline vertices",
            thickness: Scalar = "polyline thickness",
            closed: Optional<Expr> = "close path (`true` or `false`)"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, points, thickness, closed| {
        let points = collect_polyline_points(ctx, points)?;
        let closed = parse_bool_like(ctx, closed, "closed", false)?;
        let radius = sx_mul(thickness, Sx::Lit(0.5));

        let mut parts: Vec<ShapeId> = Vec::new();
        for pair in points.windows(2) {
            let seg = ctx.hir.shape(Shape::Capsule {
                from: pair[0].clone(),
                to: pair[1].clone(),
                radius: radius.clone(),
            });
            parts.push(seg);
        }

        if closed {
            let seg = ctx.hir.shape(Shape::Capsule {
                from: points[points.len() - 1].clone(),
                to: points[0].clone(),
                radius: radius.clone(),
            });
            parts.push(seg);
        }

        for p in &points {
            parts.push(ctx.hir.shape(Shape::Circle {
                center: p.clone(),
                radius: radius.clone(),
            }));
        }

        let shape = union_many(ctx, parts).expect("polyline must create at least one segment");
        Value::Shape(shape)
    }
}

builtin! {
    name = "sector",
    signature = single {
        args(
            at: Vec2 = "sector center",
            radius: Scalar = "sector radius",
            from: Scalar = "start angle (radians)",
            to: Scalar = "end angle (radians)",
            steps: Optional<Expr> = "arc tessellation steps"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, at, radius, from, to, steps| {
        let steps = parse_steps_literal(ctx, steps, "steps", 48, 8, 256)?;
        let (cx, cy) = at;
        let delta = sx_sub(to, from.clone());
        let mut contour: Vec<(Sx, Sx)> = Vec::with_capacity(steps + 2);
        contour.push((cx.clone(), cy.clone()));
        for i in 0..=steps {
            let t = Sx::Lit(i as f32 / steps as f32);
            let angle = sx_add(from.clone(), sx_mul(delta.clone(), t));
            let x = sx_add(cx.clone(), sx_mul(radius.clone(), sx_cos(angle.clone())));
            let y = sx_add(cy.clone(), sx_mul(radius.clone(), sx_sin(angle)));
            contour.push((x, y));
        }
        Value::Shape(ctx.hir.shape(Shape::Polygon {
            contours: vec![contour],
            fill_rule: FillRule::NonZero,
        }))
    }
}

builtin! {
    name = "arc",
    signature = single {
        args(
            at: Vec2 = "arc center",
            radius: Scalar = "arc radius",
            width: Scalar = "arc width",
            from: Scalar = "start angle (radians)",
            to: Scalar = "end angle (radians)",
            steps: Optional<Expr> = "arc tessellation steps"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, at, radius, width, from, to, steps| {
        let steps = parse_steps_literal(ctx, steps, "steps", 48, 8, 256)?;
        let (cx, cy) = at;
        let delta = sx_sub(to, from.clone());
        let mut contour: Vec<(Sx, Sx)> = Vec::with_capacity(steps + 2);
        contour.push((cx.clone(), cy.clone()));
        for i in 0..=steps {
            let t = Sx::Lit(i as f32 / steps as f32);
            let angle = sx_add(from.clone(), sx_mul(delta.clone(), t));
            let x = sx_add(cx.clone(), sx_mul(radius.clone(), sx_cos(angle.clone())));
            let y = sx_add(cy.clone(), sx_mul(radius.clone(), sx_sin(angle)));
            contour.push((x, y));
        }
        let sector = ctx.hir.shape(Shape::Polygon {
            contours: vec![contour],
            fill_rule: FillRule::NonZero,
        });
        Value::Shape(ctx.hir.shape(Shape::Outline { inner: sector, width }))
    }
}

builtin! {
    name = "superellipse",
    signature = single {
        args(
            at: Vec2 = "superellipse center",
            radii: Vec2 = "superellipse radii",
            power: Scalar = "shape exponent",
            steps: Optional<Expr> = "tessellation steps"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, at, radii, power, steps| {
        let steps = parse_steps_literal(ctx, steps, "steps", 64, 16, 256)?;
        let (cx, cy) = at;
        let (rx, ry) = radii;
        let inv = sx_div(Sx::Lit(2.0), power);
        let mut contour: Vec<(Sx, Sx)> = Vec::with_capacity(steps);
        for i in 0..steps {
            let angle = Sx::Lit(std::f32::consts::TAU * (i as f32 / steps as f32));
            let ct = sx_cos(angle.clone());
            let st = sx_sin(angle);
            let x_norm = sx_mul(sx_sign(ct.clone()), sx_pow(sx_abs(ct), inv.clone()));
            let y_norm = sx_mul(sx_sign(st.clone()), sx_pow(sx_abs(st), inv.clone()));
            let x = sx_add(cx.clone(), sx_mul(rx.clone(), x_norm));
            let y = sx_add(cy.clone(), sx_mul(ry.clone(), y_norm));
            contour.push((x, y));
        }
        Value::Shape(ctx.hir.shape(Shape::Polygon {
            contours: vec![contour],
            fill_rule: FillRule::NonZero,
        }))
    }
}

builtin! {
    name = "rounded_polygon",
    signature = single {
        args(
            points: Expr = "polygon vertices",
            round: Scalar = "roundness offset",
            fill_rule: Optional<Expr> = "fill rule (`non_zero` or `even_odd`)"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, points, round, fill_rule| {
        let fill_rule = parse_fill_rule(ctx, fill_rule)?;
        let (points, _had_vec3) = collect_polygon_points(ctx, points, false)?;
        let poly = ctx.hir.shape(Shape::Polygon {
            contours: vec![points],
            fill_rule,
        });
        Value::Shape(ctx.hir.shape(Shape::Offset {
            inner: poly,
            delta: round,
        }))
    }
}

builtin! {
    name = "plus",
    signature = single {
        args(
            at: Vec2 = "plus center",
            size: Vec2 = "plus size",
            thickness: Scalar = "arm thickness"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, at, size, thickness| {
        let (cx, cy) = at;
        let (sx, sy) = size;
        let h = sx_mul(thickness, Sx::Lit(0.5));
        let hx = sx_mul(sx, Sx::Lit(0.5));
        let hy = sx_mul(sy, Sx::Lit(0.5));

        let horiz = ctx.hir.shape(Shape::RBox {
            center: (cx.clone(), cy.clone()),
            half: (hx, h.clone()),
            round: Sx::Lit(0.0),
        });
        let vert = ctx.hir.shape(Shape::RBox {
            center: (cx, cy),
            half: (h, hy),
            round: Sx::Lit(0.0),
        });
        Value::Shape(ctx.hir.shape(Shape::Union(horiz, vert)))
    }
}

builtin! {
    name = "x_cross",
    signature = single {
        args(
            at: Vec2 = "cross center",
            size: Vec2 = "cross size",
            thickness: Scalar = "stroke thickness"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, at, size, thickness| {
        let (cx, cy) = at;
        let (sx, sy) = size;
        let hx = sx_mul(sx, Sx::Lit(0.5));
        let hy = sx_mul(sy, Sx::Lit(0.5));
        let radius = sx_mul(thickness, Sx::Lit(0.5));

        let a0 = (sx_sub(cx.clone(), hx.clone()), sx_sub(cy.clone(), hy.clone()));
        let a1 = (sx_add(cx.clone(), hx.clone()), sx_add(cy.clone(), hy.clone()));
        let b0 = (sx_sub(cx.clone(), hx.clone()), sx_add(cy.clone(), hy.clone()));
        let b1 = (sx_add(cx, hx), sx_sub(cy, hy));

        let d1 = ctx.hir.shape(Shape::Capsule {
            from: a0,
            to: a1,
            radius: radius.clone(),
        });
        let d2 = ctx.hir.shape(Shape::Capsule {
            from: b0,
            to: b1,
            radius,
        });
        Value::Shape(ctx.hir.shape(Shape::Union(d1, d2)))
    }
}

builtin! {
    name = "crescent",
    signature = single {
        args(
            at: Vec2 = "crescent center",
            radius: Scalar = "crescent radius",
            offset: Vec2 = "inner circle offset"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, at, radius, offset| {
        let (cx, cy) = at;
        let (ox, oy) = offset;
        let outer = ctx.hir.shape(Shape::Circle {
            center: (cx.clone(), cy.clone()),
            radius: radius.clone(),
        });
        let inner = ctx.hir.shape(Shape::Circle {
            center: (sx_add(cx, ox), sx_add(cy, oy)),
            radius,
        });
        Value::Shape(ctx.hir.shape(Shape::Subtract(outer, inner)))
    }
}

builtin! {
    name = "lens",
    signature = single {
        args(
            at: Vec2 = "lens center",
            radius: Scalar = "circle radius",
            separation: Scalar = "distance between circle centers"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, at, radius, separation| {
        let (cx, cy) = at;
        let half = sx_mul(separation, Sx::Lit(0.5));
        let a = ctx.hir.shape(Shape::Circle {
            center: (sx_sub(cx.clone(), half.clone()), cy.clone()),
            radius: radius.clone(),
        });
        let b = ctx.hir.shape(Shape::Circle {
            center: (sx_add(cx, half), cy),
            radius,
        });
        Value::Shape(ctx.hir.shape(Shape::Intersect(a, b)))
    }
}

builtin! {
    name = "heart",
    signature = single {
        args(
            at: Vec2 = "heart center",
            size: Scalar = "heart size",
            steps: Optional<Expr> = "tessellation steps"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, at, size, steps| {
        let steps = parse_steps_literal(ctx, steps, "steps", 80, 16, 256)?;
        let (cx, cy) = at;
        let scale = sx_div(size, Sx::Lit(18.0));
        let mut contour = Vec::with_capacity(steps);
        for i in 0..steps {
            let t = Sx::Lit(std::f32::consts::TAU * (i as f32 / steps as f32));
            let st = sx_sin(t.clone());
            let ct = sx_cos(t.clone());
            let st2 = sx_mul(st.clone(), st.clone());
            let xshape = sx_mul(Sx::Lit(16.0), sx_mul(st2, st.clone()));

            let c2 = sx_cos(sx_mul(Sx::Lit(2.0), t.clone()));
            let c3 = sx_cos(sx_mul(Sx::Lit(3.0), t.clone()));
            let c4 = sx_cos(sx_mul(Sx::Lit(4.0), t));
            let yshape = sx_sub(
                sx_sub(
                    sx_sub(sx_mul(Sx::Lit(13.0), ct), sx_mul(Sx::Lit(5.0), c2)),
                    sx_mul(Sx::Lit(2.0), c3),
                ),
                c4,
            );

            let x = sx_add(cx.clone(), sx_mul(scale.clone(), xshape));
            let y = sx_add(cy.clone(), sx_mul(scale.clone(), yshape));
            contour.push((x, y));
        }
        Value::Shape(ctx.hir.shape(Shape::Polygon {
            contours: vec![contour],
            fill_rule: FillRule::NonZero,
        }))
    }
}

builtin! {
    name = "gear",
    signature = single {
        args(
            at: Vec2 = "gear center",
            inner: Scalar = "inner radius",
            outer: Scalar = "outer radius",
            teeth: Scalar = "tooth count"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, at, inner, outer, teeth| {
        let teeth_u32 = match teeth {
            Sx::Lit(n) if (3.0..=64.0).contains(&n) && n.fract() == 0.0 => n as u32,
            _ => 12,
        };
        Value::Shape(ctx.hir.shape(Shape::Star {
            center: at,
            outer,
            inner,
            points: teeth_u32,
        }))
    }
}

builtin! {
    name = "starburst",
    signature = single {
        args(
            at: Vec2 = "burst center",
            inner: Scalar = "inner radius",
            outer: Scalar = "outer radius",
            rays: Scalar = "ray count"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, at, inner, outer, rays| {
        let rays_u32 = match rays {
            Sx::Lit(n) if (3.0..=64.0).contains(&n) && n.fract() == 0.0 => n as u32,
            _ => 16,
        };
        Value::Shape(ctx.hir.shape(Shape::Star {
            center: at,
            outer,
            inner,
            points: rays_u32,
        }))
    }
}

builtin! {
    name = "rhombus",
    signature = single {
        args(
            at: Vec2 = "rhombus center",
            size: Vec2 = "rhombus width/height"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, at, size| {
        let (cx, cy) = at;
        let (sx, sy) = size;
        let hx = sx_mul(sx, Sx::Lit(0.5));
        let hy = sx_mul(sy, Sx::Lit(0.5));
        let points = vec![
            (cx.clone(), sx_add(cy.clone(), hy.clone())),
            (sx_add(cx.clone(), hx.clone()), cy.clone()),
            (cx.clone(), sx_sub(cy.clone(), hy)),
            (sx_sub(cx, hx), cy),
        ];
        Value::Shape(ctx.hir.shape(Shape::Polygon {
            contours: vec![points],
            fill_rule: FillRule::NonZero,
        }))
    }
}

builtin! {
    name = "parallelogram",
    signature = single {
        args(
            at: Vec2 = "parallelogram center",
            size: Vec2 = "parallelogram width/height",
            skew: Scalar = "horizontal skew"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, at, size, skew| {
        let (cx, cy) = at;
        let (sx, sy) = size;
        let hx = sx_mul(sx, Sx::Lit(0.5));
        let hy = sx_mul(sy, Sx::Lit(0.5));
        let points = vec![
            (
                sx_sub(sx_sub(cx.clone(), hx.clone()), skew.clone()),
                sx_add(cy.clone(), hy.clone()),
            ),
            (
                sx_add(sx_add(cx.clone(), hx.clone()), skew.clone()),
                sx_add(cy.clone(), hy.clone()),
            ),
            (
                sx_add(sx_add(cx.clone(), hx.clone()), sx_mul(skew.clone(), Sx::Lit(-1.0))),
                sx_sub(cy.clone(), hy.clone()),
            ),
            (
                sx_sub(sx_sub(cx, hx), sx_mul(skew, Sx::Lit(-1.0))),
                sx_sub(cy, hy),
            ),
        ];
        Value::Shape(ctx.hir.shape(Shape::Polygon {
            contours: vec![points],
            fill_rule: FillRule::NonZero,
        }))
    }
}

builtin! {
    name = "droplet",
    signature = single {
        args(
            at: Vec2 = "droplet center",
            size: Scalar = "droplet size",
            steps: Optional<Expr> = "tessellation steps"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, at, size, steps| {
        let steps = parse_steps_literal(ctx, steps, "steps", 64, 16, 256)?;
        let (cx, cy) = at;
        let mut contour = Vec::with_capacity(steps);
        for i in 0..steps {
            let t = Sx::Lit(std::f32::consts::TAU * (i as f32 / steps as f32));
            let st = sx_sin(t.clone());
            let ct = sx_cos(t.clone());
            let c2 = sx_cos(sx_mul(Sx::Lit(2.0), t));
            let xshape = sx_mul(Sx::Lit(0.7), st);
            let yshape = sx_sub(sx_mul(Sx::Lit(0.85), ct), sx_mul(Sx::Lit(0.3), c2));
            let x = sx_add(cx.clone(), sx_mul(size.clone(), xshape));
            let y = sx_add(cy.clone(), sx_mul(size.clone(), yshape));
            contour.push((x, y));
        }
        Value::Shape(ctx.hir.shape(Shape::Polygon {
            contours: vec![contour],
            fill_rule: FillRule::NonZero,
        }))
    }
}

builtin! {
    name = "capsule",
    signature = single {
        args(
            from: Vec2 = "capsule start point",
            to: Vec2 = "capsule end point",
            radius: Scalar = "capsule radius"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, from, to, radius| {
        Value::Shape(ctx.hir.shape(Shape::Capsule {
            from,
            to,
            radius,
        }))
    }
}

builtin! {
    name = "box",
    signature = single {
        args(
            at: Vec2 = "box center",
            size: Vec2 = "box size"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, at, size| {
        let (sx, sy) = size;
        let half = (
            Sx::Mul(Box::new(sx), Box::new(Sx::Lit(0.5))),
            Sx::Mul(Box::new(sy), Box::new(Sx::Lit(0.5))),
        );
        Value::Shape(ctx.hir.shape(Shape::RBox {
            center: at,
            half,
            round: Sx::Lit(0.0),
        }))
    }
}

builtin! {
    name = "ngon",
    signature = single {
        args(
            at: Vec2 = "ngon center",
            radius: Scalar = "ngon radius",
            sides: Scalar = "number of sides"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, at, radius, sides| {
        // Extract constant integer sides count (default to 6 if not a literal)
        let sides_u32 = match sides {
            Sx::Lit(n) if (3.0..=12.0).contains(&n) && n.fract() == 0.0 => n as u32,
            Sx::Lit(_) => {
                ctx.diags.push(
                    crate::diag::Diag::error(
                        crate::ast::Span::default(),
                        "ngon sides must be a literal integer between 3 and 12",
                    )
                );
                6 // default fallback
            }
            _ => 6, // fallback for non-literal
        };
        let (cx, cy) = at;
        let mut points = Vec::with_capacity(sides_u32 as usize);
        for i in 0..sides_u32 {
            let angle = std::f32::consts::TAU * (i as f32) / (sides_u32 as f32);
            let px = Sx::Add(
                Box::new(cx.clone()),
                Box::new(Sx::Mul(Box::new(radius.clone()), Box::new(Sx::Lit(angle.cos())))),
            );
            let py = Sx::Add(
                Box::new(cy.clone()),
                Box::new(Sx::Mul(Box::new(radius.clone()), Box::new(Sx::Lit(angle.sin())))),
            );
            points.push((px, py));
        }

        let shape = ctx.hir.shape(Shape::Polygon {
            contours: vec![points],
            fill_rule: FillRule::NonZero,
        });
        ctx.hir
            .notes
            .push(format!("ngon: lowered {} side(s) as contour polygon", sides_u32));
        Value::Shape(shape)
    }
}

builtin! {
    name = "ellipse",
    signature = single {
        args(
            at: Vec2 = "ellipse center",
            radii: Vec2 = "ellipse radii (rx, ry)"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, at, radii| {
        Value::Shape(ctx.hir.shape(Shape::Ellipse {
            center: at,
            radii,
        }))
    }
}

builtin! {
    name = "star",
    signature = single {
        args(
            at: Vec2 = "star center",
            outer: Scalar = "outer radius",
            inner: Scalar = "inner radius",
            points: Scalar = "number of points"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, at, outer, inner, points| {
        let points_u32 = match points {
            Sx::Lit(n) if (3.0..=12.0).contains(&n) && n.fract() == 0.0 => n as u32,
            Sx::Lit(_) => {
                ctx.diags.push(
                    crate::diag::Diag::error(
                        crate::ast::Span::default(),
                        "star points must be a literal integer between 3 and 12",
                    )
                );
                5 // default fallback
            }
            _ => 5, // fallback for non-literal
        };

        let (cx, cy) = at;
        let vert_count = (points_u32 as usize) * 2;
        let mut contour = Vec::with_capacity(vert_count);
        for i in 0..vert_count {
            let angle = std::f32::consts::TAU * (i as f32) / (vert_count as f32)
                + std::f32::consts::FRAC_PI_2;
            let radius = if i % 2 == 0 {
                outer.clone()
            } else {
                inner.clone()
            };
            let vx = Sx::Add(
                Box::new(cx.clone()),
                Box::new(Sx::Mul(Box::new(radius.clone()), Box::new(Sx::Lit(angle.cos())))),
            );
            let vy = Sx::Add(
                Box::new(cy.clone()),
                Box::new(Sx::Mul(Box::new(radius), Box::new(Sx::Lit(angle.sin())))),
            );
            contour.push((vx, vy));
        }

        let shape = ctx.hir.shape(Shape::Polygon {
            contours: vec![contour],
            fill_rule: FillRule::NonZero,
        });
        ctx.hir.notes.push(format!(
            "star: lowered {} point(s) as sharp contour polygon",
            points_u32
        ));
        Value::Shape(shape)
    }
}

builtin! {
    name = "flower",
    signature = single {
        args(
            at: Vec2 = "flower center",
            outer: Scalar = "outer radius",
            inner: Scalar = "inner radius",
            points: Scalar = "number of petals"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, at, outer, inner, points| {
        let points_u32 = match points {
            Sx::Lit(n) if (3.0..=12.0).contains(&n) && n.fract() == 0.0 => n as u32,
            Sx::Lit(_) => {
                ctx.diags.push(
                    crate::diag::Diag::error(
                        crate::ast::Span::default(),
                        "flower points must be a literal integer between 3 and 12",
                    )
                );
                5 // default fallback
            }
            _ => 5, // fallback for non-literal
        };
        Value::Shape(ctx.hir.shape(Shape::Star {
            center: at,
            outer,
            inner,
            points: points_u32,
        }))
    }
}

builtin! {
    name = "triangle",
    signature = single {
        args(
            a: Vec2 = "triangle vertex A",
            b: Vec2 = "triangle vertex B",
            c: Vec2 = "triangle vertex C"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, a, b, c| {
        Value::Shape(ctx.hir.shape(Shape::Triangle {
            a,
            b,
            c,
        }))
    }
}

builtin! {
    name = "trapezoid",
    signature = single {
        args(
            at: Vec2 = "trapezoid center",
            top: Scalar = "top width",
            bottom: Scalar = "bottom width",
            height: Scalar = "height"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, at, top, bottom, height| {
        let (cx, cy) = at;
        let half = Sx::Mul(Box::new(height), Box::new(Sx::Lit(0.5)));
        let y_top = Sx::Add(Box::new(cy.clone()), Box::new(half.clone()));
        let y_bottom = Sx::Sub(Box::new(cy), Box::new(half));

        let h_top = Sx::Mul(Box::new(top), Box::new(Sx::Lit(0.5)));
        let h_bottom = Sx::Mul(Box::new(bottom), Box::new(Sx::Lit(0.5)));

        let p0 = (
            Sx::Sub(Box::new(cx.clone()), Box::new(h_top.clone())),
            y_top.clone(),
        );
        let p1 = (Sx::Add(Box::new(cx.clone()), Box::new(h_top)), y_top);
        let p2 = (
            Sx::Add(Box::new(cx.clone()), Box::new(h_bottom.clone())),
            y_bottom.clone(),
        );
        let p3 = (Sx::Sub(Box::new(cx), Box::new(h_bottom)), y_bottom);

        let points = vec![p0, p1, p2, p3];
        Value::Shape(ctx.hir.shape(Shape::Polygon {
            contours: vec![points],
            fill_rule: FillRule::NonZero,
        }))
    }
}

builtin! {
    name = "polygon2",
    signature = single {
        args(
            points: Expr = "polygon vertices",
            fill_rule: Optional<Expr> = "fill rule (`non_zero` or `even_odd`)"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, points, fill_rule| {
        let fill_rule = parse_fill_rule(ctx, fill_rule)?;
        let (points, _had_vec3) = collect_polygon_points(ctx, points, false)?;
        let point_count = points.len();
        let shape = ctx.hir.shape(Shape::Polygon {
            contours: vec![points],
            fill_rule,
        });
        ctx.hir.notes.push(format!(
            "polygon2: lowered {} point(s) as contour polygon (fill_rule: {:?})",
            point_count, fill_rule
        ));
        Value::Shape(shape)
    }
}

builtin! {
    name = "polygon3",
    signature = single {
        args(
            points: Expr = "polygon vertices",
            fill_rule: Optional<Expr> = "fill rule (`non_zero` or `even_odd`)",
            plane_mode: Optional<Expr> = "plane mode (`auto` or `explicit`)"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, points, fill_rule, plane_mode| {
        let fill_rule = parse_fill_rule(ctx, fill_rule)?;
        let plane_mode = parse_plane_mode(ctx, plane_mode)?;
        let (points, had_vec3) = collect_polygon_points(ctx, points, true)?;
        let point_count = points.len();
        let shape = ctx.hir.shape(Shape::Polygon {
            contours: vec![points],
            fill_rule,
        });
        ctx.hir.notes.push(format!(
            "polygon3: lowered {} point(s) to projected contour polygon (fill_rule: {:?}, plane_mode: {plane_mode})",
            point_count, fill_rule
        ));
        if had_vec3 {
            ctx.hir.notes.push(
                "polygon3: v0 projection uses x/y components and ignores z during shape lowering"
                    .to_string(),
            );
        }
        Value::Shape(shape)
    }
}

fn parse_axis(ctx: &mut crate::check::Checker, expr: &crate::ast::SExpr) -> Option<u8> {
    match &expr.node {
        crate::ast::Expr::Var(v) if v == "x" || v == "Axis.x" => Some(0),
        crate::ast::Expr::Var(v) if v == "y" || v == "Axis.y" => Some(1),
        _ => {
            ctx.diags.push(
                crate::diag::Diag::error(
                    expr.span.clone(),
                    "axis expects `x` or `y` (or `Axis.x`, `Axis.y`)",
                )
                .with_help("use `along: x` or `along: y`"),
            );
            None
        }
    }
}

fn require_positive_literal_or_dynamic(
    ctx: &mut crate::check::Checker,
    value: &Sx,
    span: &crate::ast::Span,
    what: &str,
) -> Option<()> {
    if let Some(v) = crate::check::Checker::try_eval_static_scalar(value)
        && v <= 0.0
    {
        ctx.diags.push(
            crate::diag::Diag::error(span.clone(), format!("{what} must be > 0"))
                .with_help("use a positive value"),
        );
        return None;
    }
    Some(())
}

builtin! {
    name = "lines",
    signature = single {
        args(
            along: Expr = "axis along which lines are perpendicular (x or y)",
            every: Scalar = "spacing between lines",
            offset: Optional<Scalar> = "phase offset for line positions"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, along, every, offset| {
        let axis = parse_axis(ctx, along)?;
        require_positive_literal_or_dynamic(
            ctx,
            &every,
            &along.span,
            "`lines` spacing",
        )?;
        let offset = offset.unwrap_or(Sx::Lit(0.0));
        let axis_name = if axis == 0 { "x" } else { "y" };
        ctx.hir.notes.push(format!(
            "lines: periodic line family perpendicular to {axis_name}-axis with spacing={:?}, offset={:?} (lipschitz_exact=true)",
            every, offset
        ));
        Value::Shape(ctx.hir.shape(Shape::LineFamily {
            axis,
            spacing: every,
            offset,
        }))
    }
}

builtin! {
    name = "gridline",
    signature = single {
        args(
            along: Expr = "axis along which the single line runs (x or y)",
            at: Scalar = "position of the line along the perpendicular axis"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, along, at| {
        let axis = parse_axis(ctx, along)?;
        let axis_name = if axis == 0 { "x" } else { "y" };
        ctx.hir.notes.push(format!(
            "gridline: single line perpendicular to {axis_name}-axis at position={:?} (lipschitz_exact=true, exact_sdf=true)",
            at
        ));
        Value::Shape(ctx.hir.shape(Shape::GridLine { axis, at }))
    }
}
