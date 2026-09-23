//! Reflect logical attachment versions from the composed, checked recipe graph.
use crate::ast::{StyleContractDecl, StylePointDecl};
use fresco_artifact::{
    ManifestRecipeStep, ManifestResourcePort, ManifestResourceVersion, ManifestRoot,
};
use std::collections::BTreeSet;

use super::style_graph::precedes;

pub(super) fn member<'a>(
    contract: &'a StyleContractDecl,
    path: &'a str,
) -> Result<Option<(&'a StylePointDecl, &'a str)>, String> {
    let Some((binding, member)) = path.split_once('.') else {
        return Ok(None);
    };
    for point in &contract.points {
        if super::style_graph::port(point)? == Some(binding) {
            return Ok(Some((point, member)));
        }
    }
    Ok(None)
}

fn uses(step: &ManifestRecipeStep, resource: &str) -> bool {
    step.bindings.values().any(|value| value == resource)
        || step.colors.values().any(|value| value == resource)
        || step.depth.as_deref() == Some(resource)
}

pub(super) fn resolve<D, N>(root: &mut ManifestRoot<D, N>) -> Result<(), String> {
    for renderer in &mut root.renderers {
        let depth_writers: BTreeSet<_> = renderer
            .steps
            .iter()
            .filter(|step| {
                let Some(depth) = &step.depth else {
                    return false;
                };
                // A clear writes regardless of raster depth-write state. Otherwise
                // use the actual state for material routes that can execute the node.
                step.attachments.get(depth).is_none_or(|ops| !ops.load)
                    || root
                        .surfaces
                        .iter()
                        .filter_map(|surface| surface.settings.as_ref())
                        .any(|settings| {
                            let active = step.condition.as_ref().is_none_or(|condition| {
                                settings.recipe_conditions.get(condition).copied() != Some(false)
                            });
                            active
                                && settings
                                    .pass_states
                                    .get(&step.pass)
                                    .and_then(|state| state.depth_write)
                                    != Some(false)
                        })
            })
            .map(|step| step.name.clone())
            .collect();
        renderer.resource_ports =
            reflect(&renderer.resource_ports, &renderer.steps, &depth_writers)?;
    }
    Ok(())
}

fn reflect(
    declarations: &[ManifestResourcePort],
    steps: &[ManifestRecipeStep],
    depth_writers: &BTreeSet<String>,
) -> Result<Vec<ManifestResourcePort>, String> {
    let mut ports = Vec::new();
    for declaration in declarations {
        let ManifestResourcePort {
            point,
            name,
            member,
            resource,
            access,
            incoming,
            outgoing,
            queue,
            storage_writers,
            ..
        } = declaration;
        let mut operations: Vec<_> = steps
            .iter()
            .filter(|step| {
                (step
                    .invocation
                    .as_ref()
                    .is_some_and(|invocation| invocation.point == *point)
                    || (queue.is_some() && step.transparent_queue.as_ref() == queue.as_ref()))
                    && uses(step, resource)
            })
            .collect();
        // The graph, not recipe/source order, determines the version chain.
        // Nodes in one transparent queue are a single runtime-sorted writer.
        for (index, left) in operations.iter().enumerate() {
            for right in operations.iter().skip(index + 1) {
                let same_queue = left.transparent_queue.is_some()
                    && left.transparent_queue == right.transparent_queue;
                let forward = precedes(steps, &left.name, &right.name);
                let reverse = precedes(steps, &right.name, &left.name);
                if forward && reverse {
                    return Err(format!("resource port `{name}` has cyclic writers"));
                }
                if !same_queue && !forward && !reverse && access == "preserve_update" {
                    return Err(format!(
                        "resource port `{name}` has ambiguous writers `{}` and `{}`",
                        left.name, right.name
                    ));
                }
            }
        }
        let operation_names: Vec<_> = operations.iter().map(|step| step.name.clone()).collect();
        operations.sort_by_cached_key(|step| {
            let ancestors = operation_names
                .iter()
                .filter(|name| **name != step.name && precedes(steps, name, &step.name))
                .count();
            (ancestors, step.name.clone())
        });
        let mut versions = vec![ManifestResourceVersion {
            id: 0,
            previous: None,
            producers: incoming.clone(),
            readers: Vec::new(),
        }];
        let mut index = 0;
        while index < operations.len() {
            let operation = operations[index];
            let mut group = vec![operation.name.clone()];
            index += 1;
            if let Some(queue) = &operation.transparent_queue {
                while index < operations.len()
                    && operations[index].transparent_queue.as_ref() == Some(queue)
                {
                    group.push(operations[index].name.clone());
                    index += 1;
                }
            }
            let current = versions.last_mut().expect("initial resource version");
            current.readers.extend(group.clone());
            if access == "preserve_update" {
                let previous = current.id;
                let id = previous
                    .checked_add(1)
                    .ok_or("resource version count exceeds u32")?;
                versions.push(ManifestResourceVersion {
                    id,
                    previous: Some(previous),
                    producers: group,
                    readers: Vec::new(),
                });
            }
        }
        let final_version = versions.last_mut().expect("initial resource version");
        let downstream: Vec<_> = steps
            .iter()
            .filter(|step| {
                uses(step, resource)
                    && outgoing
                        .iter()
                        .any(|node| precedes(steps, node, &step.name))
            })
            .collect();
        final_version.readers.extend(
            downstream
                .iter()
                .filter(|step| {
                    // Once a downstream attachment pass writes, subsequent
                    // consumers read that pass's result rather than this port's
                    // exported version. Keep only the direct use frontier.
                    !downstream.iter().any(|writer| {
                        writer.name != step.name
                            && (writer.colors.values().any(|value| value == resource)
                                || storage_writers.contains(&writer.name)
                                || (writer.depth.as_deref() == Some(resource)
                                    && depth_writers.contains(&writer.name)))
                            && precedes(steps, &writer.name, &step.name)
                    })
                })
                .map(|step| step.name.clone()),
        );
        for version in &mut versions {
            version.producers.sort();
            version.producers.dedup();
            version.readers.sort();
            version.readers.dedup();
        }
        ports.push(ManifestResourcePort {
            point: point.clone(),
            name: name.clone(),
            member: member.clone(),
            resource: resource.clone(),
            access: access.clone(),
            queue: queue.clone(),
            storage_writers: storage_writers.clone(),
            incoming: incoming.clone(),
            outgoing: outgoing.clone(),
            versions,
        });
    }
    ports.sort_by(|left, right| {
        (&left.point, &left.name, &left.member).cmp(&(&right.point, &right.name, &right.member))
    });
    Ok(ports)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    fn engine() -> String {
        include_str!("../../tests/fixtures/engines/style-operations.fr")
            .replace(
                "point finish: Target {",
                "point finish: Target { port: completed;",
            )
            .replace(
                "at finish as target { Paint(geometry: self, target: target.target) }",
                "Paint(geometry: self, target: completed.target)",
            )
    }

    fn files(engine: String) -> HashMap<String, String> {
        HashMap::from([
            ("engine/engine.fr".into(), engine),
            ("main.fr".into(), "surface item(sp: SamplePoint) -> material(Value) { properties { action: Extra }; compose { Value(tint: #fff) } }".into()),
        ])
    }

    #[test]
    fn inferred_calls_reject_unknown_ambiguous_and_shadowed_resources() {
        let engine = engine();
        super::super::compile_bundle_virtual(&files(engine.clone()), "main.fr", false).unwrap();
        for (from, to, expected) in [
            (
                "target: completed.target",
                "target: completed.missing",
                "does not provide",
            ),
            (
                "target: completed.target",
                "target: result",
                "declared resource port",
            ),
            (
                "for self {",
                "for self { let completed = 1u;",
                "shadows an existing graph binding",
            ),
            (
                "for self {",
                "param completed: f32 = 1.0; for self {",
                "shadows resource port",
            ),
            (
                "for self {",
                "for self { at finish as completed {};",
                "integration target shadows",
            ),
            (
                "for self {",
                "for self { static for completed in 0..1 {};",
                "static loop variable shadows",
            ),
            (
                "@draw(project, paint, mesh) @color(0, result) first",
                "@after(display) @draw(project, paint, mesh) @color(0, result) first",
                "cycle",
            ),
        ] {
            let errors = super::super::compile_bundle_virtual(
                &files(engine.replace(from, to)),
                "main.fr",
                false,
            )
            .unwrap_err();
            assert!(
                format!("{errors:?}").contains(expected),
                "{expected}: {errors:?}"
            );
        }
        let ambiguous = engine.replace("fn value(x: f32) -> f32 }",
            "optional point other: Target { port: another; scope: view; accepts: raster_draws; composition: ordered_draws(engine.stable_draw_order); after: complete; before: presentation }; fn value(x: f32) -> f32 }")
            .replace("target: completed.target)", "target: completed.target, extra: another.target)");
        let errors =
            super::super::compile_bundle_virtual(&files(ambiguous), "main.fr", false).unwrap_err();
        assert!(
            format!("{errors:?}").contains("multiple integration boundaries"),
            "{errors:?}"
        );
    }

    #[test]
    fn explicit_and_port_names_cannot_alias_one_attachment_twice() {
        let source = engine()
            .replace("Paint(geometry: self, target: completed.target)",
                "at finish as target { Paint(geometry: self, target: completed.target, extra: target.target) }")
            .replace("draw Paint(geometry: DrawRange, target: attachment<rgba16float, preserve_update>)",
                "draw Paint(geometry: DrawRange, target: attachment<rgba16float, preserve_update>, extra: attachment<rgba16float, preserve_update>)")
            .replace("attachments { target: load_store }", "attachments { target: load_store; extra: load_store }");
        let errors =
            super::super::compile_bundle_virtual(&files(source), "main.fr", false).unwrap_err();
        assert!(
            format!("{errors:?}").contains("attachment arguments must not alias"),
            "{errors:?}"
        );
    }
    #[test]
    fn stable_composition_does_not_follow_material_or_import_order() {
        let engine = engine();
        let style = engine
            .lines()
            .find(|line| line.starts_with("style Extra "))
            .unwrap();
        let base = engine.replace(style, "");
        let first = "surface first(sp: SamplePoint) -> material(Value) { properties { action: Extra }; compose { Value(tint: #fff) } }";
        let second = first.replace("surface first", "surface second");
        let mut recipes = Vec::new();
        for reversed in [false, true] {
            let mut files = files(base.clone());
            files.insert("style.fr".into(), style.into());
            files.insert("first.fr".into(), first.into());
            files.insert("second.fr".into(), second.clone());
            files.insert(
                "main.fr".into(),
                if reversed {
                    "import \"second.fr\"\nimport \"style.fr\"\nimport \"first.fr\""
                } else {
                    "import \"first.fr\"\nimport \"second.fr\"\nimport \"style.fr\""
                }
                .into(),
            );
            let output = super::super::compile_bundle_virtual(&files, "main.fr", false).unwrap();
            let manifest: fresco_artifact::ManifestRoot =
                serde_json::from_str(&output.manifest).unwrap();
            recipes.push(serde_json::to_value(&manifest.renderers).unwrap());
        }
        assert_eq!(recipes[0], recipes[1]);
    }
}
