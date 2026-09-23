//! Bind authored declaration vocabulary to engine interfaces after import resolution.
use crate::ast::{Arg, Canvas, CanvasParam, Expr, FnDecl, Program, Spanned, Stmt};
use crate::diag::Diag;
use std::collections::{HashMap, HashSet};

pub(super) fn resolve(program: &mut Program) -> Result<(), Vec<Diag>> {
    let mut registrations = HashMap::new();
    for interface in &program.interfaces {
        if let Some((kind, method)) = &interface.entry {
            if registrations.insert(kind.clone(), interface).is_some() {
                return Err(vec![Diag::error(
                    interface.name_span.clone(),
                    format!("duplicate @entry({kind}, ...) registration"),
                )]);
            }
            if !interface
                .methods
                .iter()
                .any(|candidate| candidate.name == *method)
            {
                return Err(vec![Diag::error(
                    interface.name_span.clone(),
                    format!(
                        "entry method `{method}` is not declared by `{}`",
                        interface.name
                    ),
                )]);
            }
        }
    }
    // Validate engine composition contracts even when an effect uses an explicit
    // method implementation (which does not need the block sugar).
    for template in &program.templates {
        for plug in &template.plugs {
            if template.entry.is_none()
                && let Some(composition) = &plug.compose_param
            {
                return Err(vec![Diag::error(
                    composition.span.clone(),
                    "@compose requires an engine-registered @entry interface",
                )]);
            }
            if let Some(composition) = &plug.compose_param
                && !plug.params.first().is_some_and(|state| {
                    state.name == composition.node
                        && !state.is_context
                        && !state.keyword_only
                        && plug
                            .return_ty
                            .as_ref()
                            .is_some_and(|ty| ty.node == state.ty_name)
                })
            {
                return Err(vec![Diag::error(
                    composition.span.clone(),
                    "@compose requires the named first parameter and result to have the same state type",
                )]);
            }
        }
    }
    // Registered compute pipelines are templates, not live systems. Only an
    // authored entry instantiates their resources and dispatch plan.
    let compute_interfaces: HashSet<_> = program
        .pipelines
        .iter()
        .filter(|pipeline| pipeline.pipeline_type == "compute")
        .filter_map(|pipeline| pipeline.material_name.as_ref())
        .filter(|name| {
            registrations
                .values()
                .any(|interface| interface.name == **name)
        })
        .cloned()
        .collect();
    let mut compute_plans = Vec::new();
    program.pipelines.retain(|pipeline| {
        if pipeline.pipeline_type == "compute"
            && pipeline
                .material_name
                .as_ref()
                .is_some_and(|name| compute_interfaces.contains(name))
        {
            compute_plans.push(pipeline.clone());
            false
        } else {
            true
        }
    });
    let mut compute_passes = Vec::new();
    program.passes.retain(|pass| {
        if pass
            .material_name
            .as_ref()
            .is_some_and(|name| compute_interfaces.contains(name))
        {
            compute_passes.push(pass.clone());
            false
        } else {
            true
        }
    });
    let mut retained = Vec::new();
    for canvas in program.canvases.drain(..) {
        if registrations.contains_key(&canvas.entry_kind) {
            program.authored_entries.push(crate::ast::AuthoredEntry {
                config: Vec::new(),
                kind: canvas.entry_kind,
                name: canvas.name,
                name_span: canvas.name_span,
                params: canvas.params,
                return_ty: canvas.declared_return_ty,
                body: canvas.body,
                blocks: Vec::new(),
                span: canvas.span,
            });
        } else {
            retained.push(canvas);
        }
    }
    program.canvases = retained;
    for canvas in &program.canvases {
        if let Some(interface) = registrations.get(&canvas.entry_kind)
            && canvas.params.is_empty()
            && canvas.body.is_empty()
        {
            let (_, method) = interface.entry.as_ref().expect("registered interface");
            return Err(vec![Diag::error(
                canvas.name_span.clone(),
                format!("missing required entry block `{method}`"),
            )]);
        }
        if let Some(ty) = &canvas.declared_return_ty
            && ty.node != "color"
        {
            return Err(vec![Diag::error(
                ty.span.clone(),
                "raster entry result type must be color",
            )]);
        }
        if let Some(interface) = registrations.get(&canvas.entry_kind)
            && interface.methods.len() != 1
            && !canvas.params.is_empty()
        {
            return Err(vec![Diag::error(
                canvas.name_span.clone(),
                "entry shorthand requires named blocks for all interface methods",
            )]);
        }
    }
    let configuration_program = program.clone();
    for mut entry in program.authored_entries.drain(..) {
        if entry.blocks.is_empty()
            && entry.params.is_empty()
            && entry
                .body
                .iter()
                .any(|statement| matches!(statement, Stmt::LocalFnDecl(_)))
        {
            let mut params = Vec::new();
            for statement in std::mem::take(&mut entry.body) {
                match statement {
                    Stmt::LocalFnDecl(method) => {
                        entry.blocks.push(crate::ast::AuthoredEntryBlock {
                            params: Some(method.params),
                            return_ty: method.ret_ty.map(|(node, span)| Spanned { node, span }),
                            name: method.name,
                            name_span: method.name_span,
                            body: method.body,
                            span: method.span,
                        });
                    }
                    param @ Stmt::Param { .. } => params.push(param),
                    _ => {
                        return Err(vec![Diag::error(
                            entry.name_span.clone(),
                            "registered contract bodies require instance parameters and method implementations",
                        )]);
                    }
                }
            }
            entry.body = params;
        }
        let interface = registrations.get(&entry.kind).ok_or_else(|| {
            vec![
                Diag::error(
                    entry.name_span.clone(),
                    format!("unregistered entry declaration `{}`", entry.kind),
                )
                .with_help(
                    "the engine must declare @entry(declaration_name, method) on an interface",
                ),
            ]
        })?;
        let (_, entry_method) = interface.entry.as_ref().expect("registered interface");
        let method = interface
            .methods
            .iter()
            .find(|method| method.name == *entry_method)
            .expect("validated registration");
        let schema = program
            .templates
            .iter()
            .find(|template| template.name == interface.name);
        let mut settings = HashMap::new();
        let mut setting_spans = HashMap::new();
        let mut config_diags = Vec::new();
        for (name, value) in std::mem::take(&mut entry.config) {
            setting_spans.insert(name.node.clone(), value.span.clone());
            if schema.is_none_or(|schema| {
                !schema
                    .config_params
                    .iter()
                    .any(|field| field.name == name.node)
            }) {
                config_diags.push(Diag::error(
                    name.span.clone(),
                    format!(
                        "unknown entry setting `{}` for `{}`",
                        name.node, interface.name
                    ),
                ));
            }
            if settings.insert(name.node.clone(), value).is_some() {
                config_diags.push(Diag::error(
                    name.span,
                    format!("duplicate entry setting `{}`", name.node),
                ));
            }
        }
        let mut config_body = Vec::new();
        let mut declared = HashSet::new();
        if let Some(schema) = schema {
            for field in &schema.config_params {
                if !declared.insert(&field.name) {
                    config_diags.push(Diag::error(
                        field.name_span.clone(),
                        format!("duplicate engine configuration field `{}`", field.name),
                    ));
                    continue;
                }
                let Some(value) = settings
                    .remove(&field.name)
                    .or_else(|| field.default.clone())
                else {
                    config_diags.push(Diag::error(entry.name_span.clone(), format!("missing required entry setting `{}`", field.name))
                        .with_help(format!("provide `{}: <{} value>` in the entry or declare an explicit default in engine interface `{}`", field.name, field.ty_name, interface.name)));
                    continue;
                };
                config_body.push(Stmt::Const {
                    name: field.name.clone(),
                    name_span: field.name_span.clone(),
                    ty_name: field.ty_name.clone(),
                    ty_span: value.span.clone(),
                    value,
                });
            }
        }
        if !config_diags.is_empty() {
            return Err(config_diags);
        }
        if entry.blocks.is_empty() && entry.params.is_empty() && entry.body.is_empty() {
            return Err(vec![Diag::error(
                entry.name_span.clone(),
                format!("missing required entry block `{entry_method}`"),
            )]);
        }
        let compute = compute_interfaces.contains(&interface.name);
        // Select a supported execution contract by its explicit typed signature.
        // Vocabulary names never choose lowering behavior.
        if !compute
            && (method.params.len() != 1
                || !method.params[0].is_context
                || method.ret_ty.as_ref().is_none_or(|(ty, _)| ty != "color"))
        {
            return Err(vec![Diag::error(
                entry.name_span.clone(),
                "registered entry has no supported execution contract; raster entries require one @context parameter and a color result",
            )]);
        }
        if !compute
            && entry
                .return_ty
                .as_ref()
                .is_some_and(|ty| ty.node != "color")
        {
            return Err(vec![Diag::error(
                entry.return_ty.as_ref().expect("present").span.clone(),
                "entry result type does not match the registered method's color result",
            )]);
        }
        if compute && entry.return_ty.is_some() {
            return Err(vec![Diag::error(
                entry.name_span.clone(),
                "compute entry methods inherit result types from the engine interface; omit the entry result annotation",
            )]);
        }
        if compute && entry.blocks.is_empty() {
            return Err(vec![Diag::error(
                entry.name_span.clone(),
                "compute entries require named method blocks",
            )]);
        }
        let (params, body) = if entry.blocks.is_empty() {
            if interface.methods.len() != 1 {
                return Err(vec![Diag::error(
                    entry.name_span.clone(),
                    "entry shorthand requires named blocks for all interface methods",
                )]);
            }
            config_body.extend(entry.body);
            (entry.params, config_body)
        } else {
            if !entry.params.is_empty() {
                return Err(vec![Diag::error(
                    entry.name_span.clone(),
                    "named entry blocks inherit their parameters from the engine interface",
                )]);
            }
            let mut seen = HashSet::new();
            let mut diagnostics = Vec::new();
            for block in &entry.blocks {
                if !seen.insert(block.name.clone()) {
                    diagnostics.push(Diag::error(
                        block.name_span.clone(),
                        format!("duplicate entry block `{}`", block.name),
                    ));
                }
                if !interface
                    .methods
                    .iter()
                    .any(|method| method.name == block.name)
                {
                    diagnostics.push(Diag::error(
                        block.name_span.clone(),
                        format!(
                            "unknown entry block `{}` for `{}`",
                            block.name, interface.name
                        ),
                    ));
                }
            }
            for required in &interface.methods {
                if !seen.contains(&required.name) {
                    diagnostics.push(Diag::error(
                        entry.name_span.clone(),
                        format!("missing required entry block `{}`", required.name),
                    ));
                }
            }
            if !diagnostics.is_empty() {
                return Err(diagnostics);
            }
            let mut body = config_body;
            body.extend(entry.body);
            for mut block in entry.blocks {
                let required = interface
                    .methods
                    .iter()
                    .find(|method| method.name == block.name)
                    .expect("validated entry block");
                if block.params.is_none()
                    && let Some(composition) = schema
                        .and_then(|schema| schema.plugs.iter().find(|plug| plug.name == block.name))
                        .and_then(|plug| plug.compose_param.as_ref())
                {
                    let Some(state) = required.params.first().filter(|state| {
                        state.name == composition.node
                            && !state.is_context
                            && required
                                .ret_ty
                                .as_ref()
                                .is_some_and(|(ty, _)| *ty == state.ty_name)
                    }) else {
                        return Err(vec![Diag::error(
                            composition.span.clone(),
                            "@compose requires the named first parameter and result to have the same state type",
                        )]);
                    };
                    let mut composed = Vec::new();
                    for statement in block.body {
                        let Stmt::Expr(mut call) = statement else {
                            return Err(vec![Diag::error(
                                block.name_span.clone(),
                                "composition blocks require module calls; use an explicit fn implementation for control flow",
                            )]);
                        };
                        let Expr::Call { args, .. } = &mut call.node else {
                            return Err(vec![Diag::error(
                                call.span.clone(),
                                "composition blocks require module calls",
                            )]);
                        };
                        args.insert(
                            0,
                            Arg {
                                name: None,
                                value: Spanned {
                                    node: Expr::Var(state.name.clone()),
                                    span: call.span.clone(),
                                },
                            },
                        );
                        composed.push(Stmt::Let {
                            mutable: false,
                            name: state.name.clone(),
                            name_span: call.span.clone(),
                            declared_ty_name: Some(state.ty_name.clone()),
                            declared_ty_span: Some(call.span.clone()),
                            value: call,
                        });
                    }
                    composed.push(Stmt::Return {
                        value: Spanned {
                            node: Expr::Var(state.name.clone()),
                            span: block.span.clone(),
                        },
                        span: block.span.clone(),
                    });
                    block.body = composed;
                }
                let params = if let Some(params) = block.params {
                    if params.len() != required.params.len()
                        || params
                            .iter()
                            .zip(&required.params)
                            .any(|(actual, expected)| {
                                actual.ty_name != expected.ty_name
                                    || actual.is_context != expected.is_context
                            })
                        || block.return_ty.as_ref().map(|ty| &ty.node)
                            != required.ret_ty.as_ref().map(|(ty, _)| ty)
                    {
                        return Err(vec![Diag::error(
                            block.name_span.clone(),
                            format!(
                                "entry block `{}` signature does not match engine interface `{}`",
                                block.name, interface.name
                            ),
                        )]);
                    }
                    params
                } else {
                    required.params.clone()
                };
                body.push(Stmt::LocalFnDecl(FnDecl {
                    typed_body: None,
                    name: block.name,
                    name_span: block.name_span,
                    docs: None,
                    type_params: Vec::new(),
                    const_params: Vec::new(),
                    params,
                    ret_ty: required.ret_ty.clone(),
                    is_internal: false,
                    derivative_free: false,
                    is_builtin: false,
                    source_file: String::new(),
                    body: block.body,
                    span: block.span,
                }));
            }
            if compute {
                let plans: Vec<_> = compute_plans
                    .iter()
                    .filter(|plan| plan.material_name.as_deref() == Some(&interface.name))
                    .collect();
                if plans.len() != 1 {
                    return Err(vec![Diag::error(
                        entry.name_span.clone(),
                        "compute entry requires exactly one engine pipeline for its interface",
                    )]);
                }
                let mut bindings: Vec<_> = body
                    .iter()
                    .filter(|stmt| matches!(stmt, Stmt::Const { .. }))
                    .cloned()
                    .collect();
                let mut aliases = HashSet::new();
                for required in &interface.methods {
                    let alias = schema.and_then(|schema| schema.plugs.iter().find(|plug| plug.name == required.name))
                        .and_then(|plug| plug.binding_name.as_ref()).ok_or_else(|| vec![Diag::error(required.name_span.clone(),
                            format!("compute entry method `{}` requires an explicit @bind(engine_callable) declaration", required.name))])?;
                    if !aliases.insert(alias.node.clone()) {
                        return Err(vec![Diag::error(
                            alias.span.clone(),
                            "duplicate compute entry binding alias",
                        )]);
                    }
                    let mut implementation = body.clone();
                    implementation.push(Stmt::Return {
                        value: Spanned {
                            node: Expr::Call {
                                name: required.name.clone(),
                                name_span: required.name_span.clone(),
                                const_args: Vec::new(),
                                args: required
                                    .params
                                    .iter()
                                    .map(|param| Arg {
                                        name: None,
                                        value: Spanned {
                                            node: Expr::Var(param.name.clone()),
                                            span: param.name_span.clone(),
                                        },
                                    })
                                    .collect(),
                            },
                            span: entry.span.clone(),
                        },
                        span: entry.span.clone(),
                    });
                    bindings.push(Stmt::LocalFnDecl(FnDecl {
                        typed_body: None,
                        name: alias.node.clone(),
                        name_span: alias.span.clone(),
                        docs: None,
                        type_params: Vec::new(),
                        const_params: Vec::new(),
                        params: required.params.clone(),
                        ret_ty: required.ret_ty.clone(),
                        is_internal: false,
                        derivative_free: false,
                        is_builtin: false,
                        source_file: String::new(),
                        body: implementation,
                        span: entry.span.clone(),
                    }));
                }
                let mut properties = Vec::new();
                if let Some(schema) = schema {
                    for field in &schema.config_params {
                        if !field.attrs.iter().any(|(name, args)| {
                            name == "config" && args.iter().any(|arg| arg == "editor")
                        }) {
                            continue;
                        }
                        let expression = Spanned {
                            node: Expr::Var(field.name.clone()),
                            span: entry.name_span.clone(),
                        };
                        let value = crate::check::compute::constant_scalar(
                            &configuration_program,
                            &bindings,
                            &expression,
                        )?;
                        let choices = if field.ty_name == "bool" {
                            vec![("false".into(), 0.0), ("true".into(), 1.0)]
                        } else if let Some(enumeration) = program
                            .enums
                            .iter()
                            .find(|enumeration| enumeration.name == field.ty_name)
                        {
                            enumeration
                                .variants
                                .iter()
                                .map(|variant| {
                                    let name = format!("{}.{}", enumeration.name, variant.name);
                                    let expression = Spanned {
                                        node: Expr::Var(name.clone()),
                                        span: entry.name_span.clone(),
                                    };
                                    crate::check::compute::constant_scalar(
                                        &configuration_program,
                                        &bindings,
                                        &expression,
                                    )
                                    .map(|value| (name, value))
                                })
                                .collect::<Result<Vec<_>, _>>()?
                        } else {
                            Vec::new()
                        };
                        properties.push(crate::ast::EntryProperty {
                            editable: true,
                            block: None,
                            block_present: false,
                            entry: entry.name.clone(),
                            name: field.name.clone(),
                            ty: field.ty_name.clone(),
                            value,
                            choices,
                            permutation: field.attrs.iter().any(|(name, _)| name == "permutation"),
                            value_span: setting_spans.get(&field.name).cloned(),
                            insert_at: body
                                .iter()
                                .find_map(|stmt| {
                                    if let Stmt::LocalFnDecl(function) = stmt {
                                        Some(function.span.start)
                                    } else {
                                        None
                                    }
                                })
                                .unwrap_or(entry.span.end - 1),
                        });
                    }
                }
                let mut plan = plans[0].clone();
                plan.name = format!("{}_{}", entry.name, plan.name);
                plan.material_name = None;
                plan.material_span = None;
                let mut remapped = HashMap::new();
                for reference in &mut plan.passes {
                    if let Some(name) = remapped.get(&reference.node) {
                        reference.node.clone_from(name);
                        continue;
                    }
                    if let Some(template) = compute_passes
                        .iter()
                        .find(|pass| pass.name == reference.node)
                    {
                        if template.material_name.as_deref() != Some(&interface.name)
                            || template
                                .stage
                                .as_ref()
                                .is_none_or(|stage| stage.node != "compute")
                        {
                            return Err(vec![Diag::error(
                                reference.span.clone(),
                                "entry pipeline references an incompatible compute pass contract",
                            )]);
                        }
                        if template
                            .hooks
                            .iter()
                            .any(|hook| aliases.contains(&hook.name))
                        {
                            return Err(vec![Diag::error(
                                template.name_span.clone(),
                                "entry binding alias conflicts with an engine pass hook",
                            )]);
                        }
                        let mut instance = template.clone();
                        instance.name = format!("{}_{}", entry.name, template.name);
                        instance.material_name = None;
                        instance.material_span = None;
                        instance.entry_bindings.clone_from(&bindings);
                        instance.entry_properties.clone_from(&properties);
                        remapped.insert(reference.node.clone(), instance.name.clone());
                        reference.node.clone_from(&instance.name);
                        program.passes.push(instance);
                    } else if !program
                        .passes
                        .iter()
                        .any(|pass| pass.name == reference.node)
                    {
                        return Err(vec![Diag::error(
                            reference.span.clone(),
                            "entry pipeline references an unknown pass",
                        )]);
                    }
                }
                if remapped.is_empty() {
                    return Err(vec![Diag::error(
                        entry.name_span.clone(),
                        "compute entry pipeline must bind a compute pass for its interface",
                    )]);
                }
                for reference in &mut plan.pass_refs {
                    if let Some(name) = remapped.get(&reference.name) {
                        reference.name = name.clone();
                    }
                }
                for attribute in &mut plan.attrs {
                    resolve_attribute(attribute, &configuration_program, &bindings)?;
                }
                program.pipelines.push(plan);
                continue;
            }
            let param = &method.params[0];
            let params = vec![CanvasParam {
                name: param.name.clone(),
                name_span: entry.name_span.clone(),
                ty_name: param.ty_name.clone(),
                ty_span: entry.name_span.clone(),
            }];
            let call = Spanned {
                node: Expr::Call {
                    name: entry_method.clone(),
                    name_span: entry.name_span.clone(),
                    const_args: Vec::new(),
                    args: vec![Arg {
                        name: None,
                        value: Spanned {
                            node: Expr::Var(param.name.clone()),
                            span: entry.name_span.clone(),
                        },
                    }],
                },
                span: entry.span.clone(),
            };
            body.push(Stmt::Expr(Spanned {
                node: Expr::Layer(Box::new(call)),
                span: entry.span.clone(),
            }));
            (params, body)
        };
        program.canvases.push(Canvas {
            entry_kind: entry.kind,
            declared_return_ty: entry.return_ty,
            name: entry.name,
            name_span: entry.name_span,
            params,
            body,
            span: entry.span,
        });
    }
    let constants_program = program.clone();
    for pass in &mut program.passes {
        let bindings: Vec<_> = pass
            .entry_bindings
            .iter()
            .filter(|binding| matches!(binding, Stmt::Const { .. }))
            .cloned()
            .collect();
        for attribute in &mut pass.attrs {
            resolve_attribute(attribute, &constants_program, &bindings)?;
        }
    }
    for pipeline in &mut program.pipelines {
        for attribute in &mut pipeline.attrs {
            if attribute.name == "meta" && attribute.args.is_empty() {
                resolve_attribute(attribute, &constants_program, &[])?;
            }
        }
    }
    Ok(())
}

fn resolve_attribute(
    attribute: &mut crate::ast::PipelineAttribute,
    program: &Program,
    bindings: &[Stmt],
) -> Result<(), Vec<Diag>> {
    if attribute.expressions.is_empty() {
        return Ok(());
    }
    attribute.args = attribute
        .expressions
        .iter()
        .enumerate()
        .map(|(index, expression)| {
            if attribute.name == "meta" {
                if let Expr::Str(value) = &expression.node {
                    return Ok(value.clone());
                }
                if index == 0 {
                    if let Expr::Var(name) = &expression.node {
                        return Ok(name.clone());
                    }
                    return Err(vec![Diag::error(
                        expression.span.clone(),
                        "metadata key must be a name or string",
                    )]);
                }
            }
            crate::check::compute::constant_scalar(program, bindings, expression)
                .map(|v| v.to_string())
        })
        .collect::<Result<_, _>>()?;
    Ok(())
}
