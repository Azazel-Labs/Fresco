//! Shape-to-Layer effect builtins (receiver-based).

use crate::builtin;
use crate::check::{Checker, Value};
use crate::hir::{ColorSource, GlowColorSpace, GlowFalloff, Layer, Sx};

const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

builtin! {
    name = "shadow",
    signature = single {
        receiver = Shape,
        args(
            offset: Vec2 = "shadow offset",
            soften: Scalar = "shadow softness radius",
            color: Color = "shadow color"
        ),
        result = Layer,
        caps = PURE,
    },
    check = |ctx, offset, soften, color| {
        let extent = Checker::try_eval_static_scalar(&soften).map(f32::abs);
        ctx.note_wide_effect_if_needed_with_extent(_receiver, "shadow", bag.call_span(), extent);

        Value::Layer(ctx.hir.layer(Layer::Shadow {
            shape: _receiver,
            off: offset,
            soften,
            color,
        }))
    }
}

builtin! {
    name = "glow",
    signature = single {
        receiver = Shape,
        args(
            reach: Expr = "glow reach distance (scalar or vec2)",
            strength: Scalar = "glow strength multiplier",
            color: Optional<ColorExpr> = "glow color or gradient (default: white)",
            falloff: Optional<Expr> = "glow falloff (`exp`, `gaussian`, or `linear`)",
            color_space: Optional<Expr> = "glow gradient anchoring (`scene`, `shape`, or `glow`)"
        ),
        result = Layer,
        caps = PURE,
    },
    check = |ctx, reach, strength, color, falloff, color_space| {
        let reach = ctx.as_glow_reach(reach)?;
        let color = match color {
            Some(expr) => ctx.as_color_source(expr)?,
            None => ColorSource::Solid([
                Sx::Lit(WHITE[0]),
                Sx::Lit(WHITE[1]),
                Sx::Lit(WHITE[2]),
                Sx::Lit(WHITE[3]),
            ]),
        };
        let falloff = match falloff {
            Some(expr) => ctx.parse_glow_falloff(expr)?,
            None => GlowFalloff::Exp,
        };
        let color_space = match color_space {
            Some(expr) => ctx.parse_glow_color_space(expr)?,
            None => GlowColorSpace::Scene,
        };

        let extent = Checker::static_glow_reach_extent(&reach);
        ctx.note_wide_effect_if_needed_with_extent(_receiver, "glow", bag.call_span(), extent);

        Value::Layer(ctx.hir.layer(Layer::Glow {
            shape: _receiver,
            reach,
            strength,
            color,
            falloff,
            color_space,
        }))
    }
}

builtin! {
    name = "inner_glow",
    signature = single {
        receiver = Shape,
        args(
            reach: Expr = "inner glow reach distance (scalar or vec2)",
            strength: Scalar = "inner glow strength multiplier",
            color: Optional<ColorExpr> = "inner glow color or gradient (default: white)",
            falloff: Optional<Expr> = "inner glow falloff (`exp`, `gaussian`, or `linear`)",
            color_space: Optional<Expr> = "inner glow gradient anchoring (`scene`, `shape`, or `glow`)"
        ),
        result = Layer,
        caps = PURE,
    },
    check = |ctx, reach, strength, color, falloff, color_space| {
        let reach = ctx.as_glow_reach(reach)?;
        let color = match color {
            Some(expr) => ctx.as_color_source(expr)?,
            None => ColorSource::Solid([
                Sx::Lit(WHITE[0]),
                Sx::Lit(WHITE[1]),
                Sx::Lit(WHITE[2]),
                Sx::Lit(WHITE[3]),
            ]),
        };
        let falloff = match falloff {
            Some(expr) => ctx.parse_glow_falloff(expr)?,
            None => GlowFalloff::Exp,
        };
        let color_space = match color_space {
            Some(expr) => ctx.parse_glow_color_space(expr)?,
            None => GlowColorSpace::Scene,
        };

        let extent = Checker::static_glow_reach_extent(&reach);
        ctx.note_wide_effect_if_needed_with_extent(_receiver, "inner_glow", bag.call_span(), extent);

        Value::Layer(ctx.hir.layer(Layer::InnerGlow {
            shape: _receiver,
            reach,
            strength,
            color,
            falloff,
            color_space,
        }))
    }
}

builtin! {
    name = "bevel",
    signature = single {
        receiver = Shape,
        args(
            width: Optional<Scalar> = "bevel width (default: 2px)",
            light: Optional<Vec2> = "light direction (default: (1, -1))",
            strength: Optional<Scalar> = "bevel strength (default: 1.0)",
            highlight: Optional<Color> = "highlight color (default: white 70%)",
            shadow: Optional<Color> = "shadow color (default: black 60%)"
        ),
        result = Layer,
        caps = PURE,
    },
    check = |ctx, width, light, strength, highlight, shadow| {
        let width = width.unwrap_or(Sx::PxLit(2.0));
        let light = light.unwrap_or((Sx::Lit(1.0), Sx::Lit(-1.0)));
        let strength = strength.unwrap_or(Sx::Lit(1.0));
        let highlight = highlight.unwrap_or([
            Sx::Lit(1.0),
            Sx::Lit(1.0),
            Sx::Lit(1.0),
            Sx::Lit(0.7),
        ]);
        let shadow = shadow.unwrap_or([
            Sx::Lit(0.0),
            Sx::Lit(0.0),
            Sx::Lit(0.0),
            Sx::Lit(0.6),
        ]);

        let extent = Checker::try_eval_static_scalar(&width).map(f32::abs);
        ctx.note_wide_effect_if_needed_with_extent(_receiver, "bevel", bag.call_span(), extent);

        Value::Layer(ctx.hir.layer(Layer::Bevel {
            shape: _receiver,
            width,
            light,
            strength,
            highlight,
            shadow: Box::new(shadow),
        }))
    }
}

builtin! {
    name = "soften",
    signature = single {
        receiver = Shape,
        args(
            radius: Scalar = "soften radius",
            color: Optional<Color> = "soften color (default: white)"
        ),
        result = Layer,
        caps = PURE,
    },
    check = |ctx, radius, color| {
        let color = color.unwrap_or([
            Sx::Lit(WHITE[0]),
            Sx::Lit(WHITE[1]),
            Sx::Lit(WHITE[2]),
            Sx::Lit(WHITE[3]),
        ]);

        let extent = Checker::try_eval_static_scalar(&radius).map(f32::abs);
        ctx.note_wide_effect_if_needed_with_extent(_receiver, "soften", bag.call_span(), extent);

        Value::Layer(ctx.hir.layer(Layer::Soften {
            shape: _receiver,
            radius,
            color,
        }))
    }
}
