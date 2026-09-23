//! The analytic rewrite pass (design doc §10) and locality analysis (§6, §11).
//!
//! v0 scope: every builtin except `blur` is *born analytic* (its HIR node
//! already is its rewritten form — Shadow, Glow, Outline). `blur` is the one
//! effect that exists pre-rewrite, so this pass demonstrates the real
//! machinery: `blur(r) ∘ fill(s)  ⇒  soften(fill(s), r)` when the operand is
//! a shape fill, and a *pass-partitioning requirement* otherwise — which the
//! pass partitioner (§11) handles by selecting a kernel strategy and allocating
//! intermediate render targets.

use crate::hir::Sx;
use crate::hir::{Hir, Layer, Locality, Xform};

fn is_synthetic_root_motion_blur(hir: &Hir, layer_id: usize) -> bool {
    layer_id == hir.root && matches!(hir.layers[hir.root], Layer::MotionBlur { .. })
}

fn color_to_expr(color: [f32; 4]) -> [Sx; 4] {
    [
        Sx::Lit(color[0]),
        Sx::Lit(color[1]),
        Sx::Lit(color[2]),
        Sx::Lit(color[3]),
    ]
}

pub fn run(hir: &mut Hir) {
    // --- footprint propagation analysis (§3.8, §24.1) --------------------
    // Analyze the canvas_space transform chain and document how the footprint
    // Jacobian matrix is computed for gradient-driven filtering.
    analyze_footprint_propagation(hir);

    // --- rewrite: blur ∘ fill ⇒ soften (analytic) -----------------------
    // blur(r) over a fill(shape) rewrites to a widened coverage soften (§10.2).
    // blur(r) over a soften(shape) merges the two radii analytically (§10.3).
    // blur(r) over any other layer cannot be rewritten analytically; it is left
    // in HIR as `Layer::Blur` and assigned `Locality::Local` so the pass
    // partitioner (§11) selects a kernel strategy and may allocate an
    // intermediate render target.
    for i in 0..hir.layers.len() {
        let (inner, radius): (usize, _) = match &hir.layers[i] {
            Layer::Blur { inner, radius, .. } => (*inner, radius.clone()),
            _ => continue,
        };
        match hir.layers[inner].clone() {
            Layer::Fill { shape, color } => {
                hir.layers[i] = Layer::Soften {
                    shape,
                    radius,
                    color: color_to_expr(color),
                };
                hir.notes.push(format!(
                    "rewrite: blur(r) ∘ fill(shape#{shape}) ⇒ soften — Sdf-conforming shape, analytic (§10.2, §17.1)"
                ));
            }
            Layer::Soften {
                shape,
                radius: r0,
                color,
            } => {
                // blur(r1) ∘ blur(r2) ⇒ blur(√(r1²+r2²)).
                hir.layers[i] = Layer::Soften {
                    shape,
                    radius: Sx::Sqrt(Box::new(Sx::Add(
                        Box::new(Sx::Mul(Box::new(r0.clone()), Box::new(r0))),
                        Box::new(Sx::Mul(Box::new(radius.clone()), Box::new(radius))),
                    ))),
                    color,
                };
                hir.notes.push(
                    "rewrite: blur ∘ blur merged with exact radius sqrt(r1^2 + r2^2) (§10.3)"
                        .into(),
                );
            }
            _ => {
                // No analytic rewrite applies; leave Layer::Blur for the pass
                // partitioner.  The locality analysis below will mark this
                // layer `Local`, which drives kernel-strategy selection (§11
                // step 3) and intermediate-target allocation.
                hir.notes.push(format!(
                    "pass-partitioner: blur(layer#{inner}) → Local; \
                     kernel strategy selected in pass plan (§11 step 3)"
                ));
            }
        }
    }

    // --- rewrite: user-defined composition rules (§16.1 Phase 5) -----------
    // Walk the layer arena looking for a UserEffect node whose `inner` is
    // also a UserEffect.  For each such pair, search all effect declarations
    // for a matching `rewrite outer(…) compose inner(…) => result(…)` rule.
    // When a rule matches (and its optional `when` guard evaluates to true for
    // the statically-known argument constants), replace the outer node with
    // the result effect.  Only the first matching rule fires per node.
    apply_user_defined_rewrites(hir);

    // --- locality analysis + explain notes -------------------------------
    let mut analytic_shadows = 0usize;
    let mut analytic_shape_glows = 0usize;
    let mut analytic_layer_glows = 0usize;
    let mut analytic_inner_glows = 0usize;
    let mut analytic_bevels = 0usize;
    for l in &hir.layers {
        match l {
            Layer::Shadow { .. } => analytic_shadows += 1,
            Layer::Glow { .. } => analytic_shape_glows += 1,
            Layer::GlowFx { .. } => analytic_layer_glows += 1,
            Layer::InnerGlow { .. } => analytic_inner_glows += 1,
            Layer::Bevel { .. } => analytic_bevels += 1,
            _ => {}
        }
    }

    let mut memo = vec![None; hir.layers.len()];
    let layer_marks: Vec<Locality> = (0..hir.layers.len())
        .map(|i| layer_locality(hir, i, &mut memo))
        .collect();
    hir.layer_locality = layer_marks;

    let mut worst = Locality::Point;
    for (idx, mark) in hir.layer_locality.iter().enumerate() {
        if is_synthetic_root_motion_blur(hir, idx) {
            continue;
        }
        worst = max_locality(worst, *mark);
    }
    let mut point_count = 0usize;
    let mut local_count = 0usize;
    let mut global_count = 0usize;
    for (idx, mark) in hir.layer_locality.iter().enumerate() {
        if is_synthetic_root_motion_blur(hir, idx) {
            continue;
        }
        match mark {
            Locality::Point => point_count += 1,
            Locality::Local => local_count += 1,
            Locality::Global => global_count += 1,
        }
    }
    if analytic_shadows > 0 {
        hir.notes.push(format!(
            "rewrite: shadow lowered analytically ×{analytic_shadows} — soften(fill(translate(s))) (§10.2)"
        ));
    }
    if analytic_shape_glows > 0 {
        hir.notes.push(format!(
            "rewrite: shape glow lowered analytically ×{analytic_shape_glows} — distance-field glow with selectable falloff (§10.2)"
        ));
    }
    if analytic_layer_glows > 0 {
        hir.notes.push(format!(
            "rewrite: layer glow approximation ×{analytic_layer_glows} — single-pass post-layer glow model (§10.2)"
        ));
    }
    if analytic_inner_glows > 0 {
        hir.notes.push(format!(
            "rewrite: inner_glow lowered analytically ×{analytic_inner_glows} — interior distance-field glow with selectable falloff (§10.2)"
        ));
    }
    if analytic_bevels > 0 {
        hir.notes.push(format!(
            "rewrite: bevel lowered analytically ×{analytic_bevels} — edge-band shading from SDF finite-difference normals (§10.2)"
        ));
    }
    if worst == Locality::Point {
        hir.notes.push(
            "locality: all nodes point after rewrites ⇒ 1 fused fragment pass (§11 step 2)".into(),
        );
    } else {
        hir.notes.push(format!(
            "locality: point={point_count}, local={local_count}, global={global_count} (§11 phase-1 persisted marks)"
        ));
        hir.notes.push(format!(
            "pass-plan(v0 estimate): {} pass(es) needed before full partitioner",
            estimated_passes(point_count, local_count, global_count)
        ));
    }
}

fn estimated_passes(point_count: usize, local_count: usize, global_count: usize) -> usize {
    let mut passes = 0usize;
    if point_count > 0 {
        passes += 1;
    }
    if local_count > 0 {
        passes += 1;
    }
    if global_count > 0 {
        passes += 1;
    }
    passes.max(1)
}

fn layer_locality(hir: &Hir, id: usize, memo: &mut [Option<Locality>]) -> Locality {
    if let Some(mark) = memo[id] {
        return mark;
    }

    let mark = match &hir.layers[id] {
        Layer::Solid(_)
        | Layer::ColorExpr { .. }
        | Layer::Grey { .. }
        | Layer::Fill { .. }
        | Layer::FillExpr { .. }
        | Layer::FillGradient { .. }
        | Layer::Shadow { .. }
        | Layer::Glow { .. }
        | Layer::InnerGlow { .. }
        | Layer::Bevel { .. }
        | Layer::Soften { .. }
        | Layer::Image { .. }
        | Layer::ImageAt { .. } => Locality::Point,
        Layer::Blur { .. } | Layer::MotionBlur { .. } => Locality::Local,
        Layer::GlowFx { inner, .. }
        | Layer::Opacity { inner, .. }
        | Layer::Tint { inner, .. }
        | Layer::PostProcess { inner, .. }
        | Layer::InSpace { inner, .. } => layer_locality(hir, *inner, memo),
        Layer::If {
            then_layer,
            else_layer,
            ..
        } => max_locality(
            layer_locality(hir, *then_layer, memo),
            layer_locality(hir, *else_layer, memo),
        ),
        Layer::Compose(entries) => entries.iter().fold(Locality::Point, |acc, (lid, _)| {
            max_locality(acc, layer_locality(hir, *lid, memo))
        }),
        Layer::ScatterBins { body, .. } => layer_locality(hir, *body, memo),
        Layer::UserEffect { def_idx, inner, .. } => {
            // The effect's own locality sets a floor; the piped-in layer may
            // add more.
            let effect_locality = hir
                .effects
                .get(*def_idx)
                .map_or(Locality::Point, |e| e.locality);
            let inner_locality = inner
                .map(|id| layer_locality(hir, id, memo))
                .unwrap_or(Locality::Point);
            max_locality(effect_locality, inner_locality)
        }
    };

    memo[id] = Some(mark);
    mark
}

fn max_locality(a: Locality, b: Locality) -> Locality {
    match (a, b) {
        (Locality::Global, _) | (_, Locality::Global) => Locality::Global,
        (Locality::Local, _) | (_, Locality::Local) => Locality::Local,
        _ => Locality::Point,
    }
}

/// Apply user-defined effect composition rewrite rules (§16.1, Phase 5).
///
/// Scans the HIR layer arena for pairs where an outer `UserEffect` wraps an
/// inner `UserEffect`.  For each such pair the effect declarations are searched
/// for a matching `rewrite outer(holes…) compose inner(holes…) => result(holes…)`
/// rule.  If the pattern matches and the optional `when` guard evaluates to a
/// statically-true constant, the outer node is replaced with the result effect.
///
/// Guards that depend on runtime values (canvas params, coordinates, time) are
/// skipped conservatively; only guards that reduce to a constant after
/// substituting the literal argument values are evaluated.  The first matching
/// rule wins.
fn apply_user_defined_rewrites(hir: &mut Hir) {
    for outer_id in 0..hir.layers.len() {
        // Extract outer UserEffect info — clone to avoid holding a borrow.
        let Layer::UserEffect {
            def_idx: outer_def_idx,
            inner: Some(inner_id),
            args: outer_args,
            span: outer_span,
        } = hir.layers[outer_id].clone()
        else {
            continue;
        };

        // The inner must also be a UserEffect.
        let Layer::UserEffect {
            def_idx: inner_def_idx,
            args: inner_args,
            ..
        } = hir.layers[inner_id].clone()
        else {
            continue;
        };

        let outer_effect_name = match hir.effects.get(outer_def_idx) {
            Some(e) => e.name.clone(),
            None => continue,
        };
        let inner_effect_name = match hir.effects.get(inner_def_idx) {
            Some(e) => e.name.clone(),
            None => continue,
        };

        // Collect all rules from all effect declarations whose pattern matches
        // this outer/inner pair.  Clone the rules to release the HIR borrow.
        let rules: Vec<crate::hir::CompiledRewriteRule> = hir
            .effects
            .iter()
            .flat_map(|e| e.rewrites.iter())
            .filter(|r| r.outer_name == outer_effect_name && r.inner_name == inner_effect_name)
            .cloned()
            .collect();

        for rule in &rules {
            // Build Sx bindings: hole_name → actual arg Sx expression.
            let mut sx_bindings: std::collections::HashMap<String, Sx> =
                std::collections::HashMap::new();
            for (hole, arg) in rule.outer_param_names.iter().zip(outer_args.iter()) {
                sx_bindings.insert(hole.clone(), arg.clone());
            }
            for (hole, arg) in rule.inner_param_names.iter().zip(inner_args.iter()) {
                sx_bindings.insert(hole.clone(), arg.clone());
            }

            // Evaluate the guard expression if present.
            if let Some(guard) = &rule.guard_expr {
                // Extract constant f32 values for each hole's argument.
                // If any argument is not statically constant, skip this rule.
                let mut const_bindings: std::collections::HashMap<String, f32> =
                    std::collections::HashMap::new();
                let mut all_const = true;
                for (hole, arg_sx) in &sx_bindings {
                    match arg_sx.try_eval_with_vars(&std::collections::HashMap::new()) {
                        Some(v) => {
                            const_bindings.insert(hole.clone(), v);
                        }
                        None => {
                            all_const = false;
                            break;
                        }
                    }
                }
                if !all_const {
                    // Guard depends on runtime values — skip conservatively.
                    continue;
                }
                let ref_bindings: std::collections::HashMap<&str, f32> = const_bindings
                    .iter()
                    .map(|(k, v)| (k.as_str(), *v))
                    .collect();
                match guard.try_eval_with_vars(&ref_bindings) {
                    None => continue,      // Guard not evaluable.
                    Some(0.0) => continue, // Guard is false.
                    Some(_) => {}          // Guard is true — proceed.
                }
            }

            // Look up the result effect by name.
            let Some(result_def_idx) = hir.effect_by_name.get(&rule.result_name).copied() else {
                continue;
            };

            // Build result argument list from bindings.
            let result_args: Vec<Sx> = rule
                .result_param_names
                .iter()
                .filter_map(|name| sx_bindings.get(name).cloned())
                .collect();

            // Fire: replace the outer UserEffect with the result effect.
            // The inner layer is no longer referenced after the fusion.
            hir.layers[outer_id] = Layer::UserEffect {
                def_idx: result_def_idx,
                inner: None,
                args: result_args,
                span: outer_span.clone(),
            };

            let guard_note = if rule.guard_expr.is_some() {
                " [guard: true]".to_string()
            } else {
                String::new()
            };
            let tolerance_note = rule
                .tolerance
                .map(|v| format!(" [within {v}]"))
                .unwrap_or_default();
            hir.notes.push(format!(
                "rewrite: {}({}) ∘ {}({}) ⇒ {}({}) — user-defined rule fired (§16.1){guard_note}{tolerance_note}",
                outer_effect_name,
                rule.outer_param_names.join(", "),
                inner_effect_name,
                rule.inner_param_names.join(", "),
                rule.result_name,
                rule.result_param_names.join(", "),
            ));

            break; // First matching rule wins.
        }
    }
}

/// Analyze the canvas_space transform chain and document footprint propagation
/// for gradient-driven filtering (§3.8, §24.1).
///
/// The footprint Jacobian matrix J represents how local space moves per output
/// pixel step. For a chain of transforms `A · B · C`, the chain rule applies
/// right-to-left (innermost to outermost): `J(A · B · C) = J_C · J_B · J_A`.
///
/// This function adds receipt notes documenting the transform chain
/// and how the Jacobian is computed. The actual Jacobian values are computed
/// and wired up in `lower/canvas.rs`.
fn analyze_footprint_propagation(hir: &mut Hir) {
    let Some(ref chain) = hir.canvas_space else {
        // No canvas_space → identity Jacobian (implicit)
        hir.notes.push(
            "footprint: no canvas_space transforms → identity Jacobian (1 output px = 1 local unit)"
                .into(),
        );
        return;
    };

    if chain.is_empty() {
        return;
    }

    // Count transforms by type for summary
    let mut rotate_count = 0usize;
    let mut scale_count = 0usize;
    let mut translate_count = 0usize;
    let mut perspective_count = 0usize;
    let mut repeat_count = 0usize;
    let mut polar_count = 0usize;
    let mut orientation_count = 0usize;
    let mut aspect_count = 0usize;
    let mut other_count = 0usize;

    for xform in chain {
        match xform {
            Xform::Rotate { .. } => rotate_count += 1,
            Xform::Scale { .. } => scale_count += 1,
            Xform::Translate(_) | Xform::Translate3 { .. } => translate_count += 1,
            Xform::Perspective { .. } => perspective_count += 1,
            Xform::RepeatX(_)
            | Xform::RepeatY(_)
            | Xform::Repeat2D { .. }
            | Xform::Cellular(_)
            | Xform::RepeatRadial { .. } => repeat_count += 1,
            Xform::Polar { .. } => polar_count += 1,
            Xform::Orientation { .. } => orientation_count += 1,
            Xform::Aspect(_) | Xform::Centered { .. } => aspect_count += 1,
            _ => other_count += 1,
        }
    }

    // Build summary note
    let mut parts = Vec::new();
    if rotate_count > 0 {
        parts.push(format!("rotate×{rotate_count}"));
    }
    if scale_count > 0 {
        parts.push(format!("scale×{scale_count}"));
    }
    if translate_count > 0 {
        parts.push(format!("translate×{translate_count}"));
    }
    if orientation_count > 0 {
        parts.push(format!("orientation×{orientation_count}"));
    }
    if aspect_count > 0 {
        parts.push(format!("aspect×{aspect_count}"));
    }
    if perspective_count > 0 {
        parts.push(format!("perspective×{perspective_count}"));
    }
    if repeat_count > 0 {
        parts.push(format!("repeat×{repeat_count}"));
    }
    if polar_count > 0 {
        parts.push(format!("polar×{polar_count}"));
    }
    if other_count > 0 {
        parts.push(format!("other×{other_count}"));
    }

    let summary = parts.join(", ");
    let total = chain.len();

    // Receipt header
    hir.notes.push(format!(
        "footprint: canvas_space chain ({total} transform{}) → Jacobian J computed via chain rule (§3.8)",
        if total == 1 { "" } else { "s" }
    ));

    // Transform breakdown
    hir.notes
        .push(format!("footprint: transform breakdown: {summary}"));

    // Jacobian identity table receipt
    let mut identity_transforms = Vec::new();
    let mut analytic_transforms = Vec::new();
    let mut placeholder_transforms = Vec::new();

    if translate_count > 0 {
        identity_transforms.push(format!("translate×{translate_count}"));
    }
    if rotate_count > 0 {
        analytic_transforms.push(format!("rotate×{rotate_count} (R(-θ))"));
    }
    if scale_count > 0 {
        analytic_transforms.push(format!("scale×{scale_count} (diag(1/k))"));
    }
    if orientation_count > 0 {
        analytic_transforms.push(format!(
            "orientation×{orientation_count} (y:down → diag(1,-1))"
        ));
    }
    if aspect_count > 0 {
        analytic_transforms.push(format!("aspect/centered×{aspect_count} (scale-derived)"));
    }
    if repeat_count > 0 {
        identity_transforms.push(format!("repeat×{repeat_count} (piecewise I)"));
    }
    if polar_count > 0 {
        placeholder_transforms.push(format!("polar×{polar_count} (→I, future work)"));
    }
    if perspective_count > 0 {
        placeholder_transforms.push(format!("perspective×{perspective_count} (→I, future work)"));
    }
    if other_count > 0 {
        placeholder_transforms.push(format!("other×{other_count} (→I, future phases)"));
    }

    if !identity_transforms.is_empty() {
        hir.notes.push(format!(
            "footprint: identity J: {} — no local space distortion",
            identity_transforms.join(", ")
        ));
    }
    if !analytic_transforms.is_empty() {
        hir.notes.push(format!(
            "footprint: analytic J: {} — exact derivatives (§3.8 identity table)",
            analytic_transforms.join(", ")
        ));
    }
    if !placeholder_transforms.is_empty() {
        hir.notes.push(format!(
            "footprint: placeholder J: {} — future refinement",
            placeholder_transforms.join(", ")
        ));
    }

    // Computation details
    hir.notes.push(
        "footprint: chain rule application: J(A·B·C) = J_C·J_B·J_A (right-to-left, innermost to outermost)"
            .into(),
    );

    // Final wiring receipt
    hir.notes.push(
        "footprint: Jacobian matrix [[j11, j12], [j21, j22]] computed at root uv and wired up in lowering"
            .into(),
    );
    hir.notes.push(
        "footprint: matrix columns are (j11,j21) right-arrow and (j12,j22) up-arrow — \
         parallelogram is the pixel footprint in local space"
            .into(),
    );
}
