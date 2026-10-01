//! Signal and animation builtins.

use crate::builtin;
use crate::check::{Checker, Value};
use crate::hir::Sx;
use std::f32::consts::TAU;

builtin! {
    name = "age_norm",
    signature = single {
        args(instance: Expr = "scatter instance binding"),
        result = Scalar,
        caps = PURE,
    },
    check = |ctx, instance| {
        let inst_value = ctx.eval(instance)?;
        if !matches!(inst_value, Value::ScatterInstance { .. }) {
            ctx.diags.push(
                crate::diag::Diag::error(
                    instance.span.clone(),
                    "`age_norm` expects a scatter instance binding",
                )
                .with_help("declare one in the `lifetime name: ...` header and call `age_norm(name)`"),
            );
            return None;
        }
        ctx.hir
            .notes
            .push("signal: age_norm lowered from the scatter instance lifecycle phase".to_string());
        Value::Coverage(Sx::ScatterInstanceAgeNorm)
    }
}

builtin! {
    name = "wave",
    signature = discriminated {
        discriminator = "shape",
        default = "sine",
        variants = {
            "sine" => {
                args(
                    period: Scalar @period = "wave period",
                    phase: Scalar = "phase offset",
                    range: Vec2 @range = "output numeric source (lo, hi) or [lo, hi]",
                    ease: Optional<Expr> = "easing shorthand (for example `in_out_quad`)",
                    transition: Optional<Expr> = "phase easing transition family",
                    mode: Optional<Expr> = "phase easing mode (`in|out|in_out|out_in`)"
                )
            },
            "saw" => {
                args(
                    period: Scalar @period = "wave period",
                    phase: Scalar = "phase offset",
                    range: Vec2 @range = "output numeric source (lo, hi) or [lo, hi]",
                    ease: Optional<Expr> = "easing shorthand (for example `in_out_quad`)",
                    transition: Optional<Expr> = "phase easing transition family",
                    mode: Optional<Expr> = "phase easing mode (`in|out|in_out|out_in`)"
                )
            },
            "triangle" => {
                args(
                    period: Scalar @period = "wave period",
                    phase: Scalar = "phase offset",
                    range: Vec2 @range = "output numeric source (lo, hi) or [lo, hi]",
                    ease: Optional<Expr> = "easing shorthand (for example `in_out_quad`)",
                    transition: Optional<Expr> = "phase easing transition family",
                    mode: Optional<Expr> = "phase easing mode (`in|out|in_out|out_in`)"
                )
            },
            "square" => {
                args(
                    period: Scalar @period = "wave period",
                    phase: Scalar = "phase offset",
                    range: Vec2 @range = "output numeric source (lo, hi) or [lo, hi]",
                    ease: Optional<Expr> = "easing shorthand (for example `in_out_quad`)",
                    transition: Optional<Expr> = "phase easing transition family",
                    mode: Optional<Expr> = "phase easing mode (`in|out|in_out|out_in`)"
                )
            }
        },
        result = Scalar,
        caps = PURE,
    },
    check = |ctx, variant, args| {
        let period_arg = args.require("period", &mut ctx.diags)?;
        let period = ctx.as_scalar(period_arg)?;

        let phase = match args.take("phase") {
            Some(e) => ctx.as_scalar(e)?,
            None => Sx::Lit(0.0),
        };

        let range_expr = args.require("range", &mut ctx.diags)?;
        let (lo, hi) = ctx.require_range_pair(range_expr, "`wave(range: ...)`")?;

        let ease_expr = args.take("ease");
        let transition_expr = args.take("transition");
        let mode_expr = args.take("mode");
        let phase_shaping = ease_expr.is_some() || transition_expr.is_some() || mode_expr.is_some();
        let (transition, mode) = if phase_shaping {
            ctx.parse_signal_ease_spec("wave", ease_expr, transition_expr, mode_expr)?
        } else {
            ("linear", "out")
        };

        let phase_time = ctx.runtime_time() / period + phase;

        let phase01_raw = Sx::Fract(Box::new(phase_time.clone()));
        let phase01 = if phase_shaping {
            Checker::sx_ease(phase01_raw, transition, mode)
        } else {
            Sx::Fract(Box::new(phase_time))
        };
        let theta = Sx::Lit(TAU) * phase01.clone();

        let wave01 = match variant {
            "saw" => phase01,
            "triangle" => {
                let saw = phase01;
                let two_saw = Sx::Lit(2.0) * saw;
                let centered = two_saw - Sx::Lit(1.0);
                Sx::Lit(1.0) - Sx::Abs(Box::new(centered))
            }
            "square" => Sx::Lit(0.5) + Sx::Lit(0.5) * Sx::Sign(Box::new(Sx::Sin(Box::new(theta)))),
            _ => {
                // sine (default)
                Sx::Lit(0.5) + Sx::Lit(0.5) * Sx::Sin(Box::new(theta))
            }
        };

        let range = hi - lo.clone();
        let value = lo + range * wave01;

        let ease_note = if phase_shaping {
            format!(", ease: {}", Checker::signal_ease_label(transition, mode))
        } else {
            String::new()
        };

        ctx.hir.notes.push(format!(
            "signal: wave(shape: {variant}{ease_note}) lowered in closed-form over range in v0"
        ));
        Value::Coverage(value)
    }
}

builtin! {
    name = "pulse",
    signature = single {
        args(
            every: Scalar @period = "period",
            width: Scalar = "pulse width fraction",
            ease: Optional<Expr> = "easing shorthand (for example `out_quad`)",
            transition: Optional<Expr> = "easing transition family",
            mode: Optional<Expr> = "easing mode (`in|out|in_out|out_in`)"
        ),
        result = Scalar,
        caps = PURE,
    },
    check = |ctx, every, width, ease, transition, mode| {
        if let Some(v) = Checker::try_eval_static_scalar(&every)
            && v <= 0.0
        {
            ctx.diags.push(
                crate::diag::Diag::error(
                    crate::ast::Span::default(),
                    "`pulse` period `every` must be > 0",
                )
                .with_help("use a positive period, for example `pulse(every: 1.0, width: 0.25)`"),
            );
            return None;
        }
        if let Some(v) = Checker::try_eval_static_scalar(&width)
            && v <= 0.0
        {
            ctx.diags.push(
                crate::diag::Diag::error(
                    crate::ast::Span::default(),
                    "`pulse` width must be > 0",
                )
                .with_help("use a positive width fraction, for example `pulse(every: 1.0, width: 0.25)`"),
            );
            return None;
        }

        let (transition, mode) = ctx.parse_signal_ease_spec(
            "pulse",
            ease,
            transition,
            mode,
        )?;
        let ease_label = Checker::signal_ease_label(transition, mode);

        let phase = Sx::Fract(Box::new(ctx.runtime_time() / every));
        let t = Checker::sx_clamp01(phase / width);
        let value = Sx::Lit(1.0) - Checker::sx_ease(t, transition, mode);

        ctx.hir.notes.push(format!(
            "signal: pulse(ease: {ease_label}) lowered as a closed-form periodic decay window in v0"
        ));
        Value::Coverage(value)
    }
}

builtin! {
    name = "ramp",
    signature = single {
        args(
            from: Scalar = "start time",
            over: Scalar @window = "duration",
            ease: Optional<Expr> = "easing shorthand (for example `out_quad`)",
            transition: Optional<Expr> = "easing transition family",
            mode: Optional<Expr> = "easing mode (`in|out|in_out|out_in`)"
        ),
        result = Scalar,
        caps = PURE,
    },
    check = |ctx, from, over, ease, transition, mode| {
        let (transition, mode) = ctx.parse_signal_ease_spec(
            "ramp",
            ease,
            transition,
            mode,
        )?;
        let ease_label = Checker::signal_ease_label(transition, mode);

        let raw = (ctx.runtime_time() - from) / over;
        let t = Checker::sx_clamp01(raw);
        let value = Checker::sx_ease(t, transition, mode);

        ctx.hir.notes.push(format!(
            "signal: ramp(ease: {ease_label}) lowered as a closed-form one-shot clock ramp in v0"
        ));
        Value::Scalar(value)
    }
}
