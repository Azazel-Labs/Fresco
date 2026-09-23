//! Effect declaration checking and locality soundness enforcement (§16.1, §16.2).
//!
//! This module handles:
//! - Registering and type-checking user-defined `effect` declarations.
//! - Enforcing the **cheapen-only law**: an effect's body may not call builtins
//!   with a *higher* locality than the effect itself declares.
//! - Checking `rewrite` rules attached to an effect for soundness.

use super::*;

/// Names that are reserved for builtin effects and cannot be redeclared by users.
const BUILTIN_EFFECT_NAMES: &[&str] = &[
    "blur",
    "soften",
    "shadow",
    "glow",
    "inner_glow",
    "bevel",
    "tint",
    "opacity",
    "glow_fx",
    "motion_blur",
    "filtering",
];

impl Checker {
    /// Register and check all user-defined effect declarations.
    ///
    /// This runs before the canvas body is checked so that effect names are
    /// available for pipe-call dispatch inside the canvas.
    pub(super) fn check_effect_decls(&mut self, effects: &[EffectDecl]) {
        for decl in effects {
            self.check_effect_decl(decl);
        }
    }

    fn check_effect_decl(&mut self, decl: &EffectDecl) {
        // --- Guard: builtin name reservation ---------------------------------
        if BUILTIN_EFFECT_NAMES.contains(&decl.name.as_str()) {
            self.diags.push(
                Diag::error(
                    decl.name_span.clone(),
                    format!(
                        "effect name `{}` is reserved for a builtin; user effects may not shadow builtins",
                        decl.name
                    ),
                )
                .with_help("choose a different name for your user-defined effect"),
            );
            return;
        }

        // --- Guard: duplicate effect names -----------------------------------
        if self.effect_defs.contains_key(&decl.name) {
            self.diags.push(
                Diag::error(
                    decl.name_span.clone(),
                    format!("duplicate effect declaration `{}`", decl.name),
                )
                .with_help("effect names must be unique"),
            );
            return;
        }

        // --- Resolve locality -----------------------------------------------
        let locality = match &decl.locality {
            LocalityClass::Point => hir::Locality::Point,
            LocalityClass::Local { .. } => hir::Locality::Local,
            LocalityClass::Global => hir::Locality::Global,
        };

        // --- Collect param metadata -----------------------------------------
        let param_names: Vec<String> = decl.params.iter().map(|p| p.name.clone()).collect();
        let param_types: Vec<String> = decl.params.iter().map(|p| p.ty_name.clone()).collect();

        // --- Check body in a new scope with symbolic param values -----------
        // Push a scope so param names don't leak into the outer canvas scope.
        self.scopes.push(HashMap::new());
        self.style_scopes.push(HashMap::new());

        // Bind each param as a symbolic scalar (Sx::Var) so the body can
        // reference them by name and produce a layer tree with those variables.
        for param in &decl.params {
            let sym = Sx::Var(param.name.clone());
            self.bind(param.name.clone(), Value::Scalar(sym));
        }

        self.bind(
            "self".to_string(),
            Value::ColorField {
                rgba: [
                    Sx::EffectInputChannel {
                        sample_x: Box::new(Sx::CoordX),
                        sample_y: Box::new(Sx::CoordY),
                        channel: 0,
                    },
                    Sx::EffectInputChannel {
                        sample_x: Box::new(Sx::CoordX),
                        sample_y: Box::new(Sx::CoordY),
                        channel: 1,
                    },
                    Sx::EffectInputChannel {
                        sample_x: Box::new(Sx::CoordX),
                        sample_y: Box::new(Sx::CoordY),
                        channel: 2,
                    },
                    Sx::EffectInputChannel {
                        sample_x: Box::new(Sx::CoordX),
                        sample_y: Box::new(Sx::CoordY),
                        channel: 3,
                    },
                ],
                space: ColorSpace::Linear,
            },
        );
        self.bind("coord".to_string(), Value::Vec2((Sx::CoordX, Sx::CoordY)));

        let locality_radius = match &decl.locality {
            LocalityClass::Local { constraint } => {
                constraint.as_ref().and_then(|expr| self.as_scalar(expr))
            }
            _ => None,
        };

        // Evaluate the effect body as a block.  The last expression must be a
        // Layer value; that layer tree becomes the effect's `body_layer`.
        let body_layer = self.eval_block(&decl.body, &decl.name_span, true, false);

        self.scopes.pop();
        self.style_scopes.pop();

        let Some(body_layer_id) = body_layer else {
            if !self.diags.iter().any(|d| d.severity == Severity::Error) {
                self.diags.push(
                    Diag::error(
                        decl.span.clone(),
                        format!("effect `{}` body produces no layer", decl.name),
                    )
                    .with_help("the effect body must end with a layer expression"),
                );
            }
            // Register a stub so downstream code doesn't see the name as
            // unknown and generate a second error.
            let stub_layer = self.hir.layer(Layer::Solid([0.0; 4]));
            self.register_effect_stub(decl, locality, param_names, param_types, stub_layer);
            return;
        };

        // --- Locality soundness check (cheapen-only law) --------------------
        // Walk the body layer subtree and verify no node has higher locality
        // than the effect's declared locality.
        self.check_body_locality(body_layer_id, locality, &decl.span);

        // --- Compile rewrite rules ------------------------------------------
        let compiled_rewrites = self.compile_rewrite_rules(decl, &param_names);

        // --- Register the effect def ----------------------------------------
        let def_idx = self.hir.effects.len();
        self.hir.effects.push(hir::EffectDef {
            name: decl.name.clone(),
            locality,
            locality_radius,
            param_names: param_names.clone(),
            param_types,
            body_layer: body_layer_id,
            rewrites: compiled_rewrites,
            span: decl.span.clone(),
        });
        self.hir.effect_by_name.insert(decl.name.clone(), def_idx);

        self.effect_defs.insert(
            decl.name.clone(),
            EffectRegistered {
                name: decl.name.clone(),
                def_idx,
                param_count: param_names.len(),
                locality,
                param_names,
                span: decl.span.clone(),
            },
        );
    }

    fn register_effect_stub(
        &mut self,
        decl: &EffectDecl,
        locality: hir::Locality,
        param_names: Vec<String>,
        param_types: Vec<String>,
        stub_layer: LayerId,
    ) {
        let def_idx = self.hir.effects.len();
        self.hir.effects.push(hir::EffectDef {
            name: decl.name.clone(),
            locality,
            locality_radius: None,
            param_names: param_names.clone(),
            param_types,
            body_layer: stub_layer,
            rewrites: Vec::new(),
            span: decl.span.clone(),
        });
        self.hir.effect_by_name.insert(decl.name.clone(), def_idx);

        self.effect_defs.insert(
            decl.name.clone(),
            EffectRegistered {
                name: decl.name.clone(),
                def_idx,
                param_count: param_names.len(),
                locality,
                param_names,
                span: decl.span.clone(),
            },
        );
    }

    /// Enforce the cheapen-only law on a body layer subtree.
    ///
    /// A `point` effect may not contain `Local` or `Global` nodes.
    /// A `local` effect may not contain `Global` nodes.
    fn check_body_locality(&mut self, root: LayerId, declared: hir::Locality, span: &Span) {
        // Quick exit: global effects allow everything.
        if declared == hir::Locality::Global {
            return;
        }

        let mut visited = vec![false; self.hir.layers.len()];
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if id >= self.hir.layers.len() || visited[id] {
                continue;
            }
            visited[id] = true;

            let node_locality = match &self.hir.layers[id] {
                Layer::Blur { .. } | Layer::MotionBlur { .. } => hir::Locality::Local,
                Layer::UserEffect { def_idx, .. } => self
                    .hir
                    .effects
                    .get(*def_idx)
                    .map_or(hir::Locality::Point, |e| e.locality),
                _ => hir::Locality::Point,
            };

            if locality_exceeds(node_locality, declared) {
                self.diags.push(
                    Diag::error(
                        span.clone(),
                        format!(
                            "effect body contains a `{}` operation but the effect is declared `{}`; \
                             effects may only call operations with equal or lower locality (cheapen-only law)",
                            locality_name(node_locality),
                            locality_name(declared)
                        ),
                    )
                    .with_help("declare the effect with a higher locality, or remove the offending call"),
                );
                // Report only one violation per effect to avoid noise.
                return;
            }

            // Push children.
            match &self.hir.layers[id].clone() {
                Layer::GlowFx { inner, .. }
                | Layer::Opacity { inner, .. }
                | Layer::Tint { inner, .. }
                | Layer::PostProcess { inner, .. }
                | Layer::InSpace { inner, .. }
                | Layer::Blur { inner, .. }
                | Layer::MotionBlur { inner, .. } => stack.push(*inner),
                Layer::If {
                    then_layer,
                    else_layer,
                    ..
                } => {
                    stack.push(*then_layer);
                    stack.push(*else_layer);
                }
                Layer::Compose(entries) => {
                    for (lid, _) in entries {
                        stack.push(*lid);
                    }
                }
                Layer::ScatterBins { body, .. } => stack.push(*body),
                Layer::UserEffect {
                    inner: Some(inner_id),
                    ..
                } => stack.push(*inner_id),
                _ => {}
            }
        }
    }

    /// Compile the rewrite rules attached to an `effect` declaration.
    fn compile_rewrite_rules(
        &mut self,
        decl: &EffectDecl,
        _effect_param_names: &[String],
    ) -> Vec<hir::CompiledRewriteRule> {
        let mut out = Vec::new();
        for rule in &decl.rewrites {
            // Collect hole names from outer and inner patterns.
            let outer_param_names: Vec<String> =
                rule.lhs.outer.args.iter().map(expr_hole_name).collect();
            let inner_param_names: Vec<String> =
                rule.lhs.inner.args.iter().map(expr_hole_name).collect();
            let result_param_names: Vec<String> =
                rule.result.args.iter().map(expr_hole_name).collect();
            let all_holes: Vec<String> = outer_param_names
                .iter()
                .chain(inner_param_names.iter())
                .cloned()
                .collect();

            // Validate that result holes are all bound on the LHS.
            let all_bound: HashSet<&str> = outer_param_names
                .iter()
                .chain(inner_param_names.iter())
                .map(String::as_str)
                .collect();
            for name in &result_param_names {
                if !all_bound.contains(name.as_str()) {
                    self.diags.push(
                        Diag::error(
                            rule.result.span.clone(),
                            format!(
                                "rewrite result uses unbound hole `{name}`; \
                                 it must be bound in the LHS patterns"
                            ),
                        )
                        .with_help("add the hole to the outer or inner LHS pattern"),
                    );
                }
            }

            // Compile the guard expression to an `Sx` tree with pattern holes
            // represented as `Sx::Var(name)`.  The compiled guard is evaluated
            // at rewrite time after substituting actual argument constant values.
            let tolerance = if let Some(tol_expr) = &rule.tolerance {
                self.scopes.push(HashMap::new());
                self.style_scopes.push(HashMap::new());
                for name in &all_holes {
                    self.bind(name.clone(), Value::Scalar(Sx::Var(name.clone())));
                }
                let tol_eval = self.eval(tol_expr);
                self.scopes.pop();
                self.style_scopes.pop();
                match tol_eval {
                    Some(Value::Scalar(sx)) => {
                        match sx.try_eval_with_vars(&HashMap::new()) {
                            Some(v) if v.is_finite() && v > 0.0 => Some(v),
                            Some(v) => {
                                self.diags.push(
                                    Diag::error(
                                        tol_expr.span.clone(),
                                        format!(
                                            "rewrite tolerance `within` must be a positive finite constant; found {v}"
                                        ),
                                    )
                                    .with_help("use a literal or constant expression greater than 0, e.g. `within 0.5/255`"),
                                );
                                None
                            }
                            None => {
                                self.diags.push(
                                    Diag::error(
                                        tol_expr.span.clone(),
                                        "rewrite tolerance `within` must be compile-time constant",
                                    )
                                    .with_help("remove runtime-dependent values from the tolerance expression"),
                                );
                                None
                            }
                        }
                    }
                    Some(Value::Error) | None => None,
                    Some(other) => {
                        self.diags.push(
                            Diag::error(
                                tol_expr.span.clone(),
                                format!(
                                    "rewrite tolerance `within` must evaluate to a scalar; found {}",
                                    other.kind()
                                ),
                            )
                            .with_help("use a scalar value such as `within 0.5/255`"),
                        );
                        None
                    }
                }
            } else {
                None
            };

            let guard_expr = if let Some(g) = &rule.guard {
                // Push a temporary scope binding each hole name as Sx::Var(name)
                // so the checker can evaluate the guard expression symbolically.
                self.scopes.push(HashMap::new());
                self.style_scopes.push(HashMap::new());
                for name in &all_holes {
                    self.bind(name.clone(), Value::Scalar(Sx::Var(name.clone())));
                }
                let guard_val = self.eval(g);
                self.scopes.pop();
                self.style_scopes.pop();
                match guard_val {
                    Some(Value::Scalar(sx)) => Some(sx),
                    Some(Value::Error) | None => {
                        // Guard evaluation produced an error — treat as never-fire
                        // (conservative; the error was already reported above).
                        Some(Sx::Lit(0.0))
                    }
                    Some(other) => {
                        self.diags.push(
                            Diag::error(
                                g.span.clone(),
                                format!(
                                    "rewrite guard must evaluate to a scalar boolean; found {}",
                                    other.kind()
                                ),
                            )
                            .with_help("use a comparison expression like `a > b`"),
                        );
                        Some(Sx::Lit(0.0))
                    }
                }
            } else {
                None
            };

            out.push(hir::CompiledRewriteRule {
                outer_name: rule.lhs.outer.name.clone(),
                outer_param_names,
                inner_name: rule.lhs.inner.name.clone(),
                inner_param_names,
                result_name: rule.result.name.clone(),
                result_param_names,
                guard_expr,
                tolerance,
            });
        }
        out
    }
}

/// Returns `true` if `a` exceeds the allowed `limit` locality.
fn locality_exceeds(a: hir::Locality, limit: hir::Locality) -> bool {
    match limit {
        hir::Locality::Point => {
            matches!(a, hir::Locality::Local | hir::Locality::Global)
        }
        hir::Locality::Local => matches!(a, hir::Locality::Global),
        hir::Locality::Global => false,
    }
}

fn locality_name(l: hir::Locality) -> &'static str {
    match l {
        hir::Locality::Point => "point",
        hir::Locality::Local => "local",
        hir::Locality::Global => "global",
    }
}

/// Extract the name of a "hole" variable from a rewrite pattern argument
/// expression.  In `rewrite grain(a) compose grain(b) => grain(max(a, b))`,
/// the hole names in the first pattern are `"a"` and `"b"`.
///
/// For v0 we only support plain variable-name holes.
fn expr_hole_name(expr: &SExpr) -> String {
    match &expr.node {
        Expr::Var(name) => name.clone(),
        _ => "<expr>".to_string(),
    }
}
