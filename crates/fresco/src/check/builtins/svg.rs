//! SVG import builtins.

use crate::builtin;
use crate::check::Value;
use crate::diag::Diag;
use crate::hir::{FillRule, Shape, Sx};
use crate::registry::{
    BuiltinArgDecl, BuiltinDecl, BuiltinId, BuiltinLowering, BuiltinSignature, PrimitiveType,
    TypeRef,
};
use roxmltree::Document;
use svgtypes::{PathParser, PathSegment};

#[derive(Clone, Copy)]
enum PathNormalizeMode {
    Auto,
    Off,
    On,
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
                Diag::error(
                    expr.span.clone(),
                    "polygon fill_rule expects `non_zero` or `even_odd`",
                )
                .with_help("use `fill_rule: non_zero` or `fill_rule: even_odd`"),
            );
            None
        }
    }
}

fn require_string_literal(
    ctx: &mut crate::check::Checker,
    expr: &crate::ast::SExpr,
    what: &str,
) -> Option<String> {
    match &expr.node {
        crate::ast::Expr::Str(s) => Some(s.clone()),
        _ => {
            ctx.diags.push(
                Diag::error(
                    expr.span.clone(),
                    format!("{what} expects a string literal"),
                )
                .with_help("use a quoted string literal value"),
            );
            None
        }
    }
}

fn parse_view_box_value(value: &str) -> Option<(f32, f32, f32, f32)> {
    let parts = value
        .split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>();
    if parts.len() != 4 {
        return None;
    }
    let min_x = parts[0].parse::<f32>().ok()?;
    let min_y = parts[1].parse::<f32>().ok()?;
    let width = parts[2].parse::<f32>().ok()?;
    let height = parts[3].parse::<f32>().ok()?;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    Some((min_x, min_y, width, height))
}

fn decode_svg_entities(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;
    while i < input.len() {
        let rest = &input[i..];
        if !rest.starts_with('&') {
            let ch = rest.chars().next().expect("valid utf-8 char");
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }

        let semi = rest.find(';');
        if let Some(semi_idx) = semi {
            let entity = &rest[1..semi_idx];
            let decoded = match entity {
                "quot" => Some('"'),
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "apos" => Some('\''),
                _ => {
                    if let Some(hex) = entity
                        .strip_prefix("#x")
                        .or_else(|| entity.strip_prefix("#X"))
                    {
                        u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
                    } else if let Some(dec) = entity.strip_prefix('#') {
                        dec.parse::<u32>().ok().and_then(char::from_u32)
                    } else {
                        None
                    }
                }
            };

            if let Some(ch) = decoded {
                out.push(ch);
                i += semi_idx + 1;
                continue;
            }
        }

        out.push('&');
        i += 1;
    }
    out
}

fn extract_svg_path_data_and_view_box(input: &str) -> (String, Option<(f32, f32, f32, f32)>) {
    let trimmed = input.trim();
    if !trimmed.starts_with('<') {
        return (decode_svg_entities(trimmed), None);
    }

    let Ok(doc) = Document::parse(trimmed) else {
        return (decode_svg_entities(trimmed), None);
    };

    let root = doc.root_element();
    let mut inferred_view_box = None;

    if root.tag_name().name() == "svg"
        && let Some(vb) = root.attribute("viewBox")
    {
        inferred_view_box = parse_view_box_value(vb);
    }

    if root.tag_name().name() == "path"
        && let Some(d) = root.attribute("d")
    {
        return (decode_svg_entities(d), inferred_view_box);
    }

    for node in root.descendants().filter(roxmltree::Node::is_element) {
        if node.tag_name().name() == "path"
            && let Some(d) = node.attribute("d")
        {
            return (decode_svg_entities(d), inferred_view_box);
        }
    }

    (decode_svg_entities(trimmed), inferred_view_box)
}

fn primitives_bounds(primitives: &[crate::hir::PathPrimitive]) -> Option<(f32, f32, f32, f32)> {
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;

    let mut include = |x: f32, y: f32| {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    };

    for p in primitives {
        match *p {
            crate::hir::PathPrimitive::Line { from, to } => {
                include(from.0, from.1);
                include(to.0, to.1);
            }
            crate::hir::PathPrimitive::Quadratic { p0, p1, p2 } => {
                include(p0.0, p0.1);
                include(p1.0, p1.1);
                include(p2.0, p2.1);
            }
            crate::hir::PathPrimitive::Cubic { p0, p1, p2, p3 } => {
                include(p0.0, p0.1);
                include(p1.0, p1.1);
                include(p2.0, p2.1);
                include(p3.0, p3.1);
            }
            crate::hir::PathPrimitive::Arc {
                from,
                center,
                radius,
                ..
            } => {
                include(from.0, from.1);
                include(center.0 - radius, center.1 - radius);
                include(center.0 + radius, center.1 + radius);
            }
        }
    }

    if !min_x.is_finite() || !min_y.is_finite() || !max_x.is_finite() || !max_y.is_finite() {
        return None;
    }

    let w = max_x - min_x;
    let h = max_y - min_y;
    if w <= f32::EPSILON || h <= f32::EPSILON {
        return None;
    }

    Some((min_x, min_y, w, h))
}

fn primitives_need_uv_normalization(primitives: &[crate::hir::PathPrimitive]) -> bool {
    let Some((min_x, min_y, max_x, max_y)) =
        primitives_bounds(primitives).map(|(x, y, w, h)| (x, y, x + w, y + h))
    else {
        return false;
    };
    let eps = 1.0e-4_f32;
    min_x < -eps || min_y < -eps || max_x > 1.0 + eps || max_y > 1.0 + eps
}

fn normalize_primitives_to_view_box(
    primitives: &mut [crate::hir::PathPrimitive],
    min_x: f32,
    min_y: f32,
    width: f32,
    height: f32,
) {
    let width = width.max(f32::EPSILON);
    let height = height.max(f32::EPSILON);

    let norm = |p: (f32, f32)| ((p.0 - min_x) / width, (p.1 - min_y) / height);
    let radius_scale = f32::midpoint(1.0 / width, 1.0 / height);

    for primitive in primitives {
        match primitive {
            crate::hir::PathPrimitive::Line { from, to } => {
                *from = norm(*from);
                *to = norm(*to);
            }
            crate::hir::PathPrimitive::Quadratic { p0, p1, p2 } => {
                *p0 = norm(*p0);
                *p1 = norm(*p1);
                *p2 = norm(*p2);
            }
            crate::hir::PathPrimitive::Cubic { p0, p1, p2, p3 } => {
                *p0 = norm(*p0);
                *p1 = norm(*p1);
                *p2 = norm(*p2);
                *p3 = norm(*p3);
            }
            crate::hir::PathPrimitive::Arc {
                from,
                center,
                radius,
                ..
            } => {
                *from = norm(*from);
                *center = norm(*center);
                *radius *= radius_scale;
            }
        }
    }
}

fn parse_svg_path_contours(
    ctx: &mut crate::check::Checker,
    data: &str,
    span: &crate::ast::Span,
    what: &str,
) -> Option<Vec<Vec<(f32, f32)>>> {
    let mut contours: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut current: Vec<(f32, f32)> = Vec::new();
    let mut cursor = (0.0_f32, 0.0_f32);
    let mut start = (0.0_f32, 0.0_f32);

    for seg_result in PathParser::from(data) {
        let seg = match seg_result {
            Ok(seg) => seg,
            Err(err) => {
                ctx.diags.push(
                    Diag::error(
                        span.clone(),
                        format!("{what} failed to parse SVG path data: {err}"),
                    )
                    .with_help("limit path commands to M, L, H, V, and Z for v1"),
                );
                return None;
            }
        };

        match seg {
            PathSegment::MoveTo { abs, x, y } => {
                if !current.is_empty() {
                    ctx.diags.push(
                        Diag::error(
                            span.clone(),
                            format!(
                                "{what} does not support open subpaths before a new `M` command"
                            ),
                        )
                        .with_help("close each contour with `Z` before starting the next"),
                    );
                    return None;
                }
                cursor = if abs {
                    (x as f32, y as f32)
                } else {
                    (cursor.0 + x as f32, cursor.1 + y as f32)
                };
                start = cursor;
                current.push(cursor);
            }
            PathSegment::LineTo { abs, x, y } => {
                cursor = if abs {
                    (x as f32, y as f32)
                } else {
                    (cursor.0 + x as f32, cursor.1 + y as f32)
                };
                current.push(cursor);
            }
            PathSegment::HorizontalLineTo { abs, x } => {
                cursor = if abs {
                    (x as f32, cursor.1)
                } else {
                    (cursor.0 + x as f32, cursor.1)
                };
                current.push(cursor);
            }
            PathSegment::VerticalLineTo { abs, y } => {
                cursor = if abs {
                    (cursor.0, y as f32)
                } else {
                    (cursor.0, cursor.1 + y as f32)
                };
                current.push(cursor);
            }
            PathSegment::ClosePath { .. } => {
                if current.len() < 3 {
                    ctx.diags.push(
                        Diag::error(
                            span.clone(),
                            format!("{what} contour must contain at least three vertices"),
                        )
                        .with_help("ensure each closed path emits three or more points"),
                    );
                    return None;
                }
                if current.last().is_some_and(|last| {
                    (last.0 - start.0).abs() <= f32::EPSILON
                        && (last.1 - start.1).abs() <= f32::EPSILON
                }) {
                    current.pop();
                }
                contours.push(std::mem::take(&mut current));
                cursor = start;
            }
            _ => {
                ctx.diags.push(
                    Diag::error(
                        span.clone(),
                        format!(
                            "{what} encountered unsupported SVG path command; v1 supports only M, L, H, V, and Z"
                        ),
                    )
                    .with_help(
                        "simplify the path to straight-line segments and explicit close commands",
                    ),
                );
                return None;
            }
        }
    }

    if !current.is_empty() {
        ctx.diags.push(
            Diag::error(
                span.clone(),
                format!("{what} requires closed contours (missing trailing `Z`)"),
            )
            .with_help("close each contour with `Z`"),
        );
        return None;
    }

    if contours.is_empty() {
        ctx.diags.push(
            Diag::error(
                span.clone(),
                format!("{what} did not contain any supported contours"),
            )
            .with_help("provide at least one closed contour"),
        );
        return None;
    }

    Some(contours)
}

fn lower_contour_polygon(
    ctx: &mut crate::check::Checker,
    contours: &[Vec<(f32, f32)>],
) -> Option<Vec<Vec<(Sx, Sx)>>> {
    let mut out: Vec<Vec<(Sx, Sx)>> = Vec::with_capacity(contours.len());
    for contour in contours {
        if contour.len() < 3 {
            ctx.diags.push(
                Diag::error(
                    0..0,
                    "svg contour must contain at least three vertices after normalization",
                )
                .with_help("ensure imported contours contain valid polygon loops"),
            );
            return None;
        }
        let sx_points = contour
            .iter()
            .map(|(x, y)| (Sx::Lit(*x), Sx::Lit(*y)))
            .collect::<Vec<_>>();
        out.push(sx_points);
    }
    Some(out)
}

fn parse_svg_path_primitives(
    ctx: &mut crate::check::Checker,
    data: &str,
    span: &crate::ast::Span,
    what: &str,
) -> Option<Vec<crate::hir::PathPrimitive>> {
    let mut primitives: Vec<crate::hir::PathPrimitive> = Vec::new();
    let mut cursor = (0.0_f32, 0.0_f32);
    let mut start: Option<(f32, f32)> = None;
    let mut last_cubic_ctrl: Option<(f32, f32)> = None;
    let mut saw_subpath = false;

    for seg_result in PathParser::from(data) {
        let seg = match seg_result {
            Ok(seg) => seg,
            Err(err) => {
                ctx.diags.push(
                    Diag::error(
                        span.clone(),
                        format!("{what} failed to parse SVG path data: {err}"),
                    )
                    .with_help("provide valid SVG path data using M/L/H/V/C/S/Z commands"),
                );
                return None;
            }
        };

        match seg {
            PathSegment::MoveTo { abs, x, y } => {
                if saw_subpath {
                    ctx.diags.push(
                        Diag::error(
                            span.clone(),
                            format!("{what} currently supports only a single SVG subpath"),
                        )
                        .with_help("combine the flourish into one continuous path for v1"),
                    );
                    return None;
                }
                cursor = if abs {
                    (x as f32, y as f32)
                } else {
                    (cursor.0 + x as f32, cursor.1 + y as f32)
                };
                start = Some(cursor);
                saw_subpath = true;
                last_cubic_ctrl = None;
            }
            PathSegment::LineTo { abs, x, y } => {
                let from = cursor;
                cursor = if abs {
                    (x as f32, y as f32)
                } else {
                    (cursor.0 + x as f32, cursor.1 + y as f32)
                };
                primitives.push(crate::hir::PathPrimitive::Line { from, to: cursor });
                last_cubic_ctrl = None;
            }
            PathSegment::HorizontalLineTo { abs, x } => {
                let from = cursor;
                cursor = if abs {
                    (x as f32, cursor.1)
                } else {
                    (cursor.0 + x as f32, cursor.1)
                };
                primitives.push(crate::hir::PathPrimitive::Line { from, to: cursor });
                last_cubic_ctrl = None;
            }
            PathSegment::VerticalLineTo { abs, y } => {
                let from = cursor;
                cursor = if abs {
                    (cursor.0, y as f32)
                } else {
                    (cursor.0, cursor.1 + y as f32)
                };
                primitives.push(crate::hir::PathPrimitive::Line { from, to: cursor });
                last_cubic_ctrl = None;
            }
            PathSegment::CurveTo {
                abs,
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                let p0 = cursor;
                let p1 = if abs {
                    (x1 as f32, y1 as f32)
                } else {
                    (cursor.0 + x1 as f32, cursor.1 + y1 as f32)
                };
                let p2 = if abs {
                    (x2 as f32, y2 as f32)
                } else {
                    (cursor.0 + x2 as f32, cursor.1 + y2 as f32)
                };
                let p3 = if abs {
                    (x as f32, y as f32)
                } else {
                    (cursor.0 + x as f32, cursor.1 + y as f32)
                };
                primitives.push(crate::hir::PathPrimitive::Cubic { p0, p1, p2, p3 });
                cursor = p3;
                last_cubic_ctrl = Some(p2);
            }
            PathSegment::SmoothCurveTo { abs, x2, y2, x, y } => {
                let p0 = cursor;
                let p1 = last_cubic_ctrl
                    .map(|ctrl| (2.0 * cursor.0 - ctrl.0, 2.0 * cursor.1 - ctrl.1))
                    .unwrap_or(cursor);
                let p2 = if abs {
                    (x2 as f32, y2 as f32)
                } else {
                    (cursor.0 + x2 as f32, cursor.1 + y2 as f32)
                };
                let p3 = if abs {
                    (x as f32, y as f32)
                } else {
                    (cursor.0 + x as f32, cursor.1 + y as f32)
                };
                primitives.push(crate::hir::PathPrimitive::Cubic { p0, p1, p2, p3 });
                cursor = p3;
                last_cubic_ctrl = Some(p2);
            }
            PathSegment::ClosePath { .. } => {
                if let Some(start_point) = start
                    && ((cursor.0 - start_point.0).abs() > f32::EPSILON
                        || (cursor.1 - start_point.1).abs() > f32::EPSILON)
                {
                    primitives.push(crate::hir::PathPrimitive::Line {
                        from: cursor,
                        to: start_point,
                    });
                }
                cursor = start.unwrap_or(cursor);
                last_cubic_ctrl = None;
            }
            _ => {
                ctx.diags.push(
                    Diag::error(
                        span.clone(),
                        format!(
                            "{what} encountered unsupported SVG path command; v1 supports M, L, H, V, C, S, and Z"
                        ),
                    )
                    .with_help("simplify the path or limit it to M/L/H/V/C/S/Z commands"),
                );
                return None;
            }
        }
    }

    Some(primitives)
}

pub fn path_svg_check_impl(
    ctx: &mut crate::check::Checker,
    bag: &mut crate::check::ArgBag<'_>,
) -> Option<Value> {
    let path_expr = bag.require("path", &mut ctx.diags)?;
    let path_src = require_string_literal(ctx, path_expr, "`path_svg`")?;
    let view_box_expr = bag.take_named("view_box");
    let normalize_expr = bag.take_named("normalize");

    let (path_data, inferred_view_box) = extract_svg_path_data_and_view_box(&path_src);
    let explicit_view_box = if let Some(vb_expr) = view_box_expr {
        let vb_src = require_string_literal(ctx, vb_expr, "`path_svg(view_box: ...)`")?;
        let Some(vb) = parse_view_box_value(&vb_src) else {
            ctx.diags.push(
                Diag::error(
                    vb_expr.span.clone(),
                    "`path_svg(view_box: ...)` must be `minX minY width height` with positive width/height",
                )
                .with_help("example: view_box: \"0 0 680 210\""),
            );
            return Some(Value::Error);
        };
        Some(vb)
    } else {
        None
    };

    let normalize_mode = if let Some(n_expr) = normalize_expr {
        let n_value = ctx.eval(n_expr)?;
        let Some((n_sx, _)) = crate::check::Checker::as_numeric_scalar(&n_value) else {
            if !matches!(n_value, Value::Error) {
                ctx.diags.push(
                    Diag::error(
                        n_expr.span.clone(),
                        format!(
                            "argument `normalize` to `path_svg` expected scalar, found {}",
                            n_value.kind()
                        ),
                    )
                    .with_help("use `normalize: 0` (off) or `normalize: 1` (on)"),
                );
            }
            return Some(Value::Error);
        };
        let Some(v) = crate::check::Checker::try_eval_static_scalar(&n_sx) else {
            ctx.diags.push(
                Diag::error(
                    n_expr.span.clone(),
                    "`path_svg(normalize: ...)` requires a compile-time scalar",
                )
                .with_help("use `normalize: 0` or `normalize: 1`"),
            );
            return Some(Value::Error);
        };
        if (v.round() as i32) == 0 {
            PathNormalizeMode::Off
        } else {
            PathNormalizeMode::On
        }
    } else {
        PathNormalizeMode::Auto
    };

    let mut primitives = parse_svg_path_primitives(ctx, &path_data, &path_expr.span, "`path_svg`")?;

    let should_normalize = match normalize_mode {
        PathNormalizeMode::On => true,
        PathNormalizeMode::Off => false,
        PathNormalizeMode::Auto => primitives_need_uv_normalization(&primitives),
    };

    if should_normalize
        && let Some((vx, vy, vw, vh)) = explicit_view_box
            .or(inferred_view_box)
            .or_else(|| primitives_bounds(&primitives))
    {
        normalize_primitives_to_view_box(&mut primitives, vx, vy, vw, vh);
        let source = if explicit_view_box.is_some() {
            "explicit view_box"
        } else if inferred_view_box.is_some() {
            "inferred SVG viewBox"
        } else {
            "primitive AABB"
        };
        ctx.hir
            .notes
            .push(format!("path_svg: normalized to uv using {source}"));
    }

    let mut sampling = crate::hir::PathSamplingPolicy::default();
    let samples_expr = bag.take_named("samples");
    let tolerance_expr = bag.take_named("tolerance");
    let max_subdivisions_expr = bag.take_named("max_subdivisions");
    let preserve_cubics_expr = bag.take_named("preserve_cubics");

    if samples_expr.is_some() && tolerance_expr.is_some() {
        ctx.diags.push(
            Diag::error(
                path_expr.span.clone(),
                "`path_svg` accepts either `samples` or `tolerance`, not both",
            )
            .with_help(
                "use `samples: N` for fixed sampling, or `tolerance: X` (+ optional `max_subdivisions`) for adaptive sampling",
            ),
        );
        return Some(Value::Error);
    }

    if tolerance_expr.is_none()
        && let Some(max_subdivisions) = max_subdivisions_expr.as_ref()
    {
        ctx.diags.push(
            Diag::error(
                max_subdivisions.span.clone(),
                "`path_svg(max_subdivisions: ...)` requires `tolerance`",
            )
            .with_help("add `tolerance: ...` or remove `max_subdivisions`"),
        );
        return Some(Value::Error);
    }

    if let Some(samples_expr) = samples_expr {
        let samples_value = ctx.eval(samples_expr)?;
        let Some((samples_sx, _)) = crate::check::Checker::as_numeric_scalar(&samples_value) else {
            if !matches!(samples_value, Value::Error) {
                ctx.diags.push(
                    Diag::error(
                        samples_expr.span.clone(),
                        format!(
                            "argument `samples` to `path_svg` expected scalar, found {}",
                            samples_value.kind()
                        ),
                    )
                    .with_label("type mismatch in path_svg sampling argument"),
                );
            }
            return Some(Value::Error);
        };
        let Some(samples_raw) = crate::check::Checker::try_eval_static_scalar(&samples_sx) else {
            ctx.diags.push(
                Diag::error(
                    samples_expr.span.clone(),
                    "`path_svg(samples: ...)` requires a compile-time constant scalar",
                )
                .with_help("use a numeric literal such as `samples: 6`"),
            );
            return Some(Value::Error);
        };
        let samples = samples_raw.round() as i32;
        if samples < 1 {
            ctx.diags.push(
                Diag::error(
                    samples_expr.span.clone(),
                    "`path_svg(samples: ...)` must be at least 1",
                )
                .with_help("use a small positive integer such as `samples: 6`"),
            );
            return Some(Value::Error);
        }
        sampling.cubic = crate::hir::CubicSamplingPolicy::FixedSamples(samples as usize);
    }

    if let Some(tolerance_expr) = tolerance_expr {
        let tolerance_value = ctx.eval(tolerance_expr)?;
        let Some((tol_sx, _)) = crate::check::Checker::as_numeric_scalar(&tolerance_value) else {
            if !matches!(tolerance_value, Value::Error) {
                ctx.diags.push(
                    Diag::error(
                        tolerance_expr.span.clone(),
                        format!(
                            "argument `tolerance` to `path_svg` expected scalar, found {}",
                            tolerance_value.kind()
                        ),
                    )
                    .with_label("type mismatch in path_svg sampling argument"),
                );
            }
            return Some(Value::Error);
        };
        let Some(tolerance) = crate::check::Checker::try_eval_static_scalar(&tol_sx) else {
            ctx.diags.push(
                Diag::error(
                    tolerance_expr.span.clone(),
                    "`path_svg(tolerance: ...)` requires a compile-time constant scalar",
                )
                .with_help("use a numeric literal such as `tolerance: 1.0`"),
            );
            return Some(Value::Error);
        };
        if !(tolerance.is_finite() && tolerance > 0.0) {
            ctx.diags.push(
                Diag::error(
                    tolerance_expr.span.clone(),
                    "`path_svg(tolerance: ...)` must be a finite scalar > 0",
                )
                .with_help("use a positive value such as `tolerance: 1.0`"),
            );
            return Some(Value::Error);
        }

        let mut max_depth: usize = 10;
        if let Some(max_subdivisions_expr) = max_subdivisions_expr {
            let max_value = ctx.eval(max_subdivisions_expr)?;
            let Some((max_sx, _)) = crate::check::Checker::as_numeric_scalar(&max_value) else {
                if !matches!(max_value, Value::Error) {
                    ctx.diags.push(
                        Diag::error(
                            max_subdivisions_expr.span.clone(),
                            format!(
                                "argument `max_subdivisions` to `path_svg` expected scalar, found {}",
                                max_value.kind()
                            ),
                        )
                        .with_label("type mismatch in path_svg sampling argument"),
                    );
                }
                return Some(Value::Error);
            };
            let Some(max_raw) = crate::check::Checker::try_eval_static_scalar(&max_sx) else {
                ctx.diags.push(
                    Diag::error(
                        max_subdivisions_expr.span.clone(),
                        "`path_svg(max_subdivisions: ...)` requires a compile-time constant scalar",
                    )
                    .with_help("use a numeric literal such as `max_subdivisions: 10`"),
                );
                return Some(Value::Error);
            };
            let max_i = max_raw.round() as i32;
            if max_i < 1 {
                ctx.diags.push(
                    Diag::error(
                        max_subdivisions_expr.span.clone(),
                        "`path_svg(max_subdivisions: ...)` must be at least 1",
                    )
                    .with_help("use a positive integer like `max_subdivisions: 10`"),
                );
                return Some(Value::Error);
            }
            max_depth = max_i as usize;
        }

        sampling.cubic = crate::hir::CubicSamplingPolicy::Adaptive {
            tolerance,
            max_depth,
        };
    }

    if let Some(preserve_expr) = preserve_cubics_expr {
        let preserve_value = ctx.eval(preserve_expr)?;
        let preserve = match preserve_value {
            Value::Scalar(sx) => {
                let Some(v) = crate::check::Checker::try_eval_static_scalar(&sx) else {
                    ctx.diags.push(
                        Diag::error(
                            preserve_expr.span.clone(),
                            "`path_svg(preserve_cubics: ...)` requires a compile-time scalar",
                        )
                        .with_help("use `preserve_cubics: 1` to keep cubics, or `0` to preprocess"),
                    );
                    return Some(Value::Error);
                };
                (v.round() as i32) != 0
            }
            Value::Error => return Some(Value::Error),
            _ => {
                ctx.diags.push(
                    Diag::error(
                        preserve_expr.span.clone(),
                        "argument `preserve_cubics` to `path_svg` expects a scalar",
                    )
                    .with_help("use `preserve_cubics: 1` or `preserve_cubics: 0`"),
                );
                return Some(Value::Error);
            }
        };
        sampling.preserve_cubics = preserve;
    }

    let path = ctx.register_path_profile_from_primitives_with_sampling(
        primitives,
        sampling,
        &path_expr.span,
        "`path_svg`",
    )?;
    let cubic_mode = match sampling.cubic {
        crate::hir::CubicSamplingPolicy::FixedSamples(samples) => {
            format!("fixed samples: {samples}")
        }
        crate::hir::CubicSamplingPolicy::Adaptive {
            tolerance,
            max_depth,
        } => {
            format!("adaptive tolerance: {tolerance:.4}, max_subdivisions: {max_depth}")
        }
    };
    ctx.hir.notes.push(format!(
        "path_svg: preserved SVG path commands as native path primitives (length ~ {:.4}, {}, preserve_cubics={})",
        crate::check::Checker::try_eval_static_scalar(&path.total_length_sx()).unwrap_or_default(),
        cubic_mode,
        if sampling.preserve_cubics { "on" } else { "off" }
    ));
    Some(Value::PathFuture(path))
}

inventory::submit! {
    BuiltinDecl {
        id: BuiltinId("path_svg"),
        name: "path_svg",
        discriminator: None,
        signature: BuiltinSignature {
            receiver: None,
            args: &[
                BuiltinArgDecl::required(
                    "path",
                    TypeRef::Primitive(PrimitiveType::Expr),
                    "SVG path data literal",
                ),
                BuiltinArgDecl::optional(
                    "samples",
                    TypeRef::Primitive(PrimitiveType::Scalar),
                    "compile-time cubic subdivision count for lazy flattening",
                ),
                BuiltinArgDecl::optional(
                    "tolerance",
                    TypeRef::Primitive(PrimitiveType::Scalar),
                    "adaptive cubic flatness tolerance; derives subdivision count",
                ),
                BuiltinArgDecl::optional(
                    "max_subdivisions",
                    TypeRef::Primitive(PrimitiveType::Scalar),
                    "adaptive recursion cap for cubic subdivision; requires `tolerance`",
                ),
                BuiltinArgDecl::optional(
                    "preserve_cubics",
                    TypeRef::Primitive(PrimitiveType::Scalar),
                    "when nonzero, keep cubic primitives for cubic-native lowering",
                ),
                BuiltinArgDecl::optional(
                    "normalize",
                    TypeRef::Primitive(PrimitiveType::Scalar),
                    "0 disables uv normalization, 1 forces uv normalization, omitted = auto",
                ),
                BuiltinArgDecl::optional(
                    "view_box",
                    TypeRef::Primitive(PrimitiveType::Expr),
                    "optional `minX minY width height` box used for normalization",
                ),
            ],
            result: TypeRef::Primitive(PrimitiveType::Path),
            result_alternatives: &[],
            caps: crate::builtin_catalog::BuiltinCaps::from_bits_retain(
                crate::builtin_catalog::BuiltinCaps::PURE.bits()
                    | crate::builtin_catalog::BuiltinCaps::SCALAR_OUT.bits(),
            ),
        },
        lowering: BuiltinLowering::Impl(path_svg_check_impl),
        docs: "Parse SVG path data into a Fresco path.",
    }
}

builtin! {
    name = "svg_path",
    signature = single {
        args(
            path: Expr = "SVG path data literal",
            fill_rule: Optional<Expr> = "fill rule (`non_zero` or `even_odd`)"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, path, fill_rule| {
        let fill_rule = parse_fill_rule(ctx, fill_rule)?;
        let path_src = require_string_literal(ctx, path, "`svg_path`")?;
        let contours = parse_svg_path_contours(ctx, &path_src, &path.span, "`svg_path`")?;
        let contour_count = contours.len();
        let contours = lower_contour_polygon(ctx, &contours)?;
        let shape = ctx.hir.shape(Shape::Polygon { contours, fill_rule });
        ctx.hir.notes.push(format!(
            "svg_path: lowered {} contour(s) as contour polygon (fill_rule: {:?})",
            contour_count, fill_rule
        ));
        Value::Shape(shape)
    }
}

builtin! {
    name = "svg",
    signature = single {
        args(
            source: Expr = "SVG document literal",
            fill_rule: Optional<Expr> = "fill rule (`non_zero` or `even_odd`)"
        ),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, source, fill_rule| {
        let fill_rule = parse_fill_rule(ctx, fill_rule)?;
        let source_text = require_string_literal(ctx, source, "`svg`")?;

        let doc = match Document::parse(&source_text) {
            Ok(doc) => doc,
            Err(err) => {
                ctx.diags.push(
                    Diag::error(source.span.clone(), format!("`svg` failed to parse XML: {err}"))
                        .with_help("provide a valid SVG XML document string"),
                );
                return None;
            }
        };

        let root = doc.root_element();
        if root.tag_name().name() != "svg" {
            ctx.diags.push(
                Diag::error(source.span.clone(), "`svg` expects an `<svg ...>` root element")
                    .with_help("wrap imported paths in a top-level `<svg>` element"),
            );
            return None;
        }

        let mut view_box: Option<(f32, f32, f32, f32)> = None;
        for attr in root.attributes() {
            let name = attr.name();
            match name {
                "viewBox" => {
                    let Some(vb) = parse_view_box_value(attr.value()) else {
                        ctx.diags.push(
                            Diag::error(
                                source.span.clone(),
                                "`svg` viewBox must be `minX minY width height` with positive width/height",
                            )
                            .with_help("example: viewBox='0 0 100 100'"),
                        );
                        return None;
                    };
                    view_box = Some(vb);
                }
                "xmlns" | "width" | "height" => {}
                other => {
                    ctx.diags.push(
                        Diag::error(
                            source.span.clone(),
                            format!("`svg` does not support root attribute `{other}` in v1"),
                        )
                        .with_help("supported root attributes: viewBox, width, height, xmlns"),
                    );
                    return None;
                }
            }
        }

        let mut contours: Vec<Vec<(f32, f32)>> = Vec::new();
        for node in root.descendants().filter(roxmltree::Node::is_element) {
            if node == root {
                continue;
            }

            let tag = node.tag_name().name();
            match tag {
                "g" => {
                    if node.attributes().len() > 0 {
                        ctx.diags.push(
                            Diag::error(
                                source.span.clone(),
                                "`svg` currently supports `<g>` only without attributes",
                            )
                            .with_help("remove group attributes such as transform/style in v1"),
                        );
                        return None;
                    }
                }
                "path" => {
                    let mut d_value: Option<&str> = None;
                    for attr in node.attributes() {
                        match attr.name() {
                            "d" => d_value = Some(attr.value()),
                            "fill-rule" => match attr.value() {
                                "nonzero" | "evenodd" => {}
                                other => {
                                    ctx.diags.push(
                                        Diag::error(
                                            source.span.clone(),
                                            format!(
                                                "`svg` unsupported fill-rule value `{other}` on `<path>`"
                                            ),
                                        )
                                        .with_help("supported fill-rule values: nonzero, evenodd"),
                                    );
                                    return None;
                                }
                            },
                            other => {
                                ctx.diags.push(
                                    Diag::error(
                                        source.span.clone(),
                                        format!(
                                            "`svg` unsupported `<path>` attribute `{other}` in v1"
                                        ),
                                    )
                                    .with_help("supported path attributes: d, fill-rule"),
                                );
                                return None;
                            }
                        }
                    }

                    let Some(d) = d_value else {
                        ctx.diags.push(
                            Diag::error(
                                source.span.clone(),
                                "`svg` path element is missing required `d` attribute",
                            )
                            .with_help("add path geometry data via `d='M ... Z'`"),
                        );
                        return None;
                    };

                    let mut parsed = parse_svg_path_contours(ctx, d, &source.span, "`svg`")?;
                    contours.append(&mut parsed);
                }
                other => {
                    ctx.diags.push(
                        Diag::error(
                            source.span.clone(),
                            format!("`svg` does not support `<{other}>` elements in v1"),
                        )
                        .with_help("supported elements: svg, g, path"),
                    );
                    return None;
                }
            }
        }

        if contours.is_empty() {
            ctx.diags.push(
                Diag::error(
                    source.span.clone(),
                    "`svg` did not contain any supported `<path>` geometry",
                )
                .with_help("add at least one `<path d='...'>` element"),
            );
            return None;
        }

        if let Some((min_x, min_y, width, height)) = view_box {
            for contour in &mut contours {
                for point in contour {
                    point.0 = (point.0 - min_x) / width;
                    point.1 = (point.1 - min_y) / height;
                }
            }
        }

        let contour_count = contours.len();
        let contours = lower_contour_polygon(ctx, &contours)?;
        let shape = ctx.hir.shape(Shape::Polygon { contours, fill_rule });
        ctx.hir.notes.push(format!(
            "svg: lowered {} contour(s) from supported SVG subset as contour polygon (fill_rule: {:?})",
            contour_count, fill_rule
        ));
        if view_box.is_none() {
            ctx.hir.notes
                .push("svg: no viewBox provided; coordinates were used as-authored".to_string());
        }
        Value::Shape(shape)
    }
}
