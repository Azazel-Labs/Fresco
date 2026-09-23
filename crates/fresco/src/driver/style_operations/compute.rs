//! Selected compute calls become independent, material-owned GPU programs.
use super::*;
use fresco_artifact::{
    ComputeScalarType, ManifestComputeArgument as Argument, ManifestComputeInvocation,
    ManifestComputeOutput,
};

pub(super) fn invocation_name(surface: &SurfaceDecl, style: &StyleDecl, ordinal: usize) -> String {
    format!("__style_{}_{}_{ordinal:04}", surface.name, style.name)
}

pub(super) fn scalar_argument(
    context: &Program,
    style: &StyleDecl,
    selection: &ManifestImplementationSelection,
    call: &Call,
    ty: &str,
    value: &SExpr,
) -> Result<Argument, Vec<Diag>> {
    Ok(
        if let Some((index, setting)) = path(value).and_then(|name| {
            style
                .params
                .iter()
                .enumerate()
                .find(|(_, setting)| setting.name == name)
        }) {
            let offset = selection
                .settings_offset
                .checked_add(
                    u32::try_from(index)
                        .map_err(|_| fail(&value.span, "compute setting offset overflow"))?,
                )
                .ok_or_else(|| fail(&value.span, "compute setting offset overflow"))?;
            Argument::Setting {
                ty: setting.ty_name.clone(),
                name: setting.name.clone(),
                offset,
            }
        } else {
            let constants = constant_program(context, &call.bindings);
            let parameter = GlobalParamDecl {
                name: "__compute_argument".into(),
                name_span: value.span.clone(),
                ty_name: ty.into(),
                ty_span: value.span.clone(),
                default: None,
                range: None,
                span: value.span.clone(),
            };
            Argument::Constant {
                ty: ty.into(),
                value: crate::check::compute::style_parameter_value(&constants, &parameter, value)?,
            }
        },
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "shared invocation lowering context"
)]
pub(super) fn instantiate(
    program: &mut Program,
    context: &Program,
    style: &StyleDecl,
    surface: &SurfaceDecl,
    selection: &ManifestImplementationSelection,
    call: &Call,
    ordinal: u32,
    definition: &PassDecl,
) -> Result<(), Vec<Diag>> {
    let operation = definition
        .operation
        .as_ref()
        .expect("validated compute operation");
    let compute = operation.compute.as_ref().expect("compute operation");
    let output = compute.output.as_ref().expect("validated owned output");
    let projections = captures::arguments(context, style, call)?;
    for value in output
        .extents
        .iter()
        .chain(compute.threads.iter().flatten())
        .chain(&operation.requirements)
    {
        captures::reject_host(value, &projections)?;
    }
    let projected_types = projections
        .iter()
        .map(|(name, projection)| {
            (
                name.clone(),
                (projection.root_ty.clone(), projection.member.clone()),
            )
        })
        .collect();
    let mut kernel = super::super::compute_operations::shader_definition_projected(
        context,
        definition,
        &projected_types,
    )?;
    kernel.pass.name = invocation_name(surface, style, ordinal as usize);
    kernel.pass.material_name = None;
    kernel.pass.attrs.push(attr("shader", vec![]));
    let (_, reflection) = kernel.emit(context).map_err(|m| fail(&call.span, m))?;
    let entry = reflection.entries.first().expect("validated compute entry");
    let reads = reflection
        .compute_shader_reads(&entry.entry)
        .map_err(|m| fail(&call.span, m))?;
    let layout = match super::super::compute_operations::resource_type(&output.ty, "write")
        .map_err(|m| fail(&output.span, m))?
    {
        super::super::compute_operations::ResourceType::Buffer(element) => {
            let stride = reflection
                .bindings
                .iter()
                .find(|b| b.name == output.name)
                .and_then(|b| b.element_stride)
                .ok_or_else(|| fail(&output.span, "owned output has no reflected buffer stride"))?;
            fresco_artifact::ManifestComputeOutputLayout::Buffer { element, stride }
        }
        super::super::compute_operations::ResourceType::Image(format) => {
            fresco_artifact::ManifestComputeOutputLayout::Image {
                format: format.info().name.into(),
            }
        }
    };
    let mut arguments = BTreeMap::new();
    let mut dependencies = BTreeSet::new();
    let mut engine_dependencies = BTreeSet::new();
    let mut metadata_dependencies = BTreeSet::new();
    for parameter in &operation.inputs {
        let value = &call
            .args
            .iter()
            .find(|arg| arg.name.as_ref() == Some(&parameter.name))
            .expect("validated argument")
            .value;
        let projected_value = projections.get(&parameter.name).map(|projection| Spanned {
            node: Expr::Var(projection.root.clone()),
            span: value.span.clone(),
        });
        let value = projected_value.as_ref().unwrap_or(value);
        let ty = compact(
            projections
                .get(&parameter.name)
                .map_or(parameter.ty.as_str(), |projection| {
                    projection.root_ty.as_str()
                }),
        );
        let argument = if let Some(producer) = call.resources.get(&parameter.name) {
            let producer = invocation_name(surface, style, *producer);
            // A bound handle always needs an allocation, including query-only
            // uses and unused bindings retained in the pipeline layout.
            metadata_dependencies.insert(producer.clone());
            if reads.data.contains(&parameter.name) {
                dependencies.insert(producer.clone());
            }
            Argument::Output { producer }
        } else if scalar(&ty) {
            scalar_argument(context, style, selection, call, &ty, value)?
        } else if context.resource_types.iter().any(|r| r.name == ty) {
            let geometry = super::super::prepared_geometry::select_resource(
                context, style, parameter, &call.args,
            )?;
            // Geometry preparation is its own producer, not the opaque draw
            // belonging to the same authored engine pass.
            Argument::Geometry {
                producer: geometry.producer_pass,
                hook: geometry.producer_hook,
                node: geometry.producer_node,
                resource_type: geometry.resource.name,
            }
        } else {
            let renderer = context
                .pipelines
                .iter()
                .find(|p| p.attrs.iter().any(|a| a.name == "selected_renderer"))
                .ok_or_else(|| {
                    fail(
                        &call.span,
                        "compute invocation requires a selected renderer",
                    )
                })?;
            let provider = context
                .style_providers
                .iter()
                .find(|p| p.contract == style.contract && p.renderer == renderer.name)
                .ok_or_else(|| {
                    fail(
                        &call.span,
                        "compute input requires a selected renderer provider",
                    )
                })?;
            let name = path(value).expect("validated resource argument");
            let contract = context
                .style_contracts
                .iter()
                .find(|c| c.name == style.contract)
                .expect("validated contract");
            let (provided, provider_type) = if let Some((capability, member)) =
                super::super::prepared_geometry::member(context, contract, &name)
                    .map_err(|m| fail(&value.span, m))?
            {
                (
                    provider
                        .blocks
                        .iter()
                        .find(|b| b.name == capability.name)
                        .and_then(|b| b.fields.iter().find(|field| field.name == name))
                        .and_then(|field| {
                            if let [value] = field.values.as_slice() {
                                Some(value)
                            } else {
                                None
                            }
                        }),
                    member.ty.as_str(),
                )
            } else {
                (
                    provider
                        .inputs
                        .iter()
                        .find(|(input, _)| input == &name)
                        .map(|(_, value)| value),
                    contract
                        .inputs
                        .iter()
                        .find(|input| input.name == name)
                        .expect("checked contract input")
                        .ty
                        .as_str(),
                )
            };
            let provided =
                provided.ok_or_else(|| fail(&value.span, "missing compute input provider"))?;
            let (resources, steps) =
                super::super::recipes::reflect(renderer).map_err(|m| fail(&value.span, m))?;
            let resource = super::super::style_graph::bound_resource(
                context,
                &resources,
                &steps,
                provided,
                provider_type,
                None,
            )
            .map_err(|m| fail(&value.span, m))?;
            if reads.data.contains(&parameter.name) || reads.values.contains(&parameter.name) {
                for step in &steps {
                    let writes = super::super::style_graph::bindings(context, step, None)
                        .iter()
                        .any(|binding| {
                            step.bindings.get(&binding.name) == Some(&resource)
                                && binding.attrs.iter().any(|attr| {
                                    attr.name == "access"
                                        && attr.args.iter().any(|access| {
                                            matches!(access.as_str(), "write" | "read_write")
                                        })
                                })
                        });
                    if writes {
                        engine_dependencies.insert(step.name.clone());
                    }
                }
            }
            Argument::External { resource, ty }
        };
        arguments.insert(parameter.name.clone(), argument);
    }
    let lower = |value: &SExpr, expected| {
        super::super::compute_host::lower(context, operation, value, Some(expected))
    };
    kernel.pass.compute_invocation = Some(Box::new(ManifestComputeInvocation {
        material: surface.name.clone(),
        operation: call.operation.clone(),
        ordinal,
        arguments,
        bindings: kernel.bindings,
        output: ManifestComputeOutput {
            binding: output.name.clone(),
            ty: compute.return_ty.node.clone(),
            layout,
            extents: output
                .extents
                .iter()
                .map(|v| lower(v, ComputeScalarType::U32))
                .collect::<Result<_, _>>()?,
        },
        threads: compute
            .threads
            .as_ref()
            .expect("validated dispatch")
            .iter()
            .map(|v| lower(v, ComputeScalarType::U32))
            .collect::<Result<_, _>>()?,
        requirements: operation
            .requirements
            .iter()
            .map(|v| lower(v, ComputeScalarType::Bool))
            .collect::<Result<_, _>>()?,
        dependencies: dependencies.into_iter().collect(),
        engine_dependencies: engine_dependencies.into_iter().collect(),
        metadata_dependencies: metadata_dependencies.into_iter().collect(),
    }));
    program.passes.push(kernel.pass);
    Ok(())
}
