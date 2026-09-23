//! Engine-authored renderer catalog and compile configuration selection.
use crate::{
    ast::{Expr, PipelineAttribute, PipelineDecl, Program},
    diag::Diag,
};
use fresco_artifact::ManifestRenderer;
use std::collections::BTreeSet;

pub(super) fn catalog(pipelines: &[PipelineDecl]) -> Result<Vec<ManifestRenderer>, String> {
    let mut result = Vec::new();
    let mut ids = BTreeSet::new();
    for pipeline in pipelines {
        let attrs: Vec<_> = pipeline
            .attrs
            .iter()
            .filter(|a| a.name == "renderer")
            .collect();
        if attrs.is_empty() {
            continue;
        }
        let [attr] = attrs.as_slice() else {
            return Err("duplicate renderer declaration".into());
        };
        if attr.args.len() != 2 || attr.args.iter().any(String::is_empty) {
            return Err("@renderer requires an identifier and label".into());
        }
        if !ids.insert(attr.args[0].clone()) {
            return Err(format!("duplicate renderer `{}`", attr.args[0]));
        }
        let defaults: Vec<_> = pipeline
            .attrs
            .iter()
            .filter(|a| a.name == "default")
            .collect();
        if defaults.len() > 1 || defaults.iter().any(|a| !a.args.is_empty()) {
            return Err("renderer requires at most one argument-free @default".into());
        }
        let (resources, steps) = super::recipes::reflect(pipeline)?;
        result.push(ManifestRenderer {
            resource_ports: pipeline.resource_ports.clone(),
            resources,
            steps,
            id: attr.args[0].clone(),
            label: attr.args[1].clone(),
            pipeline: pipeline.name.clone(),
            selected: pipeline.attrs.iter().any(|a| a.name == "selected_renderer"),
            default: pipeline.attrs.iter().any(|a| a.name == "default"),
            passes: pipeline.passes.iter().map(|p| p.node.clone()).collect(),
        });
    }
    if !result.is_empty() && result.iter().filter(|r| r.default).count() != 1 {
        return Err("engine renderer catalog requires exactly one @default".into());
    }
    Ok(result)
}

pub(super) fn select(program: &mut Program, requested: Option<&str>) -> Result<(), Vec<Diag>> {
    let entries = catalog(&program.pipelines).map_err(|e| vec![Diag::error(0..0, e)])?;
    if entries.is_empty() {
        if let Some(name) = requested {
            return Err(vec![Diag::error(
                0..0,
                format!("engine has no renderer `{name}`"),
            )]);
        }
        return Ok(());
    }
    let selected = match requested {
        Some(name) => entries.iter().find(|e| e.id == name).ok_or_else(|| {
            vec![Diag::error(
                0..0,
                format!(
                    "unknown renderer `{name}`; available: {}",
                    entries
                        .iter()
                        .map(|e| e.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )]
        })?,
        None => entries
            .iter()
            .find(|e| e.default)
            .expect("validated default"),
    };
    for pipeline in &mut program.pipelines {
        pipeline
            .attrs
            .retain(|attr| attr.name != "selected_renderer");
    }
    let pipeline = program
        .pipelines
        .iter_mut()
        .find(|p| p.name == selected.pipeline)
        .expect("catalog pipeline");
    let mut configured = BTreeSet::new();
    for attr in pipeline.attrs.iter().filter(|a| a.name == "configure") {
        let [name, value] = attr.expressions.as_slice() else {
            return Err(vec![Diag::error(
                attr.span.clone(),
                "@configure requires (constant, expression)",
            )]);
        };
        let Expr::Var(name) = &name.node else {
            return Err(vec![Diag::error(
                name.span.clone(),
                "configuration target must be a constant name",
            )]);
        };
        if !configured.insert(name.clone()) {
            return Err(vec![Diag::error(
                attr.span.clone(),
                format!("duplicate configuration `{name}`"),
            )]);
        }
        let declaration = program
            .consts
            .iter_mut()
            .find(|c| c.name == *name)
            .ok_or_else(|| {
                vec![Diag::error(
                    attr.span.clone(),
                    format!("unknown renderer configuration constant `{name}`"),
                )]
            })?;
        declaration.value = value.clone();
    }
    pipeline.attrs.push(PipelineAttribute {
        name: "selected_renderer".into(),
        name_span: pipeline.name_span.clone(),
        args: Vec::new(),
        args_span: None,
        expressions: Vec::new(),
        span: pipeline.span.clone(),
    });
    Ok(())
}
