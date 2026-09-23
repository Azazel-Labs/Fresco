//! Named, stateless interface implementations and explicitly bound GPU dispatch.
//! All contract and method names are authored vocabulary.
use crate::ast::*;
use crate::diag::Diag;
use logos::Logos;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn contract_type(ty: &str) -> Option<&str> {
    ty.strip_prefix("implementation<")?.strip_suffix('>')
}

pub(super) fn catalog(program: &Program, contract: &str) -> Result<Vec<String>, Vec<Diag>> {
    if program
        .interfaces
        .iter()
        .filter(|i| i.name == contract)
        .count()
        != 1
    {
        return Err(vec![Diag::error(
            0..0,
            format!("implementation reference requires one interface `{contract}`"),
        )]);
    }
    let mut symbols = BTreeSet::new();
    for record in &program.structs {
        let attrs: Vec<_> = record
            .attrs
            .iter()
            .filter(|a| a.name == "implementation")
            .collect();
        if attrs.is_empty() {
            continue;
        }
        let [attr] = attrs.as_slice() else {
            return Err(vec![Diag::error(
                record.span.clone(),
                "duplicate @implementation",
            )]);
        };
        if attr.args.len() != 1 || !record.fields.is_empty() {
            return Err(vec![Diag::error(
                record.span.clone(),
                "@implementation requires one interface and a stateless record",
            )]);
        }
        if attr.args[0] != contract {
            continue;
        }
        let conformances: Vec<_> = program
            .conformances
            .iter()
            .filter(|c| c.type_name == record.name && c.interface_name == contract)
            .collect();
        if conformances.len() != 1 || !symbols.insert(record.name.clone()) {
            return Err(vec![Diag::error(
                record.span.clone(),
                format!(
                    "implementation `{}` requires one unambiguous conformance to `{contract}`",
                    record.name
                ),
            )]);
        }
        if conformances[0].methods.iter().any(|method| {
            !method.type_params.is_empty()
                || !method.const_params.is_empty()
                || method
                    .params
                    .iter()
                    .any(|p| p.keyword_only || p.default.is_some())
        }) {
            return Err(vec![Diag::error(
                record.span.clone(),
                "registered implementations require concrete positional methods",
            )]);
        }
    }
    Ok(symbols.into_iter().collect())
}

fn function_name(symbol: &str, method: &str, specialization: Option<u32>) -> String {
    let name = format!("fresco_implementation_{symbol}_{method}");
    specialization.map_or(name.clone(), |id| format!("{name}_instance{id}"))
}

fn selection_key(
    selection: &fresco_artifact::ManifestImplementationSelection,
) -> (String, String, String) {
    (
        selection.contract.clone(),
        selection.symbol.clone(),
        serde_json::to_string(&selection.static_parameters).expect("serializable static settings"),
    )
}

pub(super) fn prepare(program: &mut Program) -> Result<(), Vec<Diag>> {
    // Validate every registered declaration, even if it is not assigned.
    for record in &program.structs {
        for attribute in &record.attrs {
            if attribute.name == "implementation" && attribute.args.len() != 1 {
                return Err(vec![Diag::error(
                    record.span.clone(),
                    "@implementation requires one interface",
                )]);
            }
        }
    }
    let contracts: BTreeSet<_> = program
        .structs
        .iter()
        .flat_map(|r| &r.attrs)
        .filter(|a| a.name == "implementation")
        .filter_map(|a| a.args.first().cloned())
        .collect();
    for contract in &contracts {
        catalog(program, contract)?;
    }
    let mut errors = Vec::new();
    crate::check::validate_interface_conformance_methods(
        &program.interfaces,
        &program.conformances,
        &mut errors,
    );
    if !errors.is_empty() {
        return Err(errors);
    }
    let used: BTreeSet<_> = program
        .surfaces
        .iter()
        .filter_map(|s| s.settings.as_ref())
        .flat_map(|s| &s.implementations)
        .map(selection_key)
        .collect();
    let mut ids = BTreeMap::new();
    for (index, key) in used.into_iter().enumerate() {
        let id = u32::try_from(index)
            .ok()
            .and_then(|n| n.checked_add(1))
            .ok_or_else(|| vec![Diag::error(0..0, "implementation ID capacity exceeded")])?;
        ids.insert(key, id);
    }
    let mut settings_offset = 0u32;
    let mut surfaces: Vec<_> = program.surfaces.iter_mut().collect();
    surfaces.sort_by(|a, b| a.name.cmp(&b.name));
    for surface in surfaces {
        let Some(settings) = &mut surface.settings else {
            continue;
        };
        for selection in &mut settings.implementations {
            selection.settings_offset = settings_offset;
            settings_offset = u32::try_from(selection.parameters.len())
                .ok()
                .and_then(|n| settings_offset.checked_add(n))
                .ok_or_else(|| vec![Diag::error(0..0, "style settings capacity exceeded")])?;
            selection.id = ids[&selection_key(selection)];
            settings
                .property_u32
                .insert(selection.name.clone(), selection.id);
        }
    }
    for ((contract, symbol, static_key), id) in &ids {
        let conform = program
            .conformances
            .iter()
            .find(|c| &c.interface_name == contract && &c.type_name == symbol)
            .expect("validated selection");
        for method in &conform.methods {
            if !method.type_params.is_empty()
                || !method.const_params.is_empty()
                || method
                    .params
                    .iter()
                    .any(|p| p.keyword_only || p.default.is_some())
            {
                return Err(vec![Diag::error(
                    method.span.clone(),
                    "dispatch implementations require concrete positional signatures",
                )]);
            }
            let mut function = method.clone();
            function.derivative_free = program.styles.iter().any(|style| &style.name == symbol);
            function.name =
                function_name(symbol, &method.name, (static_key != "[]").then_some(*id));
            if let Some(style) = program.styles.iter().find(|s| &s.name == symbol)
                && style.methods.iter().any(|m| m.name == method.name)
            {
                super::styles::add_captures(&mut function, &style.params);
                super::styles::add_shading_captures(&mut function, &style.shading_inputs);
                let parameters: Vec<fresco_artifact::ManifestParam> =
                    serde_json::from_str(static_key).expect("serialized static settings");
                let value = serde_json::json!({"symbol": symbol, "settings": parameters.iter().map(|p| (p.name.clone(), p.default.clone())).collect::<serde_json::Map<_,_>>()});
                let Expr::Call { args, .. } = super::styles::selection_override(
                    program,
                    value.as_object().expect("object"),
                    &style.span,
                )
                .map_err(|message| vec![Diag::error(style.span.clone(), message)])?
                else {
                    unreachable!("style selection call")
                };
                let constants: Vec<_> = args
                    .into_iter()
                    .map(|arg| {
                        let name = arg.name.expect("named setting");
                        let declaration = style
                            .static_params
                            .iter()
                            .find(|p| p.name == name)
                            .expect("static declaration");
                        Stmt::Const {
                            name,
                            name_span: declaration.name_span.clone(),
                            ty_name: declaration.ty_name.clone(),
                            ty_span: declaration.ty_span.clone(),
                            value: arg.value,
                        }
                    })
                    .collect();
                if let Some(body) = &mut function.typed_body {
                    body.splice(0..0, constants.clone());
                }
                function.body.splice(0..0, constants);
            }
            if program.functions.iter().any(|f| f.name == function.name) {
                return Err(vec![Diag::error(
                    method.span.clone(),
                    "generated implementation symbol collision",
                )]);
            }
            program.functions.push(function);
        }
    }
    for pass in &mut program.passes {
        for hook in &mut pass.hooks {
            let attrs: Vec<_> = hook.attrs.iter().filter(|a| a.name == "dispatch").collect();
            if attrs.is_empty() {
                continue;
            }
            let [attr] = attrs.as_slice() else {
                return Err(vec![Diag::error(hook.span.clone(), "duplicate @dispatch")]);
            };
            if !matches!(attr.args.len(), 2..=4)
                || (attr.args.len() == 4 && attr.args[3] != "draw")
                || hook.attrs.len() != 1
            {
                return Err(vec![Diag::error(
                    hook.span.clone(),
                    "@dispatch requires (interface, method[, settings_buffer[, draw]]) on an ordinary function",
                )]);
            }
            let contract = &attr.args[0];
            let method_name = &attr.args[1];
            let method = program
                .interfaces
                .iter()
                .find(|i| &i.name == contract)
                .and_then(|i| i.methods.iter().find(|m| &m.name == method_name))
                .ok_or_else(|| {
                    vec![Diag::error(
                        hook.span.clone(),
                        "unknown dispatch interface method",
                    )]
                })?;
            let prefix_count = if attr.args.len() >= 3 { 2 } else { 1 };
            if prefix_count == 1
                && program.styles.iter().any(|s| {
                    &s.contract == contract
                        && !s.params.is_empty()
                        && ids.keys().any(|(contract, symbol, _)| {
                            contract == &s.contract && symbol == &s.name
                        })
                })
            {
                return Err(vec![Diag::error(
                    hook.span.clone(),
                    "selected style requires explicit settings buffer and record offset in @dispatch",
                )]);
            }
            if (prefix_count == 2 && hook.params.get(1).is_none_or(|p| p.ty_name != "u32"))
                || hook.params.first().is_none_or(|p| p.ty_name != "u32")
                || hook.params.len() != method.params.len() + prefix_count
                || hook.params[prefix_count..]
                    .iter()
                    .zip(&method.params)
                    .any(|(a, b)| a.ty_name != b.ty_name)
                || hook.return_ty.as_ref().map(|t| &t.node)
                    != method.ret_ty.as_ref().map(|(t, _)| t)
            {
                return Err(vec![Diag::error(
                    hook.span.clone(),
                    "dispatch signature must prepend a u32 selector (and u32 settings offset when a buffer is supplied) to the exact interface signature",
                )]);
            }
            let arguments = hook.params[prefix_count..]
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let mut cases = Vec::new();
            for ((selected_contract, symbol, static_key), id) in &ids {
                if selected_contract != contract {
                    continue;
                }
                let mut arguments = arguments.clone();
                if let Some(style) = program.styles.iter().find(|s| &s.name == symbol)
                    && style.methods.iter().any(|m| &m.name == method_name)
                {
                    for (index, param) in style.params.iter().enumerate() {
                        let lane =
                            format!("{}[{} + u32({index})]", attr.args[2], hook.params[1].name);
                        let value = match param.ty_name.as_str() {
                            "f32" => format!("{lane}.x"),
                            "u32" => format!("(u32({lane}.x) | (u32({lane}.y) << u32(16)))"),
                            "i32" => format!("i32(u32({lane}.x) | (u32({lane}.y) << u32(16)))"),
                            "bool" => format!("({lane}.x != 0.0)"),
                            "vec2" => format!("{lane}.xy"),
                            "vec3" => format!("{lane}.xyz"),
                            "vec4" | "color" => lane,
                            _ => unreachable!("validated setting type"),
                        };
                        if !arguments.is_empty() {
                            arguments.push_str(", ");
                        }
                        arguments.push_str(&value);
                    }
                }
                let function =
                    function_name(symbol, method_name, (static_key != "[]").then_some(*id));
                cases.push(PassDispatchCase {
                    shading_inputs: program
                        .styles
                        .iter()
                        .find(|style| {
                            &style.name == symbol
                                && style
                                    .methods
                                    .iter()
                                    .any(|method| &method.name == method_name)
                        })
                        .map(|style| style.shading_inputs.clone())
                        .unwrap_or_default(),
                    selector: *id,
                    function,
                    arguments,
                });
            }
            // The authored body handles IDs outside the generated table. No implicit fallback.
            hook.dispatch = Some(PassDispatchPlan {
                contract: contract.clone(),
                draw_scoped: attr.args.len() == 4,
                cases,
                fallback: hook.body.clone(),
            });
            render_dispatch(hook)?;
            hook.attrs.retain(|a| a.name != "dispatch");
        }
    }
    Ok(())
}

/// Rebuild from the retained plan, never from previously generated branches.
/// Pass specialization can change a case's resource arguments without losing
/// the authored behavior for selectors outside the implementation table.
pub(super) fn render_dispatch(hook: &mut PassFnHookDecl) -> Result<(), Vec<Diag>> {
    let Some(plan) = &hook.dispatch else {
        return Ok(());
    };
    let selector = &hook.params[0].name;
    let mut prefix = String::new();
    for case in &plan.cases {
        let call = format!("{}({})", case.function, case.arguments);
        let action = if hook.return_ty.is_some() {
            format!("return {call};")
        } else {
            format!("{call}; return;")
        };
        prefix.push_str(&format!(
            "if {selector} == u32({}) {{ {action} }}\n",
            case.selector
        ));
    }
    let tokens = crate::lexer::Token::lexer(&prefix)
        .map(|token| {
            token
                .map(|token| (token, hook.span.clone()))
                .map_err(|_| vec![Diag::error(hook.span.clone(), "invalid generated dispatch")])
        })
        .collect::<Result<Vec<_>, _>>()?;
    hook.body = plan.fallback.clone();
    hook.body.splice(1..1, tokens);
    Ok(())
}

/// Only an explicit engine draw-scoped dispatcher can narrow its callable set.
/// General dispatchers keep all cases, including fullscreen deferred dispatch.
pub(super) fn specialize_draw_dispatch(
    pass: &mut PassDecl,
    settings: Option<&SurfaceSettings>,
    factories: &[VertexFactoryDecl],
) -> Result<BTreeMap<String, fresco_artifact::ManifestDrawComputeBinding>, Vec<Diag>> {
    let mut captures = BTreeMap::new();
    let mut declared_captures = BTreeSet::new();
    for hook in &mut pass.hooks {
        let Some(plan) = &mut hook.dispatch else {
            continue;
        };
        if !plan.draw_scoped {
            if plan
                .cases
                .iter()
                .any(|case| !case.shading_inputs.is_empty())
            {
                return Err(vec![Diag::error(
                    hook.span.clone(),
                    "resource-capturing dispatch requires explicit draw scope",
                )]);
            }
            continue;
        }
        let mut selections = settings
            .into_iter()
            .flat_map(|s| &s.implementations)
            .filter(|selection| selection.contract == plan.contract);
        let Some(selected) = selections.next().map(|selection| selection.id) else {
            // Shared engine passes may contain dispatchers for other schemas.
            // Keep an empty plan so a reachable call diagnoses the missing
            // selection without making unused hooks require that contract.
            plan.cases.clear();
            render_dispatch(hook)?;
            continue;
        };
        if selections.next().is_some() || !plan.cases.iter().any(|case| case.selector == selected) {
            return Err(vec![Diag::error(
                hook.span.clone(),
                "draw-scoped dispatch requires one unambiguous implementation case",
            )]);
        }
        plan.cases.retain(|case| case.selector == selected);
        let case = &mut plan.cases[0];
        for input in &case.shading_inputs {
            let sampler = if input.ty.trim() == "sampler" {
                Some(
                    settings
                        .and_then(|s| s.shading_samplers.get(&plan.contract))
                        .and_then(|inputs| inputs.get(&input.name))
                        .copied()
                        .ok_or_else(|| {
                            vec![Diag::error(
                                input.span.clone(),
                                format!("missing sampler for shading input `{}`", input.name),
                            )]
                        })?,
                )
            } else {
                None
            };
            let resource = settings
                .and_then(|s| s.shading_inputs.get(&plan.contract))
                .and_then(|inputs| inputs.get(&input.name));
            if sampler.is_none() && resource.is_none() {
                return Err(vec![Diag::error(
                    input.span.clone(),
                    format!("missing producer for shading input `{}`", input.name),
                )]);
            }
            let name = format!(
                "__fresco_shading_{}_{selected}_{}",
                plan.contract, input.name
            );
            if declared_captures.insert(name.clone()) {
                let factory = pass
                    .attrs
                    .iter()
                    .find(|a| a.name == "factory")
                    .and_then(|a| a.args.first())
                    .and_then(|name| factories.iter().find(|f| &f.name == name))
                    .ok_or_else(|| {
                        vec![Diag::error(
                            pass.span.clone(),
                            "shading capture requires a vertex factory",
                        )]
                    })?;
                if pass
                    .bindings
                    .iter()
                    .chain(factories.iter().flat_map(|f| &f.bindings))
                    .any(|b| b.name == name)
                {
                    return Err(vec![Diag::error(
                        input.span.clone(),
                        "shading capture collides with an authored binding",
                    )]);
                }
                let group = factory
                    .bindings
                    .iter()
                    .find_map(|b| b.group_index)
                    .unwrap_or(0);
                let occupied: BTreeSet<_> = pass
                    .bindings
                    .iter()
                    .chain(factories.iter().flat_map(|f| &f.bindings))
                    .filter(|b| b.group_index == Some(group))
                    .filter_map(|b| b.binding_index)
                    .collect();
                let binding = (0..u32::MAX)
                    .find(|b| !occupied.contains(b))
                    .ok_or_else(|| {
                        vec![Diag::error(
                            input.span.clone(),
                            "shading binding space exhausted",
                        )]
                    })?;
                let signature = if sampler.is_some() {
                    "sampler".into()
                } else {
                    let ty = crate::resource_type::resource_type(&input.ty, "read")
                        .map_err(|message| vec![Diag::error(input.span.clone(), message)])?;
                    super::compute_operations::resource_signature(&ty, false)
                };
                pass.bindings.push(PassBindingDecl {
                    operation_alias: None,
                    group_index: Some(group),
                    binding_index: Some(binding),
                    name: name.clone(),
                    name_span: input.span.clone(),
                    attrs: sampler
                        .map(|value| PipelineAttribute {
                            name: "sampler".into(),
                            args: vec![value.name().into()],
                            expressions: vec![],
                            name_span: input.span.clone(),
                            args_span: None,
                            span: input.span.clone(),
                        })
                        .into_iter()
                        .collect(),
                    value_signature: Some(signature),
                    span: input.span.clone(),
                });
                if let Some(resource) = resource {
                    captures.insert(name.clone(), resource.clone());
                }
            }
            if !case.arguments.is_empty() {
                case.arguments.push_str(", ");
            }
            case.arguments.push_str(&name);
        }
        render_dispatch(hook)?;
    }
    Ok(captures)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chumsky::Parser;

    #[test]
    fn captures_with_equal_local_ids_and_slots_remain_independent_across_contracts() {
        let source = "vertex_factory factory for Mesh {}\n@factory(factory) pass test {\nfn first(id: u32) -> vec4 { return vec4(0.0) }\nfn second(id: u32) -> vec4 { return vec4(0.0) }\n}";
        let tokens = crate::lexer::lex_spanned(source);
        let mut program = crate::parser::program()
            .parse(crate::parser::input(&tokens, source.len()..source.len()))
            .into_result()
            .unwrap();
        let mut settings = SurfaceSettings {
            shading_inputs: Default::default(),
            shading_samplers: Default::default(),
            implementation_slots: Default::default(),
            property_u32: Default::default(),
            recipe_conditions: Default::default(),
            pass_states: Default::default(),
            evaluation_axes: Default::default(),
            properties: vec![],
            usages: vec![],
            implementations: vec![],
        };
        let pass = &mut program.passes[0];
        for (hook, (contract, preset)) in pass.hooks.iter_mut().zip([
            (
                "First",
                fresco_artifact::types::SamplerPreset::NearestRepeat,
            ),
            ("Second", fresco_artifact::types::SamplerPreset::LinearClamp),
        ]) {
            settings.implementations.push(
                serde_json::from_value(serde_json::json!({
                    "name": contract, "contract": contract, "symbol": "Selected", "id": 1,
                    "available": ["Selected"], "editable": true
                }))
                .unwrap(),
            );
            settings.shading_inputs.insert(
                contract.into(),
                BTreeMap::from([(
                    "values".into(),
                    fresco_artifact::ManifestDrawComputeBinding {
                        producer: contract.into(),
                        ty: "buffer<vec4, read>".into(),
                        dimension: None,
                    },
                )]),
            );
            settings.shading_samplers.insert(
                contract.into(),
                BTreeMap::from([("filtering".into(), preset)]),
            );
            hook.dispatch = Some(PassDispatchPlan {
                contract: contract.into(),
                draw_scoped: true,
                fallback: hook.body.clone(),
                cases: vec![PassDispatchCase {
                    selector: 1,
                    function: format!("selected_{contract}"),
                    arguments: String::new(),
                    shading_inputs: [("values", "buffer<vec4, read>"), ("filtering", "sampler")]
                        .into_iter()
                        .map(|(name, ty)| StyleShadingInputDecl {
                            name: name.into(),
                            ty: ty.into(),
                            scope: "draw".into(),
                            span: hook.span.clone(),
                        })
                        .collect(),
                }],
            });
        }
        let captures =
            specialize_draw_dispatch(pass, Some(&settings), &program.vertex_factories).unwrap();
        assert_eq!(captures.len(), 2);
        assert_eq!(pass.bindings.len(), 4);
        for (hook, contract) in pass.hooks.iter().zip(["First", "Second"]) {
            let arguments = &hook.dispatch.as_ref().unwrap().cases[0].arguments;
            let names: Vec<_> = arguments.split(", ").collect();
            assert_eq!(names.len(), 2);
            assert_eq!(captures[names[0]].producer, contract);
            let binding = pass
                .bindings
                .iter()
                .find(|binding| binding.name == names[1])
                .unwrap();
            assert_eq!(
                binding.attrs[0].args,
                [settings.shading_samplers[contract]["filtering"].name()]
            );
        }
    }

    #[test]
    fn only_explicit_draw_dispatch_is_specialized_to_the_material() {
        let source =
            "@shader pass test { fn response(id: u32, value: vec4) -> vec4 { return value } }";
        let tokens = crate::lexer::lex_spanned(source);
        let mut program = crate::parser::program()
            .parse(crate::parser::input(&tokens, source.len()..source.len()))
            .into_result()
            .unwrap();
        let pass = &mut program.passes[0];
        pass.hooks[0].dispatch = Some(PassDispatchPlan {
            contract: "Response".into(),
            draw_scoped: false,
            fallback: pass.hooks[0].body.clone(),
            cases: [1, 2]
                .into_iter()
                .map(|selector| PassDispatchCase {
                    shading_inputs: vec![],
                    selector,
                    function: format!("response_{selector}"),
                    arguments: "value".into(),
                })
                .collect(),
        });
        let settings = SurfaceSettings {
            shading_inputs: Default::default(),
            shading_samplers: Default::default(),
            implementation_slots: Default::default(),
            property_u32: Default::default(),
            recipe_conditions: Default::default(),
            pass_states: Default::default(),
            evaluation_axes: Default::default(),
            properties: vec![],
            usages: vec![],
            implementations: vec![
                serde_json::from_value(serde_json::json!({
                    "name": "style", "contract": "Response", "symbol": "Second", "id": 2,
                    "available": ["First", "Second"], "editable": true
                }))
                .unwrap(),
            ],
        };
        specialize_draw_dispatch(pass, Some(&settings), &[]).unwrap();
        assert_eq!(pass.hooks[0].dispatch.as_ref().unwrap().cases.len(), 2);
        pass.hooks[0].dispatch.as_mut().unwrap().draw_scoped = true;
        let original = pass.clone();
        specialize_draw_dispatch(pass, Some(&settings), &[]).unwrap();
        let plan = pass.hooks[0].dispatch.as_ref().unwrap();
        assert_eq!(plan.cases.len(), 1);
        assert_eq!(plan.cases[0].selector, 2);
        assert!(!format!("{:?}", pass.hooks[0].body).contains("response_1"));
        assert_eq!(original.hooks[0].dispatch.as_ref().unwrap().cases.len(), 2);
        let mut unrelated = original;
        specialize_draw_dispatch(&mut unrelated, None, &[]).unwrap();
        assert!(
            unrelated.hooks[0]
                .dispatch
                .as_ref()
                .unwrap()
                .cases
                .is_empty()
        );
    }

    #[test]
    fn dispatch_rebinding_replaces_generated_cases_and_preserves_fallback() {
        let source = "@shader pass test { fn response(id: u32, value: vec4) -> vec4 { return value * 0.25 } }";
        let tokens = crate::lexer::lex_spanned(source);
        let mut program = crate::parser::program()
            .parse(crate::parser::input(&tokens, source.len()..source.len()))
            .into_result()
            .unwrap();
        let hook = &mut program.passes[0].hooks[0];
        let fallback = hook.body.clone();
        hook.dispatch = Some(PassDispatchPlan {
            contract: "Response".into(),
            draw_scoped: false,
            fallback: fallback.clone(),
            cases: vec![PassDispatchCase {
                shading_inputs: vec![],
                selector: 7,
                function: "selected".into(),
                arguments: "value, first_resource".into(),
            }],
        });
        render_dispatch(hook).unwrap();
        let first = format!("{:?}", hook.body);
        render_dispatch(hook).unwrap();
        assert_eq!(format!("{:?}", hook.body), first);
        hook.dispatch.as_mut().unwrap().cases[0].arguments = "value, second_resource".into();
        render_dispatch(hook).unwrap();
        let rebound = format!("{:?}", hook.body);
        assert!(rebound.contains("second_resource"));
        assert!(!rebound.contains("first_resource"));
        hook.dispatch.as_mut().unwrap().cases.clear();
        render_dispatch(hook).unwrap();
        assert_eq!(format!("{:?}", hook.body), format!("{fallback:?}"));
    }
}
