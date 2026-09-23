//! Structural validation shared by authored pass hooks, before body lowering.

use std::collections::HashSet;

use crate::ast::{PassDecl, Program};
use crate::diag::Diag;

pub(super) fn validate(pass: &PassDecl, program: &Program, diags: &mut Vec<Diag>) {
    let stage = pass.stage.as_ref().map(|stage| stage.node.as_str());

    let interface_fullscreen = pass
        .draw
        .as_ref()
        .is_some_and(|draw| draw.node == "fullscreen")
        && pass.material_name.as_ref().is_some_and(|name| {
            program
                .interfaces
                .iter()
                .any(|interface| interface.name == *name)
        });
    let vertex_return = pass
        .hooks
        .iter()
        .find(|hook| hook.name == "vertex")
        .and_then(|hook| hook.return_ty.as_ref())
        .map(|ty| ty.node.as_str());
    // An empty hook list is the legacy declaration form whose browser wrapper
    // remains supported during migration. Once a pass authors any hook, it opts
    // into the executable contract and must provide the complete stage pair.
    let executable_fullscreen = interface_fullscreen && !pass.hooks.is_empty();
    if executable_fullscreen {
        for required in ["vertex", "shade"] {
            if !pass.hooks.iter().any(|hook| hook.name == required) {
                diags.push(super::pass_error(
                    pass,
                    pass.name_span.clone(),
                    format!(
                        "fullscreen pass `{}` is missing required `{required}` hook",
                        pass.name
                    ),
                ));
            }
        }
        for binding in &pass.bindings {
            let Some(signature) = binding.value_signature.as_deref() else {
                continue;
            };
            let Some(uniform_type) = signature
                .strip_prefix("uniform<")
                .and_then(|value| value.strip_suffix('>'))
            else {
                continue;
            };
            let Some(resource) = program
                .params
                .iter()
                .find(|param| param.name == binding.name)
            else {
                diags.push(
                    super::pass_error(
                        pass,
                        binding.name_span.clone(),
                        format!(
                            "fullscreen pass `{}` requires uniform resource `{}`",
                            pass.name, binding.name
                        ),
                    )
                    .with_help(format!(
                        "declare `param {}: {uniform_type}` in the engine bundle so the host can bind it",
                        binding.name
                    )),
                );
                continue;
            };
            if resource.ty_name != uniform_type {
                diags.push(super::pass_error(
                    pass,
                    binding.span.clone(),
                    format!(
                        "fullscreen pass `{}` binding `{}` expects `{uniform_type}`, but the declared resource has type `{}`",
                        pass.name, binding.name, resource.ty_name
                    ),
                ));
            }
        }
    }
    for hook in &pass.hooks {
        let required_stage = match hook.name.as_str() {
            "vertex" | "shade" => Some("raster"),
            "main" => Some("compute"),
            // Other names are pass-local helpers, not stage entry hooks.
            _ => None,
        };
        if let (Some(stage), Some(required_stage)) = (stage, required_stage)
            && stage != required_stage
        {
            diags.push(super::pass_error(
                pass,
                hook.name_span.clone(),
                format!(
                    "pass `{}` hook `{}` requires stage `{required_stage}`, but the pass declares `{stage}`",
                    pass.name, hook.name,
                ),
            ));
        }

        let mut names = HashSet::new();
        for param in &hook.params {
            if !names.insert(param.name.as_str()) {
                diags.push(super::pass_error(
                    pass,
                    param.name_span.clone(),
                    format!(
                        "duplicate parameter `{}` in pass `{}` hook `{}`",
                        param.name, pass.name, hook.name,
                    ),
                ));
            }
        }

        if executable_fullscreen && hook.name == "vertex" {
            let valid = hook.params.len() == 1
                && hook.params[0].ty_name == "u32"
                && hook.return_ty.is_some();
            if !valid {
                diags.push(super::pass_error(
                    pass,
                    hook.span.clone(),
                    format!(
                        "fullscreen pass `{}` vertex hook must accept `vertex_id: u32` and return a varying struct",
                        pass.name
                    ),
                ));
            }
        }
        if executable_fullscreen && hook.name == "shade" {
            let interface = pass
                .material_name
                .as_deref()
                .expect("interface-backed pass");
            let valid = hook.params.len() == 2
                && vertex_return.is_some_and(|varying| hook.params[0].ty_name == varying)
                && hook.params[1].ty_name == interface
                && hook.return_ty.as_ref().is_some_and(|ty| ty.node == "color");
            if !valid {
                diags.push(super::pass_error(
                    pass,
                    hook.span.clone(),
                    format!(
                        "fullscreen pass `{}` shade hook must accept the vertex varying and `{interface}`, then return `color`",
                        pass.name
                    ),
                ));
            }
        }

        // Interface-backed fullscreen passes are the first family being prepared
        // for execution. Legacy material/lighting passes still carry staged syntax.
        if executable_fullscreen && let Err(errors) = crate::parser::pass_hook_body(hook) {
            for error in errors {
                diags.push(super::pass_error(
                    pass,
                    error.span().clone(),
                    format!(
                        "invalid body of pass `{}` hook `{}`: {:?}",
                        pass.name,
                        hook.name,
                        error.reason(),
                    ),
                ));
            }
        }
    }
}
