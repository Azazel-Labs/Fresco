//! Resolve authored GPU state using the existing bounded constant evaluator.
use crate::{
    ast::{Expr, Program, Spanned, Stmt, SurfaceSettings},
    diag::Diag,
};
use fresco_artifact::ManifestRasterState;
use std::collections::{BTreeMap, HashSet};

fn applies(program: &Program, surface: &crate::ast::SurfaceDecl, target: Option<&str>) -> bool {
    let Some(target) = target else {
        return true;
    };
    let mut name = match &surface.material_ty {
        crate::ast::MaterialReturnTy::Named(name) => Some(name.as_str()),
        crate::ast::MaterialReturnTy::Default => program
            .material_properties
            .iter()
            .find(|m| m.is_default)
            .map(|m| m.name.as_str()),
    };
    for _ in 0..=program.material_properties.len() {
        let Some(current) = name else {
            return false;
        };
        if current == target {
            return true;
        }
        name = program
            .material_properties
            .iter()
            .find(|m| m.name == current)
            .and_then(|m| m.extends_name.as_deref());
    }
    false
}

pub(super) fn resolve(program: &mut Program) -> Result<(), Vec<Diag>> {
    let context = program.clone();
    for surface in &mut program.surfaces {
        let mut states = BTreeMap::new();
        for pass in &context.passes {
            if pass.state.is_empty() || !applies(&context, surface, pass.material_name.as_deref()) {
                continue;
            }
            let mut result = ManifestRasterState::default();
            let mut seen = HashSet::new();
            for (name, expression) in &pass.state {
                let fail = |message: String| vec![Diag::error(expression.span.clone(), message)];
                if !seen.insert(name) {
                    return Err(fail(format!("duplicate raster state `{name}`")));
                }
                let choices: &[&str] = match name.as_str() {
                    "cull" => &["none", "front", "back"],
                    "depth_test" | "depth_compare" => &[
                        "never",
                        "less",
                        "equal",
                        "less_equal",
                        "greater",
                        "not_equal",
                        "greater_equal",
                        "always",
                    ],
                    "depth_write" => &["off", "on"],
                    "blend_all" => &["replace", "alpha", "additive", "premultiplied"],
                    _ => {
                        return Err(fail(format!(
                            "unsupported executable raster state `{name}`"
                        )));
                    }
                };
                let mut bindings: Vec<_> = choices
                    .iter()
                    .enumerate()
                    .map(|(index, name)| Stmt::Const {
                        name: (*name).into(),
                        name_span: expression.span.clone(),
                        ty_name: "f32".into(),
                        ty_span: expression.span.clone(),
                        value: Spanned {
                            node: Expr::Num(index as f64, crate::ast::Unit::None),
                            span: expression.span.clone(),
                        },
                    })
                    .collect();
                bindings.extend(
                    surface
                        .body
                        .iter()
                        .filter(|s| matches!(s, Stmt::Const { .. }))
                        .cloned(),
                );
                let value =
                    crate::check::compute::constant_scalar(&context, &bindings, expression)?;
                if !value.is_finite()
                    || value.fract() != 0.0
                    || value < 0.0
                    || value >= choices.len() as f32
                {
                    return Err(fail(format!("invalid value for raster state `{name}`")));
                }
                let value = choices[value as usize].to_string();
                match name.as_str() {
                    "cull" => result.cull = Some(value),
                    "depth_test" | "depth_compare" => {
                        if result.depth_compare.replace(value).is_some() {
                            return Err(fail("duplicate depth comparison".into()));
                        }
                    }
                    "depth_write" => result.depth_write = Some(value == "on"),
                    "blend_all" => result.blend = Some(value),
                    _ => unreachable!("validated raster state"),
                }
            }
            states.insert(pass.name.clone(), result);
        }
        let mut recipe_conditions = BTreeMap::new();
        let bindings: Vec<_> = surface
            .body
            .iter()
            .filter(|s| matches!(s, Stmt::Const { .. }))
            .cloned()
            .collect();
        for pipeline in &context.pipelines {
            if !applies(&context, surface, pipeline.material_name.as_deref()) {
                continue;
            }
            for (index, step) in pipeline.pass_refs.iter().enumerate() {
                let implementation = step.attrs.iter().find(|a| a.name == "implementation_when");
                let selected = if let Some(attr) = implementation {
                    let [contract, symbol] = attr.args.as_slice() else {
                        return Err(vec![Diag::error(
                            attr.span.clone(),
                            "invalid implementation condition",
                        )]);
                    };
                    surface.settings.as_ref().is_some_and(|settings| {
                        settings
                            .implementations
                            .iter()
                            .any(|s| s.contract == *contract && s.symbol == *symbol)
                    })
                } else {
                    true
                };
                if selected {
                    for requirement in step
                        .attrs
                        .iter()
                        .filter(|a| a.name == "implementation_require")
                    {
                        let [expression] = requirement.expressions.as_slice() else {
                            return Err(vec![Diag::error(
                                requirement.span.clone(),
                                "contribution @require needs one boolean expression",
                            )]);
                        };
                        if crate::check::compute::constant_scalar(&context, &bindings, expression)?
                            != 1.0
                        {
                            return Err(vec![Diag::error(
                                requirement.span.clone(),
                                format!(
                                    "contribution requirement is not satisfied for surface `{}`",
                                    surface.name
                                ),
                            )]);
                        }
                    }
                }
                let conditions: Vec<_> = step.attrs.iter().filter(|a| a.name == "when").collect();
                if conditions.len() > 1 {
                    return Err(vec![Diag::error(
                        step.span.clone(),
                        "duplicate step condition",
                    )]);
                }
                let mut enabled = selected
                    && step
                        .invocation
                        .as_ref()
                        .is_none_or(|i| i.material == surface.name);
                for condition in &conditions {
                    let [expression] = condition.expressions.as_slice() else {
                        return Err(vec![Diag::error(
                            condition.span.clone(),
                            "@when requires one constant expression",
                        )]);
                    };
                    let value =
                        crate::check::compute::constant_scalar(&context, &bindings, expression)?;
                    if value != 0.0 && value != 1.0 {
                        return Err(vec![Diag::error(
                            expression.span.clone(),
                            "step condition must be boolean",
                        )]);
                    }
                    enabled &= value != 0.0;
                }
                if implementation.is_some() || !conditions.is_empty() || step.invocation.is_some() {
                    recipe_conditions.insert(format!("{}:{index}", pipeline.name), enabled);
                }
            }
        }
        if states.is_empty() && recipe_conditions.is_empty() {
            continue;
        }
        let settings = surface.settings.get_or_insert_with(|| SurfaceSettings {
            shading_inputs: Default::default(),
            shading_samplers: Default::default(),
            implementation_slots: Default::default(),
            implementations: Vec::new(),
            property_u32: Default::default(),
            recipe_conditions: BTreeMap::new(),
            pass_states: BTreeMap::new(),
            evaluation_axes: BTreeMap::new(),
            properties: Vec::new(),
            usages: Vec::new(),
        });
        settings.pass_states = states;
        settings.recipe_conditions = recipe_conditions;
    }
    Ok(())
}
