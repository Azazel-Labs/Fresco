//! Compose compiler-generated operation invocations at checked provider boundaries.
//! These private graph records are not an alternative authoring syntax.
use crate::{
    ast::{PipelineAttribute, Program, Spanned},
    diag::Diag,
};
use std::collections::{BTreeMap, BTreeSet};

// Run before provider/operation expansion, including declarations in imported engines.
pub(super) fn reject_legacy_syntax(program: &Program) -> Result<(), Vec<Diag>> {
    let mut diagnostics = Vec::new();
    for pipeline in &program.pipelines {
        for attribute in pipeline
            .attrs
            .iter()
            .chain(pipeline.pass_refs.iter().flat_map(|r| &r.attrs))
        {
            if matches!(
                attribute.name.as_str(),
                "contribute"
                    | "stage"
                    | "stage_input"
                    | "stage_join"
                    | "stage_next"
                    | "stage_queue"
            ) {
                diagnostics.push(Diag::error(attribute.span.clone(), format!(
                    "@{} style integration was removed; declare a typed contract point and renderer provide block, then call a draw operation inside `for self {{ at point as target {{ ... }} }}`; see LANGUAGE.md",
                    attribute.name
                )));
            } else if attribute.name.starts_with("__style_") {
                diagnostics.push(Diag::error(attribute.span.clone(), "compiler operation metadata cannot be authored; use typed contract points and operation calls"));
            }
        }
    }
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

fn attribute(name: &str, args: Vec<String>) -> PipelineAttribute {
    PipelineAttribute {
        name: name.into(),
        args,
        expressions: vec![],
        name_span: 0..0,
        args_span: None,
        span: 0..0,
    }
}
fn node(reference: &crate::ast::PipelinePassRef) -> String {
    reference
        .attrs
        .iter()
        .find(|a| a.name == "node")
        .and_then(|a| a.args.first())
        .unwrap_or(&reference.name)
        .clone()
}

fn queue(reference: &crate::ast::PipelinePassRef) -> Option<&String> {
    reference
        .attrs
        .iter()
        .find(|attr| attr.name == "transparent_queue")
        .and_then(|attr| attr.args.first())
}

pub(super) fn prepare(program: &mut Program) -> Result<(), Vec<Diag>> {
    compose(program).map_err(|message| vec![Diag::error(0..0, message)])
}
fn compose(program: &mut Program) -> Result<(), String> {
    // Validate published ports even if no implementation currently consumes them.
    for pipeline in &program.pipelines {
        let mut stages = BTreeSet::new();
        for attr in pipeline.attrs.iter().filter(|a| a.name == "__style_point") {
            let [name] = attr.args.as_slice() else {
                return Err("invalid checked operation point identity".into());
            };
            if !stages.insert(name.clone()) {
                return Err(format!("duplicate integration point `{name}`"));
            }
        }
        let mut ports = BTreeSet::new();
        for attr in pipeline.attrs.iter().filter(|a| a.name == "__style_input") {
            let [stage, name, resource, _ty] = attr.args.as_slice() else {
                return Err("invalid checked operation input binding".into());
            };
            if !stages.contains(stage) || !ports.insert((stage, name)) {
                return Err("stage input names an unknown stage or duplicates an input".into());
            }
            if !pipeline.attrs.iter().any(|a| {
                matches!(
                    a.name.as_str(),
                    "image" | "buffer" | "external" | "table_data"
                ) && a.args.first() == Some(resource)
            }) {
                return Err(format!(
                    "stage input references undeclared resource `{resource}`"
                ));
            }
        }
    }
    let mut contributions: Vec<_> = program
        .pipelines
        .iter()
        .filter(|p| p.attrs.iter().any(|a| a.name == "__style_operation"))
        .cloned()
        .collect();
    // The engine's stable draw policy uses checked invocation identity; generated
    // symbol spelling is not part of the ordering contract.
    contributions.sort_by_cached_key(|pipeline| {
        pipeline
            .pass_refs
            .iter()
            .filter_map(|reference| reference.invocation.as_ref())
            .map(|invocation| {
                (
                    invocation.material.clone(),
                    invocation.ordinal,
                    invocation.operation.clone(),
                )
            })
            .collect::<Vec<_>>()
    });
    let mut previous = BTreeMap::<(String, String), Vec<String>>::new();
    let selected: BTreeSet<_> = program
        .surfaces
        .iter()
        .filter_map(|s| s.settings.as_ref())
        .flat_map(|s| &s.implementations)
        .map(|s| (s.contract.clone(), s.symbol.clone()))
        .collect();
    for contribution in &contributions {
        let markers: Vec<_> = contribution
            .attrs
            .iter()
            .filter(|a| a.name == "__style_operation")
            .collect();
        let [marker] = markers.as_slice() else {
            return Err(
                "operation graph requires one checked contract/implementation/point identity"
                    .into(),
            );
        };
        let [contract, symbol, port] = marker.args.as_slice() else {
            return Err("invalid checked operation identity".into());
        };
        if !super::implementations::catalog(program, contract)
            .map_err(|_| format!("invalid contribution contract `{contract}`"))?
            .contains(symbol)
        {
            return Err(format!(
                "contribution references unregistered implementation `{symbol}`"
            ));
        }
        if contribution.pass_refs.is_empty()
            || contribution
                .pass_refs
                .iter()
                .any(|r| r.invocation.is_none())
        {
            return Err("operation composition requires checked invocation identity".into());
        }
        let mut inputs = BTreeMap::new();
        let mut validation = contribution.clone();
        validation.attrs.clear();
        for attr in &contribution.attrs {
            match attr.name.as_str() {
                "__style_operation" => {}
                "input" if attr.args.len() == 2 => {
                    if inputs
                        .insert(attr.args[0].clone(), attr.args[1].clone())
                        .is_some()
                    {
                        return Err("duplicate contribution input".into());
                    }
                    validation
                        .attrs
                        .push(attribute("external", attr.args.clone()));
                }
                _ => return Err(format!("unsupported contribution attribute @{}", attr.name)),
            }
        }
        let (_, steps) = super::recipes::reflect(&validation)?;
        if steps.iter().any(|s| s.domain != "mesh") {
            return Err("mesh contributions require mesh draw scope".into());
        }
        // A contribution must declare attachment lifetime explicitly.
        if steps.iter().any(|s| {
            s.colors
                .values()
                .chain(s.depth.iter())
                .any(|r| !s.attachments.contains_key(r))
        }) {
            return Err("contributed draws require explicit @attachment operations".into());
        }
        if !selected.contains(&(contract.clone(), symbol.clone())) {
            continue;
        }
        let mut installed = false;
        for pipeline in program
            .pipelines
            .iter_mut()
            .filter(|p| p.attrs.iter().any(|a| a.name == "selected_renderer"))
        {
            let ports: Vec<_> = pipeline
                .attrs
                .iter()
                .filter(|a| a.name == "__style_point" && a.args.first() == Some(port))
                .cloned()
                .collect();
            let [_stage] = ports.as_slice() else {
                return Err(format!(
                    "selected pipeline must provide exactly one stage `{port}` for `{symbol}`"
                ));
            };
            let transparent_queue = pipeline
                .attrs
                .iter()
                .find(|a| a.name == "__style_queue" && a.args.first() == Some(port))
                .and_then(|a| a.args.get(1))
                .cloned();
            if transparent_queue.is_some() && steps.iter().any(|step| !step.after.is_empty()) {
                return Err(
                    "globally sorted contributions cannot impose internal node ordering".into(),
                );
            }
            let mut incoming: Vec<_> = pipeline
                .attrs
                .iter()
                .find(|a| a.name == "__style_incoming" && a.args.first() == Some(port))
                .ok_or("operation point is missing its checked incoming boundary")?
                .args[1..]
                .to_vec();
            let outgoing_boundary = pipeline
                .attrs
                .iter()
                .find(|a| a.name == "__style_outgoing" && a.args.first() == Some(port));
            let outgoing: Vec<_> = outgoing_boundary
                .ok_or("operation point is missing its checked outgoing boundary")?
                .args[1..]
                .to_vec();
            let previous_nodes = previous
                .get(&(pipeline.name.clone(), port.clone()))
                .cloned()
                .unwrap_or_default();
            let previous_nodes = if transparent_queue.is_some() {
                Vec::new()
            } else {
                previous_nodes
            };
            if let Some(name) = &transparent_queue {
                for reference in pipeline
                    .pass_refs
                    .iter()
                    .filter(|reference| queue(reference) == Some(name))
                {
                    for dependency in reference
                        .attrs
                        .iter()
                        .filter(|attr| attr.name == "after")
                        .filter_map(|attr| attr.args.first())
                    {
                        if !incoming.contains(dependency) {
                            incoming.push(dependency.clone());
                        }
                    }
                }
                for reference in pipeline
                    .pass_refs
                    .iter_mut()
                    .filter(|reference| queue(reference) == Some(name))
                {
                    for dependency in &incoming {
                        if !reference.attrs.iter().any(|attr| {
                            attr.name == "after" && attr.args.first() == Some(dependency)
                        }) {
                            reference
                                .attrs
                                .push(attribute("after", vec![dependency.clone()]));
                        }
                    }
                }
            }
            let mut provided = BTreeMap::new();
            for attr in pipeline
                .attrs
                .iter()
                .filter(|a| a.name == "__style_input" && a.args.first() == Some(port))
            {
                let [_, name, resource, ty] = attr.args.as_slice() else {
                    return Err("invalid checked operation input binding".into());
                };
                if provided
                    .insert(name.clone(), (resource.clone(), ty.clone()))
                    .is_some()
                {
                    return Err("duplicate stage input".into());
                }
            }
            let mut resources = BTreeMap::new();
            for (name, ty) in &inputs {
                let (resource, provided_ty) = provided
                    .get(name)
                    .ok_or_else(|| format!("stage `{port}` lacks input `{name}`"))?;
                if ty.split_whitespace().collect::<String>()
                    != provided_ty.split_whitespace().collect::<String>()
                {
                    return Err(format!(
                        "stage input `{name}` requires `{ty}`, found `{provided_ty}`"
                    ));
                }
                resources.insert(name.clone(), resource.clone());
            }
            let prefix = format!("{}__{}", pipeline.name, contribution.name);
            let nodes: BTreeMap<_, _> = contribution
                .pass_refs
                .iter()
                .map(|r| {
                    let name = node(r);
                    (name.clone(), format!("{prefix}__{name}"))
                })
                .collect();
            let terminals: Vec<_> = steps
                .iter()
                .filter(|s| !steps.iter().any(|other| other.after.contains(&s.name)))
                .map(|s| nodes[&s.name].clone())
                .collect();
            // Consumers of the published stage wait for the complete contribution.
            let outgoing_queues: BTreeSet<_> = pipeline
                .pass_refs
                .iter()
                .filter(|reference| outgoing.contains(&node(reference)))
                .filter_map(queue)
                .cloned()
                .collect();
            for reference in &mut pipeline.pass_refs {
                if transparent_queue.is_some() && queue(reference) == transparent_queue.as_ref() {
                    continue;
                }
                if outgoing.contains(&node(reference))
                    || queue(reference).is_some_and(|name| outgoing_queues.contains(name))
                {
                    for terminal in &terminals {
                        reference
                            .attrs
                            .push(attribute("after", vec![terminal.clone()]));
                    }
                }
            }
            let mut expanded = Vec::new();
            for reference in &contribution.pass_refs {
                let mut reference = reference.clone();
                let original_node = node(&reference);
                reference.attrs.retain(|a| a.name != "node");
                for attr in &mut reference.attrs {
                    let index = match attr.name.as_str() {
                        "bind" | "color" => Some(1),
                        "depth" | "attachment" => Some(0),
                        _ => None,
                    };
                    if let Some(index) = index {
                        let name = attr
                            .args
                            .get_mut(index)
                            .ok_or("malformed contribution binding")?;
                        *name = resources
                            .get(name)
                            .ok_or_else(|| format!("undeclared contribution input `{name}`"))?
                            .clone();
                    }
                    if attr.name == "after" {
                        let target = nodes
                            .get(&attr.args[0])
                            .ok_or("contribution dependency escapes its graph")?;
                        attr.args[0].clone_from(target);
                    }
                }
                reference
                    .attrs
                    .push(attribute("node", vec![nodes[&original_node].clone()]));
                if let Some(name) = &transparent_queue {
                    reference
                        .attrs
                        .push(attribute("transparent_queue", vec![name.clone()]));
                }
                for dependency in incoming.iter().chain(&previous_nodes) {
                    reference
                        .attrs
                        .push(attribute("after", vec![dependency.clone()]));
                }
                reference.attrs.push(attribute(
                    "implementation_when",
                    vec![contract.clone(), symbol.clone()],
                ));
                expanded.push(reference);
            }
            let index = if let Some(name) = &transparent_queue {
                pipeline
                    .pass_refs
                    .iter()
                    .rposition(|reference| queue(reference) == Some(name))
                    .ok_or("transparent provider references an unavailable queue")?
                    + 1
            } else {
                pipeline
                    .pass_refs
                    .iter()
                    .rposition(|r| incoming.contains(&node(r)) || previous_nodes.contains(&node(r)))
                    .expect("validated stage")
                    + 1
            };
            pipeline.pass_refs.splice(index..index, expanded);
            previous.insert((pipeline.name.clone(), port.clone()), terminals);
            pipeline.passes = pipeline
                .pass_refs
                .iter()
                .map(|r| Spanned {
                    node: r.name.clone(),
                    span: r.name_span.clone(),
                })
                .collect();
            installed = true;
        }
        if !installed {
            return Err(format!(
                "implementation `{symbol}` requires an executable stage `{port}`"
            ));
        }
    }
    program
        .pipelines
        .retain(|p| !p.attrs.iter().any(|a| a.name == "__style_operation"));
    for pipeline in &mut program.pipelines {
        pipeline.attrs.retain(|a| {
            !matches!(
                a.name.as_str(),
                "__style_point"
                    | "__style_input"
                    | "__style_incoming"
                    | "__style_outgoing"
                    | "__style_queue"
            )
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    const ENGINE: &str = include_str!("../../tests/fixtures/engines/style-operations.fr");
    fn compile(
        engine: &str,
        choice: &str,
    ) -> Result<super::super::CompileBundleOutput, Vec<super::super::DiagnosticRecord>> {
        let files = HashMap::from([
            ("engine/engine.fr".into(), engine.into()),
            (
                "main.fr".into(),
                format!(
                    "surface item(sp: SamplePoint) -> material(Value) {{ properties {{ action: {choice} }}; compose {{ Value(tint: #fff) }} }}"
                ),
            ),
        ]);
        super::super::compile_bundle_virtual(&files, "main.fr", false)
    }
    #[test]
    fn removed_style_attributes_report_migration_even_when_unused() {
        for attribute in [
            "@contribute(Action, Extra, finish)",
            "@stage(finish, first)",
            "@stage_input(finish, target, result, Pixels)",
            "@stage_join(finish, first)",
            "@stage_next(finish, display)",
            "@stage_queue(finish, queue)",
        ] {
            let engine = ENGINE.replace(
                "pipeline(postprocess) main_plan",
                &format!("{attribute} pipeline(postprocess) main_plan"),
            );
            let errors = compile(&engine, "Plain").unwrap_err();
            assert!(
                errors
                    .iter()
                    .any(|e| e.message.contains("style integration was removed")
                        && e.message.contains("LANGUAGE.md")),
                "{attribute}: {errors:?}"
            );
        }
        let engine = ENGINE.replace(
            "pipeline(postprocess) main_plan",
            "@__style_operation(Action, Extra, finish) pipeline(postprocess) main_plan",
        );
        let errors = compile(&engine, "Plain").unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("cannot be authored")),
            "{errors:?}"
        );
    }

    #[test]
    fn obsolete_profile_configuration_reports_symbolic_selection_guidance() {
        let files = HashMap::from([
            ("engine/engine.fr".into(), ENGINE.into()),
            ("main.fr".into(), "surface item(sp: SamplePoint) -> material(Value) { compose { Value(tint: #fff) } }".into()),
            ("fresco.config.json".into(), r#"{"style_profile":"Extra"}"#.into()),
        ]);
        let errors = super::super::compile_bundle_virtual(&files, "main.fr", false).unwrap_err();
        assert!(
            errors.iter().any(|e| e.file == "fresco.config.json"
                && e.message.contains("unknown field")
                && e.message.contains("property_overrides")),
            "{errors:?}"
        );
    }

    #[test]
    fn selected_operations_compose_ordered_preserving_draws() {
        for (choice, count) in [("Plain", 2), ("Extra", 3)] {
            let output = compile(ENGINE, choice).unwrap();
            let manifest: fresco_artifact::ManifestRoot =
                serde_json::from_str(&output.manifest).unwrap();
            let recipe = manifest.renderers.iter().find(|r| r.selected).unwrap();
            assert_eq!(recipe.steps.len(), count);
            if count == 3 {
                let step = &recipe.steps[1];
                assert_eq!(step.invocation.as_ref().unwrap().operation, "Paint");
                assert!(step.after.contains(&"first".into()));
                assert!(step.attachments["result"].load);
                assert!(step.attachments["result"].store);
                assert!(
                    manifest.surfaces[0]
                        .settings
                        .as_ref()
                        .unwrap()
                        .recipe_conditions[step.condition.as_ref().unwrap()]
                );
            }
        }
    }
    #[test]
    fn imported_helpers_accept_the_factory_uniform_record_identity() {
        let engine = format!("{}\nstruct Data {{ scale: f32 }}\nfn place(point: vec3, data: Data) -> vec4 {{ return vec4(point * data.scale, 1.0) }}", ENGINE)
            .replace("vertex_factory plain for Triangle {", "vertex_factory plain for Triangle { binding { @group(2) @binding(0) data: uniform<Data> }")
            .replacen("Projected(vec4(v.point, 1.0))", "Projected(place(v.point, data))", 1)
            .replace("@color(0, result) first", "@color(0, result) @bind(data, input) first")
            .replace("@image(result, rgba16float)", "@image(result, rgba16float) @external(input, data)");
        let output = compile(&engine, "Plain").unwrap();
        assert!(
            output
                .wgsl
                .contains("FrescoMeshResource_plain_item_first_Data")
        );
        assert!(!output.wgsl.contains("import_type_Data"));
    }

    #[test]
    fn invalid_operation_contracts_fail_instead_of_dropping_work() {
        for (engine, message) in [
            (ENGINE.replace("complete: all(first)", "complete: all(missing)"), "unknown integration node `missing`"),
            (ENGINE.replace("target: target.target", "target: self"), "operation resource `self` does not provide `attachment<rgba16float,preserve_update>`"),
            (ENGINE.replace("target: result;", ""), "missing integration field `target`"),
            (ENGINE.replace("attachments { target: load_store }", ""), "missing explicit attachment preservation policy"),
            (ENGINE.replacen("@draw(project, paint, mesh) @color(0, result)", "@draw(project, paint, mesh) @color(0, result) @attachment(result, clear, discard)", 1), "unconditional stored producer"),
        ] {
            let errors = compile(&engine, "Extra").unwrap_err();
            assert!(errors.iter().any(|e| e.message.contains(message)), "{errors:?}");
        }
    }
}
