use super::*;
use tracing::trace_span;

impl Checker {
    fn is_shape_constructor_decl(decl: &crate::registry::BuiltinDecl) -> bool {
        decl.signature.receiver.is_none()
            && decl.signature.result
                == crate::registry::TypeRef::Primitive(crate::registry::PrimitiveType::Shape)
    }

    fn provided_arg_count_for_decl_match(
        decl: &crate::registry::BuiltinDecl,
        args: &[Arg],
    ) -> usize {
        if Self::is_shape_constructor_decl(decl)
            && args.iter().any(|arg| arg.name.as_deref() == Some("rotate"))
        {
            args.len().saturating_sub(1)
        } else {
            args.len()
        }
    }

    fn builtin_accepts_arg_count(sig: &crate::registry::BuiltinSignature, provided: usize) -> bool {
        let required = sig.args.iter().filter(|arg| arg.required).count();
        provided >= required && provided <= sig.args.len()
    }

    fn builtin_decl_accepts_arg_count(
        decl: &crate::registry::BuiltinDecl,
        provided: usize,
    ) -> bool {
        match decl.lowering {
            crate::registry::BuiltinLowering::ImplDiscriminated { default, .. } => {
                let required = decl
                    .signature
                    .args
                    .iter()
                    .filter(|arg| arg.required)
                    .count();
                let discriminator_required = usize::from(default.is_none());
                let min = required + discriminator_required;
                let max = decl.signature.args.len() + 1;
                provided >= min && provided <= max
            }
            _ => Self::builtin_accepts_arg_count(&decl.signature, provided),
        }
    }

    fn wrap_shape_with_rotation(&mut self, value: Value, rotate: Option<Sx>) -> Value {
        let Some(angle) = rotate else {
            return value;
        };
        match value {
            Value::Shape(id) => {
                let out = self.hir.shape(Shape::Rotate { inner: id, angle });
                self.note_shape_exactness(out, "rotate");
                Value::Shape(out)
            }
            other => other,
        }
    }

    fn reduce_op_default_for_seed(seed: &Value) -> &'static str {
        match seed {
            Value::Shape(_) => "subtract",
            Value::Scalar(_) | Value::Distance(_) | Value::Coverage(_) | Value::Mask(_) => "add",
            _ => "add",
        }
    }

    fn reduce_step(
        &mut self,
        op_name: &str,
        acc: Value,
        item: Value,
        span: &Span,
    ) -> Option<Value> {
        match op_name {
            "subtract" | "sub" => match (acc, item) {
                (Value::Shape(a), Value::Shape(b)) => {
                    let out = self.hir.shape(Shape::Subtract(a, b));
                    self.note_shape_exactness(out, "subtract");
                    Some(Value::Shape(out))
                }
                (a, b) => {
                    self.diags.push(
                        Diag::error(
                            span.clone(),
                            format!(
                                "`reduce(..., op: {op_name})` expects shape values, found {} and {}",
                                a.kind(),
                                b.kind()
                            ),
                        )
                        .with_help("use `op: subtract` only with shape arrays and shape seeds"),
                    );
                    None
                }
            },
            "union" => match (acc, item) {
                (Value::Shape(a), Value::Shape(b)) => {
                    let out = self.hir.shape(Shape::Union(a, b));
                    self.note_shape_exactness(out, "union");
                    Some(Value::Shape(out))
                }
                (a, b) => {
                    self.diags.push(
                        Diag::error(
                            span.clone(),
                            format!(
                                "`reduce(..., op: union)` expects shape values, found {} and {}",
                                a.kind(),
                                b.kind()
                            ),
                        )
                        .with_help("use `op: union` with shape arrays and shape seeds"),
                    );
                    None
                }
            },
            "intersect" => match (acc, item) {
                (Value::Shape(a), Value::Shape(b)) => {
                    let out = self.hir.shape(Shape::Intersect(a, b));
                    self.note_shape_exactness(out, "intersect");
                    Some(Value::Shape(out))
                }
                (a, b) => {
                    self.diags.push(
                        Diag::error(
                            span.clone(),
                            format!(
                                "`reduce(..., op: intersect)` expects shape values, found {} and {}",
                                a.kind(),
                                b.kind()
                            ),
                        )
                        .with_help("use `op: intersect` with shape arrays and shape seeds"),
                    );
                    None
                }
            },
            "add" | "mul" | "div" => {
                let Some((sa, ka)) = Self::as_numeric_scalar(&acc) else {
                    self.diags.push(
                        Diag::error(
                            span.clone(),
                            format!(
                                "`reduce(..., op: {op_name})` expects scalar-like values, found {}",
                                acc.kind()
                            ),
                        )
                        .with_help(
                            "use `add|mul|div` with scalar, distance, coverage, or mask values",
                        ),
                    );
                    return None;
                };
                let Some((sb, kb)) = Self::as_numeric_scalar(&item) else {
                    self.diags.push(
                        Diag::error(
                            span.clone(),
                            format!(
                                "`reduce(..., op: {op_name})` expects scalar-like values, found {}",
                                item.kind()
                            ),
                        )
                        .with_help(
                            "use `add|mul|div` with scalar, distance, coverage, or mask values",
                        ),
                    );
                    return None;
                };

                let bin_op = match op_name {
                    "add" => BinOp::Add,
                    "mul" => BinOp::Mul,
                    "div" => BinOp::Div,
                    _ => unreachable!(),
                };

                let Some(out_kind) = Self::promoted_scalar_kind(bin_op, ka, kb) else {
                    self.diags.push(
                        Diag::error(
                            span.clone(),
                            format!(
                                "unsafe cross-kind arithmetic in reduce: {} {op_name} {}",
                                Self::scalar_kind_name(ka),
                                Self::scalar_kind_name(kb)
                            ),
                        )
                        .with_help(
                            "allowed promotions: matching kinds, scalar with any kind, and multiplicative attenuation between distance/coverage/mask",
                        ),
                    );
                    return None;
                };

                let sx = Self::fold_bin_if_literals(bin_op, sa, sb);
                Some(Self::scalar_value_from_kind(out_kind, sx))
            }
            other => {
                self.diags.push(
                    Diag::error(span.clone(), format!("unknown reduce op `{other}`")).with_help(
                        "supported reduce ops: add, mul, div, subtract, union, intersect",
                    ),
                );
                None
            }
        }
    }

    fn builtin_reduce(&mut self, bag: &mut ArgBag<'_>) -> Option<Value> {
        let items_expr = bag.require("items", &mut self.diags)?;
        let seed_expr = bag.require("seed", &mut self.diags)?;
        let op_expr = bag.take_named("op").or_else(|| bag.take("op"));

        let items_val = self.eval(items_expr)?;
        let mut acc = self.eval(seed_expr)?;

        if matches!(items_val, Value::Error) || matches!(acc, Value::Error) {
            return Some(Value::Error);
        }

        let Value::Array(items) = items_val else {
            self.diags.push(
                Diag::error(
                    items_expr.span.clone(),
                    format!(
                        "`reduce(items, seed, ...)` expects `items` to be an array, found {}",
                        items_val.kind()
                    ),
                )
                .with_help("use an array literal or comprehension for `items`"),
            );
            return None;
        };

        let item_count = items.len();

        if let Some(op_expr) = op_expr {
            match &op_expr.node {
                Expr::Var(name) => {
                    for item in items {
                        acc = self.reduce_step(name, acc, item, &seed_expr.span)?;
                    }
                    self.hir.notes.push(format!(
                        "reduce: op={name}, items={item_count}, depth={item_count}"
                    ));
                    return Some(acc);
                }
                _ => {
                    let op_val = self.eval(op_expr)?;
                    if matches!(op_val, Value::Error) {
                        return Some(Value::Error);
                    }
                    let Value::Lambda { params, body } = op_val else {
                        self.diags.push(
                            Diag::error(
                                op_expr.span.clone(),
                                "`reduce(..., op: ...)` expects an operation identifier or lambda",
                            )
                            .with_help(
                                "use one of: add, mul, div, subtract, union, intersect, or `|acc, item| expr`",
                            ),
                        );
                        return None;
                    };

                    if params.len() != 2 {
                        self.diags.push(
                            Diag::error(
                                op_expr.span.clone(),
                                "reduce lambda must have exactly 2 parameters",
                            )
                            .with_help("use `|acc, item| expr`"),
                        );
                        return None;
                    }

                    let acc_name = params[0].clone();
                    let item_name = params[1].clone();
                    for item in items {
                        self.scopes.push(HashMap::new());
                        self.bind(acc_name.clone(), acc.clone());
                        self.bind(item_name.clone(), item);
                        let next = match &body {
                            LambdaBody::Expr(expr) | LambdaBody::BlockReturn(expr) => {
                                self.eval(expr)
                            }
                        };
                        self.scopes.pop();

                        let next = next?;
                        if matches!(next, Value::Error) {
                            return Some(Value::Error);
                        }
                        acc = next;
                    }

                    self.hir.notes.push(format!(
                        "reduce: op=lambda, items={item_count}, depth={item_count}"
                    ));
                    return Some(acc);
                }
            }
        }

        let op_name = Self::reduce_op_default_for_seed(&acc);
        for item in items {
            acc = self.reduce_step(op_name, acc, item, &seed_expr.span)?;
        }

        self.hir.notes.push(format!(
            "reduce: op={op_name}, items={item_count}, depth={item_count}"
        ));

        Some(acc)
    }

    fn shape_lipschitz_exact(&self, id: ShapeId) -> bool {
        match &self.hir.shapes[id] {
            Shape::Circle { .. }
            | Shape::Capsule { .. }
            | Shape::RBox { .. }
            | Shape::Ellipse { .. }
            | Shape::Triangle { .. }
            | Shape::LineFamily { .. }
            | Shape::GridLine { .. } => true,
            Shape::Polygon { .. } => false,
            Shape::Star { .. } => false, // Star uses approximation
            Shape::Outline { inner, .. } => self.shape_lipschitz_exact(*inner),
            Shape::Offset { inner, .. } => self.shape_lipschitz_exact(*inner),
            Shape::Rotate { inner, .. } => self.shape_lipschitz_exact(*inner),
            Shape::Mix(a, b, _) => self.shape_lipschitz_exact(*a) && self.shape_lipschitz_exact(*b),
            Shape::Union(a, b) | Shape::Intersect(a, b) | Shape::Subtract(a, b) => {
                self.shape_lipschitz_exact(*a) && self.shape_lipschitz_exact(*b)
            }
            Shape::SmoothUnion(_, _, _) => false,
        }
    }

    pub(super) fn note_shape_exactness(&mut self, id: ShapeId, kind: &str) {
        let exact = self.shape_lipschitz_exact(id);
        self.hir.notes.push(format!(
            "shape: {kind} produced shape#{id} (lipschitz_exact={exact})"
        ));
    }

    fn shape_distance_error_bound(&self, id: ShapeId) -> Option<f32> {
        match &self.hir.shapes[id] {
            Shape::Circle { .. }
            | Shape::Capsule { .. }
            | Shape::RBox { .. }
            | Shape::Ellipse { .. }
            | Shape::Triangle { .. }
            | Shape::LineFamily { .. }
            | Shape::GridLine { .. } => Some(0.0),
            Shape::Polygon { .. } => Some(0.0),
            Shape::Star { .. } => Some(0.01), // Star has small approximation error
            Shape::Outline { inner, .. } | Shape::Offset { inner, .. } => {
                self.shape_distance_error_bound(*inner)
            }
            Shape::Rotate { inner, .. } => self.shape_distance_error_bound(*inner),
            Shape::Mix(a, b, _) => Some(
                self.shape_distance_error_bound(*a)?
                    .max(self.shape_distance_error_bound(*b)?),
            ),
            Shape::Union(a, b) | Shape::Intersect(a, b) | Shape::Subtract(a, b) => Some(
                self.shape_distance_error_bound(*a)?
                    .max(self.shape_distance_error_bound(*b)?),
            ),
            Shape::SmoothUnion(a, b, k) => {
                let child = self
                    .shape_distance_error_bound(*a)?
                    .max(self.shape_distance_error_bound(*b)?);
                let k = Self::try_eval_static_scalar(k)?.abs();
                // Conservative smooth-union distance error model:
                // For common polynomial smooth-min forms, max deviation from exact min
                // is bounded by roughly k/4. We propagate child error and smoothing error
                // as B_out <= max(B_a, B_b) + k/4.
                Some(child + 0.25 * k)
            }
        }
    }

    pub(crate) fn note_wide_effect_if_needed_with_extent(
        &mut self,
        shape: ShapeId,
        effect: &str,
        span: &Span,
        effect_extent: Option<f32>,
    ) {
        if self.shape_lipschitz_exact(shape) {
            return;
        }

        let Some(effect_extent) = effect_extent.filter(|v| v.is_finite() && *v > 0.0) else {
            self.diags.push(
                Diag::warning(
                    span.clone(),
                    format!(
                        "{effect} applied to non-lipschitz-exact shape#{shape}; cannot bound risk statically (non-constant reach/width)"
                    ),
                )
                .with_help(
                    "use literal effect sizes to enable bound checks, or keep exact primitives/boolean ops for strict distance behavior",
                ),
            );
            return;
        };

        let Some(bound) = self.shape_distance_error_bound(shape) else {
            self.diags.push(
                Diag::warning(
                    span.clone(),
                    format!(
                        "{effect} applied to non-lipschitz-exact shape#{shape}; distance error bound is unknown"
                    ),
                )
                .with_help(
                    "use literal smooth/effect sizes to enable quantitative checks, or accept approximate wide-effect falloff",
                ),
            );
            return;
        };

        // Risk estimator:
        //   ratio = B / r
        // where B is the shape distance-error bound and r is effect extent.
        // This approximates relative falloff-position error for wide effects.
        let ratio = if effect_extent > 0.0 {
            bound / effect_extent
        } else {
            f32::INFINITY
        };

        if ratio <= self.check_options.wide_effect_note_ratio {
            self.hir.notes.push(format!(
                "wide-effect: {effect} on shape#{shape} accepted (bound={bound:.4}, extent={effect_extent:.4}, ratio={ratio:.4})"
            ));
            return;
        }

        if ratio > self.check_options.wide_effect_warn_ratio {
            let min_extent = bound / self.check_options.wide_effect_warn_ratio;
            self.diags.push(
                Diag::warning(
                    span.clone(),
                    format!(
                        "{effect} on shape#{shape} may show approximation artifacts (bound={bound:.4}, extent={effect_extent:.4}, ratio={ratio:.3})"
                    ),
                )
                .with_help(format!(
                    "reduce smooth radius/complex ops or increase effect extent to >= {min_extent:.4} for ratio <= {:.2}",
                    self.check_options.wide_effect_warn_ratio
                )),
            );
        }
    }

    fn sx_floor(x: Sx) -> Sx {
        Sx::Sub(Box::new(x.clone()), Box::new(Sx::Fract(Box::new(x))))
    }

    fn sx_square(x: Sx) -> Sx {
        Sx::Mul(Box::new(x.clone()), Box::new(x))
    }

    fn sx_cube(x: Sx) -> Sx {
        Sx::Mul(Box::new(Self::sx_square(x.clone())), Box::new(x))
    }

    fn sx_quart(x: Sx) -> Sx {
        Self::sx_square(Self::sx_square(x))
    }

    fn sx_quint(x: Sx) -> Sx {
        Sx::Mul(Box::new(Self::sx_quart(x.clone())), Box::new(x))
    }

    fn sx_pow2(x: Sx) -> Sx {
        Sx::Pow(Box::new(Sx::Lit(2.0)), Box::new(x))
    }

    fn sx_step(edge: Sx, x: Sx) -> Sx {
        Sx::Step(Box::new(edge), Box::new(x))
    }

    fn sx_select(mask01: Sx, when_zero: Sx, when_one: Sx) -> Sx {
        let inv = Sx::Sub(Box::new(Sx::Lit(1.0)), Box::new(mask01.clone()));
        Sx::Add(
            Box::new(Sx::Mul(Box::new(when_zero), Box::new(inv))),
            Box::new(Sx::Mul(Box::new(when_one), Box::new(mask01))),
        )
    }

    pub(crate) fn sx_clamp01(x: Sx) -> Sx {
        let one = Sx::Lit(1.0);
        let max0 = Sx::Mul(
            Box::new(Sx::Lit(0.5)),
            Box::new(Sx::Add(Box::new(x.clone()), Box::new(Sx::Abs(Box::new(x))))),
        );
        let delta = Sx::Sub(Box::new(max0.clone()), Box::new(one.clone()));
        Sx::Mul(
            Box::new(Sx::Lit(0.5)),
            Box::new(Sx::Sub(
                Box::new(Sx::Add(Box::new(max0), Box::new(one))),
                Box::new(Sx::Abs(Box::new(delta))),
            )),
        )
    }

    fn sx_ease_in_transition(t: Sx, transition: &str) -> Sx {
        match transition {
            "sine" => {
                // easeInSine: 1 - cos((t * pi) / 2)
                let angle = Sx::Mul(Box::new(Sx::Lit(std::f32::consts::FRAC_PI_2)), Box::new(t));
                Sx::Sub(Box::new(Sx::Lit(1.0)), Box::new(Sx::Cos(Box::new(angle))))
            }
            "quad" => Self::sx_square(t),
            "cubic" => Self::sx_cube(t),
            "quart" => Self::sx_quart(t),
            "quint" => Self::sx_quint(t),
            "expo" => {
                let exp = Sx::Sub(
                    Box::new(Sx::Mul(Box::new(Sx::Lit(10.0)), Box::new(t))),
                    Box::new(Sx::Lit(10.0)),
                );
                Self::sx_pow2(exp)
            }
            "circ" => {
                let one_minus_t2 = Sx::Sub(Box::new(Sx::Lit(1.0)), Box::new(Self::sx_square(t)));
                let clamped = Sx::Max(Box::new(Sx::Lit(0.0)), Box::new(one_minus_t2));
                Sx::Sub(
                    Box::new(Sx::Lit(1.0)),
                    Box::new(Sx::Sqrt(Box::new(clamped))),
                )
            }
            "back" => {
                // easeInBack: c3*t^3 - c1*t^2
                let c1 = Sx::Lit(1.70158);
                let c3 = Sx::Lit(2.70158);
                let t2 = Self::sx_square(t.clone());
                let t3 = Self::sx_cube(t);
                Sx::Sub(
                    Box::new(Sx::Mul(Box::new(c3), Box::new(t3))),
                    Box::new(Sx::Mul(Box::new(c1), Box::new(t2))),
                )
            }
            "elastic" => {
                // easeInElastic approximation without endpoint special-casing.
                let pow_exp = Sx::Sub(
                    Box::new(Sx::Mul(Box::new(Sx::Lit(10.0)), Box::new(t.clone()))),
                    Box::new(Sx::Lit(10.0)),
                );
                let pow = Self::sx_pow2(pow_exp);
                let angle = Sx::Mul(
                    Box::new(Sx::Lit(std::f32::consts::TAU / 3.0)),
                    Box::new(Sx::Sub(
                        Box::new(Sx::Mul(Box::new(Sx::Lit(10.0)), Box::new(t))),
                        Box::new(Sx::Lit(10.75)),
                    )),
                );
                Sx::Neg(Box::new(Sx::Mul(
                    Box::new(pow),
                    Box::new(Sx::Sin(Box::new(angle))),
                )))
            }
            "linear" => t,
            _ => t,
        }
    }

    fn sx_ease_out_bounce(t: Sx) -> Sx {
        // Piecewise bounce approximation using step() masks.
        let n1 = Sx::Lit(7.5625);
        let d1 = Sx::Lit(2.75);
        let a = Sx::Div(Box::new(Sx::Lit(1.0)), Box::new(d1.clone()));
        let b = Sx::Div(Box::new(Sx::Lit(2.0)), Box::new(d1.clone()));
        let c = Sx::Div(Box::new(Sx::Lit(2.5)), Box::new(d1.clone()));

        let t_a = Sx::Sub(
            Box::new(t.clone()),
            Box::new(Sx::Div(Box::new(Sx::Lit(1.5)), Box::new(d1.clone()))),
        );
        let t_b = Sx::Sub(
            Box::new(t.clone()),
            Box::new(Sx::Div(Box::new(Sx::Lit(2.25)), Box::new(d1.clone()))),
        );
        let t_c = Sx::Sub(
            Box::new(t.clone()),
            Box::new(Sx::Div(Box::new(Sx::Lit(2.625)), Box::new(d1))),
        );

        let f1 = Sx::Mul(Box::new(n1.clone()), Box::new(Self::sx_square(t.clone())));
        let f2 = Sx::Add(
            Box::new(Sx::Mul(
                Box::new(n1.clone()),
                Box::new(Self::sx_square(t_a)),
            )),
            Box::new(Sx::Lit(0.75)),
        );
        let f3 = Sx::Add(
            Box::new(Sx::Mul(
                Box::new(n1.clone()),
                Box::new(Self::sx_square(t_b)),
            )),
            Box::new(Sx::Lit(0.9375)),
        );
        let f4 = Sx::Add(
            Box::new(Sx::Mul(Box::new(n1), Box::new(Self::sx_square(t_c)))),
            Box::new(Sx::Lit(0.984375)),
        );

        let s_a = Self::sx_step(a, t.clone());
        let s_b = Self::sx_step(b, t.clone());
        let s_c = Self::sx_step(c, t);

        let m1 = Sx::Sub(Box::new(Sx::Lit(1.0)), Box::new(s_a.clone()));
        let m2 = Sx::Mul(
            Box::new(s_a),
            Box::new(Sx::Sub(Box::new(Sx::Lit(1.0)), Box::new(s_b.clone()))),
        );
        let m3 = Sx::Mul(
            Box::new(s_b),
            Box::new(Sx::Sub(Box::new(Sx::Lit(1.0)), Box::new(s_c.clone()))),
        );
        let m4 = s_c;

        Sx::Add(
            Box::new(Sx::Add(
                Box::new(Sx::Mul(Box::new(m1), Box::new(f1))),
                Box::new(Sx::Mul(Box::new(m2), Box::new(f2))),
            )),
            Box::new(Sx::Add(
                Box::new(Sx::Mul(Box::new(m3), Box::new(f3))),
                Box::new(Sx::Mul(Box::new(m4), Box::new(f4))),
            )),
        )
    }

    fn sx_ease_out_transition(t: Sx, transition: &str) -> Sx {
        match transition {
            "bounce" => Self::sx_ease_out_bounce(t),
            _ => {
                let one_minus_t = Sx::Sub(Box::new(Sx::Lit(1.0)), Box::new(t));
                Sx::Sub(
                    Box::new(Sx::Lit(1.0)),
                    Box::new(Self::sx_ease_in_transition(one_minus_t, transition)),
                )
            }
        }
    }

    pub(crate) fn sx_ease(t: Sx, transition: &str, mode: &str) -> Sx {
        match mode {
            "in" => Self::sx_ease_in_transition(t, transition),
            "in_out" => {
                let two_t = Sx::Mul(Box::new(Sx::Lit(2.0)), Box::new(t.clone()));
                let left = Sx::Mul(
                    Box::new(Sx::Lit(0.5)),
                    Box::new(Self::sx_ease_in_transition(two_t.clone(), transition)),
                );
                let right_t = Sx::Sub(Box::new(two_t), Box::new(Sx::Lit(1.0)));
                let right = Sx::Add(
                    Box::new(Sx::Lit(0.5)),
                    Box::new(Sx::Mul(
                        Box::new(Sx::Lit(0.5)),
                        Box::new(Self::sx_ease_out_transition(right_t, transition)),
                    )),
                );
                let gate = Self::sx_step(Sx::Lit(0.5), t);
                Self::sx_select(gate, left, right)
            }
            "out_in" => {
                let two_t = Sx::Mul(Box::new(Sx::Lit(2.0)), Box::new(t.clone()));
                let left = Sx::Mul(
                    Box::new(Sx::Lit(0.5)),
                    Box::new(Self::sx_ease_out_transition(two_t.clone(), transition)),
                );
                let right_t = Sx::Sub(Box::new(two_t), Box::new(Sx::Lit(1.0)));
                let right = Sx::Add(
                    Box::new(Sx::Lit(0.5)),
                    Box::new(Sx::Mul(
                        Box::new(Sx::Lit(0.5)),
                        Box::new(Self::sx_ease_in_transition(right_t, transition)),
                    )),
                );
                let gate = Self::sx_step(Sx::Lit(0.5), t);
                Self::sx_select(gate, left, right)
            }
            "out" => Self::sx_ease_out_transition(t, transition),
            _ => Self::sx_ease_out_transition(t, transition),
        }
    }

    fn parse_signal_transition_name(name: &str) -> Option<&'static str> {
        match name {
            "linear" => Some("linear"),
            "sine" => Some("sine"),
            "quad" => Some("quad"),
            "cubic" => Some("cubic"),
            "quart" => Some("quart"),
            "quint" => Some("quint"),
            "expo" => Some("expo"),
            "circ" => Some("circ"),
            "back" => Some("back"),
            "elastic" => Some("elastic"),
            "bounce" => Some("bounce"),
            _ => None,
        }
    }

    fn parse_signal_mode_name(name: &str) -> Option<&'static str> {
        match name {
            "in" | "ease_in" => Some("in"),
            "out" | "ease_out" => Some("out"),
            "in_out" | "ease_in_out" => Some("in_out"),
            "out_in" | "ease_out_in" => Some("out_in"),
            _ => None,
        }
    }

    pub(crate) fn parse_signal_ease_name(name: &str) -> Option<(&'static str, &'static str)> {
        if name == "linear" {
            return Some(("linear", "out"));
        }

        if name == "in_out" || name == "ease_in_out" {
            return Some(("cubic", "in_out"));
        }

        let normalized = name.strip_prefix("ease_").unwrap_or(name);

        for (mode_prefix, mode) in [
            ("in_out_", "in_out"),
            ("out_in_", "out_in"),
            ("in_", "in"),
            ("out_", "out"),
        ] {
            if let Some(rest) = normalized.strip_prefix(mode_prefix) {
                let transition = Self::parse_signal_transition_name(rest)?;
                return Some((transition, mode));
            }
        }

        None
    }

    pub(crate) fn parse_signal_ease(
        &mut self,
        expr: &SExpr,
        builtin: &str,
    ) -> Option<(&'static str, &'static str)> {
        match &expr.node {
            Expr::Var(name) => match Self::parse_signal_ease_name(name) {
                Some(spec) => Some(spec),
                None => {
                    self.diags.push(
                        Diag::error(
                            expr.span.clone(),
                            format!("`{builtin}(ease: ...)` expects a known easing name"),
                        )
                        .with_help(
                            "use `ease: linear|out_quad|out_back|in_sine|in_out_cubic|out_in_expo`",
                        ),
                    );
                    None
                }
            },
            _ => {
                self.diags.push(
                    Diag::error(
                        expr.span.clone(),
                        format!("`{builtin}(ease: ...)` expects an easing identifier"),
                    )
                    .with_help(
                        "use `ease: linear|out_quad|out_back|in_sine|in_out_cubic|out_in_expo`",
                    ),
                );
                None
            }
        }
    }

    pub(crate) fn parse_signal_ease_transition(
        &mut self,
        expr: &SExpr,
        builtin: &str,
    ) -> Option<&'static str> {
        match &expr.node {
            Expr::Var(name) => match Self::parse_signal_transition_name(name) {
                Some(transition) => Some(transition),
                None => {
                    self.diags.push(
                        Diag::error(
                            expr.span.clone(),
                            format!(
                                "`{builtin}(transition: ...)` expects a known easing transition"
                            ),
                        )
                        .with_help(
                            "use `transition: linear|sine|quad|cubic|quart|quint|expo|circ|back|elastic|bounce`",
                        ),
                    );
                    None
                }
            },
            _ => {
                self.diags.push(
                    Diag::error(
                        expr.span.clone(),
                        format!("`{builtin}(transition: ...)` expects an identifier"),
                    )
                    .with_help(
                        "use `transition: linear|sine|quad|cubic|quart|quint|expo|circ|back|elastic|bounce`",
                    ),
                );
                None
            }
        }
    }

    pub(crate) fn parse_signal_ease_mode(
        &mut self,
        expr: &SExpr,
        builtin: &str,
    ) -> Option<&'static str> {
        match &expr.node {
            Expr::Var(name) => match Self::parse_signal_mode_name(name) {
                Some(mode) => Some(mode),
                None => {
                    self.diags.push(
                        Diag::error(
                            expr.span.clone(),
                            format!("`{builtin}(mode: ...)` expects a known easing mode"),
                        )
                        .with_help("use `mode: in|out|in_out|out_in` (or `ease_in|ease_out|ease_in_out|ease_out_in`)"),
                    );
                    None
                }
            },
            _ => {
                self.diags.push(
                    Diag::error(
                        expr.span.clone(),
                        format!("`{builtin}(mode: ...)` expects an identifier"),
                    )
                    .with_help("use `mode: in|out|in_out|out_in` (or `ease_in|ease_out|ease_in_out|ease_out_in`)"),
                );
                None
            }
        }
    }

    pub(crate) fn parse_signal_ease_spec(
        &mut self,
        builtin: &str,
        ease: Option<&SExpr>,
        transition: Option<&SExpr>,
        mode: Option<&SExpr>,
    ) -> Option<(&'static str, &'static str)> {
        if let Some(ease_expr) = ease {
            if let Some(conflict) = transition.or(mode) {
                self.diags.push(
                    Diag::error(
                        conflict.span.clone(),
                        format!(
                            "`{builtin}` accepts either `ease: ...` or `transition: ...`/`mode: ...`, not both"
                        ),
                    )
                    .with_help("use only `ease` for shorthand, or use `transition` and optional `mode`"),
                );
                return None;
            }
            return self.parse_signal_ease(ease_expr, builtin);
        }

        let transition = match transition {
            Some(expr) => self.parse_signal_ease_transition(expr, builtin)?,
            None => "linear",
        };
        let mode = match mode {
            Some(expr) => self.parse_signal_ease_mode(expr, builtin)?,
            None => "out",
        };

        Some((transition, mode))
    }

    pub(crate) fn signal_ease_label(transition: &str, mode: &str) -> String {
        if transition == "linear" {
            "linear".to_string()
        } else {
            format!("{mode}_{transition}")
        }
    }

    fn sx_lerp(a: Sx, b: Sx, t: Sx) -> Sx {
        let one_minus_t = Sx::Sub(Box::new(Sx::Lit(1.0)), Box::new(t.clone()));
        Sx::Add(
            Box::new(Sx::Mul(Box::new(a), Box::new(one_minus_t))),
            Box::new(Sx::Mul(Box::new(b), Box::new(t))),
        )
    }

    fn sx_hash2(ix: Sx, iy: Sx) -> Sx {
        let dot = Sx::Add(
            Box::new(Sx::Mul(Box::new(ix), Box::new(Sx::Lit(127.1)))),
            Box::new(Sx::Mul(Box::new(iy), Box::new(Sx::Lit(311.7)))),
        );
        let n = Sx::Mul(
            Box::new(Sx::Sin(Box::new(dot))),
            Box::new(Sx::Lit(43_758.547)),
        );
        Sx::Fract(Box::new(n))
    }

    fn sx_hash3(ix: Sx, iy: Sx, iz: Sx) -> Sx {
        let dot_xy = Sx::Add(
            Box::new(Sx::Mul(Box::new(ix), Box::new(Sx::Lit(127.1)))),
            Box::new(Sx::Mul(Box::new(iy), Box::new(Sx::Lit(311.7)))),
        );
        let dot = Sx::Add(
            Box::new(dot_xy),
            Box::new(Sx::Mul(Box::new(iz), Box::new(Sx::Lit(74.7)))),
        );
        let n = Sx::Mul(
            Box::new(Sx::Sin(Box::new(dot))),
            Box::new(Sx::Lit(43_758.547)),
        );
        Sx::Fract(Box::new(n))
    }

    fn sx_value_noise1(px: Sx) -> Sx {
        Self::sx_value_noise2(px, Sx::Lit(0.0))
    }

    fn sx_value_noise2(px: Sx, py: Sx) -> Sx {
        let ix = Self::sx_floor(px.clone());
        let iy = Self::sx_floor(py.clone());
        let fx = Sx::Fract(Box::new(px));
        let fy = Sx::Fract(Box::new(py));

        // Smooth interpolation (Hermite): u = f*f*(3 - 2*f)
        let two_fx = Sx::Mul(Box::new(Sx::Lit(2.0)), Box::new(fx.clone()));
        let two_fy = Sx::Mul(Box::new(Sx::Lit(2.0)), Box::new(fy.clone()));
        let ux = Sx::Mul(
            Box::new(Sx::Mul(Box::new(fx.clone()), Box::new(fx))),
            Box::new(Sx::Sub(Box::new(Sx::Lit(3.0)), Box::new(two_fx))),
        );
        let uy = Sx::Mul(
            Box::new(Sx::Mul(Box::new(fy.clone()), Box::new(fy))),
            Box::new(Sx::Sub(Box::new(Sx::Lit(3.0)), Box::new(two_fy))),
        );

        let ix1 = Sx::Add(Box::new(ix.clone()), Box::new(Sx::Lit(1.0)));
        let iy1 = Sx::Add(Box::new(iy.clone()), Box::new(Sx::Lit(1.0)));

        let n00 = Self::sx_hash2(ix.clone(), iy.clone());
        let n10 = Self::sx_hash2(ix1.clone(), iy);
        let n01 = Self::sx_hash2(ix, iy1.clone());
        let n11 = Self::sx_hash2(ix1, iy1);

        let nx0 = Self::sx_lerp(n00, n10, ux.clone());
        let nx1 = Self::sx_lerp(n01, n11, ux);
        Self::sx_lerp(nx0, nx1, uy)
    }

    fn sx_value_noise3(px: Sx, py: Sx, pz: Sx) -> Sx {
        let ix = Self::sx_floor(px.clone());
        let iy = Self::sx_floor(py.clone());
        let iz = Self::sx_floor(pz.clone());
        let fx = Sx::Fract(Box::new(px));
        let fy = Sx::Fract(Box::new(py));
        let fz = Sx::Fract(Box::new(pz));

        let two_fx = Sx::Mul(Box::new(Sx::Lit(2.0)), Box::new(fx.clone()));
        let two_fy = Sx::Mul(Box::new(Sx::Lit(2.0)), Box::new(fy.clone()));
        let two_fz = Sx::Mul(Box::new(Sx::Lit(2.0)), Box::new(fz.clone()));
        let ux = Sx::Mul(
            Box::new(Sx::Mul(Box::new(fx.clone()), Box::new(fx))),
            Box::new(Sx::Sub(Box::new(Sx::Lit(3.0)), Box::new(two_fx))),
        );
        let uy = Sx::Mul(
            Box::new(Sx::Mul(Box::new(fy.clone()), Box::new(fy))),
            Box::new(Sx::Sub(Box::new(Sx::Lit(3.0)), Box::new(two_fy))),
        );
        let uz = Sx::Mul(
            Box::new(Sx::Mul(Box::new(fz.clone()), Box::new(fz))),
            Box::new(Sx::Sub(Box::new(Sx::Lit(3.0)), Box::new(two_fz))),
        );

        let ix1 = Sx::Add(Box::new(ix.clone()), Box::new(Sx::Lit(1.0)));
        let iy1 = Sx::Add(Box::new(iy.clone()), Box::new(Sx::Lit(1.0)));
        let iz1 = Sx::Add(Box::new(iz.clone()), Box::new(Sx::Lit(1.0)));

        let n000 = Self::sx_hash3(ix.clone(), iy.clone(), iz.clone());
        let n100 = Self::sx_hash3(ix1.clone(), iy.clone(), iz.clone());
        let n010 = Self::sx_hash3(ix.clone(), iy1.clone(), iz.clone());
        let n110 = Self::sx_hash3(ix1.clone(), iy1.clone(), iz);
        let n001 = Self::sx_hash3(ix.clone(), iy.clone(), iz1.clone());
        let n101 = Self::sx_hash3(ix1.clone(), iy, iz1.clone());
        let n011 = Self::sx_hash3(ix, iy1.clone(), iz1.clone());
        let n111 = Self::sx_hash3(ix1, iy1, iz1);

        let nx00 = Self::sx_lerp(n000, n100, ux.clone());
        let nx10 = Self::sx_lerp(n010, n110, ux.clone());
        let nx01 = Self::sx_lerp(n001, n101, ux.clone());
        let nx11 = Self::sx_lerp(n011, n111, ux);
        let nxy0 = Self::sx_lerp(nx00, nx10, uy.clone());
        let nxy1 = Self::sx_lerp(nx01, nx11, uy);
        Self::sx_lerp(nxy0, nxy1, uz)
    }

    fn static_fract(x: f32) -> f32 {
        x - x.floor()
    }

    fn static_hash2(ix: f32, iy: f32) -> f32 {
        let dot = ix * 127.1 + iy * 311.7;
        Self::static_fract(dot.sin() * 43_758.547)
    }

    fn static_hash3(ix: f32, iy: f32, iz: f32) -> f32 {
        let dot = ix * 127.1 + iy * 311.7 + iz * 74.7;
        Self::static_fract(dot.sin() * 43_758.547)
    }

    fn static_value_noise1(px: f32) -> f32 {
        Self::static_value_noise2(px, 0.0)
    }

    fn static_value_noise2(px: f32, py: f32) -> f32 {
        let ix = px.floor();
        let iy = py.floor();
        let fx = Self::static_fract(px);
        let fy = Self::static_fract(py);

        let ux = fx * fx * (3.0 - 2.0 * fx);
        let uy = fy * fy * (3.0 - 2.0 * fy);

        let n00 = Self::static_hash2(ix, iy);
        let n10 = Self::static_hash2(ix + 1.0, iy);
        let n01 = Self::static_hash2(ix, iy + 1.0);
        let n11 = Self::static_hash2(ix + 1.0, iy + 1.0);

        let nx0 = n00 * (1.0 - ux) + n10 * ux;
        let nx1 = n01 * (1.0 - ux) + n11 * ux;
        nx0 * (1.0 - uy) + nx1 * uy
    }

    fn static_value_noise3(px: f32, py: f32, pz: f32) -> f32 {
        let ix = px.floor();
        let iy = py.floor();
        let iz = pz.floor();
        let fx = Self::static_fract(px);
        let fy = Self::static_fract(py);
        let fz = Self::static_fract(pz);

        let ux = fx * fx * (3.0 - 2.0 * fx);
        let uy = fy * fy * (3.0 - 2.0 * fy);
        let uz = fz * fz * (3.0 - 2.0 * fz);

        let n000 = Self::static_hash3(ix, iy, iz);
        let n100 = Self::static_hash3(ix + 1.0, iy, iz);
        let n010 = Self::static_hash3(ix, iy + 1.0, iz);
        let n110 = Self::static_hash3(ix + 1.0, iy + 1.0, iz);
        let n001 = Self::static_hash3(ix, iy, iz + 1.0);
        let n101 = Self::static_hash3(ix + 1.0, iy, iz + 1.0);
        let n011 = Self::static_hash3(ix, iy + 1.0, iz + 1.0);
        let n111 = Self::static_hash3(ix + 1.0, iy + 1.0, iz + 1.0);

        let nx00 = n000 * (1.0 - ux) + n100 * ux;
        let nx10 = n010 * (1.0 - ux) + n110 * ux;
        let nx01 = n001 * (1.0 - ux) + n101 * ux;
        let nx11 = n011 * (1.0 - ux) + n111 * ux;
        let nxy0 = nx00 * (1.0 - uy) + nx10 * uy;
        let nxy1 = nx01 * (1.0 - uy) + nx11 * uy;
        nxy0 * (1.0 - uz) + nxy1 * uz
    }

    fn static_intrinsic_noise1(px: &Sx) -> Option<f32> {
        let px = Self::try_eval_static_scalar(px)?;
        Some(Self::static_value_noise1(px))
    }

    fn static_intrinsic_noise2(px: &Sx, py: &Sx) -> Option<f32> {
        let px = Self::try_eval_static_scalar(px)?;
        let py = Self::try_eval_static_scalar(py)?;
        Some(Self::static_value_noise2(px, py))
    }

    fn static_intrinsic_noise3(px: &Sx, py: &Sx, pz: &Sx) -> Option<f32> {
        let px = Self::try_eval_static_scalar(px)?;
        let py = Self::try_eval_static_scalar(py)?;
        let pz = Self::try_eval_static_scalar(pz)?;
        Some(Self::static_value_noise3(px, py, pz))
    }

    fn noise1_helper_id() -> String {
        "__builtin_noise1".to_string()
    }

    fn ensure_noise1_helper(&mut self) -> String {
        let helper_id = Self::noise1_helper_id();
        if self.hir.user_helpers.contains_key(&helper_id) {
            return helper_id;
        }

        let helper = hir::UserFnHelper {
            ret_kind: crate::typed_scalar::Kind::F32,
            id: helper_id.clone(),
            params: vec![hir::UserFnParam {
                scalar_kind: crate::typed_scalar::Kind::F32,
                name: "px".to_string(),
                ty: hir::UserFnParamTy::Scalar,
                scalar_slots: vec!["px".to_string()],
            }],
            param_scalars: vec!["px".to_string()],
            ret_components: 1,
            sample_point_invariant: false,
            needs_entry_inputs: false,
            body_stmts: vec![
                hir::UserFnStmt::Let {
                    name: "n".to_string(),
                    slots: vec!["n".to_string()],
                    init: hir::UserFnExpr::Value(hir::UserFnValue::Scalar(Self::sx_value_noise1(
                        Sx::Param("px".to_string()),
                    ))),
                },
                hir::UserFnStmt::Return {
                    value: hir::UserFnExpr::Value(hir::UserFnValue::Scalar(Sx::Param(
                        "n".to_string(),
                    ))),
                },
            ],
        };

        self.hir.user_helpers.insert(helper_id.clone(), helper);
        helper_id
    }

    fn noise2_helper_id() -> String {
        "__builtin_noise2".to_string()
    }

    fn ensure_noise2_helper(&mut self) -> String {
        let helper_id = Self::noise2_helper_id();
        if self.hir.user_helpers.contains_key(&helper_id) {
            return helper_id;
        }

        let helper = hir::UserFnHelper {
            ret_kind: crate::typed_scalar::Kind::F32,
            id: helper_id.clone(),
            params: vec![
                hir::UserFnParam {
                    scalar_kind: crate::typed_scalar::Kind::F32,
                    name: "px".to_string(),
                    ty: hir::UserFnParamTy::Scalar,
                    scalar_slots: vec!["px".to_string()],
                },
                hir::UserFnParam {
                    scalar_kind: crate::typed_scalar::Kind::F32,
                    name: "py".to_string(),
                    ty: hir::UserFnParamTy::Scalar,
                    scalar_slots: vec!["py".to_string()],
                },
            ],
            param_scalars: vec!["px".to_string(), "py".to_string()],
            ret_components: 1,
            sample_point_invariant: false,
            needs_entry_inputs: false,
            body_stmts: vec![
                hir::UserFnStmt::Let {
                    name: "n".to_string(),
                    slots: vec!["n".to_string()],
                    init: hir::UserFnExpr::Value(hir::UserFnValue::Scalar(Self::sx_value_noise2(
                        Sx::Param("px".to_string()),
                        Sx::Param("py".to_string()),
                    ))),
                },
                hir::UserFnStmt::Return {
                    value: hir::UserFnExpr::Value(hir::UserFnValue::Scalar(Sx::Param(
                        "n".to_string(),
                    ))),
                },
            ],
        };

        self.hir.user_helpers.insert(helper_id.clone(), helper);
        helper_id
    }

    fn noise3_helper_id() -> String {
        "__builtin_noise3".to_string()
    }

    fn ensure_noise3_helper(&mut self) -> String {
        let helper_id = Self::noise3_helper_id();
        if self.hir.user_helpers.contains_key(&helper_id) {
            return helper_id;
        }

        let helper = hir::UserFnHelper {
            ret_kind: crate::typed_scalar::Kind::F32,
            id: helper_id.clone(),
            params: vec![
                hir::UserFnParam {
                    scalar_kind: crate::typed_scalar::Kind::F32,
                    name: "px".to_string(),
                    ty: hir::UserFnParamTy::Scalar,
                    scalar_slots: vec!["px".to_string()],
                },
                hir::UserFnParam {
                    scalar_kind: crate::typed_scalar::Kind::F32,
                    name: "py".to_string(),
                    ty: hir::UserFnParamTy::Scalar,
                    scalar_slots: vec!["py".to_string()],
                },
                hir::UserFnParam {
                    scalar_kind: crate::typed_scalar::Kind::F32,
                    name: "pz".to_string(),
                    ty: hir::UserFnParamTy::Scalar,
                    scalar_slots: vec!["pz".to_string()],
                },
            ],
            param_scalars: vec!["px".to_string(), "py".to_string(), "pz".to_string()],
            ret_components: 1,
            sample_point_invariant: false,
            needs_entry_inputs: false,
            body_stmts: vec![
                hir::UserFnStmt::Let {
                    name: "n".to_string(),
                    slots: vec!["n".to_string()],
                    init: hir::UserFnExpr::Value(hir::UserFnValue::Scalar(Self::sx_value_noise3(
                        Sx::Param("px".to_string()),
                        Sx::Param("py".to_string()),
                        Sx::Param("pz".to_string()),
                    ))),
                },
                hir::UserFnStmt::Return {
                    value: hir::UserFnExpr::Value(hir::UserFnValue::Scalar(Sx::Param(
                        "n".to_string(),
                    ))),
                },
            ],
        };

        self.hir.user_helpers.insert(helper_id.clone(), helper);
        helper_id
    }

    fn static_intrinsic_fbm(
        px: &Sx,
        py: &Sx,
        octaves: usize,
        lacunarity: &Sx,
        gain: &Sx,
    ) -> Option<f32> {
        let mut fx = Self::try_eval_static_scalar(px)?;
        let mut fy = Self::try_eval_static_scalar(py)?;
        let lacunarity = Self::try_eval_static_scalar(lacunarity)?;
        let gain = Self::try_eval_static_scalar(gain)?;

        let mut amp = 0.5_f32;
        let mut acc = 0.0_f32;
        for _ in 0..octaves {
            acc += amp * Self::static_value_noise2(fx, fy);
            fx *= lacunarity;
            fy *= lacunarity;
            amp *= gain;
        }
        Some(acc)
    }

    fn fbm_helper_id(octaves: usize) -> String {
        format!("__builtin_fbm_{octaves}")
    }

    fn ensure_fbm_helper(&mut self, octaves: usize) -> String {
        let helper_id = Self::fbm_helper_id(octaves);
        if self.hir.user_helpers.contains_key(&helper_id) {
            return helper_id;
        }

        let helper = hir::UserFnHelper {
            ret_kind: crate::typed_scalar::Kind::F32,
            id: helper_id.clone(),
            params: vec![
                hir::UserFnParam {
                    scalar_kind: crate::typed_scalar::Kind::F32,
                    name: "px".to_string(),
                    ty: hir::UserFnParamTy::Scalar,
                    scalar_slots: vec!["px".to_string()],
                },
                hir::UserFnParam {
                    scalar_kind: crate::typed_scalar::Kind::F32,
                    name: "py".to_string(),
                    ty: hir::UserFnParamTy::Scalar,
                    scalar_slots: vec!["py".to_string()],
                },
                hir::UserFnParam {
                    scalar_kind: crate::typed_scalar::Kind::F32,
                    name: "lacunarity".to_string(),
                    ty: hir::UserFnParamTy::Scalar,
                    scalar_slots: vec!["lacunarity".to_string()],
                },
                hir::UserFnParam {
                    scalar_kind: crate::typed_scalar::Kind::F32,
                    name: "gain".to_string(),
                    ty: hir::UserFnParamTy::Scalar,
                    scalar_slots: vec!["gain".to_string()],
                },
            ],
            param_scalars: vec![
                "px".to_string(),
                "py".to_string(),
                "lacunarity".to_string(),
                "gain".to_string(),
            ],
            ret_components: 1,
            sample_point_invariant: false,
            needs_entry_inputs: false,
            body_stmts: {
                let mut body = vec![
                    hir::UserFnStmt::Let {
                        name: "fx".to_string(),
                        slots: vec!["fx".to_string()],
                        init: hir::UserFnExpr::Value(hir::UserFnValue::Scalar(Sx::Param(
                            "px".to_string(),
                        ))),
                    },
                    hir::UserFnStmt::Let {
                        name: "fy".to_string(),
                        slots: vec!["fy".to_string()],
                        init: hir::UserFnExpr::Value(hir::UserFnValue::Scalar(Sx::Param(
                            "py".to_string(),
                        ))),
                    },
                    hir::UserFnStmt::Let {
                        name: "amp".to_string(),
                        slots: vec!["amp".to_string()],
                        init: hir::UserFnExpr::Value(hir::UserFnValue::Scalar(Sx::Lit(0.5))),
                    },
                    hir::UserFnStmt::Let {
                        name: "acc".to_string(),
                        slots: vec!["acc".to_string()],
                        init: hir::UserFnExpr::Value(hir::UserFnValue::Scalar(Sx::Lit(0.0))),
                    },
                ];

                let octave_values = vec![Sx::Lit(0.0); octaves];
                let oct_body = vec![
                    hir::UserFnStmt::Let {
                        name: "n".to_string(),
                        slots: vec!["n".to_string()],
                        init: hir::UserFnExpr::Value(hir::UserFnValue::Scalar(
                            Self::sx_value_noise2(
                                Sx::Param("fx".to_string()),
                                Sx::Param("fy".to_string()),
                            ),
                        )),
                    },
                    hir::UserFnStmt::Assign {
                        name: "acc".to_string(),
                        field_path: None,
                        value: hir::UserFnExpr::Value(hir::UserFnValue::Scalar(Sx::Add(
                            Box::new(Sx::Param("acc".to_string())),
                            Box::new(Sx::Mul(
                                Box::new(Sx::Param("amp".to_string())),
                                Box::new(Sx::Param("n".to_string())),
                            )),
                        ))),
                    },
                    hir::UserFnStmt::Assign {
                        name: "fx".to_string(),
                        field_path: None,
                        value: hir::UserFnExpr::Value(hir::UserFnValue::Scalar(Sx::Mul(
                            Box::new(Sx::Param("fx".to_string())),
                            Box::new(Sx::Param("lacunarity".to_string())),
                        ))),
                    },
                    hir::UserFnStmt::Assign {
                        name: "fy".to_string(),
                        field_path: None,
                        value: hir::UserFnExpr::Value(hir::UserFnValue::Scalar(Sx::Mul(
                            Box::new(Sx::Param("fy".to_string())),
                            Box::new(Sx::Param("lacunarity".to_string())),
                        ))),
                    },
                    hir::UserFnStmt::Assign {
                        name: "amp".to_string(),
                        field_path: None,
                        value: hir::UserFnExpr::Value(hir::UserFnValue::Scalar(Sx::Mul(
                            Box::new(Sx::Param("amp".to_string())),
                            Box::new(Sx::Param("gain".to_string())),
                        ))),
                    },
                ];

                body.push(hir::UserFnStmt::For {
                    name: "octave".to_string(),
                    slots: vec!["octave".to_string()],
                    values: octave_values,
                    body: oct_body,
                    index_name: None,
                });

                body.push(hir::UserFnStmt::Return {
                    value: hir::UserFnExpr::Value(hir::UserFnValue::Scalar(Sx::Param(
                        "acc".to_string(),
                    ))),
                });
                body
            },
        };

        self.hir.user_helpers.insert(helper_id.clone(), helper);
        helper_id
    }

    pub(crate) fn lower_intrinsic_noise1_components(&mut self, px: Sx) -> Value {
        self.hir.notes.push(
            "field: noise1 lowered as deterministic smoothed value noise in v0 (reusable helper path)"
                .to_string(),
        );
        if let Some(v) = Self::static_intrinsic_noise1(&px) {
            Value::Mask(Sx::Lit(v))
        } else {
            let helper_id = self.ensure_noise1_helper();
            Value::Mask(Sx::UserCall {
                call: std::rc::Rc::new(hir::UserFnCall {
                    helper_id,
                    args: vec![px],
                    ret_components: 1,
                }),
                component: None,
            })
        }
    }

    pub(crate) fn lower_intrinsic_noise2_components(&mut self, px: Sx, py: Sx) -> Value {
        self.hir.notes.push(
            "field: noise2 lowered as deterministic smoothed value noise in v0 (reusable helper path)"
                .to_string(),
        );
        if let Some(v) = Self::static_intrinsic_noise2(&px, &py) {
            Value::Mask(Sx::Lit(v))
        } else {
            let helper_id = self.ensure_noise2_helper();
            Value::Mask(Sx::UserCall {
                call: std::rc::Rc::new(hir::UserFnCall {
                    helper_id,
                    args: vec![px, py],
                    ret_components: 1,
                }),
                component: None,
            })
        }
    }

    pub(crate) fn lower_intrinsic_noise3_components(&mut self, px: Sx, py: Sx, pz: Sx) -> Value {
        self.hir.notes.push(
            "field: noise3 lowered as deterministic smoothed value noise in v0 (reusable helper path)"
                .to_string(),
        );
        if let Some(v) = Self::static_intrinsic_noise3(&px, &py, &pz) {
            Value::Mask(Sx::Lit(v))
        } else {
            let helper_id = self.ensure_noise3_helper();
            Value::Mask(Sx::UserCall {
                call: std::rc::Rc::new(hir::UserFnCall {
                    helper_id,
                    args: vec![px, py, pz],
                    ret_components: 1,
                }),
                component: None,
            })
        }
    }

    pub(crate) fn lower_intrinsic_fbm_components(
        &mut self,
        px0: Sx,
        py0: Sx,
        octaves: usize,
        lacunarity: Sx,
        gain: Sx,
    ) -> Value {
        if let Some(v) = Self::static_intrinsic_fbm(&px0, &py0, octaves, &lacunarity, &gain) {
            self.hir.notes.push(format!(
                "field: fbm lowered with deterministic value-noise base, octaves={octaves}, lacunarity and gain in closed-form scalar ops (reusable helper path)",
            ));
            Value::Mask(Sx::Lit(v))
        } else {
            self.hir.notes.push(format!(
                "field: fbm lowered with deterministic value-noise base, octaves={octaves}, lacunarity and gain in closed-form scalar ops (reusable helper path)",
            ));
            let helper_id = self.ensure_fbm_helper(octaves);
            Value::Mask(Sx::UserCall {
                call: std::rc::Rc::new(hir::UserFnCall {
                    helper_id,
                    args: vec![px0, py0, lacunarity, gain],
                    ret_components: 1,
                }),
                component: None,
            })
        }
    }

    pub(super) fn try_eval_static_scalar(sx: &Sx) -> Option<f32> {
        crate::signal_eval::evaluate(sx, &HashMap::new()).ok()
    }

    pub(crate) fn require_number_source(
        &mut self,
        expr: &SExpr,
        what: &str,
    ) -> Option<NumberSource> {
        match &expr.node {
            Expr::Range(lo_expr, hi_expr) => {
                let lo = self.as_scalar(lo_expr)?;
                let hi = self.as_scalar(hi_expr)?;
                Some(NumberSource::Range {
                    lo,
                    hi,
                    span: expr.span.clone(),
                })
            }
            Expr::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.as_scalar(item)?);
                }
                Some(NumberSource::List {
                    items: out,
                    span: expr.span.clone(),
                })
            }
            Expr::Vec2(a, b) => {
                let x = self.as_scalar(a)?;
                let y = self.as_scalar(b)?;
                Some(NumberSource::List {
                    items: vec![x, y],
                    span: expr.span.clone(),
                })
            }
            _ => {
                self.diags.push(
                    Diag::error(
                        expr.span.clone(),
                        format!("{what} expects a numeric source (`a .. b` or `[a, b, ...]`)"),
                    )
                    .with_help("use `a .. b`, `[a, b]`, or `(a, b)`"),
                );
                None
            }
        }
    }

    pub(crate) fn require_range_pair(&mut self, expr: &SExpr, what: &str) -> Option<(Sx, Sx)> {
        let source = self.require_number_source(expr, what)?;
        match source {
            NumberSource::Range { lo, hi, span } => {
                let lo_const = Self::try_eval_static_scalar(&lo);
                let hi_const = Self::try_eval_static_scalar(&hi);

                if let (Some(lo_num), Some(hi_num)) = (lo_const, hi_const) {
                    if lo_num > hi_num {
                        self.diags.push(
                            Diag::warning(
                                span,
                                format!("{what} range is inverted; normalizing endpoints"),
                            )
                            .with_label(format!("got {} .. {}", lo_num, hi_num))
                            .with_help("v0 normalizes literal ranges to `min .. max`; write `min .. max` directly to silence this warning"),
                        );
                        return Some((hi, lo));
                    }
                } else {
                    self.hir.notes.push(format!(
                        "signal: {what} range kept as-authored (dynamic endpoints are preserved at runtime)"
                    ));
                }
                Some((lo, hi))
            }
            NumberSource::List { items, span } => {
                if items.len() != 2 {
                    self.diags.push(
                        Diag::error(
                            span,
                            format!(
                                "{what} explicit numeric lists must contain exactly two values"
                            ),
                        )
                        .with_help("use `[lo, hi]` or `(lo, hi)`"),
                    );
                    return None;
                }
                Some((items[0].clone(), items[1].clone()))
            }
        }
    }

    pub(super) fn builtin(
        &mut self,
        name: &str,
        name_span: &Span,
        recv: Option<Value>,
        args: &[Arg],
        call_span: &Span,
    ) -> Option<Value> {
        let _span = trace_span!(
            "check.builtin",
            name = %name,
            has_recv = recv.is_some(),
            argc = args.len()
        )
        .entered();

        let mut bag = ArgBag::new(name, args, call_span.clone());
        if matches!(name, "inset_distance" | "boundary_point" | "contour")
            && let Some(Value::RepeatCell(cell)) = &recv
        {
            let result = self.cell_geometry_method(cell, name, &mut bag, call_span);
            bag.finish(&mut self.diags);
            return result;
        }

        if (recv.is_none() && matches!(name, "band" | "chase"))
            || (name == "point" && matches!(&recv, Some(Value::CellContour { .. })))
        {
            let result = self.contour_builtin(name, recv, &mut bag, call_span);
            bag.finish(&mut self.diags);
            return result;
        }
        // --- Check user-defined effects first --------------------------------
        // If `name` matches a declared effect, emit a `Layer::UserEffect` node.
        if let Some(effect_reg) = self.effect_defs.get(name).cloned() {
            let result = self.dispatch_user_effect(&effect_reg, recv, args, name_span, call_span);
            return result;
        }

        if recv.is_none() && name == "reduce" {
            let result = self.builtin_reduce(&mut bag);
            bag.finish(&mut self.diags);
            return result;
        }

        // Preserve the migration diagnostic for legacy `fill(path, ...)`
        // call-sites even when extra args are present (for example `width:`).
        if recv.is_none()
            && name == "fill"
            && let Some(first_arg) = args.first()
        {
            let first = self.eval(&first_arg.value)?;
            if matches!(first, Value::PathFuture(_)) {
                self.diags.push(
                    Diag::error(
                        first_arg.value.span.clone(),
                        "`fill(path, ...)` is no longer supported",
                    )
                    .with_help(
                        "use `some_path |> fill(color)` for closed fills or `some_path |> stroke(width: ..., color: ...)` for path strokes",
                    ),
                );
                return Some(Value::Error);
            }
        }

        // Try registry-based implementation first.
        // Multiple declarations may share a name (e.g. shape and layer receivers),
        // so select the first declaration compatible with the current receiver kind.
        let compatible_decl = crate::registry::builtin_decl_by_name(name)
            .iter()
            .copied()
            .find(|decl| match (&decl.lowering, &recv) {
                (crate::registry::BuiltinLowering::Impl(_), None)
                | (crate::registry::BuiltinLowering::ImplDiscriminated { .. }, None) => {
                    Self::builtin_decl_accepts_arg_count(
                        decl,
                        Self::provided_arg_count_for_decl_match(decl, args),
                    )
                }
                (crate::registry::BuiltinLowering::ImplReceiver(rf), Some(Value::Shape(_))) => {
                    rf.supports_shape()
                        && Self::builtin_decl_accepts_arg_count(
                            decl,
                            Self::provided_arg_count_for_decl_match(decl, args),
                        )
                }
                (crate::registry::BuiltinLowering::ImplReceiver(rf), Some(Value::Layer(_))) => {
                    rf.supports_layer()
                        && Self::builtin_decl_accepts_arg_count(
                            decl,
                            Self::provided_arg_count_for_decl_match(decl, args),
                        )
                }
                (
                    crate::registry::BuiltinLowering::ImplReceiver(rf),
                    Some(Value::PathFuture(_)),
                ) => {
                    rf.supports_path()
                        && Self::builtin_decl_accepts_arg_count(
                            decl,
                            Self::provided_arg_count_for_decl_match(decl, args),
                        )
                }
                _ => false,
            });

        let pipe_fallback_decl = if recv.is_some() {
            crate::registry::builtin_decl_by_name(name)
                .iter()
                .copied()
                .find(|decl| {
                    decl.signature.receiver.is_none()
                        && decl
                            .signature
                            .caps
                            .contains(crate::builtin_catalog::BuiltinCaps::PIPEABLE)
                        && decl.signature.args.first().is_some_and(|arg| arg.required)
                        && !matches!(
                            decl.lowering,
                            crate::registry::BuiltinLowering::ImplReceiver(_)
                        )
                        && Self::builtin_decl_accepts_arg_count(decl, args.len() + 1)
                })
        } else {
            None
        };

        // Shape constructors support an optional `rotate:` post-transform.
        let rotate = if matches!(
            compatible_decl,
            Some(decl)
                if decl.signature.receiver.is_none()
                    && decl.signature.result
                        == crate::registry::TypeRef::Primitive(crate::registry::PrimitiveType::Shape)
        ) {
            if let Some(expr) = bag.take_exact_named("rotate") {
                let rotate_value = self.eval(expr)?;
                let Some((angle, _kind)) = Checker::as_numeric_scalar(&rotate_value) else {
                    if !matches!(rotate_value, Value::Error) {
                        self.diags.push(
                            Diag::error(
                                expr.span.clone(),
                                format!(
                                    "argument `rotate` to `{name}` expected scalar, found {}",
                                    rotate_value.kind()
                                ),
                            )
                            .with_label("type mismatch in shape rotation argument"),
                        );
                    }
                    return Some(Value::Error);
                };
                Some(angle)
            } else {
                None
            }
        } else {
            None
        };

        if let Some(decl) = compatible_decl {
            match (&decl.lowering, &recv) {
                (crate::registry::BuiltinLowering::Impl(impl_fn), None) => {
                    if let Some(result) = impl_fn(self, &mut bag) {
                        let result = self.wrap_shape_with_rotation(result, rotate);
                        bag.finish(&mut self.diags);
                        return Some(result);
                    }
                    return None;
                }
                (
                    crate::registry::BuiltinLowering::ImplReceiver(rf),
                    Some(Value::Shape(shape_id)),
                ) => {
                    if let Some(result) = rf.call_with_shape(self, *shape_id, &mut bag) {
                        let result = self.wrap_shape_with_rotation(result, rotate);
                        bag.finish(&mut self.diags);
                        return Some(result);
                    }
                    return None;
                }
                (
                    crate::registry::BuiltinLowering::ImplReceiver(rf),
                    Some(Value::Layer(layer_id)),
                ) => {
                    if let Some(result) = rf.call_with_layer(self, *layer_id, &mut bag) {
                        let result = self.wrap_shape_with_rotation(result, rotate);
                        bag.finish(&mut self.diags);
                        return Some(result);
                    }
                    return None;
                }
                (
                    crate::registry::BuiltinLowering::ImplReceiver(rf),
                    Some(Value::PathFuture(path)),
                ) => {
                    bag.set_path_receiver(path.clone());
                    if let Some(result) = rf.call_with_path(self, &mut bag) {
                        let result = self.wrap_shape_with_rotation(result, rotate);
                        bag.finish(&mut self.diags);
                        return Some(result);
                    }
                    return None;
                }
                (
                    crate::registry::BuiltinLowering::ImplDiscriminated {
                        discriminator,
                        default,
                        impl_fn,
                    },
                    None,
                ) => {
                    let disc_arg = bag.take(discriminator);
                    let variant_name = if let Some(expr) = disc_arg {
                        match &expr.node {
                            Expr::Var(name) => name.as_str(),
                            _ => {
                                self.diags.push(Diag::error(
                                    expr.span.clone(),
                                    format!(
                                        "discriminator `{}` must be an identifier",
                                        discriminator
                                    ),
                                ));
                                return None;
                            }
                        }
                    } else if let Some(default_variant) = default {
                        default_variant
                    } else {
                        self.diags.push(Diag::error(
                            call_span.clone(),
                            format!("missing required discriminator field `{}`", discriminator),
                        ));
                        return None;
                    };

                    if let Some(result) = impl_fn(self, variant_name, &mut bag) {
                        let result = self.wrap_shape_with_rotation(result, rotate);
                        bag.finish(&mut self.diags);
                        return Some(result);
                    }
                    return None;
                }
                _ => {}
            }
        }

        if let Some(recv) = recv.clone()
            && pipe_fallback_decl.is_some()
        {
            return self.with_piped_arg(recv, args, call_span, |ctx, piped_args| {
                ctx.builtin(name, name_span, None, piped_args, call_span)
            });
        }

        let result = match (name, &recv) {
            ("point_at" | "tangent_at", Some(Value::PathFuture(path))) => {
                let s_expr = bag.require("s", &mut self.diags)?;
                let s_val = self.eval(s_expr)?;
                let Some((s_sx, _kind)) = Checker::as_numeric_scalar(&s_val) else {
                    if !matches!(s_val, Value::Error) {
                        self.diags.push(
                            Diag::error(
                                s_expr.span.clone(),
                                format!(
                                    "argument `s` to `{name}` expected scalar, found {}",
                                    s_val.kind()
                                ),
                            )
                            .with_label("type mismatch in path evaluator call"),
                        );
                    }
                    return Some(Value::Error);
                };
                if name == "point_at" {
                    self.record_path_eval_demand(path.profile_id(), "point_at");
                    Some(Value::Vec2((
                        Sx::PathPointAtComponent {
                            path_id: path.profile_id(),
                            s: Box::new(s_sx.clone()),
                            component: 0,
                        },
                        Sx::PathPointAtComponent {
                            path_id: path.profile_id(),
                            s: Box::new(s_sx),
                            component: 1,
                        },
                    )))
                } else {
                    self.record_path_eval_demand(path.profile_id(), "tangent_at");
                    Some(Value::Vec2((
                        Sx::PathTangentAtComponent {
                            path_id: path.profile_id(),
                            s: Box::new(s_sx.clone()),
                            component: 0,
                        },
                        Sx::PathTangentAtComponent {
                            path_id: path.profile_id(),
                            s: Box::new(s_sx),
                            component: 1,
                        },
                    )))
                }
            }
            ("dist" | "along" | "tangent", Some(Value::PathFuture(path))) => match name {
                "dist" => {
                    self.record_path_channel_demand(path.profile_id(), "dist");
                    Some(Value::Scalar(Sx::PathDist {
                        path_id: path.profile_id(),
                    }))
                }
                "along" => {
                    self.record_path_channel_demand(path.profile_id(), "along");
                    Some(Value::Scalar(Sx::PathAlong {
                        path_id: path.profile_id(),
                    }))
                }
                "tangent" => {
                    self.record_path_channel_demand(path.profile_id(), "tangent");
                    Some(Value::Vec2((
                        Sx::PathTangentComponent {
                            path_id: path.profile_id(),
                            component: 0,
                        },
                        Sx::PathTangentComponent {
                            path_id: path.profile_id(),
                            component: 1,
                        },
                    )))
                }
                _ => None,
            },
            (_, Some(r)) => {
                self.diags.push(
                    Diag::error(name_span.clone(), format!("no builtin `{name}` on a {}", r.kind()))
                        .with_help("effects on shapes: round, smooth, dilate, erode, fill, stroke, shadow, glow, inner_glow, soften, bevel. On layers: blur, glow, opacity, mask, tint, soften, inner_glow, bevel"),
                );
                None
            }
            (_, None) => {
                self.diags.push(
                    Diag::error(name_span.clone(), format!("unknown builtin `{name}`"))
                        .with_help("constructors: circle(at, radius), box(at, size), fill(color), image(texture_name); collections: reduce(items, seed, op?)"),
                );
                None
            }
        };
        bag.finish(&mut self.diags);
        result
    }

    /// Dispatch a call to a user-defined effect, emitting a `Layer::UserEffect` node.
    fn dispatch_user_effect(
        &mut self,
        effect_reg: &EffectRegistered,
        recv: Option<Value>,
        args: &[Arg],
        name_span: &Span,
        call_span: &Span,
    ) -> Option<Value> {
        // Evaluate the inner (piped-in) layer.
        let inner_layer = match recv {
            Some(Value::Layer(id)) => Some(id),
            Some(Value::Error) => return Some(Value::Error),
            Some(other) => {
                self.diags.push(
                    Diag::error(
                        name_span.clone(),
                        format!(
                            "effect `{}` expects a layer receiver, found {}",
                            effect_reg.name,
                            other.kind()
                        ),
                    )
                    .with_help("pipe a layer into the effect with `layer |> effect_name(...)`"),
                );
                return Some(Value::Error);
            }
            None => None,
        };

        // Evaluate positional scalar arguments.
        if args.len() != effect_reg.param_count {
            self.diags.push(
                Diag::error(
                    call_span.clone(),
                    format!(
                        "effect `{}` expects {} argument(s), found {}",
                        effect_reg.name,
                        effect_reg.param_count,
                        args.len()
                    ),
                )
                .with_help("pass the correct number of arguments to the effect"),
            );
            return Some(Value::Error);
        }

        let mut evaluated_args: Vec<Sx> = Vec::with_capacity(args.len());
        for arg in args {
            let val = self.eval(&arg.value)?;
            match val {
                Value::Scalar(sx) => evaluated_args.push(sx),
                Value::Error => return Some(Value::Error),
                other => {
                    self.diags.push(
                        Diag::error(
                            arg.value.span.clone(),
                            format!("effect argument expected scalar, found {}", other.kind()),
                        )
                        .with_help("effect parameters must be scalar values"),
                    );
                    return Some(Value::Error);
                }
            }
        }

        let layer_id = self.hir.layer(Layer::UserEffect {
            def_idx: effect_reg.def_idx,
            inner: inner_layer,
            args: evaluated_args,
            span: call_span.clone(),
        });

        Some(Value::Layer(layer_id))
    }
}
