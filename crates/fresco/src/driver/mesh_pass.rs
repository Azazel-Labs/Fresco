//! Lowering for engine-authored mesh vertex factories and material raster passes.

use super::gpu_function::*;
use crate::ast::{BinOp, Expr, SExpr, Stmt};
mod preparation_shader;
mod program;
pub(super) use program::{
    OperationUse, field_type, service_closure, validate_operation, validate_services,
};
pub(super) mod shader;

use std::collections::{BTreeSet, HashMap};

use crate::ast::{
    PassDecl, StructDecl, VertexFactoryDecl, VertexFormatDecl, VertexInterfaceMemberDecl,
};
use crate::material_hir::MaterialHir;

#[derive(Debug, Clone)]
pub(super) struct MeshStage {
    pub procedural: bool,
    pub shading_inputs:
        std::collections::BTreeMap<String, fresco_artifact::ManifestDrawComputeBinding>,
    /// Bindings from the actual material-specialized pass used for shader emission.
    pub bindings: Vec<crate::ast::PassBindingDecl>,
    pub entries: Vec<fresco_artifact::ManifestRasterEntry>,
    pub variants: Vec<MeshVariant>,
    pub pass: String,
    pub factory: String,
    pub wgsl: String,
    pub preparation: Option<fresco_artifact::ManifestMeshPreparation>,
}

pub(super) use fresco_artifact::ManifestMeshVariant as MeshVariant;

/// Explicit pass bindings own their slots even when their declarations are
/// unused. An entry must not read a material global at such a slot while the
/// host supplies the pass resource instead. Separate passes may reuse slots.
pub(super) fn validate_binding_scope(
    module: &naga::Module,
    info: &naga::valid::ModuleInfo,
    stage: &MeshStage,
    range: std::ops::Range<usize>,
) -> Result<(), String> {
    let owned = |handle| {
        module
            .global_variables
            .get_span(handle)
            .to_range()
            .is_some_and(|span| range.start <= span.start && span.end <= range.end)
    };
    let slots: BTreeSet<_> = module
        .global_variables
        .iter()
        .filter_map(|(handle, global)| {
            global
                .binding
                .as_ref()
                .filter(|_| owned(handle))
                .map(|slot| (slot.group, slot.binding))
        })
        .collect();
    let entries: BTreeSet<_> = stage
        .variants
        .iter()
        .flat_map(|variant| &variant.entries)
        .chain(&stage.entries)
        .map(|entry| entry.entry.as_str())
        .chain(
            stage
                .preparation
                .iter()
                .map(|preparation| preparation.entry.as_str()),
        )
        .collect();
    for (index, entry) in module.entry_points.iter().enumerate() {
        if !entries.contains(entry.name.as_str()) {
            continue;
        }
        for (handle, global) in module.global_variables.iter() {
            if !owned(handle)
                && !info.get_entry_point(index)[handle].is_empty()
                && let Some(slot) = &global.binding
                && slots.contains(&(slot.group, slot.binding))
            {
                return Err(format!(
                    "mesh pass `{}` binding at group {} binding {} shadows an external shader resource used by entry `{}`",
                    stage.pass, slot.group, slot.binding, entry.name
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn for_surface(
    material: &MaterialHir,
    library: &crate::check::compute::ComputeLibrary,
    structs: &[StructDecl],
    passes: &[PassDecl],
    pipelines: &[crate::ast::PipelineDecl],
    formats: &[VertexFormatDecl],
    factories: &[VertexFactoryDecl],
) -> Result<Vec<MeshStage>, String> {
    let selected = pipelines
        .iter()
        .find(|p| p.attrs.iter().any(|a| a.name == "selected_renderer"));
    let eligible: Vec<_> = passes
        .iter()
        .filter(|pass| {
            if let Some(pipeline) = selected
                && !pipeline.pass_refs.iter().enumerate().any(|(index, r)| {
                    r.name == pass.name
                        && material
                            .settings
                            .as_ref()
                            .and_then(|s| {
                                s.recipe_conditions
                                    .get(&format!("{}:{index}", pipeline.name))
                            })
                            .copied()
                            .unwrap_or(true)
                })
            {
                return false;
            }
            let Some(expected) = &pass.material_name else {
                return true;
            };
            let mut actual = material.material_properties_name.as_deref();
            for _ in 0..=material.material_schemas.len() {
                let Some(name) = actual else {
                    return false;
                };
                if name == expected {
                    return true;
                }
                actual = material
                    .material_schemas
                    .get(name)
                    .and_then(Option::as_deref);
            }
            false
        })
        .cloned()
        .collect();
    let mut demand: BTreeSet<_> = eligible
        .iter()
        .filter_map(|p| p.prepared_draw.as_ref().map(|d| d.producer_pass.clone()))
        .collect();
    for invocation in passes
        .iter()
        .filter_map(|p| p.compute_invocation.as_ref())
        .filter(|i| i.material == material.name)
    {
        for argument in invocation.arguments.values() {
            if let fresco_artifact::ManifestComputeArgument::Geometry { producer, .. } = argument {
                demand.insert(producer.clone());
            }
        }
    }
    let eligible: Vec<_> = eligible
        .into_iter()
        .map(|mut pass| {
            if pass.preparation.is_some() && !demand.contains(&pass.name) {
                pass.preparation = None;
                pass.bindings
                    .retain(|b| !b.attrs.iter().any(|a| a.name == "geometry_resource"));
            }
            pass
        })
        .collect();
    let passes = eligible.as_slice();
    let usages = material
        .settings
        .as_ref()
        .map(|settings| settings.usages.as_slice())
        .unwrap_or(&[]);
    let requested: Vec<Option<&str>> = if usages.is_empty() {
        vec![None]
    } else {
        usages.iter().map(|name| Some(name.as_str())).collect()
    };
    let mut stages = Vec::new();
    for pass in passes
        .iter()
        .filter(|p| p.attrs.iter().any(|a| a.name == "factory"))
    {
        let mut pass = pass.clone();
        let shading_inputs = super::implementations::specialize_draw_dispatch(
            &mut pass,
            material.settings.as_ref(),
            factories,
        )
        .map_err(|errors| format!("invalid draw dispatch: {errors:?}"))?;
        let pass = &pass;
        let mut result: Option<MeshStage> = None;
        for factory in &requested {
            let Some(mut stage) = for_factory(
                material,
                library,
                structs,
                std::slice::from_ref(pass),
                formats,
                factories,
                *factory,
            )?
            else {
                continue;
            };
            stage.shading_inputs.clone_from(&shading_inputs);
            let variant = MeshVariant {
                entries: stage.entries.clone(),
                factory: stage.factory.clone(),
            };
            if let Some(result) = result.as_mut() {
                result.wgsl.push_str(&stage.wgsl);
                result.variants.push(variant);
            } else {
                stage.variants.push(variant);
                result = Some(stage);
            }
        }
        if let Some(stage) = result {
            stages.push(stage);
        }
    }
    if stages.is_empty() && !usages.is_empty() {
        return Err("surface usage requires an executable mesh pass".into());
    }
    Ok(stages)
}

fn for_factory(
    material: &MaterialHir,
    library: &crate::check::compute::ComputeLibrary,
    structs: &[StructDecl],
    passes: &[PassDecl],
    formats: &[VertexFormatDecl],
    factories: &[VertexFactoryDecl],
    requested_factory: Option<&str>,
) -> Result<Option<MeshStage>, String> {
    let candidates = passes
        .iter()
        .filter_map(|pass| {
            let factory_name = pass
                .attrs
                .iter()
                .find(|attr| attr.name == "factory")?
                .args
                .first()?;
            Some((pass, factory_name.as_str()))
        })
        .collect::<Vec<_>>();
    let Some((pass, factory_name)) = candidates.first().copied() else {
        return Ok(None);
    };
    if !pass
        .stage
        .as_ref()
        .is_some_and(|stage| stage.node == "raster")
        || pass.draw.is_none()
    {
        return Err(format!(
            "executable draw pass `{}` must be a raster pass with a draw declaration",
            pass.name
        ));
    }
    let factory_name = requested_factory.unwrap_or(factory_name);
    let factory = factories
        .iter()
        .find(|factory| factory.name == factory_name)
        .ok_or_else(|| {
            format!(
                "mesh pass `{}` selects unknown vertex factory `{factory_name}`",
                pass.name
            )
        })?;
    if pass
        .draw
        .as_ref()
        .is_some_and(|draw| draw.node == "fullscreen")
    {
        let mut procedural = pass.clone();
        // Preserve factory-specific entry names when a material supports more
        // than one factory. Resources remain the explicitly declared bindings.
        procedural.name = format!("{}_{}", pass.name, factory.name);
        procedural.bindings.extend(factory.bindings.iter().cloned());
        let (wgsl, program) = shader::emit(&procedural, structs, library, Some(material), "")?;
        return Ok(Some(MeshStage {
            procedural: true,
            shading_inputs: Default::default(),
            bindings: pass.bindings.clone(),
            entries: program.entries,
            variants: Vec::new(),
            pass: pass.name.clone(),
            factory: factory.name.clone(),
            wgsl,
            preparation: None,
        }));
    }
    if let Some(prepared) = &pass.prepared_draw {
        if prepared.producer_hook.is_empty() {
            return Err("prepared draw has no preparation function".into());
        }
        let (wgsl, program) = shader::emit(pass, structs, library, Some(material), "")?;
        return Ok(Some(MeshStage {
            procedural: false,
            shading_inputs: Default::default(),
            bindings: pass.bindings.clone(),
            entries: program.entries,
            variants: Vec::new(),
            pass: pass.name.clone(),
            factory: factory.name.clone(),
            wgsl,
            preparation: None,
        }));
    }
    let transform = factory
        .hooks
        .iter()
        .find(|hook| hook.name == "transform")
        .ok_or_else(|| {
            format!(
                "mesh pass `{}` selects vertex factory `{factory_name}` without a transform hook",
                pass.name
            )
        })?;
    let vertex = pass
        .hooks
        .iter()
        .find(|hook| hook.attrs.iter().any(|a| a.name == "vertex"))
        .ok_or_else(|| {
            format!(
                "executable mesh pass `{}` requires a vertex hook",
                pass.name
            )
        })?;
    if vertex.params.len() != 1 || vertex.return_ty.is_none() {
        return Err(format!(
            "mesh pass `{}` vertex hook must accept one vertex interface and return a varying struct",
            pass.name
        ));
    }
    let varying_name = &vertex.return_ty.as_ref().expect("checked return type").node;
    let varying = find_struct(structs, varying_name).ok_or_else(|| {
        format!(
            "mesh pass `{}` returns unknown varying struct `{varying_name}`",
            pass.name
        )
    })?;
    validate_varying(pass, varying)?;

    let formats_by_name = formats
        .iter()
        .map(|format| (format.name.as_str(), format))
        .collect::<HashMap<_, _>>();
    let members = resolve_format_members(
        &factory.target_format,
        &formats_by_name,
        &mut BTreeSet::new(),
    )
    .ok_or_else(|| {
        format!(
            "vertex factory `{factory_name}` has an unresolved format `{}`",
            factory.target_format
        )
    })?;
    let material_suffix = sanitize(&material.name);
    let suffix = format!(
        "{}_{}_{}",
        material_suffix,
        sanitize(factory_name),
        sanitize(&pass.name)
    );
    let factory_suffix = format!(
        "{}_{}_{}",
        sanitize(factory_name),
        material_suffix,
        sanitize(&pass.name)
    );
    let input_ty = format!("FrescoMeshInput_{factory_suffix}");
    let varying_ty = format!("FrescoMeshVarying_{suffix}");
    let entry_base = format!("fresco_mesh_{}_{}", sanitize(&pass.name), suffix);
    let transform_entry = format!("fresco_vertex_factory_{factory_suffix}_transform");

    let mut binding_vars = HashMap::new();
    let mut resource_decls = String::new();
    let mut resource_types = HashMap::new();
    let mut resource_structs = BTreeSet::new();
    let mut slots = BTreeSet::new();
    let mut names = BTreeSet::new();
    for binding in factory.bindings.iter().chain(&pass.bindings) {
        let group = binding.group_index.ok_or_else(|| format!("vertex factory `{factory_name}` binding `{}` requires a resolved @group(...) declaration", binding.name))?;
        let binding_index = binding
            .binding_index
            .ok_or("unresolved factory binding index")?;
        if !slots.insert((group, binding_index)) || !names.insert(&binding.name) {
            return Err(format!(
                "mesh pass `{}` has conflicting resource bindings",
                pass.name
            ));
        }
        let signature = binding.value_signature.as_deref().ok_or_else(|| {
            format!(
                "vertex factory `{factory_name}` binding `{}` requires a resource signature",
                binding.name
            )
        })?;
        let var_name = format!(
            "fresco_resource_{factory_suffix}_{}",
            sanitize(&binding.name)
        );
        let (address_space, resource_ty) = if let Some(source_ty_name) = uniform_type(signature) {
            let resource_ty = if let Some(source_ty) = find_struct(structs, source_ty_name) {
                let resource_ty = format!(
                    "FrescoMeshResource_{factory_suffix}_{}",
                    sanitize(&source_ty.name)
                );
                resource_types.insert(source_ty.name.clone(), resource_ty.clone());
                if resource_structs.insert(resource_ty.clone()) {
                    resource_decls.push_str(&emit_plain_struct(source_ty, &resource_ty)?);
                }
                resource_ty
            } else {
                wgsl_type(source_ty_name)?
            };
            ("uniform", resource_ty)
        } else if let Some(element) = signature
            .strip_prefix("buffer<")
            .and_then(|s| s.strip_suffix('>'))
        {
            let element_ty = if let Some(record) = find_struct(structs, element) {
                let name = format!("FrescoMeshResource_{factory_suffix}_{}", sanitize(element));
                resource_types.insert(element.into(), name.clone());
                if resource_structs.insert(name.clone()) {
                    resource_decls.push_str(&emit_plain_struct(record, &name)?);
                }
                name
            } else {
                wgsl_type(element)?
            };
            let write = binding.attrs.iter().any(|a| {
                a.name == "geometry_resource" && a.args.get(1).is_some_and(|r| r == "output")
            });
            (
                if write {
                    "storage, read_write"
                } else {
                    "storage, read"
                },
                format!("array<{element_ty}>"),
            )
        } else if matches!(
            signature,
            "texture_2d<f32>"
                | "texture_2d<u32>"
                | "texture_2d<i32>"
                | "texture_depth_2d"
                | "sampler"
        ) {
            ("", signature.to_string())
        } else {
            return Err(format!(
                "executable vertex factory `{factory_name}` binding `{}` requires a uniform, read-only buffer, or sampled texture",
                binding.name
            ));
        };
        {
            let address = if address_space.is_empty() {
                String::new()
            } else {
                format!("<{address_space}>")
            };
            resource_decls.push_str(&format!(
                "@group({group}) @binding({binding_index})\nvar{address} {var_name}: {resource_ty};\n\n"
            ));
        }
        if let Some(name) = &binding.operation_alias
            && binding_vars
                .insert(name.as_str(), var_name.clone())
                .is_some()
        {
            return Err("operation binding alias is ambiguous".into());
        }
        binding_vars.insert(binding.name.as_str(), var_name);
    }

    let transform_context = ExprContext {
        resource_specializations: None,
        imported_functions: None,
        imported_records: None,
        referenced_calls: None,
        material: Some(material),
        input_var: &transform.params[0].name,
        varying_source_ty: varying_name,
        varying_ty: &varying_ty,
        factory_entry: &transform_entry,
        binding_vars: &binding_vars,
        type_aliases: Some(&resource_types),
    };
    program::emit(
        pass,
        library,
        factory,
        material,
        structs,
        &members,
        &input_ty,
        &varying_ty,
        &entry_base,
        &transform_entry,
        &resource_decls,
        &transform_context,
    )
    .map(Some)
}

fn find_struct<'a>(structs: &'a [StructDecl], name: &str) -> Option<&'a StructDecl> {
    structs.iter().find(|candidate| candidate.name == name)
}

fn validate_varying(pass: &PassDecl, varying: &StructDecl) -> Result<(), String> {
    let position: Vec<_> = varying
        .fields
        .iter()
        .filter(|field| field.semantic.as_deref() == Some("position"))
        .collect();
    if position.len() != 1 || position[0].ty_name != "vec4" {
        return Err(format!(
            "raster pass `{}` varying `{}` requires exactly one @semantic(position) vec4 field",
            pass.name, varying.name
        ));
    }
    Ok(())
}

fn resolve_format_members<'a>(
    name: &str,
    formats: &HashMap<&str, &'a VertexFormatDecl>,
    visiting: &mut BTreeSet<String>,
) -> Option<Vec<&'a VertexInterfaceMemberDecl>> {
    if !visiting.insert(name.to_string()) {
        return None;
    }
    let format = formats.get(name)?;
    let mut members = if let Some(parent) = &format.parent {
        resolve_format_members(parent, formats, visiting)?
    } else {
        Vec::new()
    };
    for member in &format.members {
        if let Some(index) = members
            .iter()
            .position(|candidate| candidate.name == member.name)
        {
            members[index] = member;
        } else {
            members.push(member);
        }
    }
    visiting.remove(name);
    Some(members)
}

fn emit_vertex_input(name: &str, members: &[&VertexInterfaceMemberDecl]) -> Result<String, String> {
    let mut output = format!("struct {name} {{\n");
    for (location, member) in members.iter().enumerate() {
        output.push_str(&format!(
            "  @location({location}) {}: {},\n",
            shader_identifier(&member.name),
            wgsl_type(&member.ty_name)?
        ));
    }
    output.push_str("}\n\n");
    Ok(output)
}

fn emit_varying(source: &StructDecl, name: &str) -> Result<String, String> {
    let mut output = format!("struct {name} {{\n");
    let mut location = 0_u32;
    for field in &source.fields {
        if field.semantic.as_deref() == Some("position") {
            output.push_str(&format!(
                "  @builtin(position) {}: {},\n",
                shader_identifier(&field.name),
                wgsl_type(&field.ty_name)?
            ));
        } else {
            output.push_str(&format!(
                "  @location({location}) {}: {},\n",
                shader_identifier(&field.name),
                wgsl_type(&field.ty_name)?
            ));
            location += 1;
        }
    }
    output.push_str("}\n\n");
    Ok(output)
}

fn emit_plain_struct(source: &StructDecl, name: &str) -> Result<String, String> {
    let mut output = format!("struct {name} {{\n");
    for field in &source.fields {
        output.push_str(&format!(
            "  {}: {},\n",
            shader_identifier(&field.name),
            wgsl_type(&field.ty_name)?
        ));
    }
    output.push_str("}\n\n");
    Ok(output)
}

fn uniform_type(signature: &str) -> Option<&str> {
    signature
        .strip_prefix("uniform<")
        .and_then(|value| value.strip_suffix('>'))
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect()
}
