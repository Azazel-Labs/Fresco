//! Schema-targeted shading contracts and per-material runtime settings.
use crate::ast::*;
use crate::diag::Diag;
use std::collections::BTreeSet;

pub(super) fn lower(program: &mut Program) -> Result<(), Vec<Diag>> {
    let mut names: BTreeSet<_> = program.interfaces.iter().map(|i| i.name.clone()).collect();
    names.extend(program.structs.iter().map(|s| s.name.clone()));
    names.extend(program.enums.iter().map(|e| e.name.clone()));
    names.extend(program.material_properties.iter().map(|s| s.name.clone()));
    for contract in &program.style_contracts {
        if !names.insert(contract.name.clone()) {
            return Err(vec![
                Diag::error(
                    contract.name_span.clone(),
                    "duplicate shading contract name",
                )
                .with_file(&contract.source_file),
            ]);
        }
        require_schema(
            program,
            &contract.schema,
            &contract.schema_span,
            &contract.source_file,
        )?;
        let mut hooks = BTreeSet::new();
        for hook in &contract.hooks {
            let signature = &hook.signature;
            if !hooks.insert(&signature.name) {
                return Err(vec![
                    Diag::error(signature.name_span.clone(), "duplicate contract hook")
                        .with_file(&contract.source_file),
                ]);
            }
            if signature
                .params
                .iter()
                .any(|p| p.keyword_only || p.default.is_some())
                || hook
                    .default
                    .as_ref()
                    .is_some_and(|f| !f.type_params.is_empty() || !f.const_params.is_empty())
            {
                return Err(vec![
                    Diag::error(
                        signature.span.clone(),
                        "shading hooks require concrete positional signatures",
                    )
                    .with_file(&contract.source_file),
                ]);
            }
        }
    }
    // Validate first, then build the existing interface/implementation representation.
    for style in &program.styles {
        parameters(program, &style.name, &[])?;
        let mut captures: BTreeSet<_> = style
            .params
            .iter()
            .chain(&style.static_params)
            .map(|parameter| parameter.name.as_str())
            .collect();
        for input in &style.shading_inputs {
            let invalid = |message: String| {
                vec![Diag::error(input.span.clone(), message).with_file(&style.source_file)]
            };
            if input.scope != "draw" {
                return Err(invalid(
                    "shading inputs currently require scope draw".into(),
                ));
            }
            if !captures.insert(&input.name) || input.name.starts_with("__fresco_") {
                return Err(invalid("duplicate or reserved shading input name".into()));
            }
            if input.ty.trim() != "sampler" {
                super::compute_operations::resource_type(&input.ty, "read").map_err(invalid)?;
            }
        }
        if !names.insert(style.name.clone()) {
            return Err(vec![
                Diag::error(style.name_span.clone(), "duplicate style name")
                    .with_file(&style.source_file),
            ]);
        }
        require_schema(
            program,
            &style.schema,
            &style.schema_span,
            &style.source_file,
        )?;
        let contract = program
            .style_contracts
            .iter()
            .find(|c| c.name == style.contract)
            .ok_or_else(|| {
                vec![
                    Diag::error(
                        style.contract_span.clone(),
                        format!("unknown shading contract `{}`", style.contract),
                    )
                    .with_file(&style.source_file),
                ]
            })?;
        if style.schema != contract.schema {
            return Err(vec![
                Diag::error(
                    style.schema_span.clone(),
                    format!(
                        "style `{}` targets schema `{}`, but contract `{}` requires `{}`",
                        style.name, style.schema, contract.name, contract.schema
                    ),
                )
                .with_file(&style.source_file),
            ]);
        }
        let mut methods = BTreeSet::new();
        for method in &style.methods {
            if method.params.iter().any(|parameter| {
                style
                    .shading_inputs
                    .iter()
                    .any(|input| input.name == parameter.name)
            }) {
                return Err(vec![
                    Diag::error(
                        method.name_span.clone(),
                        "hook parameter shadows a shading input",
                    )
                    .with_file(&style.source_file),
                ]);
            }
            if method.params.iter().any(|p| {
                style
                    .params
                    .iter()
                    .chain(&style.static_params)
                    .any(|s| s.name == p.name)
            }) {
                return Err(vec![
                    Diag::error(
                        method.name_span.clone(),
                        "hook parameter shadows a style setting",
                    )
                    .with_file(&style.source_file),
                ]);
            }
            if !methods.insert(&method.name) {
                return Err(vec![
                    Diag::error(method.name_span.clone(), "duplicate style hook")
                        .with_file(&style.source_file),
                ]);
            }
            let hook = contract
                .hooks
                .iter()
                .find(|h| h.signature.name == method.name)
                .ok_or_else(|| {
                    vec![
                        Diag::error(
                            method.name_span.clone(),
                            format!(
                                "unknown style hook `{}` in contract `{}`",
                                method.name, contract.name
                            ),
                        )
                        .with_file(&style.source_file),
                    ]
                })?;
            if method
                .params
                .iter()
                .map(|p| &p.name)
                .collect::<BTreeSet<_>>()
                .len()
                != method.params.len()
            {
                return Err(vec![
                    Diag::error(method.name_span.clone(), "duplicate style hook parameter")
                        .with_file(&style.source_file),
                ]);
            }
            if !method.type_params.is_empty()
                || !method.const_params.is_empty()
                || method
                    .params
                    .iter()
                    .any(|p| p.keyword_only || p.default.is_some())
                || method.params.len() != hook.signature.params.len()
                || method
                    .params
                    .iter()
                    .zip(&hook.signature.params)
                    .any(|(a, b)| a.ty_name != b.ty_name)
                || method.ret_ty.as_ref().map(|(t, _)| t)
                    != hook.signature.ret_ty.as_ref().map(|(t, _)| t)
            {
                return Err(vec![
                    Diag::error(
                        method.name_span.clone(),
                        format!(
                            "style hook `{}` must match the exact contract signature",
                            method.name
                        ),
                    )
                    .with_file(&style.source_file),
                ]);
            }
        }
        for hook in &contract.hooks {
            if hook.default.is_none() && !methods.contains(&hook.signature.name) {
                return Err(vec![
                    Diag::error(
                        style.span.clone(),
                        format!(
                            "style `{}` is missing required hook `{}`",
                            style.name, hook.signature.name
                        ),
                    )
                    .with_file(&style.source_file),
                ]);
            }
        }
    }
    for contract in &program.style_contracts {
        program.interfaces.push(InterfaceDecl {
            entry: None,
            name: contract.name.clone(),
            name_span: contract.name_span.clone(),
            methods: contract.hooks.iter().map(|h| h.signature.clone()).collect(),
            span: contract.span.clone(),
        });
    }
    for style in &program.styles {
        let contract = program
            .style_contracts
            .iter()
            .find(|c| c.name == style.contract)
            .expect("validated contract");
        let mut methods = style.methods.clone();
        for hook in &contract.hooks {
            if !methods.iter().any(|m| m.name == hook.signature.name) {
                methods.push(hook.default.clone().expect("validated required hook"));
            }
        }
        program.structs.push(StructDecl {
            attrs: vec![PipelineAttribute {
                name: "implementation".into(),
                name_span: style.name_span.clone(),
                args: vec![style.contract.clone()],
                args_span: Some(style.contract_span.clone()),
                expressions: Vec::new(),
                span: style.span.clone(),
            }],
            name: style.name.clone(),
            name_span: style.name_span.clone(),
            fields: Vec::new(),
            span: style.span.clone(),
        });
        program.conformances.push(ConformanceDecl {
            type_name: style.name.clone(),
            type_name_span: style.name_span.clone(),
            interface_name: style.contract.clone(),
            interface_name_span: style.contract_span.clone(),
            methods,
            body_checked_with_instance: true,
            span: style.span.clone(),
        });
    }
    Ok(())
}

fn require_schema(program: &Program, name: &str, span: &Span, file: &str) -> Result<(), Vec<Diag>> {
    if program
        .material_properties
        .iter()
        .filter(|s| s.name == name)
        .count()
        != 1
    {
        return Err(vec![
            Diag::error(
                span.clone(),
                format!("shading declaration requires one material schema `{name}`"),
            )
            .with_file(file),
        ]);
    }
    Ok(())
}

pub(super) fn validate_defaults(
    program: &Program,
    options: &crate::check::CheckOptions,
) -> Result<(), Vec<Diag>> {
    crate::check::validate_style_contract_signatures(program, options)?;
    // Check even overridden/unused defaults, without exposing any style-local hooks
    // or material/pass bindings as captures of the contract's function body.
    let defaults: Vec<_> = program
        .style_contracts
        .iter()
        .map(|c| ConformanceDecl {
            type_name: c.name.clone(),
            type_name_span: c.name_span.clone(),
            interface_name: c.name.clone(),
            interface_name_span: c.name_span.clone(),
            methods: c.hooks.iter().filter_map(|h| h.default.clone()).collect(),
            body_checked_with_instance: false,
            span: c.span.clone(),
        })
        .collect();
    validate_hook_effects(program, &defaults)?;
    crate::check::validate_conformance_method_bodies(
        &program.functions,
        &program.consts,
        &program.enums,
        &program.structs,
        &program.params,
        &program.texture_types,
        &program.interfaces,
        &defaults,
        &program.effects,
        options,
    )?;
    // Style bodies see only declared settings and resource slots as captures.
    // Contract defaults remain checked separately, without access to these names.
    let mut styles: Vec<_> = program
        .styles
        .iter()
        .map(|style| ConformanceDecl {
            type_name: style.name.clone(),
            type_name_span: style.name_span.clone(),
            interface_name: style.contract.clone(),
            interface_name_span: style.contract_span.clone(),
            methods: style
                .methods
                .iter()
                .map(|method| {
                    let mut method = method.clone();
                    add_captures(&mut method, &style.params);
                    add_captures(&mut method, &style.static_params);
                    add_shading_captures(&mut method, &style.shading_inputs);
                    method
                })
                .collect(),
            body_checked_with_instance: false,
            span: style.span.clone(),
        })
        .collect();
    validate_hook_effects(program, &styles)?;
    let mut resource_program = program.clone();
    // Validate slot layouts even when a style inherits every hook and never
    // reads its slot. Unused declarations must not conceal invalid storage types.
    for style in program
        .styles
        .iter()
        .filter(|style| !style.shading_inputs.is_empty())
    {
        let body = vec![Stmt::Return {
            value: Spanned {
                node: Expr::Num(0.0, Unit::None),
                span: style.span.clone(),
            },
            span: style.span.clone(),
        }];
        let mut declaration = FnDecl {
            derivative_free: true,
            typed_body: Some(body.clone()),
            name: format!("__fresco_slot_layout_{}", style.name),
            name_span: style.name_span.clone(),
            docs: None,
            type_params: Vec::new(),
            const_params: Vec::new(),
            params: Vec::new(),
            ret_ty: Some(("f32".into(), style.span.clone())),
            is_internal: false,
            is_builtin: false,
            source_file: style.source_file.clone(),
            body,
            span: style.span.clone(),
        };
        add_shading_captures(&mut declaration, &style.shading_inputs);
        resource_program.functions.push(declaration);
    }
    for conformance in &mut styles {
        let mut ordinary = Vec::new();
        for mut method in std::mem::take(&mut conformance.methods) {
            if method.params.iter().any(|parameter| {
                crate::resource_type::shader_resource_type(&parameter.ty_name).is_some()
            }) {
                method.name = format!("__fresco_hook_{}_{}", conformance.type_name, method.name);
                method.derivative_free = true;
                resource_program.functions.push(method);
            } else {
                ordinary.push(method);
            }
        }
        conformance.methods = ordinary;
    }
    super::schema_function::validate_resource_functions(&resource_program)?;
    crate::check::validate_conformance_method_bodies(
        &program.functions,
        &program.consts,
        &program.enums,
        &program.structs,
        &[],
        &program.texture_types,
        &program.interfaces,
        &styles,
        &program.effects,
        options,
    )
}

fn validate_hook_effects(
    program: &Program,
    conformances: &[ConformanceDecl],
) -> Result<(), Vec<Diag>> {
    for method in conformances
        .iter()
        .flat_map(|conformance| &conformance.methods)
    {
        let mut functions = program.functions.clone();
        let root = functions.len();
        // The hook is a root, not an ordinary function exported into helper scope.
        let mut hook = method.clone();
        hook.is_internal = true;
        functions.push(hook);
        super::schema_function::validate_library_effects(root, &functions, &program.structs)
            .map_err(|message| {
                vec![Diag::error(method.span.clone(), message).with_file(&method.source_file)]
            })?;
    }
    Ok(())
}

pub(super) fn add_captures(method: &mut FnDecl, params: &[GlobalParamDecl]) {
    method.params.extend(params.iter().map(|p| FnParam {
        is_context: false,
        name: p.name.clone(),
        name_span: p.name_span.clone(),
        ty_name: p.ty_name.clone(),
        ty_span: p.ty_span.clone(),
        keyword_only: false,
        default: None,
    }));
}

pub(super) fn add_shading_captures(method: &mut FnDecl, inputs: &[StyleShadingInputDecl]) {
    method.params.extend(inputs.iter().map(|input| FnParam {
        is_context: false,
        name: input.name.clone(),
        name_span: input.span.clone(),
        ty_name: input.ty.clone(),
        ty_span: input.span.clone(),
        keyword_only: false,
        default: None,
    }));
}

/// Persist symbols and named values, never bundle-local IDs or offsets.
pub(super) fn selection_override(
    program: &Program,
    value: &serde_json::Map<String, serde_json::Value>,
    span: &Span,
) -> Result<Expr, String> {
    if value
        .keys()
        .any(|key| !matches!(key.as_str(), "symbol" | "settings"))
    {
        return Err("style override accepts only symbol and settings".into());
    }
    let symbol = value
        .get("symbol")
        .and_then(|s| s.as_str())
        .ok_or("style override requires a symbol")?;
    let style = program
        .styles
        .iter()
        .find(|s| s.name == symbol)
        .ok_or("style override requires a declared style")?;
    let settings = value
        .get("settings")
        .and_then(|s| s.as_object())
        .ok_or("style override requires a settings object")?;
    let mut args = Vec::new();
    for (name, value) in settings {
        let parameter = style
            .params
            .iter()
            .chain(&style.static_params)
            .find(|p| &p.name == name)
            .ok_or_else(|| format!("unknown style setting `{name}`"))?;
        let number = |value: &serde_json::Value| {
            value
                .as_f64()
                .filter(|v| v.is_finite())
                .map(|v| Spanned {
                    node: Expr::Num(v, Unit::None),
                    span: span.clone(),
                })
                .ok_or_else(|| "style setting requires finite numeric components".to_string())
        };
        let node = if matches!(parameter.ty_name.as_str(), "f32" | "u32" | "i32") {
            let number = number(value)?;
            if parameter.ty_name == "f32" {
                number.node
            } else {
                let n = value.as_f64().ok_or("expected integer setting")?;
                let (min, max) = if parameter.ty_name == "u32" {
                    (0.0, f64::from(u32::MAX))
                } else {
                    (f64::from(i32::MIN), f64::from(i32::MAX))
                };
                if n.fract() != 0.0 || n < min || n > max {
                    return Err("style integer setting is outside its declared type".into());
                }
                Expr::Call {
                    name: parameter.ty_name.clone(),
                    name_span: span.clone(),
                    const_args: vec![],
                    args: vec![Arg {
                        name: None,
                        value: number,
                    }],
                }
            }
        } else if parameter.ty_name == "bool" {
            Expr::Var(
                value
                    .as_bool()
                    .ok_or("expected boolean setting")?
                    .to_string(),
            )
        } else {
            let values = value
                .as_array()
                .ok_or("style vector/color override requires an array")?;
            let count = match parameter.ty_name.as_str() {
                "vec2" => 2,
                "vec3" => 3,
                "vec4" | "color" => 4,
                _ => return Err("unsupported style setting type".into()),
            };
            if values.len() != count {
                return Err("style setting has the wrong component count".into());
            }
            if parameter.ty_name == "color" {
                let mut rgba = [0.0; 4];
                for (lane, value) in rgba.iter_mut().zip(values) {
                    let Expr::Num(n, _) = number(value)?.node else {
                        unreachable!()
                    };
                    *lane = n as f32;
                }
                Expr::Color(rgba)
            } else {
                Expr::Call {
                    name: parameter.ty_name.clone(),
                    name_span: span.clone(),
                    const_args: Vec::new(),
                    args: values
                        .iter()
                        .map(|v| {
                            Ok(Arg {
                                name: None,
                                value: number(v)?,
                            })
                        })
                        .collect::<Result<_, String>>()?,
                }
            }
        };
        args.push(Arg {
            name: Some(name.clone()),
            value: Spanned {
                node,
                span: span.clone(),
            },
        });
    }
    Ok(Expr::Call {
        name: symbol.into(),
        name_span: span.clone(),
        const_args: Vec::new(),
        args,
    })
}

pub(super) fn parameters(
    program: &Program,
    symbol: &str,
    args: &[Arg],
) -> Result<
    (
        Vec<fresco_artifact::ManifestParam>,
        Vec<fresco_artifact::ManifestParam>,
    ),
    Vec<Diag>,
> {
    let Some(style) = program.styles.iter().find(|s| s.name == symbol) else {
        return if args.is_empty() {
            Ok((Vec::new(), Vec::new()))
        } else {
            Err(vec![Diag::error(
                args[0].value.span.clone(),
                "only styles accept settings",
            )])
        };
    };
    let fail = |span: Span, message: String| {
        vec![Diag::error(span, message).with_file(&style.source_file)]
    };
    let assignment_fail = |span: Span, message: String| vec![Diag::error(span, message)];
    let mut names = BTreeSet::new();
    for arg in args {
        let Some(name) = &arg.name else {
            return Err(assignment_fail(
                arg.value.span.clone(),
                "style settings require named arguments".into(),
            ));
        };
        if !names.insert(name) {
            return Err(assignment_fail(
                arg.value.span.clone(),
                format!("duplicate style setting `{name}`"),
            ));
        }
        if !style
            .params
            .iter()
            .chain(&style.static_params)
            .any(|p| &p.name == name)
        {
            return Err(assignment_fail(
                arg.value.span.clone(),
                format!("unknown style setting `{name}`"),
            ));
        }
    }
    let mut declarations = BTreeSet::new();
    let values = style
        .params
        .iter()
        .chain(&style.static_params)
        .map(|p| {
            if !declarations.insert(&p.name) {
                return Err(fail(p.name_span.clone(), "duplicate style setting".into()));
            }
            if !matches!(
                p.ty_name.as_str(),
                "f32" | "vec2" | "vec3" | "vec4" | "color" | "u32" | "i32" | "bool"
            ) {
                return Err(fail(
                    p.ty_span.clone(),
                    "style settings support scalar, float vector, and color types".into(),
                ));
            }
            let default = p
                .default
                .as_ref()
                .ok_or_else(|| fail(p.span.clone(), "style setting requires a default".into()))?;
            let bounds = p
                .range
                .as_ref()
                .map(|(min, max)| {
                    let bound = |expression: &SExpr| {
                        if matches!(p.ty_name.as_str(), "u32" | "i32") {
                            crate::check::compute::style_parameter_value(program, p, expression)
                                .map(|value| value.as_f64().expect("typed integer setting"))
                        } else {
                            crate::check::compute::constant_number(program, &[], expression)
                        }
                    };
                    Ok::<_, Vec<Diag>>((bound(min)?, bound(max)?))
                })
                .transpose()?;
            if p.ty_name == "bool" && bounds.is_some() {
                return Err(fail(
                    p.span.clone(),
                    "boolean settings cannot have numeric ranges".into(),
                ));
            }
            if bounds.is_some_and(|(min, max)| min > max) {
                return Err(fail(
                    p.span.clone(),
                    "style setting range is reversed".into(),
                ));
            }
            let validate = |expression: &SExpr, declaration: bool| {
                let value = crate::check::compute::style_parameter_value(program, p, expression)
                    .map_err(|errors| {
                        errors
                            .into_iter()
                            .map(|mut d| {
                                if declaration {
                                    d.file = Some(style.source_file.clone());
                                } else if d.file.as_deref() == Some("") {
                                    d.file = None;
                                }
                                d
                            })
                            .collect::<Vec<_>>()
                    })?;
                if p.ty_name == "bool" {
                    return Ok(value);
                }
                let values = if let Some(values) = value.as_array() {
                    values.clone()
                } else {
                    vec![value.clone()]
                };
                if values.iter().any(|v| {
                    v.as_f64().is_none_or(|v| {
                        !v.is_finite() || bounds.is_some_and(|(min, max)| v < min || v > max)
                    })
                }) {
                    let message = "style setting value is outside its declared range".into();
                    return Err(if declaration {
                        fail(expression.span.clone(), message)
                    } else {
                        assignment_fail(expression.span.clone(), message)
                    });
                }
                Ok(value)
            };
            let mut value = validate(default, true)?;
            if let Some(arg) = args.iter().find(|a| a.name.as_deref() == Some(&p.name)) {
                value = validate(&arg.value, false)?;
            }
            Ok(fresco_artifact::ManifestParam {
                name: p.name.clone(),
                ty: p.ty_name.clone(),
                param_type: None,
                default: value,
                min: bounds.map(|b| b.0),
                max: bounds.map(|b| b.1),
            })
        })
        .collect::<Result<Vec<_>, Vec<Diag>>>()?;
    let (runtime, statics) = values.split_at(style.params.len());
    Ok((runtime.to_vec(), statics.to_vec()))
}

pub(super) fn check_selection(
    program: &Program,
    contract: &str,
    surface: &SurfaceDecl,
) -> Result<(), Vec<Diag>> {
    let Some(contract) = program.style_contracts.iter().find(|c| c.name == contract) else {
        return Ok(());
    };
    let mut actual = match &surface.material_ty {
        MaterialReturnTy::Named(name) => Some(name.as_str()),
        MaterialReturnTy::Default => program
            .material_properties
            .iter()
            .find(|s| s.is_default)
            .map(|s| s.name.as_str()),
    };
    for _ in 0..=program.material_properties.len() {
        let Some(name) = actual else {
            break;
        };
        if name == contract.schema {
            return Ok(());
        }
        actual = program
            .material_properties
            .iter()
            .find(|s| s.name == name)
            .and_then(|s| s.extends_name.as_deref());
    }
    Err(vec![Diag::error(
        surface.name_span.clone(),
        format!(
            "style contract `{}` requires material schema `{}`",
            contract.name, contract.schema
        ),
    )])
}
