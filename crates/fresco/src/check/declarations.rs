use super::*;
use tracing::{info, info_span};
use web_time::Instant;

fn profile_timing_notes_enabled() -> bool {
    std::env::var("FRESCO_CHECK_PROFILE_TIMINGS")
        .ok()
        .map(|v| {
            let lower = v.to_ascii_lowercase();
            matches!(lower.as_str(), "1" | "true" | "on" | "yes")
        })
        .unwrap_or(false)
}

pub(super) fn bind_top_level_consts(checker: &mut Checker, consts: &[ConstDecl]) {
    let mut seen: HashMap<String, Span> = HashMap::new();

    for decl in consts {
        if let Some(first_span) = seen.get(&decl.name) {
            checker.diags.push(
                Diag::error(
                    decl.name_span.clone(),
                    format!("duplicate const declaration `{}`", decl.name),
                )
                .with_related_label(
                    first_span.clone(),
                    format!("first declaration of `{}`", decl.name),
                )
                .with_help("const names must be unique across merged imports"),
            );
            continue;
        }
        seen.insert(decl.name.clone(), decl.name_span.clone());

        let Some(value) = checker.eval(&decl.value) else {
            checker.diags.push(
                Diag::error(
                    decl.value.span.clone(),
                    format!("failed to evaluate const `{}`", decl.name),
                )
                .with_help("const values must be compile-time evaluable expressions"),
            );
            continue;
        };

        if matches!(value, Value::Error) {
            continue;
        }

        if !local_decl_type_matches(
            &value,
            &decl.ty_name,
            &checker.enum_defs,
            &checker.struct_defs,
            &[],
        ) {
            checker.diags.push(
                Diag::error(
                    decl.ty_span.clone(),
                    format!(
                        "const `{}` expected {}, but got {}",
                        decl.name,
                        local_decl_expected_kind(
                            &decl.ty_name,
                            &checker.enum_defs,
                            &checker.struct_defs,
                            &[]
                        ),
                        value.kind()
                    ),
                )
                .with_help("adjust the declared type or const initializer expression"),
            );
            continue;
        }

        let what = format!("const `{}` initializer", decl.name);
        let Some(folded_value) =
            checker.eval_compile_time_const_value(&value, &decl.value.span, &what)
        else {
            continue;
        };

        checker.bind(decl.name.clone(), folded_value);
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "This compiler boundary explicitly threads independent checking or lowering inputs."
)]
pub(super) fn create_checker_for_surface(
    name: String,
    functions: &[FnDecl],
    consts: &[ConstDecl],
    enums: &[EnumDecl],
    structs: &[StructDecl],
    params: &[GlobalParamDecl],
    texture_types: &[TextureTypeDecl],
    interfaces: &[InterfaceDecl],
    conformances: &[ConformanceDecl],
    effects: &[EffectDecl],
    options: &CheckOptions,
) -> (Checker, Vec<Diag>) {
    let mut prep_diags = Vec::new();
    let mut enum_defs = HashMap::new();
    let mut struct_defs = HashMap::new();

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
    if !conformances.is_empty() {
        validate_interface_conformance_methods(interfaces, conformances, &mut prep_diags);
    }
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

        let Some((ret_name, ret_span)) = func.ret_ty.as_ref() else {
            prep_diags.push(
                Diag::error(
                    func.name_span.clone(),
                    format!("function `{}` must declare a return type", func.name),
                )
                .with_file(func.source_file.clone())
                .with_help(
                    "supported types: f32, f64, half, i32, u32, bool, angle, length, vec2, coord, coord_like, resolution, vec3, vec4, mat2, mat3, mat4, color, shape, layer, signal, delta, mask, coverage, or a defined enum",
                ),
            );
            continue;
        };

        let Some(ret) = parse_fn_type(FnTypeInputs {
            name: ret_name,
            span: ret_span,
            diags: &mut prep_diags,
            context: "return type",
            source_file: Some(func.source_file.as_str()),
            enum_defs: &enum_defs,
            struct_defs: &struct_defs,
            type_params: &type_param_names,
            interface_names: &interface_names,
        }) else {
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
                let Some(ty) = parse_fn_type(FnTypeInputs {
                    name: &ty_name,
                    span: &p.ty_span,
                    diags: &mut prep_diags,
                    context: "parameter type",
                    source_file: Some(func.source_file.as_str()),
                    enum_defs: &enum_defs,
                    struct_defs: &struct_defs,
                    type_params: &type_param_names,
                    interface_names: &interface_names,
                }) else {
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
            name,
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
        diags: Vec::new(),
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
        workbook_enabled: false,
        expr_watchdog_every: Checker::expr_watchdog_every_default(),
        expr_watchdog_counter: 0,
        expr_timeout: Checker::expr_timeout_default(options),
        expr_timeout_reported: false,
        check_started_at: Instant::now(),
        expr_hotspots: HashMap::new(),
        expr_hotspot_summary_emitted: false,
        span_log: Vec::new(),
        capture_target: None,
        captured: None,
        current_filtering_state: hir::FilteringState::Auto,
        effect_defs: HashMap::new(),
        global_uniforms: global_uniform_registry,
        runtime_channel_cache: globals::RuntimeChannelCache::default(),
        evaluation_context: None,
    };

    bind_top_level_consts(&mut c, consts);
    c.bind_global_uniforms();
    c.declare_global_params(params, structs);

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
    (c, prep_diags)
}

/// Validate required signatures too: an unused contract must not hide unknown types.
pub(crate) fn validate_style_contract_signatures(
    program: &Program,
    options: &CheckOptions,
) -> Result<(), Vec<Diag>> {
    if program.style_contracts.is_empty() {
        return Ok(());
    }
    let (checker, mut diags) = create_checker_for_surface(
        "style_contract_signatures".into(),
        &program.functions,
        &program.consts,
        &program.enums,
        &program.structs,
        &program.params,
        &program.texture_types,
        &program.interfaces,
        &[],
        &program.effects,
        options,
    );
    let interface_names = program.interfaces.iter().map(|i| i.name.clone()).collect();
    for contract in &program.style_contracts {
        for hook in &contract.hooks {
            let mut names = HashSet::new();
            for param in &hook.signature.params {
                if !names.insert(&param.name) {
                    diags.push(
                        Diag::error(param.name_span.clone(), "duplicate contract hook parameter")
                            .with_file(&contract.source_file),
                    );
                }
            }
            for (name, span) in hook
                .signature
                .params
                .iter()
                .map(|p| (&p.ty_name, &p.ty_span))
                .chain(
                    hook.signature
                        .ret_ty
                        .iter()
                        .map(|(name, span)| (name, span)),
                )
            {
                parse_fn_type(FnTypeInputs {
                    name,
                    span,
                    diags: &mut diags,
                    context: "shading contract hook",
                    source_file: Some(&contract.source_file),
                    enum_defs: &checker.enum_defs,
                    struct_defs: &checker.struct_defs,
                    type_params: &[],
                    interface_names: &interface_names,
                });
            }
        }
    }
    if diags.iter().any(|d| d.severity == Severity::Error) {
        Err(diags)
    } else {
        Ok(())
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "This compiler boundary explicitly threads independent checking or lowering inputs."
)]
pub(crate) fn validate_conformance_method_bodies(
    functions: &[FnDecl],
    consts: &[ConstDecl],
    enums: &[EnumDecl],
    structs: &[StructDecl],
    params: &[GlobalParamDecl],
    texture_types: &[TextureTypeDecl],
    interfaces: &[InterfaceDecl],
    conformances: &[ConformanceDecl],
    effects: &[EffectDecl],
    options: &CheckOptions,
) -> Result<(), Vec<Diag>> {
    fn enum_variants_in_decl_order(checker: &Checker, enum_name: &str) -> Option<Vec<String>> {
        let enum_def = checker.enum_defs.get(enum_name)?;
        let mut variants = enum_def
            .variants
            .iter()
            .map(|(name, span)| (name.clone(), span.start))
            .collect::<Vec<_>>();
        variants.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        Some(variants.into_iter().map(|(name, _)| name).collect())
    }

    fn conformance_return_value_matches(
        checker: &mut Checker,
        value: &Value,
        ret_ty: &FnValueTy,
        ret_ty_span: &Span,
        method_name: &str,
        conform_type_name: &str,
    ) -> bool {
        match (ret_ty, value) {
            (FnValueTy::Scalar, Value::Scalar(_))
            | (FnValueTy::Scalar, Value::Distance(_))
            | (FnValueTy::Scalar, Value::Coverage(_))
            | (FnValueTy::Scalar, Value::Mask(_))
            | (FnValueTy::Vec2, Value::Vec2(_))
            | (FnValueTy::Vec3, Value::Vec3(_))
            | (FnValueTy::Vec4, Value::Vec4(_))
            | (FnValueTy::Mat2, Value::Mat2(_))
            | (FnValueTy::Mat3, Value::Mat3(_))
            | (FnValueTy::Mat4, Value::Mat4(_))
            | (FnValueTy::CoordLike, Value::Vec2(_))
            | (FnValueTy::Texture, Value::TypedTextureSample { .. })
            | (FnValueTy::Color, Value::Color { .. })
            | (FnValueTy::Color, Value::ColorField { .. })
            | (FnValueTy::Shape, Value::Shape(_))
            | (FnValueTy::Layer, Value::Layer(_))
            | (FnValueTy::Callable { .. }, Value::FnRef(_))
            | (FnValueTy::Callable { .. }, Value::Lambda { .. })
            | (FnValueTy::TypeVar(_), _) => true,
            (FnValueTy::Array(expected), Value::Array(_)) => local_decl_type_matches(
                value,
                expected,
                &checker.enum_defs,
                &checker.struct_defs,
                &[],
            ),
            (FnValueTy::Struct(expected), Value::Struct { ty_name, .. }) if ty_name == expected => {
                true
            }
            (FnValueTy::Enum(enum_name), enum_value) => {
                let Some((sx, _kind)) = Checker::as_numeric_scalar(enum_value) else {
                    return false;
                };

                if let Some(ord) = Checker::try_eval_static_scalar(&sx) {
                    if ord.is_finite() {
                        let rounded = ord.round();
                        if (rounded - ord).abs() <= 1e-6 && rounded >= 0.0 {
                            let idx = rounded as usize;
                            if enum_variants_in_decl_order(checker, enum_name)
                                .and_then(|variants| variants.get(idx).cloned())
                                .is_some()
                            {
                                return true;
                            }
                        }
                    }

                    let variants = enum_variants_in_decl_order(checker, enum_name)
                        .unwrap_or_default()
                        .join(", ");
                    let help = if variants.is_empty() {
                        format!("use a declared `{enum_name}` variant")
                    } else {
                        format!("use `{enum_name}.<variant>` where <variant> is one of: {variants}")
                    };
                    checker.diags.push(
                        Diag::error(
                            ret_ty_span.clone(),
                            format!(
                                "conformance method `{method_name}` for `{conform_type_name}` returns an invalid `{enum_name}` variant value"
                            ),
                        )
                        .with_label("invalid enum variant value")
                        .with_help(help),
                    );
                    return false;
                }

                true
            }
            _ => false,
        }
    }

    fn placeholder_value_for_type(
        checker: &Checker,
        ty_name: &str,
        symbol_seed: &str,
        struct_stack: &mut HashSet<String>,
        type_param_names: &HashSet<String>,
    ) -> Option<Value> {
        let normalized = super::strip_spatial_type_suffix(ty_name);
        if type_param_names.contains(normalized) {
            return Some(Value::Scalar(Sx::Var(symbol_seed.to_string())));
        }

        if let Some((base, args)) = try_parse_type_application(ty_name) {
            if base == "field" && args.len() == 1 {
                return placeholder_value_for_type(
                    checker,
                    &args[0],
                    symbol_seed,
                    struct_stack,
                    type_param_names,
                );
            }

            if base == "texture" {
                let tex_name = args
                    .first()
                    .cloned()
                    .unwrap_or_else(|| symbol_seed.to_string());
                return Some(Value::TypedTextureSample {
                    layer_id: 0,
                    tex_name,
                    sample_at: None,
                });
            }

            if base == "array" && args.len() == 1 {
                let elem_type = match super::strip_spatial_type_suffix(&args[0]) {
                    "f32" | "f64" | "half" => hir::ArrayElemType::F32,
                    "i32" => hir::ArrayElemType::I32,
                    "u32" => hir::ArrayElemType::U32,
                    "bool" => hir::ArrayElemType::Bool,
                    "vec2" | "coord" | "coord_like" | "resolution" => hir::ArrayElemType::Vec2,
                    "vec3" => hir::ArrayElemType::Vec3,
                    "vec4" => hir::ArrayElemType::Vec4,
                    "color" => hir::ArrayElemType::Color,
                    "mat2" => hir::ArrayElemType::Mat2,
                    "mat3" => hir::ArrayElemType::Mat3,
                    "mat4" => hir::ArrayElemType::Mat4,
                    _ => return None,
                };

                return Some(Value::DynamicArray {
                    param_name: symbol_seed.to_string(),
                    elem_type,
                });
            }
        }

        if try_parse_callable_ty(ty_name).is_some() {
            return Some(Value::FnRef(symbol_seed.to_string()));
        }
        match normalized {
            "i32" | "u32" | "bool" => Some(Value::Scalar(crate::typed_scalar::Scalar::input(
                Sx::Var(symbol_seed.to_string()),
                crate::typed_scalar::Kind::parse(normalized).expect("concrete scalar type"),
            ))),
            "f32" | "f64" | "half" | "angle" | "length" | "signal" | "delta" | "mask"
            | "coverage" => Some(Value::Scalar(Sx::Var(symbol_seed.to_string()))),
            "vec2" | "coord" | "coord_like" | "resolution" => Some(Value::Vec2((
                Sx::Var(format!("{symbol_seed}_x")),
                Sx::Var(format!("{symbol_seed}_y")),
            ))),
            "vec3" => Some(Value::Vec3((
                Sx::Var(format!("{symbol_seed}_x")),
                Sx::Var(format!("{symbol_seed}_y")),
                Sx::Var(format!("{symbol_seed}_z")),
            ))),
            "vec4" => Some(Value::Vec4((
                Sx::Var(format!("{symbol_seed}_x")),
                Sx::Var(format!("{symbol_seed}_y")),
                Sx::Var(format!("{symbol_seed}_z")),
                Sx::Var(format!("{symbol_seed}_w")),
            ))),
            "color" => Some(Value::ColorField {
                rgba: [
                    Sx::Var(format!("{symbol_seed}_r")),
                    Sx::Var(format!("{symbol_seed}_g")),
                    Sx::Var(format!("{symbol_seed}_b")),
                    Sx::Var(format!("{symbol_seed}_a")),
                ],
                space: ColorSpace::Linear,
            }),
            "shape" => Some(Value::Shape(0)),
            "layer" => Some(Value::Layer(0)),
            "space" => Some(Value::Space(Vec::new())),
            "mat2" | "mat3" | "mat4" => Some(Value::Error),
            other => {
                if checker.enum_defs.contains_key(other) {
                    return Some(Value::Scalar(Sx::Lit(0.0)));
                }

                let struct_def = checker.struct_defs.get(other)?;

                if !struct_stack.insert(other.to_string()) {
                    return None;
                }

                let mut fields = HashMap::new();
                for (field_name, field_def) in &struct_def.fields {
                    let field_seed = format!("{symbol_seed}_{field_name}");
                    let Some(field_value) = placeholder_value_for_type(
                        checker,
                        &field_def.ty_name,
                        &field_seed,
                        struct_stack,
                        type_param_names,
                    ) else {
                        struct_stack.remove(other);
                        return None;
                    };
                    fields.insert(field_name.clone(), field_value);
                }

                struct_stack.remove(other);
                Some(Value::Struct {
                    ty_name: other.to_string(),
                    fields,
                })
            }
        }
    }

    let mut all_diags = Vec::new();
    let interface_names: HashSet<String> =
        interfaces.iter().map(|iface| iface.name.clone()).collect();

    let no_conformances: &[ConformanceDecl] = &[];
    for conform in conformances {
        if conform.body_checked_with_instance {
            continue;
        }
        for method in &conform.methods {
            let checker_name = format!(
                "conformance_method_{}_{}_{}",
                conform.type_name, conform.interface_name, method.name
            );
            let (mut checker, prep_diags) = create_checker_for_surface(
                checker_name,
                functions,
                consts,
                enums,
                structs,
                params,
                texture_types,
                interfaces,
                no_conformances,
                effects,
                options,
            );
            all_diags.extend(prep_diags);
            let locate = |mut diag: Diag| {
                if diag.file.is_none() && !method.source_file.is_empty() {
                    diag.file = Some(method.source_file.clone());
                }
                diag
            };

            let method_type_param_names = method
                .type_params
                .iter()
                .map(|param| param.name.clone())
                .collect::<HashSet<_>>();

            for param in &method.params {
                let mut struct_stack = HashSet::new();
                let symbol_seed = format!("{}_{}", method.name, param.name);
                let Some(value) = placeholder_value_for_type(
                    &checker,
                    &param.ty_name,
                    &symbol_seed,
                    &mut struct_stack,
                    &method_type_param_names,
                ) else {
                    checker.diags.push(
                        Diag::error(
                            param.ty_span.clone(),
                            format!(
                                "conformance method parameter `{}.{}` uses unsupported type `{}` for body type-checking",
                                method.name, param.name, param.ty_name
                            ),
                        )
                        .with_help(
                            "use scalar/vector/color/struct/surf-compatible parameter types, or extend placeholder binding support",
                        ),
                    );
                    continue;
                };
                checker.bind(param.name.clone(), value);
            }

            let result = checker.eval_standalone_stmt_block(
                &format!(
                    "conformance method `{}::{}` for `{}`",
                    conform.interface_name, method.name, conform.type_name
                ),
                &method.source_file,
                &method.span,
                &method.body,
            );

            if let Some((ret_ty_name, ret_ty_span)) = &method.ret_ty
                && let Some(result_value) = result
            {
                let type_param_names = method
                    .type_params
                    .iter()
                    .map(|param| param.name.clone())
                    .collect::<Vec<_>>();
                let Some(ret_ty) = parse_fn_type(FnTypeInputs {
                    name: ret_ty_name,
                    span: ret_ty_span,
                    diags: &mut checker.diags,
                    context: "method return type",
                    source_file: Some(&method.source_file),
                    enum_defs: &checker.enum_defs,
                    struct_defs: &checker.struct_defs,
                    type_params: &type_param_names,
                    interface_names: &interface_names,
                }) else {
                    all_diags.extend(checker.diags.into_iter().map(locate));
                    continue;
                };

                if !conformance_return_value_matches(
                    &mut checker,
                    &result_value,
                    &ret_ty,
                    ret_ty_span,
                    &method.name,
                    &conform.type_name,
                ) {
                    if matches!(ret_ty, FnValueTy::Enum(_))
                        && Checker::as_numeric_scalar(&result_value)
                            .and_then(|(sx, _)| Checker::try_eval_static_scalar(&sx))
                            .is_some()
                    {
                        all_diags.extend(checker.diags.into_iter().map(locate));
                        continue;
                    }
                    checker.diags.push(
                        Diag::error(
                            ret_ty_span.clone(),
                            format!(
                                "conformance method `{}` for `{}` returns {}, expected `{}`",
                                method.name,
                                conform.type_name,
                                result_value.kind(),
                                ret_ty_name
                            ),
                        )
                        .with_help(
                            "adjust the method return expression or declared return type to match the interface contract",
                        ),
                    );
                }
            }

            all_diags.extend(checker.diags.into_iter().map(locate));
        }
    }

    if all_diags
        .iter()
        .any(|diag| diag.severity == Severity::Error)
    {
        Err(all_diags)
    } else {
        Ok(())
    }
}

pub fn validate_vertex_contracts(
    vertex_interfaces: &[VertexInterfaceDecl],
    vertex_formats: &[VertexFormatDecl],
    vertex_factories: &[VertexFactoryDecl],
) -> Vec<Diag> {
    let mut diags = Vec::new();

    let mut vertex_interface_defs = HashMap::new();
    for iface in vertex_interfaces {
        if vertex_interface_defs.contains_key(&iface.name) {
            diags.push(
                Diag::error(
                    iface.name_span.clone(),
                    format!("duplicate vertex_interface declaration `{}`", iface.name),
                )
                .with_help("vertex interface names must be unique"),
            );
            continue;
        }
        vertex_interface_defs.insert(
            iface.name.clone(),
            VertexInterfaceDef {
                members: iface
                    .members
                    .iter()
                    .map(|member| VertexContractMemberDef {
                        name: member.name.clone(),
                        ty_name: member.ty_name.clone(),
                        optional: member.optional,
                        default: member.default.clone(),
                        span: member.span.clone(),
                    })
                    .collect(),
            },
        );
    }

    let mut vertex_format_defs = HashMap::new();
    for format_decl in vertex_formats {
        if vertex_format_defs.contains_key(&format_decl.name) {
            diags.push(
                Diag::error(
                    format_decl.name_span.clone(),
                    format!("duplicate vertex_format declaration `{}`", format_decl.name),
                )
                .with_help("vertex format names must be unique"),
            );
            continue;
        }
        vertex_format_defs.insert(
            format_decl.name.clone(),
            VertexFormatDef {
                parent: format_decl.parent.clone(),
                members: format_decl
                    .members
                    .iter()
                    .map(|member| VertexContractMemberDef {
                        name: member.name.clone(),
                        ty_name: member.ty_name.clone(),
                        optional: member.optional,
                        default: member.default.clone(),
                        span: member.span.clone(),
                    })
                    .collect(),
            },
        );
    }

    for format_decl in vertex_formats {
        if let Some(parent) = &format_decl.parent
            && !vertex_format_defs.contains_key(parent)
        {
            diags.push(
                Diag::error(
                    format_decl
                        .parent_span
                        .clone()
                        .unwrap_or_else(|| format_decl.span.clone()),
                    format!(
                        "vertex_format `{}` extends unknown vertex_format `{parent}`",
                        format_decl.name
                    ),
                )
                .with_help("declare the parent vertex_format before extending it"),
            );
        }
    }

    let mut vertex_factory_defs = HashMap::new();
    for factory in vertex_factories {
        if vertex_factory_defs.contains_key(&factory.name) {
            diags.push(
                Diag::error(
                    factory.name_span.clone(),
                    format!("duplicate vertex_factory declaration `{}`", factory.name),
                )
                .with_help("vertex factory names must be unique"),
            );
            continue;
        }
        vertex_factory_defs.insert(
            factory.name.clone(),
            VertexFactoryDef {
                target_format: factory.target_format.clone(),
                target_format_span: factory.target_format_span.clone(),
            },
        );
    }

    for factory in vertex_factories {
        if !vertex_format_defs.contains_key(&factory.target_format) {
            diags.push(
                Diag::error(
                    factory.target_format_span.clone(),
                    format!(
                        "vertex factory `{}` targets unknown vertex_format `{}`",
                        factory.name, factory.target_format
                    ),
                )
                .with_help(
                    "declare the referenced vertex_format before using it in a vertex_factory",
                ),
            );
        }
    }

    for iface in &vertex_interface_defs {
        let interface_name = iface.0;
        let interface_def = iface.1;
        for interface_member in &interface_def.members {
            let interface_member_key = type_name_to_key(&interface_member.ty_name);
            for format in &vertex_format_defs {
                let format_name = format.0;
                if let Some(resolved_members) =
                    resolve_vertex_format_members(&vertex_format_defs, format_name)
                {
                    let has_member = resolved_members
                        .iter()
                        .any(|member| member.name == interface_member.name);
                    if !has_member {
                        continue;
                    }
                    if let Some(format_member) = resolved_members
                        .iter()
                        .find(|member| member.name == interface_member.name)
                    {
                        let format_member_key = type_name_to_key(&format_member.ty_name);
                        if interface_member_key != format_member_key {
                            diags.push(
                                Diag::error(
                                    format_member.span.clone(),
                                    format!(
                                        "vertex_format `{format_name}` member `{}` does not match vertex_interface `{interface_name}` member `{}`",
                                        format_member.name, interface_member.name,
                                    ),
                                )
                                .with_help("use the same type name for the matching member"),
                            );
                        }
                        if format_member.optional && format_member.default.is_none() {
                            diags.push(
                                Diag::error(
                                    format_member.span.clone(),
                                    format!(
                                        "vertex_format `{format_name}` member `{}` is optional but has no default, so it cannot satisfy vertex_interface `{interface_name}`",
                                        format_member.name,
                                    ),
                                )
                                .with_help("provide a declared default for optional members that satisfy an interface"),
                            );
                        }
                    }
                }
            }
        }
    }

    for (format_name, format_def) in &vertex_format_defs {
        for iface in &vertex_interface_defs {
            let interface_name = iface.0;
            let interface_def = iface.1;
            let Some(resolved_members) =
                resolve_vertex_format_members(&vertex_format_defs, format_name)
            else {
                continue;
            };
            let mut missing_members = Vec::new();
            for interface_member in &interface_def.members {
                let Some(format_member) = resolved_members
                    .iter()
                    .find(|member| member.name == interface_member.name)
                else {
                    missing_members.push(interface_member.name.clone());
                    continue;
                };
                if type_name_to_key(&format_member.ty_name)
                    != type_name_to_key(&interface_member.ty_name)
                {
                    missing_members.push(interface_member.name.clone());
                }
            }
            if !missing_members.is_empty() {
                diags.push(
                    Diag::error(
                        format_def
                            .members
                            .first()
                            .map(|member| member.span.clone())
                            .unwrap_or_else(|| 0..format_name.len()),
                        format!(
                            "vertex_format `{format_name}` does not satisfy vertex_interface `{interface_name}`; missing members: {}",
                            missing_members.join(", ")
                        ),
                    )
                    .with_help("add matching members with compatible types, or make the format member optional with a default"),
                );
            }
        }
    }

    diags
}

#[expect(
    clippy::too_many_arguments,
    reason = "This compiler boundary explicitly threads independent checking or lowering inputs."
)]
pub fn check(
    entry: &NormalizedRootEntry,
    functions: &[FnDecl],
    consts: &[ConstDecl],
    enums: &[EnumDecl],
    structs: &[StructDecl],
    params: &[GlobalParamDecl],
    texture_types: &[TextureTypeDecl],
    interfaces: &[InterfaceDecl],
    conformances: &[ConformanceDecl],
    effects: &[EffectDecl],
    vertex_interfaces: &[VertexInterfaceDecl],
    vertex_formats: &[VertexFormatDecl],
    vertex_factories: &[VertexFactoryDecl],
    options: &CheckOptions,
) -> Result<(Hir, Vec<Diag>), Vec<Diag>> {
    let _span = info_span!(
        "check.entry",
        entry = %entry.name,
        fn_count = functions.len(),
        enum_count = enums.len()
    )
    .entered();
    info!("checker phase start: validate_entry_template_params");
    let mut prep_diags = Vec::new();
    let contract = if matches!(entry.kind, RootEntryKind::Canvas) {
        crate::context::raster_contract(&entry.entry_kind, interfaces, structs)?
    } else {
        None
    };
    if contract.is_none() {
        validate_normalized_root_entry_params(entry, &mut prep_diags);
    }
    info!(
        prep_diags = prep_diags.len(),
        "checker phase done: validate_normalized_root_entry_params"
    );
    let mut enum_defs = HashMap::new();
    let mut struct_defs = HashMap::new();

    // Load stdlib enums first
    info!("checker phase start: load_stdlib_enums");
    let stdlib_enum_start = Instant::now();
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
    info!(
        ms = stdlib_enum_start.elapsed().as_secs_f64() * 1000.0,
        stdlib_enum_defs = enum_defs.len(),
        "checker phase done: load_stdlib_enums"
    );

    // Then process user-defined enums
    info!(
        user_enums = enums.len(),
        "checker phase start: register_user_enums"
    );
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
    info!(
        total_enum_defs = enum_defs.len(),
        prep_diags = prep_diags.len(),
        "checker phase done: register_user_enums"
    );

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

    prep_diags.extend(validate_vertex_contracts(
        vertex_interfaces,
        vertex_formats,
        vertex_factories,
    ));

    // Then process user-defined functions
    info!(
        functions = functions.len(),
        "checker phase start: register_functions"
    );
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
            parse_fn_type(FnTypeInputs {
                name,
                span,
                diags: &mut prep_diags,
                context: "return type",
                source_file: Some(func.source_file.as_str()),
                enum_defs: &enum_defs,
                struct_defs: &struct_defs,
                type_params: &type_param_names,
                interface_names: &interface_names,
            })
        }) else {
            prep_diags.push(
                Diag::error(
                    func.name_span.clone(),
                    format!("function `{}` must declare a return type", func.name),
                )
                .with_file(func.source_file.clone())
                .with_help(
                    "supported types: f32, f64, half, i32, u32, bool, angle, length, vec2, coord, coord_like, resolution, vec3, vec4, mat2, mat3, mat4, color, shape, layer, signal, delta, mask, coverage, or a defined enum",
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
                let Some(ty) = parse_fn_type(FnTypeInputs {
                    name: &ty_name,
                    span: &p.ty_span,
                    diags: &mut prep_diags,
                    context: "parameter type",
                    source_file: Some(func.source_file.as_str()),
                    enum_defs: &enum_defs,
                    struct_defs: &struct_defs,
                    type_params: &type_param_names,
                    interface_names: &interface_names,
                }) else {
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
    let fn_overloads = fn_defs.values().map(Vec::len).sum::<usize>();
    info!(
        fn_names = fn_defs.len(),
        fn_overloads = fn_overloads,
        prep_diags = prep_diags.len(),
        "checker phase done: register_functions"
    );

    info!("checker phase start: init_checker");
    let global_uniform_registry =
        globals::build_global_uniform_registry(params, structs, &mut prep_diags);
    let mut c = Checker {
        hir: Hir {
            name: entry.name.clone(),
            entry_context: None,
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
        workbook_enabled: false,
        expr_watchdog_every: Checker::expr_watchdog_every_default(),
        expr_watchdog_counter: 0,
        expr_timeout: Checker::expr_timeout_default(options),
        expr_timeout_reported: false,
        check_started_at: Instant::now(),
        expr_hotspots: HashMap::new(),
        expr_hotspot_summary_emitted: false,
        span_log: Vec::new(),
        capture_target: None,
        captured: None,
        current_filtering_state: hir::FilteringState::Auto,
        effect_defs: HashMap::new(),
        global_uniforms: global_uniform_registry,
        runtime_channel_cache: globals::RuntimeChannelCache::default(),
        evaluation_context: None,
    };
    bind_top_level_consts(&mut c, consts);
    c.bind_global_uniforms();
    c.declare_global_params(params, structs);
    info!("checker phase done: init_checker");

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

    // Register and type-check user-defined effects.
    info!(
        effects = effects.len(),
        "checker phase start: check_effects"
    );
    c.check_effect_decls(effects);
    info!("checker phase done: check_effects");

    info!(
        params = entry.params.len(),
        "checker phase start: bind_entry_params"
    );
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
    info!("checker phase done: bind_entry_params");

    info!("checker phase start: validate_function_overloads");
    c.validate_registered_function_overloads();
    info!("checker phase done: validate_function_overloads");

    info!(
        stmts = entry.body.len(),
        "checker phase start: eval_entry_body"
    );
    let eval_start = Instant::now();
    let root = c.eval_block(&entry.body, &entry.name_span, true, false);
    c.emit_path_channel_demand_note();
    c.materialize_pending_user_helpers();
    c.validate_temporal_purity_requirements();
    c.validate_gradient_color_anchors(&entry.name_span);
    info!(
        ms = eval_start.elapsed().as_secs_f64() * 1000.0,
        has_root = root.is_some(),
        diags = c.diags.len(),
        "checker phase done: eval_entry_body"
    );
    if profile_timing_notes_enabled() {
        c.emit_check_profile_timing_note();
    }
    c.emit_expr_hotspot_summary();
    match root {
        Some(root) if !c.diags.iter().any(|d| d.severity == Severity::Error) => {
            c.hir.root = root;
            info!(
                shapes = c.hir.shapes.len(),
                layers = c.hir.layers.len(),
                diags = c.diags.len(),
                "checker entry success"
            );
            Ok((c.hir, c.diags))
        }
        _ => {
            if !c.diags.iter().any(|d| d.severity == Severity::Error) {
                c.diags.push(
                    Diag::error(entry.name_span.clone(), "entry body produces no layer").with_help(
                        "end the entry with a `compose { ... }` block or a layer expression",
                    ),
                );
            }
            info!(diags = c.diags.len(), "checker entry failed");
            Err(c.diags)
        }
    }
}

impl Checker {
    fn validate_gradient_color_anchors(&mut self, span: &Span) {
        for helper in self.hir.user_helpers.values() {
            let mut requires_shape = false;
            for stmt in &helper.body_stmts {
                stmt.walk_sx(&mut |expr| requires_shape |= expr.requires_shape_color_frame());
            }
            if requires_shape {
                self.diags.push(Diag::error(span.clone(), "`anchor: shape` requires a shape receiver")
                    .with_help("sample shape-anchored colors at the shape receiver, or use `anchor: scene` in scalar helper functions"));
            }
        }
        for (id, layer) in self.hir.layers.iter().enumerate() {
            if self.hir.color_shape(id).is_some() {
                continue;
            }
            let mut requires_shape = false;
            layer.for_each_color_sx(|expr| requires_shape |= expr.requires_shape_color_frame());
            if requires_shape {
                self.diags.push(Diag::error(span.clone(), "`anchor: shape` requires a shape receiver")
                    .with_help("apply the color to a shape or direct shape fill, or use `anchor: scene`"));
            }
        }
    }

    fn validate_temporal_purity_requirements(&mut self) {
        struct TemporalPurityRequirement {
            owner_layer: LayerId,
            root: LayerId,
            owner_span: Span,
            owner_name: &'static str,
        }

        enum HistoryBoundLayerLeaf {
            FeedbackTexture { tex_name: String },
            RememberStateTexture { tex_name: String },
        }

        impl HistoryBoundLayerLeaf {
            fn display_name(&self) -> String {
                match self {
                    HistoryBoundLayerLeaf::FeedbackTexture { tex_name }
                    | HistoryBoundLayerLeaf::RememberStateTexture { tex_name } => tex_name.clone(),
                }
            }
        }

        fn classify_history_bound_image(tex_name: &str) -> Option<HistoryBoundLayerLeaf> {
            if tex_name == "previous_frame" || tex_name.starts_with("previous_frame_") {
                return Some(HistoryBoundLayerLeaf::FeedbackTexture {
                    tex_name: tex_name.to_string(),
                });
            }
            if tex_name.starts_with("remember_") {
                return Some(HistoryBoundLayerLeaf::RememberStateTexture {
                    tex_name: tex_name.to_string(),
                });
            }
            None
        }

        fn first_history_bound_scalar_name(sx: &Sx) -> Option<String> {
            let mut found = None;
            sx.walk_preorder(&mut |node| {
                if found.is_none() {
                    found = node.temporal_history_source_name().map(str::to_string);
                }
            });
            found
        }

        fn first_history_bound_v2_name(v: &V2) -> Option<String> {
            first_history_bound_scalar_name(&v.0).or_else(|| first_history_bound_scalar_name(&v.1))
        }

        fn first_history_bound_color_name(c: &[Sx; 4]) -> Option<String> {
            c.iter().find_map(first_history_bound_scalar_name)
        }

        fn first_history_bound_gradient_kind_name(kind: &GradientKind) -> Option<String> {
            match kind {
                GradientKind::Linear { along, .. } => first_history_bound_v2_name(along),
                GradientKind::Radial { center, radius } => first_history_bound_v2_name(center)
                    .or_else(|| first_history_bound_scalar_name(radius)),
            }
        }

        fn first_history_bound_color_source_name(source: &ColorSource) -> Option<String> {
            match source {
                ColorSource::Solid(c) => first_history_bound_color_name(c),
                ColorSource::Gradient { kind, stops } => {
                    first_history_bound_gradient_kind_name(kind).or_else(|| {
                        stops.iter().find_map(|s| {
                            first_history_bound_scalar_name(&s.at)
                                .or_else(|| first_history_bound_color_name(&s.color))
                        })
                    })
                }
            }
        }

        fn first_history_bound_glow_reach_name(reach: &GlowReach) -> Option<String> {
            match reach {
                GlowReach::Scalar(sx) => first_history_bound_scalar_name(sx),
                GlowReach::Vec2(v) => first_history_bound_v2_name(v),
            }
        }

        fn classify_history_bound_scalar_in_layer(layer: &Layer) -> Option<String> {
            match layer {
                Layer::FillExpr { r, g, b, a, .. } | Layer::ColorExpr { r, g, b, a } => {
                    [r, g, b, a]
                        .iter()
                        .find_map(|sx| first_history_bound_scalar_name(sx))
                }
                Layer::Shadow {
                    off, soften, color, ..
                } => first_history_bound_v2_name(off)
                    .or_else(|| first_history_bound_scalar_name(soften))
                    .or_else(|| first_history_bound_color_name(color)),
                Layer::Glow {
                    reach,
                    strength,
                    color,
                    ..
                }
                | Layer::InnerGlow {
                    reach,
                    strength,
                    color,
                    ..
                }
                | Layer::GlowFx {
                    reach,
                    strength,
                    color,
                    ..
                } => first_history_bound_glow_reach_name(reach)
                    .or_else(|| first_history_bound_scalar_name(strength))
                    .or_else(|| first_history_bound_color_source_name(color)),
                Layer::Bevel {
                    width,
                    light,
                    strength,
                    highlight,
                    shadow,
                    ..
                } => first_history_bound_scalar_name(width)
                    .or_else(|| first_history_bound_v2_name(light))
                    .or_else(|| first_history_bound_scalar_name(strength))
                    .or_else(|| first_history_bound_color_name(highlight))
                    .or_else(|| first_history_bound_color_name(shadow)),
                Layer::Soften { radius, color, .. } => first_history_bound_scalar_name(radius)
                    .or_else(|| first_history_bound_color_name(color)),
                Layer::Blur { radius, .. } => first_history_bound_scalar_name(radius),
                Layer::MotionBlur {
                    shutter, offset, ..
                } => first_history_bound_scalar_name(shutter)
                    .or_else(|| first_history_bound_v2_name(offset)),
                Layer::Opacity { alpha, .. } => first_history_bound_scalar_name(alpha),
                Layer::Grey { value } => first_history_bound_scalar_name(value),
                Layer::Tint { color, amount, .. } => first_history_bound_color_name(color)
                    .or_else(|| first_history_bound_scalar_name(amount)),
                Layer::PostProcess { rgba, .. } => first_history_bound_color_name(rgba),
                Layer::ScatterBins {
                    min,
                    max,
                    lifecycle,
                    ..
                } => first_history_bound_v2_name(min)
                    .or_else(|| first_history_bound_v2_name(max))
                    .or_else(|| {
                        lifecycle.as_ref().and_then(|p| {
                            first_history_bound_scalar_name(&p.lifetime)
                                .or_else(|| first_history_bound_scalar_name(&p.respawn_every))
                        })
                    }),
                Layer::If { cond, .. } => first_history_bound_scalar_name(cond),
                Layer::Solid(_)
                | Layer::Fill { .. }
                | Layer::FillGradient { .. }
                | Layer::Image { .. }
                | Layer::ImageAt { .. }
                | Layer::InSpace { .. }
                | Layer::Compose(_)
                | Layer::UserEffect { .. } => None,
            }
        }

        fn layer_inputs(layer: &Layer) -> Vec<usize> {
            match layer {
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
                | Layer::ImageAt { .. } => Vec::new(),
                Layer::Blur { inner, .. }
                | Layer::MotionBlur { inner, .. }
                | Layer::GlowFx { inner, .. }
                | Layer::Opacity { inner, .. }
                | Layer::Tint { inner, .. }
                | Layer::PostProcess { inner, .. }
                | Layer::InSpace { inner, .. } => vec![*inner],
                Layer::If {
                    then_layer,
                    else_layer,
                    ..
                } => vec![*then_layer, *else_layer],
                Layer::Compose(entries) => entries.iter().map(|(id, _)| *id).collect(),
                Layer::ScatterBins { body, .. } => vec![*body],
                Layer::UserEffect { inner, .. } => inner.map(|id| vec![id]).unwrap_or_default(),
            }
        }

        fn collect_temporal_purity_requirements(
            layers: &[Layer],
        ) -> Vec<TemporalPurityRequirement> {
            let mut requirements = Vec::new();
            for (id, layer) in layers.iter().enumerate() {
                if let Some(req) = layer.temporal_purity_requirement() {
                    requirements.push(TemporalPurityRequirement {
                        owner_layer: id,
                        root: req.root,
                        owner_span: req.span.clone(),
                        owner_name: req.owner_name,
                    });
                }
            }
            requirements
        }

        fn find_history_bound_leaf(
            layers: &[Layer],
            root: LayerId,
        ) -> Option<(LayerId, String, String)> {
            let mut stack = vec![(root, vec![root])];
            let mut seen: HashSet<LayerId> = HashSet::new();
            while let Some((id, path)) = stack.pop() {
                if !seen.insert(id) {
                    continue;
                }
                match &layers[id] {
                    Layer::Image { tex_name } | Layer::ImageAt { tex_name, .. } => {
                        let Some(leaf) = classify_history_bound_image(tex_name) else {
                            if let Some(leaf_name) =
                                classify_history_bound_scalar_in_layer(&layers[id])
                            {
                                let path_text = path
                                    .into_iter()
                                    .map(|p| format!("layer#{p}"))
                                    .collect::<Vec<_>>()
                                    .join(" -> ");
                                return Some((id, leaf_name, path_text));
                            }
                            continue;
                        };
                        let path_text = path
                            .into_iter()
                            .map(|p| format!("layer#{p}"))
                            .collect::<Vec<_>>()
                            .join(" -> ");
                        return Some((id, leaf.display_name(), path_text));
                    }
                    _ => {
                        if let Some(leaf_name) = classify_history_bound_scalar_in_layer(&layers[id])
                        {
                            let path_text = path
                                .into_iter()
                                .map(|p| format!("layer#{p}"))
                                .collect::<Vec<_>>()
                                .join(" -> ");
                            return Some((id, leaf_name, path_text));
                        }
                    }
                }
                for child in layer_inputs(&layers[id]) {
                    let mut child_path = path.clone();
                    child_path.push(child);
                    stack.push((child, child_path));
                }
            }
            None
        }

        for requirement in collect_temporal_purity_requirements(&self.hir.layers) {
            if let Some((leaf_id, leaf_name, path)) =
                find_history_bound_leaf(&self.hir.layers, requirement.root)
            {
                self.diags.push(
                    Diag::error(
                        requirement.owner_span,
                        format!(
                            "`{}` (layer#{}) requires temporal-pure inputs and cannot contain history-bound leaf `{leaf_name}` (found at layer#{leaf_id})",
                            requirement.owner_name,
                            requirement.owner_layer,
                        ),
                    )
                    .with_label("temporal integration requires time-shift-safe scene evaluation")
                    .with_help(format!(
                        "offending path: {path}; move `{}` inside the feedback/previous-frame loop instead of wrapping it",
                        requirement.owner_name,
                    )),
                );
            }
        }
    }

    fn emit_check_profile_timing_note(&mut self) {
        let t = self.check_profile_timing;
        let entries = [
            ("eval_expr", t.eval_expr_total),
            ("eval_expr_num", t.eval_expr_num_total),
            ("eval_expr_color", t.eval_expr_color_total),
            ("eval_expr_var", t.eval_expr_var_total),
            ("eval_expr_vec2", t.eval_expr_vec2_total),
            ("eval_expr_range", t.eval_expr_range_total),
            ("eval_expr_array", t.eval_expr_array_total),
            ("eval_expr_field", t.eval_expr_field_total),
            ("eval_expr_layer", t.eval_expr_layer_total),
            ("eval_expr_unary", t.eval_expr_unary_total),
            ("eval_expr_binary", t.eval_expr_binary_total),
            ("eval_expr_call", t.eval_expr_call_total),
            ("eval_expr_pipe", t.eval_expr_pipe_total),
            ("eval_binary", t.eval_binary_total),
            ("as_scalar", t.as_scalar_total),
            ("as_vec2", t.as_vec2_total),
            ("as_vec3", t.as_vec3_total),
            ("as_vec4", t.as_vec4_total),
            ("as_color_expr", t.as_color_expr_total),
            ("as_fill_style", t.as_fill_style_total),
            ("builtin_dispatch", t.builtin_dispatch_total),
            ("user_fn_call", t.user_fn_call_total),
            ("fn_body", t.fn_body_total),
            ("eval_block", t.eval_block_total),
            ("eval_compose", t.eval_compose_total),
        ];

        let mut parts = Vec::new();
        for (name, d) in entries {
            if d.is_zero() {
                continue;
            }
            parts.push(format!("{name}={:.3}ms", d.as_secs_f64() * 1000.0));
        }
        if parts.is_empty() {
            return;
        }

        self.hir
            .notes
            .push(format!("check profile timings: {}", parts.join(", ")));
    }
}

fn resolve_vertex_format_members(
    format_defs: &HashMap<String, VertexFormatDef>,
    format_name: &str,
) -> Option<Vec<VertexContractMemberDef>> {
    let format_def = format_defs.get(format_name)?;
    let mut members = Vec::new();
    if let Some(parent) = &format_def.parent {
        members.extend(resolve_vertex_format_members(format_defs, parent)?);
    }
    members.extend(format_def.members.iter().cloned());
    Some(members)
}

struct FnTypeInputs<'a> {
    name: &'a str,
    span: &'a Span,
    diags: &'a mut Vec<Diag>,
    context: &'a str,
    source_file: Option<&'a str>,
    enum_defs: &'a HashMap<String, EnumDef>,
    struct_defs: &'a HashMap<String, StructDef>,
    type_params: &'a [String],
    interface_names: &'a HashSet<String>,
}

fn parse_fn_type(input: FnTypeInputs<'_>) -> Option<FnValueTy> {
    let FnTypeInputs {
        name,
        span,
        diags,
        context,
        source_file,
        enum_defs,
        struct_defs,
        type_params,
        interface_names,
    } = input;

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
        parse_fn_type(FnTypeInputs {
            name: element,
            span,
            diags,
            context: "array element type",
            source_file,
            enum_defs,
            struct_defs,
            type_params: &[],
            interface_names,
        })?;
        return Some(FnValueTy::Array(
            name.chars().filter(|c| !c.is_whitespace()).collect(),
        ));
    }

    if let Some((param_tys_str, ret_str)) = try_parse_callable_ty(name) {
        let param_tys: Option<Vec<FnValueTy>> = param_tys_str
            .iter()
            .map(|p| {
                parse_fn_type(FnTypeInputs {
                    name: p,
                    span,
                    diags,
                    context: "callable parameter type",
                    source_file,
                    enum_defs,
                    struct_defs,
                    type_params,
                    interface_names,
                })
            })
            .collect();
        let param_tys = param_tys?;
        let ret_ty = Box::new(parse_fn_type(FnTypeInputs {
            name: &ret_str,
            span,
            diags,
            context: "callable return type",
            source_file,
            enum_defs,
            struct_defs,
            type_params,
            interface_names,
        })?);
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
        return parse_fn_type(FnTypeInputs {
            name: &args[0],
            span,
            diags,
            context,
            source_file,
            enum_defs,
            struct_defs,
            type_params,
            interface_names,
        });
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
            // Check if it's a defined enum type
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

#[cfg(test)]
mod tests {
    use super::{check, create_checker_for_surface, validate_conformance_method_bodies};
    use crate::ast::{
        ConformanceDecl, Expr, FnDecl, FnParam, InterfaceDecl, InterfaceMethodDecl,
        NormalizedRootEntry, NormalizedRootEntryParam, RootEntryKind, RootEntryParamRole, Spanned,
        TextureChannelDef, TextureTypeDecl,
    };
    use crate::check::CheckOptions;
    use crate::diag::Severity;

    fn span(start: usize, end: usize) -> std::ops::Range<usize> {
        start..end
    }

    fn var(name: &str) -> crate::ast::SExpr {
        Spanned {
            node: Expr::Var(name.to_string()),
            span: span(0, 1),
        }
    }

    fn vec2(x: crate::ast::SExpr, y: crate::ast::SExpr) -> crate::ast::SExpr {
        Spanned {
            node: Expr::Vec2(Box::new(x), Box::new(y)),
            span: span(0, 1),
        }
    }

    fn vec3(x: crate::ast::SExpr, y: crate::ast::SExpr, z: crate::ast::SExpr) -> crate::ast::SExpr {
        Spanned {
            node: Expr::Vec3(Box::new(x), Box::new(y), Box::new(z)),
            span: span(0, 1),
        }
    }

    fn normalized_root_entry_fixture(
        name: &str,
        param_name: &str,
        ty_name: &str,
    ) -> NormalizedRootEntry {
        NormalizedRootEntry {
            entry_kind: "canvas".into(),
            kind: RootEntryKind::Canvas,
            name: name.to_string(),
            name_span: span(0, 7),
            params: vec![NormalizedRootEntryParam {
                name: param_name.to_string(),
                name_span: span(8, 10),
                ty_name: ty_name.to_string(),
                ty_span: span(11, 15),
                role: RootEntryParamRole::Primary,
            }],
            material_ty: None,
            body: Vec::new(),
            span: span(0, 15),
        }
    }

    fn channel(channel: &str, semantic_name: &str) -> TextureChannelDef {
        TextureChannelDef {
            channel: channel.to_string(),
            channel_span: span(1, 2),
            semantic_name: semantic_name.to_string(),
            decode: None,
            span: span(1, 2),
        }
    }

    #[test]
    fn check_accepts_shared_entry_template() {
        let entry = normalized_root_entry_fixture("preview", "uv", "coord");

        let result = check(
            &entry,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &CheckOptions::default(),
        );

        assert!(
            result.is_err(),
            "expected shared entry template check to fail validation for an empty body"
        );
    }

    #[test]
    fn check_accepts_surface_style_shared_entry_template() {
        let entry = normalized_root_entry_fixture("preview_surface", "sp", "surf");

        let diags = check(
            &entry,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &CheckOptions::default(),
        )
        .expect_err("expected empty surface-style entry template to fail at body validation");

        assert!(diags.iter().all(|diag| {
            !diag
                .message
                .contains("unsupported entry parameter `sp: surf`")
        }));
        assert!(
            diags
                .iter()
                .all(|diag| !diag.message.contains("missing a primary input parameter"))
        );
    }

    #[test]
    fn check_accepts_template_decl_as_reusable_entry_contract() {
        let entry = NormalizedRootEntry {
            entry_kind: "canvas".into(),
            kind: RootEntryKind::Canvas,
            name: "fullscreen_effect".to_string(),
            name_span: span(0, 17),
            params: Vec::new(),
            material_ty: None,
            body: Vec::new(),
            span: span(0, 40),
        };

        let result = check(
            &entry,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &CheckOptions::default(),
        );

        assert!(
            result.is_err(),
            "expected template-backed entry contract to fail validation for an empty body"
        );
    }

    #[test]
    fn texture_type_result_allows_spatial_vec3_declaration() {
        let tt = TextureTypeDecl {
            name: "NormalGL".to_string(),
            name_span: span(0, 1),
            result_ty: Some(("vec3 in tangent".to_string(), span(0, 1))),
            channels: vec![channel("r", "x"), channel("g", "y"), channel("b", "z")],
            result_expr: Some(vec3(var("x"), var("y"), var("z"))),
            span: span(0, 4),
        };

        let (checker, prep_diags) = create_checker_for_surface(
            "texture_type_spatial_result_ok".to_string(),
            &[],
            &[],
            &[],
            &[],
            &[],
            &[tt],
            &[],
            &[],
            &[],
            &CheckOptions::default(),
        );

        assert!(
            prep_diags
                .iter()
                .all(|diag| diag.severity != Severity::Error),
            "unexpected prep diagnostics: {prep_diags:?}"
        );
        assert!(
            checker
                .diags
                .iter()
                .all(|diag| diag.severity != Severity::Error),
            "unexpected checker diagnostics: {:?}",
            checker.diags
        );
    }

    #[test]
    fn texture_type_result_reports_mismatch_for_spatial_vec3_declaration() {
        let tt = TextureTypeDecl {
            name: "NormalGL".to_string(),
            name_span: span(0, 1),
            result_ty: Some(("vec3 in tangent".to_string(), span(0, 1))),
            channels: vec![channel("r", "x"), channel("g", "y"), channel("b", "z")],
            result_expr: Some(vec2(var("x"), var("y"))),
            span: span(0, 4),
        };

        let (checker, _prep_diags) = create_checker_for_surface(
            "texture_type_spatial_result_mismatch".to_string(),
            &[],
            &[],
            &[],
            &[],
            &[],
            &[tt],
            &[],
            &[],
            &[],
            &CheckOptions::default(),
        );

        assert!(
            checker.diags.iter().any(|diag| {
                diag.severity == Severity::Error
                    && diag.message.contains("texture_type result expression for `NormalGL` must produce `vec3 in tangent`, found vec2")
            }),
            "expected spatial result mismatch diagnostic, got: {:?}",
            checker.diags
        );
    }

    #[test]
    fn validate_conformance_method_bodies_reports_unsupported_array_shape_placeholder() {
        let param_ty_span = span(48, 60);
        let interfaces = vec![InterfaceDecl {
            entry: None,
            name: "shape_array_provider".to_string(),
            name_span: span(0, 20),
            methods: vec![InterfaceMethodDecl {
                name: "choose".to_string(),
                name_span: span(21, 27),
                params: vec![FnParam {
                    is_context: false,
                    name: "xs".to_string(),
                    name_span: span(28, 30),
                    ty_name: "array<shape>".to_string(),
                    ty_span: param_ty_span.clone(),
                    keyword_only: false,
                    default: None,
                }],
                ret_ty: None,
                span: span(21, 61),
            }],
            span: span(0, 61),
        }];

        let conformances = vec![ConformanceDecl {
            type_name: "glow".to_string(),
            type_name_span: span(62, 66),
            interface_name: "shape_array_provider".to_string(),
            interface_name_span: span(67, 87),
            methods: vec![FnDecl {
                typed_body: None,
                name: "choose".to_string(),
                name_span: span(88, 94),
                docs: None,
                type_params: vec![],
                const_params: vec![],
                params: vec![FnParam {
                    is_context: false,
                    name: "xs".to_string(),
                    name_span: span(95, 97),
                    ty_name: "array<shape>".to_string(),
                    ty_span: param_ty_span.clone(),
                    keyword_only: false,
                    default: None,
                }],
                ret_ty: None,
                is_internal: false,
                derivative_free: false,
                is_builtin: false,
                source_file: "<checker-test>".to_string(),
                body: vec![],
                span: span(88, 120),
            }],
            body_checked_with_instance: false,
            span: span(62, 120),
        }];

        let diags = validate_conformance_method_bodies(
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &interfaces,
            &conformances,
            &[],
            &CheckOptions::default(),
        )
        .expect_err("array<shape> conformance placeholder should be rejected");

        let diag = diags
            .iter()
            .find(|diag| {
                diag.message.contains(
                    "conformance method parameter `choose.xs` uses unsupported type `array<shape>`",
                )
            })
            .expect("expected unsupported array<shape> placeholder diagnostic");

        assert_eq!(diag.span.start, param_ty_span.start);
        assert_eq!(diag.span.end, param_ty_span.end);
    }
}
