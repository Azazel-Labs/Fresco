//! Compatibility reflection uses the same selected graph expansion as compilation.
//! Probes never mutate the executable program or instantiate runtime work.
use crate::ast::{Expr, MaterialReturnTy, Program, Spanned};
use fresco_artifact::{ManifestCapabilityAvailability, ManifestImplementationAvailability};

pub(super) fn reflect(program: &mut Program, context: &Program) {
    let renderer = context
        .pipelines
        .iter()
        .find(|p| p.attrs.iter().any(|a| a.name == "selected_renderer"));
    for surface in &mut program.surfaces {
        let original = context
            .surfaces
            .iter()
            .find(|s| s.name == surface.name)
            .expect("surface");
        let Some(settings) = surface.settings.as_mut() else {
            continue;
        };
        let schema = match &original.material_ty {
            MaterialReturnTy::Named(name) => name.clone(),
            MaterialReturnTy::Default => context
                .material_properties
                .iter()
                .find(|s| s.is_default)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "default".into()),
        };
        let factories: Vec<_> = super::style_graph::selected_factories(context, original, renderer)
            .into_iter()
            .cloned()
            .collect();
        for selection in &mut settings.implementations {
            let provider = renderer.and_then(|r| {
                context
                    .style_providers
                    .iter()
                    .find(|p| p.contract == selection.contract && p.renderer == r.name)
            });
            let contract = context
                .style_contracts
                .iter()
                .find(|c| c.name == selection.contract);
            let mut availability = Vec::new();
            for symbol in &selection.available {
                let mut candidate = selection.clone();
                candidate.symbol.clone_from(symbol);
                candidate.availability.clear();
                let mut probe = context.clone();
                probe.surfaces = vec![original.clone()];
                let result = (|| {
                    if symbol != &selection.symbol {
                        (candidate.parameters, candidate.static_parameters) =
                            super::styles::parameters(context, symbol, &[])?;
                    }
                    probe.surfaces[0]
                        .settings
                        .as_mut()
                        .expect("settings")
                        .implementations = vec![candidate.clone()];
                    super::styles::check_selection(
                        &probe,
                        &selection.contract,
                        &probe.surfaces[0],
                    )?;
                    super::style_graph::requirements(&probe)?;
                    if let Some(style) = probe.styles.iter().find(|s| s.name == *symbol) {
                        let mut expanded = probe.clone();
                        super::style_operations::prepare_selection(
                            &mut expanded,
                            &probe,
                            style,
                            &probe.surfaces[0],
                            &candidate,
                        )?;
                        super::style_graph::operation_inputs(&expanded)?;
                    }
                    Ok::<(), Vec<crate::diag::Diag>>(())
                })();
                let member = |name: &str| {
                    let provided =
                        provider.is_some_and(|p| p.blocks.iter().any(|b| b.name == name));
                    let reasons = if !provided {
                        vec![format!("selected renderer does not provide `{name}`")]
                    } else if let (Some(renderer), Some(provider)) = (renderer, provider) {
                        super::style_graph::check_capability(
                            context,
                            symbol,
                            &provider.source_file,
                            original,
                            renderer,
                            provider,
                            &Spanned {
                                node: Expr::Var(name.into()),
                                span: provider.span.clone(),
                            },
                        )
                        .err()
                        .into_iter()
                        .flatten()
                        .map(|d| d.message)
                        .collect()
                    } else {
                        unreachable!("a provided member has a renderer provider")
                    };
                    ManifestCapabilityAvailability {
                        name: name.into(),
                        provided,
                        supported: reasons.is_empty(),
                        reasons,
                    }
                };
                availability.push(ManifestImplementationAvailability {
                    symbol: symbol.clone(),
                    renderer: renderer.map(|r| r.name.clone()),
                    schema: schema.clone(),
                    factories: factories.clone(),
                    provider: provider.is_some(),
                    capabilities: contract
                        .into_iter()
                        .flat_map(|c| &c.capabilities)
                        .map(|c| member(&c.name))
                        .collect(),
                    integration_points: contract
                        .into_iter()
                        .flat_map(|c| &c.points)
                        .map(|c| member(&c.name))
                        .collect(),
                    static_parameters: candidate.static_parameters,
                    supported: result.is_ok(),
                    reasons: result
                        .err()
                        .into_iter()
                        .flatten()
                        .map(|d| d.message)
                        .collect(),
                });
            }
            selection.availability = availability;
        }
    }
}
