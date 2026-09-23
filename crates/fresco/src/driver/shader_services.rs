//! Link explicitly exported shader services through typed renderer providers.
use crate::ast::*;
use fresco_artifact::{ManifestRecipeResource, ManifestRecipeStep};
use std::collections::BTreeSet;

pub(super) struct ServiceSource {
    pub node: String,
    pub pass: String,
}

pub(super) fn source(
    program: &Program,
    resources: &[ManifestRecipeResource],
    steps: &[ManifestRecipeStep],
    expression: &SExpr,
    expected: &str,
    factory: Option<&str>,
) -> Result<ServiceSource, String> {
    let Expr::Call {
        name,
        args,
        const_args,
        ..
    } = &expression.node
    else {
        return Err("shader service providers require shader_service(node)".into());
    };
    if name != "shader_service"
        || args.len() != 1
        || args[0].name.is_some()
        || !const_args.is_empty()
    {
        return Err("shader service providers require shader_service(node)".into());
    }
    let node = super::style_graph::path(&args[0].value)?;
    let step = steps
        .iter()
        .find(|s| s.name == node)
        .ok_or_else(|| format!("unknown shader service node `{node}`"))?;
    let pass = program
        .passes
        .iter()
        .find(|p| p.name == step.pass)
        .ok_or("missing shader service pass")?;
    let interface = program
        .interfaces
        .iter()
        .find(|i| i.name == expected && i.entry.is_none())
        .ok_or_else(|| format!("unknown shader service interface `{expected}`"))?;
    if !pass
        .attrs
        .iter()
        .any(|a| a.name == "service" && a.args.as_slice() == [expected])
    {
        return Err(format!(
            "pass `{}` does not export shader service `{expected}`",
            pass.name
        ));
    }
    let methods = interface.methods.iter().map(|m| m.name.clone()).collect();
    let closure = super::mesh_pass::service_closure(program, pass, Some((expected, &methods)), "")?;
    for name in closure.resources {
        let bindings = super::style_graph::bindings(program, step, factory);
        let binding = bindings
            .iter()
            .find(|b| b.name == name)
            .ok_or_else(|| format!("missing shader service binding `{name}`"))?;
        let ty = binding
            .value_signature
            .as_deref()
            .ok_or("untyped shader service binding")?;
        if binding
            .attrs
            .iter()
            .any(|a| a.name == "access" && a.args.iter().any(|s| s != "read"))
        {
            return Err("shader service captures must be read-only".into());
        }
        let endpoint = Spanned {
            node: Expr::Var(format!("{node}.{name}")),
            span: expression.span.clone(),
        };
        super::style_graph::bound_resource(program, resources, steps, &endpoint, ty, factory)?;
    }
    Ok(ServiceSource {
        node,
        pass: pass.name.clone(),
    })
}

fn attribute(name: &str, args: Vec<String>) -> PipelineAttribute {
    PipelineAttribute {
        name: name.into(),
        args,
        expressions: Vec::new(),
        name_span: 0..0,
        args_span: None,
        span: 0..0,
    }
}

/// Rewrite only resolved call sites. Source spans preserve overload resolution,
/// grouping, contextual identifiers, and line breaks without textual name search.
pub(super) fn calls(
    pass: &mut PassDecl,
    input: &str,
    prefix: &str,
    uses: &super::mesh_pass::OperationUse,
) -> Result<BTreeSet<String>, String> {
    let mut methods = BTreeSet::new();
    for (index, hook) in pass.hooks.iter_mut().enumerate() {
        if !uses.reachable.contains(&index) {
            continue;
        }
        let mut sites: Vec<_> = uses
            .service_calls
            .get(&index)
            .into_iter()
            .flatten()
            .filter(|call| call.input == input)
            .collect();
        sites.sort_by_key(|call| std::cmp::Reverse(call.method_span.start));
        for call in sites {
            let mut start = hook
                .body
                .iter()
                .position(|(_, span)| span.start == call.receiver.start)
                .ok_or("missing resolved service receiver")?;
            let end = hook
                .body
                .iter()
                .position(|(_, span)| span.end == call.method_span.end)
                .ok_or("missing resolved service method")?
                + 1;
            // Grouping is semantically transparent in the AST. Include its
            // matching source delimiters when replacing the callable prefix.
            let groups = hook.body[start..end]
                .iter()
                .filter(|(t, _)| *t == crate::lexer::Token::RParen)
                .count();
            for _ in 0..groups {
                loop {
                    start = start
                        .checked_sub(1)
                        .ok_or("unbalanced service receiver grouping")?;
                    if hook.body[start].0 != crate::lexer::Token::Newline {
                        break;
                    }
                }
                if hook.body[start].0 != crate::lexer::Token::LParen {
                    return Err("unbalanced service receiver grouping".into());
                }
            }
            let span = hook.body[start].1.start..call.method_span.end;
            hook.body.splice(
                start..end,
                [(
                    crate::lexer::Token::Ident(format!("{prefix}{}", call.method).into()),
                    span,
                )],
            );
            methods.insert(call.method.clone());
        }
    }
    Ok(methods)
}

pub(super) struct DrawLink<'a> {
    pub renderer: &'a mut PipelineDecl,
    pub pass: &'a mut PassDecl,
    pub reference: &'a mut PipelinePassRef,
    pub attributes: &'a mut Vec<PipelineAttribute>,
    pub uses: &'a super::mesh_pass::OperationUse,
}

pub(super) fn link(
    program: &Program,
    style: &StyleDecl,
    parameter: &StyleInputDecl,
    argument: &SExpr,
    point: &str,
    target: DrawLink<'_>,
) -> Result<Vec<PassFnHookDecl>, String> {
    let DrawLink {
        renderer,
        pass,
        reference,
        attributes,
        uses,
    } = target;
    let contract = program
        .style_contracts
        .iter()
        .find(|c| c.name == style.contract)
        .ok_or("missing service contract")?;
    let name = super::style_graph::path(argument)?;
    let provider = program
        .style_providers
        .iter()
        .find(|p| p.contract == style.contract && p.renderer == renderer.name)
        .ok_or("missing shader service provider")?;
    let expression = if let Some((capability, _)) =
        super::prepared_geometry::member(program, contract, &name)?
    {
        let values = &provider
            .blocks
            .iter()
            .find(|b| b.name == capability.name)
            .and_then(|b| b.fields.iter().find(|f| f.name == name))
            .ok_or("missing shader service capability binding")?
            .values;
        let [expression] = values.as_slice() else {
            return Err("service capability requires one source".into());
        };
        expression
    } else {
        &provider
            .inputs
            .iter()
            .find(|(n, _)| *n == name)
            .ok_or("missing shader service contract input")?
            .1
    };
    let (resources, steps) = super::recipes::reflect(renderer)?;
    let selected = source(program, &resources, &steps, expression, &parameter.ty, None)?;
    let source_pass = program
        .passes
        .iter()
        .find(|p| p.name == selected.pass)
        .ok_or("missing service source pass")?;
    let step = steps
        .iter()
        .find(|s| s.name == selected.node)
        .ok_or("missing service source node")?;
    let prefix = format!("__service_{}_{}_", pass.name, parameter.name);
    let methods = calls(pass, &parameter.name, &prefix, uses)?;
    let closure = super::mesh_pass::service_closure(
        program,
        source_pass,
        Some((&parameter.ty, &methods)),
        &prefix,
    )?;
    let incoming = renderer
        .attrs
        .iter()
        .find(|a| a.name == "__style_incoming" && a.args.first().is_some_and(|n| n == point))
        .ok_or("service invocation requires an explicit integration boundary")?;
    let mut incoming = incoming.args[1..].to_vec();
    if let Some(queue) = renderer
        .attrs
        .iter()
        .find(|a| a.name == "__style_queue" && a.args.first().is_some_and(|n| n == point))
        .and_then(|a| a.args.get(1))
    {
        let members: BTreeSet<_> = steps
            .iter()
            .filter(|s| s.transparent_queue.as_ref() == Some(queue))
            .map(|s| &s.name)
            .collect();
        for step in steps.iter().filter(|s| members.contains(&s.name)) {
            incoming.extend(step.after.iter().filter(|n| !members.contains(n)).cloned());
        }
    }
    for name in closure.resources {
        let source_bindings = super::style_graph::bindings(program, step, None);
        let binding = source_bindings
            .iter()
            .find(|b| b.name == name)
            .ok_or("missing service capture binding")?;
        let ty = binding
            .value_signature
            .as_ref()
            .ok_or("untyped service capture")?;
        let resource = step
            .bindings
            .get(&name)
            .ok_or("unbound shader service capture")?;
        let declaration = resources
            .iter()
            .find(|r| r.name == *resource)
            .ok_or("unknown service resource")?;
        if !matches!(declaration.kind.as_str(), "external" | "table_data") {
            let ready = steps.iter().any(|s| {
                let writes = s.colors.values().any(|r| r == resource)
                    || s.depth.as_ref() == Some(resource)
                    || super::style_graph::bindings(program, s, None)
                        .iter()
                        .any(|b| {
                            s.bindings.get(&b.name) == Some(resource)
                                && b.attrs.iter().any(|a| {
                                    a.name == "access"
                                        && a.args
                                            .iter()
                                            .any(|v| matches!(v.as_str(), "write" | "read_write"))
                                })
                        });
                writes
                    && s.condition.is_none()
                    && s.attachments.get(resource).is_none_or(|a| a.store)
                    && incoming
                        .iter()
                        .any(|n| super::style_graph::precedes(&steps, &s.name, n))
            });
            if !ready {
                return Err(format!(
                    "shader service resource `{resource}` is not initialized before `{point}`"
                ));
            }
        }
        let linked_name = format!("{prefix}{name}");
        pass.service_captures.push(StyleInputDecl {
            name: linked_name.clone(),
            ty: ty.clone(),
            span: argument.span.clone(),
        });
        super::style_operations::ensure_binding(pass, program, &linked_name, ty)
            .map_err(|d| format!("invalid service binding: {d:?}"))?;
        renderer.attrs.push(attribute(
            "__style_input",
            vec![
                point.into(),
                linked_name.clone(),
                resource.clone(),
                ty.clone(),
            ],
        ));
        attributes.push(attribute("input", vec![linked_name.clone(), ty.clone()]));
        reference
            .attrs
            .push(attribute("bind", vec![linked_name.clone(), linked_name]));
    }
    Ok(closure.hooks)
}
