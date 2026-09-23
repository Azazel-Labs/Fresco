//! Gradient builtin implementation with discriminated signature.

use crate::ast::{Expr, SExpr};
use crate::builtin;
use crate::check::{ArgBag, Checker, GradientKind, GradientStop, Value};
use crate::diag::Diag;
use crate::hir::{GradientAnchor, Sx, V2};

// Gradient builtin using discriminated signature
builtin! {
    name = "gradient",
    signature = discriminated {
        discriminator = "kind",
        default = "linear",
        variants = {
            "linear" => {
                args(
                    along: Vec2 = "direction vector for linear gradient",
                    anchor: Optional<Expr> = "gradient anchor (`scene` or `shape`)"
                )
            },
            "radial" => {
                args(
                    center: Vec2 = "center point of radial gradient",
                    radius: Scalar = "radius of radial gradient"
                )
            }
        },
        result = Gradient,
        caps = PURE,
    },
    check = |ctx, variant, args| {
        // Parse variant-specific arguments
        let kind = match variant {
            "linear" => {
                let along_arg = args.require("along", &mut ctx.diags)?;
                let along = ctx.parse_gradient_along(along_arg)?;
                let anchor = match args.take_named("anchor") {
                    Some(anchor_expr) => ctx.parse_gradient_anchor(anchor_expr)?,
                    None => GradientAnchor::Scene,
                };
                GradientKind::Linear { along, anchor }
            }
            "radial" => {
                let center_arg = args.require("center", &mut ctx.diags)?;
                let radius_arg = args.require("radius", &mut ctx.diags)?;
                let center = ctx.as_vec2(center_arg)?;
                let radius = ctx.as_scalar(radius_arg)?;
                GradientKind::Radial { center, radius }
            }
            _ => unreachable!("variant validation should prevent this"),
        };

        // Parse stops (common to all variants)
        let stops = ctx.parse_gradient_stops_from_args(args)?;

        Value::Gradient { kind, stops }
    }
}

impl Checker {
    fn parse_gradient_stops_from_args(
        &mut self,
        args: &mut ArgBag<'_>,
    ) -> Option<Vec<GradientStop>> {
        if let Some(stops_expr) = args.take_named("stops") {
            return self.parse_gradient_stop_array(stops_expr);
        }

        let start_expr = args.take_named("start");
        let end_expr = args.take_named("end");
        if start_expr.is_some() || end_expr.is_some() {
            let start = start_expr?;
            let end = end_expr?;
            let start = self.as_color_expr(start)?;
            let end = self.as_color_expr(end)?;
            return Some(vec![
                GradientStop {
                    at: Sx::Lit(0.0),
                    color: start,
                },
                GradientStop {
                    at: Sx::Lit(1.0),
                    color: end,
                },
            ]);
        }

        args.require("stops", &mut self.diags)
            .and_then(|stops_expr| self.parse_gradient_stop_array(stops_expr))
    }

    fn parse_gradient_along(&mut self, expr: &SExpr) -> Option<V2> {
        match &expr.node {
            Expr::Var(name) if Self::is_gradient_horizontal_axis(name) => {
                Some((Sx::Lit(1.0), Sx::Lit(0.0)))
            }
            Expr::Var(name) if Self::is_gradient_vertical_axis(name) => {
                Some((Sx::Lit(0.0), Sx::Lit(1.0)))
            }
            _ => self.as_vec2(expr),
        }
    }

    fn parse_gradient_anchor(&mut self, expr: &SExpr) -> Option<GradientAnchor> {
        let Expr::Var(name) = &expr.node else {
            self.diags.push(
                Diag::error(expr.span.clone(), "`anchor:` expects `scene` or `shape`")
                    .with_help("example: `gradient(along: y, anchor: shape, stops: [...])`"),
            );
            return None;
        };

        let symbol = name.rsplit('.').next().unwrap_or(name.as_str());
        match symbol {
            "scene" => Some(GradientAnchor::Scene),
            "shape" => Some(GradientAnchor::Shape),
            "path" => {
                self.diags.push(
                    Diag::error(expr.span.clone(), "`anchor: path` is not implemented yet")
                        .with_help("use `anchor: scene` or `anchor: shape` for now"),
                );
                None
            }
            _ => {
                self.diags.push(
                    Diag::error(
                        expr.span.clone(),
                        format!("unknown gradient anchor `{symbol}`"),
                    )
                    .with_help("use `scene` or `shape`"),
                );
                None
            }
        }
    }

    fn parse_gradient_stop(&mut self, expr: &SExpr) -> Option<GradientStop> {
        let Expr::Call { name, args, .. } = &expr.node else {
            self.diags.push(
                Diag::error(
                    expr.span.clone(),
                    "`stop` entries must be call expressions like `stop(at: 0.4, color: #88aa77)`",
                )
                .with_help("example: `gradient(..., stops: [stop(at: 0.4, color: #88aa77), ...])`"),
            );
            return None;
        };

        if name != "stop" {
            self.diags.push(
                Diag::error(
                    expr.span.clone(),
                    format!("unknown gradient stop constructor `{name}`"),
                )
                .with_help("use `stop(at: <0..1>, color: #rrggbb)`"),
            );
            return None;
        }

        let mut at_expr: Option<&SExpr> = None;
        let mut color_expr: Option<&SExpr> = None;

        for a in args {
            match a.name.as_deref() {
                Some("at") if at_expr.is_none() => at_expr = Some(&a.value),
                Some("color") if color_expr.is_none() => color_expr = Some(&a.value),
                Some("at") | Some("color") => {
                    self.diags.push(
                        Diag::error(a.value.span.clone(), "duplicate gradient stop argument")
                            .with_help("use each of `at` and `color` once"),
                    );
                    return None;
                }
                Some(other) => {
                    self.diags.push(
                        Diag::error(
                            a.value.span.clone(),
                            format!("unknown gradient stop argument `{other}`"),
                        )
                        .with_help("supported stop args: `at`, `color`"),
                    );
                    return None;
                }
                None => {
                    if at_expr.is_none() {
                        at_expr = Some(&a.value);
                    } else if color_expr.is_none() {
                        color_expr = Some(&a.value);
                    } else {
                        self.diags.push(
                            Diag::error(
                                a.value.span.clone(),
                                "too many positional arguments for gradient stop",
                            )
                            .with_help("use `stop(at: <0..1>, color: #rrggbb)`"),
                        );
                        return None;
                    }
                }
            }
        }

        let at_expr = at_expr?;
        let at = self.as_scalar(at_expr)?;
        if let Sx::Lit(v) = at
            && !(0.0..=1.0).contains(&v)
        {
            self.diags.push(
                Diag::error(
                    at_expr.span.clone(),
                    "gradient stop `at` must be within [0,1]",
                )
                .with_help("use normalized stop positions like `0.25`, `0.5`, `0.9`"),
            );
            return None;
        }

        let color = self.as_color_expr(color_expr?)?;
        Some(GradientStop { at, color })
    }

    fn parse_gradient_stop_array(&mut self, expr: &SExpr) -> Option<Vec<GradientStop>> {
        let Expr::Array(items) = &expr.node else {
            self.diags.push(
                Diag::error(
                    expr.span.clone(),
                    "`gradient(stops: ...)` expects an array like `[stop(...), stop(...)]`",
                )
                .with_help(
                    "example: `stops: [stop(at: 0.0, color: #000), stop(at: 1.0, color: #fff)]`",
                ),
            );
            return None;
        };

        if items.is_empty() {
            self.diags.push(
                Diag::error(expr.span.clone(), "`gradient(stops: ...)` cannot be empty")
                    .with_help("provide at least one stop entry"),
            );
            return None;
        }

        let mut out = Vec::with_capacity(items.len());
        for item in items {
            out.push(self.parse_gradient_stop(item)?);
        }
        Some(out)
    }

    fn is_gradient_horizontal_axis(name: &str) -> bool {
        matches!(name, "x" | "horizontal")
    }

    fn is_gradient_vertical_axis(name: &str) -> bool {
        matches!(name, "y" | "vertical")
    }
}
