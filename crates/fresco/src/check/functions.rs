use super::*;
use std::collections::HashSet;
use std::rc::Rc;
use tracing::trace_span;
use web_time::Instant;

type InlineWrites = BTreeMap<(usize, String), Value>;

fn has_errors(diags: &[Diag]) -> bool {
    diags.iter().any(|d| d.severity == Severity::Error)
}

#[expect(
    clippy::too_many_arguments,
    reason = "This compiler boundary explicitly threads independent checking or lowering inputs."
)]
fn parse_fn_type(
    name: &str,
    span: &Span,
    diags: &mut Vec<Diag>,
    context: &str,
    source_file: Option<&str>,
    enum_defs: &HashMap<String, EnumDef>,
    struct_defs: &HashMap<String, StructDef>,
    type_params: &[String],
    interface_names: &HashSet<String>,
) -> Option<FnValueTy> {
    let name = strip_spatial_type_suffix(name);
    if let Some(resource) = crate::resource_type::shader_resource_type(name) {
        return match resource {
            Ok(resource) => Some(FnValueTy::ShaderResource(resource)),
            Err(message) => {
                let mut diagnostic = Diag::error(span.clone(), message);
                if let Some(file) = source_file {
                    diagnostic = diagnostic.with_file(file);
                }
                diags.push(diagnostic);
                None
            }
        };
    }
    if let Some((element, length)) = hir::parse_array_param_type(name) {
        if length == 0 {
            diags.push(Diag::error(
                span.clone(),
                "function array types require a nonzero length",
            ));
            return None;
        }
        parse_fn_type(
            element,
            span,
            diags,
            "array element type",
            source_file,
            enum_defs,
            struct_defs,
            &[],
            interface_names,
        )?;
        return Some(FnValueTy::Array(
            name.chars().filter(|c| !c.is_whitespace()).collect(),
        ));
    }

    if let Some((param_tys_str, ret_str)) = try_parse_callable_ty(name) {
        let param_tys: Option<Vec<FnValueTy>> = param_tys_str
            .iter()
            .map(|p| {
                parse_fn_type(
                    p,
                    span,
                    diags,
                    "callable parameter type",
                    source_file,
                    enum_defs,
                    struct_defs,
                    type_params,
                    interface_names,
                )
            })
            .collect();
        let param_tys = param_tys?;
        let ret_ty = Box::new(parse_fn_type(
            &ret_str,
            span,
            diags,
            "callable return type",
            source_file,
            enum_defs,
            struct_defs,
            type_params,
            interface_names,
        )?);
        return Some(FnValueTy::Callable {
            params: param_tys,
            ret: ret_ty,
        });
    }
    if let Some((base, args)) = try_parse_type_application(name)
        && base == "field"
    {
        if args.len() != 1 {
            let mut d = Diag::error(
                span.clone(),
                format!(
                    "`field<...>` expects exactly one type argument, found {}",
                    args.len()
                ),
            )
            .with_help("use `field<T>` with a single element type parameter");
            if let Some(file) = source_file {
                d = d.with_file(file);
            }
            diags.push(d);
            return None;
        }
        return parse_fn_type(
            &args[0],
            span,
            diags,
            context,
            source_file,
            enum_defs,
            struct_defs,
            type_params,
            interface_names,
        );
    }
    if let Some((base, _args)) = try_parse_type_application(name)
        && base == "texture"
    {
        return Some(FnValueTy::Texture);
    }
    match name {
        "f32" | "f64" | "half" | "i32" | "u32" | "signal" | "mask" | "coverage" | "delta"
        | "bool" | "angle" | "length" => Some(FnValueTy::Scalar),
        "vec2" | "uvec2" | "ivec2" | "bvec2" | "coord" | "resolution" => Some(FnValueTy::Vec2),
        "vec3" | "uvec3" | "ivec3" | "bvec3" => Some(FnValueTy::Vec3),
        "vec4" | "uvec4" | "ivec4" | "bvec4" => Some(FnValueTy::Vec4),
        "mat2" => Some(FnValueTy::Mat2),
        "mat3" => Some(FnValueTy::Mat3),
        "mat4" => Some(FnValueTy::Mat4),
        "coord_like" => Some(FnValueTy::CoordLike),
        "color" => Some(FnValueTy::Color),
        "shape" => Some(FnValueTy::Shape),
        "layer" => Some(FnValueTy::Layer),
        other => {
            if type_params.iter().any(|param| param == other) {
                return Some(FnValueTy::TypeVar(other.to_string()));
            }
            if interface_names.contains(other) {
                let mut d = Diag::error(
                    span.clone(),
                    format!("interface `{other}` cannot be used as a value type in v1"),
                )
                .with_help(format!(
                    "use a generic type parameter with an interface bound instead, e.g. `<T: {other}>`"
                ));
                if let Some(file) = source_file {
                    d = d.with_file(file);
                }
                diags.push(d);
                return None;
            }
            // Check if it's a user-defined enum type
            if enum_defs.contains_key(other) {
                return Some(FnValueTy::Enum(other.to_string()));
            }
            if struct_defs.contains_key(other) {
                return Some(FnValueTy::Struct(other.to_string()));
            }

            let mut d = Diag::error(
                span.clone(),
                format!("unsupported function {context} `{other}`"),
            )
            .with_help(
                "supported types are family-first: scalar aliases (f32/f64/half/i32/u32/bool/signal/delta/mask/coverage/angle/length), vector aliases (vec2/uvec2/coord/coord_like/resolution, vec3/uvec3, vec4/uvec4), matrix aliases (mat2/mat3/mat4), texture<T>, color, shape, layer, fixed-size arrays, records, or a defined enum",
            );
            if let Some(file) = source_file {
                d = d.with_file(file);
            }
            diags.push(d);
            None
        }
    }
}

fn strip_spatial_type_suffix(name: &str) -> &str {
    let trimmed = name.trim();

    if let Some((base, suffix)) = trimmed.rsplit_once(" in ")
        && !base.trim().is_empty()
        && !suffix.trim().is_empty()
    {
        return base.trim();
    }

    if let Some((base, suffix)) = trimmed.rsplit_once(" from ")
        && !base.trim().is_empty()
        && !suffix.trim().is_empty()
        && let Some((_src, dst)) = suffix.rsplit_once(" to ")
        && !dst.trim().is_empty()
    {
        return base.trim();
    }

    trimmed
}

impl Checker {
    pub(super) fn eval_standalone_stmt_block(
        &mut self,
        block_name: &str,
        source_file: &str,
        span: &Span,
        stmts: &[Stmt],
    ) -> Option<Value> {
        let synthetic = FnDef {
            infer_return: true,
            ret_kind: crate::typed_scalar::Kind::F32,
            params: Vec::new(),
            ret: FnValueTy::Scalar,
            is_builtin: false,
            is_internal: false,
            source_file: source_file.to_string(),
            body: Vec::new(),
            span: span.clone(),
            type_params: Vec::new(),
            const_params: Vec::new(),
            const_bindings: Vec::new(),
        };

        self.scopes.push(HashMap::new());
        let flow = self.eval_fn_stmt_list(block_name, &synthetic, stmts, false);
        self.scopes.pop();
        let flow = flow?;

        if let Some((value, _)) = self.finish_inline_value(block_name, &synthetic, flow) {
            return Some(value);
        }

        self.diags.push(
            Diag::error(
                span.clone(),
                format!(
                    "{block_name} must return a value; add `return ...` or a trailing expression"
                ),
            )
            .with_file(source_file.to_string()),
        );
        None
    }

    fn user_helper_sxvec_is_sample_point_invariant(
        v: &hir::SxVec,
        helper_invariant: &HashMap<String, bool>,
    ) -> bool {
        match v {
            hir::SxVec::V2(v) => {
                Self::user_helper_sx_is_sample_point_invariant(&v.0, helper_invariant)
                    && Self::user_helper_sx_is_sample_point_invariant(&v.1, helper_invariant)
            }
            hir::SxVec::V3(v) => {
                Self::user_helper_sx_is_sample_point_invariant(&v.0, helper_invariant)
                    && Self::user_helper_sx_is_sample_point_invariant(&v.1, helper_invariant)
                    && Self::user_helper_sx_is_sample_point_invariant(&v.2, helper_invariant)
            }
            hir::SxVec::V4(v) => {
                Self::user_helper_sx_is_sample_point_invariant(&v.0, helper_invariant)
                    && Self::user_helper_sx_is_sample_point_invariant(&v.1, helper_invariant)
                    && Self::user_helper_sx_is_sample_point_invariant(&v.2, helper_invariant)
                    && Self::user_helper_sx_is_sample_point_invariant(&v.3, helper_invariant)
            }
        }
    }

    fn user_helper_sx_is_sample_point_invariant(
        s: &Sx,
        helper_invariant: &HashMap<String, bool>,
    ) -> bool {
        match s {
            Sx::Lit(_) | Sx::EntryInput(_) | Sx::UniformField { .. } | Sx::Param(_) => true,
            Sx::Neg(a)
            | Sx::Sin(a)
            | Sx::Cos(a)
            | Sx::Tan(a)
            | Sx::Asin(a)
            | Sx::Acos(a)
            | Sx::Atan(a)
            | Sx::Sqrt(a)
            | Sx::InverseSqrt(a)
            | Sx::Fract(a)
            | Sx::Abs(a)
            | Sx::Sign(a)
            | Sx::Floor(a)
            | Sx::Ceil(a)
            | Sx::Round(a)
            | Sx::Trunc(a)
            | Sx::Exp(a)
            | Sx::Exp2(a)
            | Sx::Log(a)
            | Sx::Log2(a)
            | Sx::SrgbToLinear(a)
            | Sx::LinearToSrgb(a) => {
                Self::user_helper_sx_is_sample_point_invariant(a, helper_invariant)
            }
            Sx::Add(a, b)
            | Sx::Sub(a, b)
            | Sx::Mul(a, b)
            | Sx::Div(a, b)
            | Sx::Lt(a, b)
            | Sx::Le(a, b)
            | Sx::Gt(a, b)
            | Sx::Ge(a, b)
            | Sx::Eq(a, b)
            | Sx::Ne(a, b)
            | Sx::Atan2(a, b)
            | Sx::Pow(a, b)
            | Sx::Min(a, b)
            | Sx::Max(a, b)
            | Sx::Step(a, b) => {
                Self::user_helper_sx_is_sample_point_invariant(a, helper_invariant)
                    && Self::user_helper_sx_is_sample_point_invariant(b, helper_invariant)
            }
            Sx::Clamp(x, lo, hi)
            | Sx::Mix(x, lo, hi)
            | Sx::Select(x, lo, hi)
            | Sx::SmoothStep(x, lo, hi) => {
                Self::user_helper_sx_is_sample_point_invariant(x, helper_invariant)
                    && Self::user_helper_sx_is_sample_point_invariant(lo, helper_invariant)
                    && Self::user_helper_sx_is_sample_point_invariant(hi, helper_invariant)
            }
            Sx::Dot { a, b } => {
                Self::user_helper_sxvec_is_sample_point_invariant(a, helper_invariant)
                    && Self::user_helper_sxvec_is_sample_point_invariant(b, helper_invariant)
            }
            Sx::NormalizeComponent { v, .. } | Sx::Length(v) => {
                Self::user_helper_sxvec_is_sample_point_invariant(v, helper_invariant)
            }
            Sx::MinComponent { a, b, .. } | Sx::MaxComponent { a, b, .. } => {
                Self::user_helper_sxvec_is_sample_point_invariant(a, helper_invariant)
                    && Self::user_helper_sxvec_is_sample_point_invariant(b, helper_invariant)
            }
            Sx::ClampVecComponent { x, lo, hi, .. } => {
                Self::user_helper_sxvec_is_sample_point_invariant(x, helper_invariant)
                    && Self::user_helper_sxvec_is_sample_point_invariant(lo, helper_invariant)
                    && Self::user_helper_sxvec_is_sample_point_invariant(hi, helper_invariant)
            }
            Sx::UserCall { call, .. } => {
                call.args.iter().all(|arg| {
                    Self::user_helper_sx_is_sample_point_invariant(arg, helper_invariant)
                }) && helper_invariant
                    .get(&call.helper_id)
                    .copied()
                    .unwrap_or(false)
            }
            _ => false,
        }
    }

    fn user_helper_body_is_sample_point_invariant(
        body_stmts: &[hir::UserFnStmt],
        helper_invariant: &HashMap<String, bool>,
    ) -> bool {
        let mut invariant = true;
        for stmt in body_stmts {
            stmt.walk_sx(&mut |sx| {
                if invariant
                    && !Self::user_helper_sx_is_sample_point_invariant(sx, helper_invariant)
                {
                    invariant = false;
                }
            });
            if !invariant {
                break;
            }
        }
        invariant
    }

    fn compute_user_helper_sample_point_invariance(&mut self) {
        if self.hir.user_helpers.is_empty() {
            return;
        }

        let mut helper_invariant: HashMap<String, bool> = self
            .hir
            .user_helpers
            .keys()
            .map(|id| (id.clone(), true))
            .collect();

        for _ in 0..self.hir.user_helpers.len().max(1) {
            let mut changed = false;
            for (id, helper) in &self.hir.user_helpers {
                let invariant = Self::user_helper_body_is_sample_point_invariant(
                    &helper.body_stmts,
                    &helper_invariant,
                );
                let prior = helper_invariant.get(id).copied().unwrap_or(true);
                if prior != invariant {
                    helper_invariant.insert(id.clone(), invariant);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }

        // Propagate entry-input captures through nested helper calls. Pure helpers
        // keep their existing signatures; only capturing helpers receive context.
        let mut needs_entry_inputs = HashSet::new();
        loop {
            let mut changed = false;
            for (id, helper) in &self.hir.user_helpers {
                if needs_entry_inputs.contains(id) {
                    continue;
                }
                let mut needs = false;
                for stmt in &helper.body_stmts {
                    stmt.walk_sx(&mut |sx| sx.walk_preorder(&mut |node| {
                        if matches!(node, Sx::EntryInput(_) | Sx::CoordX | Sx::CoordY)
                            || matches!(node, Sx::UserCall { call, .. } if needs_entry_inputs.contains(&call.helper_id)) {
                            needs = true;
                        }
                    }));
                }
                if needs {
                    changed |= needs_entry_inputs.insert(id.clone());
                }
            }
            if !changed {
                break;
            }
        }

        for (id, helper) in &mut self.hir.user_helpers {
            helper.sample_point_invariant = helper_invariant.get(id).copied().unwrap_or(false);
            helper.needs_entry_inputs = needs_entry_inputs.contains(id);
        }
    }

    fn enum_variants_in_decl_order(&self, enum_name: &str) -> Option<Vec<String>> {
        let enum_def = self.enum_defs.get(enum_name)?;
        let mut variants = enum_def
            .variants
            .iter()
            .map(|(name, span)| (name.clone(), span.start))
            .collect::<Vec<_>>();
        variants.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        Some(variants.into_iter().map(|(name, _)| name).collect())
    }

    fn enum_variant_name_from_scalar(&self, enum_name: &str, sx: &Sx) -> Option<String> {
        let ordinal = Self::try_eval_static_scalar(sx)?;
        if !ordinal.is_finite() {
            return None;
        }
        let rounded = ordinal.round();
        if (rounded - ordinal).abs() > 1e-6 || rounded < 0.0 {
            return None;
        }
        let idx = rounded as usize;
        self.enum_variants_in_decl_order(enum_name)?
            .get(idx)
            .cloned()
    }

    fn validate_enum_scalar_value(
        &mut self,
        enum_name: &str,
        value: &Value,
        span: &Span,
        context: &str,
    ) -> bool {
        let Some((sx, _kind)) = Self::as_numeric_scalar(value) else {
            self.diags.push(
                Diag::error(
                    span.clone(),
                    format!(
                        "{context} expected enum `{enum_name}` variant, found {}",
                        value.kind()
                    ),
                )
                .with_label("enum type mismatch"),
            );
            return false;
        };

        // If the scalar is a compile-time constant, validate it maps to a valid variant ordinal.
        // Runtime values (parameters, helper call results, slot variables) cannot be statically
        // evaluated and are accepted as-is: their type is already known to be scalar, and they
        // originate from a validated enum-typed source (parameter placeholder or call site).
        if Self::try_eval_static_scalar(&sx).is_none() {
            return true;
        }

        if self.enum_variant_name_from_scalar(enum_name, &sx).is_some() {
            return true;
        }

        let variants = self
            .enum_variants_in_decl_order(enum_name)
            .unwrap_or_default();
        let help = if variants.is_empty() {
            format!("use a declared `{enum_name}` variant")
        } else {
            format!(
                "use `{enum_name}.<variant>` where <variant> is one of: {}",
                variants.join(", ")
            )
        };
        self.diags.push(
            Diag::error(
                span.clone(),
                format!("{context} is not a valid `{enum_name}` variant value"),
            )
            .with_label("invalid enum variant value")
            .with_help(help),
        );
        false
    }

    fn validate_enum_call_arg_expr(
        &mut self,
        expected_enum: &str,
        arg: &SExpr,
        fn_name: &str,
        param_name: &str,
    ) -> bool {
        let Expr::Var(name) = &arg.node else {
            return true;
        };

        if let Some((enum_name, variant_name)) = name.split_once('.') {
            let Some(enum_def) = self.enum_defs.get(enum_name) else {
                return true;
            };
            if !enum_def.variants.contains_key(variant_name) {
                return true;
            }
            if enum_name != expected_enum {
                self.diags.push(
                    Diag::error(
                        arg.span.clone(),
                        format!(
                            "argument `{param_name}` to `{fn_name}` expected enum `{expected_enum}`, found variant `{enum_name}.{variant_name}`"
                        ),
                    )
                    .with_label("enum type mismatch"),
                );
                return false;
            }
            return true;
        }

        let owners: Vec<&str> = self
            .enum_defs
            .iter()
            .filter_map(|(enum_name, enum_def)| {
                enum_def
                    .variants
                    .contains_key(name)
                    .then_some(enum_name.as_str())
            })
            .collect();
        if owners.len() == 1 && owners[0] != expected_enum {
            self.diags.push(
                Diag::error(
                    arg.span.clone(),
                    format!(
                        "argument `{param_name}` to `{fn_name}` expected enum `{expected_enum}`, found variant `{}`",
                        name
                    ),
                )
                .with_label("enum type mismatch")
                .with_help(format!(
                    "qualify the expected enum variant, for example `{expected_enum}.{name}`"
                )),
            );
            return false;
        }
        true
    }

    fn postprocess_source_color() -> Value {
        Value::ColorField {
            rgba: [
                Sx::PostColorR,
                Sx::PostColorG,
                Sx::PostColorB,
                Sx::PostColorA,
            ],
            space: ColorSpace::Linear,
        }
    }

    fn postprocess_uv_value() -> Value {
        Value::Vec2((Sx::CoordX, Sx::CoordY))
    }

    fn postprocess_time_value(&mut self) -> Value {
        Value::Scalar(self.runtime_time())
    }

    fn postprocess_delta_value(&mut self) -> Value {
        Value::Scalar(self.runtime_delta_time())
    }

    fn postprocess_res_value(&mut self) -> Value {
        let (x, y) = self.runtime_resolution();
        Value::Vec2((x, y))
    }

    fn eval_postprocess_fn_body(&mut self, name: &str, def: &FnDef) -> Option<Value> {
        if self.fn_call_stack.contains(&def.declaration_identity(name)) {
            self.diags.push(
                Diag::error(
                    def.span.clone(),
                    format!("recursive function call to `{name}` is not supported in v0"),
                )
                .with_file(def.source_file.clone())
                .with_help(
                    "use non-recursive helper functions until recursive evaluation is implemented",
                ),
            );
            return None;
        }
        if self.fn_call_stack.len() > 32 {
            self.diags.push(
                Diag::error(
                    def.span.clone(),
                    format!("function call depth exceeded while evaluating `{name}`"),
                )
                .with_file(def.source_file.clone())
                .with_help("reduce nested function call depth"),
            );
            return None;
        }

        let mut bound_values = Vec::with_capacity(def.params.len());
        bound_values.push((def.params[0].name.clone(), Self::postprocess_source_color()));
        bound_values.push((def.params[1].name.clone(), Self::postprocess_uv_value()));
        bound_values.push((def.params[2].name.clone(), self.postprocess_time_value()));
        if def.params.len() == 5 {
            bound_values.push((def.params[3].name.clone(), self.postprocess_delta_value()));
            bound_values.push((def.params[4].name.clone(), self.postprocess_res_value()));
        } else {
            bound_values.push((def.params[3].name.clone(), self.postprocess_res_value()));
        }

        self.fn_call_stack.push(def.declaration_identity(name));
        self.scopes.push(HashMap::new());
        for (const_name, const_value) in &def.const_bindings {
            let bound = match const_value {
                ConstTemplateValue::U32(v) => Value::Scalar(
                    crate::typed_scalar::Scalar::from_literal(naga::Literal::U32(*v)),
                ),
                ConstTemplateValue::I32(v) => Value::Scalar(
                    crate::typed_scalar::Scalar::from_literal(naga::Literal::I32(*v)),
                ),
            };
            self.bind(const_name.clone(), bound);
        }
        for (param_name, value) in bound_values {
            self.bind(param_name, value);
        }
        let diag_start = self.diags.len();
        let out = self.eval_fn_body(name, def);
        for d in self.diags.iter_mut().skip(diag_start) {
            if d.file.is_none() {
                d.file = Some(def.source_file.clone());
            }
        }
        self.scopes.pop();
        self.fn_call_stack.pop();
        out
    }

    pub(super) fn eval_postprocess_fn_ref(
        &mut self,
        name: &str,
        name_span: &Span,
        call_span: &Span,
    ) -> Option<[Sx; 4]> {
        let Some(defs) = self.fn_defs(name) else {
            self.diags.push(
                Diag::error(name_span.clone(), format!("unknown postprocess function `{name}`"))
                    .with_help(
                        "declare a function like `fn finish(src: color, uv: coord_like, time: signal, res: resolution) -> color`",
                    ),
            );
            return None;
        };

        let matches_signature = |def: &FnDef| {
            if def.is_builtin || !matches!(def.ret, FnValueTy::Color) {
                return false;
            }

            match def.params.as_slice() {
                [a, b, c, d]
                    if matches!(a.ty, FnValueTy::Color)
                        && matches!(b.ty, FnValueTy::CoordLike)
                        && matches!(c.ty, FnValueTy::Scalar)
                        && matches!(d.ty, FnValueTy::Vec2) =>
                {
                    true
                }
                [a, b, c, d, e]
                    if matches!(a.ty, FnValueTy::Color)
                        && matches!(b.ty, FnValueTy::CoordLike)
                        && matches!(c.ty, FnValueTy::Scalar)
                        && matches!(d.ty, FnValueTy::Scalar)
                        && matches!(e.ty, FnValueTy::Vec2) =>
                {
                    true
                }
                _ => false,
            }
        };

        let mut matches = defs.into_iter().filter(matches_signature);
        let Some(def) = matches.next() else {
            self.diags.push(
                Diag::error(
                    call_span.clone(),
                    format!("`postprocess({name})` requires a function with a supported signature"),
                )
                .with_help(
                    "supported signatures: `(src: color, uv: coord_like, time: signal, res: resolution) -> color` or `(src: color, uv: coord_like, time: signal, dt: delta, res: resolution) -> color`",
                ),
            );
            return None;
        };

        if matches.next().is_some() {
            self.diags.push(
                Diag::error(
                    name_span.clone(),
                    format!("`postprocess({name})` is ambiguous across multiple overloads"),
                )
                .with_help(
                    "keep exactly one postprocess-compatible overload for this function name",
                ),
            );
            return None;
        }

        let out = self.eval_postprocess_fn_body(name, &def)?;
        match out {
            Value::Color { rgba, .. } => Some(Self::color_to_field(rgba)),
            Value::ColorField { rgba, .. } => Some(rgba),
            other => {
                self.diags.push(
                    Diag::error(
                        call_span.clone(),
                        format!(
                            "postprocess function `{name}` must return color, found {}",
                            other.kind()
                        ),
                    )
                    .with_file(def.source_file)
                    .with_help(
                        "declare the function with `-> color` and return an rgba(...) value",
                    ),
                );
                None
            }
        }
    }

    fn cond_mask_from_scalar(cond: Sx) -> Sx {
        if let Some(v) = Self::try_eval_static_scalar(&cond) {
            return Sx::Lit(if v != 0.0 { 1.0 } else { 0.0 });
        }
        // Function `if` conditions use non-zero truthiness.
        // Build a 0/1 mask that works for both positive and negative non-zero values.
        let sign = Sx::Sign(Box::new(cond));
        let abs = Sx::Abs(Box::new(sign));
        Sx::Min(Box::new(Sx::Lit(1.0)), Box::new(abs))
    }

    fn select_sx_on_cond(cond: Sx, then_value: Sx, else_value: Sx) -> Sx {
        if let Sx::Lit(v) = cond {
            return if v != 0.0 { then_value } else { else_value };
        }
        crate::typed_scalar::Scalar::select(else_value, then_value, cond)
    }

    pub(super) fn merge_conditional_values(
        &mut self,
        fn_name: &str,
        source_file: &str,
        cond: Sx,
        then_value: Value,
        else_value: Value,
        span: &Span,
    ) -> Option<Value> {
        match (then_value, else_value) {
            (Value::Scalar(a), Value::Scalar(b)) => {
                Some(Value::Scalar(Self::select_sx_on_cond(cond, a, b)))
            }
            (Value::Distance(a), Value::Distance(b)) => {
                Some(Value::Distance(Self::select_sx_on_cond(cond, a, b)))
            }
            (Value::Coverage(a), Value::Coverage(b)) => {
                Some(Value::Coverage(Self::select_sx_on_cond(cond, a, b)))
            }
            (Value::Mask(a), Value::Mask(b)) => {
                Some(Value::Mask(Self::select_sx_on_cond(cond, a, b)))
            }
            (Value::Vec2((ax, ay)), Value::Vec2((bx, by))) => Some(Value::Vec2((
                Self::select_sx_on_cond(cond.clone(), ax, bx),
                Self::select_sx_on_cond(cond, ay, by),
            ))),
            (Value::Vec3((ax, ay, az)), Value::Vec3((bx, by, bz))) => Some(Value::Vec3((
                Self::select_sx_on_cond(cond.clone(), ax, bx),
                Self::select_sx_on_cond(cond.clone(), ay, by),
                Self::select_sx_on_cond(cond, az, bz),
            ))),
            (Value::Vec4((ax, ay, az, aw)), Value::Vec4((bx, by, bz, bw))) => Some(Value::Vec4((
                Self::select_sx_on_cond(cond.clone(), ax, bx),
                Self::select_sx_on_cond(cond.clone(), ay, by),
                Self::select_sx_on_cond(cond.clone(), az, bz),
                Self::select_sx_on_cond(cond, aw, bw),
            ))),
            (
                Value::Color {
                    rgba: a_rgba,
                    space: a_space,
                },
                Value::Color {
                    rgba: b_rgba,
                    space: b_space,
                },
            ) => {
                let mut a = Self::color_to_field(a_rgba);
                let mut b = Self::color_to_field(b_rgba);
                if a_space != b_space {
                    a = Self::to_working_color_space(a, a_space);
                    b = Self::to_working_color_space(b, b_space);
                }
                Some(Value::ColorField {
                    rgba: [
                        Self::select_sx_on_cond(cond.clone(), a[0].clone(), b[0].clone()),
                        Self::select_sx_on_cond(cond.clone(), a[1].clone(), b[1].clone()),
                        Self::select_sx_on_cond(cond.clone(), a[2].clone(), b[2].clone()),
                        Self::select_sx_on_cond(cond, a[3].clone(), b[3].clone()),
                    ],
                    space: if a_space == b_space {
                        a_space
                    } else {
                        ColorSpace::Linear
                    },
                })
            }
            (
                Value::Color { rgba, space },
                Value::ColorField {
                    rgba: b,
                    space: b_space,
                },
            ) => {
                let mut a = Self::color_to_field(rgba);
                if space != b_space {
                    a = Self::to_working_color_space(a, space);
                }
                let b = if space == b_space {
                    b
                } else {
                    Self::to_working_color_space(b, b_space)
                };
                Some(Value::ColorField {
                    rgba: [
                        Self::select_sx_on_cond(cond.clone(), a[0].clone(), b[0].clone()),
                        Self::select_sx_on_cond(cond.clone(), a[1].clone(), b[1].clone()),
                        Self::select_sx_on_cond(cond.clone(), a[2].clone(), b[2].clone()),
                        Self::select_sx_on_cond(cond, a[3].clone(), b[3].clone()),
                    ],
                    space: if space == b_space {
                        space
                    } else {
                        ColorSpace::Linear
                    },
                })
            }
            (
                Value::ColorField {
                    rgba: a,
                    space: a_space,
                },
                Value::Color { rgba, space },
            ) => {
                let mut b = Self::color_to_field(rgba);
                if a_space != space {
                    b = Self::to_working_color_space(b, space);
                }
                let a = if a_space == space {
                    a
                } else {
                    Self::to_working_color_space(a, a_space)
                };
                Some(Value::ColorField {
                    rgba: [
                        Self::select_sx_on_cond(cond.clone(), a[0].clone(), b[0].clone()),
                        Self::select_sx_on_cond(cond.clone(), a[1].clone(), b[1].clone()),
                        Self::select_sx_on_cond(cond.clone(), a[2].clone(), b[2].clone()),
                        Self::select_sx_on_cond(cond, a[3].clone(), b[3].clone()),
                    ],
                    space: if a_space == space {
                        a_space
                    } else {
                        ColorSpace::Linear
                    },
                })
            }
            (
                Value::ColorField {
                    rgba: a,
                    space: a_space,
                },
                Value::ColorField {
                    rgba: b,
                    space: b_space,
                },
            ) => {
                let a = if a_space == b_space {
                    a
                } else {
                    Self::to_working_color_space(a, a_space)
                };
                let b = if a_space == b_space {
                    b
                } else {
                    Self::to_working_color_space(b, b_space)
                };
                Some(Value::ColorField {
                    rgba: [
                        Self::select_sx_on_cond(cond.clone(), a[0].clone(), b[0].clone()),
                        Self::select_sx_on_cond(cond.clone(), a[1].clone(), b[1].clone()),
                        Self::select_sx_on_cond(cond.clone(), a[2].clone(), b[2].clone()),
                        Self::select_sx_on_cond(cond, a[3].clone(), b[3].clone()),
                    ],
                    space: if a_space == b_space {
                        a_space
                    } else {
                        ColorSpace::Linear
                    },
                })
            }
            (Value::Layer(then_layer), Value::Layer(else_layer)) => {
                let layer = self.hir.layer(Layer::If {
                    cond,
                    then_layer,
                    else_layer,
                });
                Some(Value::Layer(layer))
            }
            (
                Value::Struct { ty_name, fields: a },
                Value::Struct {
                    ty_name: b_name,
                    mut fields,
                },
            ) if ty_name == b_name && a.len() == fields.len() => {
                let mut keys: Vec<_> = a.keys().collect();
                keys.sort();
                for key in keys {
                    let b = fields
                        .remove(key)
                        .expect("same struct type has the same fields");
                    let value = self.merge_conditional_values(
                        fn_name,
                        source_file,
                        cond.clone(),
                        a[key].clone(),
                        b,
                        span,
                    )?;
                    fields.insert(key.clone(), value);
                }
                Some(Value::Struct { ty_name, fields })
            }
            (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
                let values = a
                    .into_iter()
                    .zip(b)
                    .map(|(a, b)| {
                        self.merge_conditional_values(
                            fn_name,
                            source_file,
                            cond.clone(),
                            a,
                            b,
                            span,
                        )
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some(Value::Array(values))
            }
            (then_value, else_value) => {
                self.diags.push(
                    Diag::error(
                        span.clone(),
                        format!(
                            "function `{fn_name}` has incompatible `if` branch value kinds: {} vs {}",
                            then_value.kind(),
                            else_value.kind()
                        ),
                    )
                    .with_file(source_file)
                    .with_help("make both branches return the same value kind"),
                );
                None
            }
        }
    }

    fn inline_mask_and(a: Sx, b: Sx) -> Sx {
        match (&a, &b) {
            (Sx::Lit(0.0), _) | (_, Sx::Lit(0.0)) => Sx::Lit(0.0),
            (Sx::Lit(1.0), _) => b,
            (_, Sx::Lit(1.0)) => a,
            _ => Sx::Mul(Box::new(a), Box::new(b)),
        }
    }

    fn inline_mask_not(a: Sx) -> Sx {
        match a {
            Sx::Lit(v) => Sx::Lit(1.0 - v),
            other => Sx::Sub(Box::new(Sx::Lit(1.0)), Box::new(other)),
        }
    }

    fn inline_mask_or(a: Option<Sx>, b: Sx) -> Sx {
        match a {
            None => b,
            Some(a) => Self::inline_mask_not(Self::inline_mask_and(
                Self::inline_mask_not(a),
                Self::inline_mask_not(b),
            )),
        }
    }

    fn capture_inline_writes(
        &mut self,
        fn_name: &str,
        def: &FnDef,
        stmts: &[Stmt],
        in_loop: bool,
    ) -> Option<(ReturnPathAnalysis, InlineWrites)> {
        let before = self.scopes.clone();
        self.assignment_scopes.push(Default::default());
        let flow = self.eval_fn_stmt_list(fn_name, def, stmts, in_loop);
        let writes = self
            .assignment_scopes
            .pop()
            .expect("inline write tracking frame");
        let mut changed = InlineWrites::new();
        for (index, name) in writes {
            if let Some(old) = before.get(index).and_then(|scope| scope.get(&name)) {
                let new = self.scopes[index]
                    .insert(name.clone(), old.clone())
                    .expect("assigned local remains in its lexical scope");
                changed.insert((index, name), new);
            }
        }
        Some((flow?, changed))
    }

    fn merge_inline_writes(
        &mut self,
        fn_name: &str,
        def: &FnDef,
        cond: Sx,
        then_writes: InlineWrites,
        mut else_writes: InlineWrites,
        span: &Span,
    ) -> Option<()> {
        let mut merged = InlineWrites::new();
        for ((index, name), then_value) in then_writes {
            let else_value = else_writes
                .remove(&(index, name.clone()))
                .unwrap_or_else(|| self.scopes[index][&name].clone());
            let value = if let Sx::Lit(v) = cond {
                if v != 0.0 { then_value } else { else_value }
            } else {
                self.merge_conditional_values(
                    fn_name,
                    &def.source_file,
                    cond.clone(),
                    then_value,
                    else_value,
                    span,
                )?
            };
            merged.insert((index, name), value);
        }
        for ((index, name), else_value) in else_writes {
            let then_value = self.scopes[index][&name].clone();
            let value = if let Sx::Lit(v) = cond {
                if v != 0.0 { then_value } else { else_value }
            } else {
                self.merge_conditional_values(
                    fn_name,
                    &def.source_file,
                    cond.clone(),
                    then_value,
                    else_value,
                    span,
                )?
            };
            merged.insert((index, name), value);
        }
        for ((index, name), value) in merged {
            // Keep accumulated loop state shared when both a guard and an
            // assignment reference it on the next iteration.
            let mut pending = Vec::new();
            let value =
                Self::placeholder_for_call_arg(&name, value, &mut self.let_counter, &mut pending);
            let value = Self::apply_pending_lets(value, &pending);
            self.scopes[index].insert(name.clone(), value);
            for writes in &mut self.assignment_scopes {
                writes.insert((index, name.clone()));
            }
        }
        Some(())
    }

    fn finish_inline_value(
        &mut self,
        fn_name: &str,
        def: &FnDef,
        flow: ReturnPathAnalysis,
    ) -> Option<(Value, Span)> {
        let (mut value, span) = flow.guaranteed_return.or(flow.fallthrough_value)?;
        for (cond, returned, return_span) in flow.conditional_returns.into_iter().rev() {
            value = self.merge_conditional_values(
                fn_name,
                &def.source_file,
                cond,
                returned,
                value,
                &return_span,
            )?;
        }
        Some((value, span))
    }

    fn eval_fn_stmt_list(
        &mut self,
        fn_name: &str,
        def: &FnDef,
        stmts: &[Stmt],
        in_loop: bool,
    ) -> Option<ReturnPathAnalysis> {
        let mut flow = ReturnPathAnalysis::default();
        let mut active = Sx::Lit(1.0);
        for stmt in stmts {
            if let Some(exit_span) = flow
                .guaranteed_return_span
                .as_ref()
                .or(flow.break_span.as_ref())
            {
                let (span, kind) = fn_stmt_span_kind(stmt);
                let exit = if flow.guaranteed_return_span.is_some() {
                    "return"
                } else {
                    "break"
                };
                self.diags.push(Diag::error(span,
                    format!("function `{fn_name}` has unreachable `{kind}` statement after `{exit}`"))
                    .with_file(def.source_file.clone())
                    .with_label("this statement is unreachable")
                    .with_help(format!("remove unreachable statements or move them before the {exit} at {exit_span:?}")));
                return None;
            }
            let mut next = if matches!(active, Sx::Lit(1.0)) {
                self.eval_fn_stmt_list_inner(fn_name, def, std::slice::from_ref(stmt), in_loop)?
            } else {
                let (next, writes) =
                    self.capture_inline_writes(fn_name, def, std::slice::from_ref(stmt), in_loop)?;
                self.merge_inline_writes(
                    fn_name,
                    def,
                    active.clone(),
                    writes,
                    InlineWrites::new(),
                    &fn_stmt_span_kind(stmt).0,
                )?;
                next
            };
            if matches!(stmt, Stmt::If { .. } | Stmt::Match { .. }) {
                flow.fallthrough_value = next.fallthrough_value.take();
            } else if let Some(value) = next.fallthrough_value.take() {
                flow.fallthrough_value = Some(value);
            }
            let mut exits = next.conditional_break.clone();
            for (cond, value, span) in next.conditional_returns {
                exits = Some(Self::inline_mask_or(exits, cond.clone()));
                flow.conditional_returns.push((
                    Self::inline_mask_and(active.clone(), cond),
                    value,
                    span,
                ));
            }
            if let Some((mut value, span)) = next.guaranteed_return {
                if flow.conditional_break.is_none() {
                    for (cond, returned, return_span) in flow.conditional_returns.drain(..).rev() {
                        value = self.merge_conditional_values(
                            fn_name,
                            &def.source_file,
                            cond,
                            returned,
                            value,
                            &return_span,
                        )?;
                    }
                    flow.guaranteed_return = Some((value, span.clone()));
                    flow.guaranteed_return_span = Some(span);
                    flow.fallthrough_value = None;
                } else {
                    flow.conditional_returns.push((active.clone(), value, span));
                }
                exits = Some(Sx::Lit(1.0));
            }
            if let Some(span) = next.break_span {
                if matches!(active, Sx::Lit(1.0)) {
                    flow.break_span = Some(span);
                } else {
                    flow.conditional_break =
                        Some(Self::inline_mask_or(flow.conditional_break, active.clone()));
                }
                exits = Some(Sx::Lit(1.0));
            }
            if let Some(cond) = next.conditional_break {
                flow.conditional_break = Some(Self::inline_mask_or(
                    flow.conditional_break,
                    Self::inline_mask_and(active.clone(), cond),
                ));
            }
            if let Some(exits) = exits {
                active = Self::inline_mask_and(active, Self::inline_mask_not(exits));
            }
        }
        Some(flow)
    }

    fn eval_fn_stmt_list_inner(
        &mut self,
        fn_name: &str,
        def: &FnDef,
        stmts: &[Stmt],
        in_loop: bool,
    ) -> Option<ReturnPathAnalysis> {
        let mut flow = ReturnPathAnalysis::default();

        for stmt in stmts {
            if let Some(ret_span) = &flow.guaranteed_return_span {
                let (span, kind) = fn_stmt_span_kind(stmt);
                self.diags.push(
                    Diag::error(
                        span,
                        format!(
                            "function `{fn_name}` has unreachable `{kind}` statement after `return`"
                        ),
                    )
                    .with_file(def.source_file.clone())
                    .with_label("this statement is unreachable")
                    .with_help(format!(
                        "remove unreachable statements or move them before the return at {:?}",
                        ret_span
                    )),
                );
                return None;
            }

            if let Some(break_span) = &flow.break_span {
                let (span, kind) = fn_stmt_span_kind(stmt);
                self.diags.push(
                    Diag::error(
                        span,
                        format!(
                            "function `{fn_name}` has unreachable `{kind}` statement after `break`"
                        ),
                    )
                    .with_file(def.source_file.clone())
                    .with_label("this statement is unreachable")
                    .with_help(format!(
                        "remove unreachable statements or move them before the break at {:?}",
                        break_span
                    )),
                );
                return None;
            }

            match stmt {
                Stmt::Store { span, .. } => {
                    self.diags.push(Diag::error(
                        span.clone(),
                        "indexed writes require an explicit GPU stage program",
                    ));
                    return None;
                }
                Stmt::Let {
                    name,
                    value,
                    declared_ty_name,
                    declared_ty_span,
                    ..
                } => {
                    let v = self.eval_scalar_expected(
                        value,
                        declared_ty_name
                            .as_deref()
                            .and_then(crate::typed_scalar::Kind::parse),
                    )?;
                    let type_param_names: Vec<&str> =
                        def.type_params.iter().map(|(n, _)| n.as_str()).collect();
                    if let (Some(ty_name), Some(ty_span)) =
                        (declared_ty_name.as_ref(), declared_ty_span.as_ref())
                        && !local_decl_type_matches(
                            &v,
                            ty_name,
                            &self.enum_defs,
                            &self.struct_defs,
                            &type_param_names,
                        )
                    {
                        self.diags.push(
                            Diag::error(
                                ty_span.clone(),
                                format!(
                                    "typed local `{name}` expected {}, but got {}",
                                    local_decl_expected_kind(
                                        ty_name,
                                        &self.enum_defs,
                                        &self.struct_defs,
                                        &type_param_names
                                    ),
                                    v.kind()
                                ),
                            )
                            .with_file(def.source_file.clone())
                            .with_label("declared type does not match initializer")
                            .with_help("adjust the declared type or initializer expression"),
                        );
                        return None;
                    }
                    self.bind(name.clone(), v);
                }
                Stmt::Const {
                    name,
                    value,
                    ty_name,
                    ty_span,
                    ..
                } => {
                    let v = self
                        .eval_scalar_expected(value, crate::typed_scalar::Kind::parse(ty_name))?;
                    let type_param_names: Vec<&str> =
                        def.type_params.iter().map(|(n, _)| n.as_str()).collect();
                    if !local_decl_type_matches(
                        &v,
                        ty_name,
                        &self.enum_defs,
                        &self.struct_defs,
                        &type_param_names,
                    ) {
                        self.diags.push(
                            Diag::error(
                                ty_span.clone(),
                                format!(
                                    "const `{name}` expected {}, but got {}",
                                    local_decl_expected_kind(
                                        ty_name,
                                        &self.enum_defs,
                                        &self.struct_defs,
                                        &type_param_names,
                                    ),
                                    v.kind()
                                ),
                            )
                            .with_file(def.source_file.clone())
                            .with_label("declared type does not match initializer")
                            .with_help("adjust the declared type or const initializer expression"),
                        );
                        return None;
                    }

                    let what = format!("const `{name}` initializer");
                    let folded = self.eval_compile_time_const_value(&v, &value.span, &what)?;

                    self.bind(name.clone(), folded);
                }
                Stmt::Assign {
                    name,
                    name_span,
                    field_path,
                    value,
                    ..
                } => {
                    // For a plain (non-swizzle) reassignment, hoist the variable's
                    // current value behind a cheap `Sx::Var` placeholder before
                    // evaluating the RHS. A compound update like `t += f(t)`,
                    // repeated across many loop iterations, otherwise references
                    // `t`'s full accumulated expression more than once per
                    // iteration (once directly, once inside `f(t)`), and each
                    // reference clones it — doubling the tree roughly every
                    // iteration. The real expression is grafted back exactly
                    // once via `Sx::Let` after the RHS is computed.
                    let mut pending_lets: Vec<(String, Rc<Sx>)> = Vec::new();
                    if field_path.is_none()
                        && let Some(old) = self.lookup(name)
                    {
                        let placeholder = Self::placeholder_for_call_arg(
                            name,
                            old,
                            &mut self.let_counter,
                            &mut pending_lets,
                        );
                        self.assign(name, placeholder);
                    }
                    {
                        let expected = match self.lookup(name) {
                            Some(Value::Scalar(ref scalar)) if field_path.is_none() => {
                                Some(scalar.scalar_kind())
                            }
                            _ => None,
                        };
                        let mut v = self.eval_scalar_expected(value, expected)?;
                        if !pending_lets.is_empty() {
                            v = Self::apply_pending_lets(v, &pending_lets);
                        }
                        if let Some(field_path) = field_path {
                            if field_path.contains('.') {
                                let result = self
                                    .lookup(name)
                                    .ok_or_else(|| format!("undeclared local `{name}`"))
                                    .and_then(|base| self.assign_field_path(base, field_path, v));
                                match result {
                                    Ok(updated) => {
                                        self.assign(name, updated);
                                    }
                                    Err(message) => {
                                        self.diags.push(Diag::error(name_span.clone(), format!("invalid assignment target `{name}.{field_path}`")).with_file(def.source_file.clone()).with_help(message));
                                        return None;
                                    }
                                }
                                continue;
                            }

                            let Some(base) = self.lookup(name) else {
                                self.diags.push(
                                    Diag::error(
                                        name_span.clone(),
                                        format!("cannot assign to undeclared local `{name}`"),
                                    )
                                    .with_file(def.source_file.clone())
                                    .with_label("unknown assignment target")
                                    .with_help(
                                        "declare the variable first with `let name = ...` or `type name = ...`",
                                    ),
                                );
                                return None;
                            };

                            let updated = match base {
                                Value::Struct {
                                    ty_name,
                                    mut fields,
                                } => {
                                    let Some(field_def) = self
                                        .struct_defs
                                        .get(&ty_name)
                                        .and_then(|def| def.fields.get(field_path))
                                    else {
                                        self.diags.push(
                                            Diag::error(
                                                name_span.clone(),
                                                format!(
                                                    "invalid assignment target `{name}.{field_path}`"
                                                ),
                                            )
                                            .with_file(def.source_file.clone())
                                            .with_help(format!(
                                                "`{ty_name}` has no field `{field_path}`"
                                            )),
                                        );
                                        return None;
                                    };

                                    if !local_decl_type_matches(
                                        &v,
                                        &field_def.ty_name,
                                        &self.enum_defs,
                                        &self.struct_defs,
                                        &[],
                                    ) {
                                        self.diags.push(
                                            Diag::error(
                                                name_span.clone(),
                                                format!(
                                                    "cannot assign {} to `{name}.{field_path}`",
                                                    v.kind()
                                                ),
                                            )
                                            .with_file(def.source_file.clone())
                                            .with_help(format!(
                                                "expected {} for field `{field_path}`",
                                                local_decl_expected_kind(
                                                    &field_def.ty_name,
                                                    &self.enum_defs,
                                                    &self.struct_defs,
                                                    &[]
                                                )
                                            )),
                                        );
                                        return None;
                                    }

                                    fields.insert(field_path.clone(), v);
                                    Value::Struct { ty_name, fields }
                                }
                                other => match apply_swizzle_assignment(other, field_path, v) {
                                    Ok(updated) => updated,
                                    Err(msg) => {
                                        self.diags.push(
                                            Diag::error(
                                                name_span.clone(),
                                                format!(
                                                    "invalid assignment target `{name}.{field_path}`"
                                                ),
                                            )
                                            .with_file(def.source_file.clone())
                                            .with_help(msg),
                                        );
                                        return None;
                                    }
                                },
                            };
                            let _ = self.assign(name, updated);
                        } else if !self.assign(name, v) {
                            self.diags.push(
                                Diag::error(
                                    name_span.clone(),
                                    format!("cannot assign to undeclared local `{name}`"),
                                )
                                .with_file(def.source_file.clone())
                                .with_label("unknown assignment target")
                                .with_help(
                                    "declare the variable first with `let name = ...` or `type name = ...`",
                                ),
                            );
                            return None;
                        }
                    }
                }
                Stmt::Expr(e) => {
                    let v = self.eval(e)?;
                    flow.fallthrough_value = Some((v, e.span.clone()));
                }
                Stmt::ReturnVoid { span } => {
                    self.diags.push(Diag::error(
                        span.clone(),
                        "value-returning functions require a return value",
                    ));
                    return None;
                }
                Stmt::Return { value, span } => {
                    let v = self.eval_scalar_expected(
                        value,
                        (!def.infer_return && matches!(def.ret, FnValueTy::Scalar))
                            .then_some(def.ret_kind),
                    )?;
                    flow.guaranteed_return = Some((v, span.clone()));
                    flow.guaranteed_return_span = Some(span.clone());
                }
                Stmt::If {
                    cond,
                    then_body,
                    else_body,
                    span,
                } => {
                    let cond_sx = self.as_scalar(cond)?;

                    let cond_sx = Self::cond_mask_from_scalar(cond_sx);
                    self.scopes.push(HashMap::new());
                    let then_result = self.capture_inline_writes(fn_name, def, then_body, in_loop);
                    self.scopes.pop();
                    let (mut then_flow, then_writes) = then_result?;

                    let (mut else_flow, else_writes) = if let Some(else_body) = else_body {
                        self.scopes.push(HashMap::new());
                        let result = self.capture_inline_writes(fn_name, def, else_body, in_loop);
                        self.scopes.pop();
                        result?
                    } else {
                        (ReturnPathAnalysis::default(), InlineWrites::new())
                    };
                    self.merge_inline_writes(
                        fn_name,
                        def,
                        cond_sx.clone(),
                        then_writes,
                        else_writes,
                        span,
                    )?;
                    let not_cond = Self::inline_mask_not(cond_sx.clone());
                    for (mask, value, return_span) in then_flow.conditional_returns {
                        flow.conditional_returns.push((
                            Self::inline_mask_and(cond_sx.clone(), mask),
                            value,
                            return_span,
                        ));
                    }
                    for (mask, value, return_span) in else_flow.conditional_returns {
                        flow.conditional_returns.push((
                            Self::inline_mask_and(not_cond.clone(), mask),
                            value,
                            return_span,
                        ));
                    }
                    let then_returns = then_flow.guaranteed_return.is_some();
                    let else_returns = else_flow.guaranteed_return.is_some();
                    match (
                        then_flow.guaranteed_return.take(),
                        else_flow.guaranteed_return.take(),
                    ) {
                        (Some((a, _)), Some((b, _))) => {
                            flow.guaranteed_return = Some((
                                self.merge_conditional_values(
                                    fn_name,
                                    &def.source_file,
                                    cond_sx.clone(),
                                    a,
                                    b,
                                    span,
                                )?,
                                span.clone(),
                            ));
                            flow.guaranteed_return_span = Some(span.clone());
                        }
                        (Some((value, return_span)), None) => {
                            flow.conditional_returns
                                .push((cond_sx.clone(), value, return_span));
                        }
                        (None, Some((value, return_span))) => {
                            flow.conditional_returns
                                .push((not_cond.clone(), value, return_span));
                        }
                        (None, None) => {}
                    }
                    flow.fallthrough_value =
                        match (then_flow.fallthrough_value, else_flow.fallthrough_value) {
                            (Some((a, _)), Some((b, _))) => Some((
                                self.merge_conditional_values(
                                    fn_name,
                                    &def.source_file,
                                    cond_sx.clone(),
                                    a,
                                    b,
                                    span,
                                )?,
                                span.clone(),
                            )),
                            (value @ Some(_), None) if else_returns => value,
                            (None, value @ Some(_)) if then_returns => value,
                            _ => None,
                        };
                    let then_break = if then_flow.break_span.is_some() {
                        Some(Sx::Lit(1.0))
                    } else {
                        then_flow.conditional_break
                    };
                    let else_break = if else_flow.break_span.is_some() {
                        Some(Sx::Lit(1.0))
                    } else {
                        else_flow.conditional_break
                    };
                    if then_flow.break_span.is_some() && else_flow.break_span.is_some() {
                        flow.break_span = Some(span.clone());
                    } else {
                        if let Some(mask) = then_break {
                            flow.conditional_break = Some(Self::inline_mask_and(cond_sx, mask));
                        }
                        if let Some(mask) = else_break {
                            flow.conditional_break = Some(Self::inline_mask_or(
                                flow.conditional_break,
                                Self::inline_mask_and(not_cond, mask),
                            ));
                        }
                    }
                }
                Stmt::Match {
                    value,
                    arms,
                    default_body,
                    default_span,
                    span,
                } => {
                    let lowered = self.lower_match_stmt_to_if_chain(
                        value,
                        arms,
                        default_body.as_ref(),
                        default_span.as_ref(),
                        &def.source_file,
                        span,
                    )?;
                    let lowered_flow = self.eval_fn_stmt_list(
                        fn_name,
                        def,
                        std::slice::from_ref(&lowered),
                        in_loop,
                    )?;

                    flow = lowered_flow;
                }
                Stmt::For {
                    name,
                    iterable,
                    body,
                    span,
                    index_name,
                    ..
                } => {
                    let values =
                        self.eval_compile_time_iter_values(iterable, "function for-loop")?;

                    let mut loop_active = Sx::Lit(1.0);
                    let mut loop_writes: BTreeSet<(usize, String)> = Default::default();
                    let mut pending = Vec::new();
                    let mut last_loop_value: Option<(Value, Span)> = None;
                    for (iter_idx, value) in values.into_iter().enumerate() {
                        for (index, name) in &loop_writes {
                            let index: usize = *index;
                            let value = self.scopes[index][name].clone();
                            if let Value::Scalar(sx) = &value
                                && let Some(value) = sx.try_eval_with_vars(&HashMap::new())
                            {
                                self.scopes[index]
                                    .insert(name.clone(), Value::Scalar(Sx::Lit(value)));
                                continue;
                            }
                            let value = Self::placeholder_for_call_arg(
                                name,
                                value,
                                &mut self.let_counter,
                                &mut pending,
                            );
                            self.scopes[index].insert(name.clone(), value);
                        }
                        self.scopes.push(HashMap::new());
                        self.bind(name.clone(), value);
                        if let Some((idx_name, _)) = index_name {
                            self.bind(idx_name.clone(), Value::Scalar(Sx::Lit(iter_idx as f32)));
                        }
                        let result = self.capture_inline_writes(fn_name, def, body, true);
                        self.scopes.pop();
                        let (iter_flow, mut writes) = result?;
                        writes.retain(|(index, _), _| *index < self.scopes.len());
                        loop_writes.extend(writes.keys().cloned());
                        self.merge_inline_writes(
                            fn_name,
                            def,
                            loop_active.clone(),
                            writes,
                            InlineWrites::new(),
                            span,
                        )?;
                        if iter_flow.guaranteed_return.is_some()
                            || !iter_flow.conditional_returns.is_empty()
                        {
                            self.diags.push(
                                Diag::error(
                                    span.clone(),
                                    format!(
                                        "function `{fn_name}` does not support `return` inside `for` bodies yet"
                                    ),
                                )
                                .with_file(def.source_file.clone())
                                .with_help(
                                    "move `return` outside the loop or express loop output through a trailing expression",
                                ),
                            );
                            return None;
                        }

                        if let Some(v) = iter_flow.fallthrough_value {
                            last_loop_value = Some(v);
                        }

                        if iter_flow.break_span.is_some() {
                            break;
                        }
                        if let Some(mask) = iter_flow.conditional_break {
                            loop_active =
                                Self::inline_mask_and(loop_active, Self::inline_mask_not(mask));
                            let Value::Scalar(shared) = Self::placeholder_for_call_arg(
                                "loop_active",
                                Value::Scalar(loop_active),
                                &mut self.let_counter,
                                &mut pending,
                            ) else {
                                unreachable!("scalar placeholder preserves the value type")
                            };
                            loop_active = shared;
                        }
                    }
                    // Close one binding chain after the loop, rather than
                    // embedding the full history separately in every guard
                    // and state update on every iteration.
                    for (index, name) in &loop_writes {
                        let value = self.scopes[*index][name].clone();
                        self.scopes[*index]
                            .insert(name.clone(), Self::apply_pending_lets(value, &pending));
                    }
                    if let Some((value, span)) = last_loop_value.take() {
                        last_loop_value = Some((Self::apply_pending_lets(value, &pending), span));
                    }

                    if let Some(v) = last_loop_value {
                        flow.fallthrough_value = Some(v);
                    }
                }
                Stmt::Block { body, .. } => {
                    self.scopes.push(HashMap::new());
                    let nested_flow = self.eval_fn_stmt_list(fn_name, def, body, in_loop);
                    self.scopes.pop();

                    flow = nested_flow?;
                }
                Stmt::Seq { body, .. } => {
                    for inner in body {
                        match inner {
                            Stmt::Let {
                                name,
                                value,
                                declared_ty_name,
                                declared_ty_span,
                                ..
                            } => {
                                let v = self.eval_scalar_expected(
                                    value,
                                    declared_ty_name
                                        .as_deref()
                                        .and_then(crate::typed_scalar::Kind::parse),
                                )?;
                                let type_param_names: Vec<&str> =
                                    def.type_params.iter().map(|(n, _)| n.as_str()).collect();
                                if let (Some(ty_name), Some(ty_span)) =
                                    (declared_ty_name.as_ref(), declared_ty_span.as_ref())
                                    && !local_decl_type_matches(
                                        &v,
                                        ty_name,
                                        &self.enum_defs,
                                        &self.struct_defs,
                                        &type_param_names,
                                    )
                                {
                                    self.diags.push(
                                        Diag::error(
                                            ty_span.clone(),
                                            format!(
                                                "typed local `{name}` expected {}, but got {}",
                                                local_decl_expected_kind(
                                                    ty_name,
                                                    &self.enum_defs,
                                                    &self.struct_defs,
                                                    &type_param_names
                                                ),
                                                v.kind()
                                            ),
                                        )
                                        .with_file(def.source_file.clone())
                                        .with_label("declared type does not match initializer")
                                        .with_help(
                                            "adjust the declared type or initializer expression",
                                        ),
                                    );
                                    return None;
                                }
                                self.bind(name.clone(), v);
                            }
                            Stmt::Break { span } => {
                                if !in_loop {
                                    self.diags.push(
                                        Diag::error(
                                            span.clone(),
                                            "`break` is only valid inside loop bodies",
                                        )
                                        .with_file(def.source_file.clone())
                                        .with_help("use `break` inside a `for` loop body"),
                                    );
                                    return None;
                                }
                                flow.break_span = Some(span.clone());
                                break;
                            }
                            _ => {
                                self.diags.push(
                                    Diag::error(
                                        fn_stmt_span_kind(inner).0,
                                        "internal error: parser produced unsupported statement in sequence",
                                    )
                                    .with_file(def.source_file.clone()),
                                );
                                return None;
                            }
                        }
                    }
                }
                Stmt::Break { span } => {
                    if !in_loop {
                        self.diags.push(
                            Diag::error(span.clone(), "`break` is only valid inside loop bodies")
                                .with_file(def.source_file.clone())
                                .with_help("use `break` inside a `for` loop body"),
                        );
                        return None;
                    }
                    flow.break_span = Some(span.clone());
                }
                Stmt::Param { .. }
                | Stmt::TextureBinding { .. }
                | Stmt::LetScatter { .. }
                | Stmt::SpaceDecl { .. }
                | Stmt::StyleDecl { .. }
                | Stmt::CanvasSpace { .. }
                | Stmt::InSpace { .. }
                | Stmt::InContext { .. }
                | Stmt::Compose { .. }
                | Stmt::ComposePiped { .. }
                | Stmt::SurfaceVertex { .. } => {
                    let (span, kind) = fn_stmt_span_kind(stmt);
                    self.diags.push(
                        Diag::error(
                            span,
                            format!(
                                "function `{fn_name}` does not support `{kind}` statements yet"
                            ),
                        )
                        .with_file(def.source_file.clone())
                        .with_help(
                            "v0 function semantics support scalar/vec2/helper logic with `if`, `let`, assignment, `return`, and expression statements",
                        ),
                    );
                    return None;
                }
                Stmt::LocalFnDecl(local_fn) => {
                    self.register_local_fn_decl(local_fn);
                }
            }
        }

        Some(flow)
    }

    pub(super) fn fn_defs(&self, name: &str) -> Option<Vec<FnDef>> {
        self.fn_defs.get(name).cloned()
    }

    pub(super) fn can_access_internal_functions(&self) -> bool {
        self.fn_source_stack
            .last()
            .is_some_and(|source| is_stdlib_source(source))
    }

    fn overload_param_label(param: &FnParamDef) -> String {
        match param.ty.family() {
            FnTypeFamily::Scalar => match param.scalar_specialization {
                ScalarSpecialization::Any => "scalar".to_string(),
                ScalarSpecialization::F32 => "f32".to_string(),
                ScalarSpecialization::I32 => "i32".to_string(),
                ScalarSpecialization::U32 => "u32".to_string(),
            },
            _ => param.ty.canonical_type_name().to_string(),
        }
    }

    fn overload_signature(def: &FnDef) -> String {
        let params = def
            .params
            .iter()
            .map(Self::overload_param_label)
            .collect::<Vec<_>>()
            .join(", ");
        format!("({params})")
    }

    fn scalar_arg_hint(expr: &SExpr) -> Option<(bool, bool, bool, bool, bool)> {
        match &expr.node {
            Expr::Num(v, Unit::None) => {
                if v.fract() == 0.0 {
                    let fits_i32 = *v >= f64::from(i32::MIN) && *v <= f64::from(i32::MAX);
                    Some((true, *v >= 0.0, false, false, fits_i32))
                } else {
                    Some((false, false, false, false, false))
                }
            }
            Expr::Unary(UnOp::Neg, inner) => match &inner.node {
                Expr::Num(v, Unit::None) if v.fract() == 0.0 => {
                    let value = -*v;
                    let fits_i32 = value >= f64::from(i32::MIN) && value <= f64::from(i32::MAX);
                    Some((true, false, true, false, fits_i32))
                }
                Expr::Num(_, Unit::None) => Some((false, false, true, false, false)),
                _ => None,
            },
            Expr::Call { name, .. } => match name.as_str() {
                "int" | "i32" => Some((true, false, true, false, true)),
                "u32" => Some((true, true, true, true, false)),
                "float" | "f32" | "f64" | "half" => Some((false, false, true, false, false)),
                _ => None,
            },
            _ => None,
        }
    }

    fn arg_shape_hint(&self, expr: &SExpr) -> Option<&'static str> {
        fn bound_value(checker: &Checker, expr: &SExpr) -> Option<Value> {
            match &expr.node {
                Expr::Var(name) => checker.lookup(name).or_else(|| {
                    let mut parts = name.split('.');
                    let mut value = checker.lookup(parts.next()?)?;
                    for field in parts {
                        let Value::Struct { fields, .. } = value else {
                            return None;
                        };
                        value = fields.get(field)?.clone();
                    }
                    Some(value)
                }),
                Expr::Member(base, field) => {
                    let Value::Struct { fields, .. } = bound_value(checker, base)? else {
                        return None;
                    };
                    fields.get(field).cloned()
                }
                _ => None,
            }
        }
        if let Some(value) = bound_value(self, expr) {
            match value {
                Value::Vec2(_) => return Some("vec2"),
                Value::Vec3(_) => return Some("vec3"),
                Value::Vec4(_) => return Some("vec4"),
                Value::Scalar(_) => return Some("scalar"),
                _ => {}
            }
        }
        match &expr.node {
            Expr::Num(_, Unit::None) => Some("scalar"),
            Expr::Unary(UnOp::Neg, value) => self.arg_shape_hint(value),
            Expr::Binary(BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div, a, b) => {
                let a = self.arg_shape_hint(a)?;
                let b = self.arg_shape_hint(b)?;
                if a == b || b == "scalar" {
                    Some(a)
                } else if a == "scalar" {
                    Some(b)
                } else {
                    None
                }
            }
            Expr::Color(_) => Some("color"),
            Expr::Vec2(_, _) => Some("vec2"),
            Expr::Vec3(_, _, _) => Some("vec3"),
            Expr::Vec4(_, _, _, _) => Some("vec4"),
            Expr::Var(name) => match name.as_str() {
                "coord" | "center" | "resolution" => Some("vec2"),
                "time" | "delta" => Some("scalar"),
                _ => None,
            },
            Expr::Member(base, field) => {
                let _ = base;
                match field.len() {
                    1 => Some("scalar"),
                    2 => Some("vec2"),
                    3 => Some("vec3"),
                    4 => Some("vec4"),
                    _ => None,
                }
            }
            Expr::Call { name, args, .. } => match name.as_str() {
                "i32" | "int" | "u32" | "float" | "f32" | "f64" | "half" => Some("scalar"),
                "vec2" => Some("vec2"),
                "vec3" => Some("vec3"),
                "vec4" => Some("vec4"),
                "mat2" => Some("mat2"),
                "mat3" => Some("mat3"),
                "mat4" => Some("mat4"),
                "rgb" | "rgba" | "rgb24" | "rgba32" => Some("color"),
                "cross" => match self.arg_shape_hint(&args.first()?.value)? {
                    "vec2" => Some("scalar"),
                    "vec3" => Some("vec3"),
                    _ => None,
                },
                "min" | "max" | "clamp" | "mix" | "pow" | "step" | "smoothstep" => {
                    args.iter().try_fold("scalar", |shape, arg| {
                        let next = self.arg_shape_hint(&arg.value)?;
                        if shape == next || next == "scalar" {
                            Some(shape)
                        } else if shape == "scalar" {
                            Some(next)
                        } else {
                            None
                        }
                    })
                }
                "normalize" | "abs" | "sign" | "reflect" | "refract" | "floor" | "ceil"
                | "fract" | "sqrt" | "inverse_sqrt" | "inverseSqrt" | "inversesqrt" | "rsqrt"
                | "sin" | "cos" | "tan" | "exp" | "log" => {
                    self.arg_shape_hint(&args.first()?.value)
                }
                _ => {
                    let definitions = self.fn_defs.get(name)?;
                    let scored: Vec<_> = definitions
                        .iter()
                        .filter_map(|def| {
                            self.overload_score_for_args(def, args)
                                .map(|score| (def, score))
                        })
                        .collect();
                    let best = scored.iter().map(|(_, score)| *score).max()?;
                    let mut results = scored
                        .iter()
                        .filter(|(_, score)| *score == best)
                        .map(|(def, _)| def.ret.canonical_type_name());
                    let first = results.next()?;
                    if !matches!(
                        first,
                        "scalar" | "vec2" | "vec3" | "vec4" | "mat2" | "mat3" | "mat4" | "color"
                    ) {
                        return None;
                    }
                    results.all(|ty| ty == first).then_some(first)
                }
            },
            _ => None,
        }
    }

    fn non_scalar_shape_score(param_ty: &FnValueTy, hint: Option<&'static str>) -> Option<i32> {
        let Some(shape) = hint else {
            return Some(0);
        };

        let matches = match shape {
            "scalar" => matches!(
                param_ty,
                FnValueTy::Scalar | FnValueTy::Enum(_) | FnValueTy::TypeVar(_)
            ),
            "vec2" => matches!(
                param_ty,
                FnValueTy::Vec2 | FnValueTy::CoordLike | FnValueTy::TypeVar(_)
            ),
            "vec3" => matches!(param_ty, FnValueTy::Vec3 | FnValueTy::TypeVar(_)),
            "vec4" => matches!(param_ty, FnValueTy::Vec4 | FnValueTy::TypeVar(_)),
            "mat2" => matches!(param_ty, FnValueTy::Mat2 | FnValueTy::TypeVar(_)),
            "mat3" => matches!(param_ty, FnValueTy::Mat3 | FnValueTy::TypeVar(_)),
            "mat4" => matches!(param_ty, FnValueTy::Mat4 | FnValueTy::TypeVar(_)),
            "color" => matches!(param_ty, FnValueTy::Color | FnValueTy::TypeVar(_)),
            _ => false,
        };

        if matches { Some(3) } else { None }
    }

    fn scalar_specialization_score(
        spec: ScalarSpecialization,
        hint: Option<(bool, bool, bool, bool, bool)>,
    ) -> Option<i32> {
        match spec {
            ScalarSpecialization::Any => Some(0),
            ScalarSpecialization::F32 => match hint {
                Some((true, _, _, _, _)) => Some(1),
                Some((false, _, _, _, _)) => Some(3),
                None => Some(1),
            },
            ScalarSpecialization::I32 => match hint {
                Some((true, _, explicit, explicit_unsigned, fits_i32)) => {
                    if !fits_i32 || (explicit && explicit_unsigned) {
                        None
                    } else {
                        Some(3)
                    }
                }
                Some((false, _, _, _, _)) => None,
                None => Some(0),
            },
            ScalarSpecialization::U32 => match hint {
                Some((true, non_negative, explicit, explicit_unsigned, _)) => {
                    if explicit && !explicit_unsigned {
                        return None;
                    }
                    if non_negative { Some(3) } else { None }
                }
                Some((false, _, _, _, _)) => None,
                None => Some(0),
            },
        }
    }

    fn overload_score_for_args(&self, def: &FnDef, args: &[Arg]) -> Option<i32> {
        let mut taken = vec![false; args.len()];
        let mut score = 0;
        for param in &def.params {
            let found_idx = args
                .iter()
                .enumerate()
                .find(|(i, a)| !taken[*i] && a.name.as_deref() == Some(param.name.as_str()))
                .map(|(i, _)| i)
                .or_else(|| {
                    if param.keyword_only {
                        None
                    } else {
                        args.iter()
                            .enumerate()
                            .find(|(i, a)| !taken[*i] && a.name.is_none())
                            .map(|(i, _)| i)
                    }
                });

            let idx = found_idx?;
            taken[idx] = true;

            if param.ty.family() == FnTypeFamily::Scalar {
                let known_kind = match &args[idx].value.node {
                    Expr::Var(name) => self
                        .lookup(name)
                        .and_then(|value| Self::value_element_kind(&value)),
                    Expr::Call { name, .. } => crate::typed_scalar::Kind::parse(name),
                    _ => None,
                };
                if let Some(kind) = known_kind {
                    if kind != param.scalar_kind {
                        return None;
                    }
                    score += 5;
                    continue;
                }
                score += Self::scalar_specialization_score(
                    param.scalar_specialization,
                    Self::scalar_arg_hint(&args[idx].value),
                )?;
            } else {
                score +=
                    Self::non_scalar_shape_score(&param.ty, self.arg_shape_hint(&args[idx].value))?;
            }
        }
        Some(score)
    }

    fn select_overload_for_call(
        &mut self,
        name: &str,
        name_span: &Span,
        const_arg_count: usize,
        args: &[Arg],
        defs: Vec<FnDef>,
    ) -> Option<FnDef> {
        let candidates: Vec<FnDef> = defs
            .into_iter()
            .filter(|d| d.params.len() == args.len() && d.const_params.len() == const_arg_count)
            .collect();
        if candidates.is_empty() {
            let expected_templates = self
                .fn_defs
                .get(name)
                .into_iter()
                .flat_map(|v| {
                    v.iter().filter_map(|d| {
                        if d.params.len() == args.len() && !d.const_params.is_empty() {
                            let parts = d
                                .const_params
                                .iter()
                                .map(|p| {
                                    format!(
                                        "{}: {}",
                                        p.name,
                                        match p.ty {
                                            ConstTemplateParamTy::U32 => "u32",
                                            ConstTemplateParamTy::I32 => "i32",
                                        }
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join(", ");
                            Some(format!("<{}>", parts))
                        } else {
                            None
                        }
                    })
                })
                .collect::<Vec<_>>();
            let mut supported: Vec<String> = self
                .fn_defs
                .get(name)
                .into_iter()
                .flat_map(|v| {
                    v.iter().map(|d| {
                        if d.const_params.is_empty() {
                            format!("{}", d.params.len())
                        } else {
                            format!("{} + <{} const>", d.params.len(), d.const_params.len())
                        }
                    })
                })
                .collect();
            supported.sort();
            supported.dedup();
            self.diags.push(
                Diag::error(
                    name_span.clone(),
                    format!(
                        "`{name}` has no overload accepting {} argument(s)",
                        args.len()
                    ),
                )
                .with_help(format!(
                    "supported overload arities: {}{}",
                    supported.join(", "),
                    if expected_templates.is_empty() {
                        String::new()
                    } else {
                        format!(
                            "; expected const template(s) for this arity: {}",
                            expected_templates.join(" | ")
                        )
                    }
                )),
            );
            return None;
        }
        if candidates.len() == 1 {
            return candidates.into_iter().next();
        }

        let mut scored: Vec<(FnDef, i32)> = candidates
            .into_iter()
            .filter_map(|def| {
                self.overload_score_for_args(&def, args)
                    .map(|score| (def, score))
            })
            .collect();

        if scored.is_empty() {
            let signatures = self
                .fn_defs
                .get(name)
                .into_iter()
                .flat_map(|defs| defs.iter())
                .filter(|d| d.params.len() == args.len())
                .map(Self::overload_signature)
                .collect::<Vec<_>>()
                .join(", ");
            self.diags.push(
                Diag::error(
                    name_span.clone(),
                    format!("`{name}` has no overload matching argument specializations"),
                )
                .with_help(format!(
                    "try explicit casts (`float(...)`, `i32(...)`, `u32(...)`); candidates: {signatures}"
                )),
            );
            return None;
        }

        scored.sort_by_key(|a| std::cmp::Reverse(a.1));
        let best_score = scored[0].1;
        let best = scored
            .iter()
            .filter(|(_, score)| *score == best_score)
            .map(|(def, _)| def.clone())
            .collect::<Vec<_>>();

        if best.len() > 1 {
            let signatures = best
                .iter()
                .map(Self::overload_signature)
                .collect::<Vec<_>>()
                .join(", ");
            self.diags.push(
                Diag::error(
                    name_span.clone(),
                    format!("`{name}` call is ambiguous across specialized overloads"),
                )
                .with_help(format!(
                    "disambiguate with explicit casts; matching overloads: {signatures}"
                )),
            );
            return None;
        }

        best.into_iter().next()
    }

    fn helper_ret_components(ty: &FnValueTy) -> Option<u8> {
        if matches!(ty.family(), FnTypeFamily::Scalar | FnTypeFamily::Enum) {
            return Some(1);
        }
        ty.vector_width()
    }

    fn helper_param_ty(ty: &FnValueTy) -> Option<hir::UserFnParamTy> {
        match ty {
            FnValueTy::Scalar => Some(hir::UserFnParamTy::Scalar),
            FnValueTy::Enum(_) => Some(hir::UserFnParamTy::Scalar),
            FnValueTy::Vec2 | FnValueTy::CoordLike => Some(hir::UserFnParamTy::Vec2),
            FnValueTy::Vec3 => Some(hir::UserFnParamTy::Vec3),
            FnValueTy::Vec4 => Some(hir::UserFnParamTy::Vec4),
            FnValueTy::Mat2 => Some(hir::UserFnParamTy::Mat2),
            FnValueTy::Mat3 => Some(hir::UserFnParamTy::Mat3),
            FnValueTy::Mat4 => Some(hir::UserFnParamTy::Mat4),
            FnValueTy::TypeVar(_) => None,
            _ => None,
        }
    }

    fn helper_flat_param_arity(ty: &FnValueTy) -> Option<usize> {
        if matches!(ty.family(), FnTypeFamily::Scalar | FnTypeFamily::Enum) {
            return Some(1);
        }
        if let Some(width) = ty.vector_width() {
            return Some(width as usize);
        }
        if let Some((cols, rows)) = ty.matrix_dims() {
            return Some(cols as usize * rows as usize);
        }
        None
    }

    fn helper_param_ty_name(ty: &FnValueTy) -> &'static str {
        ty.canonical_type_name()
    }

    fn helper_id(name: &str, def: &FnDef) -> String {
        let sig = def
            .params
            .iter()
            .map(|param| match param.ty.family() {
                FnTypeFamily::Scalar => match param.scalar_specialization {
                    ScalarSpecialization::Any => {
                        if param.scalar_kind == crate::typed_scalar::Kind::Bool {
                            "b".to_string()
                        } else {
                            "s".to_string()
                        }
                    }
                    ScalarSpecialization::F32 => "f".to_string(),
                    ScalarSpecialization::I32 => "i".to_string(),
                    ScalarSpecialization::U32 => "u".to_string(),
                },
                FnTypeFamily::Vector if param.scalar_kind != crate::typed_scalar::Kind::F32 => {
                    format!(
                        "v{}_{}",
                        param.ty.vector_width().expect("vector width"),
                        param.scalar_kind.name()
                    )
                }
                FnTypeFamily::Vector => match param.ty.vector_width() {
                    Some(2) => "v2".to_string(),
                    Some(3) => "v3".to_string(),
                    Some(4) => "v4".to_string(),
                    _ => "v".to_string(),
                },
                FnTypeFamily::Matrix => match param.ty.matrix_dims() {
                    Some((2, 2)) => "m2".to_string(),
                    Some((3, 3)) => "m3".to_string(),
                    Some((4, 4)) => "m4".to_string(),
                    _ => "m".to_string(),
                },
                FnTypeFamily::CoordLike => "coord".to_string(),
                FnTypeFamily::ShaderResource => param.ty.canonical_type_name().into(),
                FnTypeFamily::Texture => "tex".to_string(),
                FnTypeFamily::Color => "color".to_string(),
                FnTypeFamily::Shape => "shape".to_string(),
                FnTypeFamily::Layer => "layer".to_string(),
                FnTypeFamily::Struct => "struct".to_string(),
                FnTypeFamily::Array => "array".to_string(),
                FnTypeFamily::Enum => "enum".to_string(),
                FnTypeFamily::Callable => "fn".to_string(),
                FnTypeFamily::TypeVar => "t".to_string(),
            })
            .collect::<Vec<_>>()
            .join("_");

        let const_suffix = if def.const_bindings.is_empty() {
            String::new()
        } else {
            let encoded = def
                .const_bindings
                .iter()
                .map(|(param_name, value)| match value {
                    ConstTemplateValue::U32(v) => format!("{param_name}_u{v}"),
                    ConstTemplateValue::I32(v) => format!("{param_name}_i{v}"),
                })
                .collect::<Vec<_>>()
                .join("_");
            format!("_ct_{encoded}")
        };

        format!("__{name}_{}_{}{}", def.params.len(), sig, const_suffix)
    }

    fn register_pending_user_helper(&mut self, name: &str, def: &FnDef) -> Option<String> {
        Self::helper_ret_components(&def.ret)?;
        // Functions with callable parameters cannot be lowered to WGSL helpers — force inline path.
        if def
            .params
            .iter()
            .any(|p| matches!(p.ty, FnValueTy::Callable { .. }))
        {
            return None;
        }
        let helper_id = Self::helper_id(name, def);
        if !self.hir.user_helpers.contains_key(&helper_id)
            && !self.pending_user_helpers.contains_key(&helper_id)
        {
            tracing::info!(
                helper_id = %helper_id,
                helper_name = %name,
                params = def.params.len(),
                "checker helper queued"
            );
            self.pending_user_helpers
                .insert(helper_id.clone(), (name.to_string(), def.clone()));
        }
        Some(helper_id)
    }

    pub(super) fn materialize_pending_user_helpers(&mut self) {
        if self.pending_user_helpers.is_empty() {
            return;
        }

        let helper_timeout_ms = std::env::var("FRESCO_CHECK_HELPER_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(2000);
        let max_passes = std::env::var("FRESCO_CHECK_HELPER_MATERIALIZE_MAX_PASSES")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(512);

        let prior_timeout_reported = self.expr_timeout_reported;
        let prior_timeout = self.expr_timeout;
        let prior_started_at = self.check_started_at;
        self.expr_timeout_reported = false;
        let mut failed_helpers = HashSet::new();
        let mut pass = 0usize;

        tracing::info!(
            pending = self.pending_user_helpers.len(),
            max_passes = max_passes,
            "checker helper materialization start"
        );

        while !self.pending_user_helpers.is_empty() {
            pass = pass.saturating_add(1);
            if pass > max_passes {
                let preview = self
                    .pending_user_helpers
                    .keys()
                    .take(8)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                tracing::error!(
                    pass = pass,
                    pending = self.pending_user_helpers.len(),
                    failed = failed_helpers.len(),
                    preview = %preview,
                    "checker helper materialization exceeded max passes"
                );

                if let Some((_, (name, def))) = self.pending_user_helpers.iter().next() {
                    self.diags.push(
                        Diag::error(
                            def.span.clone(),
                            format!(
                                "helper materialization exceeded {max_passes} passes (possible cycle while lowering `{name}`)"
                            ),
                        )
                        .with_help(format!(
                            "set FRESCO_CHECK_HELPER_MATERIALIZE_MAX_PASSES higher for debugging; pending helpers sample: {preview}"
                        )),
                    );
                }
                self.pending_user_helpers.clear();
                break;
            }

            let pending = std::mem::take(&mut self.pending_user_helpers);
            tracing::info!(
                pass = pass,
                pending = pending.len(),
                failed = failed_helpers.len(),
                emitted = self.hir.user_helpers.len(),
                "checker helper materialization pass"
            );

            for (helper_id, (name, def)) in pending {
                if failed_helpers.contains(&helper_id) {
                    continue;
                }

                let helper_t0 = Instant::now();
                tracing::info!(
                    pass = pass,
                    helper_id = %helper_id,
                    helper_name = %name,
                    "checker helper materialization helper start"
                );

                self.expr_timeout = Some(Duration::from_millis(helper_timeout_ms));
                self.check_started_at = Instant::now();
                self.expr_timeout_reported = false;

                if self.ensure_user_helper(&name, &def).is_none() {
                    failed_helpers.insert(helper_id);
                    self.diags.push(
                        Diag::error(
                            def.span.clone(),
                            format!(
                                "failed to materialize user helper `{name}` for runtime lowering"
                            ),
                        )
                        .with_help(
                            format!(
                                "helper lowering currently supports scalar/enum/vec2/vec3/vec4/mat2/mat3/mat4 signatures and bodies that resolve to those value kinds; helper checker budget is {}ms (set FRESCO_CHECK_HELPER_TIMEOUT_MS to tune)",
                                helper_timeout_ms
                            ),
                        ),
                    );
                    tracing::info!(
                        pass = pass,
                        helper_name = %name,
                        ms = helper_t0.elapsed().as_secs_f64() * 1000.0,
                        "checker helper materialization helper failed"
                    );
                } else {
                    tracing::info!(
                        pass = pass,
                        helper_name = %name,
                        ms = helper_t0.elapsed().as_secs_f64() * 1000.0,
                        "checker helper materialization helper done"
                    );
                }
            }
        }

        tracing::info!(
            passes = pass,
            emitted = self.hir.user_helpers.len(),
            failed = failed_helpers.len(),
            "checker helper materialization done"
        );

        self.compute_user_helper_sample_point_invariance();

        self.expr_timeout = prior_timeout;
        self.check_started_at = prior_started_at;
        self.expr_timeout_reported = prior_timeout_reported;
    }

    fn flatten_param_value(&self, param: &FnParamDef, value: &Value) -> Option<Vec<Sx>> {
        match (&param.ty, value) {
            (FnValueTy::Scalar, Value::Scalar(s))
            | (FnValueTy::Scalar, Value::Distance(s))
            | (FnValueTy::Scalar, Value::Coverage(s))
            | (FnValueTy::Scalar, Value::Mask(s)) => Some(vec![s.clone()]),
            (FnValueTy::Enum(_), Value::Scalar(s)) => Some(vec![s.clone()]),
            (FnValueTy::Vec2, Value::Vec2((x, y)))
            | (FnValueTy::CoordLike, Value::Vec2((x, y))) => Some(vec![x.clone(), y.clone()]),
            (FnValueTy::Vec3, Value::Vec3((x, y, z))) => {
                Some(vec![x.clone(), y.clone(), z.clone()])
            }
            (FnValueTy::Vec4, Value::Vec4((x, y, z, w))) => {
                Some(vec![x.clone(), y.clone(), z.clone(), w.clone()])
            }
            (FnValueTy::Mat2, Value::Mat2(((c0x, c0y), (c1x, c1y)))) => {
                Some(vec![c0x.clone(), c0y.clone(), c1x.clone(), c1y.clone()])
            }
            (FnValueTy::Mat3, Value::Mat3(m)) => Some(vec![
                m.0.0.clone(),
                m.0.1.clone(),
                m.0.2.clone(),
                m.1.0.clone(),
                m.1.1.clone(),
                m.1.2.clone(),
                m.2.0.clone(),
                m.2.1.clone(),
                m.2.2.clone(),
            ]),
            (FnValueTy::Mat4, Value::Mat4(m)) => Some(vec![
                m.0.0.clone(),
                m.0.1.clone(),
                m.0.2.clone(),
                m.0.3.clone(),
                m.1.0.clone(),
                m.1.1.clone(),
                m.1.2.clone(),
                m.1.3.clone(),
                m.2.0.clone(),
                m.2.1.clone(),
                m.2.2.clone(),
                m.2.3.clone(),
                m.3.0.clone(),
                m.3.1.clone(),
                m.3.2.clone(),
                m.3.3.clone(),
            ]),
            (FnValueTy::TypeVar(_), _) => None,
            _ => None,
        }
    }

    fn helper_param_placeholders(&self, param: &FnParamDef) -> Option<(Value, Vec<String>)> {
        match &param.ty {
            FnValueTy::Scalar | FnValueTy::Enum(_) => {
                let key = format!("arg__{}", param.name);
                Some((
                    Value::Scalar(crate::typed_scalar::Scalar::input(
                        Sx::Param(key.clone()),
                        param.scalar_kind,
                    )),
                    vec![key],
                ))
            }
            FnValueTy::Vec2 | FnValueTy::CoordLike => {
                let x = format!("arg__{}__x", param.name);
                let y = format!("arg__{}__y", param.name);
                Some((
                    Value::Vec2((
                        crate::typed_scalar::Scalar::input(Sx::Param(x.clone()), param.scalar_kind),
                        crate::typed_scalar::Scalar::input(Sx::Param(y.clone()), param.scalar_kind),
                    )),
                    vec![x, y],
                ))
            }
            FnValueTy::Vec3 => {
                let x = format!("arg__{}__x", param.name);
                let y = format!("arg__{}__y", param.name);
                let z = format!("arg__{}__z", param.name);
                Some((
                    Value::Vec3((
                        crate::typed_scalar::Scalar::input(Sx::Param(x.clone()), param.scalar_kind),
                        crate::typed_scalar::Scalar::input(Sx::Param(y.clone()), param.scalar_kind),
                        crate::typed_scalar::Scalar::input(Sx::Param(z.clone()), param.scalar_kind),
                    )),
                    vec![x, y, z],
                ))
            }
            FnValueTy::Vec4 => {
                let x = format!("arg__{}__x", param.name);
                let y = format!("arg__{}__y", param.name);
                let z = format!("arg__{}__z", param.name);
                let w = format!("arg__{}__w", param.name);
                Some((
                    Value::Vec4((
                        crate::typed_scalar::Scalar::input(Sx::Param(x.clone()), param.scalar_kind),
                        crate::typed_scalar::Scalar::input(Sx::Param(y.clone()), param.scalar_kind),
                        crate::typed_scalar::Scalar::input(Sx::Param(z.clone()), param.scalar_kind),
                        crate::typed_scalar::Scalar::input(Sx::Param(w.clone()), param.scalar_kind),
                    )),
                    vec![x, y, z, w],
                ))
            }
            FnValueTy::Mat2 => {
                let c0r0 = format!("arg__{}__c0r0", param.name);
                let c0r1 = format!("arg__{}__c0r1", param.name);
                let c1r0 = format!("arg__{}__c1r0", param.name);
                let c1r1 = format!("arg__{}__c1r1", param.name);
                Some((
                    Value::Mat2((
                        (Sx::Param(c0r0.clone()), Sx::Param(c0r1.clone())),
                        (Sx::Param(c1r0.clone()), Sx::Param(c1r1.clone())),
                    )),
                    vec![c0r0, c0r1, c1r0, c1r1],
                ))
            }
            FnValueTy::Mat3 => {
                let c0r0 = format!("arg__{}__c0r0", param.name);
                let c0r1 = format!("arg__{}__c0r1", param.name);
                let c0r2 = format!("arg__{}__c0r2", param.name);
                let c1r0 = format!("arg__{}__c1r0", param.name);
                let c1r1 = format!("arg__{}__c1r1", param.name);
                let c1r2 = format!("arg__{}__c1r2", param.name);
                let c2r0 = format!("arg__{}__c2r0", param.name);
                let c2r1 = format!("arg__{}__c2r1", param.name);
                let c2r2 = format!("arg__{}__c2r2", param.name);
                Some((
                    Value::Mat3(Box::new((
                        (
                            Sx::Param(c0r0.clone()),
                            Sx::Param(c0r1.clone()),
                            Sx::Param(c0r2.clone()),
                        ),
                        (
                            Sx::Param(c1r0.clone()),
                            Sx::Param(c1r1.clone()),
                            Sx::Param(c1r2.clone()),
                        ),
                        (
                            Sx::Param(c2r0.clone()),
                            Sx::Param(c2r1.clone()),
                            Sx::Param(c2r2.clone()),
                        ),
                    ))),
                    vec![c0r0, c0r1, c0r2, c1r0, c1r1, c1r2, c2r0, c2r1, c2r2],
                ))
            }
            FnValueTy::Mat4 => {
                let c0r0 = format!("arg__{}__c0r0", param.name);
                let c0r1 = format!("arg__{}__c0r1", param.name);
                let c0r2 = format!("arg__{}__c0r2", param.name);
                let c0r3 = format!("arg__{}__c0r3", param.name);
                let c1r0 = format!("arg__{}__c1r0", param.name);
                let c1r1 = format!("arg__{}__c1r1", param.name);
                let c1r2 = format!("arg__{}__c1r2", param.name);
                let c1r3 = format!("arg__{}__c1r3", param.name);
                let c2r0 = format!("arg__{}__c2r0", param.name);
                let c2r1 = format!("arg__{}__c2r1", param.name);
                let c2r2 = format!("arg__{}__c2r2", param.name);
                let c2r3 = format!("arg__{}__c2r3", param.name);
                let c3r0 = format!("arg__{}__c3r0", param.name);
                let c3r1 = format!("arg__{}__c3r1", param.name);
                let c3r2 = format!("arg__{}__c3r2", param.name);
                let c3r3 = format!("arg__{}__c3r3", param.name);
                Some((
                    Value::Mat4(Box::new((
                        (
                            Sx::Param(c0r0.clone()),
                            Sx::Param(c0r1.clone()),
                            Sx::Param(c0r2.clone()),
                            Sx::Param(c0r3.clone()),
                        ),
                        (
                            Sx::Param(c1r0.clone()),
                            Sx::Param(c1r1.clone()),
                            Sx::Param(c1r2.clone()),
                            Sx::Param(c1r3.clone()),
                        ),
                        (
                            Sx::Param(c2r0.clone()),
                            Sx::Param(c2r1.clone()),
                            Sx::Param(c2r2.clone()),
                            Sx::Param(c2r3.clone()),
                        ),
                        (
                            Sx::Param(c3r0.clone()),
                            Sx::Param(c3r1.clone()),
                            Sx::Param(c3r2.clone()),
                            Sx::Param(c3r3.clone()),
                        ),
                    ))),
                    vec![
                        c0r0, c0r1, c0r2, c0r3, c1r0, c1r1, c1r2, c1r3, c2r0, c2r1, c2r2, c2r3,
                        c3r0, c3r1, c3r2, c3r3,
                    ],
                ))
            }
            FnValueTy::TypeVar(_) => None,
            _ => None,
        }
    }

    fn validate_registered_fn_overload(&mut self, name: &str, def: &FnDef) -> bool {
        if !def.const_params.is_empty() {
            // Const-template functions are validated when instantiated at call-sites.
            return true;
        }

        let mut bound_values: Vec<(String, Value)> = Vec::with_capacity(def.params.len());
        for param in &def.params {
            let Some((value, _)) = self.helper_param_placeholders(param) else {
                // We cannot synthesize placeholder values for all parameter kinds yet.
                // Skip eager validation for this overload and let call-site checking handle it.
                return true;
            };
            bound_values.push((param.name.clone(), value));
        }

        self.fn_call_stack.push(def.declaration_identity(name));
        self.scopes.push(HashMap::new());
        for (const_name, const_value) in &def.const_bindings {
            let bound = match const_value {
                ConstTemplateValue::U32(v) => Value::Scalar(
                    crate::typed_scalar::Scalar::from_literal(naga::Literal::U32(*v)),
                ),
                ConstTemplateValue::I32(v) => Value::Scalar(
                    crate::typed_scalar::Scalar::from_literal(naga::Literal::I32(*v)),
                ),
            };
            self.bind(const_name.clone(), bound);
        }
        for (param_name, value) in bound_values {
            self.bind(param_name, value);
        }
        let diag_start = self.diags.len();
        let out = self.eval_fn_body(name, def);
        for d in self.diags.iter_mut().skip(diag_start) {
            if d.file.is_none() {
                d.file = Some(def.source_file.clone());
            }
        }
        self.scopes.pop();
        self.fn_call_stack.pop();

        out.is_some()
    }

    pub(super) fn validate_registered_function_overloads(&mut self) {
        self.validate_function_overloads_matching(|_, _| true);
    }

    fn validate_function_overloads_matching(&mut self, include: impl Fn(&str, &FnDef) -> bool) {
        // Eager checking must diagnose unused functions without making their
        // callees reachable from the actual entry point.
        let saved_helpers = self.hir.user_helpers.clone();
        let saved_pending_helpers = self.pending_user_helpers.clone();
        let work = self
            .fn_defs
            .iter()
            .flat_map(|(name, defs)| {
                defs.iter()
                    .filter(|def| !def.is_builtin && include(name, def))
                    .map(|def| (name.clone(), def.clone()))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        for (name, def) in work {
            // Once the expression watchdog trips once, `timeout_exceeded()`
            // unconditionally returns true for every remaining expression —
            // continuing this eager, best-effort pass would just produce a
            // wall of unrelated "function X returns error" diagnostics for
            // otherwise-fine functions that happen to be validated after the
            // one that was actually slow. Stop here instead of cascading.
            if self.expr_timeout_reported {
                break;
            }
            let _ = self.validate_registered_fn_overload(&name, &def);
        }

        self.hir.user_helpers = saved_helpers;
        self.pending_user_helpers = saved_pending_helpers;

        // Give whatever comes after this eager pass (the real canvas/surface
        // body evaluation) a fresh timeout budget instead of inheriting a
        // permanently-tripped watchdog from validating unrelated functions.
        self.expr_timeout_reported = false;
        self.check_started_at = Instant::now();
    }

    fn helper_slots(name: &str, components: u8) -> Vec<String> {
        match components {
            1 => vec![format!("loc__{name}")],
            2 => vec![format!("loc__{name}__x"), format!("loc__{name}__y")],
            3 => vec![
                format!("loc__{name}__x"),
                format!("loc__{name}__y"),
                format!("loc__{name}__z"),
            ],
            4 => vec![
                format!("loc__{name}__x"),
                format!("loc__{name}__y"),
                format!("loc__{name}__z"),
                format!("loc__{name}__w"),
            ],
            _ => Vec::new(),
        }
    }

    fn helper_value_components(value: &Value) -> Option<u8> {
        match value {
            Value::Scalar(_) | Value::Distance(_) | Value::Coverage(_) | Value::Mask(_) => Some(1),
            Value::Vec2(_) => Some(2),
            Value::Vec3(_) => Some(3),
            Value::Vec4(_) => Some(4),
            _ => None,
        }
    }

    fn helper_placeholder_value(slots: &[String]) -> Option<Value> {
        match slots {
            [x] => Some(Value::Scalar(Sx::Param(x.clone()))),
            [x, y] => Some(Value::Vec2((Sx::Param(x.clone()), Sx::Param(y.clone())))),
            [x, y, z] => Some(Value::Vec3((
                Sx::Param(x.clone()),
                Sx::Param(y.clone()),
                Sx::Param(z.clone()),
            ))),
            [x, y, z, w] => Some(Value::Vec4((
                Sx::Param(x.clone()),
                Sx::Param(y.clone()),
                Sx::Param(z.clone()),
                Sx::Param(w.clone()),
            ))),
            _ => None,
        }
    }

    fn helper_ir_expr(value: Value) -> Option<hir::UserFnExpr> {
        match value {
            Value::Scalar(s) | Value::Distance(s) | Value::Coverage(s) | Value::Mask(s) => {
                Some(hir::UserFnExpr::Value(hir::UserFnValue::Scalar(s)))
            }
            Value::Vec2((x, y)) => {
                if let (Sx::Param(a), Sx::Param(b)) = (&x, &y) {
                    return Some(hir::UserFnExpr::SlotSwizzle2([a.clone(), b.clone()]));
                }
                match (x, y) {
                    (
                        Sx::NormalizeComponent {
                            v: hir::SxVec::V2(v),
                            index: 0,
                        },
                        Sx::NormalizeComponent { index: 1, .. },
                    ) => Some(hir::UserFnExpr::Normalize2(*v)),
                    (
                        Sx::MinComponent {
                            a: hir::SxVec::V2(a),
                            b: hir::SxVec::V2(b),
                            index: 0,
                        },
                        Sx::MinComponent { index: 1, .. },
                    ) => Some(hir::UserFnExpr::Min2 { a: *a, b: *b }),
                    (
                        Sx::MaxComponent {
                            a: hir::SxVec::V2(a),
                            b: hir::SxVec::V2(b),
                            index: 0,
                        },
                        Sx::MaxComponent { index: 1, .. },
                    ) => Some(hir::UserFnExpr::Max2 { a: *a, b: *b }),
                    (
                        Sx::ClampVecComponent {
                            x: hir::SxVec::V2(x),
                            lo: hir::SxVec::V2(lo),
                            hi: hir::SxVec::V2(hi),
                            index: 0,
                        },
                        Sx::ClampVecComponent { index: 1, .. },
                    ) => Some(hir::UserFnExpr::Clamp2 {
                        x: *x,
                        lo: *lo,
                        hi: *hi,
                    }),
                    (x, y) => Some(hir::UserFnExpr::Value(hir::UserFnValue::Vec2((x, y)))),
                }
            }
            Value::Vec3((x, y, z)) => {
                if let (Sx::Param(a), Sx::Param(b), Sx::Param(c)) = (&x, &y, &z) {
                    return Some(hir::UserFnExpr::SlotSwizzle3([
                        a.clone(),
                        b.clone(),
                        c.clone(),
                    ]));
                }
                match (x, y, z) {
                    (
                        Sx::NormalizeComponent {
                            v: hir::SxVec::V3(v),
                            index: 0,
                        },
                        Sx::NormalizeComponent { index: 1, .. },
                        Sx::NormalizeComponent { index: 2, .. },
                    ) => Some(hir::UserFnExpr::Normalize3(*v)),
                    (
                        Sx::MinComponent {
                            a: hir::SxVec::V3(a),
                            b: hir::SxVec::V3(b),
                            index: 0,
                        },
                        Sx::MinComponent { index: 1, .. },
                        Sx::MinComponent { index: 2, .. },
                    ) => Some(hir::UserFnExpr::Min3 { a: *a, b: *b }),
                    (
                        Sx::MaxComponent {
                            a: hir::SxVec::V3(a),
                            b: hir::SxVec::V3(b),
                            index: 0,
                        },
                        Sx::MaxComponent { index: 1, .. },
                        Sx::MaxComponent { index: 2, .. },
                    ) => Some(hir::UserFnExpr::Max3 { a: *a, b: *b }),
                    (
                        Sx::ClampVecComponent {
                            x: hir::SxVec::V3(x),
                            lo: hir::SxVec::V3(lo),
                            hi: hir::SxVec::V3(hi),
                            index: 0,
                        },
                        Sx::ClampVecComponent { index: 1, .. },
                        Sx::ClampVecComponent { index: 2, .. },
                    ) => Some(hir::UserFnExpr::Clamp3 {
                        x: *x,
                        lo: *lo,
                        hi: *hi,
                    }),
                    (x, y, z) => Some(hir::UserFnExpr::Value(hir::UserFnValue::Vec3((x, y, z)))),
                }
            }
            Value::Vec4((x, y, z, w)) => {
                if let (Sx::Param(a), Sx::Param(b), Sx::Param(c), Sx::Param(d)) = (&x, &y, &z, &w) {
                    return Some(hir::UserFnExpr::SlotSwizzle4([
                        a.clone(),
                        b.clone(),
                        c.clone(),
                        d.clone(),
                    ]));
                }
                match (x, y, z, w) {
                    (
                        Sx::NormalizeComponent {
                            v: hir::SxVec::V4(v),
                            index: 0,
                        },
                        Sx::NormalizeComponent { index: 1, .. },
                        Sx::NormalizeComponent { index: 2, .. },
                        Sx::NormalizeComponent { index: 3, .. },
                    ) => Some(hir::UserFnExpr::Normalize4(*v)),
                    (
                        Sx::MinComponent {
                            a: hir::SxVec::V4(a),
                            b: hir::SxVec::V4(b),
                            index: 0,
                        },
                        Sx::MinComponent { index: 1, .. },
                        Sx::MinComponent { index: 2, .. },
                        Sx::MinComponent { index: 3, .. },
                    ) => Some(hir::UserFnExpr::Min4 { a: *a, b: *b }),
                    (
                        Sx::MaxComponent {
                            a: hir::SxVec::V4(a),
                            b: hir::SxVec::V4(b),
                            index: 0,
                        },
                        Sx::MaxComponent { index: 1, .. },
                        Sx::MaxComponent { index: 2, .. },
                        Sx::MaxComponent { index: 3, .. },
                    ) => Some(hir::UserFnExpr::Max4 { a: *a, b: *b }),
                    (
                        Sx::ClampVecComponent {
                            x: hir::SxVec::V4(x),
                            lo: hir::SxVec::V4(lo),
                            hi: hir::SxVec::V4(hi),
                            index: 0,
                        },
                        Sx::ClampVecComponent { index: 1, .. },
                        Sx::ClampVecComponent { index: 2, .. },
                        Sx::ClampVecComponent { index: 3, .. },
                    ) => Some(hir::UserFnExpr::Clamp4 {
                        x: *x,
                        lo: *lo,
                        hi: *hi,
                    }),
                    (x, y, z, w) => {
                        Some(hir::UserFnExpr::Value(hir::UserFnValue::Vec4((x, y, z, w))))
                    }
                }
            }
            _ => None,
        }
    }

    fn infer_enum_for_unqualified_match_variant(&self, variant_name: &str) -> Vec<String> {
        self.enum_defs
            .iter()
            .filter_map(|(enum_name, def)| {
                def.variants
                    .contains_key(variant_name)
                    .then_some(enum_name.clone())
            })
            .collect()
    }

    fn lower_match_stmt_to_if_chain(
        &mut self,
        value: &SExpr,
        arms: &[MatchArm],
        default_body: Option<&Vec<Stmt>>,
        _default_span: Option<&Span>,
        source_file: &str,
        match_span: &Span,
    ) -> Option<Stmt> {
        if arms.is_empty() {
            return Some(Stmt::Block {
                body: default_body.cloned().unwrap_or_default(),
                span: match_span.clone(),
            });
        }

        struct ResolvedArm {
            then_body: Vec<Stmt>,
            cond_rhs: SExpr,
        }

        let mut resolved = Vec::with_capacity(arms.len());
        let mut active_enum: Option<String> = None;
        let mut seen_variants = HashSet::new();

        for arm in arms {
            let enum_name = if let Some(explicit_enum) = &arm.enum_name {
                if !self.enum_defs.contains_key(explicit_enum) {
                    self.diags.push(
                        Diag::error(
                            arm.enum_name_span
                                .clone()
                                .unwrap_or_else(|| arm.span.clone()),
                            format!("unknown enum `{explicit_enum}` in match arm"),
                        )
                        .with_file(source_file.to_string())
                        .with_help("use a declared enum name before `.` in match labels"),
                    );
                    return None;
                }
                explicit_enum.clone()
            } else if let Some(current) = &active_enum {
                current.clone()
            } else {
                let owners = self.infer_enum_for_unqualified_match_variant(&arm.variant_name);
                if owners.is_empty() {
                    self.diags.push(
                        Diag::error(
                            arm.variant_span.clone(),
                            format!("unknown enum variant `{}` in match arm", arm.variant_name),
                        )
                        .with_file(source_file.to_string())
                        .with_help(
                            "qualify the arm as `<Enum>.<variant>` or use a declared variant name",
                        ),
                    );
                    return None;
                }
                if owners.len() > 1 {
                    self.diags.push(
                        Diag::error(
                            arm.variant_span.clone(),
                            format!("ambiguous enum variant `{}` in match arm", arm.variant_name),
                        )
                        .with_file(source_file.to_string())
                        .with_help(
                            owners
                                .iter()
                                .map(|owner| format!("`{owner}.{}'", arm.variant_name))
                                .collect::<Vec<_>>()
                                .join(" or "),
                        ),
                    );
                    return None;
                }
                owners[0].clone()
            };

            if let Some(current) = &active_enum {
                if current != &enum_name {
                    self.diags.push(
                        Diag::error(
                            arm.span.clone(),
                            format!(
                                "all match arms must target the same enum; found `{}` and `{}`",
                                current, enum_name
                            ),
                        )
                        .with_file(source_file.to_string())
                        .with_help("qualify labels consistently and keep all arms on one enum"),
                    );
                    return None;
                }
            } else {
                active_enum = Some(enum_name.clone());
            }

            let enum_def = self.enum_defs.get(&enum_name)?;

            if !enum_def.variants.contains_key(&arm.variant_name) {
                self.diags.push(
                    Diag::error(
                        arm.variant_span.clone(),
                        format!(
                            "unknown variant `{}` on enum `{}`",
                            arm.variant_name, enum_name
                        ),
                    )
                    .with_file(source_file.to_string())
                    .with_help("use a declared variant for the selected enum"),
                );
                return None;
            }

            if !seen_variants.insert(arm.variant_name.clone()) {
                self.diags.push(
                    Diag::error(
                        arm.variant_span.clone(),
                        format!("duplicate match arm for variant `{}`", arm.variant_name),
                    )
                    .with_file(source_file.to_string()),
                );
                return None;
            }

            let cond_rhs = Spanned {
                node: Expr::Var(format!("{}.{}", enum_name, arm.variant_name)),
                span: arm.variant_span.clone(),
            };

            // Validate that the rhs resolves as a scalar enum constant.
            let _ = self.as_scalar(&cond_rhs)?;

            resolved.push(ResolvedArm {
                then_body: arm.body.clone(),
                cond_rhs,
            });
        }

        if default_body.is_none()
            && let Some(enum_name) = &active_enum
        {
            let enum_def = self.enum_defs.get(enum_name)?;
            let mut missing = enum_def
                .variants
                .keys()
                .filter(|variant| !seen_variants.contains(*variant))
                .cloned()
                .collect::<Vec<_>>();
            missing.sort();
            if !missing.is_empty() {
                self.diags.push(
                    Diag::error(
                        match_span.clone(),
                        format!(
                            "non-exhaustive enum match on `{enum_name}`; missing variant(s): {}",
                            missing.join(", ")
                        ),
                    )
                    .with_file(source_file.to_string())
                    .with_help("add the missing variants or provide a `default:` arm"),
                );
                return None;
            }
        }

        let mut else_body = if let Some(body) = default_body {
            body.clone()
        } else {
            resolved
                .pop()
                .expect("validated exhaustive enum match has an arm")
                .then_body
        };
        for arm in resolved.into_iter().rev() {
            let cond = Spanned {
                node: Expr::Binary(BinOp::Eq, Box::new(value.clone()), Box::new(arm.cond_rhs)),
                span: match_span.clone(),
            };

            let next_else = if else_body.is_empty() {
                None
            } else {
                Some(else_body)
            };

            else_body = vec![Stmt::If {
                cond,
                then_body: arm.then_body,
                else_body: next_else,
                span: match_span.clone(),
            }];
        }

        Some(Stmt::Block {
            body: else_body,
            span: match_span.clone(),
        })
    }

    fn emit_user_helper_stmt_list(
        &mut self,
        helper_id: &str,
        fn_name: &str,
        def: &FnDef,
        stmts: &[Stmt],
        in_loop: bool,
    ) -> Option<Vec<hir::UserFnStmt>> {
        let mut out = Vec::new();

        for stmt in stmts {
            match stmt {
                Stmt::Store { span, .. } => {
                    self.diags.push(Diag::error(
                        span.clone(),
                        "indexed writes require an explicit GPU stage program",
                    ));
                    return None;
                }
                Stmt::Let {
                    name,
                    value,
                    declared_ty_name,
                    declared_ty_span,
                    ..
                } => {
                    let init = self.eval_scalar_expected(
                        value,
                        declared_ty_name
                            .as_deref()
                            .and_then(crate::typed_scalar::Kind::parse),
                    )?;
                    let type_param_names: Vec<&str> =
                        def.type_params.iter().map(|(n, _)| n.as_str()).collect();
                    if let (Some(ty_name), Some(ty_span)) =
                        (declared_ty_name.as_ref(), declared_ty_span.as_ref())
                        && !local_decl_type_matches(
                            &init,
                            ty_name,
                            &self.enum_defs,
                            &self.struct_defs,
                            &type_param_names,
                        )
                    {
                        self.diags.push(
                            Diag::error(
                                ty_span.clone(),
                                format!(
                                    "typed local `{name}` expected {}, but got {}",
                                    local_decl_expected_kind(
                                        ty_name,
                                        &self.enum_defs,
                                        &self.struct_defs,
                                        &type_param_names
                                    ),
                                    init.kind()
                                ),
                            )
                            .with_file(def.source_file.clone())
                            .with_label("declared type does not match initializer")
                            .with_help("adjust the declared type or initializer expression"),
                        );
                        return None;
                    }

                    let components = if let Some(ty_name) = declared_ty_name {
                        match ty_name.as_str() {
                            "f32" | "i32" | "u32" | "bool" | "float" | "int" => 1,
                            "vec2" | "uvec2" | "ivec2" | "bvec2" => 2,
                            "vec3" | "uvec3" | "ivec3" | "bvec3" => 3,
                            "vec4" | "uvec4" | "ivec4" | "bvec4" => 4,
                            _ => {
                                self.diags.push(
                                    Diag::error(
                                        value.span.clone(),
                                        format!(
                                            "function `{fn_name}` local `{name}` uses unsupported helper type `{ty_name}`"
                                        ),
                                    )
                                    .with_file(def.source_file.clone())
                                    .with_help("helper lowering supports only scalar/vec2/vec3/vec4 locals"),
                                );
                                return None;
                            }
                        }
                    } else {
                        Self::helper_value_components(&init)?
                    };

                    let slots = Self::helper_slots(name, components);
                    let placeholder = Self::helper_placeholder_value(&slots)?;
                    let placeholder = if let Some(kind) = Self::value_element_kind(&init) {
                        Self::map_value_lanes(placeholder, |lane| {
                            crate::typed_scalar::Scalar::input(lane, kind)
                        })
                    } else {
                        placeholder
                    };
                    self.bind(name.clone(), placeholder);

                    out.push(hir::UserFnStmt::Let {
                        name: name.clone(),
                        slots,
                        init: Self::helper_ir_expr(init)?,
                    });
                }
                Stmt::Const {
                    name,
                    value,
                    ty_name,
                    ty_span,
                    ..
                } => {
                    let init = self
                        .eval_scalar_expected(value, crate::typed_scalar::Kind::parse(ty_name))?;
                    let type_param_names: Vec<&str> =
                        def.type_params.iter().map(|(n, _)| n.as_str()).collect();
                    if !local_decl_type_matches(
                        &init,
                        ty_name,
                        &self.enum_defs,
                        &self.struct_defs,
                        &type_param_names,
                    ) {
                        self.diags.push(
                            Diag::error(
                                ty_span.clone(),
                                format!(
                                    "const `{name}` expected {}, but got {}",
                                    local_decl_expected_kind(
                                        ty_name,
                                        &self.enum_defs,
                                        &self.struct_defs,
                                        &type_param_names,
                                    ),
                                    init.kind()
                                ),
                            )
                            .with_file(def.source_file.clone())
                            .with_label("declared type does not match initializer")
                            .with_help("adjust the declared type or const initializer expression"),
                        );
                        return None;
                    }

                    let what = format!("const `{name}` initializer");
                    let folded = self.eval_compile_time_const_value(&init, &value.span, &what)?;
                    self.bind(name.clone(), folded);
                }
                Stmt::Assign {
                    name,
                    name_span,
                    field_path,
                    value,
                    ..
                } => {
                    let expected = match self.lookup(name) {
                        Some(Value::Scalar(ref scalar)) if field_path.is_none() => {
                            Some(scalar.scalar_kind())
                        }
                        _ => None,
                    };
                    let rhs = self.eval_scalar_expected(value, expected)?;
                    let Some(existing) = self.lookup(name) else {
                        self.diags.push(
                            Diag::error(
                                name_span.clone(),
                                format!("cannot assign to undeclared local `{name}`"),
                            )
                            .with_file(def.source_file.clone())
                            .with_label("unknown assignment target")
                            .with_help(
                                "declare the variable first with `let name = ...` or `type name = ...`",
                            ),
                        );
                        return None;
                    };

                    let target_components = if field_path.is_some() {
                        match &existing {
                            Value::Vec2(_) | Value::Vec3(_) | Value::Vec4(_) => {
                                match field_path.as_deref() {
                                    Some(path) => path.chars().count() as u8,
                                    None => 0,
                                }
                            }
                            _ => 0,
                        }
                    } else {
                        Self::helper_value_components(&existing).unwrap_or(0)
                    };
                    let rhs_components = Self::helper_value_components(&rhs).unwrap_or(0);
                    if target_components == 0 || target_components != rhs_components {
                        self.diags.push(
                            Diag::error(
                                name_span.clone(),
                                format!(
                                    "helper `{fn_name}` ({helper_id}) statement `assignment` to `{name}` expects {target_components} component(s), but got {rhs_components}"
                                ),
                            )
                            .with_tag("E_HELPER_ASSIGN_ARITY")
                            .with_file(def.source_file.clone())
                            .with_help("make assignment value match target scalar/vector arity"),
                        );
                        return None;
                    }

                    out.push(hir::UserFnStmt::Assign {
                        name: name.clone(),
                        field_path: field_path.clone(),
                        value: Self::helper_ir_expr(rhs)?,
                    });
                }
                Stmt::Expr(e) => {
                    let v = self.eval(e)?;
                    out.push(hir::UserFnStmt::Expr {
                        value: Self::helper_ir_expr(v)?,
                    });
                }
                Stmt::If {
                    cond,
                    then_body,
                    else_body,
                    ..
                } => {
                    let cond = self.as_scalar(cond)?;

                    self.scopes.push(HashMap::new());
                    let then_body = self
                        .emit_user_helper_stmt_list(helper_id, fn_name, def, then_body, in_loop)?;
                    self.scopes.pop();

                    let else_body = if let Some(else_body) = else_body {
                        self.scopes.push(HashMap::new());
                        let emitted = self.emit_user_helper_stmt_list(
                            helper_id, fn_name, def, else_body, in_loop,
                        )?;
                        self.scopes.pop();
                        emitted
                    } else {
                        Vec::new()
                    };

                    out.push(hir::UserFnStmt::If {
                        cond,
                        then_body,
                        else_body,
                    });
                }
                Stmt::Match {
                    value,
                    arms,
                    default_body,
                    default_span,
                    span,
                } => {
                    let lowered = self.lower_match_stmt_to_if_chain(
                        value,
                        arms,
                        default_body.as_ref(),
                        default_span.as_ref(),
                        &def.source_file,
                        span,
                    )?;
                    let nested = self.emit_user_helper_stmt_list(
                        helper_id,
                        fn_name,
                        def,
                        std::slice::from_ref(&lowered),
                        in_loop,
                    )?;
                    out.extend(nested);
                }
                Stmt::For {
                    name,
                    iterable,
                    body,
                    span,
                    index_name,
                    ..
                } => {
                    let iter_values =
                        self.eval_compile_time_iter_values(iterable, "function for-loop")?;
                    let mut values = Vec::with_capacity(iter_values.len());
                    for value in iter_values {
                        match value {
                            Value::Scalar(s)
                            | Value::Distance(s)
                            | Value::Coverage(s)
                            | Value::Mask(s) => values.push(s),
                            other => {
                                self.diags.push(
                                    Diag::error(
                                        span.clone(),
                                        format!(
                                            "function `{fn_name}` for-loop iterable must be scalar, found {}",
                                            other.kind()
                                        ),
                                    )
                                    .with_file(def.source_file.clone()),
                                );
                                return None;
                            }
                        }
                    }

                    let slots = Self::helper_slots(name, 1);
                    self.scopes.push(HashMap::new());
                    let loop_var = Self::helper_placeholder_value(&slots)?;
                    self.bind(name.clone(), loop_var);
                    // Bind a placeholder for the index variable so that references
                    // inside the body type-check.  The lowering pass substitutes
                    // the real loop counter at runtime.
                    let idx_slot_name = if let Some((iname, _)) = index_name {
                        let idx_slots = Self::helper_slots(iname, 1);
                        if let Some(idx_val) = Self::helper_placeholder_value(&idx_slots) {
                            self.bind(iname.clone(), idx_val);
                        }
                        Some(iname.clone())
                    } else {
                        None
                    };
                    let body =
                        self.emit_user_helper_stmt_list(helper_id, fn_name, def, body, true)?;
                    self.scopes.pop();

                    out.push(hir::UserFnStmt::For {
                        name: name.clone(),
                        slots,
                        values,
                        body,
                        index_name: idx_slot_name,
                    });
                }
                Stmt::ReturnVoid { span } => {
                    self.diags.push(Diag::error(
                        span.clone(),
                        "value-returning functions require a return value",
                    ));
                    return None;
                }
                Stmt::Return { value, .. } => {
                    let v = self.eval_scalar_expected(
                        value,
                        (!def.infer_return && matches!(def.ret, FnValueTy::Scalar))
                            .then_some(def.ret_kind),
                    )?;
                    out.push(hir::UserFnStmt::Return {
                        value: Self::helper_ir_expr(v)?,
                    });
                }
                Stmt::Break { span } => {
                    if !in_loop {
                        self.diags.push(
                            Diag::error(
                                span.clone(),
                                format!(
                                    "helper `{fn_name}` ({helper_id}) statement `break` is only valid inside loop bodies"
                                ),
                            )
                                .with_tag("E_HELPER_BREAK_OUTSIDE_LOOP")
                                .with_file(def.source_file.clone())
                                .with_help("use `break` inside a `for` loop body"),
                        );
                        return None;
                    }
                    out.push(hir::UserFnStmt::Break);
                }
                Stmt::Block { body, .. } => {
                    self.scopes.push(HashMap::new());
                    let nested =
                        self.emit_user_helper_stmt_list(helper_id, fn_name, def, body, in_loop)?;
                    self.scopes.pop();
                    out.extend(nested);
                }
                Stmt::Seq { body, .. } => {
                    let nested =
                        self.emit_user_helper_stmt_list(helper_id, fn_name, def, body, in_loop)?;
                    out.extend(nested);
                }
                Stmt::Param { .. }
                | Stmt::TextureBinding { .. }
                | Stmt::LetScatter { .. }
                | Stmt::SpaceDecl { .. }
                | Stmt::StyleDecl { .. }
                | Stmt::CanvasSpace { .. }
                | Stmt::InSpace { .. }
                | Stmt::InContext { .. }
                | Stmt::Compose { .. }
                | Stmt::ComposePiped { .. }
                | Stmt::SurfaceVertex { .. } => {
                    let (span, kind) = fn_stmt_span_kind(stmt);
                    self.diags.push(
                        Diag::error(
                            span,
                            format!(
                                "helper `{fn_name}` ({helper_id}) statement `{kind}` is not supported yet"
                            ),
                        )
                        .with_tag("E_HELPER_UNSUPPORTED_STMT")
                        .with_file(def.source_file.clone())
                        .with_help(
                            "runtime helper lowering supports scalar/vec2/vec3/vec4 logic with `if`, `for`, `let`, assignment, `return`, and expression statements",
                        ),
                    );
                    return None;
                }
                Stmt::LocalFnDecl(local_fn) => {
                    // Register nested local fn so it is visible during helper body evaluation.
                    self.register_local_fn_decl(local_fn);
                }
            }
        }

        Some(out)
    }

    fn ensure_user_helper(&mut self, name: &str, def: &FnDef) -> Option<String> {
        let ret_components = Self::helper_ret_components(&def.ret)?;
        let helper_id = Self::helper_id(name, def);
        if self.hir.user_helpers.contains_key(&helper_id) {
            return Some(helper_id);
        }

        tracing::info!(
            helper_id = %helper_id,
            helper_name = %name,
            params = def.params.len(),
            ret_components = ret_components,
            pending = self.pending_user_helpers.len(),
            emitted = self.hir.user_helpers.len(),
            "checker helper ensure start"
        );

        let mut bound_values: Vec<(String, Value)> = Vec::with_capacity(def.params.len());
        let mut flattened_params: Vec<String> = Vec::new();
        let mut helper_params: Vec<hir::UserFnParam> = Vec::with_capacity(def.params.len());
        for param in &def.params {
            let (value, scalar_names) = self.helper_param_placeholders(param)?;
            let value = match value {
                Value::Scalar(s) => {
                    Value::Scalar(crate::typed_scalar::Scalar::input(s, param.scalar_kind))
                }
                v => v,
            };
            let ty = Self::helper_param_ty(&param.ty)?;
            flattened_params.extend(scalar_names.iter().cloned());
            helper_params.push(hir::UserFnParam {
                scalar_kind: param.scalar_kind,
                name: param.name.clone(),
                ty,
                scalar_slots: scalar_names,
            });
            bound_values.push((param.name.clone(), value));
        }

        self.fn_call_stack.push(def.declaration_identity(name));
        self.scopes.push(HashMap::new());
        for (const_name, const_value) in &def.const_bindings {
            let bound = match const_value {
                ConstTemplateValue::U32(v) => Value::Scalar(
                    crate::typed_scalar::Scalar::from_literal(naga::Literal::U32(*v)),
                ),
                ConstTemplateValue::I32(v) => Value::Scalar(
                    crate::typed_scalar::Scalar::from_literal(naga::Literal::I32(*v)),
                ),
            };
            self.bind(const_name.clone(), bound);
        }
        for (param_name, value) in bound_values {
            self.bind(param_name, value);
        }
        let helper_emit_t0 = Instant::now();
        let out = self.emit_user_helper_stmt_list(&helper_id, name, def, &def.body, false);
        let helper_emit_ms = helper_emit_t0.elapsed().as_secs_f64() * 1000.0;
        const HELPER_SEMANTIC_VALIDATE_MAX_SPAN_BYTES: usize = 256;
        let helper_span_bytes = def.span.end.saturating_sub(def.span.start);
        let semantic_validation_skipped =
            helper_span_bytes > HELPER_SEMANTIC_VALIDATE_MAX_SPAN_BYTES;
        if name == "raycast" {
            tracing::info!(
                helper_name = %name,
                helper_span_bytes = helper_span_bytes,
                skipped = semantic_validation_skipped,
                "checker helper semantic validation check"
            );
        }
        let semantic_ok = if out.is_some() {
            // Keep helper lowering aligned with function semantics for small helpers, but avoid
            // expensive duplicate evaluation on large helper bodies.
            semantic_validation_skipped || self.eval_fn_body(name, def).is_some()
        } else {
            false
        };
        self.scopes.pop();
        self.fn_call_stack.pop();

        match &out {
            Some(v) => tracing::info!(
                helper_id = %helper_id,
                helper_name = %name,
                ms = helper_emit_ms,
                body_stmts = v.len(),
                semantic_ok = semantic_ok,
                semantic_validation_skipped = semantic_validation_skipped,
                "checker helper ensure eval done"
            ),
            None => tracing::info!(
                helper_id = %helper_id,
                helper_name = %name,
                ms = helper_emit_ms,
                "checker helper ensure eval failed"
            ),
        }

        let body_stmts = out?;
        if !semantic_ok {
            return None;
        }

        self.hir.user_helpers.insert(
            helper_id.clone(),
            hir::UserFnHelper {
                ret_kind: def.ret_kind,
                id: helper_id.clone(),
                params: helper_params,
                param_scalars: flattened_params,
                ret_components,
                sample_point_invariant: false,
                needs_entry_inputs: false,
                body_stmts,
            },
        );

        Some(helper_id)
    }

    fn make_helper_call_value(
        &mut self,
        def: &FnDef,
        helper_name: &str,
        call_span: &Span,
        helper_id: String,
        bound_values: &[(String, Value)],
    ) -> Option<Value> {
        let ret_components = Self::helper_ret_components(&def.ret)?;
        let mut flat_args: Vec<Sx> = Vec::new();

        let mut expected_args = 0usize;
        for param in &def.params {
            expected_args = expected_args.saturating_add(Self::helper_flat_param_arity(&param.ty)?);
        }

        for (param, (_, value)) in def.params.iter().zip(bound_values.iter()) {
            let Some(flattened) = self.flatten_param_value(param, value) else {
                self.diags.push(
                    Diag::error(
                        call_span.clone(),
                        format!(
                            "helper `{helper_name}` ({helper_id}) argument `{}` cannot be packed for lowering (expected {})",
                            param.name,
                            Self::helper_param_ty_name(&param.ty)
                        ),
                    )
                    .with_tag("E_HELPER_CALL_PACK_UNSUPPORTED")
                    .with_file(def.source_file.clone())
                    .with_help("use helper argument types supported by runtime helper packing"),
                );
                return Some(Value::Error);
            };
            flat_args.extend(flattened);
        }

        if flat_args.len() != expected_args {
            self.diags.push(
                Diag::error(
                    call_span.clone(),
                    format!(
                        "helper `{helper_name}` ({helper_id}) packed {} scalar slot(s), but {} were expected",
                        flat_args.len(),
                        expected_args
                    ),
                )
                .with_tag("E_HELPER_CALL_PACK_MISMATCH")
                .with_file(def.source_file.clone())
                .with_help("this indicates helper call packing drift between checker and lowering"),
            );
            return Some(Value::Error);
        }

        let call = Rc::new(hir::UserFnCall {
            helper_id,
            args: flat_args,
            ret_components,
        });

        let result = match ret_components {
            1 => Some(Value::Scalar(crate::typed_scalar::Scalar::input(
                Sx::UserCall {
                    call,
                    component: None,
                },
                def.ret_kind,
            ))),
            2 => Some(Value::Vec2((
                Sx::UserCall {
                    call: call.clone(),
                    component: Some(0),
                },
                Sx::UserCall {
                    call,
                    component: Some(1),
                },
            ))),
            3 => Some(Value::Vec3((
                Sx::UserCall {
                    call: call.clone(),
                    component: Some(0),
                },
                Sx::UserCall {
                    call: call.clone(),
                    component: Some(1),
                },
                Sx::UserCall {
                    call,
                    component: Some(2),
                },
            ))),
            4 => Some(Value::Vec4((
                Sx::UserCall {
                    call: call.clone(),
                    component: Some(0),
                },
                Sx::UserCall {
                    call: call.clone(),
                    component: Some(1),
                },
                Sx::UserCall {
                    call: call.clone(),
                    component: Some(2),
                },
                Sx::UserCall {
                    call,
                    component: Some(3),
                },
            ))),
            _ => None,
        };
        result.map(|value| {
            Self::map_value_lanes(value, |lane| {
                crate::typed_scalar::Scalar::input(lane, def.ret_kind)
            })
        })
    }

    pub(super) fn eval_user_fn_call(
        &mut self,
        name: &str,
        name_span: &Span,
        call_span: &Span,
        const_args: &[ConstTemplateArg],
        args: &[Arg],
    ) -> Option<Value> {
        with_checker_stack(|| {
            self.eval_user_fn_call_on_stack(name, name_span, call_span, const_args, args)
        })
    }

    #[inline(never)]
    fn eval_user_fn_call_on_stack(
        &mut self,
        name: &str,
        name_span: &Span,
        call_span: &Span,
        const_args: &[ConstTemplateArg],
        args: &[Arg],
    ) -> Option<Value> {
        let _span = trace_span!("check.eval_user_fn_call", name = %name).entered();
        let all_defs = self.fn_defs(name)?;
        let defs: Vec<FnDef> = if self.can_access_internal_functions() {
            all_defs
        } else {
            all_defs.into_iter().filter(|d| !d.is_internal).collect()
        };
        if defs.is_empty() {
            self.diags.push(
                Diag::error(
                    name_span.clone(),
                    format!("`{name}` is internal and cannot be called directly"),
                )
                .with_help("use a public stdlib wrapper or call this only from stdlib functions"),
            );
            return None;
        }
        let t0 = Instant::now();
        let mut def =
            self.select_overload_for_call(name, name_span, const_args.len(), args, defs)?;

        if def.const_params.len() != const_args.len() {
            let expected_template = if def.const_params.is_empty() {
                "<>".to_string()
            } else {
                format!(
                    "<{}>",
                    def.const_params
                        .iter()
                        .map(|p| {
                            format!(
                                "{}: {}",
                                p.name,
                                match p.ty {
                                    ConstTemplateParamTy::U32 => "u32",
                                    ConstTemplateParamTy::I32 => "i32",
                                }
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            self.diags.push(
                Diag::error(
                    name_span.clone(),
                    format!(
                        "`{name}` expects {} const template argument(s), found {}; expected template signature {}",
                        def.const_params.len(),
                        const_args.len(),
                        expected_template
                    ),
                )
                .with_help(format!(
                    "provide explicit const template arguments like `{name}<...>(...)` matching {}",
                    expected_template
                )),
            );
            return None;
        }

        let mut const_bindings = Vec::with_capacity(def.const_params.len());
        for (param, arg) in def.const_params.iter().zip(const_args.iter()) {
            let arg_num = match &arg.value.node {
                Expr::Num(v, Unit::None) => Some(*v),
                Expr::Var(var_name) => {
                    let Some(bound) = self.lookup(var_name) else {
                        self.diags.push(
                            Diag::error(
                                arg.span.clone(),
                                format!(
                                    "const template argument `{var_name}` must resolve to a compile-time integer"
                                ),
                            )
                            .with_help(
                                "use an integer literal or a compile-time known axis/const in this scope",
                            ),
                        );
                        return None;
                    };

                    match bound {
                        Value::Scalar(Sx::Typed(value)) => match value.evaluate(&HashMap::new()) {
                            Ok(naga::Literal::I32(value)) => Some(f64::from(value)),
                            Ok(naga::Literal::U32(value)) => Some(f64::from(value)),
                            Ok(naga::Literal::F32(value)) => Some(f64::from(value)),
                            _ => None,
                        },
                        Value::Scalar(Sx::Lit(v)) => Some(f64::from(v)),
                        Value::Distance(Sx::Lit(v)) => Some(f64::from(v)),
                        Value::Coverage(Sx::Lit(v)) => Some(f64::from(v)),
                        Value::Mask(Sx::Lit(v)) => Some(f64::from(v)),
                        _ => {
                            self.diags.push(
                                Diag::error(
                                    arg.span.clone(),
                                    format!(
                                        "const template argument `{var_name}` must resolve to a compile-time integer"
                                    ),
                                )
                                .with_help(
                                    "use an integer literal or a compile-time known axis/const in this scope",
                                ),
                            );
                            return None;
                        }
                    }
                }
                _ => None,
            };

            let value = match (&param.ty, arg_num) {
                (ConstTemplateParamTy::U32, Some(v))
                    if v.fract() == 0.0 && v >= 0.0 && v <= f64::from(u32::MAX) =>
                {
                    ConstTemplateValue::U32(v as u32)
                }
                (ConstTemplateParamTy::I32, Some(v))
                    if v.fract() == 0.0 && v >= f64::from(i32::MIN) && v <= f64::from(i32::MAX) =>
                {
                    ConstTemplateValue::I32(v as i32)
                }
                _ => {
                    self.diags.push(
                        Diag::error(
                            arg.span.clone(),
                            format!(
                                "const template argument for `{}` must be a {} integer literal or compile-time integer identifier",
                                param.name,
                                match param.ty {
                                    ConstTemplateParamTy::U32 => "u32",
                                    ConstTemplateParamTy::I32 => "i32",
                                }
                            ),
                        )
                        .with_help(
                            "use a plain integer literal or compile-time known integer name in the specialization list",
                        ),
                    );
                    return None;
                }
            };
            const_bindings.push((param.name.clone(), value));
        }
        def.const_bindings = const_bindings;

        if def.is_builtin {
            // Builtin declarations are signature-only; dispatch directly to lowering.
            return self.builtin(name, name_span, None, args, call_span);
        }

        if self.fn_call_stack.contains(&def.declaration_identity(name)) {
            self.diags.push(
                Diag::error(
                    name_span.clone(),
                    format!("recursive function call to `{name}` is not supported in v0"),
                )
                .with_help(
                    "use non-recursive helper functions until recursive evaluation is implemented",
                ),
            );
            return None;
        }
        if self.fn_call_stack.len() > 32 {
            self.diags.push(
                Diag::error(
                    name_span.clone(),
                    format!("function call depth exceeded while evaluating `{name}`"),
                )
                .with_help("reduce nested function call depth"),
            );
            return None;
        }

        let mut taken = vec![false; args.len()];
        let mut bound_values: Vec<(String, Value)> = Vec::with_capacity(def.params.len());
        let mut type_var_bindings: HashMap<String, &'static str> = HashMap::new();
        for param in &def.params {
            let found_idx = args
                .iter()
                .enumerate()
                .find(|(i, a)| !taken[*i] && a.name.as_deref() == Some(param.name.as_str()))
                .map(|(i, _)| i)
                .or_else(|| {
                    if param.keyword_only {
                        None
                    } else {
                        args.iter()
                            .enumerate()
                            .find(|(i, a)| !taken[*i] && a.name.is_none())
                            .map(|(i, _)| i)
                    }
                });

            let Some(idx) = found_idx else {
                let help = if param.keyword_only {
                    format!(
                        "provide `{}` by name; parameters after `*` are keyword-only",
                        param.name
                    )
                } else {
                    "provide all declared function parameters by name or position".to_string()
                };
                self.diags.push(
                    Diag::error(
                        name_span.clone(),
                        format!("`{name}` is missing argument `{}`", param.name),
                    )
                    .with_help(help),
                );
                return None;
            };
            taken[idx] = true;

            let arg = &args[idx].value;
            let raw = self.eval_scalar_expected(
                arg,
                matches!(param.ty, FnValueTy::Scalar).then_some(param.scalar_kind),
            )?;
            if let FnValueTy::TypeVar(var_name) = &param.ty {
                let type_key = value_to_type_key(&raw);
                if let Some(existing) = type_var_bindings.get(var_name.as_str()) {
                    if *existing != type_key {
                        self.diags.push(Diag::error(
                            arg.span.clone(),
                            format!(
                                "conflicting types for type parameter `{var_name}`: was `{existing}`, now `{type_key}`"
                            ),
                        ));
                        return None;
                    }
                } else {
                    type_var_bindings.insert(var_name.clone(), type_key);
                }
            }
            let coerced = match (&param.ty, &raw) {
                (FnValueTy::Scalar, Value::Scalar(_)) => raw,
                (FnValueTy::Scalar, Value::Distance(_)) => raw,
                (FnValueTy::Scalar, Value::Coverage(_)) => raw,
                (FnValueTy::Scalar, Value::Mask(_)) => raw,
                (FnValueTy::Vec2, Value::Vec2(_)) => raw,
                (FnValueTy::Vec3, Value::Vec3(_)) => raw,
                (FnValueTy::Vec4, Value::Vec4(_)) => raw,
                (FnValueTy::Mat2, Value::Mat2(_)) => raw,
                (FnValueTy::Mat3, Value::Mat3(_)) => raw,
                (FnValueTy::Mat4, Value::Mat4(_)) => raw,
                (FnValueTy::CoordLike, Value::Vec2(_)) => raw,
                (FnValueTy::Texture, Value::TypedTextureSample { .. }) => raw,
                (FnValueTy::Color, Value::Color { .. }) => raw,
                (FnValueTy::Color, Value::ColorField { .. }) => raw,
                (FnValueTy::Shape, Value::Shape(_)) => raw,
                (FnValueTy::Layer, Value::Layer(_)) => raw,
                (FnValueTy::Struct(expected), Value::Struct { ty_name, .. })
                    if ty_name == expected =>
                {
                    raw
                }
                (FnValueTy::Array(expected), Value::Array(_))
                    if local_decl_type_matches(
                        &raw,
                        expected,
                        &self.enum_defs,
                        &self.struct_defs,
                        &[],
                    ) =>
                {
                    raw
                }
                (FnValueTy::TypeVar(_), _) => raw,
                (FnValueTy::Enum(enum_name), Value::Scalar(_)) => {
                    if !self.validate_enum_call_arg_expr(enum_name, arg, name, &param.name)
                        || !self.validate_enum_scalar_value(
                            enum_name,
                            &raw,
                            &arg.span,
                            &format!("argument `{}` to `{name}`", param.name),
                        )
                    {
                        return None;
                    }
                    raw
                }
                // Callable parameter: validate the fn reference signature matches.
                (
                    FnValueTy::Callable {
                        params: sig_params,
                        ret: sig_ret,
                    },
                    Value::FnRef(target_name),
                ) => {
                    if let Some(target_defs) = self.fn_defs.get(target_name.as_str()) {
                        let matching = target_defs
                            .iter()
                            .find(|d| d.params.len() == sig_params.len());
                        if matching.is_none() {
                            self.diags.push(
                                Diag::error(
                                    arg.span.clone(),
                                    format!(
                                        "function `{target_name}` has no overload with {} parameter(s) for callable `{}`",
                                        sig_params.len(),
                                        param.name
                                    ),
                                )
                                .with_label("callable signature mismatch"),
                            );
                            return None;
                        }
                        let _ = sig_ret; // ret-type check reserved for future validation
                        raw
                    } else if let Some(effect_reg) =
                        self.effect_defs.get(target_name.as_str()).cloned()
                    {
                        // Effect reference (§16.1 Phase 6): validate arity against the effect's
                        // declared parameter count.  Effects always return layer, so the return
                        // type is implicitly compatible with fn(…)->layer callables.
                        if effect_reg.param_count != sig_params.len() {
                            self.diags.push(
                                Diag::error(
                                    arg.span.clone(),
                                    format!(
                                        "effect `{target_name}` expects {} argument(s), \
                                         but callable `{}` requires {} — arity mismatch",
                                        effect_reg.param_count,
                                        param.name,
                                        sig_params.len()
                                    ),
                                )
                                .with_label("arity mismatch"),
                            );
                            return None;
                        }
                        let _ = sig_ret; // Effects produce layer; ret-type check reserved
                        raw
                    } else {
                        self.diags.push(
                            Diag::error(
                                arg.span.clone(),
                                format!(
                                    "unknown function `{target_name}` passed as callable `{}`",
                                    param.name
                                ),
                            )
                            .with_help(
                                "declare the function before passing it as a callable argument",
                            ),
                        );
                        return None;
                    }
                }
                (expected_ty, v) => {
                    let expected_name = match expected_ty {
                        FnValueTy::Scalar => "scalar",
                        FnValueTy::Vec2 => "vec2",
                        FnValueTy::Vec3 => "vec3",
                        FnValueTy::Vec4 => "vec4",
                        FnValueTy::Mat2 => "mat2",
                        FnValueTy::Mat3 => "mat3",
                        FnValueTy::Mat4 => "mat4",
                        FnValueTy::CoordLike => "coord_like",
                        FnValueTy::ShaderResource(_) => "shader resource",
                        FnValueTy::Texture => "texture",
                        FnValueTy::Color => "color",
                        FnValueTy::Shape => "shape",
                        FnValueTy::Layer => "layer",
                        FnValueTy::Struct(name) | FnValueTy::Array(name) => name.as_str(),
                        FnValueTy::TypeVar(name) => name.as_str(),
                        FnValueTy::Enum(e) => e.as_str(),
                        FnValueTy::Callable { .. } => "fn reference",
                    };
                    self.diags.push(
                        Diag::error(
                            arg.span.clone(),
                            format!(
                                "argument `{}` to `{name}` expected {}, found {}",
                                param.name,
                                expected_name,
                                v.kind()
                            ),
                        )
                        .with_label("type mismatch in function call"),
                    );
                    return None;
                }
            };
            bound_values.push((param.name.clone(), coerced));
        }

        if !def.type_params.is_empty() {
            for (var_name, bounds) in &def.type_params {
                if let Some(concrete_type) = type_var_bindings.get(var_name.as_str()) {
                    for bound in bounds {
                        if !self
                            .conformance_set
                            .contains(&(concrete_type.to_string(), bound.clone()))
                        {
                            self.diags.push(
                                Diag::error(
                                    name_span.clone(),
                                    format!(
                                        "type `{concrete_type}` does not conform to interface `{bound}`"
                                    ),
                                )
                                .with_help(format!(
                                    "add `conform {concrete_type} : {bound} {{ ... }}` to declare conformance"
                                )),
                            );
                            return None;
                        }
                    }
                } else if !bounds.is_empty() {
                    self.diags.push(Diag::error(
                        name_span.clone(),
                        format!(
                            "cannot infer type parameter `{var_name}` — ensure at least one argument uses this type"
                        ),
                    ));
                    return None;
                }
            }
            if !type_var_bindings.is_empty() {
                let mut bindings = type_var_bindings
                    .iter()
                    .map(|(name, ty)| format!("{name}={ty}"))
                    .collect::<Vec<_>>();
                bindings.sort();
                let note = format!(
                    "generic `{name}` specialized with [{}]",
                    bindings.join(", ")
                );
                if !self
                    .specialization_notes
                    .iter()
                    .any(|existing| existing == &note)
                {
                    self.specialization_notes.push(note.clone());
                    self.hir.specialization_notes.push(note.clone());
                    self.hir
                        .notes
                        .push(format!("  specialization: {note} (§17.4)"));
                }
            }
        }

        for (i, a) in args.iter().enumerate() {
            if !taken[i] {
                let what = a
                    .name
                    .as_deref()
                    .map(|n| format!("unknown argument `{n}`"))
                    .unwrap_or_else(|| "unexpected extra argument".to_string());
                self.diags.push(
                    Diag::error(a.value.span.clone(), format!("{what} to `{name}`"))
                        .with_help("remove extra arguments or fix the function signature"),
                );
                return None;
            }
        }

        if self.emit_user_helper_calls
            && let Some(helper_id) = self.register_pending_user_helper(name, &def)
            && let Some(v) =
                self.make_helper_call_value(&def, name, call_span, helper_id.clone(), &bound_values)
        {
            if name == "map" || name == "raycast" {
                tracing::info!(name = %name, helper_id = %helper_id, "user_fn_call helper path");
            }
            self.record_check_profile_timing("user_fn_call", t0.elapsed());
            return Some(v);
        }
        if name == "map" || name == "raycast" {
            tracing::info!(
                name = %name,
                emit = self.emit_user_helper_calls,
                "user_fn_call INLINE path"
            );
        }

        let cache_key = {
            let mut key = format!("{:?}#{}", def.declaration_identity(name), def.params.len());
            if !type_var_bindings.is_empty() {
                let mut bindings = type_var_bindings.iter().collect::<Vec<_>>();
                bindings.sort_by(|a, b| a.0.cmp(b.0));
                for (var_name, ty) in bindings {
                    key.push('|');
                    key.push_str("typeparam:");
                    key.push_str(var_name);
                    key.push('=');
                    key.push_str(ty);
                }
            }
            for (param_name, v) in &bound_values {
                key.push('|');
                key.push_str(param_name);
                key.push('=');
                key.push_str(&Checker::value_cache_key(v));
            }
            for (const_name, const_value) in &def.const_bindings {
                key.push('|');
                key.push_str("const:");
                key.push_str(const_name);
                key.push('=');
                match const_value {
                    ConstTemplateValue::U32(v) => key.push_str(&format!("u{v}")),
                    ConstTemplateValue::I32(v) => key.push_str(&format!("i{v}")),
                }
            }
            key
        };
        if let Some(cached) = self.fn_eval_cache.get(&cache_key).cloned() {
            self.record_check_profile_timing("user_fn_call", t0.elapsed());
            return Some(cached);
        }

        self.fn_call_stack.push(def.declaration_identity(name));
        self.fn_source_stack.push(def.source_file.clone());
        self.scopes.push(HashMap::new());
        for (const_name, const_value) in &def.const_bindings {
            let bound = match const_value {
                ConstTemplateValue::U32(v) => Value::Scalar(
                    crate::typed_scalar::Scalar::from_literal(naga::Literal::U32(*v)),
                ),
                ConstTemplateValue::I32(v) => Value::Scalar(
                    crate::typed_scalar::Scalar::from_literal(naga::Literal::I32(*v)),
                ),
            };
            self.bind(const_name.clone(), bound);
        }
        // Only hoist arguments behind `Sx::Let` when the return shape is a plain
        // scalar/vector — that's the only shape `Self::apply_pending_lets` can
        // graft the bindings back onto. Anything else (struct/color/shape/...)
        // is bound directly, unchanged, since a param referenced inside such a
        // return value could leave dangling `Sx::Var` placeholders otherwise.
        let hoist_args = def.ret_kind == crate::typed_scalar::Kind::F32 && !bound_values.iter().any(|(_, value)| matches!(value, Value::Scalar(s) if s.scalar_kind() != crate::typed_scalar::Kind::F32)) && matches!(
            def.ret,
            FnValueTy::Scalar | FnValueTy::Vec2 | FnValueTy::Vec3 | FnValueTy::Vec4
        );
        let mut pending_lets: Vec<(String, Rc<Sx>)> = Vec::new();
        for (param_name, v) in bound_values {
            let bound = if hoist_args {
                Self::placeholder_for_call_arg(
                    &param_name,
                    v,
                    &mut self.let_counter,
                    &mut pending_lets,
                )
            } else {
                v
            };
            self.bind(param_name, bound);
        }
        let diag_start = self.diags.len();
        let mut out = self.eval_fn_body(name, &def);
        if hoist_args
            && !pending_lets.is_empty()
            && let Some(v) = out.take()
        {
            out = Some(Self::apply_pending_lets(v, &pending_lets));
        }
        for d in self.diags.iter_mut().skip(diag_start) {
            if d.file.is_none() {
                d.file = Some(def.source_file.clone());
            }
        }
        self.scopes.pop();
        self.fn_call_stack.pop();
        self.fn_source_stack.pop();
        if let Some(v) = out.clone() {
            self.fn_eval_cache.insert(cache_key, v);
        }
        self.record_check_profile_timing("user_fn_call", t0.elapsed());
        out
    }

    /// Generate a fresh checker-unique `Sx::Var` placeholder for a call
    /// argument that's about to be bound as a function parameter, recording
    /// `(placeholder_name, original_expression)` in `pending` so the caller
    /// can graft the real expression back exactly once (via `Sx::Let`)
    /// around the call's result, rather than letting the callee's body
    /// duplicate the argument's expression tree on every reference to it.
    ///
    /// Values that don't carry plain scalar/vector `Sx` leaves (structs,
    /// colors, shapes, textures, ...) are returned unchanged.
    fn placeholder_for_call_arg(
        param_name: &str,
        value: Value,
        let_counter: &mut u64,
        pending: &mut Vec<(String, Rc<Sx>)>,
    ) -> Value {
        let mut hoist = |label: &str, sx: Sx| -> Sx {
            if sx.scalar_kind() != crate::typed_scalar::Kind::F32 {
                return sx;
            }
            *let_counter += 1;
            let name = format!("__let_{param_name}_{label}_{let_counter}");
            pending.push((name.clone(), Rc::new(sx)));
            Sx::Var(name)
        };
        match value {
            Value::Scalar(sx) if sx.scalar_kind() != crate::typed_scalar::Kind::F32 => {
                Value::Scalar(sx)
            }
            Value::Scalar(sx) => Value::Scalar(hoist("s", sx)),
            Value::Distance(sx) => Value::Distance(hoist("d", sx)),
            Value::Coverage(sx) => Value::Coverage(hoist("c", sx)),
            Value::Mask(sx) => Value::Mask(hoist("m", sx)),
            Value::Vec2((x, y)) => Value::Vec2((hoist("x", x), hoist("y", y))),
            Value::Vec3((x, y, z)) => Value::Vec3((hoist("x", x), hoist("y", y), hoist("z", z))),
            Value::Vec4((x, y, z, w)) => {
                Value::Vec4((hoist("x", x), hoist("y", y), hoist("z", z), hoist("w", w)))
            }
            other => other,
        }
    }

    /// Wrap every `Sx` leaf carried by `value` with the pending `Sx::Let`
    /// bindings accumulated by `placeholder_for_call_arg`, so the returned
    /// expression embeds each hoisted argument's real expression exactly
    /// once instead of once per reference inside the callee's body.
    fn apply_pending_lets(value: Value, pending: &[(String, Rc<Sx>)]) -> Value {
        let wrap = |sx: Sx| -> Sx {
            pending.iter().rev().fold(sx, |body, (name, bound)| {
                // A guard may not depend on any of the loop's mutable values.
                // Do not embed unrelated accumulated state in that guard.
                let mut used = false;
                body.walk_preorder(&mut |node| {
                    if matches!(node, Sx::Var(var) if var == name) {
                        used = true;
                    }
                });
                if used {
                    Sx::Let {
                        name: name.clone(),
                        value: Rc::clone(bound),
                        body: Box::new(body),
                    }
                } else {
                    body
                }
            })
        };
        match value {
            Value::Scalar(sx) => Value::Scalar(wrap(sx)),
            Value::Distance(sx) => Value::Distance(wrap(sx)),
            Value::Coverage(sx) => Value::Coverage(wrap(sx)),
            Value::Mask(sx) => Value::Mask(wrap(sx)),
            Value::Vec2((x, y)) => Value::Vec2((wrap(x), wrap(y))),
            Value::Vec3((x, y, z)) => Value::Vec3((wrap(x), wrap(y), wrap(z))),
            Value::Vec4((x, y, z, w)) => Value::Vec4((wrap(x), wrap(y), wrap(z), wrap(w))),
            other => other,
        }
    }

    pub(super) fn eval_fn_body(&mut self, name: &str, def: &FnDef) -> Option<Value> {
        let previous_context = self.evaluation_context.take();
        let previous_cache = std::mem::take(&mut self.runtime_channel_cache);
        let marked = def
            .params
            .iter()
            .filter(|param| param.is_context)
            .collect::<Vec<_>>();
        if marked.len() > 1 {
            self.diags.push(Diag::error(
                def.span.clone(),
                "a function may have only one @context parameter",
            ));
        } else if let Some(param) = marked.first()
            && let Some(value) = self.lookup(&param.name)
        {
            self.evaluation_context = Some(self.context_roles(&value, &def.span));
        }
        let result = self.eval_fn_body_in_context(name, def);
        self.evaluation_context = previous_context;
        self.runtime_channel_cache = previous_cache;
        result
    }

    fn eval_fn_body_in_context(&mut self, name: &str, def: &FnDef) -> Option<Value> {
        let _span = trace_span!("check.eval_fn_body", name = %name).entered();
        let t0 = Instant::now();
        let Some(flow) = self.eval_fn_stmt_list(name, def, &def.body, false) else {
            self.record_check_profile_timing("fn_body", t0.elapsed());
            return None;
        };

        let selected = self.finish_inline_value(name, def, flow);
        let Some((value, value_span)) = selected else {
            self.diags.push(
                Diag::error(
                    def.span.clone(),
                    format!(
                        "not all control-flow paths in function `{name}` return a value"
                    ),
                )
                .with_file(def.source_file.clone())
                .with_help(
                    "add `return <expr>` on all paths or provide a trailing expression for fallthrough",
                ),
            );
            self.record_check_profile_timing("fn_body", t0.elapsed());
            return None;
        };

        let out = match (&def.ret, value) {
            (FnValueTy::Scalar, Value::Scalar(s)) => Some(Value::Scalar(s)),
            (FnValueTy::Scalar, Value::Distance(s)) => Some(Value::Distance(s)),
            (FnValueTy::Scalar, Value::Coverage(s)) => Some(Value::Coverage(s)),
            (FnValueTy::Scalar, Value::Mask(s)) => Some(Value::Mask(s)),
            (FnValueTy::Vec2, Value::Vec2(v)) => Some(Value::Vec2(v)),
            (FnValueTy::Vec3, Value::Vec3(v)) => Some(Value::Vec3(v)),
            (FnValueTy::Vec4, Value::Vec4(v)) => Some(Value::Vec4(v)),
            (FnValueTy::Mat2, Value::Mat2(v)) => Some(Value::Mat2(v)),
            (FnValueTy::Mat3, Value::Mat3(v)) => Some(Value::Mat3(v)),
            (FnValueTy::Mat4, Value::Mat4(v)) => Some(Value::Mat4(v)),
            (FnValueTy::CoordLike, Value::Vec2(v)) => Some(Value::Vec2(v)),
            (FnValueTy::Texture, v @ Value::TypedTextureSample { .. }) => Some(v),
            (FnValueTy::Color, Value::Color { rgba, space }) => Some(Value::Color { rgba, space }),
            (FnValueTy::Color, Value::ColorField { rgba, space }) => {
                Some(Value::ColorField { rgba, space })
            }
            (FnValueTy::Shape, Value::Shape(s)) => Some(Value::Shape(s)),
            (FnValueTy::Layer, Value::Layer(l)) => Some(Value::Layer(l)),
            (FnValueTy::Struct(expected), Value::Struct { ty_name, fields })
                if &ty_name == expected =>
            {
                Some(Value::Struct { ty_name, fields })
            }
            (FnValueTy::Array(expected), value @ Value::Array(_))
                if local_decl_type_matches(
                    &value,
                    expected,
                    &self.enum_defs,
                    &self.struct_defs,
                    &[],
                ) =>
            {
                Some(value)
            }
            (FnValueTy::TypeVar(_), value) => Some(value),
            (FnValueTy::Enum(enum_name), val) => {
                if self.validate_enum_scalar_value(
                    enum_name,
                    &val,
                    &value_span,
                    &format!("function `{name}` return value"),
                ) {
                    Some(val)
                } else {
                    None
                }
            }
            (ret_ty, other) => {
                let ret_ty_name = match ret_ty {
                    FnValueTy::Scalar => "scalar",
                    FnValueTy::Vec2 => "vec2",
                    FnValueTy::Vec3 => "vec3",
                    FnValueTy::Vec4 => "vec4",
                    FnValueTy::Mat2 => "mat2",
                    FnValueTy::Mat3 => "mat3",
                    FnValueTy::Mat4 => "mat4",
                    FnValueTy::CoordLike => "coord_like",
                    FnValueTy::ShaderResource(_) => "shader resource",
                    FnValueTy::Texture => "texture",
                    FnValueTy::Color => "color",
                    FnValueTy::Shape => "shape",
                    FnValueTy::Layer => "layer",
                    FnValueTy::Struct(name) | FnValueTy::Array(name) => name.as_str(),
                    FnValueTy::TypeVar(name) => name.as_str(),
                    FnValueTy::Enum(e) => e.as_str(),
                    FnValueTy::Callable { .. } => "fn reference",
                };
                self.diags.push(
                    Diag::error(
                        value_span,
                        format!(
                            "function `{name}` returns {}, but declared return type is {ret_ty_name}",
                            other.kind()
                        ),
                    )
                    .with_file(def.source_file.clone())
                    .with_label("return type mismatch"),
                );
                None
            }
        };
        self.record_check_profile_timing("fn_body", t0.elapsed());
        out
    }
}

fn fn_stmt_span_kind(stmt: &Stmt) -> (Span, &'static str) {
    match stmt {
        Stmt::Param { span, .. } => (span.clone(), "param"),
        Stmt::TextureBinding { span, .. } => (span.clone(), "uniform texture binding"),
        Stmt::Let { name_span, .. } => (name_span.clone(), "let"),
        Stmt::Const { name_span, .. } => (name_span.clone(), "const"),
        Stmt::Assign { name_span, .. } => (name_span.clone(), "assignment"),
        Stmt::Store { span, .. } => (span.clone(), "indexed assignment"),
        Stmt::For { span, .. } => (span.clone(), "for-loop"),
        Stmt::If { span, .. } => (span.clone(), "if"),
        Stmt::Match { span, .. } => (span.clone(), "match"),
        Stmt::LetScatter { scatter, .. } => (scatter.span.clone(), "scatter"),
        Stmt::SpaceDecl { span, .. } => (span.clone(), "space declaration"),
        Stmt::StyleDecl { span, .. } => (span.clone(), "style declaration"),
        Stmt::CanvasSpace { span, .. } => (span.clone(), "canvas_space declaration"),
        Stmt::Compose { span, .. } => (span.clone(), "compose"),
        Stmt::ComposePiped { span, .. } => (span.clone(), "compose with pipes"),
        Stmt::SurfaceVertex { span, .. } => (span.clone(), "surface vertex"),
        Stmt::Return { span, .. } | Stmt::ReturnVoid { span } => (span.clone(), "return"),
        Stmt::Break { span } => (span.clone(), "break"),
        Stmt::InSpace { span, .. } => (span.clone(), "in-space block"),
        Stmt::InContext { span, .. } => (span.clone(), "in-context block"),
        Stmt::Block { span, .. } => (span.clone(), "grouped block"),
        Stmt::Seq { span, .. } => (span.clone(), "statement sequence"),
        Stmt::LocalFnDecl(fn_decl) => (fn_decl.name_span.clone(), "local fn declaration"),
        Stmt::Expr(e) => (e.span.clone(), "expression"),
    }
}

/// Result of a workbook-aware check pass.
pub struct WorkbookCheckResult {
    pub hir: Hir,
    #[allow(
        dead_code,
        reason = "workbook diagnostics are retained for future API consumers"
    )]
    pub diags: Vec<Diag>,
    /// All `(span, kind)` pairs recorded during evaluation.
    pub span_log: Vec<(Span, SpanValueKind)>,
    /// The tightest-fitting capture at the requested byte range, if any.
    pub captured: Option<SpanCapture>,
}

/// Like [`check`], but also records span→type information for every evaluated
/// expression.  When `capture_range` is `Some((start, end))`, additionally
/// captures the tightest-fitting expression value at that byte range so the
/// caller can synthesize a variant canvas entry (§21.7).
#[expect(
    clippy::too_many_arguments,
    reason = "This compiler boundary explicitly threads independent checking or lowering inputs."
)]
pub fn check_with_workbook(
    entry: &NormalizedRootEntry,
    consts: &[ConstDecl],
    functions: &[FnDecl],
    enums: &[EnumDecl],
    structs: &[StructDecl],
    params: &[GlobalParamDecl],
    texture_types: &[TextureTypeDecl],
    interfaces: &[InterfaceDecl],
    conformances: &[ConformanceDecl],
    effects: &[EffectDecl],
    options: &CheckOptions,
    capture_range: Option<(usize, usize)>,
) -> Result<WorkbookCheckResult, Vec<Diag>> {
    let _span = tracing::info_span!(
        "check.entry.workbook",
        entry = %entry.name,
        fn_count = functions.len(),
        enum_count = enums.len()
    )
    .entered();
    let mut prep_diags = Vec::new();
    let contract = if matches!(entry.kind, RootEntryKind::Canvas) {
        crate::context::raster_contract(&entry.entry_kind, interfaces, structs)?
    } else {
        None
    };
    if contract.is_none() {
        validate_normalized_root_entry_params(entry, &mut prep_diags);
    }
    let mut enum_defs = HashMap::new();
    let mut struct_defs = HashMap::new();

    // Load stdlib enums first
    {
        let stdlib_enums_program = native_enums_program();
        for en in stdlib_enums_program.enums {
            let mut variants = HashMap::new();
            for variant in &en.variants {
                variants.insert(variant.name.clone(), variant.span.clone());
            }
            enum_defs.insert(en.name.clone(), EnumDef { variants });
        }
    }

    // Then process user-defined enums
    for en in enums {
        if enum_defs.contains_key(&en.name) {
            prep_diags.push(
                Diag::error(
                    en.name_span.clone(),
                    format!("duplicate enum declaration `{}`", en.name),
                )
                .with_label("enum name redefined")
                .with_help("enum names must be unique"),
            );
            continue;
        }

        let mut variants = HashMap::new();
        for variant in &en.variants {
            if variants.contains_key(&variant.name) {
                prep_diags.push(
                    Diag::error(
                        variant.span.clone(),
                        format!(
                            "duplicate enum variant `{}` in enum `{}`",
                            variant.name, en.name
                        ),
                    )
                    .with_label("variant redefined")
                    .with_help("enum variant names must be unique within an enum"),
                );
                continue;
            }
            variants.insert(variant.name.clone(), variant.span.clone());
        }

        enum_defs.insert(en.name.clone(), EnumDef { variants });
    }

    for st in structs {
        if struct_defs.contains_key(&st.name) {
            prep_diags.push(
                Diag::error(
                    st.name_span.clone(),
                    format!("duplicate struct declaration `{}`", st.name),
                )
                .with_label("struct name redefined")
                .with_help("struct names must be unique"),
            );
            continue;
        }

        let mut fields = HashMap::new();
        for field in &st.fields {
            if fields.contains_key(&field.name) {
                prep_diags.push(
                    Diag::error(
                        field.name_span.clone(),
                        format!("duplicate field `{}` in struct `{}`", field.name, st.name),
                    )
                    .with_label("field redefined")
                    .with_help("struct field names must be unique within a struct"),
                );
                continue;
            }
            fields.insert(
                field.name.clone(),
                StructFieldDef {
                    semantic: field.semantic.clone(),
                    ty_name: field.ty_name.clone(),
                    span: field.ty_span.clone(),
                },
            );
        }

        struct_defs.insert(st.name.clone(), StructDef { fields });
    }

    let interface_names: HashSet<String> =
        interfaces.iter().map(|iface| iface.name.clone()).collect();
    validate_interface_conformance_methods(interfaces, conformances, &mut prep_diags);
    let mut interface_defs = HashMap::new();
    for iface in interfaces {
        interface_defs.insert(
            iface.name.clone(),
            InterfaceDef {
                method_names: iface
                    .methods
                    .iter()
                    .map(|method| method.name.clone())
                    .collect(),
            },
        );
    }
    let mut conformance_set = HashSet::new();
    if interface_defs.contains_key("Sdf") {
        conformance_set.insert(("shape".to_string(), "Sdf".to_string()));
    }
    if interface_defs.contains_key("Lerpable") {
        for ty in ["scalar", "color", "vec2", "vec3", "vec4"] {
            conformance_set.insert((ty.to_string(), "Lerpable".to_string()));
        }
    }
    for conform in conformances {
        conformance_set.insert((
            type_name_to_key(&conform.type_name),
            conform.interface_name.clone(),
        ));
    }

    let mut fn_defs: HashMap<String, Vec<FnDef>> = HashMap::new();
    for func in functions {
        let incoming_is_stdlib = is_stdlib_source(&func.source_file);
        let type_param_names: Vec<String> = func
            .type_params
            .iter()
            .map(|param| param.name.clone())
            .collect();
        let type_params: Vec<(String, Vec<String>)> = func
            .type_params
            .iter()
            .map(|param| (param.name.clone(), param.bounds.clone()))
            .collect();
        let mut const_params: Vec<ConstTemplateParamDef> = Vec::new();
        let mut template_name_set: HashSet<String> = type_param_names.iter().cloned().collect();
        let mut bad_template = false;
        for const_param in &func.const_params {
            if !template_name_set.insert(const_param.name.clone()) {
                prep_diags.push(
                    Diag::error(
                        const_param.name_span.clone(),
                        format!(
                            "duplicate template parameter `{}` in function `{}`",
                            const_param.name, func.name
                        ),
                    )
                    .with_file(func.source_file.clone())
                    .with_help("template parameter names must be unique"),
                );
                bad_template = true;
                continue;
            }

            let ty = match const_param.kind {
                ConstTemplateKind::U32 => ConstTemplateParamTy::U32,
                ConstTemplateKind::I32 => ConstTemplateParamTy::I32,
            };
            const_params.push(ConstTemplateParamDef {
                name: const_param.name.clone(),
                ty,
            });
        }
        if bad_template {
            continue;
        }

        let Some(ret) = func.ret_ty.as_ref().and_then(|(name, span)| {
            parse_fn_type(
                name,
                span,
                &mut prep_diags,
                "return type",
                Some(func.source_file.as_str()),
                &enum_defs,
                &struct_defs,
                &type_param_names,
                &interface_names,
            )
        }) else {
            prep_diags.push(
                Diag::error(
                    func.name_span.clone(),
                    format!("function `{}` must declare a return type", func.name),
                )
                .with_file(func.source_file.clone())
                .with_help(
                    "supported types: f32, vec2, vec3, color, shape, layer, or a defined enum",
                ),
            );
            continue;
        };

        let mut param_options: Vec<Vec<FnParamDef>> = Vec::with_capacity(func.params.len());
        let mut bad_param = false;
        let mut seen_param_names: HashMap<String, Span> = HashMap::new();
        for p in &func.params {
            if seen_param_names.contains_key(&p.name) {
                prep_diags.push(
                    Diag::error(
                        p.name_span.clone(),
                        format!(
                            "duplicate parameter `{}` in function `{}`",
                            p.name, func.name
                        ),
                    )
                    .with_file(func.source_file.clone())
                    .with_label("parameter name redefined")
                    .with_help(format!(
                        "rename this parameter; `{}` was already declared earlier",
                        p.name
                    )),
                );
                bad_param = true;
                continue;
            }
            seen_param_names.insert(p.name.clone(), p.name_span.clone());

            let mut options = Vec::new();
            for ty_name in split_decl_type_union(&p.ty_name) {
                let Some(ty) = parse_fn_type(
                    &ty_name,
                    &p.ty_span,
                    &mut prep_diags,
                    "parameter type",
                    Some(func.source_file.as_str()),
                    &enum_defs,
                    &struct_defs,
                    &type_param_names,
                    &interface_names,
                ) else {
                    bad_param = true;
                    continue;
                };
                options.push(FnParamDef {
                    is_context: p.is_context,
                    name: p.name.clone(),
                    ty,
                    scalar_kind: crate::typed_scalar::Kind::element(&ty_name)
                        .unwrap_or(crate::typed_scalar::Kind::F32),
                    scalar_specialization: scalar_specialization_from_type_name(&ty_name),
                    keyword_only: p.keyword_only,
                });
            }

            if options.is_empty() {
                bad_param = true;
                continue;
            }

            param_options.push(options);
        }
        if bad_param {
            continue;
        }

        let mut expanded_params: Vec<Vec<FnParamDef>> = vec![Vec::new()];
        for options in param_options {
            let mut next = Vec::new();
            for base in &expanded_params {
                for opt in &options {
                    let mut branch = base.clone();
                    branch.push(opt.clone());
                    next.push(branch);
                }
            }
            expanded_params = next;
        }

        for params in expanded_params {
            let def = FnDef {
                infer_return: false,
                ret_kind: func
                    .ret_ty
                    .as_ref()
                    .and_then(|(ty, _)| crate::typed_scalar::Kind::element(ty))
                    .unwrap_or(crate::typed_scalar::Kind::F32),
                params,
                ret: ret.clone(),
                is_internal: func.is_internal,
                is_builtin: func.is_builtin,
                source_file: func.source_file.clone(),
                body: func.body.clone(),
                span: func.span.clone(),
                type_params: type_params.clone(),
                const_params: const_params.clone(),
                const_bindings: Vec::new(),
            };

            let entry = fn_defs.entry(func.name.clone()).or_default();

            if !incoming_is_stdlib && entry.iter().all(|e| is_stdlib_source(&e.source_file)) {
                entry.clear();
            }

            if entry.iter().any(|e| e.same_overload_signature(&def)) {
                prep_diags.push(
                    Diag::error(
                        func.name_span.clone(),
                        format!(
                            "duplicate function declaration `{}` with matching overload signature",
                            func.name,
                        ),
                    )
                    .with_file(func.source_file.clone())
                    .with_help("function overloads must differ by parameter type specialization, arity, or keyword-only placement"),
                );
                continue;
            }

            entry.push(def);
        }
    }

    let global_uniform_registry =
        globals::build_global_uniform_registry(params, structs, &mut prep_diags);

    let mut c = Checker {
        hir: Hir {
            entry_context: None,
            name: entry.name.clone(),
            params: Vec::new(),
            rendering_policy: options.rendering_policy(),
            canvas_space: None,
            canvas_jacobian: None,
            shapes: Vec::new(),
            layers: Vec::new(),
            layer_locality: Vec::new(),
            root: 0,
            notes: Vec::new(),
            specialization_notes: Vec::new(),
            textures: Vec::new(),
            texture_index: HashMap::new(),
            texture_metadata: HashMap::new(),
            texture_type_defs: HashMap::new(),
            user_helpers: HashMap::new(),
            path_profiles: Vec::new(),
            effects: Vec::new(),
            effect_by_name: HashMap::new(),
            global_uniforms: global_uniform_registry.defs.clone(),
        },
        diags: prep_diags,
        scopes: vec![HashMap::new()],
        assignment_scopes: Vec::new(),
        style_scopes: vec![HashMap::new()],
        scatter_rand_scopes: Vec::new(),
        next_repeat_cell_scope_id: 0,
        enum_defs,
        struct_defs,
        fn_defs,
        interface_defs,
        conformance_set,
        interface_names,
        fn_eval_cache: HashMap::new(),
        pending_user_helpers: HashMap::new(),
        emit_user_helper_calls: true,
        fn_call_stack: Vec::new(),
        fn_source_stack: Vec::new(),
        let_counter: 0,
        specialization_notes: Vec::new(),
        check_profile_timing: CheckProfileTiming::default(),
        check_options: *options,
        workbook_enabled: true,
        expr_watchdog_every: Checker::expr_watchdog_every_default(),
        expr_watchdog_counter: 0,
        expr_timeout: Checker::expr_timeout_default(options),
        expr_timeout_reported: false,
        check_started_at: Instant::now(),
        expr_hotspots: HashMap::new(),
        expr_hotspot_summary_emitted: false,
        span_log: Vec::new(),
        capture_target: capture_range,
        captured: None,
        current_filtering_state: hir::FilteringState::Auto,
        effect_defs: HashMap::new(),
        global_uniforms: global_uniform_registry,
        runtime_channel_cache: globals::RuntimeChannelCache::default(),
        evaluation_context: None,
    };

    declarations::bind_top_level_consts(&mut c, consts);
    c.bind_global_uniforms();
    c.declare_global_params(params, structs);

    // Register texture_type definitions into the HIR.
    for tt in texture_types {
        let mut channels = Vec::new();
        for ch_def in &tt.channels {
            let channel_idx = match ch_def.channel.as_str() {
                "r" => 0u8,
                "g" => 1u8,
                "b" => 2u8,
                "a" => 3u8,
                _ => {
                    c.diags.push(Diag::error(
                        ch_def.channel_span.clone(),
                        format!(
                            "invalid channel `{}` in texture_type `{}`; expected r, g, b, or a",
                            ch_def.channel, tt.name
                        ),
                    ));
                    continue;
                }
            };
            let (decode_mul, decode_add, decode_expr) = match &ch_def.decode {
                Some(TextureChannelDecode::Affine { mul, add }) => (*mul, *add, None),
                Some(TextureChannelDecode::Expr(expr)) => {
                    c.scopes.push(HashMap::new());
                    c.bind("raw".to_string(), Value::Scalar(Sx::Var("raw".to_string())));
                    c.bind(
                        "texel".to_string(),
                        Value::Vec4((
                            Sx::Var("texel_r".to_string()),
                            Sx::Var("texel_g".to_string()),
                            Sx::Var("texel_b".to_string()),
                            Sx::Var("texel_a".to_string()),
                        )),
                    );
                    let decoded = c.eval(expr).unwrap_or(Value::Error);
                    c.scopes.pop();

                    let sx = match decoded {
                        Value::Scalar(s)
                        | Value::Distance(s)
                        | Value::Coverage(s)
                        | Value::Mask(s) => Some(s),
                        Value::Error => None,
                        other => {
                            c.diags.push(
                                Diag::error(
                                    expr.span.clone(),
                                    format!(
                                        "texture decode expression for `{}` must produce a scalar, found {}",
                                        ch_def.semantic_name,
                                        other.kind()
                                    ),
                                )
                                .with_help("use `raw` (selected channel) or `texel.r/g/b/a` and return a scalar decode expression"),
                            );
                            None
                        }
                    };

                    (1.0, 0.0, sx)
                }
                None => (1.0, 0.0, None),
            };
            channels.push(hir::TextureChannelDef {
                channel_idx,
                channel_name: ch_def.channel.clone(),
                semantic_name: ch_def.semantic_name.clone(),
                decode_mul,
                decode_add,
                decode_expr,
            });
        }

        let result_expr = match (&tt.result_ty, &tt.result_expr) {
            (None, None) => None,
            (Some((ty_name, _ty_span)), Some(expr)) => {
                c.scopes.push(HashMap::new());
                for (src_ch, h_ch) in tt.channels.iter().zip(&channels) {
                    let decoded = Sx::TexChannel {
                        tex_name: tt.name.clone(),
                        channel: h_ch.channel_idx,
                        sample_at: None,
                        decode_mul: h_ch.decode_mul,
                        decode_add: h_ch.decode_add,
                        decode_expr: h_ch.decode_expr.clone().map(Box::new),
                    };
                    c.bind(src_ch.channel.clone(), Value::Scalar(decoded.clone()));
                    c.bind(src_ch.semantic_name.clone(), Value::Scalar(decoded));
                }
                let value = c.eval(expr);
                c.scopes.pop();
                match value {
                    Some(value) => {
                        if local_decl_type_matches(
                            &value,
                            ty_name,
                            &c.enum_defs,
                            &c.struct_defs,
                            &[],
                        ) {
                            Some(expr.clone())
                        } else {
                            c.diags.push(
                                Diag::error(
                                    expr.span.clone(),
                                    format!(
                                        "texture_type result expression for `{}` must produce `{ty_name}`, found {}",
                                        tt.name,
                                        value.kind()
                                    ),
                                )
                                .with_help(
                                    "use a builtin value expression that matches the declared result type, such as `rgb(...)`, `rgba(...)`, `vec2(...)`, `vec3(...)`, or `vec4(...)`",
                                ),
                            );
                            None
                        }
                    }
                    None => None,
                }
            }
            (Some((_ty_name, _ty_span)), None) => None,
            (None, Some(expr)) => {
                c.diags.push(Diag::error(
                    expr.span.clone(),
                    "texture_type return expression requires `-> vec3` or `-> color`",
                ));
                None
            }
        };
        c.hir.texture_type_defs.insert(
            tt.name.clone(),
            hir::TextureTypeDef {
                channels,
                result_expr,
            },
        );
    }

    c.check_effect_decls(effects);

    if let Some(contract) = contract {
        c.bind_entry_context(entry, contract);
        c.append_context_components();
    } else {
        for param in &entry.params {
            match param.ty_name.as_str() {
                "coord" => {
                    c.bind(param.name.clone(), Value::Vec2((Sx::CoordX, Sx::CoordY)));
                }
                other => {
                    c.bind_runtime_channel_param(&param.name, other);
                }
            }
        }
    }

    c.validate_registered_function_overloads();

    let root = c.eval_block(&entry.body, &entry.name_span, true, false);
    c.materialize_pending_user_helpers();
    match root {
        Some(root) if !has_errors(&c.diags) => {
            c.hir.root = root;
            Ok(WorkbookCheckResult {
                hir: c.hir,
                diags: c.diags,
                span_log: c.span_log,
                captured: c.captured.map(|(_, cap)| cap),
            })
        }
        _ => {
            if !has_errors(&c.diags) {
                c.diags.push(
                    Diag::error(entry.name_span.clone(), "entry body produces no layer").with_help(
                        "end the entry with a `compose { ... }` block or a layer expression",
                    ),
                );
            }
            Err(c.diags)
        }
    }
}

impl Checker {
    /// Called from the `eval` wrapper in `expr.rs` after every expression is
    /// evaluated.  Records the span kind and, when a capture target is set,
    /// updates the tightest-fit capture.
    pub(super) fn record_span_value(&mut self, span_start: usize, span_end: usize, value: &Value) {
        let kind = match value {
            Value::Error => return,
            Value::Layer(_) => SpanValueKind::Layer,
            Value::Shape(_) => SpanValueKind::Shape,
            Value::Color { .. } => SpanValueKind::Color,
            Value::ColorField { .. } => SpanValueKind::ColorField,
            Value::Scalar(_) | Value::Distance(_) | Value::Coverage(_) | Value::Mask(_) => {
                SpanValueKind::Scalar
            }
            Value::Slot { .. } => SpanValueKind::Scalar,
            Value::Vec2(_) => SpanValueKind::Vec2,
            Value::Vec3(_) => SpanValueKind::Vec3,
            Value::Vec4(_) => SpanValueKind::Vec4,
            Value::Mat2(_) | Value::Mat3(_) | Value::Mat4(_) => SpanValueKind::Scalar,
            Value::Array(_) => SpanValueKind::Scalar,
            Value::ScatterInstance { .. } => SpanValueKind::Vec2,
            Value::RepeatCell(_) => SpanValueKind::Vec2,
            Value::CellContour { .. } => SpanValueKind::Scalar,
            Value::Gradient { .. } => SpanValueKind::Color,
            Value::Space(_) => SpanValueKind::Space,
            Value::Lambda { .. } => SpanValueKind::Scalar,
            Value::FnRef(_) => SpanValueKind::Scalar,
            Value::TypedTextureSample { .. } => SpanValueKind::Layer,
            Value::PathFuture(_) => SpanValueKind::Scalar,
            Value::DynamicArray { .. } => SpanValueKind::Scalar,
            Value::Struct { .. } => SpanValueKind::Scalar,
        };

        if self.workbook_enabled {
            self.span_log.push((span_start..span_end, kind));
        }

        if let Some((tgt_start, tgt_end)) = self.capture_target
            && span_start <= tgt_start
            && tgt_end <= span_end
        {
            let span_size = span_end - span_start;
            let is_better = self
                .captured
                .as_ref()
                .is_none_or(|(prev_size, _)| span_size < *prev_size);
            if is_better {
                let capture = match value {
                    Value::Error => None,
                    Value::Layer(id) => Some(SpanCapture::Layer(*id)),
                    Value::Shape(id) => Some(SpanCapture::Shape(*id)),
                    Value::Color { rgba, .. } => Some(SpanCapture::Color(*rgba)),
                    Value::ColorField { rgba, .. } => Some(SpanCapture::ColorField(rgba.clone())),
                    Value::Scalar(sx)
                    | Value::Distance(sx)
                    | Value::Coverage(sx)
                    | Value::Mask(sx) => Some(SpanCapture::Scalar(sx.clone())),
                    Value::Vec2(v) => Some(SpanCapture::Vec2(v.clone())),
                    Value::Vec3(v) => Some(SpanCapture::Vec3(v.clone())),
                    Value::Vec4(v) => Some(SpanCapture::Vec4(v.clone())),
                    Value::Mat2(_) | Value::Mat3(_) | Value::Mat4(_) => None,
                    Value::Array(_) => None,
                    Value::ScatterInstance { pos, .. } => Some(SpanCapture::Vec2(pos.clone())),
                    Value::RepeatCell(cell) => Some(SpanCapture::Vec2(cell.center.clone())),
                    Value::CellContour { .. } => None,
                    Value::Slot { .. } => None,
                    Value::Gradient { .. } | Value::Lambda { .. } | Value::FnRef(_) => None,
                    Value::Space(xforms) => Some(SpanCapture::Space(xforms.clone())),
                    Value::TypedTextureSample { layer_id, .. } => {
                        Some(SpanCapture::Layer(*layer_id))
                    }
                    Value::PathFuture(_) => None,
                    Value::DynamicArray { .. } => None,
                    Value::Struct { .. } => None,
                };
                if let Some(cap) = capture {
                    self.captured = Some((span_size, cap));
                }
            }
        }
    }
}
