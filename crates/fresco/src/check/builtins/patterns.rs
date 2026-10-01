//! Procedural pattern builtins (analytic prefiltering, §3).
//!
//! This module implements spatial patterns (checker, stripe, square wave)
//! with support for analytic prefiltering to eliminate shimmer/aliasing
//! under minification, rotation, and warp.
//!
//! Patterns default to filtered when a footprint is available, and fall back
//! to unfiltered point-sampling when disabled with `|> filtering(off)`.

use crate::builtin;
use crate::check::{Checker, Value};
use crate::hir::Sx;
use crate::{deriv, hir};

/// Helper: compute a simple unfiltered checker pattern.
///
/// checker(x, y) = 0.5 * (1 - s(x) * s(y))
/// where s(x) is the unit square wave: +1 for fract(x) < 0.5, -1 otherwise.
fn unfiltered_checker(x: Sx, y: Sx) -> Sx {
    // s(x) = sign(0.5 - fract(x)) = 2*step(0.5, fract(x)) - 1
    // Simplified: s(x) = 1 - 2*step(0.5, fract(x))

    let fract_x = Sx::Fract(Box::new(x));
    let fract_y = Sx::Fract(Box::new(y));

    // step(0.5, fract) returns 1 if fract >= 0.5, else 0
    let step_x = Sx::Step(Box::new(Sx::Lit(0.5)), Box::new(fract_x));
    let step_y = Sx::Step(Box::new(Sx::Lit(0.5)), Box::new(fract_y));

    // s(x) = 1 - 2*step_x
    let s_x = Sx::Lit(1.0) - Sx::Lit(2.0) * step_x;
    let s_y = Sx::Lit(1.0) - Sx::Lit(2.0) * step_y;

    // s(x) * s(y)
    let product = s_x * s_y;

    // checker = 0.5 * (1 - product)
    let one_minus_prod = Sx::Lit(1.0) - product;
    Sx::Lit(0.5) * one_minus_prod
}

/// Helper: compute a simple unfiltered stripe pattern along x-axis.
///
/// stripe(x) = s(x) where s(x) is the unit square wave.
fn unfiltered_stripe(x: Sx) -> Sx {
    let fract_x = Sx::Fract(Box::new(x));
    let step_x = Sx::Step(Box::new(Sx::Lit(0.5)), Box::new(fract_x));

    // s(x) = 1 - 2*step_x (ranges from -1 to +1)
    // Map to 0..1: (s(x) + 1) / 2
    let s_x = Sx::Lit(1.0) - Sx::Lit(2.0) * step_x;

    // Map to [0, 1]
    Sx::Lit(0.5) * (s_x + Sx::Lit(1.0))
}

/// Screen pixel size in UV-space (independent of local `in space` remaps).
fn screen_px_uv(ctx: &mut Checker) -> Sx {
    let (_res_x, res_y) = ctx.runtime_resolution();
    Sx::Lit(1.0) / Sx::Max(Box::new(res_y), Box::new(Sx::Lit(1.0e-6)))
}

builtin! {
    name = "checker",
    signature = single {
        args(
            scale: Scalar = "checker cell scale (uniform)",
            at: Optional<Vec2> = "center position (default: origin)"
        ),
        result = Scalar,
        caps = PURE,
    },
    check = |ctx, scale, at| {
        // Use uniform scale for now (could extend to vec2 later)
        let scale_x = scale.clone();
        let scale_y = scale;

        // Parse center position (default to origin)
        let (center_x, center_y) = if let Some((cx, cy)) = at {
            (cx, cy)
        } else {
            (Sx::Lit(0.0), Sx::Lit(0.0))
        };

        // Get current coordinate
        let coord_x = Sx::CoordX;
        let coord_y = Sx::CoordY;

        // Transform coordinate: (coord - center) / scale
        let rel_x = coord_x - center_x;
        let rel_y = coord_y - center_y;

        let scaled_x = rel_x / scale_x.clone();
        let scaled_y = rel_y / scale_y.clone();

        // Check filtering state and footprint availability
        let filtering_state = ctx.get_filtering_state();
        let should_filter = match filtering_state {
            hir::FilteringState::ForceOff => false,
            hir::FilteringState::ForceOn => true,
            hir::FilteringState::Auto => ctx.has_footprint(),
        };

        let pattern = if should_filter {
            // Compute footprint from Jacobian-derived span estimates.
            let (j11, j12, j21, j22) = ctx.get_canvas_jacobian();
                let (span_x, span_y) = deriv::footprint_spans(&j11, &j12, &j21, &j22);
                let px_uv = screen_px_uv(ctx);
                let pixel_span_x = span_x * px_uv.clone();
                let pixel_span_y = span_y * px_uv;

                // Convert UV-step spans into local-units per output pixel, then normalize
                // by the checker period to get the filter width in checker-space cells.
                let width_x = pixel_span_x / scale_x;
                let width_y = pixel_span_y / scale_y;

                ctx.hir.notes.push(
                    "pattern: checker (prefiltered analytic, sep-box, ε ≤ 0.02)".to_string()
                );

                deriv::filtered_checker_pattern(scaled_x, scaled_y, width_x, width_y)

        } else {
            ctx.hir.notes.push(
                "pattern: checker (unfiltered point-sample)".to_string()
            );
            unfiltered_checker(scaled_x, scaled_y)
        };

        Value::Coverage(pattern)
    }
}

builtin! {
    name = "stripe",
    signature = discriminated {
        discriminator = "along",
        default = "x",
        variants = {
            "x" => {
                args(
                    scale: Scalar = "stripe period/width",
                    at: Optional<Vec2> = "center position (default: origin)"
                )
            },
            "y" => {
                args(
                    scale: Scalar = "stripe period/width",
                    at: Optional<Vec2> = "center position (default: origin)"
                )
            }
        },
        result = Scalar,
        caps = PURE,
    },
    check = |ctx, variant, args| {
        let scale_arg = args.require("scale", &mut ctx.diags)?;
        let scale = ctx.as_scalar(scale_arg)?;

        // Parse center position (default to origin)
        let (center_x, center_y) = if let Some(at_expr) = args.take("at") {
            ctx.as_vec2(at_expr)?
        } else {
            (Sx::Lit(0.0), Sx::Lit(0.0))
        };

        // Get current coordinate
        let coord_x = Sx::CoordX;
        let coord_y = Sx::CoordY;

        // Select axis and transform
        let axis_coord = match variant {
            "x" => coord_x,
            _ => coord_y, // "y"
        };

        let center = if variant == "x" { center_x } else { center_y };

        let rel = axis_coord - center;
        let scaled = rel / scale.clone();

        // Check filtering state and footprint availability
        let filtering_state = ctx.get_filtering_state();
        let should_filter = match filtering_state {
            hir::FilteringState::ForceOff => false,
            hir::FilteringState::ForceOn => true,
            hir::FilteringState::Auto => ctx.has_footprint(),
        };

        let pattern = if should_filter {
            // Compute footprint from Jacobian
            let (j11, j12, j21, j22) = ctx.get_canvas_jacobian();
                let (span_x, span_y) = deriv::footprint_spans(&j11, &j12, &j21, &j22);

                // Select the appropriate span based on axis direction
                let span = if variant == "x" { span_x } else { span_y };
                let pixel_span = span * screen_px_uv(ctx);

                // Convert UV-step span into local-units per output pixel, then normalize
                // by the stripe period to get the filter width in stripe-space cells.
                let width = pixel_span / scale;

                ctx.hir.notes.push(format!(
                    "pattern: stripe(along: {variant}) (prefiltered analytic, sep-box, ε ≤ 0.02)"
                ));

                deriv::filtered_stripe_pattern(scaled, width)

        } else {
            ctx.hir.notes.push(format!(
                "pattern: stripe(along: {variant}) (unfiltered point-sample)"
            ));
            unfiltered_stripe(scaled)
        };

        Value::Coverage(pattern)
    }
}
