use super::*;
use crate::hir::{CellLayout, CellularSpace};

impl Checker {
    pub(super) fn cell_geometry_member(
        &mut self,
        cell: &RepeatCellValue,
        field: &str,
        span: &Span,
    ) -> Option<Value> {
        if cell.every.is_none() {
            self.diags.push(Diag::error(
                span.clone(),
                "cell geometry queries require a `cells(...)` binding",
            ));
            return None;
        }
        let local = (
            Sx::Sub(Box::new(cell.uv.0.clone()), Box::new(Sx::Lit(0.5))),
            Sx::Sub(Box::new(cell.uv.1.clone()), Box::new(Sx::Lit(0.5))),
        );
        match field {
            "local" => Some(Value::Vec2(local)),
            "angle" => Some(Value::Scalar(Sx::Atan2(
                Box::new(local.1),
                Box::new(local.0),
            ))),
            "edge_distance" => Some(Value::Scalar(Sx::CellQuery {
                scope_id: cell.scope_id,
                angle: None,
                inset: Box::new(Sx::Lit(0.0)),
            })),
            _ => unreachable!("cell geometry member selected by caller"),
        }
    }

    pub(super) fn cell_geometry_method(
        &mut self,
        cell: &RepeatCellValue,
        name: &str,
        bag: &mut ArgBag<'_>,
        span: &Span,
    ) -> Option<Value> {
        let Some(every) = cell.every else {
            self.diags.push(Diag::error(
                span.clone(),
                "cell geometry queries require a `cells(...)` binding",
            ));
            return None;
        };
        let value = match name {
            "contour" => {
                let inset = if let Some(expr) = bag.take("inset") {
                    self.as_scalar(expr)?
                } else {
                    Sx::Lit(0.0)
                };
                if Self::try_eval_static_scalar(&inset).is_some_and(|v| !v.is_finite() || v < 0.0) {
                    self.diags.push(Diag::error(
                        span.clone(),
                        "contour inset must be nonnegative",
                    ));
                    return None;
                }
                Value::CellContour {
                    scope_id: cell.scope_id,
                    inset,
                }
            }
            "inset_distance" => {
                let expr = bag.require("by", &mut self.diags)?;
                let inset = self.as_scalar(expr)?;
                Value::Scalar(Sx::Abs(Box::new(Sx::CellQuery {
                    scope_id: cell.scope_id,
                    angle: None,
                    inset: Box::new(inset),
                })))
            }
            "boundary_point" => {
                let expr = bag.require("angle", &mut self.diags)?;
                let angle = self.as_scalar(expr)?;
                let radius = Sx::CellQuery {
                    scope_id: cell.scope_id,
                    angle: Some(Box::new(angle.clone())),
                    inset: Box::new(Sx::Lit(0.0)),
                };
                let component = |center: Sx, direction: Sx, scale: f32| {
                    Sx::Add(
                        Box::new(center),
                        Box::new(Sx::Mul(
                            Box::new(Sx::Mul(Box::new(radius.clone()), Box::new(direction))),
                            Box::new(Sx::Lit(scale)),
                        )),
                    )
                };
                Value::Vec2((
                    component(
                        cell.center.0.clone(),
                        Sx::Cos(Box::new(angle.clone())),
                        every[0],
                    ),
                    component(cell.center.1.clone(), Sx::Sin(Box::new(angle)), every[1]),
                ))
            }
            _ => unreachable!("cell geometry method selected by caller"),
        };
        Some(value)
    }

    pub(super) fn cellular_space(&mut self, bag: &mut ArgBag<'_>) -> Option<CellularSpace> {
        let layout_expr = bag.require("layout", &mut self.diags)?;
        let layout = match &layout_expr.node {
            Expr::Var(name) => match name.as_str() {
                "square" => Some(CellLayout::Square),
                "brick" => Some(CellLayout::Brick),
                "hex" => Some(CellLayout::Hex),
                "jittered" => Some(CellLayout::Jittered),
                "voronoi" => Some(CellLayout::Voronoi),
                _ => None,
            },
            _ => None,
        };
        let Some(layout) = layout else {
            self.diags.push(Diag::error(
                layout_expr.span.clone(),
                "cells layout must be square, brick, hex, jittered, or voronoi",
            ));
            return None;
        };
        let every_expr = bag.require("every", &mut self.diags)?;
        let every = match self.eval(every_expr)? {
            Value::Scalar(v) => (v.clone(), v),
            Value::Vec2(v) => v,
            _ => {
                self.diags.push(Diag::error(
                    every_expr.span.clone(),
                    "cells every must be a scalar or vec2",
                ));
                return None;
            }
        };
        let every = match (
            Self::try_eval_static_scalar(&every.0),
            Self::try_eval_static_scalar(&every.1),
        ) {
            (Some(x), Some(y)) if x.is_finite() && y.is_finite() && x > 0.0 && y > 0.0 => [x, y],
            _ => {
                self.diags.push(Diag::error(
                    every_expr.span.clone(),
                    "cells every must contain positive finite compile-time lengths",
                ));
                return None;
            }
        };
        let seed_expr = bag.require("seed", &mut self.diags)?;
        let seed_sx = self.as_scalar(seed_expr)?;
        let seed = Self::try_eval_static_scalar(&seed_sx);
        let Some(seed) =
            seed.filter(|v| v.is_finite() && *v >= 0.0 && *v <= 65535.0 && v.fract() == 0.0)
        else {
            self.diags.push(Diag::error(
                seed_expr.span.clone(),
                "cells seed must be a compile-time integer in 0..65535",
            ));
            return None;
        };
        let sampling_expr = bag.require("sampling", &mut self.diags)?;
        let samples_axis = match &sampling_expr.node {
            Expr::Var(name) => match name.strip_prefix("CellSampling.").unwrap_or(name) {
                "center" => Some(1),
                "grid2x2" => Some(2),
                "grid3x3" => Some(3),
                "grid4x4" => Some(4),
                _ => None,
            },
            _ => None,
        };
        let Some(samples_axis) = samples_axis else {
            self.diags.push(Diag::error(
                sampling_expr.span.clone(),
                "cells sampling must be center, grid2x2, grid3x3, or grid4x4",
            ));
            return None;
        };
        let irregular = matches!(layout, CellLayout::Jittered | CellLayout::Voronoi);
        let jitter = if irregular {
            let expr = bag.require("jitter", &mut self.diags)?;
            let sx = self.as_scalar(expr)?;
            match Self::try_eval_static_scalar(&sx) {
                Some(v) if v.is_finite() && (0.0..=1.0).contains(&v) => v,
                _ => {
                    self.diags.push(Diag::error(
                        expr.span.clone(),
                        "cells jitter must be a compile-time value in 0..1",
                    ));
                    return None;
                }
            }
        } else {
            0.0
        };
        Some(CellularSpace {
            layout,
            every,
            seed: seed as u32,
            jitter,
            samples_axis,
            cell_scope: None,
        })
    }
}

impl Checker {
    fn contour_enum(
        &mut self,
        bag: &mut ArgBag<'_>,
        name: &str,
        choices: &[&str],
        default: &str,
    ) -> Option<String> {
        let Some(expr) = bag.take(name) else {
            return Some(default.into());
        };
        if let Expr::Var(value) = &expr.node
            && choices.contains(&value.as_str())
        {
            return Some(value.clone());
        }
        self.diags.push(Diag::error(
            expr.span.clone(),
            format!("{name} must be {}", choices.join(" or ")),
        ));
        None
    }

    fn contour_positive(&mut self, value: &Sx, span: &Span, name: &str) -> bool {
        if Self::try_eval_static_scalar(value).is_some_and(|v| !v.is_finite() || v <= 0.0)
            || matches!(value, Sx::PxLit(v) if !v.is_finite() || *v <= 0.0)
            || matches!(value, Sx::Neg(v) if matches!(v.as_ref(), Sx::PxLit(n) if *n >= 0.0))
        {
            self.diags.push(Diag::error(
                span.clone(),
                format!("{name} must be positive and finite"),
            ));
            false
        } else {
            true
        }
    }

    pub(super) fn contour_builtin(
        &mut self,
        name: &str,
        recv: Option<Value>,
        bag: &mut ArgBag<'_>,
        span: &Span,
    ) -> Option<Value> {
        let pixels = |sx: &Sx| {
            let mut found = false;
            sx.walk_preorder(&mut |s| found |= matches!(s, Sx::PxLit(_)));
            found
        };
        let div = |a: Sx, b: Sx| Sx::Div(Box::new(a), Box::new(b));
        let mul = |a: Sx, b: Sx| Sx::Mul(Box::new(a), Box::new(b));
        let sub = |a: Sx, b: Sx| Sx::Sub(Box::new(a), Box::new(b));
        let safe = |a: Sx| Sx::Max(Box::new(a), Box::new(Sx::Lit(1.0e-10)));
        match name {
            "point" => {
                let Some(Value::CellContour { scope_id, inset }) = recv else {
                    unreachable!("contour receiver dispatched");
                };
                let expr = bag.require("at", &mut self.diags)?;
                let at = self.as_scalar(expr)?;
                let query = |channel| Sx::CellContour {
                    scope_id,
                    inset: Box::new(inset.clone()),
                    at: Some(Box::new(at.clone())),
                    channel,
                };
                Some(Value::Vec2((query(6), query(7))))
            }
            "band" => {
                let expr = bag.require("distance", &mut self.diags)?;
                let mut distance = self.as_scalar(expr)?;
                let expr = bag.require("width", &mut self.diags)?;
                let mut width = self.as_scalar(expr)?;
                if !self.contour_positive(&width, span, "band width") {
                    return None;
                }
                let profile = self.contour_enum(bag, "profile", &["solid", "soft"], "solid")?;
                if pixels(&width)
                    && let Sx::CellContour { channel, .. } = &mut distance
                    && *channel == 0
                {
                    *channel = 3;
                    width = div(width, Sx::PxLit(1.0));
                }
                let distance = Sx::Abs(Box::new(distance));
                let mask = if profile == "soft" {
                    let falloff = if let Some(width) = Self::try_eval_static_scalar(&width) {
                        Sx::Lit(2.0 * std::f32::consts::LN_2 / width.max(1.0e-10))
                    } else {
                        div(Sx::Lit(2.0 * std::f32::consts::LN_2), safe(width))
                    };
                    Sx::Exp(Box::new(mul(Sx::Neg(Box::new(distance)), falloff)))
                } else {
                    let ratio = div(distance, safe(width));
                    sub(
                        Sx::Lit(1.0),
                        Sx::SmoothStep(
                            Box::new(Sx::Lit(0.4)),
                            Box::new(Sx::Lit(0.6)),
                            Box::new(ratio),
                        ),
                    )
                };
                Some(Value::Scalar(mask))
            }
            "chase" => {
                let expr = bag.require("along", &mut self.diags)?;
                let Value::CellContour { scope_id, inset } = self.eval(expr)? else {
                    self.diags.push(Diag::error(
                        expr.span.clone(),
                        "chase along must be a contour",
                    ));
                    return None;
                };
                let query = |channel| Sx::CellContour {
                    scope_id,
                    inset: Box::new(inset.clone()),
                    at: None,
                    channel,
                };
                let speed = if let Some(expr) = bag.take("speed") {
                    Some(self.as_scalar(expr)?)
                } else {
                    None
                };
                let lap = if let Some(expr) = bag.take("lap") {
                    Some(self.as_scalar(expr)?)
                } else {
                    None
                };
                let explicit_head = if let Some(expr) = bag.take("head") {
                    Some(self.as_scalar(expr)?)
                } else {
                    None
                };
                if usize::from(speed.is_some())
                    + usize::from(lap.is_some())
                    + usize::from(explicit_head.is_some())
                    != 1
                {
                    self.diags.push(Diag::error(
                        span.clone(),
                        "chase requires exactly one of speed, lap, or head",
                    ));
                    return None;
                }
                let expr = bag.require("tail", &mut self.diags)?;
                let tail = self.as_scalar(expr)?;
                if !self.contour_positive(&tail, span, "chase tail") {
                    return None;
                }
                let motion =
                    self.contour_enum(bag, "motion", &["perimeter", "angular"], "perimeter")?;
                let angular = motion == "angular";
                let pixel_metric = speed.as_ref().is_some_and(&pixels) || pixels(&tail);
                if angular && (pixel_metric || speed.is_some()) {
                    self.diags.push(Diag::error(span.clone(), "angular chase uses lap or head and a fractional tail; pixel speed requires perimeter motion"));
                    return None;
                }
                // A per-time literal is already a displacement signal driven by context(time).
                // Convert its screen units to pixels, then divide by the screen perimeter.
                let metric = if pixel_metric { 3 } else { 0 };
                let length = query(metric + 2);
                let head = if let Some(head) = explicit_head {
                    head
                } else if let Some(speed) = speed {
                    if pixel_metric && !pixels(&speed) {
                        self.diags.push(Diag::error(
                            span.clone(),
                            "pixel tails require a pixel speed or a lap duration",
                        ));
                        return None;
                    }
                    let travel = if pixel_metric {
                        div(speed, Sx::PxLit(1.0))
                    } else {
                        speed
                    };
                    div(travel, safe(length.clone()))
                } else {
                    let lap = lap.expect("one motion mode checked");
                    if !self.contour_positive(&lap, span, "chase lap") {
                        return None;
                    }
                    div(self.runtime_time(), safe(lap))
                };
                let phase = if let Some(expr) = bag.take("phase") {
                    self.as_scalar(expr)?
                } else {
                    Sx::Lit(0.0)
                };
                let direction = self.contour_enum(
                    bag,
                    "direction",
                    &["clockwise", "counterclockwise"],
                    "clockwise",
                )?;
                let progress = if angular {
                    let x = sub(Sx::RepeatCellUvX(scope_id), Sx::Lit(0.5));
                    let y = sub(Sx::RepeatCellUvY(scope_id), Sx::Lit(0.5));
                    div(
                        Sx::Atan2(Box::new(y), Box::new(x)),
                        Sx::Lit(std::f32::consts::TAU),
                    )
                } else {
                    query(metric + 1)
                };
                let gap = if direction == "counterclockwise" {
                    sub(Sx::Add(Box::new(head), Box::new(phase)), progress)
                } else {
                    Sx::Add(
                        Box::new(Sx::Add(Box::new(progress), Box::new(head))),
                        Box::new(phase),
                    )
                };
                let gap = Sx::Fract(Box::new(gap));
                // Bare tails are a fraction of a lap; pixel tails are an actual screen length.
                let tail = if pixels(&tail) {
                    div(div(tail, Sx::PxLit(1.0)), safe(length.clone()))
                } else {
                    tail
                };
                let falloff = if let Some(tail) = Self::try_eval_static_scalar(&tail) {
                    Sx::Lit(std::f32::consts::LN_2 / tail.max(1.0e-10))
                } else {
                    div(Sx::Lit(std::f32::consts::LN_2), safe(tail))
                };
                let decay = mul(Sx::Neg(Box::new(gap)), falloff);
                let mask = Sx::Exp(Box::new(decay));
                let exists = Sx::Step(Box::new(Sx::Lit(1.0e-9)), Box::new(length));
                Some(Value::Scalar(mul(mask, exists)))
            }
            _ => unreachable!("contour builtin dispatched"),
        }
    }
}
